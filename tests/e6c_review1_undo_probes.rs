// tests/e6c_review1_undo_probes.rs --- E6c review 1: the arbiter's
// grain and the design's four answers, probed in both histories.

//! Behavioral probes against E6c's rows, the states a user reaches that
//! the phase's own witnesses did not choose, each run once in a plain
//! (v0.1 stack) buffer and once in a buffer upgraded to CRDT (the
//! cross-peer arbiter), the texts compared step for step across the
//! two. Every keystroke goes through `dispatch_key` and every undo and
//! redo through its bound chord, so a probe asserts through the seam a
//! TUI user reaches.
//!
//! The rows: the everyday sequence the net-diff base got wrong
//! (`hello`, Backspace twice, then undo until nothing is left); the
//! grain as a user meets it (`foo(` and undo, `foo(` typed inside and
//! undo, a lean expansion and undo); the knob at zero, where a
//! keystroke's command must still be one step with the closer its hook
//! inserted; the sourceless edit (a script's insert goes to whoever
//! undoes next); and redo (undo, undo, redo, redo; undo, type, redo).

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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

/// `C-/`, bound to `buffer.undo`.
fn undo(s: &mut EditorState) {
    ctrl(s, '/');
}

/// `C-x r`, bound to `buffer.redo`: the prefix chord, as the owner
/// would press it.
fn redo(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('r'));
}

/// Upgrade the active buffer to CRDT in place, so its history is the
/// arbiter's from the first keystroke.
#[cfg(feature = "crdt")]
fn upgrade_active(s: &EditorState) {
    let core = s.core.borrow();
    let id = core.active_buffer_id();
    let mut reg = core.registry.borrow_mut();
    reg.get_mut(id)
        .expect("active buffer")
        .upgrade_to_crdt(1)
        .expect("upgrade");
}

/// A fresh editor on the scratch buffer; with `crdt` upgraded in place.
fn fresh(crdt: bool) -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    if crdt {
        #[cfg(feature = "crdt")]
        upgrade_active(&s);
    }
    #[cfg(not(feature = "crdt"))]
    assert!(!crdt, "a CRDT row needs the crdt feature");
    s.core.borrow_mut().pending_crdt_ops.clear();
    s
}

/// Run `story` in a plain buffer and in a CRDT one; return the text
/// after the story and after each of `steps`, per mode.
fn both_with(
    make: impl Fn(bool) -> EditorState,
    story: impl Fn(&mut EditorState),
    steps: &[fn(&mut EditorState)],
) -> (Vec<String>, Vec<String>) {
    let run = |crdt: bool| {
        let mut s = make(crdt);
        story(&mut s);
        let mut out = vec![text(&s)];
        for step in steps {
            step(&mut s);
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

fn both(
    story: impl Fn(&mut EditorState),
    steps: &[fn(&mut EditorState)],
) -> (Vec<String>, Vec<String>) {
    both_with(fresh, story, steps)
}

// ---------------------------------------------------------------------------
// Item 3: the base that was tried and the base that works
// ---------------------------------------------------------------------------

/// `hello`, Backspace twice, then undo until nothing is left. The
/// Backspaces are command chords, each its own step, so the walk back
/// is `hel`, `hell`, `hello`, empty --- and the last step is the one
/// the net-diff base got wrong: the two characters the Backspace undos
/// reinserted are new daemon-peer text, and a base holding the net
/// document diff kept them as foreign, leaving `lo`. The shipped
/// per-span base composes only *foreign* changes, so the source's own
/// compensations cancel with the groups they undid and `hello` goes
/// whole, in both histories.
#[test]
fn review_hello_backspace_twice_then_undo_to_nothing_agrees_across_histories() {
    let (plain, crdt) = both(
        |s| {
            type_str(s, "hello");
            press(s, KeyCode::Backspace);
            press(s, KeyCode::Backspace);
        },
        &[undo, undo, undo, undo],
    );
    assert_eq!(plain[0], "hel", "two Backspaces");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "hell", "one undo: the second Backspace");
    assert_eq!(plain[2], "hello", "two undos: the first Backspace");
    assert_eq!(
        plain[3], "",
        "three undos: hello goes whole, the reinserted `lo` with it"
    );
    assert_eq!(plain[4], "", "four undos: nothing left");
}

// ---------------------------------------------------------------------------
// Item 5: the grain as a user meets it
// ---------------------------------------------------------------------------

/// `foo(` with auto-pair on: one keystroke run of four characters, the
/// closer inside the fourth. One undo empties the buffer in both
/// histories; nothing more than the run goes, and a second undo finds
/// nothing.
#[test]
fn review_foo_paren_one_undo_agrees_across_histories() {
    let (plain, crdt) = both(|s| type_str(s, "foo("), &[undo, undo]);
    assert_eq!(plain[0], "foo()", "auto-pair closed the paren");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "", "one undo: the run with its closer");
    assert_eq!(plain[2], "", "two undos: nothing left");
}

/// `foo(` then a character typed inside the pair: five self-inserts,
/// one run, the closer inside the fourth. One undo empties the buffer
/// in both histories --- the run, not more.
#[test]
fn review_foo_paren_then_typing_inside_one_undo_agrees_across_histories() {
    let (plain, crdt) = both(|s| type_str(s, "foo(x"), &[undo, undo]);
    assert_eq!(plain[0], "foo(x)", "typed inside the pair");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "", "one undo: the run with its closer");
    assert_eq!(plain[2], "", "two undos: nothing left");
}

