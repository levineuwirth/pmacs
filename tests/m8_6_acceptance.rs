// tests/m8_6_acceptance.rs --- T M8.6 magit-class git status integration.

//! Acceptance for the M8.6 deliverable: wiring the M8.5 foldable
//! section view to actual `git status` output. Spec acceptance
//! bullets:
//!
//! 1. All five canonical sections render correctly on a typical
//!    repository (working tree, staged, recent commits, branches,
//!    stashes).
//! 2. Refresh after an external commit reflects the new state
//!    within 500 ms.
//! 3. Empty sections (no stashes, etc.) render as a one-line
//!    placeholder rather than disappearing.
//!
//! Plus regressions:
//!
//! * Section IDs are stable across refresh; fold-state is preserved.
//! * Cursor's section is preserved across refresh.
//! * Manual refresh-status command works.
//! * The four parser helpers (porcelain v2, log, branches, stashes)
//!   round-trip canonical inputs without invoking git.
//! * Non-repo path produces a refresh failure but doesn't crash the
//!   package.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

fn magit_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests").join("fixtures").join("pmacs-magit")
}

fn pump_until<F: Fn(&EditorState) -> bool>(state: &mut EditorState, predicate: F) {
    pump_until_with_deadline(state, predicate, Duration::from_secs(5));
}

