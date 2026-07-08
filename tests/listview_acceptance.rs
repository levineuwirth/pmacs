//! List-panel acceptance (Arc 1b phase 1) --- the `pmacs.listview`
//! module end-to-end through `dispatch_key`: open/navigate/visit,
//! `q` restore, the Q#P3 read-only intercept, the Q#P6 round-trip
//! gate (`dispatch_idle` false while a panel is focused), and
//! refresh. The references panel itself needs a live LSP and is
//! validated manually / via the m4 harness; these tests drive the
//! substrate hermetically.
//!
//! Framing: docs/lsp-panels-framing.md.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

/// Open a three-row test panel whose visits record into `_G.VISITED`.
fn open_test_panel(s: &mut EditorState) {
    s.lua_host
        .lua()
        .load(
            r#"
            _G.VISITED = nil
            pmacs.listview.open {
              name = "*test-panel*",
              header = "3 items   RET visit  q quit",
              rows = {
                { text = "alpha", item = "A" },
                { text = "beta",  item = "B" },
                { text = "gamma", item = "C" },
              },
              on_visit = function(item) _G.VISITED = item end,
              on_refresh = function()
                return { { text = "delta", item = "D" } }
              end,
            }
            "#,
        )
        .exec()
        .expect("open test panel");
}

/// `(active buffer name, buffer text, cursor line, visited)` probed
/// through the Lua surface.
fn probe(s: &EditorState) -> (String, String, i64, Option<String>) {
    s.lua_host
        .lua()
        .load(
            r"
            local b = pmacs.window.buffer()
            local d = pmacs.describe.buffer(b)
            return d.name, b:slice(0, b:len()), pmacs.editor.cursor_line(), _G.VISITED
            ",
        )
        .eval()
        .expect("probe panel state")
}

#[test]
fn open_seats_cursor_and_ret_visits_the_row() {
    let mut s = EditorState::new();
    open_test_panel(&mut s);
    let (name, text, line, _) = probe(&s);
    assert_eq!(name, "*test-panel*");
    assert!(text.starts_with("3 items"), "header renders first");
    assert_eq!(line, 1, "the cursor opens on the first data row");

    press(&mut s, KeyCode::Char('n')); // buffer-local: cursor.down
    press(&mut s, KeyCode::Enter);
    let (_, _, _, visited) = probe(&s);
    assert_eq!(visited.as_deref(), Some("B"), "RET visits the second row");
}

#[test]
fn header_row_is_not_visitable() {
    let mut s = EditorState::new();
    open_test_panel(&mut s);
    press(&mut s, KeyCode::Char('p')); // up onto the header
    press(&mut s, KeyCode::Enter);
    let (_, _, _, visited) = probe(&s);
    assert_eq!(visited, None, "the header maps to no item");
}

#[test]
fn q_restores_the_previous_buffer() {
    let mut s = EditorState::new();
    open_test_panel(&mut s);
    press(&mut s, KeyCode::Char('q'));
    let (name, _, _, _) = probe(&s);
    assert_eq!(name, "*scratch*", "q returns to the buffer we came from");
}

#[test]
fn panel_rejects_typing() {
    let mut s = EditorState::new();
    open_test_panel(&mut s);
    let (_, before, _, _) = probe(&s);
    press(&mut s, KeyCode::Char('z')); // unbound printable → self-insert → intercept rejects
    let (_, after, _, _) = probe(&s);
    assert_eq!(before, after, "the read-only intercept rejects self-insert");
}

#[test]
fn dispatch_idle_is_false_while_a_panel_is_focused() {
    // Q#P6: while the panel is the active buffer, semantic frontends
    // must round-trip every key (RET = visit, not an optimistic \n).
    let mut s = EditorState::new();
    assert!(s.dispatch_idle(), "scratch buffer: idle");
    open_test_panel(&mut s);
    assert!(!s.dispatch_idle(), "panel focused: keys must round-trip");
    press(&mut s, KeyCode::Char('q'));
    assert!(s.dispatch_idle(), "restored buffer: idle again");
}

#[test]
fn refresh_reruns_the_source_and_reseats() {
    let mut s = EditorState::new();
    open_test_panel(&mut s);
    press(&mut s, KeyCode::Char('g'));
    let (_, text, line, _) = probe(&s);
    assert!(text.contains("delta"), "g re-renders from on_refresh");
    assert!(!text.contains("alpha"), "old rows are gone");
    assert_eq!(line, 1, "cursor re-seats on a data row after refresh");
    press(&mut s, KeyCode::Enter);
    let (_, _, _, visited) = probe(&s);
    assert_eq!(visited.as_deref(), Some("D"), "the refreshed row visits");
}
