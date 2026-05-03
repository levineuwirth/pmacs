// m6_7_scrollback_acceptance.rs --- T M6.7 acceptance gates.

//! Acceptance gates for T M6.7 (Scrollback management).
//!
//! Spec §sec:repl-perf defines three gates:
//!
//! 1. Navigation latency p99 ≤ 16 ms across 10000-line scrollback.
//! 2. Search latency p99 ≤ 100 ms across 10000 lines.
//! 3. Truncation under memory pressure preserves logical region
//!    boundaries (no partial command-output blocks; no input-region
//!    truncation).
//!
//! Gates 1 and 2 are perf gates and live in
//! [`tests/m6_perf_acceptance.rs`] under the `#[ignore]` /
//! `--release` pattern locked by M5.9c. This file covers gate 3 (a
//! correctness test, not a perf test) plus the lib-level invariants
//! that anchor it: pre-first-submit block, no zero-length blocks on
//! submit, single-pass truncation, and rope/_blocks consistency.
//!
//! Test process choice: the Lua REPL (mlua's interpreter) is the
//! deterministic stand-in. Bash/zsh/fish prompts vary; the lua REPL
//! prints predictable text and is always available because lua is
//! a build dep. We don't actually need a process for the
//! invariant tests --- `pmacs.repl.create` + synthetic
//! `append_output` exercises the same scrollback path.

use pmacs::editor::EditorState;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn run(chunk: &str) {
    let mut editor = EditorState::new();
    editor
        .lua_host
        .eval(Some("@m6_7_test"), chunk)
        .expect("test chunk runs");
}

// ---------------------------------------------------------------------------
// Block-tracking invariants
// ---------------------------------------------------------------------------

#[test]
fn m6_7_pre_first_submit_block_exists_with_start_byte_zero() {
    // The first block is real and degenerate: it covers any bytes
    // received before the first user submit. Without this, a process
    // that produces a long preamble before its first prompt has no
    // block boundary to use as a truncation point.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        assert(#h._blocks == 1, "expected 1 block, got " .. #h._blocks)
        assert(h._blocks[1].start_byte == 0,
               "expected start_byte 0, got " .. h._blocks[1].start_byte)
    "#);
}