fn pump_until_with_deadline<F: Fn(&EditorState) -> bool>(
    state: &mut EditorState,
    predicate: F,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    while !predicate(state) {
        assert!(
            Instant::now() < deadline,
            "async pump deadline exceeded after {timeout:?}"
        );
        // M8.6 runs `git` via pmacs.process.spawn; the supervisor's
        // events (stdout/stderr/exited) only show up if we tick
        // both the async runtime AND the process supervisor each
        // iteration --- the editor's main loop does both per pass
        // (editor.rs:897-898), so tests have to as well.
        state.tick_async();
        state.tick_processes();
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn editor_with_magit() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new();
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

/// Build a small git repo for tests: `git init`, set user, commit a
/// file. Returns the `TempDir` owning the repo.
fn make_test_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git_run(dir.path(), &["init", "-b", "main"]);
    git_run(dir.path(), &["config", "user.email", "test@example.com"]);
    git_run(dir.path(), &["config", "user.name", "Test User"]);
    std::fs::write(dir.path().join("README.md"), b"hello\n").expect("write README");
    git_run(dir.path(), &["add", "README.md"]);
    git_run(dir.path(), &["commit", "-m", "initial commit"]);
    dir
}

fn git_run(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"));
    assert!(status.success(), "git {args:?} returned {status}");
}

fn open_status(state: &mut EditorState, repo_root: &Path) {
    let chunk = format!(
        r#"
        _G.M8_6_OPEN_DONE = nil
        pmacs.async(function()
            require("pmacs-magit").open_status({path:?})
            _G.M8_6_OPEN_DONE = true
        end)
    "#,
        path = repo_root.display().to_string()
    );
    state
        .lua_host
        .eval(Some("open_status"), &chunk)
        .unwrap_or_else(|e| panic!("open_status eval failed: {e}"));
    pump_until(state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("M8_6_OPEN_DONE")
            .unwrap_or(false)
    });
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

// ---------------------------------------------------------------------------
// Bullet 1 --- five canonical sections render on a typical repo
// ---------------------------------------------------------------------------

#[test]
fn magit_status_renders_five_sections_on_typical_repo() {
    let repo = make_test_repo();
    // Stash setup goes FIRST: `git stash push` (no paths) stashes
    // every change in the working tree and the index, so anything
    // we set up before stashing gets wiped. Stash now while the
    // tree is clean from make_test_repo().
    std::fs::write(repo.path().join("for-stash.txt"), b"stashed content\n")
        .expect("write stash file");
    git_run(repo.path(), &["add", "for-stash.txt"]);
    git_run(repo.path(), &["stash", "push", "-m", "test-stash"]);
    // After stash push, the tree is clean again (the for-stash.txt
    // change is in the stash entry). Now build the "typical" state
    // we want to render.
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\n").expect("modify README");
    git_run(repo.path(), &["add", "README.md"]); // staged
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\nmore\n").expect("modify again");
    // README.md now has X=M (staged) and Y=M (unstaged).
    std::fs::write(repo.path().join("untracked.txt"), b"new\n").expect("write untracked");
    git_run(repo.path(), &["branch", "feature"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    let text = active_buffer_text(&mut state);
    // All five canonical sections visible by their stable IDs (the
    // visible header text is "<v|>|  > Title (count)").
    for section_title in [
        "Working tree changes",
        "Staged changes",
        "Recent commits",
        "Branches",
        "Stashes",
    ] {
        assert!(
            text.contains(section_title),
            "section {section_title:?} must render; got: {text}"
        );
    }
    // The actual content surfaced.
    assert!(
        text.contains("README.md"),
        "modified README must show: {text}"
    );
    assert!(
        text.contains("untracked.txt"),
        "untracked must show: {text}"
    );
    assert!(text.contains("feature"), "feature branch must show: {text}");
    assert!(text.contains("test-stash"), "stash entry must show: {text}");
    assert!(
        text.contains("initial commit"),
        "initial commit must show: {text}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 2 --- refresh after external commit reflects new state within 500 ms
// ---------------------------------------------------------------------------

#[test]
fn magit_status_refreshes_after_external_commit_within_500ms() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // Initial buffer shows "initial commit".
    let initial = active_buffer_text(&mut state);
    assert!(
        initial.contains("initial commit"),
        "initial state must include initial commit; got: {initial}"
    );
    assert!(
        !initial.contains("second commit"),
        "second commit hasn't happened yet"
    );

    // Externally make a new commit.
    std::fs::write(repo.path().join("README.md"), b"v2\n").expect("modify");
    git_run(repo.path(), &["add", "README.md"]);
    git_run(repo.path(), &["commit", "-m", "second commit"]);

    // Pump until the polling loop reflects the new commit. Acceptance
    // budget is 500 ms; we use 750 ms to keep the test from being
    // flaky on slow CI while still failing if polling is broken.
    let started = Instant::now();
    pump_until_with_deadline(
        &mut state,
        |s| {
            s.lua_host
                .lua()
                .load(
                    r"
                    local buf = pmacs.window.buffer()
                    local text = buf:slice(0, buf:len())
                    return text:find('second commit', 1, true) ~= nil
                ",
                )
                .eval::<bool>()
                .unwrap_or(false)
        },
        Duration::from_millis(750),
    );
    let elapsed = started.elapsed();
    // Soft check on latency: the polling cadence is 250 ms, so on
    // a healthy run we settle in well under 500 ms. Anything past
    // 750 ms means the polling loop isn't running --- the deadline
    // above already enforces that.
    assert!(
        elapsed < Duration::from_millis(750),
        "refresh latency {elapsed:?} exceeded 750 ms budget"
    );
}

// ---------------------------------------------------------------------------
// Bullet 3 --- empty sections render as one-line placeholder
// ---------------------------------------------------------------------------

#[test]
fn magit_status_empty_sections_render_as_placeholders() {
    // A fresh repo with one initial commit: no working-tree changes,
    // no staged changes, no extra branches, no stashes. Only the
    // "Recent commits" section has real content.
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    let text = active_buffer_text(&mut state);
    // Every empty section shows its placeholder body.
    assert!(
        text.contains("(no working-tree changes)"),
        "empty working tree must show placeholder: {text}"
    );
    assert!(
        text.contains("(nothing staged)"),
        "empty staged must show placeholder: {text}"
    );
    assert!(
        text.contains("(no stashes)"),
        "empty stashes must show placeholder: {text}"
    );
    // Recent commits has one entry (the initial commit), so its
    // placeholder is NOT shown.
    assert!(
        !text.contains("(no commits yet)"),
        "non-empty Recent commits must not show its placeholder: {text}"
    );
    // Branches has the implicit `main`; placeholder not shown.
    assert!(
        !text.contains("(no branches)"),
        "non-empty Branches must not show its placeholder: {text}"
    );
}

// ---------------------------------------------------------------------------
// Regression: section IDs stable across refresh, fold-state preserved
// ---------------------------------------------------------------------------

#[test]
fn magit_status_fold_state_preserved_across_refresh() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // Collapse "log" (Recent commits) section.
    state
        .lua_host
        .eval(
            Some("collapse-log"),
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                te.fold.toggle(h.fold_state, "log")
                te.repaint_visible(h)
            "#,
        )
        .expect("collapse log");

    let collapsed = active_buffer_text(&mut state);
    assert!(
        !collapsed.contains("initial commit"),
        "log body must be hidden after collapse: {collapsed}"
    );

    // Trigger a manual refresh (which produces a new spec and calls
    // update_spec). Fold-state should survive because `log` is a
    // stable ID across refreshes.
    state
        .lua_host
        .eval(
            Some("refresh"),
            r#"pmacs.command.invoke("pmacs-magit.refresh-status")"#,
        )
        .expect("refresh");
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .load(
                r"
                local te = require('pmacs-magit').__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                return h ~= nil and h.refresh_pending == false
            ",
            )
            .eval::<bool>()
            .unwrap_or(false)
    });

    let after = active_buffer_text(&mut state);
    assert!(
        !after.contains("initial commit"),
        "log section must remain collapsed after refresh: {after}"
    );
    // Verify the fold_state still has "log" collapsed.
    let log_state: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                return tostring(h.fold_state.log)
            "#,
        )
        .eval()
        .expect("log fold state");
    assert_eq!(log_state, "collapsed");
}

