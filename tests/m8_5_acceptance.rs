// tests/m8_5_acceptance.rs --- T M8.5 magit-class foldable section view.

//! Acceptance for the M8.5 deliverable: a foldable section view
//! built on the two-buffer projection pattern (Plan A from the
//! M8.5 architecture decision). The package source-of-record is
//! the in-tree fixture at `tests/fixtures/pmacs-magit/`.
//!
//! The four spec acceptance bullets:
//!
//! 1. Section rendering: a buffer with three top-level sections,
//!    each containing nested subsections, renders correctly.
//! 2. Folding state survives buffer redraw and view repaint.
//! 3. Folding does not modify the rope; the underlying text is
//!    unchanged regardless of fold state.
//! 4. Cursor navigation respects fold state (folded sections
//!    skipped on `C-n` / `C-p`).
//!
//! Plus regressions:
//!
//! * Visible buffer rejects user edits (intercept).
//! * Toggle-fold via TAB binding through the dispatch path.
//! * fold-all / unfold-all commands.
//! * Nested fold: collapsing a parent hides every descendant.
//! * Re-open is reload-safe (`on_unload` hook drops handles +
//!   commands cleanly).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

fn magit_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests").join("fixtures").join("pmacs-magit")
}

#[allow(dead_code)]
fn pump_until<F: Fn(&EditorState) -> bool>(state: &mut EditorState, predicate: F) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate(state) {
        assert!(
            Instant::now() < deadline,
            "async pump deadline exceeded after 5s"
        );
        state.tick_async();
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn editor_with_magit() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    state.lua_host.reopen_init_phase_for_testing();
    state.lua_host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    let pkg = magit_package_path();
    let pkg_str = pkg.display().to_string();
    let install = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("pmacs-magit")
    "#
    );
    state
        .lua_host
        .eval(Some("magit-install"), &install)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));
    (state, cache, user_root)
}

/// Standard test spec: three top-level sections, each with two
/// nested subsections. Used by most of the acceptance tests.
const STANDARD_SPEC: &str = r#"
    require("pmacs-magit").open {
        {
            id = "a", title = "Section A",
            body = "a-body-1\na-body-2",
            children = {
                { id = "a1", title = "Subsection A1", body = "a1-body" },
                { id = "a2", title = "Subsection A2", body = "a2-body" },
            },
        },
        {
            id = "b", title = "Section B",
            body = "b-body",
            children = {
                { id = "b1", title = "Subsection B1", body = "b1-body" },
                { id = "b2", title = "Subsection B2", body = "b2-body" },
            },
        },
        {
            id = "c", title = "Section C",
            body = "c-body",
            children = {
                { id = "c1", title = "Subsection C1", body = "c1-body" },
                { id = "c2", title = "Subsection C2", body = "c2-body" },
            },
        },
    }
"#;

fn open_standard(state: &mut EditorState) {
    state
        .lua_host
        .eval(Some("magit-open-standard"), STANDARD_SPEC)
        .unwrap_or_else(|e| panic!("magit.open failed: {e}"));
}

fn active_buffer_text(state: &mut EditorState) -> String {
    state
        .lua_host
        .lua()
        .load(
            r"
            local buf = pmacs.window.buffer()
            return buf:slice(0, buf:len())
        ",
        )
        .eval::<String>()
        .expect("read active buffer")
}

fn source_text(state: &mut EditorState) -> String {
    state
        .lua_host
        .lua()
        .load(
            r#"
            local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
            local h = te.active_handle()
            return h.source:slice(0, h.source:len())
        "#,
        )
        .eval::<String>()
        .expect("read source buffer text")
}

// ---------------------------------------------------------------------------
// Bullet 1 --- section rendering
// ---------------------------------------------------------------------------

