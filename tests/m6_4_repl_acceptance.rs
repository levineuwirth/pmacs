// m6_4_repl_acceptance.rs --- T M6.4 acceptance gates.

//! Acceptance gates for T M6.4 (REPL view skeleton, Lua package).
//!
//! Spec §sec:repl-view defines four acceptance bullets:
//!
//! 1. Three-region structure renders correctly →
//!    [`m6_4_three_regions_render_correctly`],
//!    [`m6_4_appending_output_extends_history`],
//!    [`m6_4_set_prompt_replaces_prompt_region`].
//! 2. Region boundaries update correctly when synthetic output is
//!    appended → [`m6_4_appending_output_extends_history`].
//! 3. `intercept_edit` rejects edits in history and prompt regions →
//!    [`m6_4_intercept_rejects_history_edits`],
//!    [`m6_4_intercept_rejects_prompt_edits`],
//!    [`m6_4_input_edits_pass_through`].
//! 4. Edits that span the input region boundary are truncated to the
//!    input region →
//!    [`m6_4_paste_spanning_input_boundary_truncated_to_input`],
//!    [`m6_4_delete_spanning_input_boundary_truncated_to_input`].
//!
//! Plus regression tests for the M6.3 ↔ M6.4 boundary:
//!
//! - [`m6_4_ansi_styled_output_routes_to_history_only`][]: SGR-styled
//!   output produces a clean rope (no escape bytes) and `SetStyle`
//!   events on the side channel.
//! - [`m6_4_alt_screen_suppresses_history_writes`][]: a process
//!   entering alt-screen does not write to history; exiting restores
//!   normal flow.
//! - [`m6_4_submit_does_not_append_to_history`][]: locks in the
//!   contract that the process echo (in M6.5) is the path that
//!   appends user input to history, not `submit()` itself.

use pmacs::editor::EditorState;
use pmacs::lua_bindings::BufferIdLua;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// Construct a fresh editor and run the given Lua chunk against it.
/// Lua-level `assert` failures surface as test failures; we don't
/// translate them into typed Rust errors because the chunk's
/// assertions are the test contract.
fn run(chunk: &str) {
    let mut editor = EditorState::new();
    editor
        .lua_host
        .eval(Some("@m6_4_test"), chunk)
        .expect("test chunk runs");
}

/// Construct a fresh editor and return a captured value (for tests
/// that want to do final assertions on the Rust side).
fn run_returning<T: mlua::FromLuaMulti>(chunk: &str) -> T {
    let mut editor = EditorState::new();
    editor
        .lua_host
        .eval(Some("@m6_4_test"), chunk)
        .and_then(|v| T::from_lua_multi(mlua::MultiValue::from_iter([v]), editor.lua_host.lua()))
        .expect("test chunk runs and returns")
}

// ---------------------------------------------------------------------------
// Acceptance bullet 1: three-region structure
// ---------------------------------------------------------------------------

#[test]
fn m6_4_three_regions_render_correctly() {
    // Build a REPL, push some history, set a prompt, type input.
    // The buffer should hold history then prompt then input, in
    // order, with no escape bytes from the parser.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("welcome to pmacs\n")
        h:set_prompt("$ ")
        local buf = h:buffer_id()
        buf:insert(h:prompt_end(), "echo hi")

        -- Verify the regions:
        --   [0, history_end)              = "welcome to pmacs\n"
        --   [history_end, prompt_end)     = "$ "
        --   [prompt_end, buf:len())       = "echo hi"
        local history = buf:slice(0, h:history_end())
        local prompt  = buf:slice(h:history_end(), h:prompt_end())
        local input   = buf:slice(h:prompt_end(), buf:len())
        assert(history == "welcome to pmacs\n",
               "history: " .. history)
        assert(prompt == "$ ", "prompt: " .. prompt)
        assert(input == "echo hi", "input: " .. input)
    "#);
}

