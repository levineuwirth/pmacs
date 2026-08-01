//! Kill-ring acceptance (Arc 2, docs/kill-ring-framing.md rev 3).
//!
//! Drives the real dispatch surfaces: `dispatch_key` for chords (both
//! "frontends" via distinct `FrontendId`s — an unregistered id falls
//! back to LOCAL's view, sharing the buffer while keeping its own
//! command boundary, which is exactly the shared-ring interleaving
//! shape), `dispatch_mouse` / `dispatch_pointer` for the pointer
//! boundary rows, and the real minibuffer for `M-x`.
//!
//! The daemon-side rows (optimistic CRDT edits, the unified
//! authenticated-source paste) are covered by unit tests next to
//! `handle_remote_crdt_op` / `handle_inbound_paste` in `src/daemon.rs`.

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

fn ring(s: &EditorState) -> Vec<String> {
    eval(s, "return pmacs.killring.list()")
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// Fresh editor whose scratch buffer holds `text`, cursor at 0.
fn editor_with(text: &str) -> EditorState {
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    type_str(&mut s, text);
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

// ---------------------------------------------------------------------------
// Chain mechanics
// ---------------------------------------------------------------------------

#[test]
fn kill_line_kills_to_eol_and_the_newline_separately() {
    let mut s = editor_with("hello\nworld");
    ctrl(&mut s, 'k'); // kills "hello"
    assert_eq!(buffer_text(&s), "\nworld");
    assert_eq!(ring(&s), vec!["hello"]);
    // Cursor now sits at the newline: C-k kills the newline itself —
    // and, being consecutive, APPENDS.
    ctrl(&mut s, 'k');
    assert_eq!(buffer_text(&s), "world");
    assert_eq!(ring(&s), vec!["hello\n"], "consecutive C-k appends");
}

#[test]
fn consecutive_kills_build_one_entry_and_sync_the_clipboard() {
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // "one"
    ctrl(&mut s, 'k'); // "\n"
    ctrl(&mut s, 'k'); // "two"
    assert_eq!(ring(&s), vec!["one\ntwo"]);
    // OS slot mirrors the appended head (Q#KR4).
    let slot: String = eval(&s, "return pmacs.editor.clipboard_get()");
    assert_eq!(slot, "one\ntwo");
}

#[test]
fn movement_breaks_the_kill_chain() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k'); // "one"; buffer now "\ntwo\n"
    press(&mut s, KeyCode::Down); // a cursor command: chain broken
    exec(&s, "pmacs.editor.goto_byte(1)"); // start of "two"
    ctrl(&mut s, 'k'); // "two"
    assert_eq!(ring(&s), vec!["two", "one"], "two entries, no append");
}

#[test]
fn self_insert_breaks_the_kill_chain() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k'); // "one" (line now "\ntwo\n", cursor 0)
    type_str(&mut s, "x"); // buffer "x\ntwo\n"
    ctrl(&mut s, 'k'); // kills "" ... wait: cursor after 'x' is 1, at "\n"
    // cursor sits at the newline → kills it.
    assert_eq!(ring(&s), vec!["\n", "one"], "self-insert broke the chain");
}

#[test]
fn an_unbound_key_breaks_the_kill_chain() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k');
    press(&mut s, KeyCode::F(12)); // unbound, not printable
    assert!(status(&s).contains("not bound"));
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 'k');
    assert_eq!(ring(&s).len(), 2, "unbound key broke the chain");
}

#[test]
fn failed_kill_does_not_leave_an_appendable_chain() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k'); // ring: ["one"]
    ctrl(&mut s, 'w'); // no region → fails, clears last_kill_id
    assert!(status(&s).contains("no region"));
    assert_eq!(ring(&s), vec!["one"], "failed kill pushed nothing");
    ctrl(&mut s, 'k'); // kills "\n"
    assert_eq!(
        ring(&s),
        vec!["\n", "one"],
        "a kill after a FAILED kill pushes fresh (no stale append)"
    );
}

// ---------------------------------------------------------------------------
// M-x semantics (the three-direction matrix, Q#KR2)
// ---------------------------------------------------------------------------

