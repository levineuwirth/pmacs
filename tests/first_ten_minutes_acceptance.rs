// tests/first_ten_minutes_acceptance.rs --- E1 acceptance ("first ten
// minutes").

//! Acceptance for the table-stakes editing gestures E1 adds: the
//! unsaved-buffer prompts, `C-g` clearing a selection, the mark and
//! buffer-wide motion, word kills into the ring, the buffer commands,
//! the defaults, and the bound orphans.
//!
//! Dispatch-driven wherever a binding exists: a command invoked by name
//! passes with its keymap entry deleted, which is exactly the defect
//! half of these rows exist to close. Where a command has no chord by
//! design it is reached through `M-x`, typed, the same way a user
//! reaches it.
//!
//! Fixtures use `.txt` files so no `buffer.after-load` hook spawns a
//! language server.

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

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
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

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn prompt(s: &EditorState) -> String {
    eval::<Option<String>>(s, "return pmacs.minibuffer.prompt()").unwrap_or_default()
}

fn buffer_names(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local out = {}
         for _, id in ipairs(pmacs.buffer.list()) do
           out[#out + 1] = pmacs.describe.buffer(id).name
         end
         return out",
    )
}

/// An editor with isolated roots and no developer `init.lua`.
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    s
}

/// Run `name` the way a user does: `M-x`, typed, RET. The selection is
/// asserted before RET — a candidate shadows typed text, so accepting
/// blind can run a different command entirely.
fn m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    assert!(
        eval::<bool>(s, "return pmacs.minibuffer.is_active()"),
        "M-x must open the command palette"
    );
    type_str(s, name);
    assert_eq!(
        eval::<Option<String>>(s, "return pmacs.minibuffer.selected()").as_deref(),
        Some(name),
        "the completion source must have {name} selected"
    );
    press(s, KeyCode::Enter);
}

/// A `.txt` buffer holding `text`, visited from disk and left active.
fn visit(s: &EditorState, dir: &std::path::Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write fixture");
    let display = path.display().to_string();
    exec(s, &format!("pmacs.buffer.find_or_open({display:?})"));
    display
}

// ---------------------------------------------------------------------------
// E1.1 --- unsaved-buffer prompts
// ---------------------------------------------------------------------------

/// `buffer.kill-this` on a modified buffer asks, names the buffer, and
/// keeps it when the answer is no.
#[test]
fn killing_a_modified_buffer_asks_and_a_refusal_keeps_it() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");
    assert!(
        eval::<bool>(
            &s,
            "return pmacs.describe.buffer(pmacs.window.buffer()).modified"
        ),
        "precondition: the target must be modified or nothing is owed"
    );

    m_x(&mut s, "buffer.kill-this");
    assert!(
        eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "killing unsaved work must ask; status was {:?}",
        status(&s)
    );
    let asked = prompt(&s);
    assert!(
        asked.contains(&path),
        "the question must name the buffer at stake; got {asked:?}"
    );

    type_str(&mut s, "n");
    press(&mut s, KeyCode::Enter);
    assert!(
        buffer_names(&s).contains(&path),
        "`n` must keep the buffer; buffers are {:?}",
        buffer_names(&s)
    );
}

/// And `y` kills it. Separate from the refusal row: one prompt that
/// always killed would satisfy the other half alone.
#[test]
fn killing_a_modified_buffer_proceeds_on_yes() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");

    m_x(&mut s, "buffer.kill-this");
    type_str(&mut s, "y");
    press(&mut s, KeyCode::Enter);
    assert!(
        !buffer_names(&s).contains(&path),
        "`y` must kill the buffer; buffers are {:?}",
        buffer_names(&s)
    );
}

/// An unmodified buffer is killed without a question. The prompt is a
/// guard on loss, not a toll on every kill.
#[test]
fn killing_an_unmodified_buffer_asks_nothing() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = visit(&s, td.path(), "alpha.txt", "alpha\n");

    m_x(&mut s, "buffer.kill-this");
    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "an unmodified kill must not prompt"
    );
    assert!(
        !buffer_names(&s).contains(&path),
        "and it must have happened; buffers are {:?}",
        buffer_names(&s)
    );
}