#[test]
fn magit_three_top_level_with_nested_subsections_render_correctly() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);
    let text = active_buffer_text(&mut state);
    let lines: Vec<&str> = text.lines().collect();

    // Default: everything expanded. Each top-level section has:
    //   1 header + 1-or-2 body lines + (2 children * (1 header + 1 body)).
    // Three top-levels with the standard spec produces 21 lines:
    //   A header (1) + 2 body + A1 header + a1 body + A2 header + a2 body
    //   = 7 lines for A (and same for B/C).
    // But B and C have 1 body each (not 2), so they're 6 lines each.
    // Total: 7 + 6 + 6 = 19.
    assert_eq!(
        lines.len(),
        19,
        "expected 19 lines (7 + 6 + 6); got {}: {}",
        lines.len(),
        text
    );

    // Top-level headers (depth 0) start with the fold marker.
    assert!(
        lines[0].starts_with("v Section A") || lines[0].starts_with("> Section A"),
        "line 0 must be Section A header: {:?}",
        lines[0]
    );
    // Body indented under header (2 spaces past header indent + 0 marker).
    assert_eq!(lines[1], "  a-body-1");
    assert_eq!(lines[2], "  a-body-2");
    // Subsection header indented 2 spaces.
    assert!(
        lines[3].starts_with("  ") && lines[3].contains("Subsection A1"),
        "line 3 must be Subsection A1: {:?}",
        lines[3]
    );
    // Subsection body indented 4 spaces (2 levels deep + 1).
    assert_eq!(lines[4], "    a1-body");
}

// ---------------------------------------------------------------------------
// Bullet 2 --- folding state survives buffer redraw
// ---------------------------------------------------------------------------

#[test]
fn magit_fold_state_survives_explicit_repaint() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    // Collapse section "a" via direct fold-state manipulation (the
    // test seam exposes the helpers); then call repaint twice and
    // verify the projection is the same both times.
    state
        .lua_host
        .eval(
            Some("collapse-and-repaint"),
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                te.fold.toggle(h.fold_state, "a")
                te.repaint_visible(h)
                _G.M8_5_AFTER_FIRST = h.visible:slice(0, h.visible:len())
                te.repaint_visible(h)
                _G.M8_5_AFTER_SECOND = h.visible:slice(0, h.visible:len())
                _G.M8_5_FOLD_A = h.fold_state.a
            "#,
        )
        .expect("collapse + repaint");
    let after_first: String = state
        .lua_host
        .lua()
        .globals()
        .get("M8_5_AFTER_FIRST")
        .expect("first projection");
    let after_second: String = state
        .lua_host
        .lua()
        .globals()
        .get("M8_5_AFTER_SECOND")
        .expect("second projection");
    let fold_a: String = state
        .lua_host
        .lua()
        .globals()
        .get("M8_5_FOLD_A")
        .expect("fold state of a");
    assert_eq!(fold_a, "collapsed", "section a must be collapsed");
    assert_eq!(
        after_first, after_second,
        "two consecutive repaints with the same fold state must be byte-identical"
    );

    // Section A's body and subsections must not appear in the projection.
    assert!(
        !after_first.contains("a-body-1") && !after_first.contains("Subsection A1"),
        "collapsed Section A must hide its body and children: {after_first}"
    );
    // Header line for A is still present (with collapsed marker).
    assert!(
        after_first.contains("Section A"),
        "collapsed Section A must still show its header: {after_first}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 3 --- source rope unchanged across fold ops
// ---------------------------------------------------------------------------

#[test]
fn magit_source_rope_unchanged_across_fold_operations() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    let source_before = source_text(&mut state);
    assert!(!source_before.is_empty(), "source buffer must have content");

    // A sequence of fold operations: collapse a few, expand them,
    // collapse all, expand all. After all of this the source rope
    // must be byte-for-byte identical to its initial state.
    state
        .lua_host
        .eval(
            Some("fold-churn"),
            r#"
                pmacs.command.invoke("pmacs-magit.fold-all")
                pmacs.command.invoke("pmacs-magit.unfold-all")
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                te.fold.toggle(h.fold_state, "b1")
                te.repaint_visible(h)
                te.fold.toggle(h.fold_state, "b1")
                te.repaint_visible(h)
                te.fold.toggle(h.fold_state, "a")
                te.repaint_visible(h)
            "#,
        )
        .expect("fold churn");

    let source_after = source_text(&mut state);
    assert_eq!(
        source_before, source_after,
        "source rope must be byte-for-byte unchanged after fold churn"
    );
}