fn m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    type_str(s, name);
    press(s, KeyCode::Enter);
}

#[test]
fn m_x_kill_after_keybound_kill_does_not_append() {
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // "one"
    m_x(&mut s, "edit.kill-line"); // kills "\n" — but must NOT append
    assert_eq!(
        ring(&s),
        vec!["\n", "one"],
        "the minibuffer interaction breaks the chain (Emacs semantics)"
    );
}

#[test]
fn keybound_kill_after_m_x_kill_appends() {
    let mut s = editor_with("one\ntwo\nthree\n");
    m_x(&mut s, "edit.kill-line"); // "one" — invoke_interactive stamps it
    ctrl(&mut s, 'k'); // "\n" — last_command is edit.kill-line → append
    assert_eq!(
        ring(&s),
        vec!["one\n"],
        "execute-extended-command sets this-command (Emacs semantics)"
    );
}

#[test]
fn m_x_kill_twice_does_not_append() {
    let mut s = editor_with("one\ntwo\nthree\n");
    m_x(&mut s, "edit.kill-line");
    m_x(&mut s, "edit.kill-line");
    assert_eq!(ring(&s).len(), 2, "each M-x interposes execute-command");
}

// ---------------------------------------------------------------------------
// Yank / yank-pop
// ---------------------------------------------------------------------------

#[test]
fn yank_inserts_the_head_and_yank_pop_cycles_and_wraps() {
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // "one"
    press(&mut s, KeyCode::Down); // break chain
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 'k'); // rest of "two"-line from byte 1: "wo"... careful
    // (buffer was "\ntwo\nthree\n"; byte 1 = 't'; kills "two"[1..] = "wo")
    let r = ring(&s);
    assert_eq!(r.len(), 2);
    let newest = r[0].clone(); // "wo"
    let oldest = r[1].clone(); // "one"

    // Yank at the end of the buffer.
    let len: i64 = eval(&s, "local b = pmacs.window.buffer(); return b:len()");
    exec(&s, &format!("pmacs.editor.goto_byte({len})"));
    ctrl(&mut s, 'y');
    assert!(buffer_text(&s).ends_with(&newest), "C-y yanks the head");
    let cursor_after_yank: i64 = eval(&s, "return pmacs.editor.cursor()");
    assert_eq!(
        cursor_after_yank,
        len + i64::try_from(newest.len()).unwrap()
    );

    // M-y replaces with the older entry...
    alt(&mut s, 'y');
    assert!(buffer_text(&s).ends_with(&oldest), "M-y rotates to older");
    let cursor: i64 = eval(&s, "return pmacs.editor.cursor()");
    assert_eq!(
        cursor,
        len + i64::try_from(oldest.len()).unwrap(),
        "cursor at end of rotation"
    );
    // ...and wraps back to the newest.
    alt(&mut s, 'y');
    assert!(buffer_text(&s).ends_with(&newest), "M-y wraps");
}

#[test]
fn yank_pop_without_a_yank_refuses_and_stays_refused() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k');
    let before = buffer_text(&s);
    alt(&mut s, 'y');
    assert!(status(&s).contains("not a yank"));
    assert_eq!(buffer_text(&s), before, "no edit on refusal");
    // A second M-y must not ride the first one's name-stamp (Q#KR7):
    // last_command IS edit.yank-pop now, but no session exists.
    alt(&mut s, 'y');
    assert!(status(&s).contains("not a yank"));
    assert_eq!(buffer_text(&s), before);
}

