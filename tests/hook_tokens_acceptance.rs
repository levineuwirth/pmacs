//! E5.2 --- `pmacs.hook.add` returns a token and `pmacs.hook.remove`
//! takes it back.
//!
//! Before this phase `hook.add` returned nothing and no `remove`
//! existed, so the thirty-five `hook.add` sites in the runtime and
//! every package's attached the same callback again on each
//! `reload`. Pinned here through the Lua surface: the token detaches
//! exactly its callback, a stale token is `false` and not an error,
//! and a callback removed while its hook runs finishes that run
//! (`run` snapshots the list) and is absent from the next.
//!
//! Bite: make `remove` return `false` without retaining and the first
//! two rows fail; make `run` iterate the live list and the third fails.

use pmacs::editor::EditorState;

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
    state
}

#[test]
fn add_returns_a_token_and_remove_detaches_exactly_that_callback() {
    let state = editor();
    exec(
        &state,
        r#"
        pmacs.hook.define { name = "e5.tokens", description = "a hook for the witness" }
        _G.calls = {}
        _G.a = pmacs.hook.add("e5.tokens", function() _G.calls[#_G.calls + 1] = "a" end)
        _G.b = pmacs.hook.add("e5.tokens", function() _G.calls[#_G.calls + 1] = "b" end)
        _G.c = pmacs.hook.add("e5.tokens", function() _G.calls[#_G.calls + 1] = "c" end)
        "#,
    );
    let (a, b, c): (i64, i64, i64) = eval(&state, "return _G.a, _G.b, _G.c");
    assert!(
        a != b && b != c && a != c,
        "three distinct tokens: {a} {b} {c}"
    );
    exec(&state, "pmacs.hook.run('e5.tokens')");
    let calls: Vec<String> = eval(&state, "return _G.calls");
    assert_eq!(calls, vec!["a", "b", "c"]);

    let removed: bool = eval(&state, "return pmacs.hook.remove(_G.b)");
    assert!(removed, "an attached callback is removed");
    exec(&state, "_G.calls = {}; pmacs.hook.run('e5.tokens')");
    let calls: Vec<String> = eval(&state, "return _G.calls");
    assert_eq!(calls, vec!["a", "c"], "only b is gone, order kept");
}

#[test]
fn a_stale_token_is_false_not_an_error() {
    let state = editor();
    exec(
        &state,
        r#"
        pmacs.hook.define { name = "e5.stale", description = "a hook for the witness" }
        _G.t = pmacs.hook.add("e5.stale", function() end)
        "#,
    );
    let first: bool = eval(&state, "return pmacs.hook.remove(_G.t)");
    let second: bool = eval(&state, "return pmacs.hook.remove(_G.t)");
    let never: bool = eval(&state, "return pmacs.hook.remove(123456789)");
    assert!(first);
    assert!(!second, "teardown may run twice");
    assert!(!never, "a token nobody handed out is simply false");
}

#[test]
fn a_callback_removed_during_a_run_finishes_that_run_and_misses_the_next() {
    let state = editor();
    exec(
        &state,
        r#"
        pmacs.hook.define { name = "e5.snapshot", description = "a hook for the witness" }
        _G.calls = {}
        _G.first = pmacs.hook.add("e5.snapshot", function()
          _G.calls[#_G.calls + 1] = "first"
          pmacs.hook.remove(_G.second)
        end)
        _G.second = pmacs.hook.add("e5.snapshot", function()
          _G.calls[#_G.calls + 1] = "second"
        end)
        pmacs.hook.run("e5.snapshot")
        "#,
    );
    let calls: Vec<String> = eval(&state, "return _G.calls");
    assert_eq!(
        calls,
        vec!["first", "second"],
        "the run keeps its snapshot: second still ran"
    );
    exec(&state, "_G.calls = {}; pmacs.hook.run('e5.snapshot')");
    let calls: Vec<String> = eval(&state, "return _G.calls");
    assert_eq!(calls, vec!["first"], "and is absent from the next run");
}