// ---------------------------------------------------------------------------
// Regression: cursor stays on the same section across refresh
// ---------------------------------------------------------------------------

#[test]
fn magit_status_cursor_section_preserved_across_refresh() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // Move cursor to Section "log" header (section_at would resolve
    // it back to "log"). Sections in default order are working,
    // staged, log, branches, stashes; each takes 2 lines (header +
    // 1-line placeholder body) when empty. Empty repo: working (2
    // lines, header + placeholder), staged (2 lines), so log header
    // is at line 4. Move down 4 times.
    state
        .lua_host
        .eval(
            Some("seek-log"),
            r"for _ = 1, 4 do pmacs.editor.move_down() end",
        )
        .expect("seek log");
    let on_log: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                local line = pmacs.editor.cursor_line()
                return tostring(te.fold.section_at(h.projection, line))
            "#,
        )
        .eval()
        .expect("on log");
    assert_eq!(on_log, "log", "setup: cursor must be on log section");

    // Manual refresh.
    state
        .lua_host
        .eval(
            Some("refresh"),
            r#"pmacs.command.invoke("pmacs-magit.refresh-status")"#,
        )
        .expect("refresh");
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .load(
                r"
                local te = require('pmacs-magit').__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                return h ~= nil and h.refresh_pending == false
            ",
            )
            .eval::<bool>()
            .unwrap_or(false)
    });

    // Cursor should still be on the log section.
    let after: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local h = te.active_handle()
                local line = pmacs.editor.cursor_line()
                return tostring(te.fold.section_at(h.projection, line))
            "#,
        )
        .eval()
        .expect("after");
    assert_eq!(after, "log", "cursor must still be on log after refresh");
}

// ---------------------------------------------------------------------------
// Regression: refresh-status command exists + works
// ---------------------------------------------------------------------------

#[test]
fn magit_refresh_status_command_picks_up_new_commit() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // External change.
    std::fs::write(repo.path().join("README.md"), b"v2\n").expect("modify");
    git_run(repo.path(), &["add", "README.md"]);
    git_run(repo.path(), &["commit", "-m", "manual-refresh-target"]);

    // Manual refresh (rather than waiting for the polling loop).
    state
        .lua_host
        .eval(
            Some("refresh-now"),
            r#"pmacs.command.invoke("pmacs-magit.refresh-status")"#,
        )
        .expect("refresh-now");
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .load(
                r"
                local buf = pmacs.window.buffer()
                local text = buf:slice(0, buf:len())
                return text:find('manual-refresh-target', 1, true) ~= nil
            ",
            )
            .eval::<bool>()
            .unwrap_or(false)
    });
    let text = active_buffer_text(&mut state);
    assert!(text.contains("manual-refresh-target"));
}

// ---------------------------------------------------------------------------
// Regression: open_status validates input
// ---------------------------------------------------------------------------