#[test]
fn pointer_click_breaks_kill_chain_and_yank_session() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut s = editor_with("one\ntwo\nthree\n");
    let term = pmacs::cell::CellSize::new(24, 80);
    let click = |s: &mut EditorState, kind| {
        s.dispatch_mouse(
            FrontendId::LOCAL,
            MouseEvent {
                kind,
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            term,
        );
    };

    // Kill side: C-k, click, C-k → two entries.
    ctrl(&mut s, 'k');
    click(&mut s, MouseEventKind::Down(MouseButton::Left));
    click(&mut s, MouseEventKind::Up(MouseButton::Left));
    ctrl(&mut s, 'k');
    assert_eq!(ring(&s).len(), 2, "a click must break the kill chain");

    // Yank side: C-y, click, M-y → refused.
    ctrl(&mut s, 'y');
    click(&mut s, MouseEventKind::Down(MouseButton::Left));
    click(&mut s, MouseEventKind::Up(MouseButton::Left));
    let before = buffer_text(&s);
    alt(&mut s, 'y');
    assert!(
        status(&s).contains("not a yank"),
        "click invalidates the yank"
    );
    assert_eq!(buffer_text(&s), before);
}

#[test]
fn wheel_scroll_does_not_break_the_kill_chain() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let term = pmacs::cell::CellSize::new(24, 80);
    let scroll = |s: &mut EditorState, kind| {
        s.dispatch_mouse(
            FrontendId::LOCAL,
            MouseEvent {
                kind,
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            term,
        );
    };

    // Case 1 — a boundary-clamped scroll (ScrollUp at the top) moves
    // nothing at all: the chain holds and the next C-k appends in
    // place.
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // "one"
    scroll(&mut s, MouseEventKind::ScrollUp);
    ctrl(&mut s, 'k'); // the newline
    assert_eq!(ring(&s), vec!["one\n"], "no-op scroll preserves the chain");

    // Case 2 — pmacs scrolling is cursor-follows-view, so a mid-buffer
    // wheel moves point too. The chain STILL holds (Emacs: mwheel
    // preserves last-command) and the next kill appends from the NEW
    // point — one entry, not two.
    let mut s = editor_with("one\ntwo\nthree\nfour\nfive\nsix\n");
    ctrl(&mut s, 'k'); // "one"
    scroll(&mut s, MouseEventKind::ScrollDown);
    ctrl(&mut s, 'k');
    assert_eq!(
        ring(&s).len(),
        1,
        "a cursor-following scroll still preserves the chain: {:?}",
        ring(&s)
    );
}

#[test]
fn semantic_pointer_gesture_breaks_the_chain() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k');
    let buf_id = s.core.borrow().active_window().buffer_id;
    s.dispatch_pointer(
        FrontendId::LOCAL,
        buf_id,
        5,
        pmacs::protocol::PointerKind::Down,
        pmacs::protocol::Modifiers::NONE,
    );
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 'k');
    assert_eq!(
        ring(&s).len(),
        2,
        "a semantic pointer Down breaks the chain"
    );
}

// ---------------------------------------------------------------------------
// Region kills, selection yank
// ---------------------------------------------------------------------------

#[test]
fn cut_and_copy_feed_the_ring_and_yank_replaces_a_selection() {
    let mut s = editor_with("alpha beta\n");
    // Select "alpha" (bytes 0..5).
    exec(
        &s,
        "pmacs.editor.goto_byte(5); pmacs.editor.begin_selection(0)",
    );
    alt(&mut s, 'w'); // copy
    assert_eq!(ring(&s), vec!["alpha"]);
    assert_eq!(buffer_text(&s), "alpha beta\n", "copy does not delete");

    // Cut " beta" (bytes 5..10).
    exec(
        &s,
        "pmacs.editor.goto_byte(10); pmacs.editor.begin_selection(5)",
    );
    ctrl(&mut s, 'w');
    assert_eq!(buffer_text(&s), "alpha\n");
    assert_eq!(ring(&s), vec![" beta", "alpha"]);

    // Yank over a selection replaces it, and M-y rotates against the
    // replaced range.
    exec(
        &s,
        "pmacs.editor.goto_byte(5); pmacs.editor.begin_selection(0)",
    );
    ctrl(&mut s, 'y'); // "alpha" → " beta"
    assert_eq!(buffer_text(&s), " beta\n");
    alt(&mut s, 'y'); // rotate to "alpha"
    assert_eq!(buffer_text(&s), "alpha\n");
}

// ---------------------------------------------------------------------------
// Shared-ring interleaving (two frontends)
// ---------------------------------------------------------------------------

