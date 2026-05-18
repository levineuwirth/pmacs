// tests/m8_10_acceptance.rs --- T M8.10 outline-class aggregate buffer.

//! Acceptance for the M8.10 deliverable: an aggregate buffer kind
//! that selects matching entries from multiple outline source
//! buffers via a predicate, concatenates each match's source-byte
//! range into a single visible buffer, and routes user edits in
//! the aggregate back to source coordinates via `intercept_edit`.
//! Source-side edits propagate to the aggregate within one async
//! tick.
//!
//! The package source-of-record is `tests/fixtures/pmacs-outline/`
//! (`aggregate.lua` is the M8.10 module; M8.9's parser/view/nav
//! are unchanged and reused).
//!
//! The four spec acceptance bullets:
//!
//! 1. Aggregate query across five outline buffers returns matching
//!    entries.
//! 2. Editing a matched entry in the aggregate updates the source
//!    buffer's rope.
//! 3. Source buffer changes propagate to the aggregate within one
//!    tick.
//! 4. Cyclic dependencies (an aggregate that includes itself) are
//!    detected and rejected.
//!
//! Plus regressions:
//!
//! * close cleans up source-listener intercepts and write-back
//!   intercept; sources stay alive.
//! * Cross-block edits are rejected with a clear error.
//! * Multiple aggregates over overlapping sources don't interfere.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

fn outline_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests").join("fixtures").join("pmacs-outline")
}

fn editor_with_outline() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new();
    state.lua_host.reopen_init_phase_for_testing();
    state.lua_host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    let pkg = outline_package_path();
    let pkg_str = pkg.display().to_string();
    let install = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("pmacs-outline")
    "#
    );
    state
        .lua_host
        .eval(Some("outline-install"), &install)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));
    (state, cache, user_root)
}

fn agg_text(state: &mut EditorState) -> String {
    state
        .lua_host
        .lua()
        .load(r"return AGG.buffer:slice(0, AGG.buffer:len())")
        .eval()
        .expect("agg text")
}