/// `foo(`, then `End` (a command chord, which closes the run), then
/// `x` typed after the pair: the second run is `x` alone, so one undo
/// removes `x` and the next removes `foo()` --- a hook's closer never
/// crosses a keystroke boundary in either direction.
#[test]
fn review_foo_paren_end_then_x_undoes_x_alone_across_histories() {
    let (plain, crdt) = both(
        |s| {
            type_str(s, "foo(");
            press(s, KeyCode::End);
            type_str(s, "x");
        },
        &[undo, undo],
    );
    assert_eq!(plain[0], "foo()x");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "foo()", "one undo: the second run, x alone");
    assert_eq!(plain[2], "", "two undos: the first run with its closer");
}

/// A fresh editor on an empty `.lean` file, the lean input method live;
/// with `crdt` the file's buffer is upgraded in place.
fn lean(crdt: bool) -> EditorState {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-e6c-review1-lean-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
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
        "the fixture must be a lean4 buffer, or the expansion is vacuous"
    );
    if crdt {
        #[cfg(feature = "crdt")]
        upgrade_active(&s);
    }
    #[cfg(not(feature = "crdt"))]
    assert!(!crdt, "a CRDT row needs the crdt feature");
    s.core.borrow_mut().pending_crdt_ops.clear();
    s
}

/// A lean expansion and undo, in both histories: `\alpha` expands
/// eagerly inside the sixth keystroke; one undo takes the run with the
/// expansion in it, and a second finds nothing. The lean suite pins
/// this in the plain history only.
#[test]
fn review_lean_expansion_one_undo_agrees_across_histories() {
    let (plain, crdt) = both_with(lean, |s| type_str(s, "\\alpha"), &[undo, undo]);
    assert_eq!(plain[0], "α", "eager expansion");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(
        plain[1], "",
        "one undo: the run and the expansion inside it"
    );
    assert_eq!(plain[2], "", "two undos: nothing left");
}