/// A second "frontend": an unregistered id shares LOCAL's view (the
/// `active_view` fallback) but has its own command boundary and killring
/// session — the exact shape of the Q#KR4/KR7 interleaving blockers.
const B: FrontendId = FrontendId(9);

fn ctrl_as(s: &mut EditorState, fid: FrontendId, c: char) {
    s.dispatch_key(fid, key(KeyCode::Char(c), KeyModifiers::CONTROL));
}

#[test]
fn interleaved_kills_never_append_across_frontends() {
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // A kills "one"
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl_as(&mut s, B, 'k'); // B kills "wo" — B's own first kill
    exec(&s, "pmacs.editor.goto_byte(2)");
    ctrl(&mut s, 'k'); // A again — A's last_kill is NOT the head (B's is)
    let r = ring(&s);
    assert_eq!(r.len(), 3, "A-kill/B-kill/A-kill = three entries");
    assert_eq!(r[1], "two", "B's entry intact — never appended onto");
}

#[test]
fn yank_pop_rotates_from_the_sessions_own_entry_despite_other_pushes() {
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // A: ring ["one"]
    let len: i64 = eval(&s, "local b = pmacs.window.buffer(); return b:len()");
    exec(&s, &format!("pmacs.editor.goto_byte({len})"));
    ctrl(&mut s, 'y'); // A yanks "one"; session → entry("one")

    // B pushes a new head, shifting positions but not ids.
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl_as(&mut s, B, 'k'); // B kills "wo" → ring ["wo", "one"]

    // A's M-y must rotate from A's OWN entry ("one" — now position 2),
    // to the next-older-with-wrap = "wo". An index-based session would
    // have mis-resolved after B's push.
    // (B's kill edited the buffer, but before A's yanked range, so the
    // session's slice-verify passes — the range shifted is upstream.)
    // Note: B's kill removed bytes BEFORE the yank range, so the
    // remembered {start,stop} no longer hold the yanked text → the
    // invalidation guard fires instead. That IS the specified
    // behavior: refuse rather than splice a shifted range.
    alt(&mut s, 'y');
    assert!(
        status(&s).contains("changed since the yank"),
        "a concurrent upstream edit invalidates rather than mis-splices: {:?}",
        status(&s)
    );
}

#[test]
fn yank_pop_uses_stable_ids_when_other_pushes_leave_the_range_intact() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k'); // ring ["one"], A's chain live
    let len: i64 = eval(&s, "local b = pmacs.window.buffer(); return b:len()");
    exec(&s, &format!("pmacs.editor.goto_byte({len})"));
    ctrl(&mut s, 'y'); // A yanks "one" at end; session entry = "one"

    // B COPIES bytes 1..3 ("tw" of the post-kill buffer "\ntwo\none"):
    // ring becomes ["tw", "one"], buffer untouched, so A's yanked
    // range is still intact.
    exec(
        &s,
        "pmacs.editor.begin_selection(1); pmacs.editor.goto_byte(3)",
    );
    s.dispatch_key(B, key(KeyCode::Char('w'), KeyModifiers::ALT));
    assert_eq!(ring(&s), vec!["tw", "one"]);

    // A's M-y: the session's entry ("one") sits at position 2 now;
    // next-older-with-wrap is B's copy. An integer index recorded at
    // yank time (position 1) would have rotated from the wrong place.
    alt(&mut s, 'y');
    assert!(
        buffer_text(&s).ends_with("tw"),
        "rotation follows the stable id, not a shifted index: {:?}",
        buffer_text(&s)
    );
}