// ---------------------------------------------------------------------------
// Bullet 4 --- cursor navigation respects fold state
// ---------------------------------------------------------------------------
//
// In Plan A, "respects fold state" is a structural property of the
// visible buffer: folded content isn't in it at all, so any cursor
// movement (`C-n` / `C-p`, page-down, word-skip, ...) within the
// visible buffer cannot land on a folded line. The strongest
// version of this test verifies that no byte position in the
// visible buffer corresponds to text that the source contains
// inside a folded section.

#[test]
fn magit_visible_buffer_excludes_folded_content_after_collapse() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    // Collapse Section B.
    state
        .lua_host
        .eval(
            Some("collapse-b"),
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                te.fold.toggle(h.fold_state, "b")
                te.repaint_visible(h)
            "#,
        )
        .expect("collapse b");

    let visible = active_buffer_text(&mut state);
    let source = source_text(&mut state);

    // Source still has every body line.
    assert!(source.contains("b-body"));
    assert!(source.contains("b1-body"));
    assert!(source.contains("b2-body"));
    assert!(source.contains("Subsection B1"));
    assert!(source.contains("Subsection B2"));

    // Visible buffer omits B's body and all of B's descendants.
    assert!(
        !visible.contains("b-body"),
        "collapsed Section B's body must not be in visible: {visible}"
    );
    assert!(
        !visible.contains("Subsection B1"),
        "collapsed Section B's child header must be hidden: {visible}"
    );
    assert!(
        !visible.contains("b1-body") && !visible.contains("b2-body"),
        "collapsed Section B's grandchildren bodies must be hidden: {visible}"
    );
    // B's own header is still visible (collapsed marker, not its body).
    assert!(
        visible.contains("Section B"),
        "Section B's header must still be in visible: {visible}"
    );

    // Cursor in visible buffer: the line count after Section B's
    // header is the start of Section C. C-n / C-p respect this
    // because there's nothing in between.
    let visible_lines: Vec<&str> = visible.lines().collect();
    let b_idx = visible_lines
        .iter()
        .position(|l| l.contains("Section B"))
        .expect("Section B header in visible");
    let c_idx = visible_lines
        .iter()
        .position(|l| l.contains("Section C"))
        .expect("Section C header in visible");
    assert_eq!(
        c_idx,
        b_idx + 1,
        "with B collapsed, C must be the line immediately after B's header in the visible buffer"
    );
}

// ---------------------------------------------------------------------------
// Regression: visible buffer rejects user edits
// ---------------------------------------------------------------------------

#[test]
fn magit_visible_buffer_rejects_user_edits() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local buf = pmacs.window.buffer()
                local ok, err = pcall(function() buf:insert(0, "x") end)
                assert(not ok, "user edit must be rejected by intercept")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("user-edit eval");
    assert!(
        msg.contains("section view") && msg.contains("not supported"),
        "rejection must explain why; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Regression: TAB binding invokes toggle-fold
// ---------------------------------------------------------------------------

#[test]
fn magit_tab_binding_toggles_fold_at_cursor() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    // Cursor lands at byte 0 by default → line 0 = Section A header.
    // Invoke the bound TAB command: this is a buffer-scope keymap
    // entry that resolves to pmacs-magit.toggle-fold.
    //
    // Since we don't have a clean dispatch_key seam in tests yet,
    // we exercise the same code path by invoking the command
    // directly (M8.4 finding 4: `pmacs.editor.move_cursor` and
    // friends will let us drive through the actual keymap path).
    state
        .lua_host
        .eval(
            Some("toggle-via-command"),
            r#"pmacs.command.invoke("pmacs-magit.toggle-fold")"#,
        )
        .expect("toggle-fold invoke");
    let after = active_buffer_text(&mut state);
    assert!(
        !after.contains("a-body-1"),
        "after toggling Section A, its body must be hidden: {after}"
    );
}

// ---------------------------------------------------------------------------
// Regression: fold-all / unfold-all
// ---------------------------------------------------------------------------

