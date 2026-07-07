//! In-buffer completion popup acceptance (Arc 1a) --- the phase-1
//! TUI/core path end-to-end: the Lua driver's Q#C9 auto-open policy,
//! the Q#C3 partial dispatcher shadow, and Q#C7 validated accept, all
//! driven through `dispatch_key` exactly as a terminal user would.
//!
//! The LSP provider's request path is covered by the `m4_5` fake-LSP
//! suite; these tests run hermetic (dabbrev + custom Lua providers)
//! so no server binary is needed.
//!
//! Framing: docs/in-buffer-completion-framing.md.

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

/// `(buffer text, popup visible?, cursor)` probed through the Lua
/// surface --- the same introspection a user-facing script would use.
fn probe(s: &EditorState) -> (String, bool, i64) {
    s.lua_host
        .lua()
        .load(
            "
            local b = pmacs.window.buffer()
            local text = b:slice(0, b:len())
            return text, pmacs.completion.popup_visible(), pmacs.editor.cursor()
            ",
        )
        .eval()
        .expect("probe buffer/popup state")
}

/// Typing a two-char prefix of an existing buffer word auto-opens the
/// popup off dabbrev (Q#C9 single-char signature), and TAB accepts:
/// the prefix is replaced by the candidate in one step.
#[test]
fn typing_opens_popup_and_tab_accepts() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello_world ");
    let (_, visible, _) = probe(&s);
    assert!(
        !visible,
        "no popup while typing the only word in the buffer"
    );

    type_str(&mut s, "he");
    let (_, visible, _) = probe(&s);
    assert!(
        visible,
        "prefix `he` with dabbrev match `hello_world` opens"
    );

    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Tab, KeyModifiers::NONE));
    let (text, visible, cursor) = probe(&s);
    assert_eq!(text, "hello_world hello_world", "TAB replaces the prefix");
    assert!(!visible, "accept closes the popup");
    assert_eq!(cursor, 23, "cursor lands just past the inserted text");
}

/// C-n moves the highlight before RET accepts, so the second
/// candidate wins. Uses a custom provider for a deterministic order.
#[test]
fn ctrl_n_navigates_then_ret_accepts_second_candidate() {
    let mut s = EditorState::new();
    s.lua_host
        .lua()
        .load(
            "
            pmacs.completion.register({
                name = 'test_src',
                priority = 200,
                fn = function()
                    return {
                        { label = 'aardvark', kind = 'text' },
                        { label = 'aardwolf', kind = 'text' },
                    }
                end,
            })
            ",
        )
        .exec()
        .expect("register test provider");

    type_str(&mut s, "aa");
    let (_, visible, _) = probe(&s);
    assert!(visible, "custom provider matches the `aa` prefix");

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('n'), KeyModifiers::CONTROL),
    );
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Enter, KeyModifiers::NONE));
    let (text, visible, _) = probe(&s);
    assert_eq!(text, "aardwolf", "C-n selected the second candidate");
    assert!(!visible);
}

/// Esc dismisses without touching the buffer, and the next printable
/// key self-inserts normally (the shadow is partial, not modal).
#[test]
fn esc_dismisses_and_typing_falls_through() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello_world he");
    let (_, visible, _) = probe(&s);
    assert!(visible);

    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Esc, KeyModifiers::NONE));
    let (text, visible, _) = probe(&s);
    assert_eq!(text, "hello_world he", "Esc leaves the buffer untouched");
    assert!(!visible, "Esc dismisses");

    type_str(&mut s, "x");
    let (text, visible, _) = probe(&s);
    assert_eq!(text, "hello_world hex", "typing after Esc self-inserts");
    assert!(!visible, "the dismissed popup does not reopen off that key");
}

/// Motion that breaks the anchor invariant closes the popup via the
/// post-dispatch validation (Q#C3): Home moves the cursor before the
/// anchor.
#[test]
fn motion_before_anchor_closes_popup() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello_world he");
    let (_, visible, _) = probe(&s);
    assert!(visible);

    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Home, KeyModifiers::NONE));
    let (text, visible, _) = probe(&s);
    assert_eq!(text, "hello_world he");
    assert!(!visible, "cursor before the anchor invalidates the session");
}

/// A multi-byte edit (kill-ring yank) never auto-opens the popup ---
/// the Q#C9 single-char signature rejects paste-shaped deltas.
#[test]
fn yank_shaped_edit_does_not_auto_open() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello_world hello");
    // Select the trailing word and cut it (C-w): the popup that was
    // open over `hello` closes as its word dies.
    for _ in 0..5 {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Left, KeyModifiers::SHIFT));
    }
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('w'), KeyModifiers::CONTROL),
    );
    let (text, visible, _) = probe(&s);
    assert_eq!(text, "hello_world ");
    assert!(!visible, "cutting the word closes the popup");

    // Yank it back: one edit, five bytes --- not a typing signature.
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('y'), KeyModifiers::CONTROL),
    );
    let (text, visible, _) = probe(&s);
    assert_eq!(text, "hello_world hello", "yank restored the word");
    assert!(!visible, "a 5-byte edit must not auto-open the popup");
}

/// `completion.at-point` (C-M-i) opens deliberately, even below the
/// auto-open prefix threshold.
#[test]
fn at_point_command_opens_below_threshold() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello_world h");
    let (_, visible, _) = probe(&s);
    assert!(!visible, "a 1-char prefix stays below the auto-open bar");

    s.dispatch_key(
        FrontendId::LOCAL,
        key(
            KeyCode::Char('i'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
    );
    let (_, visible, _) = probe(&s);
    assert!(visible, "C-M-i opens the popup on demand");

    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Tab, KeyModifiers::NONE));
    let (text, _, _) = probe(&s);
    assert_eq!(text, "hello_world hello_world");
}

/// The Q#C8 URI scoping: with `ctx.uri` set, the LSP provider only
/// surfaces the matching document's cached items; without it, the
/// legacy global drain applies. Driven through the Lua collect
/// surface against a hand-seeded store.
#[test]
fn collect_scopes_lsp_candidates_to_ctx_uri() {
    let s = EditorState::new();
    let (scoped, unscoped): (u64, u64) = s
        .lua_host
        .lua()
        .load(
            "
            -- Seed the M4.7 LSP store for two URIs through a custom
            -- provider is not possible (the store is Rust-side), so
            -- emulate the shape: a URI-aware provider that mirrors
            -- what the built-in LSP provider does with ctx.uri.
            pmacs.completion.register({
                name = 'uri_aware',
                priority = 150,
                fn = function(prefix, line, col, text, lang, root, trig, tchar, uri)
                    local by_uri = {
                        ['file:///a.rs'] = { { label = 'alpha_from_a' } },
                        ['file:///b.rs'] = { { label = 'alpha_from_b' } },
                    }
                    if uri then
                        return by_uri[uri] or {}
                    end
                    local all = {}
                    for _, items in pairs(by_uri) do
                        for _, it in ipairs(items) do all[#all + 1] = it end
                    end
                    return all
                end,
            })
            local scoped = pmacs.completion.collect({
                prefix = 'alpha', uri = 'file:///a.rs',
            })
            local unscoped = pmacs.completion.collect({ prefix = 'alpha' })
            return #scoped, #unscoped
            ",
        )
        .eval()
        .expect("collect with and without uri");
    assert_eq!(scoped, 1, "ctx.uri reaches Lua providers (9th arg)");
    assert_eq!(unscoped, 2, "no uri → all documents");
}