#[test]
fn m6_4_appending_output_extends_history() {
    // Each append moves history_end and prompt_end forward by the
    // appended byte count. Input region (relative to prompt_end)
    // remains intact.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:set_prompt("> ")
        local buf = h:buffer_id()
        buf:insert(h:prompt_end(), "ls -la")
        local before_history = h:history_end()
        local before_prompt  = h:prompt_end()
        local before_input   = h:input_text()

        h:append_output("first line\n")
        assert(h:history_end() == before_history + #"first line\n",
               "history_end shifted by appended length")
        assert(h:prompt_end() == before_prompt + #"first line\n",
               "prompt_end shifted by same amount")
        assert(h:input_text() == before_input,
               "input text preserved across append: '" .. h:input_text() .. "'")

        h:append_output("second\n")
        assert(h:history_end() == before_history + #"first line\n" + #"second\n",
               "history_end shifted again")
        assert(h:input_text() == "ls -la",
               "input still typed text after second append")
    "#);
}

#[test]
fn m6_4_set_prompt_replaces_prompt_region() {
    // set_prompt swaps the bytes in [history_end, prompt_end) with
    // the new prompt text. Old prompt text is gone from the buffer.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("hello\n")
        h:set_prompt("user@host:~$ ")
        local buf = h:buffer_id()
        local content_first = buf:slice(0, buf:len())
        assert(content_first == "hello\nuser@host:~$ ",
               "first prompt set: " .. content_first)

        h:set_prompt("> ")
        local content_second = buf:slice(0, buf:len())
        assert(content_second == "hello\n> ",
               "second prompt replaced first: " .. content_second)
        assert(not content_second:find("user@host"), "old prompt gone")
    "#);
}

#[test]
fn m6_4_region_boundaries_are_mark_backed_across_input_edits() {
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("history\n")
        h:set_prompt("$ ")
        local buf = h:buffer_id()
        local prompt_before = h:prompt_end()

        -- User edits at the input boundary must not drag the prompt
        -- boundary forward. This is the failure mode byte-offset
        -- mirrors hid before real marks existed.
        buf:insert(prompt_before, "abc")
        assert(h:prompt_end() == prompt_before,
               "prompt_end moved across input insert")
        assert(h:input_text() == "abc", "input text after insert")

        h:append_output("more\n")
        assert(h:input_text() == "abc",
               "input preserved after output before prompt")
        local history = buf:slice(0, h:history_end())
        assert(history == "history\nmore\n",
               "history after append: " .. history)
    "#);
}

#[test]
fn m6_4_osc_133_prompt_markers_route_text_to_prompt_region() {
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("\27]133;A\7$ \27]133;B\7")
        local buf = h:buffer_id()

        assert(h:history_end() == 0,
               "prompt marker text must not enter history")
        assert(buf:slice(h:history_end(), h:prompt_end()) == "$ ",
               "prompt region should hold shell prompt")

        buf:insert(h:prompt_end(), "typed")
        h:append_output("out\n")
        assert(buf:slice(0, h:history_end()) == "out\n",
               "later output should enter history")
        assert(buf:slice(h:history_end(), h:prompt_end()) == "$ ",
               "prompt remains prompt after history output")
        assert(h:input_text() == "typed", "input preserved")
    "#);
}

// ---------------------------------------------------------------------------
// Acceptance bullet 3: read-only enforcement
// ---------------------------------------------------------------------------

#[test]
fn m6_4_intercept_rejects_history_edits() {
    // An insert at any position < history_end raises. The buffer is
    // unchanged.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("READONLY\n")
        h:set_prompt("> ")
        local buf = h:buffer_id()
        local before = buf:slice(0, buf:len())
        local ok, err = pcall(function()
            buf:insert(0, "X")  -- inside history region
        end)
        assert(not ok, "history insert must be rejected")
        -- "read-only" contains a `-` which is a Lua-pattern magic
        -- char; use the plain-string form of `find`.
        assert(tostring(err):find("read-only", 1, true),
               "error must name read-only: " .. tostring(err))
        assert(buf:slice(0, buf:len()) == before,
               "buffer unchanged after rejected edit")
    "#);
}

#[test]
fn m6_4_intercept_rejects_prompt_edits() {
    // Insert at a position in [history_end, prompt_end) is also
    // rejected (prompt region is read-only).
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("hi\n")
        h:set_prompt("$$ ")
        local buf = h:buffer_id()
        local before = buf:slice(0, buf:len())
        -- A position strictly inside the prompt region:
        local mid_prompt = h:history_end() + 1
        assert(mid_prompt < h:prompt_end(), "test geometry sanity")
        local ok, err = pcall(function()
            buf:insert(mid_prompt, "X")
        end)
        assert(not ok, "prompt insert must be rejected")
        assert(buf:slice(0, buf:len()) == before, "buffer unchanged")
    "#);
}