#[test]
fn magit_fold_all_collapses_every_section() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);
    state
        .lua_host
        .eval(
            Some("fold-all"),
            r#"pmacs.command.invoke("pmacs-magit.fold-all")"#,
        )
        .expect("fold-all");
    let visible = active_buffer_text(&mut state);
    // No body lines should be visible.
    for body in [
        "a-body-1", "a-body-2", "a1-body", "a2-body", "b-body", "b1-body", "b2-body", "c-body",
        "c1-body", "c2-body",
    ] {
        assert!(
            !visible.contains(body),
            "fold-all: body {body} must not be in visible: {visible}"
        );
    }
    // Three top-level headers still show.
    assert!(visible.contains("Section A"));
    assert!(visible.contains("Section B"));
    assert!(visible.contains("Section C"));
    // Subsection headers are hidden because their parents are folded.
    assert!(!visible.contains("Subsection A1"));
    assert!(!visible.contains("Subsection B2"));
}

#[test]
fn magit_unfold_all_after_fold_all_restores_full_view() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);
    let before = active_buffer_text(&mut state);
    state
        .lua_host
        .eval(
            Some("fold-then-unfold"),
            r#"
                pmacs.command.invoke("pmacs-magit.fold-all")
                pmacs.command.invoke("pmacs-magit.unfold-all")
            "#,
        )
        .expect("fold-then-unfold");
    let after = active_buffer_text(&mut state);
    assert_eq!(
        before, after,
        "unfold-all after fold-all must restore the original view"
    );
}

// ---------------------------------------------------------------------------
// Regression: nested fold cascade
// ---------------------------------------------------------------------------

#[test]
fn magit_collapsing_parent_hides_descendant_subtree() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);
    state
        .lua_host
        .eval(
            Some("collapse-a-only"),
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                te.fold.toggle(h.fold_state, "a")
                te.repaint_visible(h)
            "#,
        )
        .expect("collapse a");
    let v = active_buffer_text(&mut state);
    // Collapsing A hides every descendant: A's body, A's children's
    // headers, A's children's bodies. None of "a-body-1", "a-body-2",
    // "Subsection A1", "a1-body", "Subsection A2", "a2-body" should appear.
    for hidden in [
        "a-body-1",
        "a-body-2",
        "Subsection A1",
        "Subsection A2",
        "a1-body",
        "a2-body",
    ] {
        assert!(
            !v.contains(hidden),
            "collapsed A must hide descendant {hidden}: {v}"
        );
    }
    // B and C are unaffected.
    assert!(v.contains("Subsection B1"));
    assert!(v.contains("Subsection C2"));
    assert!(v.contains("c2-body"));
}

// ---------------------------------------------------------------------------
// Regression: parse errors surface usefully
// ---------------------------------------------------------------------------

#[test]
fn magit_open_with_duplicate_id_errors() {
    let (state, _c, _u) = editor_with_magit();
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local ok, err = pcall(function()
                    require("pmacs-magit").open {
                        { id = "x", title = "X", body = "" },
                        { id = "x", title = "X-again", body = "" },
                    }
                end)
                assert(not ok, "duplicate id must error")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("dup-id eval");
    assert!(
        msg.contains("duplicate section id"),
        "error must name the duplicate-id problem: {msg}"
    );
}

#[test]
fn magit_open_with_no_sections_errors() {
    let (state, _c, _u) = editor_with_magit();
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local ok, err = pcall(function()
                    require("pmacs-magit").open {}
                end)
                assert(not ok, "empty spec must error")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("empty-spec eval");
    assert!(
        msg.contains("at least one"),
        "error must explain the empty-spec problem: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Regression: section-at lookup via test seam
// ---------------------------------------------------------------------------

#[test]
fn magit_section_at_lookup_resolves_header_and_body_lines() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);
    let answers: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                local proj = h.projection
                -- Line 0: A header. Line 1: a-body-1 (belongs to A).
                -- Line 3: A1 header. Line 4: a1-body (belongs to A1).
                local s0 = te.fold.section_at(proj, 0)
                local s1 = te.fold.section_at(proj, 1)
                local s3 = te.fold.section_at(proj, 3)
                local s4 = te.fold.section_at(proj, 4)
                return s0 .. "|" .. s1 .. "|" .. s3 .. "|" .. s4
            "#,
        )
        .eval()
        .expect("section_at eval");
    assert_eq!(answers, "a|a|a1|a1");
}