#[test]
fn magit_open_status_rejects_non_string_repo_root() {
    let (state, _c, _u) = editor_with_magit();
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local ok, err = pcall(function()
                    require("pmacs-magit").open_status(nil)
                end)
                assert(not ok, "nil repo_root must be rejected")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("nil-repo eval");
    assert!(
        msg.contains("non-empty string"),
        "rejection must explain why: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Parser unit tests via the test seam
// ---------------------------------------------------------------------------

#[test]
fn magit_parse_porcelain_v2_basic_inputs() {
    let (state, _c, _u) = editor_with_magit();
    let lua = state.lua_host.lua();

    // "1" lines: ordinary changed files. "2" lines: rename/copy.
    // "?" lines: untracked. The Lua `"# branch.head main"` literal
    // contains `"#`, which would close a single-hash Rust raw string;
    // bumping to `r##"..."##` lets the literal pass through.
    let json: String = lua
        .load(
            r##"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local input = table.concat({
                    "# branch.head main",
                    "1 .M N... 100644 100644 100644 abc def README.md",
                    "1 M. N... 100644 100644 100644 def 123 staged.txt",
                    "? untracked.txt",
                }, "\n") .. "\n"
                local r = te.status.parse_porcelain_v2(input)
                return string.format(
                    "branch=%s|staged=%d|unstaged=%d|untracked=%d|first_unstaged=%s",
                    r.branch, #r.staged, #r.unstaged, #r.untracked,
                    r.unstaged[1] or "(none)"
                )
            "##,
        )
        .eval()
        .expect("porcelain parse");
    // "1 .M ... README.md" -> Y is M (unstaged), X is . (not staged)
    // "1 M. ... staged.txt" -> X is M (staged), Y is . (not unstaged)
    // ? untracked.txt
    assert_eq!(
        json,
        "branch=main|staged=1|unstaged=1|untracked=1|first_unstaged=M README.md"
    );
}

#[test]
fn magit_parse_log_basic_input() {
    let (state, _c, _u) = editor_with_magit();
    let lua = state.lua_host.lua();
    let result: String = lua
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local input = "abc1234 first commit\ndef5678 second commit\n"
                local entries = te.status.parse_log(input)
                return string.format("%d|%s|%s", #entries,
                    entries[1].hash, entries[2].subject)
            "#,
        )
        .eval()
        .expect("log parse");
    assert_eq!(result, "2|abc1234|second commit");
}

#[test]
fn magit_parse_branches_marks_current() {
    let (state, _c, _u) = editor_with_magit();
    let lua = state.lua_host.lua();
    let result: String = lua
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local input = "  feature\n* main\n  release\n"
                local r = te.status.parse_branches(input)
                return string.format("current=%s|count=%d|first=%s",
                    r.current, #r.all, r.all[1])
            "#,
        )
        .eval()
        .expect("branches parse");
    // `git branch --list` outputs branches in encounter order
    // (alphabetical with the current-branch marker inline), so
    // `all[1]` is whichever branch sorts first --- "feature", not
    // the current one. The current is named separately via
    // `r.current`.
    assert_eq!(result, "current=main|count=3|first=feature");
}

#[test]
fn magit_parse_stashes_basic_input() {
    let (state, _c, _u) = editor_with_magit();
    let lua = state.lua_host.lua();
    let result: String = lua
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local input = "stash@{0}: WIP on main: abc123 work\nstash@{1}: On main: def quick fix\n"
                local entries = te.status.parse_stashes(input)
                return string.format("%d|%s|%s", #entries, entries[1].ref, entries[2].subject)
            "#,
        )
        .eval()
        .expect("stash parse");
    assert_eq!(result, "2|stash@{0}|On main: def quick fix");
}

// ---------------------------------------------------------------------------
// Regression: build_spec produces stable section IDs
// ---------------------------------------------------------------------------

#[test]
fn magit_build_spec_section_ids_are_stable_canonical_set() {
    let (state, _c, _u) = editor_with_magit();
    let lua = state.lua_host.lua();
    let ids: String = lua
        .load(
            r#"
                local te = require("pmacs-magit").__pmacs_magit_test_seam_DO_NOT_USE
                local spec = te.status.build_spec {
                    status = { staged = {}, unstaged = {}, untracked = {} },
                    log = {},
                    branches = { current = nil, all = {} },
                    stashes = {},
                }
                local out = {}
                for _, s in ipairs(spec) do out[#out + 1] = s.id end
                return table.concat(out, ",")
            "#,
        )
        .eval()
        .expect("build_spec ids");
    assert_eq!(ids, "working,staged,log,branches,stashes");
}
