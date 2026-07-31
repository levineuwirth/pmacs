// tests/m8_7_acceptance.rs --- T M8.7 magit-class gestures.

//! Acceptance for the M8.7 deliverable: stage / unstage / commit /
//! push / branch operations as bindings on the magit buffer that
//! act on the section / item under cursor. Multi-step gestures
//! compose with `pmacs.minibuffer.read`.
//!
//! Spec acceptance bullets:
//!
//! 1. Stage / unstage / commit work end-to-end on a real repo.
//! 2. Push to a configured remote works; failure produces a
//!    readable error with the underlying Git output.
//! 3. Branch creation and switching work.
//! 4. All gestures are introspectable via `describe-key` (per the
//!    M2 contract).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::frontend::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pmacs::lua_bindings::PackageInstallOverride;
use pmacs::protocol::FrontendId;
use tempfile::TempDir;

fn plain_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

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
        state.tick_async();
        state.tick_processes();
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

fn git_capture(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} returned {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn open_status(state: &mut EditorState, repo_root: &Path) {
    let chunk = format!(
        r#"
        _G.M8_7_OPEN_DONE = nil
        pmacs.async(function()
            require("pmacs-magit").open_status({path:?})
            _G.M8_7_OPEN_DONE = true
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
            .get::<bool>("M8_7_OPEN_DONE")
            .unwrap_or(false)
    });
}

fn pump_until_refresh_settled(state: &mut EditorState) {
    pump_until(state, |s| {
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

/// Move cursor to the line whose text contains `needle`. Walks the
/// buffer top-to-bottom and stops at the first match. Asserts on
/// not-found.
fn move_cursor_to_line_containing(state: &mut EditorState, needle: &str) {
    let target_line: i64 = state
        .lua_host
        .lua()
        .load(format!(
            r#"
                local buf = pmacs.window.buffer()
                local text = buf:slice(0, buf:len())
                local line = 0
                local pos = 1
                while pos <= #text + 1 do
                    local nl = text:find("\n", pos, true)
                    local end_pos = nl or (#text + 1)
                    local segment = text:sub(pos, end_pos - 1)
                    if segment:find({needle:?}, 1, true) ~= nil then
                        return line
                    end
                    if nl == nil then return -1 end
                    pos = nl + 1
                    line = line + 1
                end
                return -1
            "#
        ))
        .eval()
        .expect("find line eval");
    assert!(
        target_line >= 0,
        "buffer does not contain line with {needle:?}"
    );
    state
        .lua_host
        .eval(
            Some("move-to-target"),
            &format!("for _ = 1, {target_line} do pmacs.editor.move_down() end"),
        )
        .expect("move-to-target");
}

// ---------------------------------------------------------------------------
// Bullet 1 --- stage / unstage / commit work end-to-end
// ---------------------------------------------------------------------------

#[test]
fn magit_stage_gesture_moves_file_from_working_to_staged() {
    let repo = make_test_repo();
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\n").expect("modify README");

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    let before = active_buffer_text(&mut state);
    assert!(
        before.contains("Working tree changes (1)"),
        "before stage: README.md should be in working: {before}"
    );
    assert!(
        before.contains("Staged changes (0)"),
        "before stage: nothing staged: {before}"
    );

    move_cursor_to_line_containing(&mut state, "README.md");
    state
        .lua_host
        .eval(
            Some("invoke-stage"),
            r#"pmacs.command.invoke("pmacs-magit.stage")"#,
        )
        .expect("invoke stage");
    pump_until_refresh_settled(&mut state);

    let after = active_buffer_text(&mut state);
    assert!(
        after.contains("Working tree changes (0)"),
        "after stage: working should be empty: {after}"
    );
    assert!(
        after.contains("Staged changes (1)"),
        "after stage: README.md should be staged: {after}"
    );
    // Disk-side check: porcelain shows X=M.
    let porcelain = git_capture(repo.path(), &["status", "--porcelain=v2"]);
    assert!(
        porcelain.contains("1 M"),
        "git status should show staged file: {porcelain}"
    );
}

#[test]
fn magit_unstage_gesture_moves_file_from_staged_to_working() {
    let repo = make_test_repo();
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\n").expect("modify");
    git_run(repo.path(), &["add", "README.md"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    let before = active_buffer_text(&mut state);
    assert!(before.contains("Staged changes (1)"));

    move_cursor_to_line_containing(&mut state, "README.md");
    state
        .lua_host
        .eval(
            Some("invoke-unstage"),
            r#"pmacs.command.invoke("pmacs-magit.unstage")"#,
        )
        .expect("invoke unstage");
    pump_until_refresh_settled(&mut state);

    let after = active_buffer_text(&mut state);
    assert!(
        after.contains("Working tree changes (1)"),
        "after unstage: README.md back in working: {after}"
    );
    assert!(
        after.contains("Staged changes (0)"),
        "after unstage: staged is empty: {after}"
    );
}

#[test]
fn magit_commit_gesture_opens_commit_message_buffer_and_creates_commit() {
    let repo = make_test_repo();
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\n").expect("modify");
    git_run(repo.path(), &["add", "README.md"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // Invoke commit gesture -> opens *magit-commit* buffer (not a
    // minibuffer prompt, per M8.7's "composes a message buffer"
    // requirement).
    state
        .lua_host
        .eval(
            Some("invoke-commit"),
            r#"pmacs.command.invoke("pmacs-magit.commit")"#,
        )
        .expect("invoke commit");

    // Active buffer should now be *magit-commit*; minibuffer is NOT active.
    let active_name: String = state
        .lua_host
        .lua()
        .load(
            r"
            local id = pmacs.window.buffer()
            local d = pmacs.describe.buffer(id)
            return d and d.name or '<nil>'
        ",
        )
        .eval()
        .expect("active buffer name");
    assert_eq!(active_name, "*magit-commit*");
    let mb_active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("mb active");
    assert!(!mb_active, "commit must use a buffer, not the minibuffer");

    // Type a multiline message into the buffer (subject + body).
    // The package's scaffold is "<empty subject line>\n\n# comment lines\n".
    // We replace the first line with "subject" and add a body.
    state
        .lua_host
        .eval(
            Some("type-message"),
            r#"
                local buf = pmacs.window.buffer()
                local existing = buf:slice(0, buf:len())
                local body = "subject line\n\nbody paragraph one.\nbody paragraph two.\n\n" ..
                             existing:gsub("^[^#]*", "")
                buf:replace(0, buf:len(), body)
            "#,
        )
        .expect("type message");

    // Submit via the C-c C-c command (binding goes through the
    // dispatch path in real use; we invoke directly here to keep
    // the test focused on the buffer flow, then have a separate
    // test exercise the dispatch path).
    state
        .lua_host
        .eval(
            Some("submit"),
            r#"pmacs.command.invoke("pmacs-magit.commit-submit")"#,
        )
        .expect("submit");
    pump_until_refresh_settled(&mut state);

    // Disk-side: commit landed with multiline message.
    let log = git_capture(repo.path(), &["log", "-1", "--format=%B"]);
    assert!(
        log.contains("subject line") && log.contains("body paragraph one"),
        "git log -1 must include the multiline message: {log}"
    );
    // Comment lines should NOT be in the message.
    assert!(
        !log.contains("# Please enter") && !log.contains("# Branch:"),
        "comment lines must be stripped from the commit message: {log}"
    );
    // Buffer reflects the new commit.
    let after = active_buffer_text(&mut state);
    assert!(
        after.contains("subject line"),
        "magit buffer should show new commit: {after}"
    );
}

#[test]
fn magit_commit_buffer_cancel_closes_buffer_without_committing() {
    let repo = make_test_repo();
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\n").expect("modify");
    git_run(repo.path(), &["add", "README.md"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());
    let head_before = git_capture(repo.path(), &["rev-parse", "HEAD"]);

    state
        .lua_host
        .eval(
            Some("invoke-commit"),
            r#"pmacs.command.invoke("pmacs-magit.commit")"#,
        )
        .expect("invoke");
    state
        .lua_host
        .eval(
            Some("cancel"),
            r#"pmacs.command.invoke("pmacs-magit.commit-cancel")"#,
        )
        .expect("cancel");

    // Active buffer is back to the magit buffer.
    let active_name: String = state
        .lua_host
        .lua()
        .load(
            r"
            local id = pmacs.window.buffer()
            local d = pmacs.describe.buffer(id)
            return d and d.name or '<nil>'
        ",
        )
        .eval()
        .expect("active buffer name");
    assert!(
        active_name.starts_with("*magit:"),
        "after cancel, active buffer must be the magit buffer; got: {active_name}"
    );
    // HEAD unchanged.
    let head_after = git_capture(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(head_before, head_after);
}

#[test]
fn magit_commit_buffer_c_c_c_c_dispatch_submits() {
    let repo = make_test_repo();
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\n").expect("modify");
    git_run(repo.path(), &["add", "README.md"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    state
        .lua_host
        .eval(
            Some("open-commit-buffer"),
            r#"pmacs.command.invoke("pmacs-magit.commit")"#,
        )
        .expect("open");
    // Set a non-empty message.
    state
        .lua_host
        .eval(
            Some("set-msg"),
            r#"
                local buf = pmacs.window.buffer()
                buf:replace(0, buf:len(), "via-c-c-c-c-dispatch\n")
            "#,
        )
        .expect("set");

    // Dispatch C-c C-c through the actual key path. This pins the
    // buffer-local multi-key binding works through dispatch (per
    // the auto-memory feedback that command.invoke bypasses dispatch
    // and dead bindings would silently pass).
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        },
    );
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        },
    );
    pump_until_refresh_settled(&mut state);

    let log = git_capture(repo.path(), &["log", "-1", "--format=%s"]);
    assert!(
        log.contains("via-c-c-c-c-dispatch"),
        "C-c C-c dispatch must submit the commit: {log}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 2 --- push to a configured remote
// ---------------------------------------------------------------------------

#[test]
fn magit_push_gesture_pushes_to_configured_remote() {
    // Set up a bare-repo "remote" alongside the working repo and add
    // it as origin.
    let remote_dir = tempfile::tempdir().expect("remote tempdir");
    git_run(remote_dir.path(), &["init", "--bare", "-b", "main"]);

    let repo = make_test_repo();
    // Pre-test setup is *only* "git remote add origin <url>". The
    // gesture must work on first push without manual `-u origin main`
    // priming --- per the M8.8 audit finding 3 fix, the gesture
    // passes `-u <remote> HEAD` so first-push sets upstream tracking
    // automatically.
    git_run(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            &remote_dir.path().display().to_string(),
        ],
    );

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // Invoke push -> opens minibuffer with "origin" as initial.
    state
        .lua_host
        .eval(
            Some("invoke-push"),
            r#"pmacs.command.invoke("pmacs-magit.push")"#,
        )
        .expect("invoke push");
    // Accept the default ("origin" was the initial value).
    let initial: String = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.contents()")
        .eval()
        .expect("mb contents");
    assert_eq!(initial, "origin", "minibuffer initial must be 'origin'");
    state
        .lua_host
        .eval(Some("accept-push"), r"pmacs.minibuffer.accept()")
        .expect("accept push");
    pump_until_refresh_settled(&mut state);

    // The remote bare repo now has the initial commit (we didn't
    // pre-prime upstream, so this is genuinely the first push).
    let remote_log = git_capture(remote_dir.path(), &["log", "--oneline", "-n", "5"]);
    assert!(
        remote_log.contains("initial commit"),
        "remote should have the pushed commit on first push without manual `-u` priming: {remote_log}"
    );
    // Upstream tracking should now be configured (the `-u` flag did its job).
    let upstream = git_capture(repo.path(), &["rev-parse", "--abbrev-ref", "main@{u}"]);
    assert_eq!(
        upstream.trim(),
        "origin/main",
        "first push should set upstream tracking via -u"
    );
}

#[test]
fn magit_push_failure_surfaces_error() {
    let repo = make_test_repo();
    // No remote configured; push will fail.
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    state
        .lua_host
        .eval(
            Some("invoke-push-fail"),
            r#"pmacs.command.invoke("pmacs-magit.push")"#,
        )
        .expect("invoke push");
    state
        .lua_host
        .eval(Some("accept-default"), "pmacs.minibuffer.accept()")
        .expect("accept");
    pump_until_refresh_settled(&mut state);

    // The package shouldn't crash; the buffer should still show its
    // previous state.
    let after = active_buffer_text(&mut state);
    assert!(
        after.contains("initial commit"),
        "buffer should still be valid after a failed push: {after}"
    );

    // The M8.7 spec requires "failure produces a readable error
    // with the underlying Git output" --- that's
    // pmacs.editor.set_status's job in run_and_refresh. Read it via
    // the editor core (no public getter for the status line in v0.1;
    // pmacs.editor exposes set_status but not get_status, so tests
    // reach into the SharedCore the same way they do for disk-side
    // assertions).
    let status_line = state.core.borrow().status.clone();
    assert!(
        status_line.contains("pmacs-magit.push: git failed"),
        "modeline must surface the failure with the gesture's name + 'git failed' prefix; got: {status_line:?}"
    );
    // Underlying Git output: with no remote configured, git's stderr
    // typically begins with "fatal:" --- e.g.,
    //   "fatal: 'origin' does not appear to be a git repository"
    // We don't pin the exact wording (git's message text changes
    // across versions), but the "fatal:" marker is stable across
    // git versions for this failure class.
    assert!(
        status_line.contains("fatal:"),
        "modeline must include git's stderr (recognizable by 'fatal:' prefix); got: {status_line:?}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 3 --- branch creation and switching
// ---------------------------------------------------------------------------

#[test]
fn magit_branch_create_gesture_creates_and_checks_out_new_branch() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    state
        .lua_host
        .eval(
            Some("invoke-branch-create"),
            r#"pmacs.command.invoke("pmacs-magit.branch-create")"#,
        )
        .expect("invoke");
    state
        .lua_host
        .eval(
            Some("type-name"),
            r#"
                pmacs.minibuffer.set_contents("topic/m8.7-test")
                pmacs.minibuffer.accept()
            "#,
        )
        .expect("set + accept");
    pump_until_refresh_settled(&mut state);

    // Disk-side: branch exists and is current.
    let branches = git_capture(repo.path(), &["branch", "--list"]);
    assert!(
        branches.contains("* topic/m8.7-test"),
        "new branch must exist and be current: {branches}"
    );
}

#[test]
fn magit_branch_switch_gesture_switches_to_existing_branch() {
    let repo = make_test_repo();
    git_run(repo.path(), &["branch", "feature"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    state
        .lua_host
        .eval(
            Some("invoke-branch-switch"),
            r#"pmacs.command.invoke("pmacs-magit.branch-switch")"#,
        )
        .expect("invoke");
    state
        .lua_host
        .eval(
            Some("set-target"),
            r#"
                pmacs.minibuffer.set_contents("feature")
                pmacs.minibuffer.accept()
            "#,
        )
        .expect("set + accept");
    pump_until_refresh_settled(&mut state);

    let branches = git_capture(repo.path(), &["branch", "--list"]);
    assert!(
        branches.contains("* feature"),
        "feature must be current branch after switch: {branches}"
    );
}

#[test]
fn magit_branch_switch_minibuffer_offers_existing_branches_as_candidates() {
    let repo = make_test_repo();
    git_run(repo.path(), &["branch", "feature"]);
    git_run(repo.path(), &["branch", "release"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    state
        .lua_host
        .eval(
            Some("invoke-branch-switch"),
            r#"pmacs.command.invoke("pmacs-magit.branch-switch")"#,
        )
        .expect("invoke");
    let candidates: Vec<String> = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.candidates()")
        .eval()
        .expect("candidates");
    // The branches snapshot is { "feature", "main", "release" }
    // (encounter order from `git branch --list`). All three should
    // be in the minibuffer candidate list.
    for branch in ["feature", "main", "release"] {
        assert!(
            candidates.iter().any(|c| c == branch),
            "branch {branch} must be in candidates: {candidates:?}"
        );
    }

    // Cancel the prompt rather than accept a selection.
    state
        .lua_host
        .eval(Some("cancel"), "pmacs.minibuffer.cancel()")
        .expect("cancel");
}

// ---------------------------------------------------------------------------
// Bullet 4 --- describe-key introspection
// ---------------------------------------------------------------------------

#[test]
fn magit_gestures_are_introspectable_via_describe() {
    let (state, _c, _u) = editor_with_magit();
    let lua = state.lua_host.lua();
    // Per the M2 contract, every defined command has a name and
    // description retrievable via `pmacs.describe.command(name)`.
    // Each gesture must be present and carry a non-empty description.
    for name in [
        "pmacs-magit.stage",
        "pmacs-magit.unstage",
        "pmacs-magit.commit",
        "pmacs-magit.commit-submit",
        "pmacs-magit.commit-cancel",
        "pmacs-magit.push",
        "pmacs-magit.branch-create",
        "pmacs-magit.branch-switch",
    ] {
        let desc: String = lua
            .load(format!(
                r#"
                    local info = pmacs.describe.command({name:?})
                    if info == nil then return "<missing>" end
                    return info.description or ""
                "#
            ))
            .eval()
            .unwrap_or_else(|e| panic!("describe.command({name}) failed: {e}"));
        assert!(
            !desc.is_empty() && desc != "<missing>",
            "command {name} must be defined with a non-empty description; got {desc:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Regression: cursor-on-irrelevant-section gestures are no-ops
// ---------------------------------------------------------------------------

#[test]
fn magit_stage_on_recent_commits_section_is_a_clear_no_op() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // Cursor lands on byte 0 (Section A header). Move to a Recent
    // commits line.
    move_cursor_to_line_containing(&mut state, "initial commit");
    state
        .lua_host
        .eval(
            Some("invoke-stage-noop"),
            r#"pmacs.command.invoke("pmacs-magit.stage")"#,
        )
        .expect("invoke");
    // Pump a moment to let any async errors surface.
    for _ in 0..5 {
        state.tick_async();
        state.tick_processes();
        std::thread::sleep(Duration::from_millis(2));
    }
    // Buffer is unchanged.
    let after = active_buffer_text(&mut state);
    assert!(
        after.contains("Recent commits (1)"),
        "log section unchanged: {after}"
    );
    assert!(
        after.contains("Working tree changes (0)"),
        "working unchanged: {after}"
    );
}

// ---------------------------------------------------------------------------
// Regression: empty commit message (only comments) is rejected
// ---------------------------------------------------------------------------
//
// The buffer-flow's submit path strips comment lines and trailing
// blanks before checking emptiness; a buffer with only the scaffold
// comment lines (no real subject) must NOT fire git commit.

#[test]
fn magit_commit_buffer_with_only_comments_is_rejected() {
    let repo = make_test_repo();
    std::fs::write(repo.path().join("README.md"), b"hello\nworld\n").expect("modify");
    git_run(repo.path(), &["add", "README.md"]);

    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());
    let head_before = git_capture(repo.path(), &["rev-parse", "HEAD"]);

    state
        .lua_host
        .eval(
            Some("invoke-commit"),
            r#"pmacs.command.invoke("pmacs-magit.commit")"#,
        )
        .expect("invoke");
    // Don't add any subject; the scaffold only contains comment
    // lines, so submit should reject.
    state
        .lua_host
        .eval(
            Some("submit-empty"),
            r#"pmacs.command.invoke("pmacs-magit.commit-submit")"#,
        )
        .expect("submit");
    for _ in 0..10 {
        state.tick_async();
        state.tick_processes();
        std::thread::sleep(Duration::from_millis(5));
    }
    let head_after = git_capture(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        head_before, head_after,
        "comment-only buffer must not commit"
    );

    // Active buffer is still *magit-commit* --- submit didn't tear
    // it down, leaving the user a chance to add content.
    let active_name: String = state
        .lua_host
        .lua()
        .load(
            r"
            local id = pmacs.window.buffer()
            local d = pmacs.describe.buffer(id)
            return d and d.name or '<nil>'
        ",
        )
        .eval()
        .expect("active name");
    assert_eq!(active_name, "*magit-commit*");
}

// ---------------------------------------------------------------------------
// M8.8 audit in-audit fix: multi-key binding dispatch coverage
// ---------------------------------------------------------------------------
//
// Per the auto-memory feedback (and M8.4 audit finding 7): buffer-
// scope keybinding tests must drive `dispatch_key` --- invoking the
// command directly bypasses dispatch, so a dead binding would
// silently pass under the existing M8.7 tests. The `b c` /
// `b b` multi-key bindings are the highest-risk cases (first time
// magit-class uses multi-key sequences); pin one path so a future
// keymap-tree refactor can't break them silently.

// ---------------------------------------------------------------------------
// M8.8 audit finding 1 fix: describe-key surfaces buffer-local bindings
// ---------------------------------------------------------------------------
//
// Per M2's describe-key contract, every gesture must be
// introspectable via the describe-key path. Earlier `pmacs.describe
// .key` always resolved with `active_buffer = None`, so buffer-
// local bindings (the entire magit gesture set) were invisible. The
// audit fix threads the active window's buffer through; this test
// pins that buffer-local bindings now resolve correctly via the
// public describe.key API, including for multi-key sequences.

#[test]
fn magit_describe_key_resolves_buffer_local_gesture_bindings() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // For each gesture binding, describe.key on the magit buffer
    // must resolve to the corresponding command. (The active
    // window is the magit buffer post-open_status.)
    let probes: &[(&str, &str)] = &[
        ("s", "pmacs-magit.stage"),
        ("u", "pmacs-magit.unstage"),
        ("c", "pmacs-magit.commit"),
        ("P", "pmacs-magit.push"),
        ("g", "pmacs-magit.refresh-status"),
        ("Tab", "pmacs-magit.toggle-fold"),
        ("b c", "pmacs-magit.branch-create"),
        ("b b", "pmacs-magit.branch-switch"),
    ];
    for (sequence, expected_cmd) in probes {
        let cmd: String = state
            .lua_host
            .lua()
            .load(format!(
                r#"
                    local info = pmacs.describe.key({sequence:?})
                    if info == nil then return "<unbound>" end
                    return info.command
                "#
            ))
            .eval()
            .unwrap_or_else(|e| panic!("describe.key({sequence:?}) failed: {e}"));
        assert_eq!(
            cmd, *expected_cmd,
            "describe.key({sequence:?}) must surface the buffer-local binding"
        );
    }
}

#[test]
fn magit_describe_key_resolves_commit_buffer_bindings() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    state
        .lua_host
        .eval(
            Some("open-commit-buffer"),
            r#"pmacs.command.invoke("pmacs-magit.commit")"#,
        )
        .expect("open commit buffer");

    let probes: &[(&str, &str)] = &[
        ("C-c C-c", "pmacs-magit.commit-submit"),
        ("C-c C-k", "pmacs-magit.commit-cancel"),
    ];
    for (sequence, expected_cmd) in probes {
        let cmd: String = state
            .lua_host
            .lua()
            .load(format!(
                r#"
                    local info = pmacs.describe.key({sequence:?})
                    if info == nil then return "<unbound>" end
                    return info.command
                "#
            ))
            .eval()
            .unwrap_or_else(|e| panic!("describe.key({sequence:?}) failed: {e}"));
        assert_eq!(
            cmd, *expected_cmd,
            "describe.key({sequence:?}) must surface the commit-buffer binding"
        );
    }

    state
        .lua_host
        .eval(
            Some("cancel-commit-buffer"),
            r#"pmacs.command.invoke("pmacs-magit.commit-cancel")"#,
        )
        .expect("cancel commit buffer");
}

#[test]
fn magit_b_c_keybinding_dispatches_to_branch_create_prompt() {
    let repo = make_test_repo();
    let (mut state, _c, _u) = editor_with_magit();
    open_status(&mut state, repo.path());

    // Press `b` then `c`. After both keys, the multi-key sequence
    // resolves to "b c" -> pmacs-magit.branch-create -> opens the
    // minibuffer prompt for the new branch name.
    state.dispatch_key(FrontendId::LOCAL, plain_key(KeyCode::Char('b')));
    state.dispatch_key(FrontendId::LOCAL, plain_key(KeyCode::Char('c')));

    let prompt: String = state
        .lua_host
        .lua()
        .load(
            r"
            if not pmacs.minibuffer.is_active() then return '' end
            return pmacs.minibuffer.contents()
        ",
        )
        .eval()
        .expect("mb state");
    let active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("mb active");
    assert!(
        active,
        "after `b c` the branch-create prompt must be active; \
         minibuffer contents was {prompt:?}"
    );
    // Cancel the prompt to leave editor state clean for teardown.
    state
        .lua_host
        .eval(Some("cancel"), "pmacs.minibuffer.cancel()")
        .expect("cancel");
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