#[test]
fn m6_7_emit_history_extends_active_block() {
    // Bytes flowing through append_output extend the active block's
    // span by extending _history_end (which acts as the block-end
    // for the active block). The pre-first-submit block is active
    // until the first submit.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("line one\nline two\nline three\n")
        assert(#h._blocks == 1, "still one block")
        assert(h._blocks[1].start_byte == 0,
               "block 1 start_byte should be 0")
        assert(h._history_end == #"line one\nline two\nline three\n",
               "history_end mismatch: " .. h._history_end)
    "#);
}

#[test]
fn m6_7_submit_opens_new_block_when_active_has_bytes() {
    // The submit boundary is what gives us a "complete command-output
    // block" to truncate. After the first submit (with bytes already
    // in history), block 2 opens at the current history_end.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("preamble\n")
        h:set_prompt("$ ")
        local buf = h:buffer_id()
        buf:insert(h:prompt_end(), "echo hi")
        local _ = h:submit()
        assert(#h._blocks == 2, "expected 2 blocks, got " .. #h._blocks)
        assert(h._blocks[2].start_byte == h._history_end,
               "block 2 should start at history_end")
    "#);
}

#[test]
fn m6_7_submit_with_no_history_bytes_does_not_open_block() {
    // Repeated submits before any output land has no boundary to
    // record. Adding zero-length blocks would violate the strictly-
    // increasing start_byte invariant; the right shape is to keep
    // the active block until it accumulates bytes.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:set_prompt("$ ")
        h:submit()
        h:submit()
        h:submit()
        assert(#h._blocks == 1,
               "expected 1 block after empty submits, got " .. #h._blocks)
        assert(h._blocks[1].start_byte == 0)
    "#);
}

#[test]
fn m6_7_blocks_strictly_increasing_after_mixed_submits() {
    // Strictly-increasing invariant under a realistic interleave:
    // each submit either opens a block (when bytes accumulated) or
    // doesn't (when none did). The Lua chunk does the assertions.
    run(r#"
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("welcome\n")
        h:set_prompt("$ ")
        h:submit()                    -- opens block 2
        h:submit()                    -- no-op; block 2 is empty
        h:append_output("output a\n") -- accumulates in block 2
        h:submit()                    -- opens block 3
        h:append_output("output b\n") -- accumulates in block 3
        h:submit()                    -- opens block 4
        assert(#h._blocks == 4,
               "expected 4 blocks, got " .. #h._blocks)
        for i = 1, #h._blocks - 1 do
            assert(h._blocks[i].start_byte < h._blocks[i + 1].start_byte,
                   "blocks not strictly increasing at i=" .. i)
        end
    "#);
}

// ---------------------------------------------------------------------------
// Truncation invariants
// ---------------------------------------------------------------------------

#[test]
fn m6_7_truncate_drops_oldest_block_when_line_limit_exceeded() {
    // With scrollback_lines = 2 and three populated blocks of one
    // line each, _maybe_truncate should drop block 1 (and possibly
    // block 2 depending on whether the active block counts toward
    // the limit). Verify the line count drops and the buffer shrinks
    // by the corresponding bytes.
    run(r#"
        local function count_newlines(s)
            local n, i = 0, 0
            while true do
                i = s:find("\n", i + 1, true)
                if not i then return n end
                n = n + 1
            end
        end
        pmacs.repl.config.scrollback_lines = 2
        pmacs.repl.config.scrollback_bytes = 1024 * 1024 * 1024
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("aaaa\n")
        h:set_prompt("$ ")
        h:submit()
        h:append_output("bbbb\n")
        h:submit()
        h:append_output("cccc\n")
        local pre_lines = count_newlines(h:buffer_id():slice(0, h._history_end))
        assert(pre_lines == 3, "pre-truncate lines: " .. pre_lines)
        h:_maybe_truncate()
        local post_lines = count_newlines(h:buffer_id():slice(0, h._history_end))
        assert(post_lines <= 2,
               "post-truncate lines should be <= 2, got " .. post_lines)
        assert(h._blocks[1].start_byte == 0,
               "block 1 start_byte should be 0 after normalize, got "
               .. h._blocks[1].start_byte)
        pmacs.repl.config.scrollback_lines = 10000
    "#);
}

#[test]
fn m6_7_truncate_drops_oldest_block_when_byte_limit_exceeded() {
    // With scrollback_bytes set tight, byte invariant fires even when
    // the line invariant holds.
    run(r#"
        pmacs.repl.config.scrollback_lines = 1000000
        pmacs.repl.config.scrollback_bytes = 12   -- 12 bytes
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("aaaaa\n")  -- block 1: 6 bytes
        h:set_prompt("$ ")
        h:submit()
        h:append_output("bbbbb\n")  -- block 2: 6 bytes
        h:submit()
        h:append_output("cccc\n")   -- block 3 (active): 5 bytes; total 17
        assert(h._history_end == 17, "pre-truncate end: " .. h._history_end)
        h:_maybe_truncate()
        assert(h._history_end <= 12,
               "post-truncate end should be <= 12, got " .. h._history_end)
        assert(h._blocks[1].start_byte == 0)
        pmacs.repl.config.scrollback_bytes = 16 * 1024 * 1024
    "#);
}

#[test]
fn m6_7_truncate_never_removes_active_block() {
    // Even with both limits set absurdly tight, the active (last)
    // block is preserved. Truncation cannot leave a handle with no
    // block to accumulate into.
    run(r#"
        pmacs.repl.config.scrollback_lines = 0
        pmacs.repl.config.scrollback_bytes = 0
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("only\n")  -- single block, active
        h:_maybe_truncate()
        assert(#h._blocks == 1,
               "active block must survive, got " .. #h._blocks .. " blocks")
        pmacs.repl.config.scrollback_lines = 10000
        pmacs.repl.config.scrollback_bytes = 16 * 1024 * 1024
    "#);
}

#[test]
fn m6_7_truncate_preserves_input_region() {
    // The user's in-flight input (past prompt_end) is the load-bearing
    // safety property: losing output is annoying, losing what the user
    // is typing is unacceptable. Even aggressive truncation leaves
    // input bytes intact.
    run(r#"
        pmacs.repl.config.scrollback_lines = 1
        local h = pmacs.repl.create({ name = "*test*" })
        h:append_output("history line\n")
        h:set_prompt("$ ")
        h:submit()
        h:append_output("more history\n")
        local buf = h:buffer_id()
        buf:insert(buf:len(), "user typing")
        local pre_input = buf:slice(h:prompt_end(), buf:len())
        assert(pre_input == "user typing")
        h:_maybe_truncate()
        local post_input = buf:slice(h:prompt_end(), buf:len())
        assert(post_input == "user typing",
               "input region truncated: " .. post_input)
        pmacs.repl.config.scrollback_lines = 10000
    "#);
}

#[test]
fn m6_7_truncate_keeps_rope_and_blocks_consistent() {
    // After truncation, every block.start_byte is a valid index in
    // the rope, the active block's start_byte is at most history_end,
    // and the array is strictly increasing.
    run(r#"
        pmacs.repl.config.scrollback_lines = 2
        local h = pmacs.repl.create({ name = "*test*" })
        for i = 1, 5 do
            h:append_output("line " .. i .. "\n")
            h:set_prompt("$ ")
            h:submit()
        end
        h:_maybe_truncate()
        for i, b in ipairs(h._blocks) do
            assert(b.start_byte >= 0 and b.start_byte <= h._history_end,
                   "block " .. i .. " start_byte " .. b.start_byte
                   .. " out of range [0, " .. h._history_end .. "]")
        end
        assert(h._blocks[1].start_byte == 0)
        for i = 1, #h._blocks - 1 do
            assert(h._blocks[i].start_byte < h._blocks[i + 1].start_byte,
                   "blocks not strictly increasing")
        end
        pmacs.repl.config.scrollback_lines = 10000
    "#);
}

// ---------------------------------------------------------------------------
// Spec gate: 50 truncation events with varied retention
// ---------------------------------------------------------------------------

#[test]
fn m6_7_50_truncation_events_preserve_block_boundaries() {
    // The spec gate: 50 truncation events under varied retention
    // values, asserting the four invariants on each event:
    //   (a) first block start_byte == 0
    //   (b) blocks strictly increasing
    //   (c) input region not truncated
    //   (d) rope and blocks consistent
    //
    // Each event is one truncation pass with a distinct retention
    // value. Varying retention sweeps through the parameter space:
    // tight line limits (forces line-driven truncation), tight byte
    // limits (forces byte-driven truncation), and mixed cases where
    // both fire.
    run(r#"
        local function check_invariants(h, label)
            assert(#h._blocks >= 1, label .. ": at least one block")
            assert(h._blocks[1].start_byte == 0,
                   label .. ": block 1 start_byte == 0")
            for i = 1, #h._blocks - 1 do
                assert(h._blocks[i].start_byte < h._blocks[i + 1].start_byte,
                       label .. ": blocks strictly increasing at i=" .. i)
            end
            for i, b in ipairs(h._blocks) do
                assert(b.start_byte >= 0 and b.start_byte <= h._history_end,
                       label .. ": block " .. i .. " start_byte out of range")
            end
        end

        for trial = 1, 50 do
            -- Vary retention: trial 1..25 sweep line limits 1..25;
            -- trial 26..50 sweep byte limits 8..208 (8-byte stride).
            if trial <= 25 then
                pmacs.repl.config.scrollback_lines = trial
                pmacs.repl.config.scrollback_bytes = 16 * 1024 * 1024
            else
                pmacs.repl.config.scrollback_lines = 10000
                pmacs.repl.config.scrollback_bytes = 8 + (trial - 26) * 8
            end

            local h = pmacs.repl.create({ name = "*trial-" .. trial .. "*" })
            -- Populate ~30 blocks of varying size so truncation has
            -- meaningful work to do.
            for i = 1, 30 do
                h:append_output("block-" .. i .. "-output\n")
                h:set_prompt("$ ")
                h:submit()
            end
            -- Add some user-typed input that must survive truncation.
            local buf = h:buffer_id()
            local marker = "USER-INPUT-" .. trial
            buf:insert(buf:len(), marker)
            local pre_input = buf:slice(h:prompt_end(), buf:len())
            assert(pre_input == marker, "trial " .. trial .. ": pre_input setup")

            h:_maybe_truncate()

            check_invariants(h, "trial " .. trial)
            local post_input = buf:slice(h:prompt_end(), buf:len())
            assert(post_input == marker,
                   "trial " .. trial .. ": input region truncated; got "
                   .. post_input)
        end

        pmacs.repl.config.scrollback_lines = 10000
        pmacs.repl.config.scrollback_bytes = 16 * 1024 * 1024
    "#);
}
