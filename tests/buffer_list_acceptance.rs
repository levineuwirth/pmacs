//! E5.5 --- `*buffer-list*` on listview.
//!
//! The classic buffer menu was a plain writable buffer rendered by
//! delete-all + insert with a hand-rolled keymap. It is now a listview
//! panel opened in place: RET visits, `d`/`u` mark and unmark through
//! the `keys` extension, `x` kills the marked, `k` kills the row's
//! buffer, `g` refreshes and `q` returns --- and the listing is not
//! writable, so a stray keystroke cannot edit it.
//!
//! Every gesture here is a keystroke through `dispatch_key`, so a dead
//! binding fails the row rather than passing through `invoke`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn editor() -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    state
}

fn press(state: &mut EditorState, code: KeyCode, mods: KeyModifiers) {
    state.dispatch_key(FrontendId::LOCAL, KeyEvent::new(code, mods));
}

fn key(state: &mut EditorState, c: char) {
    press(state, KeyCode::Char(c), KeyModifiers::NONE);
}

fn open_list(state: &mut EditorState) {
    press(state, KeyCode::Char('x'), KeyModifiers::CONTROL);
    press(state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let name: String = eval(state, "return pmacs.window.buffer():name()");
    assert_eq!(name, "*buffer-list*", "C-x C-b opens the listing in place");
}

fn active_name(state: &EditorState) -> String {
    eval(state, "return pmacs.window.buffer():name()")
}

fn list_text(state: &EditorState) -> String {
    eval(
        state,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    )
}

fn buffer_names(state: &EditorState) -> Vec<String> {
    eval(
        state,
        r"
        local out = {}
        for _, id in ipairs(pmacs.buffer.list()) do out[#out + 1] = pmacs.describe.buffer(id).name end
        table.sort(out)
        return out
        ",
    )
}

#[test]
fn the_listing_is_a_read_only_listview_that_visits_on_ret_and_returns_on_q() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.buffer.create('alpha'); pmacs.buffer.create('beta')",
    );
    open_list(&mut state);
    let text = list_text(&state);
    assert!(
        text.contains("alpha") && text.contains("beta"),
        "rows list every buffer: {text:?}"
    );
    assert!(
        text.contains("RET visit"),
        "the header names the keys: {text:?}"
    );

    // Not writable: a typed character does not land in the listing.
    key(&mut state, 'z');
    assert_eq!(
        list_text(&state),
        text,
        "a keystroke cannot edit the listing"
    );
    let writable: bool = eval(
        &state,
        "return (pcall(function() pmacs.window.buffer():insert(0, 'x') end))",
    );
    assert!(!writable, "the rope is locked between renders");

    // RET visits the row under the cursor (the first data row).
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    let visited = active_name(&state);
    assert_ne!(visited, "*buffer-list*", "RET switched to the row's buffer");

    // q from the listing returns to where it was opened from.
    exec(
        &state,
        "pmacs.window.switch_buffer(pmacs.buffer.create('gamma'))",
    );
    open_list(&mut state);
    key(&mut state, 'q');
    assert_eq!(
        active_name(&state),
        "gamma",
        "q returns to the origin buffer"
    );
}

#[test]
fn marks_render_and_x_kills_exactly_the_marked_buffers() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.buffer.create('doomed-1'); pmacs.buffer.create('doomed-2'); pmacs.buffer.create('kept')",
    );
    open_list(&mut state);
    // Walk to each doomed row and mark it; `d` moves down after marking.
    let rows: Vec<String> = list_text(&state).lines().map(str::to_owned).collect();
    let line_of = |needle: &str| rows.iter().position(|l| l.contains(needle)).expect(needle);
    let (d1, d2) = (line_of("doomed-1"), line_of("doomed-2"));
    // Seat on row d1 (line indices are 0-based with the header at 0;
    // the cursor opens on data line 1).
    let cursor: i64 = eval(&state, "return pmacs.editor.cursor_line()");
    assert_eq!(cursor, 1);
    for _ in 1..d1 {
        key(&mut state, 'n');
    }
    key(&mut state, 'd');
    let after_first = list_text(&state);
    assert!(
        after_first
            .lines()
            .nth(d1)
            .is_some_and(|l| l.starts_with('D')),
        "a D renders in column 0 of the marked row: {after_first:?}"
    );
    // `d` moved down one; walk to d2 from d1 + 1.
    for _ in (d1 + 1)..d2 {
        key(&mut state, 'n');
    }
    key(&mut state, 'd');
    // Unmark the second again, then re-mark it: u clears the D.
    key(&mut state, 'p');
    key(&mut state, 'u');
    assert!(
        !list_text(&state)
            .lines()
            .nth(d2)
            .is_some_and(|l| l.starts_with('D')),
        "u clears the mark"
    );
    key(&mut state, 'p');
    key(&mut state, 'd');
    key(&mut state, 'x');
    let names = buffer_names(&state);
    assert!(
        !names.iter().any(|n| n == "doomed-1" || n == "doomed-2"),
        "x killed the marked buffers: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "kept"),
        "and not the unmarked: {names:?}"
    );
    assert_eq!(
        active_name(&state),
        "*buffer-list*",
        "the listing stays up after x"
    );
    assert!(
        !list_text(&state).contains("doomed"),
        "and re-rendered without them"
    );
}

#[test]
fn k_kills_the_rows_buffer_and_refuses_the_listing_itself() {
    let mut state = editor();
    exec(&state, "pmacs.buffer.create('victim')");
    open_list(&mut state);
    let rows: Vec<String> = list_text(&state).lines().map(str::to_owned).collect();
    let victim = rows
        .iter()
        .position(|l| l.contains("victim"))
        .expect("victim row");
    for _ in 1..victim {
        key(&mut state, 'n');
    }
    key(&mut state, 'k');
    assert!(
        !buffer_names(&state).iter().any(|n| n == "victim"),
        "k killed the row's buffer"
    );
    assert!(
        !list_text(&state).contains("victim"),
        "and the listing refreshed"
    );

    // k on the listing's own row is refused.
    let rows: Vec<String> = list_text(&state).lines().map(str::to_owned).collect();
    let own = rows
        .iter()
        .position(|l| l.contains("*buffer-list*"))
        .expect("own row");
    exec(&state, "pmacs.editor.move_to_line(0)");
    for _ in 0..own {
        key(&mut state, 'n');
    }
    key(&mut state, 'k');
    assert_eq!(
        active_name(&state),
        "*buffer-list*",
        "the listing survives k on itself"
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("can't kill *buffer-list*"),
        "and says why: {status:?}"
    );
}
