// tests/minibuffer_source_acceptance.rs --- E4.1: the custom-source cap.

//! Acceptance for the E4.1 contract at the Lua boundary: a function
//! source receives the typed needle, its pool is filtered whole and
//! capped after sorting, `ranked = true` hands the order to the source
//! and is refused for the builtin sources, `pmacs.minibuffer.refresh`
//! recomputes without input, and `pmacs.minibuffer.rank` is the
//! session's own scorer. The unit tests in `src/minibuffer.rs` pin the
//! same rules on the type; these drive them through the real prompt.
//!
//! Typing goes through `dispatch_key`, as the find-file suite does: a
//! source that received the needle only on the Lua-driven
//! `set_contents` path would pass a weaker test and lie on the keyboard.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn editor() -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    state.lua_host.reopen_init_phase_for_testing();
    state
}

fn candidates(s: &EditorState) -> Vec<String> {
    eval::<Vec<String>>(s, "return pmacs.minibuffer.candidates()")
}

/// The needle reaches the source on every keystroke, and the pool is
/// filtered whole: the match is the 3001st entry and is still found.
#[test]
fn a_function_source_receives_the_needle_and_its_pool_is_filtered_whole() {
    let mut s = editor();
    exec(
        &s,
        r#"
        seen = {}
        pmacs.minibuffer.read {
          prompt = "P: ",
          source = function(needle)
            seen[#seen + 1] = needle
            local t = {}
            for i = 1, 3000 do t[i] = string.format("entry%04d", i) end
            t[#t + 1] = "needle-match"
            return t
          end,
          on_accept = function() end,
        }
        "#,
    );
    type_str(&mut s, "needle");
    let seen: Vec<String> = eval(&s, "return seen");
    assert_eq!(
        seen.first().map(String::as_str),
        Some(""),
        "opened with an empty needle"
    );
    assert_eq!(
        seen.last().map(String::as_str),
        Some("needle"),
        "the last recompute must carry the whole typed text; got {seen:?}"
    );
    assert_eq!(
        candidates(&s),
        vec!["needle-match".to_owned()],
        "an entry past the 1024th must be reachable by typing"
    );
}

/// `ranked = true` keeps the source's order and filters nothing; the
/// same flag on a builtin source is refused at `read`.
#[test]
fn ranked_keeps_the_source_order_and_is_refused_for_builtin_sources() {
    let mut s = editor();
    exec(
        &s,
        r#"
        pmacs.minibuffer.read {
          prompt = "P: ",
          ranked = true,
          source = function(needle) return { "b" .. needle, "a", "c" } end,
          on_accept = function() end,
        }
        "#,
    );
    type_str(&mut s, "zz");
    assert_eq!(
        candidates(&s),
        vec!["bzz".to_owned(), "a".to_owned(), "c".to_owned()],
        "a ranked source's order is the candidate order and nothing is filtered"
    );
    exec(&s, "pmacs.minibuffer.cancel()");
    let (ok, err): (bool, String) = eval(
        &s,
        r#"
        local ok, err = pcall(pmacs.minibuffer.read, {
          prompt = "P: ", ranked = true, source = "commands", on_accept = function() end,
        })
        return ok, tostring(err)
        "#,
    );
    assert!(!ok, "ranked over a builtin source must be refused");
    assert!(
        err.contains("ranked = true needs a function source"),
        "{err}"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "a refused read opens no session"
    );
}

/// `refresh` recomputes against the live source with no keystroke: a
/// pool that arrives after the prompt opened becomes visible.
#[test]
fn refresh_recomputes_the_candidates_without_input() {
    let s = editor();
    exec(
        &s,
        r#"
        pool = {}
        pmacs.minibuffer.read {
          prompt = "P: ",
          source = function() return pool end,
          on_accept = function() end,
        }
        "#,
    );
    assert!(
        candidates(&s).is_empty(),
        "the pool was empty when the prompt opened"
    );
    exec(
        &s,
        "pool = { 'late-one', 'late-two' } pmacs.minibuffer.refresh()",
    );
    assert_eq!(
        candidates(&s),
        vec!["late-one".to_owned(), "late-two".to_owned()],
        "refresh must surface the pool that landed after the prompt opened"
    );
    exec(&s, "pmacs.minibuffer.cancel()");
    // A no-op with no session, and not an error.
    exec(&s, "pmacs.minibuffer.refresh()");
}

/// `rank` filters, sorts best first, and only then applies the limit.
#[test]
fn rank_filters_sorts_and_then_caps() {
    let s = editor();
    let ranked: Vec<String> = eval(
        &s,
        "return pmacs.minibuffer.rank('ab', { 'zzab1', 'zzab2', 'nomatch', 'ab' })",
    );
    assert_eq!(
        ranked,
        vec!["ab".to_owned(), "zzab1".to_owned(), "zzab2".to_owned()],
        "filtered to matches, best first, ties lexical"
    );
    let capped: Vec<String> = eval(
        &s,
        "return pmacs.minibuffer.rank('ab', { 'zzab1', 'zzab2', 'nomatch', 'ab' }, 1)",
    );
    assert_eq!(
        capped,
        vec!["ab".to_owned()],
        "the limit applies after the sort"
    );
}