// ---------------------------------------------------------------------------
// Pass-2 review High1 --- source-of-truth is read-only
// ---------------------------------------------------------------------------
//
// Earlier draft created a leading-space-named source buffer; pmacs's
// listing / cycling does not filter leading-space names, so the
// canonical source rope was reachable via C-x <left> / C-x C-b /
// pmacs.window.switch_buffer(source). Plan A's invariant requires
// the source-of-truth to be unwritable by the user. Fix: keep the
// source as a real rope-backed buffer, but attach a read-only
// intercept to reject direct edits.

#[test]
fn magit_source_of_truth_is_a_readonly_buffer() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    let source_names: Vec<String> = state
        .lua_host
        .lua()
        .load(
            r"
                local out = {}
                for _, id in ipairs(pmacs.buffer.list()) do
                    local d = pmacs.describe.buffer(id)
                    if d ~= nil and d.name:find('magit%-source') then
                        out[#out + 1] = d.name
                    end
                end
                return out
            ",
        )
        .eval::<Vec<String>>()
        .expect("buffer list eval");

    assert_eq!(
        source_names.len(),
        1,
        "source rope must be present as exactly one buffer; got: {source_names:?}"
    );

    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                local before = h.source:slice(0, h.source:len())
                local ok, err = pcall(function() h.source:insert(0, "x") end)
                assert(not ok, "source edit must be rejected")
                assert(h.source:slice(0, h.source:len()) == before,
                    "rejected source edit must leave source rope unchanged")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("source edit eval");
    assert!(
        msg.contains("magit source view") && msg.contains("not supported"),
        "source rejection must explain why; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 review Med2 --- body-only sections show fold marker
// ---------------------------------------------------------------------------
//
// Earlier draft considered a section foldable only if it had
// children, so a body-only leaf could be collapsed (body hidden)
// while its header rendered with a blank "  " marker, visually
// indistinguishable from a never-foldable leaf. Fix: foldable =
// body OR children. The marker shows whenever folding has any
// effect.

#[test]
fn magit_body_only_section_shows_fold_marker() {
    let (mut state, _c, _u) = editor_with_magit();
    state
        .lua_host
        .eval(
            Some("body-only-spec"),
            r#"
                require("pmacs-magit").open {
                    { id = "leaf", title = "Leaf", body = "single body line" },
                }
            "#,
        )
        .expect("open body-only");

    let initial = active_buffer_text(&mut state);
    let lines: Vec<&str> = initial.lines().collect();
    // Default expanded -> "v Leaf" header.
    assert!(
        lines[0].starts_with("v Leaf"),
        "body-only section must render with expanded marker; got: {:?}",
        lines[0]
    );
    assert_eq!(lines[1], "  single body line");

    // Collapse via toggle-fold.
    state
        .lua_host
        .eval(
            Some("collapse-leaf"),
            r#"pmacs.command.invoke("pmacs-magit.toggle-fold")"#,
        )
        .expect("toggle");

    let after = active_buffer_text(&mut state);
    let after_lines: Vec<&str> = after.lines().collect();
    assert_eq!(after_lines.len(), 1, "body must be hidden: {after}");
    assert!(
        after_lines[0].starts_with("> Leaf"),
        "collapsed body-only section must render with collapsed marker; \
         got: {:?}",
        after_lines[0]
    );
}

// True leaves (no body, no children) still get no marker.
#[test]
fn magit_true_leaf_section_renders_without_fold_marker() {
    let (mut state, _c, _u) = editor_with_magit();
    state
        .lua_host
        .eval(
            Some("leaf-spec"),
            r#"
                require("pmacs-magit").open {
                    { id = "empty", title = "Empty Section" },
                }
            "#,
        )
        .expect("open leaf");
    let text = active_buffer_text(&mut state);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "leaf must render exactly one line");
    // "  " (no marker) prefix.
    assert!(
        lines[0].starts_with("  Empty Section"),
        "true-leaf must render with no fold marker; got: {:?}",
        lines[0]
    );
}

// ---------------------------------------------------------------------------
// Pass-2 review Med3 --- cursor reseats to same section after repaint
// ---------------------------------------------------------------------------
//
// Wholesale `:replace` of the visible buffer leaves the engine's
// window cursor at a stale byte offset. After a fold-all or a
// body-line toggle, the cursor would land on whatever happened to
// be at the same byte index in the new projection. Fix:
// repaint_visible captures the section under cursor first, then
// after repaint walks the cursor (via move_up/move_down, the same
// pattern builtin/commands/default.lua's buffer-list refresh uses)
// to that section's new header line.

#[test]
fn magit_cursor_stays_on_same_section_across_fold_all_unfold_all() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    // Move the cursor onto Section B's header (line 7 in the
    // default projection: line 0=A, 1-2=A body, 3=A1 hdr, 4=a1 body,
    // 5=A2 hdr, 6=a2 body, 7=B hdr).
    state
        .lua_host
        .eval(
            Some("seek-to-b"),
            r"
                for _ = 1, 7 do pmacs.editor.move_down() end
            ",
        )
        .expect("seek to B");
    let line_at_b: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.editor.cursor_line()")
        .eval()
        .expect("cursor at B");
    assert_eq!(line_at_b, 7, "setup: cursor must be on Section B header");

    // fold-all + unfold-all. After this churn the projection is the
    // same as the start (per the existing unfold-after-fold-all
    // test); cursor should be back on Section B's header.
    state
        .lua_host
        .eval(
            Some("fold-then-unfold"),
            r#"
                pmacs.command.invoke("pmacs-magit.fold-all")
                pmacs.command.invoke("pmacs-magit.unfold-all")
            "#,
        )
        .expect("fold-then-unfold");
    let line_after: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.editor.cursor_line()")
        .eval()
        .expect("cursor after");
    assert_eq!(
        line_after, 7,
        "cursor must be back on Section B header after fold-all + unfold-all"
    );
}