#[test]
fn m6_4_input_edits_pass_through() {
    // Insert in [prompt_end, buf:len()] is permitted: the user is
    // typing into the input region. Multiple inserts compose.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:set_prompt("> ")
        local buf = h:buffer_id()
        buf:insert(h:prompt_end(), "first")
        buf:insert(buf:len(), " second")
        assert(h:input_text() == "first second",
               "input region accepts pass-through edits: " .. h:input_text())
    "#);
}

// ---------------------------------------------------------------------------
// Acceptance bullet 4: cross-boundary truncation
// ---------------------------------------------------------------------------

#[test]
fn m6_4_paste_spanning_input_boundary_truncated_to_input() {
    // A replace with start < prompt_end <= end is truncated: the
    // range becomes [prompt_end, end). The replacement bytes pass
    // through unchanged (per the M6.4 byte-immutability rule on
    // LuaInterceptView).
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:set_prompt("$ ")
        local buf = h:buffer_id()
        buf:insert(h:prompt_end(), "abcde")  -- input region: "abcde"

        -- Now paste 5 bytes spanning the prompt boundary by replacing
        -- the last byte of the prompt and the first 3 bytes of input.
        --   Original range: [prompt_end - 1, prompt_end + 3)
        --   After truncate: [prompt_end,     prompt_end + 3)
        --   Replacement bytes: "WXYZQ" (5 bytes); they all land,
        --   replacing the 3 input bytes "abc" → "WXYZQ".
        local prompt_end_before = h:prompt_end()
        buf:replace(prompt_end_before - 1, prompt_end_before + 3, "WXYZQ")

        -- Prompt region must be intact.
        local prompt_chars = buf:slice(h:history_end(), prompt_end_before)
        assert(prompt_chars == "$ ",
               "prompt region preserved: '" .. prompt_chars .. "'")
        -- Input region is "WXYZQ" + "de" (the trailing input bytes
        -- that were past the truncated range).
        assert(h:input_text() == "WXYZQde",
               "input region after truncated paste: '" .. h:input_text() .. "'")
    "#);
}

#[test]
fn m6_4_delete_spanning_input_boundary_truncated_to_input() {
    // A delete with start < prompt_end <= end is truncated to
    // [prompt_end, end). No bytes are involved (Delete carries no
    // bytes), so this is lossless.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:set_prompt("XX ")  -- 3-byte prompt
        local buf = h:buffer_id()
        buf:insert(h:prompt_end(), "abcdef")
        local prompt_end_before = h:prompt_end()

        -- Delete spanning the boundary: [prompt_end - 2, prompt_end + 2).
        -- Truncated to [prompt_end, prompt_end + 2): drops "ab".
        buf:delete(prompt_end_before - 2, prompt_end_before + 2)

        local prompt_chars = buf:slice(h:history_end(), prompt_end_before)
        assert(prompt_chars == "XX ", "prompt preserved: '" .. prompt_chars .. "'")
        assert(h:input_text() == "cdef",
               "input after truncated delete: '" .. h:input_text() .. "'")
    "#);
}

// ---------------------------------------------------------------------------
// M6.3 ↔ M6.4 boundary regressions
// ---------------------------------------------------------------------------