/// The knob at zero --- every keystroke its own step --- must still
/// keep a keystroke's command whole: `(` and the closer its hook
/// inserted are one step in both histories, and `ab(` undoes as `ab`,
/// `a`, empty. The v0.1 stack collapses the command's entries whatever
/// the knob says; the arbiter opened no group at zero
/// (`Buffer::arbiter_typed_begin`'s `if limit != 0` guard), so the
/// opener and the closer landed as two groups and the CRDT history
/// walked `ab(`, `ab`, `a`, empty. Failed at the reviewed head
/// `c4be8aa` on that first undo (E6c review 1, Medium 1); the fix
/// round opens the group at zero as at any limit.
#[test]
fn review_amalgamate_zero_keeps_the_pair_as_one_step_across_histories() {
    let (plain, crdt) = both(
        |s| {
            exec(s, "pmacs.config.set('undo.amalgamate', 0)");
            type_str(s, "ab(");
        },
        &[undo, undo, undo, undo],
    );
    assert_eq!(plain[0], "ab()", "auto-pair closed the paren");
    assert_eq!(
        plain[1], "ab",
        "one undo: the keystroke with its closer, nothing else"
    );
    assert_eq!(plain[2], "a", "two undos: one character per step at zero");
    assert_eq!(plain[3], "", "three undos: empty");
    assert_eq!(plain[4], "", "four undos: nothing left");
    assert_eq!(
        plain, crdt,
        "the two histories must agree step for step at undo.amalgamate = 0"
    );
}

// ---------------------------------------------------------------------------
// Item 6: the four design answers
// ---------------------------------------------------------------------------

/// A script's insert between a run and its undo: `hello`, then a Lua
/// insert of `X` outside any command (a daemon edit, nobody's), then
/// undo twice. The script's insert is the newest thing, so the first
/// undo takes it; the second takes `hello`. Chronological, as the
/// single global stack had it, in both histories.
#[test]
fn review_script_insert_then_undo_takes_the_script_first_across_histories() {
    let (plain, crdt) = both(
        |s| {
            type_str(s, "hello");
            exec(s, "local b = pmacs.window.buffer() b:insert(b:len(), 'X')");
        },
        &[undo, undo, undo],
    );
    assert_eq!(plain[0], "helloX", "the script's insert landed");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "hello", "one undo: the script's insert, newest");
    assert_eq!(plain[2], "", "two undos: hello");
    assert_eq!(plain[3], "", "three undos: nothing left");
}

/// Two runs, undone and redone in order: `ab`, `End`, `cd`; undo, undo,
/// redo, redo walks `ab`, empty, `ab`, `abcd`, and a fifth step finds
/// nothing to redo. The compensations never became groups: the second
/// undo popped the user's first run, not the first undo's compensation.
#[test]
fn review_undo_undo_redo_redo_walks_two_runs_across_histories() {
    let (plain, crdt) = both(
        |s| {
            type_str(s, "ab");
            press(s, KeyCode::End);
            type_str(s, "cd");
        },
        &[undo, undo, redo, redo, redo],
    );
    assert_eq!(plain[0], "abcd");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "ab", "one undo: the second run");
    assert_eq!(
        plain[2], "",
        "two undos: the first run, not the compensation"
    );
    assert_eq!(plain[3], "ab", "one redo: the first run back");
    assert_eq!(plain[4], "abcd", "two redos: the second run back");
    assert_eq!(plain[5], "abcd", "three redos: nothing left to redo");
}

/// Undo, type, redo: `ab`, undo, `c`, redo. The forward edit cleared
/// the redo, so the redo restores nothing and `c` stays; a further undo
/// removes `c`, and one more finds nothing --- the undone `ab` is not
/// reachable again, in either history.
#[test]
fn review_undo_type_redo_restores_nothing_across_histories() {
    let (plain, crdt) = both(
        |s| {
            type_str(s, "ab");
            undo(s);
            type_str(s, "c");
        },
        &[redo, undo, undo],
    );
    assert_eq!(plain[0], "c", "typed after the undo");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "c", "redo after a forward edit restores nothing");
    assert_eq!(plain[2], "", "undo removes c");
    assert_eq!(plain[3], "", "the undone ab is gone for good");
}

#[path = "common/iso.rs"]
mod iso;