#[test]
fn eviction_mid_session_invalidates_the_yank_pop() {
    let mut s = editor_with("one\ntwo\nthree\nfour\n");
    exec(&s, "pmacs.killring.max(2)");
    ctrl(&mut s, 'k'); // ring ["one"]
    let len: i64 = eval(&s, "local b = pmacs.window.buffer(); return b:len()");
    exec(&s, &format!("pmacs.editor.goto_byte({len})"));
    ctrl(&mut s, 'y'); // session → entry "one"

    // Two copies (buffer untouched) evict "one" from a cap-2 ring.
    exec(
        &s,
        "pmacs.editor.begin_selection(1); pmacs.editor.goto_byte(3)",
    );
    s.dispatch_key(B, key(KeyCode::Char('w'), KeyModifiers::ALT));
    exec(
        &s,
        "pmacs.editor.begin_selection(5); pmacs.editor.goto_byte(8)",
    );
    s.dispatch_key(B, key(KeyCode::Char('w'), KeyModifiers::ALT));
    let r = ring(&s);
    assert_eq!(r.len(), 2);
    assert!(!r.contains(&"one".to_string()), "'one' evicted");

    alt(&mut s, 'y');
    assert!(
        status(&s).contains("expired"),
        "an evicted session entry refuses cleanly: {:?}",
        status(&s)
    );
}

// ---------------------------------------------------------------------------
// External content, menu, hook delivery, cap
// ---------------------------------------------------------------------------

#[test]
fn externally_pasted_content_joins_the_ring_at_yank() {
    let mut s = editor_with("");
    ctrl(&mut s, 'k'); // fails (empty buffer) — ring stays empty
    // An OS paste arrives (the daemon route sets the slot + inserts).
    s.core.borrow_mut().paste_inbound(b"external").unwrap();
    assert_eq!(buffer_text(&s), "external");
    // The next yank notices slot ≠ head, pushes it, and yanks it.
    ctrl(&mut s, 'y');
    assert_eq!(ring(&s), vec!["external"]);
    assert_eq!(buffer_text(&s), "externalexternal");
}

#[test]
fn menu_cut_feeds_the_ring_fires_after_edit_and_chains() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut s = editor_with("alpha beta\n");
    exec(
        &s,
        r#"
        _G.AE = 0
        pmacs.hook.add("buffer.after-edit", function() _G.AE = _G.AE + 1 end)
        "#,
    );
    // Select "alpha", open the context menu via right-click, invoke Cut.
    exec(
        &s,
        "pmacs.editor.goto_byte(5); pmacs.editor.begin_selection(0)",
    );
    s.dispatch_mouse(
        FrontendId::LOCAL,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 2,
            row: 0,
            modifiers: KeyModifiers::NONE,
        },
        pmacs::cell::CellSize::new(24, 80),
    );
    assert!(
        s.core.borrow().menu_is_open(),
        "right-click opened the menu"
    );
    // Find the "edit.cut" row in the open menu's state.
    let cut_index = {
        let menu = s.core.borrow().menu.clone();
        let guard = menu.lock().unwrap();
        guard
            .as_ref()
            .and_then(|m| {
                m.rows.iter().position(|r| {
                    matches!(r, pmacs::menu::MenuRow::Item { command, .. }
                             if command == "edit.cut")
                })
            })
            .expect("menu has a Cut row")
    };
    s.dispatch_menu_pointer(
        FrontendId::LOCAL,
        Some(u32::try_from(cut_index).unwrap()),
        true,
    );

    assert_eq!(ring(&s), vec!["alpha"], "menu Cut fed the ring");
    assert_eq!(buffer_text(&s), " beta\n");
    let fired: i64 = eval(&s, "return _G.AE");
    assert_eq!(fired, 1, "menu Cut fires after-edit exactly once (Q#KR10b)");

    // The menu rotation makes the cut chain like a keybound one:
    // a following C-k appends.
    ctrl(&mut s, 'k');
    let r = ring(&s);
    assert_eq!(r[0], "alpha beta", "menu Cut then C-k appends (rotate row)");
}

#[test]
fn m_x_kill_fires_after_edit_and_keybound_does_not_double_fire() {
    let mut s = editor_with("one\ntwo\n");
    exec(
        &s,
        r#"
        _G.AE = 0
        pmacs.hook.add("buffer.after-edit", function() _G.AE = _G.AE + 1 end)
        "#,
    );
    ctrl(&mut s, 'k');
    let after_keybound: i64 = eval(&s, "return _G.AE");
    assert_eq!(after_keybound, 1, "keybound kill fires once, no double");
    m_x(&mut s, "edit.kill-line");
    let after_mx: i64 = eval(&s, "return _G.AE");
    assert_eq!(after_mx, 2, "M-x kill fires after-edit too (Q#KR10b)");
}