#[test]
fn m6_4_ansi_styled_output_routes_to_history_only() {
    // SGR-coded output flows through the parser. The rope receives
    // only the literal text (no escape bytes). The set-style events
    // are observed via the parser's event stream when feed runs;
    // M6.4 captures the latest style on the handle (M6.5+ will
    // render it). The check here is the parser-rope contract: the
    // rope content matches the parser's literal-text emission.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        -- "\27[31mhello\27[0m world" → text "hello world", styles
        -- (red set, default reset) on the side channel.
        h:append_output("\27[31mhello\27[0m world")
        local buf = h:buffer_id()
        local content = buf:slice(0, buf:len())
        assert(content == "hello world",
               "rope holds only literal text, no escape bytes: '"
               .. content .. "'")
        assert(not content:find("\27"),
               "rope must not contain ESC bytes")
        local spans = h:style_spans()
        assert(#spans == 1, "expected one red style span, got " .. #spans)
        assert(spans[1].start == 0 and spans[1]["end"] == 5,
               "red span should cover hello, got [" .. spans[1].start ..
               "," .. spans[1]["end"] .. ")")
        assert(spans[1].style.fg == 1,
               "red span should carry palette fg=1")
    "#);
}

#[test]
fn m6_4_line_level_ansi_updates_history_in_place() {
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("progress 10%")
        h:append_output("\rprogress 20%\27[K")
        local content = h:buffer_id():slice(0, h:buffer_id():len())
        assert(content == "progress 20%",
               "CR overwrite + erase-to-EOL should update in place: '" ..
               content .. "'")

        h:append_output("\r\27[2Kdone\n")
        content = h:buffer_id():slice(0, h:buffer_id():len())
        assert(content == "done\n",
               "erase-line should clear current line before rewrite: '" ..
               content .. "'")

        h:append_output("abc\bZ")
        content = h:buffer_id():slice(0, h:buffer_id():len())
        assert(content == "done\nabZ",
               "backspace should rewind within current line: '" ..
               content .. "'")
    "#);
}

#[test]
fn m6_4_alt_screen_suppresses_history_writes() {
    // Bytes between alt-screen-enter and alt-screen-exit do not land
    // in history. Bytes before and after do. The handle exposes the
    // alt-screen flag so packages can render an "alt-screen active"
    // indicator (a M6.5 concern; we just lock in the flag's value).
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("before-")
        assert(not h:alt_screen_active(), "alt screen initially inactive")
        h:append_output("\27[?1049h")
        assert(h:alt_screen_active(), "alt screen active after enter")
        h:append_output("INSIDE_ALT")  -- suppressed by parser
        h:append_output("\27[?1049l")
        assert(not h:alt_screen_active(), "alt screen inactive after exit")
        h:append_output("-after")

        local content = h:buffer_id():slice(0, h:buffer_id():len())
        assert(content == "before--after",
               "alt-screen body suppressed: '" .. content .. "'")
        assert(not content:find("INSIDE_ALT"),
               "alt-screen body must not appear in history")
    "#);
}

#[test]
fn m6_4_submit_does_not_append_to_history() {
    // `submit()` returns the input region's text and clears it.
    // The popped text must NOT land in history; M6.5 will let the
    // shell echo the input back (in raw mode), and the parser will
    // append the echo to history. Doing it in submit too would
    // double the input. Lock that contract in here so M6.5 can wire
    // submit's return to pmacs.process.write without any
    // detect-and-suppress logic.
    let final_state: String = run_returning(
        r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("welcome\n")
        h:set_prompt("> ")
        local buf = h:buffer_id()
        buf:insert(h:prompt_end(), "echo hi")
        local popped = h:submit()
        assert(popped == "echo hi",
               "submit returns the input text: '" .. popped .. "'")
        assert(h:input_text() == "",
               "input region empty after submit: '" .. h:input_text() .. "'")
        -- Critical: history did not grow by len("echo hi") just because
        -- of submit. The buffer's content is exactly the prefix that
        -- was there before the user started typing.
        return buf:slice(0, buf:len())
    "#,
    );
    assert_eq!(
        final_state, "welcome\n> ",
        "post-submit buffer must not contain submitted text in history"
    );
}

// ---------------------------------------------------------------------------
// Sanity: handle types thread through Rust-side machinery
// ---------------------------------------------------------------------------

#[test]
fn m6_4_buffer_id_is_a_real_buffer_handle() {
    // The handle's `buffer_id()` returns a real BufferIdLua that
    // resolves through the registry. This locks in that the package
    // is built atop genuinely-public surface; no shadow APIs.
    let mut editor = EditorState::new();
    let returned: mlua::Value = editor
        .lua_host
        .eval(
            Some("@m6_4_handle_test"),
            r#"
            local h = pmacs.repl.create({ name = "*handle*" })
            return h:buffer_id()
            "#,
        )
        .expect("eval");
    let id_lua = match returned {
        mlua::Value::UserData(ud) => *ud.borrow::<BufferIdLua>().expect("BufferIdLua userdata"),
        other => panic!("expected BufferIdLua userdata, got {other:?}"),
    };
    let r = editor.lua_host.registry().borrow();
    let buf = r.get(id_lua.id()).expect("registered");
    assert_eq!(buf.name(), "*handle*");
}