/// An answer that is neither yes nor no re-asks rather than being read
/// as either. Guessing is safe one way and destructive the other.
#[test]
fn an_unrecognized_answer_re_asks() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");

    m_x(&mut s, "buffer.kill-this");
    type_str(&mut s, "maybe");
    press(&mut s, KeyCode::Enter);
    assert!(
        eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "an unrecognized answer must leave the question standing"
    );
    assert!(
        buffer_names(&s).contains(&path),
        "and must not have killed anything"
    );
    assert_eq!(status(&s), "please answer y or n");
}

/// `C-g` at the question is a refusal, not an answer: the buffer lives.
#[test]
fn cancelling_the_question_is_a_refusal() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");

    m_x(&mut s, "buffer.kill-this");
    ctrl(&mut s, 'g');
    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "C-g must close the question"
    );
    assert!(
        buffer_names(&s).contains(&path),
        "and must not kill; buffers are {:?}",
        buffer_names(&s)
    );
}

/// The quit prompt names every modified buffer, not just the active
/// one: a user who cannot see what is dirty cannot answer.
#[test]
fn the_quit_question_names_the_modified_buffers() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let alpha = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");
    let beta = visit(&s, td.path(), "beta.txt", "beta\n");
    type_str(&mut s, "Y");

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'c');
    let asked = prompt(&s);
    assert!(
        asked.contains(&alpha) && asked.contains(&beta),
        "the question must name both modified buffers; got {asked:?}"
    );
    assert!(!s.core.borrow().quit, "and must not have quit");
}

// ---------------------------------------------------------------------------
// E1.2 --- `C-g` clears the selection
// ---------------------------------------------------------------------------

/// `C-g` drops a live selection. Driven through the real chord and a
/// real Shift-motion selection, because the defect was that the one key
/// every editor uses to get out of a state did not get out of this one.
#[test]
fn c_g_clears_a_live_selection() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.goto_byte(0)");
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Right, KeyModifiers::SHIFT));
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Right, KeyModifiers::SHIFT));
    assert!(
        eval::<bool>(&s, "return pmacs.editor.region() ~= nil"),
        "precondition: a selection must be live or the row is vacuous"
    );

    ctrl(&mut s, 'g');
    assert!(
        eval::<bool>(&s, "return pmacs.editor.region() == nil"),
        "C-g must clear the selection"
    );
    assert_eq!(
        status(&s),
        "Quit",
        "and must keep saying so on the status line"
    );
}

/// `C-g` leaves the cursor where it was. Clearing a selection is not a
/// motion, and a `C-g` that also jumped would be a different defect.
#[test]
fn clearing_the_selection_does_not_move_the_cursor() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.goto_byte(0)");
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Right, KeyModifiers::SHIFT));
    let before: i64 = eval(&s, "return pmacs.editor.cursor()");

    ctrl(&mut s, 'g');
    assert_eq!(
        eval::<i64>(&s, "return pmacs.editor.cursor()"),
        before,
        "C-g must not move point"
    );
}

// ---------------------------------------------------------------------------
// E1.3 --- mark, buffer-wide motion, recenter
// ---------------------------------------------------------------------------

/// `M->` lands on the buffer's last byte and `M-<` on its first, both
/// through the real chords. `M->` is the half that cannot be written as
/// a line jump: on a file with no trailing newline it is the end of the
/// last line, not its start.
#[test]
fn buffer_start_and_end_motion_reach_both_ends() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "one\ntwo\nthree");
    let len: i64 = eval(&s, "return pmacs.window.buffer():len()");

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('>'), KeyModifiers::ALT),
    );
    assert_eq!(
        eval::<i64>(&s, "return pmacs.editor.cursor()"),
        len,
        "M-> must reach the last byte, not the last line's start"
    );

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('<'), KeyModifiers::ALT),
    );
    assert_eq!(
        eval::<i64>(&s, "return pmacs.editor.cursor()"),
        0,
        "M-< must reach the first byte"
    );
}