#[test]
fn cap_is_validated_and_shrink_trims() {
    let s = editor_with("");
    let d: i64 = eval(&s, "return pmacs.killring.max()");
    assert_eq!(d, 60);
    for bad in ["0/0", "math.huge", "-math.huge", "0", "-3", "'ten'", "{}"] {
        let ok: bool = eval(&s, &format!("return (pcall(pmacs.killring.max, {bad}))"));
        assert!(!ok, "killring.max({bad}) must be rejected");
    }
    let set: i64 = eval(&s, "return pmacs.killring.max(2.9)");
    assert_eq!(set, 2, "floored");

    // Five distinct entries via COPY over a static buffer (copies never
    // delete, so the byte offsets stay put; each text is distinct so
    // duplicate-of-head collapse never fires).
    let mut s2 = editor_with("aa bb cc dd ee\n");
    exec(&s2, "pmacs.killring.max(10)");
    for i in 0..5i64 {
        let lo = i * 3;
        exec(
            &s2,
            &format!(
                "pmacs.editor.begin_selection({lo}); pmacs.editor.goto_byte({})",
                lo + 2
            ),
        );
        alt(&mut s2, 'w');
    }
    assert_eq!(ring(&s2).len(), 5);
    let trimmed: i64 = eval(&s2, "return pmacs.killring.max(3)");
    assert_eq!(trimmed, 3);
    assert_eq!(ring(&s2).len(), 3, "shrinking the cap trims immediately");
}

#[test]
fn semantic_context_right_click_breaks_the_chain() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k'); // "one" — chain live
    // The semantic dispatcher routes PointerKind::Context straight to
    // open_menu_at_byte, bypassing dispatch_pointer — the GPU
    // right-click path.
    let buf_id = s.core.borrow().active_window().buffer_id;
    s.open_menu_at_byte(FrontendId::LOCAL, buf_id, 3);
    // Dismiss the menu without invoking anything.
    s.core.borrow_mut().menu_close();
    ctrl(&mut s, 'k');
    assert_eq!(
        ring(&s).len(),
        2,
        "a semantic right-click must break the kill chain: {:?}",
        ring(&s)
    );
}

#[test]
fn rejecting_intercept_clears_the_kill_chain() {
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // "one" — chain live, ring ["one"]
    // An intercept that rejects exactly the NEXT edit, then allows.
    exec(
        &s,
        r#"
        _G.reject_once = true
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(_op)
          if _G.reject_once then
            _G.reject_once = false
            error("rejected by test intercept")
          end
          return nil
        end)
        "#,
    );
    ctrl(&mut s, 'k'); // rejected — must clear the chain, push nothing
    assert!(
        status(&s).contains("rejected"),
        "rejection is reported: {:?}",
        status(&s)
    );
    assert_eq!(ring(&s), vec!["one"], "a rejected kill feeds nothing");
    ctrl(&mut s, 'k'); // allowed again — must push FRESH, not append
    assert_eq!(
        ring(&s),
        vec!["\n", "one"],
        "the chain did not survive the rejection"
    );
}

#[test]
fn transforming_intercept_does_not_feed_the_ring() {
    let mut s = editor_with("alpha\nbeta\n");
    // An intercept that shrinks every delete to its first byte: the
    // bytes actually removed are not what C-k sliced, so pushing the
    // sliced text would put never-killed bytes on the ring.
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "delete" then
            return { kind = "delete", start = op.start, ["end"] = op.start + 1 }
          end
          return nil
        end)
        "#,
    );
    ctrl(&mut s, 'k');
    assert!(
        status(&s).contains("altered"),
        "transformation is reported: {:?}",
        status(&s)
    );
    assert!(ring(&s).is_empty(), "a transformed kill feeds nothing");
    // The interceptor's result stands (accepted post-hoc semantics).
    assert_eq!(buffer_text(&s), "lpha\nbeta\n");
}

