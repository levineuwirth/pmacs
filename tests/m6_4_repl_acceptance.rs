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
    let mut editor = EditorState::new_with_roots(&crate::iso::roots());
    editor
        .lua_host
        .eval(Some("@m6_4_test"), chunk)
        .expect("test chunk runs");
}

/// Construct a fresh editor and return a captured value (for tests
/// that want to do final assertions on the Rust side).
fn run_returning<T: mlua::FromLuaMulti>(chunk: &str) -> T {
    let mut editor = EditorState::new_with_roots(&crate::iso::roots());
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

/// What the history holds after `bytes` through `h:append_output`, with no
/// process, for each row of a table, and the rows that read otherwise.
fn history_table_misses(rows: &[(&str, &str)]) -> Vec<String> {
    use std::fmt::Write as _;
    let lua_literal = |s: &str| {
        let mut literal = String::from("\"");
        for b in s.bytes() {
            write!(literal, "\\{b:03}").expect("a String takes the write");
        }
        literal.push('"');
        literal
    };
    rows.iter()
        .filter_map(|&(bytes, want)| {
            let got: mlua::String = run_returning(&format!(
                r#"
                local h = pmacs.repl.create({{ name = "*test*" }})
                h:append_output({})
                return h:buffer_id():slice(0, h:history_end())
                "#,
                lua_literal(bytes),
            ));
            let got = got.to_string_lossy();
            (got != want).then(|| format!("{bytes:?} reads {got:?}, not {want:?}"))
        })
        .collect()
}

/// An LF moves to the next line, and a CR before it leaves the line standing:
/// `src/ansi.rs` hands the LF over as text, and written at `_output_pos` after a
/// CR it overwrote the line it ends. fish turns `OPOST|ONLCR` back on before it
/// runs a command (the trampoline's `stty raw` had turned them off), so its
/// answers arrive CR LF and did not stay.
#[test]
fn m6_4_an_lf_after_a_cr_moves_and_does_not_overwrite() {
    let misses = history_table_misses(&[
        ("hello-pmacs\r\n", "hello-pmacs\n"),
        ("one\r\ntwo\r\n", "one\ntwo\n"),
        ("abc\rX\n", "Xbc\n"),
        ("abc\rX\nY\n", "Xbc\nY\n"),
        ("abc\r\n\r\ndef", "abc\n\ndef"),
    ]);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// What the row above does not change: a CR still overwrites what it comes
/// back over, and an LF at the history's end still appends.
#[test]
fn m6_4_a_cr_still_overwrites_and_an_lf_at_the_end_still_appends() {
    let misses = history_table_misses(&[
        ("abc\rX", "Xbc"),
        ("abc\rXYZW\n", "XYZW\n"),
        ("abc\n\rX", "abc\nX"),
        ("hello-pmacs\n", "hello-pmacs\n"),
        ("one\ntwo\n", "one\ntwo\n"),
    ]);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// A CR finds its line's start by looking back over segments that double from
/// 256 bytes: lines either side of the first, and one with no line before it.
#[test]
fn m6_4_a_cr_finds_its_line_at_any_length() {
    let rows: Vec<(String, String)> = [1, 255, 256, 257, 600, 5000]
        .into_iter()
        .flat_map(|len| {
            let line = "y".repeat(len);
            let rest = &line[1..];
            [
                (format!("head\n{line}\rZ"), format!("head\nZ{rest}")),
                (format!("{line}\rZ"), format!("Z{rest}")),
            ]
        })
        .collect();
    let rows: Vec<(&str, &str)> = rows.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let misses = history_table_misses(&rows);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// One row of [`reads_misses`]: the reads a handle takes, in order, and the
/// history it should hold after them.
struct Row {
    name: String,
    reads: Vec<Vec<u8>>,
    want: Vec<u8>,
}

/// A read that runs `lua` between the others, with `h` (the handle) and `M`
/// (`pmacs.repl`) bound, in place of output.
fn lua_step(lua: &str) -> Vec<u8> {
    let mut read = b"\0lua:".to_vec();
    read.extend_from_slice(lua.as_bytes());
    read
}

/// A short form of a long history, for a message.
fn shown(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.chars().count() <= 80 {
        return format!("{s:?}");
    }
    let head: String = s.chars().take(30).collect();
    let tail: Vec<char> = s.chars().rev().take(30).collect();
    let tail: String = tail.into_iter().rev().collect();
    format!("{head:?} … ({} bytes) … {tail:?}", bytes.len())
}

/// Each row's reads through its own `h:append_output` calls on a fresh handle,
/// as a PTY's reads cut a program's output, with no process; the rows whose
/// history reads otherwise.
fn reads_misses(rows: &[Row]) -> Vec<String> {
    let editor = EditorState::new_with_roots(&crate::iso::roots());
    let lua = editor.lua_host.lua();
    let replay: mlua::Function = lua
        .load(
            r#"
            local reads, n = ...
            local h = pmacs.repl.create({ name = "*reads-" .. n .. "*" })
            for _, read in ipairs(reads) do
              if read:sub(1, 5) == "\0lua:" then
                local env = setmetatable({ h = h, M = pmacs.repl }, { __index = _G })
                assert(load(read:sub(6), "=step", "t", env))()
              else
                h:append_output(read)
              end
            end
            return h:buffer_id():slice(0, h:history_end())
            "#,
        )
        .into_function()
        .expect("the replay compiles");
    let mut misses = Vec::new();
    for (n, row) in rows.iter().enumerate() {
        let reads = lua.create_table().expect("a table");
        for (i, read) in row.reads.iter().enumerate() {
            reads
                .set(i + 1, lua.create_string(read).expect("a string"))
                .expect("a read goes in");
        }
        let got: mlua::String = replay.call((reads, n)).expect("the row runs");
        if *got.as_bytes() != row.want[..] {
            misses.push(format!(
                "{}: reads {}, not {}",
                row.name,
                shown(&got.as_bytes()),
                shown(&row.want)
            ));
        }
    }
    misses
}

fn cut(name: &str, reads: &[&[u8]], want: &[u8]) -> Row {
    Row {
        name: name.to_owned(),
        reads: reads.iter().map(|r| r.to_vec()).collect(),
        want: want.to_vec(),
    }
}

/// A CR and the LF after it arrive in separate reads as often as in one, and a
/// read can end on either; the line stays whichever way they are cut. E10m
/// review 1's probe.
#[test]
fn m6_4_a_cr_lf_cut_across_reads_keeps_the_line() {
    let misses = reads_misses(&[
        cut("CR | LF", &[b"abc\r", b"\n"], b"abc\n"),
        cut("text | CR LF", &[b"abc", b"\r\n"], b"abc\n"),
        cut("CR | text | LF", &[b"abc\r", b"X", b"\n"], b"Xbc\n"),
        cut(
            "a word cut, CR | LF, the next line",
            &[b"hello-pm", b"acs\r", b"\n", b"next\r\n"],
            b"hello-pmacs\nnext\n",
        ),
        cut("CR text | text LF text", &[b"abc\rX", b"Y\nZ"], b"XYc\nZ"),
        cut("two LFs after a CR", &[b"abc\rX\nY\nZ"], b"Xbc\nY\nZ"),
        cut("CR LF on an empty history", &[b"\r\n"], b"\n"),
        cut("CR CR LF, twice", &[b"\r\r\nA\r\r\nB"], b"\nA\nB"),
        cut("CR | CR | LF", &[b"abc\r", b"\r", b"\n"], b"abc\n"),
    ]);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// A CR whose line began far further back than the first segment: the
/// look-back reaches the line's start, or the history's, however the line came.
/// E10m review 1's probe.
#[test]
fn m6_4_a_cr_far_past_the_first_segment_finds_its_line() {
    let long = "y".repeat(100_000);
    let mut over_reads: Vec<Vec<u8>> = vec![b"head\n".to_vec()];
    over_reads.extend((0..25).map(|_| "y".repeat(4096).into_bytes()));
    over_reads.push(b"\r".to_vec());
    over_reads.push(b"Z\r\n".to_vec());
    let misses = reads_misses(&[
        Row {
            name: "100,000 bytes after an LF".into(),
            reads: vec![format!("head\n{long}\rZ\n").into_bytes()],
            want: format!("head\nZ{}\n", &long[1..]).into_bytes(),
        },
        Row {
            name: "100,000 bytes, no LF before".into(),
            reads: vec![format!("{long}\rZ\n").into_bytes()],
            want: format!("Z{}\n", &long[1..]).into_bytes(),
        },
        Row {
            name: "102,400 bytes over 25 reads, CR | Z CR LF".into(),
            reads: over_reads,
            want: format!("head\nZ{}\n", "y".repeat(25 * 4096 - 1)).into_bytes(),
        },
    ]);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// A segment's first byte can fall inside a character; the look-back reads
/// bytes for an LF, which no continuation byte is. Here the first segment
/// starts inside the 173rd `é`. E10m review 1's probe.
#[test]
fn m6_4_multibyte_text_across_a_segment_edge() {
    let line = format!("{}x", "é".repeat(300));
    let mut want = b"head\nZ".to_vec();
    want.extend_from_slice(&line.as_bytes()[1..]);
    want.push(b'\n');
    let misses = reads_misses(&[Row {
        name: "300 x é, then x, CR Z LF".into(),
        reads: vec![format!("head\n{line}\rZ\n").into_bytes()],
        want,
    }]);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// A backspace and both erases on a line longer than the first segment find
/// the line's start by the same look-back as a CR. E10m review 1's probe.
#[test]
fn m6_4_backspace_and_erase_on_a_long_line() {
    let y600 = "y".repeat(600);
    let misses = reads_misses(&[
        Row {
            name: "BS BS at a long line's start".into(),
            reads: vec![format!("head\n{y600}\r\x08\x08Z").into_bytes()],
            want: format!("head\nZ{}", &y600[1..]).into_bytes(),
        },
        Row {
            name: "BS BS at a long line's end".into(),
            reads: vec![format!("head\n{y600}\x08\x08Z").into_bytes()],
            want: format!("head\n{}Zy", &y600[2..]).into_bytes(),
        },
        Row {
            name: "erase line on a long line, then Z LF".into(),
            reads: vec![format!("head\n{y600}\x1b[2KZ\n").into_bytes()],
            want: b"head\nZ\n".to_vec(),
        },
        Row {
            name: "CR, AB, erase to end of line, LF".into(),
            reads: vec![format!("head\n{y600}\rAB\x1b[K\n").into_bytes()],
            want: b"head\nAB\n".to_vec(),
        },
    ]);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// The scrollback truncation drops the block a CR's line began in: the line
/// `aaaabbbb` spans the first block and the active one, a CR goes to its
/// start, and the first block goes; the LF after the overwrite still moves.
/// E10m review 1's probe.
#[test]
fn m6_4_truncation_drops_the_block_a_crs_line_began_in() {
    let misses = reads_misses(&[Row {
        name: "aaaa | submit | bbbb CR | truncate to 4 bytes | X LF".into(),
        reads: vec![
            b"aaaa".to_vec(),
            lua_step("h:submit()"),
            b"bbbb\r".to_vec(),
            lua_step(
                "local saved = M.config.scrollback_bytes; M.config.scrollback_bytes = 4; \
                 h:_maybe_truncate(); M.config.scrollback_bytes = saved",
            ),
            b"X\n".to_vec(),
        ],
        want: b"Xbbb\n".to_vec(),
    }]);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// The look-back's segments cover 256, then 512, 1,024 and 2,048 bytes back;
/// a line whose LF stands at each one's first byte and its last, with earlier
/// lines behind it, in the same segment when the LF stands at its last byte, so
/// that the LF found there is the segment's last and not its first.
#[test]
fn m6_4_an_lf_at_each_segments_edge_starts_the_line() {
    let before = "ab\n".repeat(2000);
    let rows: Vec<Row> = [255, 256, 767, 768, 1791, 1792, 3839, 3840]
        .into_iter()
        .map(|len| {
            let line = "y".repeat(len);
            Row {
                name: format!("a line of {len} bytes, its LF {} bytes back", len + 1),
                reads: vec![format!("{before}head\n{line}\rZ").into_bytes()],
                want: format!("{before}head\nZ{}", &line[1..]).into_bytes(),
            }
        })
        .collect();
    let misses = reads_misses(&rows);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// A backspace at a 1 MB line's end looks back over the whole line on every
/// frame, as a spinner of `BS Z` there does. The base read the history from its
/// start once and searched it forward; the look-back E10m.14 brought re-sliced
/// and reversed about twice the line, 6.5 to 9 times the base's (E10m review
/// 1's Low 4). Held here to the base's order: 200 frames, each one
/// `append_output`, against the base's look-back done by hand on the same
/// history 200 times, a slice from its start and a forward search, at most
/// three times as long, the best of five rounds each. With an LF before the
/// line and with none.
#[test]
#[ignore = "wall-clock budget (D12): run by scripts/perf-budgets and CI's perf jobs"]
fn m6_4_a_backspace_at_a_long_lines_end_stays_within_the_bases_order() {
    const BOUND: f64 = 3.0;
    let editor = EditorState::new_with_roots(&crate::iso::roots());
    let lua = editor.lua_host.lua();
    let prepare: mlua::Function = lua
        .load(
            r#"
            local setup = ...
            local h = pmacs.repl.create({ name = "*budget*" })
            h:append_output(setup)
            local buf = h:buffer_id()
            local function frames()
              for _ = 1, 200 do h:append_output("\bZ") end
              return h:history_end()
            end
            local function base_look_back()
              local start = 0
              for _ = 1, 200 do
                local prefix = buf:slice(0, h:history_end())
                start = 0
                local search = 1
                while true do
                  local idx = prefix:find("\n", search, true)
                  if not idx then break end
                  start = idx
                  search = idx + 1
                end
              end
              return start
            end
            return frames, base_look_back
            "#,
        )
        .into_function()
        .expect("the budget's chunk compiles");
    let line = "y".repeat(1_000_000);
    let mut over = Vec::new();
    for (case, setup) in [
        ("an LF, then a 1 MB line", format!("head\n{line}")),
        ("a 1 MB line, no LF before", line.clone()),
    ] {
        let (frames, base): (mlua::Function, mlua::Function) = prepare
            .call(lua.create_string(&setup).expect("a string"))
            .expect("the handle is set up");
        let (mut head_best, mut base_best) = (f64::MAX, f64::MAX);
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let _: i64 = frames.call(()).expect("the frames run");
            head_best = head_best.min(t.elapsed().as_secs_f64() * 1000.0);
            let t = std::time::Instant::now();
            let _: i64 = base.call(()).expect("the base's look-back runs");
            base_best = base_best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        let ratio = head_best / base_best;
        eprintln!(
            "m6_4 budget, {case}: 200 x BS Z {head_best:.1} ms, the base's look-back 200 times \
             {base_best:.1} ms, ratio {ratio:.2} (bound {BOUND})"
        );
        if ratio > BOUND {
            over.push(format!("{case}: ratio {ratio:.2} over {BOUND}"));
        }
    }
    assert!(over.is_empty(), "{}", over.join("\n"));
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
    let mut editor = EditorState::new_with_roots(&crate::iso::roots());
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

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