/// `C-SPC` sets the mark and plain motion then extends the region ---
/// the whole point of a mark, and the reason it is the same anchor the
/// Shift-motion commands use rather than a second one.
#[test]
fn the_mark_makes_plain_motion_extend_a_region() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.goto_byte(0)");
    assert!(
        eval::<bool>(&s, "return pmacs.editor.region() == nil"),
        "precondition: no region before the mark is set"
    );

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(' '), KeyModifiers::CONTROL),
    );
    assert_eq!(status(&s), "Mark set");
    assert!(
        eval::<bool>(&s, "return pmacs.editor.region() == nil"),
        "a mark at point is an empty region, which reports as none"
    );

    ctrl(&mut s, 'f');
    ctrl(&mut s, 'f');
    ctrl(&mut s, 'f');
    let region: Vec<i64> = eval(
        &s,
        "local r = pmacs.editor.region() return { r.start, r['end'] }",
    );
    assert_eq!(
        region,
        vec![0, 3],
        "plain motion after C-SPC must extend the region"
    );
}

/// `C-x C-x` swaps the ends and keeps the region. Both halves matter:
/// an exchange that dropped the mark would leave the user with a
/// cursor where their region used to be.
#[test]
fn exchange_point_and_mark_swaps_the_ends_and_keeps_the_region() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.goto_byte(0)");
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(' '), KeyModifiers::CONTROL),
    );
    ctrl(&mut s, 'f');
    ctrl(&mut s, 'f');
    ctrl(&mut s, 'f');
    assert_eq!(eval::<i64>(&s, "return pmacs.editor.cursor()"), 3);

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'x');
    assert_eq!(
        eval::<i64>(&s, "return pmacs.editor.cursor()"),
        0,
        "point must land on the mark"
    );
    let region: Vec<i64> = eval(
        &s,
        "local r = pmacs.editor.region() return { r.start, r['end'] }",
    );
    assert_eq!(region, vec![0, 3], "and the region must survive the swap");

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'x');
    assert_eq!(
        eval::<i64>(&s, "return pmacs.editor.cursor()"),
        3,
        "and the swap must be its own inverse"
    );
}

/// `C-l` centers the line holding point over the window's `view_top`
/// and does not move point. The viewport height is set explicitly:
/// with no frame rendered the fallback would decide the answer, and a
/// row that cannot tell the two apart pins nothing.
#[test]
fn recenter_centers_the_cursor_line_without_moving_point() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let mut body = String::new();
    for n in 0..100 {
        use std::fmt::Write as _;
        writeln!(body, "line {n}").expect("format fixture");
    }
    visit(&s, td.path(), "alpha.txt", &body);
    s.core.borrow_mut().active_window_mut().last_visible_rows = 10;
    exec(&s, "pmacs.editor.move_to_line(50)");
    s.core.borrow_mut().active_window_mut().view_top = 0;
    let point: i64 = eval(&s, "return pmacs.editor.cursor()");

    ctrl(&mut s, 'l');
    assert_eq!(
        s.core.borrow().view_top(),
        45,
        "C-l must put the cursor line mid-viewport"
    );
    assert_eq!(
        eval::<i64>(&s, "return pmacs.editor.cursor()"),
        point,
        "and must not move point"
    );
}

// ---------------------------------------------------------------------------
// E1.4 --- word kills reach the kill ring
// ---------------------------------------------------------------------------

/// The ring's head text, or the empty string when the ring is empty.
fn ring_head(s: &EditorState) -> String {
    eval(s, "local r = pmacs.killring.list() return r[1] or ''")
}

/// `M-d` kills forward into the ring, and `C-y` brings it back. The
/// yank is the half that matters: a delete that merely reported a kill
/// would pass any assertion about the buffer alone.
#[test]
fn m_d_kills_the_word_forward_into_the_ring() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.goto_byte(0)");

    alt(&mut s, 'd');
    assert_eq!(
        eval::<String>(
            &s,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())"
        ),
        " beta\n",
        "M-d must delete the word"
    );
    assert_eq!(ring_head(&s), "alpha", "and it must be on the ring");

    ctrl(&mut s, 'y');
    assert_eq!(
        eval::<String>(
            &s,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())"
        ),
        "alpha beta\n",
        "C-y must bring it back"
    );
}