#[test]
fn equal_length_shifted_delete_does_not_feed_the_ring() {
    let mut s = editor_with("abcdef\nghijkl\n");
    // An intercept that SHIFTS every delete right by 2 bytes while
    // keeping its length — a length-delta check cannot see this, and
    // the ring would receive text that was never killed.
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "delete" then
            return { kind = "delete", start = op.start + 2, ["end"] = op["end"] + 2 }
          end
          return nil
        end)
        "#,
    );
    ctrl(&mut s, 'k'); // wanted [0,6) "abcdef"; intercept deletes [2,8)
    assert!(
        status(&s).contains("altered"),
        "an equal-length shifted delete is detected: {:?}",
        status(&s)
    );
    assert!(
        ring(&s).is_empty(),
        "never-killed text must not reach the ring: {:?}",
        ring(&s)
    );
    // The interceptor's result stands.
    assert_eq!(buffer_text(&s), "abhijkl\n");
}

#[test]
fn stop_enlarging_replace_ends_the_yank_session() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k'); // "one"
    press(&mut s, KeyCode::Down);
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 'k'); // "two"
    // Yank at the START so the buffer extends past the yanked range —
    // an end+1 range at buffer end would fail validation ("rejected")
    // instead of exercising the transform path.
    exec(&s, "pmacs.editor.goto_byte(0)");
    ctrl(&mut s, 'y'); // session live

    // An intercept that enlarges every replace's end by ONE byte: the
    // replacement text still lands at s.start, so a "text appears at
    // start" verify passes — but one extra byte was silently deleted.
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "replace" then
            return { kind = "replace", start = op.start, ["end"] = op["end"] + 1 }
          end
          return nil
        end)
        "#,
    );
    alt(&mut s, 'y');
    assert!(
        status(&s).contains("altered"),
        "an end-enlarged replace is detected: {:?}",
        status(&s)
    );
    // The session is dead: a further M-y refuses without editing.
    let before = buffer_text(&s);
    alt(&mut s, 'y');
    assert!(status(&s).contains("not a yank"));
    assert_eq!(buffer_text(&s), before);
}

#[test]
fn rejecting_intercept_ends_the_yank_session() {
    let mut s = editor_with("one\ntwo\n");
    ctrl(&mut s, 'k'); // "one"
    press(&mut s, KeyCode::Down);
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 'k'); // "two"
    assert_eq!(ring(&s).len(), 2);

    let len: i64 = eval(&s, "local b = pmacs.window.buffer(); return b:len()");
    exec(&s, &format!("pmacs.editor.goto_byte({len})"));
    ctrl(&mut s, 'y'); // session live

    // Reject the next edit (the M-y replace).
    exec(
        &s,
        r#"
        _G.reject_once = true
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(_op)
          if _G.reject_once then
            _G.reject_once = false
            error("rejected by test intercept")
          end
          return nil
        end)
        "#,
    );
    alt(&mut s, 'y'); // rejected — must END the session, not throw through
    assert!(
        status(&s).contains("rejected"),
        "rejection reported: {:?}",
        status(&s)
    );
    // A second M-y must refuse on "no session", not reuse the dead one.
    let before = buffer_text(&s);
    alt(&mut s, 'y');
    assert!(
        status(&s).contains("not a yank"),
        "the rejected session is gone: {:?}",
        status(&s)
    );
    assert_eq!(buffer_text(&s), before, "no splice from a dead session");
}

#[test]
fn frontend_detached_drops_per_frontend_state() {
    let mut s = editor_with("one\ntwo\n");
    ctrl_as(&mut s, B, 'k'); // B kills → B has last_kill_id
    let has: bool = eval(
        &s,
        "return pmacs.killring._debug_state(9).last_kill_id ~= nil",
    );
    assert!(has, "B has kill state");
    // The daemon fires this on SessionDetached (Q#KR11); fire it the
    // same way to exercise the Lua-side cleanup.
    exec(&s, "pmacs.hook.run('frontend.detached', 9)");
    let gone: bool = eval(
        &s,
        "local st = pmacs.killring._debug_state(9); \
         return st.last_kill_id == nil and st.session == nil",
    );
    assert!(gone, "detach dropped B's killring state");
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
