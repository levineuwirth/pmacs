//! CUA region semantics — Backspace / Delete consume the active
//! selection (set by Shift+motion) before falling back to their
//! single-codepoint behavior.
//!
//! Regression for the pmacs-gpu report "select a region with
//! shift+arrows then backspace doesn't delete as expected": the
//! `buffer.delete-backward` / `buffer.delete-forward` commands called
//! straight into the single-codepoint core primitives and never
//! consulted `active_region()`. The behavior is frontend-agnostic
//! (the GPU round-trips BS through the same dispatch), so the TUI
//! dispatch path exercised here covers both.

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

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

/// `(buffer text, region active?, cursor)` probed through the Lua
/// surface — the same introspection a user-facing script would use.
fn probe(s: &EditorState) -> (String, bool, i64) {
    s.lua_host
        .lua()
        .load(
            "
            local b = pmacs.window.buffer()
            local text = b:slice(0, b:len())
            return text, pmacs.editor.region() ~= nil, pmacs.editor.cursor()
            ",
        )
        .eval()
        .expect("probe buffer state")
}

#[test]
fn backspace_deletes_the_shift_selected_region() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello");

    // Shift+Left three times: region [2, 5), cursor at 2.
    for _ in 0..3 {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::SHIFT));
    }
    let (_, region_active, _) = probe(&s);
    assert!(region_active, "shift+arrows must leave an active region");

    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Backspace, KeyModifiers::NONE));

    let (text, region_active, cursor) = probe(&s);
    assert_eq!(text, "he", "backspace must delete the whole region");
    assert!(!region_active, "the region clears with its deletion");
    assert_eq!(cursor, 2, "cursor lands at the deleted region's start");

    // Without a region, backspace keeps single-codepoint semantics.
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Backspace, KeyModifiers::NONE));
    let (text, _, cursor) = probe(&s);
    assert_eq!(text, "h", "no region ⇒ plain single-codepoint backspace");
    assert_eq!(cursor, 1);
}

/// C-Backspace deletes the previous word (and C-Delete the next),
/// mirroring C-arrow word motion. The pmacs-gpu frontend forwards
/// chorded deletion keys to this same dispatch path.
#[test]
fn ctrl_backspace_deletes_the_previous_word() {
    let mut s = EditorState::new();
    type_str(&mut s, "alpha beta");

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Backspace, KeyModifiers::CONTROL),
    );
    let (text, _, cursor) = probe(&s);
    assert_eq!(text, "alpha ", "C-BS deletes back through the previous word");
    assert_eq!(cursor, 6);

    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::CONTROL));
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Delete, KeyModifiers::CONTROL),
    );
    let (text, _, cursor) = probe(&s);
    assert_eq!(text, " ", "C-DEL deletes forward through the next word");
    assert_eq!(cursor, 0);
}

#[test]
fn typing_replaces_the_shift_selected_region() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello");

    // Select "llo" (region [2, 5), cursor at 2), then type 'X':
    // CUA type-over replaces the region with the typed char.
    for _ in 0..3 {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::SHIFT));
    }
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('X'), KeyModifiers::SHIFT),
    );

    let (text, region_active, cursor) = probe(&s);
    assert_eq!(text, "heX", "typing must replace the selected region");
    assert!(!region_active, "the region is consumed by the replacement");
    assert_eq!(cursor, 3, "cursor sits after the typed char");

    // Enter over a selection replaces it with a newline.
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::SHIFT));
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Enter, KeyModifiers::NONE));
    let (text, region_active, cursor) = probe(&s);
    assert_eq!(text, "he\n", "Enter must replace the selected region");
    assert!(!region_active);
    assert_eq!(cursor, 3);

    // Without a selection, typing keeps plain insert semantics.
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('z'), KeyModifiers::NONE),
    );
    let (text, _, cursor) = probe(&s);
    assert_eq!(text, "he\nz", "no region ⇒ plain insert at the cursor");
    assert_eq!(cursor, 4);
}

#[test]
fn delete_forward_deletes_the_shift_selected_region() {
    let mut s = EditorState::new();
    type_str(&mut s, "world");

    // Shift+Home-equivalent: extend left over the whole word.
    for _ in 0..5 {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::SHIFT));
    }
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Delete, KeyModifiers::NONE));

    let (text, region_active, cursor) = probe(&s);
    assert_eq!(text, "", "Delete must consume the whole region");
    assert!(!region_active);
    assert_eq!(cursor, 0);

    // Without a region, Delete keeps forward single-codepoint semantics.
    type_str(&mut s, "ab");
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::NONE));
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::NONE));
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Delete, KeyModifiers::NONE));
    let (text, _, cursor) = probe(&s);
    assert_eq!(text, "b", "no region ⇒ plain forward delete at cursor");
    assert_eq!(cursor, 0);
}