/// `M-BS` kills backward into the ring.
#[test]
fn m_backspace_kills_the_word_backward_into_the_ring() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.move_buffer_end()");
    exec(&s, "pmacs.editor.goto_byte(10)");

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Backspace, KeyModifiers::ALT),
    );
    assert_eq!(
        eval::<String>(
            &s,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())"
        ),
        "alpha \n",
        "M-BS must delete the previous word"
    );
    assert_eq!(ring_head(&s), "beta", "and it must be on the ring");
}

/// Consecutive backward kills PREPEND. Appending would yank back the
/// words in the reverse of the order they stood in the buffer, which is
/// the whole reason the direction is carried into the ring at all.
#[test]
fn chained_backward_kills_prepend_rather_than_append() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.goto_byte(10)");

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Backspace, KeyModifiers::ALT),
    );
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Backspace, KeyModifiers::ALT),
    );
    assert_eq!(
        ring_head(&s),
        "alpha beta",
        "two backward kills must yank back as they stood in the buffer"
    );
    assert_eq!(
        eval::<String>(
            &s,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())"
        ),
        "\n",
        "and both words must be gone"
    );
}

/// Consecutive forward kills still append, unchanged.
#[test]
fn chained_forward_kills_still_append() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha beta\n");
    exec(&s, "pmacs.editor.goto_byte(0)");

    alt(&mut s, 'd');
    alt(&mut s, 'd');
    assert_eq!(
        ring_head(&s),
        "alpha beta",
        "two forward kills must accumulate in buffer order"
    );
}

/// A word kill at the end of the buffer is a no-op that reports, and it
/// must not leave a chain the next kill would append to.
#[test]
fn a_word_kill_with_nothing_to_kill_reports_and_breaks_the_chain() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha");
    exec(&s, "pmacs.editor.goto_byte(0)");
    alt(&mut s, 'd');
    assert_eq!(ring_head(&s), "alpha");

    alt(&mut s, 'd');
    assert_eq!(status(&s), "end of buffer");
    assert_eq!(
        ring_head(&s),
        "alpha",
        "a failed kill must add nothing to the ring"
    );
    assert!(
        !eval::<bool>(
            &s,
            "return pmacs.killring._debug_state(pmacs.frontend.id()).last_kill_id ~= nil"
        ),
        "and must leave no chain for the next kill to ride"
    );
}

// ---------------------------------------------------------------------------
// E1.5 --- buffer commands
// ---------------------------------------------------------------------------

/// `C-x k` prefills the buffer in the window, so RET is the common
/// case, and the unsaved check is the same one `buffer.kill-this` runs.
#[test]
fn c_x_k_prefills_the_current_buffer_and_kills_it_on_return() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha\n");
    let beta = visit(&s, td.path(), "beta.txt", "beta\n");

    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('k'));
    assert!(
        eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "C-x k must open a prompt"
    );
    assert_eq!(
        eval::<String>(&s, "return pmacs.minibuffer.contents()"),
        beta,
        "the prompt must be prefilled with the buffer in the window"
    );

    press(&mut s, KeyCode::Enter);
    assert!(
        !buffer_names(&s).contains(&beta),
        "RET on the prefill must kill it; buffers are {:?}",
        buffer_names(&s)
    );
}

/// And it inherits the unsaved-work question rather than having its own.
#[test]
fn c_x_k_asks_when_the_named_buffer_is_modified() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha\n");
    let beta = visit(&s, td.path(), "beta.txt", "beta\n");
    type_str(&mut s, "X");

    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('k'));
    press(&mut s, KeyCode::Enter);
    let asked = prompt(&s);
    assert!(
        asked.contains(&beta) && asked.contains("kill anyway"),
        "the kill must ask about unsaved work; got {asked:?}"
    );
    assert!(buffer_names(&s).contains(&beta), "and must not have killed");
}