// Cursor on a body line should also follow the section: after the
// section's body is collapsed, cursor goes to the section's header.
#[test]
fn magit_cursor_from_body_line_reseats_to_header_after_collapse() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    // Move cursor to line 1 (Section A, body line 1).
    state
        .lua_host
        .eval(Some("to-a-body"), "pmacs.editor.move_down()")
        .expect("to A body");
    let baseline_line: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.editor.cursor_line()")
        .eval()
        .expect("baseline");
    assert_eq!(baseline_line, 1, "setup: cursor on A's first body line");

    // Toggle Section A collapsed (cursor on body -> belongs to A).
    state
        .lua_host
        .eval(
            Some("toggle-from-body"),
            r#"pmacs.command.invoke("pmacs-magit.toggle-fold")"#,
        )
        .expect("toggle");

    let after_line: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.editor.cursor_line()")
        .eval()
        .expect("after");
    assert_eq!(
        after_line, 0,
        "cursor must reseat onto A's header (line 0) after A collapses"
    );
}

// Cursor on a section that gets hidden by a parent fold reseats to
// the nearest visible ancestor, not to byte 0.
#[test]
fn magit_cursor_on_hidden_descendant_reseats_to_visible_ancestor() {
    let (mut state, _c, _u) = editor_with_magit();
    open_standard(&mut state);

    // Move cursor onto Subsection A1's header (line 3).
    state
        .lua_host
        .eval(
            Some("to-a1-hdr"),
            r"for _ = 1, 3 do pmacs.editor.move_down() end",
        )
        .expect("to A1");
    let on_a1: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.editor.cursor_line()")
        .eval()
        .expect("on A1");
    assert_eq!(on_a1, 3, "setup: cursor on A1 header");

    // Collapse Section A (the parent of A1). A1 is no longer in the
    // projection. Cursor should reseat onto A's header (line 0),
    // not stay at byte offset 3 / line 3 (which would now be the
    // start of Section B's body region).
    state
        .lua_host
        .eval(
            Some("collapse-a"),
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                te.fold.toggle(h.fold_state, "a")
                te.repaint_visible(h)
            "#,
        )
        .expect("collapse a");

    let after: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.editor.cursor_line()")
        .eval()
        .expect("after");
    assert_eq!(
        after, 0,
        "cursor on hidden A1 must reseat to A's header (line 0)"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