// ---------------------------------------------------------------------------
// Bullet 1 --- aggregate over 5 sources returns matching entries
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_query_across_five_buffers_returns_matches() {
    let (mut state, _c, _u) = editor_with_outline();

    // Build 5 source buffers, each with three headlines: two tagged
    // :todo: and one tagged :done:. Total: 10 entries match :todo:,
    // 5 match :done:.
    state
        .lua_host
        .eval(
            Some("five-sources"),
            r#"
                local outline = require("pmacs-outline")
                _G.SRCS = {}
                for i = 1, 5 do
                    local s = pmacs.buffer.create("*src-" .. i .. "*")
                    s:replace(0, 0,
                        "* TODO-A-" .. i .. " :todo:\nbody-a\n" ..
                        "* TODO-B-" .. i .. " :todo:\nbody-b\n" ..
                        "* DONE-" .. i .. " :done:\nbody-d\n")
                    _G.SRCS[i] = s
                end
                _G.AGG = outline.aggregate(_G.SRCS, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("aggregate creation");

    // The aggregate text should contain all 10 :todo: headlines (2 per source x 5).
    let txt = agg_text(&mut state);
    let mut count = 0;
    for line in txt.lines() {
        if line.starts_with("* TODO-") {
            count += 1;
        }
    }
    assert_eq!(
        count, 10,
        "expected 10 TODO headlines across 5 sources; got {count}; agg text: {txt:?}"
    );

    // No DONE entries should appear.
    assert!(
        !txt.contains("DONE-"),
        ":done: entries must NOT appear in a :todo:-filtered aggregate; got: {txt:?}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 2 --- editing aggregate updates source rope
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_edit_propagates_to_source() {
    let (mut state, _c, _u) = editor_with_outline();

    state
        .lua_host
        .eval(
            Some("two-source-agg"),
            r#"
                local outline = require("pmacs-outline")
                _G.S1 = pmacs.buffer.create("*s1*")
                _G.S1:replace(0, 0, "* TODO 1 :todo:\nbody-1\n")
                _G.S2 = pmacs.buffer.create("*s2*")
                _G.S2:replace(0, 0, "* TODO 2 :todo:\nbody-2\n")
                _G.AGG = outline.aggregate({_G.S1, _G.S2}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    // Aggregate now has both TODOs concatenated. Find the byte
    // offset of "body-1" in the aggregate (it's inside the first
    // block); insert "X" before it.
    state
        .lua_host
        .eval(
            Some("edit-agg"),
            r#"
                local agg_text = AGG.buffer:slice(0, AGG.buffer:len())
                _G.BODY1_OFFSET = agg_text:find("body%-1") - 1  -- 0-indexed
                AGG.buffer:insert(BODY1_OFFSET, "X")
            "#,
        )
        .expect("aggregate edit");

    // Source S1 must now contain "Xbody-1".
    let s1_text: String = state
        .lua_host
        .lua()
        .load(r"return S1:slice(0, S1:len())")
        .eval()
        .expect("s1 text");
    assert!(
        s1_text.contains("Xbody-1"),
        "S1 must reflect the aggregate-side insertion; got: {s1_text:?}"
    );

    // S2 must remain untouched.
    let s2_text: String = state
        .lua_host
        .lua()
        .load(r"return S2:slice(0, S2:len())")
        .eval()
        .expect("s2 text");
    assert_eq!(
        s2_text, "* TODO 2 :todo:\nbody-2\n",
        "S2 must be unchanged; got: {s2_text:?}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 3 --- source change propagates to aggregate within one tick
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_source_change_propagates_within_one_tick() {
    let (mut state, _c, _u) = editor_with_outline();

    state
        .lua_host
        .eval(
            Some("setup"),
            r#"
                local outline = require("pmacs-outline")
                _G.S1 = pmacs.buffer.create("*s1*")
                _G.S1:replace(0, 0, "* TODO original :todo:\nbody\n")
                _G.AGG = outline.aggregate({_G.S1}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    let initial = agg_text(&mut state);
    assert!(
        initial.contains("TODO original"),
        "aggregate must show the original headline; got: {initial:?}"
    );

    // Edit the source directly.
    state
        .lua_host
        .eval(
            Some("edit-source"),
            r#"
                -- Replace "original" in the headline with "modified".
                local txt = S1:slice(0, S1:len())
                local s, e = txt:find("original")
                S1:replace(s - 1, e, "modified")
            "#,
        )
        .expect("source edit");

    // SP-7: the source-listener intercept now schedules a deferred
    // repaint via pmacs.async.yield_to_next_tick(), with no worker
    // round trip. One async tick must be sufficient.
    state.tick_async();

    let after = agg_text(&mut state);
    assert!(
        after.contains("TODO modified"),
        "aggregate must reflect the source's headline change; got: {after:?}"
    );
    assert!(
        !after.contains("TODO original"),
        "aggregate must not still show the old headline; got: {after:?}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 4 --- cyclic dependencies detected and rejected
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_cycle_rejected_via_test_seam_construction() {
    // v0.1's static API can't construct a cycle: at creation time,
    // every aggregate's `sources` list is fixed, and the new
    // aggregate's buffer doesn't exist yet so no caller can include
    // it. The cycle detector exists for v0.2 mutability and for
    // sanity-checking arbitrary user-provided sources. We exercise
    // it by manually patching an existing aggregate's `sources` via
    // the test seam to construct a cycle, then attempting to create
    // a new aggregate that triggers the check.
    let (mut state, _c, _u) = editor_with_outline();

    state
        .lua_host
        .eval(
            Some("two-aggregates"),
            r#"
                local outline = require("pmacs-outline")
                local te = outline.__pmacs_outline_test_seam_DO_NOT_USE
                _G.S1 = pmacs.buffer.create("*s1*")
                _G.S1:replace(0, 0, "* one :todo:\nbody\n")
                _G.S2 = pmacs.buffer.create("*s2*")
                _G.S2:replace(0, 0, "* two :todo:\nbody\n")
                _G.A = outline.aggregate({_G.S1}, function(_) return true end)
                _G.B = outline.aggregate({_G.S2}, function(_) return true end)
                -- Patch A.sources to include B.buffer; patch B.sources to
                -- include A.buffer. Walking either now revisits a buffer
                -- (A -> B -> A), which is a cycle.
                A.sources = { S1, B.buffer }
                B.sources = { S2, A.buffer }
                _G.WOULD_CYCLE = te.aggregate.__pmacs_outline_test_would_cycle
            "#,
        )
        .expect("two aggregates setup");

    // would_cycle on a sources list that walks through A and B
    // should return true.
    let cycle: bool = state
        .lua_host
        .lua()
        .load(r"return WOULD_CYCLE({A.buffer}, nil)")
        .eval()
        .expect("would_cycle");
    assert!(
        cycle,
        "cycle detector must flag the patched-up A->B->A chain"
    );

    // Direct creation attempt with sources that walk into the cycle
    // should error. Use pcall.
    let err_msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local outline = require("pmacs-outline")
                local ok, err = pcall(function()
                    outline.aggregate({A.buffer}, function(_) return true end)
                end)
                if ok then return "no error" end
                return tostring(err)
            "#,
        )
        .eval()
        .expect("attempt cycle creation");
    assert!(
        err_msg.contains("cycle"),
        "creation attempt must reject with a 'cycle' message; got: {err_msg:?}"
    );
}

// ---------------------------------------------------------------------------
// Regressions
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_close_removes_intercepts_keeps_sources_alive() {
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup"),
            r#"
                local outline = require("pmacs-outline")
                _G.S1 = pmacs.buffer.create("*s1*")
                _G.S1:replace(0, 0, "* TODO 1 :todo:\nbody\n")
                _G.AGG = outline.aggregate({_G.S1}, function(e)
                    return e.tagset and e.tagset.todo
                end)
                outline.aggregate_close(_G.AGG)
            "#,
        )
        .expect("setup + close");

    // S1 should still be valid after aggregate_close.
    let valid: bool = state
        .lua_host
        .lua()
        .load(r"return S1:is_valid()")
        .eval()
        .expect("s1 valid");
    assert!(
        valid,
        "source buffer must remain valid after aggregate_close"
    );

    // Editing S1 after close must not panic. The source-listener
    // intercept has been removed.
    state
        .lua_host
        .eval(Some("edit-after-close"), r#"S1:insert(0, "Z")"#)
        .expect("edit after close");

    // Trigger a tick; nothing should fail.
    state.tick_async();
}

#[test]
fn outline_aggregate_cross_block_edit_rejected() {
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup"),
            r#"
                local outline = require("pmacs-outline")
                _G.S1 = pmacs.buffer.create("*s1*")
                _G.S1:replace(0, 0, "* one :todo:\nbody1\n")
                _G.S2 = pmacs.buffer.create("*s2*")
                _G.S2:replace(0, 0, "* two :todo:\nbody2\n")
                _G.AGG = outline.aggregate({_G.S1, _G.S2}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    // Determine the agg-byte where block 1 ends (= block 2 starts).
    let split: i64 = state
        .lua_host
        .lua()
        .load(r"return AGG.blocks[1].agg_end")
        .eval()
        .expect("split byte");
    assert!(split > 0, "split byte must be positive; got {split}");

    // Try to delete a range that spans the boundary [split-2, split+2).
    let result = state.lua_host.eval(
        Some("cross-block-delete"),
        &format!(r"AGG.buffer:delete({}, {})", split - 2, split + 2),
    );
    assert!(
        result.is_err(),
        "cross-block delete must be rejected; succeeded with no error"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("block boundaries") || msg.contains("block"),
        "rejection must mention block boundaries; got: {msg}"
    );
}

#[test]
fn outline_aggregate_multiple_aggregates_over_overlapping_sources_dont_interfere() {
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*shared*")
                _G.S:replace(0, 0,
                    "* todo-only :todo:\nbody1\n" ..
                    "* done-only :done:\nbody2\n" ..
                    "* both :todo:done:\nbody3\n")
                _G.AGG_TODO = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
                _G.AGG_DONE = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.done
                end)
            "#,
        )
        .expect("setup");

    let todo_text: String = state
        .lua_host
        .lua()
        .load(r"return AGG_TODO.buffer:slice(0, AGG_TODO.buffer:len())")
        .eval()
        .expect("todo agg");
    assert!(todo_text.contains("todo-only"));
    assert!(todo_text.contains("both"));
    assert!(!todo_text.contains("done-only"));

    let done_text: String = state
        .lua_host
        .lua()
        .load(r"return AGG_DONE.buffer:slice(0, AGG_DONE.buffer:len())")
        .eval()
        .expect("done agg");
    assert!(done_text.contains("done-only"));
    assert!(done_text.contains("both"));
    assert!(!done_text.contains("todo-only"));
}

// ---------------------------------------------------------------------------
// Pass-4 regressions
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_close_detaches_parser_intercepts_when_unique_consumer() {
    // Pass-4 finding 1. aggregate() calls parser.attach for each
    // source, which adds an intercept. aggregate_close must call
    // parser.detach for each, which (when refcount hits zero)
    // removes the intercept. After close, edits to the source must
    // not bump the parser's parse counter --- evidence the
    // intercept is gone.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("aggregate-then-close"),
            r#"
                local outline = require("pmacs-outline")
                local te = outline.__pmacs_outline_test_seam_DO_NOT_USE
                te.parser.__pmacs_outline_test_reset_parse_count()
                _G.S = pmacs.buffer.create("*s*")
                _G.S:replace(0, 0, "* TODO :todo:\nbody\n")
                _G.PARSER = te.parser
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
                -- Initial render queried entries -> 1 reparse so far.
                _G.BEFORE_CLOSE = PARSER.__pmacs_outline_test_parse_count()
                outline.aggregate_close(_G.AGG)
            "#,
        )
        .expect("setup + close");

    let before_close: i64 = state
        .lua_host
        .lua()
        .globals()
        .get("BEFORE_CLOSE")
        .expect("before close");
    assert!(
        before_close >= 1,
        "must have parsed at least once before close"
    );

    // Edit S after close. With the parser intercept removed, the
    // parser will not run any new parse_region calls (no dirty
    // tracking, no reparse).
    state
        .lua_host
        .eval(Some("edit-after-close"), r#"S:insert(0, "Z")"#)
        .expect("edit");

    let after_close: i64 = state
        .lua_host
        .lua()
        .load(r"return PARSER.__pmacs_outline_test_parse_count()")
        .eval()
        .expect("after close");
    assert_eq!(
        after_close, before_close,
        "parser must be fully detached after aggregate_close when no other \
         consumer holds a ref; counts {before_close} -> {after_close}"
    );
}

#[test]
fn outline_aggregate_close_keeps_parser_when_other_consumer_holds_ref() {
    // Refcounting: outline.open(S) and aggregate({S, ...}) share the
    // same parser handle. Closing the aggregate decrements the ref
    // but the outline view still holds one, so the parser stays
    // attached and its intercept keeps invalidating the cache.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("shared-source"),
            r#"
                local outline = require("pmacs-outline")
                local te = outline.__pmacs_outline_test_seam_DO_NOT_USE
                te.parser.__pmacs_outline_test_reset_parse_count()
                _G.S = pmacs.buffer.create("*shared*")
                _G.S:replace(0, 0, "* TODO :todo:\nbody\n")
                _G.PARSER = te.parser
                _G.OUTLINE_HANDLE = outline.open(_G.S)
                _G.PH = _G.OUTLINE_HANDLE.parser_handle
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
                -- Refcount must be 2 (one from outline.open, one from
                -- aggregate).
                _G.REFCOUNT_BEFORE = te.parser.__pmacs_outline_test_refcount(_G.PH)
                outline.aggregate_close(_G.AGG)
                _G.REFCOUNT_AFTER = te.parser.__pmacs_outline_test_refcount(_G.PH)
            "#,
        )
        .expect("shared setup + agg close");

    let before: i64 = state
        .lua_host
        .lua()
        .globals()
        .get("REFCOUNT_BEFORE")
        .expect("rc before");
    let after: i64 = state
        .lua_host
        .lua()
        .globals()
        .get("REFCOUNT_AFTER")
        .expect("rc after");
    assert_eq!(
        before, 2,
        "shared source must have refcount 2; got {before}"
    );
    assert_eq!(
        after, 1,
        "after closing the aggregate, refcount must drop to 1 (outline.open still holds); got {after}"
    );

    // Reset parse counter, edit source, query via the parser handle:
    // parser must still be active and reparse the dirty range.
    state
        .lua_host
        .eval(
            Some("verify-parser-active"),
            r#"
                PARSER.__pmacs_outline_test_reset_parse_count()
                S:insert(0, "Z")
                local _ = #PARSER.entries(PH)
            "#,
        )
        .expect("verify");
    let parses: i64 = state
        .lua_host
        .lua()
        .load(r"return PARSER.__pmacs_outline_test_parse_count()")
        .eval()
        .expect("parses");
    assert!(
        parses >= 1,
        "parser must still run reparse after aggregate close (other consumer holds ref); \
         got {parses} parse calls"
    );
}

#[test]
fn outline_aggregate_insert_at_end_of_last_block_appends_to_source() {
    // Pass-4 finding 2. Inserting at agg_buf:len() (i.e., at the
    // very end of the aggregate, which equals last_block.agg_end)
    // must succeed: it maps to source_end of the last matched
    // entry.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*s*")
                _G.S:replace(0, 0, "* TODO :todo:\nbody-1\n")
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    // The aggregate currently equals the source.
    let agg_text: String = state
        .lua_host
        .lua()
        .load(r"return AGG.buffer:slice(0, AGG.buffer:len())")
        .eval()
        .expect("agg");
    assert_eq!(agg_text, "* TODO :todo:\nbody-1\n");

    // Insert at the very end of the aggregate buffer.
    state
        .lua_host
        .eval(
            Some("append"),
            r#"
                local n = AGG.buffer:len()
                AGG.buffer:insert(n, "appended\n")
            "#,
        )
        .expect("append");

    // Source must now contain the appended bytes at end-of-entry.
    let src_text: String = state
        .lua_host
        .lua()
        .load(r"return S:slice(0, S:len())")
        .eval()
        .expect("src");
    assert!(
        src_text.contains("body-1\nappended\n"),
        "appended bytes must land at end of source's last matched entry; got: {src_text:?}"
    );
}

#[test]
fn outline_aggregate_consecutive_edits_without_repaint_map_correctly() {
    // Pass-4 finding 3. Two aggregate edits in immediate succession
    // (before any tick_async runs the deferred repaint) must each
    // map correctly to source coordinates. Two SEPARATE sources are
    // required to expose the bug: when both blocks share one source,
    // the parser's source-byte shifts and the agg shifts move
    // together and the stale block math accidentally produces the
    // right answer. With two sources, edit-1 to S1 grows block 1
    // (agg-side) but doesn't touch S2's bytes; the stale block 2
    // mapping for edit-2 then computes the wrong S2 byte without
    // writeback-time bookkeeping.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup"),
            r#"
                local outline = require("pmacs-outline")
                _G.S1 = pmacs.buffer.create("*s1*")
                _G.S1:replace(0, 0, "* one :todo:\nbody1\n")
                _G.S2 = pmacs.buffer.create("*s2*")
                _G.S2:replace(0, 0, "* two :todo:\nbody2\n")
                _G.AGG = outline.aggregate({_G.S1, _G.S2}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    // Layout: each source is "* one :todo:\nbody1\n" / "* two ..." =
    // 19 bytes ("body1" / "body2" is 5 chars + newline). Aggregate
    // concatenates: block 1 spans agg [0, 19) for S1; block 2 spans
    // agg [19, 38) for S2. body1 starts at agg byte 13 (after
    // "* one :todo:\n"); body2 starts at agg byte 32 (= 19 + 13).
    //
    // Edit-1: insert "X" at agg byte 14, between 'b' and 'o' of body1.
    // Block 1 grows by 1. With writeback-time bookkeeping, block 2's
    // agg coords shift to [20, 39). Block 2's source coords stay
    // [0, 19) because S2 was untouched.
    //
    // Edit-2: insert "Y" at agg byte 34, which under the *post*-shift
    // block 2 maps to S2 byte (34 - 20) = 14 = between 'b' and 'o'
    // of body2. Without the shift, stale block 2 [19, 38) maps agg
    // byte 34 to S2 byte (34 - 19) = 15 = between 'o' and 'd' instead,
    // producing "boYdy2".
    state
        .lua_host
        .eval(
            Some("two-edits"),
            r#"
                AGG.buffer:insert(14, "X")
                AGG.buffer:insert(34, "Y")
            "#,
        )
        .expect("two edits");

    let s1_text: String = state
        .lua_host
        .lua()
        .load(r"return S1:slice(0, S1:len())")
        .eval()
        .expect("s1");
    let s2_text: String = state
        .lua_host
        .lua()
        .load(r"return S2:slice(0, S2:len())")
        .eval()
        .expect("s2");
    assert!(
        s1_text.contains("bXody1"),
        "first edit's X must land inside S1's body1; got: {s1_text:?}"
    );
    assert!(
        s2_text.contains("bYody2"),
        "second edit's Y must land between 'b' and 'o' of S2's body2; \
         this is what fails without writeback-time block-shift bookkeeping \
         (stale block 2 would map to S2 byte 15, producing 'boYdy2'); \
         got: {s2_text:?}"
    );
}

// ---------------------------------------------------------------------------
// Pass-5 regressions
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_close_then_reopen_reinstalls_parser_intercept() {
    // Pass-5 finding 1. Closing an aggregate must not leave a dead
    // (refcount=0, intercept=nil) parser handle in the registry.
    // If it did, a subsequent attach for the same source would
    // bump the dead handle's refcount and skip reinstalling the
    // intercept --- the parser cache then never invalidates on
    // future edits.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("aggregate-close-then-reopen"),
            r#"
                local outline = require("pmacs-outline")
                local te = outline.__pmacs_outline_test_seam_DO_NOT_USE
                te.parser.__pmacs_outline_test_reset_parse_count()
                _G.S = pmacs.buffer.create("*s*")
                _G.S:replace(0, 0, "* TODO :todo:\nbody\n")
                _G.PARSER = te.parser
                _G.PRED = function(e) return e.tagset and e.tagset.todo end
                local A = outline.aggregate({_G.S}, _G.PRED)
                outline.aggregate_close(A)
                -- Now create a fresh aggregate over the same source.
                _G.B = outline.aggregate({_G.S}, _G.PRED)
                _G.PH = _G.B.parser_handles[_G.S]
            "#,
        )
        .expect("close then reopen");

    // The fresh aggregate's parser handle must be live (intercept
    // installed). Reset the parse counter, edit S, query: the
    // counter must increment, proving the intercept is firing
    // dirty tracking and the next query reparses.
    state
        .lua_host
        .eval(
            Some("verify-fresh-parser"),
            r#"
                PARSER.__pmacs_outline_test_reset_parse_count()
                S:insert(0, "Z")
                local _ = #PARSER.entries(PH)
            "#,
        )
        .expect("edit + query");

    let parses: i64 = state
        .lua_host
        .lua()
        .load(r"return PARSER.__pmacs_outline_test_parse_count()")
        .eval()
        .expect("parses");
    assert!(
        parses >= 1,
        "fresh aggregate over a previously-closed source must have a live \
         parser intercept; got {parses} parse calls"
    );
}

#[test]
fn outline_aggregate_nested_overlapping_blocks_consecutive_writeback() {
    // Pass-5 finding 2. When the predicate matches both a parent
    // headline and its nested child, the aggregate emits two
    // overlapping blocks (the parent's slice contains the child's
    // bytes; the child's slice is a copy of those same bytes).
    // Editing in the parent block then editing in the child block
    // (before the deferred repaint) must map both edits to the
    // right source byte. The original shift logic only updated
    // same-source blocks whose source_start was past the edited
    // block's source_end --- nested children fail that check.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup-nested"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*s*")
                -- Layout (29 bytes):
                --   "* P :todo:\n"  bytes 0..10 (11 bytes)
                --   "bb\n"          bytes 11..13 (3 bytes)
                --   "** C :todo:\n" bytes 14..25 (12 bytes)
                --   "cc\n"          bytes 26..28 (3 bytes)
                _G.S:replace(0, 0, "* P :todo:\nbb\n** C :todo:\ncc\n")
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
                -- Both P (bytes 0..29) and C (bytes 14..29) match.
                -- Aggregate text = P's slice + C's slice = 29 + 15 = 44.
            "#,
        )
        .expect("setup");

    let agg_len: i64 = state
        .lua_host
        .lua()
        .load(r"return AGG.buffer:len()")
        .eval()
        .expect("agg len");
    assert_eq!(
        agg_len, 44,
        "expected aggregate length 44 (29 + 15); got {agg_len}"
    );

    // Edit-1: insert "X" at agg byte 12 (between the two 'b's of
    // P's body "bb"). Maps to S byte 12.
    state
        .lua_host
        .eval(Some("edit-parent"), r#"AGG.buffer:insert(12, "X")"#)
        .expect("edit parent");

    // Source after edit-1: "* P :todo:\nbXb\n** C :todo:\ncc\n"
    // (30 bytes). Layout post-edit-1:
    //   bytes 0..10  = "* P :todo:" + '\n'
    //   bytes 11..14 = "bXb\n"
    //   bytes 15..26 = "** C :todo:\n"
    //   bytes 27..28 = "cc"   (first c at 27, second at 28)
    //   byte 29      = '\n'
    //
    // Parser shifted C from [14, 29) to [15, 30); C's headline
    // starts at byte 15 ('*' of "**").
    //
    // The block-shift fix moves Block C's source coords to (15, 30)
    // and agg coords to (30, 45). Without the fix, source stays at
    // (14, 29) with agg shifted to (30, 45) --- a 1-byte gap that
    // misroutes any subsequent edit landing in C.
    //
    // Aggregate text post-edit-1 = P's slice (30 bytes) + C's
    // slice from the original render (15 bytes, unchanged in
    // memory) = 45 bytes. C's body "cc" within C's slice is at
    // offsets 12 and 13; in agg coords those are bytes 42 and 43.
    // To land "Y" *between* the two c's we insert at byte 43 = the
    // second c.
    //
    // With the Pass-5 fix: src = 15 + (43-30) = 28 = second c, in
    // S. Insert before -> "cYc". CORRECT.
    //
    // Without the fix: src = 14 + (43-30) = 27 = first c, in S.
    // Insert before -> "Ycc". WRONG.

    state
        .lua_host
        .eval(Some("edit-child"), r#"AGG.buffer:insert(43, "Y")"#)
        .expect("edit child");

    let s_text: String = state
        .lua_host
        .lua()
        .load(r"return S:slice(0, S:len())")
        .eval()
        .expect("s text");
    assert!(
        s_text.contains("bXb"),
        "edit-1 must land between the two b's of P's body; got: {s_text:?}"
    );
    assert!(
        s_text.contains("cYc"),
        "edit-2 must land between the two c's of C's body; without the \
         Pass-5 nested-overlapping fix, stale block C source coords map \
         the edit to byte 26 producing 'Ycc'; got: {s_text:?}"
    );
}

#[test]
fn outline_aggregate_duplicate_source_buffers_not_a_cycle() {
    // Pass-5 finding 3. would_cycle previously used one global
    // visited set, so passing the same plain source twice in
    // `sources` was treated as a cycle. Plain duplicate sources
    // are a legitimate use (e.g., user wants the same outline
    // counted twice for some predicate); cycles are about
    // aggregate-to-aggregate dependency loops.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("dup-sources"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*s*")
                _G.S:replace(0, 0, "* TODO :todo:\nbody\n")
                -- Should succeed: S is a plain (non-aggregate)
                -- source buffer, and duplication of plain sources
                -- is not a cycle.
                _G.AGG = outline.aggregate({_G.S, _G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("duplicate sources should not be rejected as a cycle");

    // The aggregate should contain the matching entry's content twice
    // (once per duplicate source listing).
    let txt: String = state
        .lua_host
        .lua()
        .load(r"return AGG.buffer:slice(0, AGG.buffer:len())")
        .eval()
        .expect("agg text");
    let occurrences = txt.matches("* TODO").count();
    assert_eq!(
        occurrences, 2,
        "duplicate source must produce two block emissions; got {occurrences}: {txt:?}"
    );
}

// ---------------------------------------------------------------------------
// Pass-6 regressions
// ---------------------------------------------------------------------------

#[test]
fn outline_aggregate_delete_overlapping_child_drops_invalidated_block() {
    // Pass-6 finding 1. With parent + child both matching, deleting
    // bytes in the parent block that span the child entry's source
    // range entirely deletes the child from source. The block map
    // must drop the child block; otherwise a subsequent write-back
    // edit at the (zombie) child agg range would map to source
    // bytes that no longer exist.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup-nested"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*s*")
                -- Layout (30 bytes):
                --   "* P :todo:\n"  bytes 0..10
                --   "** C :todo:\n" bytes 11..22
                --   "ccbody\n"      bytes 23..29
                _G.S:replace(0, 0, "* P :todo:\n** C :todo:\nccbody\n")
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    let n_before: i64 = state
        .lua_host
        .lua()
        .load(r"return #AGG.blocks")
        .eval()
        .expect("blocks before");
    assert_eq!(
        n_before, 2,
        "aggregate must have 2 blocks (P + C); got {n_before}"
    );

    // Delete agg [11, 30) --- the parent block's bytes from byte 11
    // through end of P's slice. This maps to source [11, 30) which
    // is exactly C's source range (level-2 entry nested in P).
    state
        .lua_host
        .eval(
            Some("delete-child-via-parent"),
            r"AGG.buffer:delete(11, 30)",
        )
        .expect("delete");

    let n_after: i64 = state
        .lua_host
        .lua()
        .load(r"return #AGG.blocks")
        .eval()
        .expect("blocks after");
    assert_eq!(
        n_after, 1,
        "C's source bytes were entirely deleted; its block must be \
         dropped from handle.blocks. got {n_after} blocks"
    );

    let surviving_source_start: i64 = state
        .lua_host
        .lua()
        .load(r"return AGG.blocks[1].source_start")
        .eval()
        .expect("ss");
    assert_eq!(
        surviving_source_start, 0,
        "surviving block must be P (source_start=0); got {surviving_source_start}"
    );

    // Source S now has just "* P :todo:\n" (11 bytes).
    let s_text: String = state
        .lua_host
        .lua()
        .load(r"return S:slice(0, S:len())")
        .eval()
        .expect("s text");
    assert_eq!(
        s_text, "* P :todo:\n",
        "source must reflect the delete; got: {s_text:?}"
    );
}

#[test]
fn outline_aggregate_delete_entire_last_block_drops_edited_block() {
    // Pass-7 finding 1. Deleting a matched block's full source
    // range must drop the edited block itself. Otherwise it remains
    // as a zero-length stale block at aggregate EOF, and a second
    // insert before deferred repaint routes into the deleted source
    // coordinates instead of being rejected.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup-single-block"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*s*")
                _G.S:replace(0, 0, "* TODO :todo:\nbody\n")
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    let len: i64 = state
        .lua_host
        .lua()
        .load(r"return AGG.buffer:len()")
        .eval()
        .expect("agg len");
    assert!(len > 0, "aggregate block must be non-empty");

    state
        .lua_host
        .eval(
            Some("delete-whole-block"),
            &format!("AGG.buffer:delete(0, {len})"),
        )
        .expect("delete whole block");

    let blocks_after: i64 = state
        .lua_host
        .lua()
        .load(r"return #AGG.blocks")
        .eval()
        .expect("blocks after");
    assert_eq!(
        blocks_after, 0,
        "full-block delete must drop the edited block from handle.blocks"
    );

    let result = state
        .lua_host
        .eval(Some("stale-tail-insert"), r#"AGG.buffer:insert(0, "Z")"#);
    assert!(
        result.is_err(),
        "insert into the deleted block's old range must be rejected before repaint"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("outside any matched entry"),
        "rejection should name the stale matched-entry range; got: {msg}"
    );

    let s_text: String = state
        .lua_host
        .lua()
        .load(r"return S:slice(0, S:len())")
        .eval()
        .expect("source text");
    assert_eq!(
        s_text, "",
        "stale aggregate insert must not write back into source; got: {s_text:?}"
    );
}

#[test]
fn outline_aggregate_package_reload_closes_aggregates() {
    // Pass-6 finding 2. pmacs.packages.reload triggers the
    // package's on_unload, which must close all live aggregate
    // handles. Otherwise source-listener intercepts and parser
    // refcounts persist past the old package's discarded module
    // closures, and aggregate buffers stay alive without a way to
    // reach them.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup-aggregate"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*s*")
                _G.S:replace(0, 0, "* TODO :todo:\nbody\n")
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
                _G.AGG_BUF = AGG.buffer
            "#,
        )
        .expect("aggregate setup");

    // Sanity: aggregate buffer is alive before reload.
    let alive_before: bool = state
        .lua_host
        .lua()
        .load(r"return AGG_BUF:is_valid()")
        .eval()
        .expect("alive before");
    assert!(
        alive_before,
        "aggregate buffer should be alive before reload"
    );

    // Reload the package. on_unload must close the aggregate, which
    // kills its buffer.
    state
        .lua_host
        .eval(Some("reload"), r#"pmacs.packages.reload("pmacs-outline")"#)
        .expect("reload");

    let alive_after: bool = state
        .lua_host
        .lua()
        .load(r"return AGG_BUF:is_valid()")
        .eval()
        .expect("alive after");
    assert!(
        !alive_after,
        "aggregate buffer must be killed by reload's on_unload calling \
         aggregate.close_all_handles --- otherwise the aggregate's \
         intercepts are still attached to source S after the package \
         module has been re-initialized"
    );
}

#[test]
fn outline_aggregate_source_change_propagates_with_tight_deadline() {
    // SP-7 regression: the aggregate source-listener uses
    // pmacs.async.yield_to_next_tick(), so the old worker-sleep
    // timing path must not come back.
    let (mut state, _c, _u) = editor_with_outline();
    state
        .lua_host
        .eval(
            Some("setup"),
            r#"
                local outline = require("pmacs-outline")
                _G.S = pmacs.buffer.create("*s*")
                _G.S:replace(0, 0, "* TODO original :todo:\nbody\n")
                _G.AGG = outline.aggregate({_G.S}, function(e)
                    return e.tagset and e.tagset.todo
                end)
            "#,
        )
        .expect("setup");

    state
        .lua_host
        .eval(
            Some("source-edit"),
            r#"
                local txt = S:slice(0, S:len())
                local s, e = txt:find("original")
                S:replace(s - 1, e, "modified")
            "#,
        )
        .expect("source edit");

    // Tight bounded-propagation check: 200ms is much less than the
    // 2s the helper allows; if the v0.1 mechanism ever regresses
    // beyond worker round-trip latency, this test catches it.
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        if Instant::now() >= deadline {
            let txt: String = state
                .lua_host
                .lua()
                .load(r"return AGG.buffer:slice(0, AGG.buffer:len())")
                .eval()
                .expect("agg text");
            panic!(
                "source-change propagation exceeded the 200ms tight \
                 deadline; see SP-7 in V0.2-PREREQUISITES.md. \
                 Aggregate text: {txt:?}"
            );
        }
        state.tick_async();
        let txt: String = state
            .lua_host
            .lua()
            .load(r"return AGG.buffer:slice(0, AGG.buffer:len())")
            .eval()
            .expect("agg text");
        if txt.contains("TODO modified") {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn outline_aggregate_empty_sources_rejected() {
    let (mut state, _c, _u) = editor_with_outline();
    let result = state.lua_host.eval(
        Some("empty-sources"),
        r#"
            local outline = require("pmacs-outline")
            outline.aggregate({}, function(_) return true end)
        "#,
    );
    assert!(result.is_err(), "empty sources list must be rejected");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("non-empty"),
        "error must mention non-empty; got: {msg}"
    );
}