/// `C-x C-w` writes to a new path and the buffer ADOPTS it: the bytes
/// land on disk, the buffer is clean, and a later `C-x C-s` saves to the
/// new file rather than the old one.
#[test]
fn c_x_c_w_writes_to_a_new_path_and_adopts_it() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let alpha = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    assert!(
        eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "C-x C-w must open a prompt"
    );
    type_str(&mut s, "zeta-out.txt");
    press(&mut s, KeyCode::Enter);

    let target = td.path().join("zeta-out.txt");
    assert_eq!(
        std::fs::read_to_string(&target).expect("the written file must exist"),
        "Xalpha\n",
        "the buffer's bytes must be what landed"
    );
    assert_eq!(
        std::fs::read_to_string(&alpha).expect("the original must still be there"),
        "alpha\n",
        "and the original file must be untouched"
    );
    assert_eq!(
        eval::<Option<String>>(&s, "return pmacs.editor.file_path()").as_deref(),
        Some(target.display().to_string().as_str()),
        "the buffer must adopt the new path"
    );
    assert!(
        !eval::<bool>(
            &s,
            "return pmacs.describe.buffer(pmacs.window.buffer()).modified"
        ),
        "and must be clean afterwards"
    );

    type_str(&mut s, "Y");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 's');
    assert_eq!(
        std::fs::read_to_string(&target).expect("read back"),
        "XYalpha\n",
        "a later C-x C-s must save to the adopted path"
    );
}

/// An existing file at the destination is a question. The buffer has
/// never read that file, so nothing in the editor knows what is in it.
#[test]
fn c_x_c_w_asks_before_it_overwrites_and_a_refusal_keeps_the_file() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha\n");
    let occupied = td.path().join("occupied.txt");
    std::fs::write(&occupied, b"do not lose me\n").expect("write occupant");

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    type_str(&mut s, "occupied.txt");
    press(&mut s, KeyCode::Enter);
    let asked = prompt(&s);
    assert!(
        asked.contains("exists") && asked.contains("overwrite"),
        "an existing destination must be a question; got {asked:?}"
    );

    type_str(&mut s, "n");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        std::fs::read_to_string(&occupied).expect("read back"),
        "do not lose me\n",
        "`n` must leave the file alone"
    );
}

/// And `y` overwrites it.
#[test]
fn c_x_c_w_overwrites_on_yes() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha\n");
    let occupied = td.path().join("occupied.txt");
    std::fs::write(&occupied, b"old\n").expect("write occupant");

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    type_str(&mut s, "occupied.txt");
    press(&mut s, KeyCode::Enter);
    type_str(&mut s, "y");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        std::fs::read_to_string(&occupied).expect("read back"),
        "alpha\n",
        "`y` must overwrite"
    );
}

/// `revert-buffer` asks before discarding unsaved edits, and reloads
/// what is on disk when the answer is yes.
#[test]
fn revert_buffer_asks_then_reloads_from_disk() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let alpha = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");
    std::fs::write(&alpha, b"changed underneath\n").expect("rewrite");

    m_x(&mut s, "revert-buffer");
    let asked = prompt(&s);
    assert!(
        asked.contains("Discard unsaved changes"),
        "a modified revert must ask; got {asked:?}"
    );

    type_str(&mut s, "y");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        eval::<String>(
            &s,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())"
        ),
        "changed underneath\n",
        "the buffer must hold what is on disk"
    );
    assert!(
        !eval::<bool>(
            &s,
            "return pmacs.describe.buffer(pmacs.window.buffer()).modified"
        ),
        "and must be clean"
    );
}

/// A clean buffer reverts without a question, and point survives inside
/// the new extent rather than dangling past it.
#[test]
fn revert_buffer_on_a_clean_buffer_asks_nothing_and_clamps_point() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let alpha = visit(&s, td.path(), "alpha.txt", "a long first line\n");
    exec(&s, "pmacs.editor.move_buffer_end()");
    let before: i64 = eval(&s, "return pmacs.editor.cursor()");
    assert!(
        before > 3,
        "precondition: point must be past the new length"
    );
    std::fs::write(&alpha, b"ab\n").expect("rewrite shorter");

    m_x(&mut s, "revert-buffer");
    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "a clean revert must not ask"
    );
    assert_eq!(
        eval::<String>(
            &s,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())"
        ),
        "ab\n"
    );
    let len: i64 = eval(&s, "return pmacs.window.buffer():len()");
    assert!(
        eval::<i64>(&s, "return pmacs.editor.cursor()") <= len,
        "point must be clamped into the reloaded buffer"
    );
}

// Isolated bootstrap storage roots: an integration test is compiled
// without `cfg(test)`, so a raw `EditorState::new()` would read the
// developer's real `init.lua` and write into their real data root.
#[path = "common/iso.rs"]
mod iso;
