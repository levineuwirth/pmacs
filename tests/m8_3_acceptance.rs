// tests/m8_3_acceptance.rs --- T M8.3 wdired (editable rename / chmod) acceptance.

//! Acceptance for the M8.3 deliverable: layering an editable
//! rename / chmod surface on top of the M8.2 dired view. Pins the
//! four spec bullets:
//!
//! 1. Renaming a file by editing its name and committing produces
//!    the matching `rename` syscall.
//! 2. Editing permissions to an invalid pattern produces a
//!    rejection at the `intercept_edit` layer, not at the syscall.
//! 3. Editing the size column (which is read-only) is rejected
//!    cleanly.
//! 4. Concurrent external file changes during edit produce a
//!    refresh prompt rather than corrupting the user's edits.
//!
//! Plus several read-only-column regressions: kind char, mtime
//! column, separators, and newline insertion.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

fn dired_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests").join("fixtures").join("pmacs-dired")
}

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

fn editor_with_dired() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    state.lua_host.reopen_init_phase_for_testing();
    state.lua_host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    let pkg = dired_package_path();
    let pkg_str = pkg.display().to_string();
    let install = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("pmacs-dired")
    "#
    );
    state
        .lua_host
        .eval(Some("dired-install"), &install)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));
    (state, cache, user_root)
}

/// Open `path` in dired and pump until M.open returns. Active buffer
/// is the dired buffer afterward; a wdired session can begin via
/// the wdired-edit command.
fn open_dired_on(state: &mut EditorState, path: &str) {
    let chunk = format!(
        r#"
        _G.M8_3_DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{path}")
            _G.M8_3_DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-for-wdired"), &chunk)
        .unwrap_or_else(|e| panic!("dired open failed: {e}"));
    pump_until(state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("M8_3_DONE")
            .unwrap_or(false)
    });
}

/// Pump until the active handle's `last_commit_outcome` is set
/// (success or failure). The wdired-commit command schedules a
/// pmacs.async whose outcome lands there; we don't observe it via
/// pcall on invoke because async errors are surfaced through
/// pmacs.error, not raised back.
fn pump_until_commit_done(state: &mut EditorState) {
    pump_until(state, |s| {
        s.lua_host
            .lua()
            .load(
                r"
                    local h = require('pmacs-dired')._test.active_handle()
                    return h ~= nil and h.last_commit_outcome ~= nil
                ",
            )
            .eval::<bool>()
            .unwrap_or(false)
    });
}

/// Read the active handle's `last_commit_outcome` ("ok" / "failed: ...").
fn read_commit_outcome(state: &mut EditorState) -> String {
    state
        .lua_host
        .lua()
        .load(
            r"
                local h = require('pmacs-dired')._test.active_handle()
                return tostring(h.last_commit_outcome)
            ",
        )
        .eval::<String>()
        .unwrap_or_default()
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

fn edit_wdired_names(state: &mut EditorState, pairs: &[(&str, &str)]) {
    let pairs_lua = pairs
        .iter()
        .map(|(from, to)| format!("[{from:?}] = {to:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let chunk = format!(
        r#"
            local replacements = {{ {pairs_lua} }}
            local h = require("pmacs-dired")._test.active_handle()
            local NAME_START = require("pmacs-dired")._test.NAME_START
            local function line_end(i)
                if i < #h.edit.line_start_marks then
                    return h.edit.line_start_marks[i + 1]:get() - 1
                end
                return h.buf:len()
            end
            for i, e in ipairs(h.edit.snapshot) do
                local to = replacements[e.name]
                if to then
                    local ls = h.edit.line_start_marks[i]:get()
                    h.buf:replace(ls + NAME_START, line_end(i), to)
                end
            end
        "#
    );
    state
        .lua_host
        .eval(Some("edit-wdired-names"), &chunk)
        .expect("edit wdired names");
}

fn install_second_rename_failure_hook(state: &mut EditorState) {
    state
        .lua_host
        .eval(
            Some("hook-second-rename"),
            r#"
                local real = pmacs.fs.rename
                local calls = 0
                pmacs.fs.rename = function(from, to)
                    calls = calls + 1
                    if calls == 2 then
                        return setmetatable({}, {
                            __index = {
                                await = function()
                                    error("synthetic mid-batch rename failure")
                                end,
                            },
                        })
                    end
                    return real(from, to)
                end
            "#,
        )
        .expect("install rename hook");
}

fn wdired_edit_active(state: &mut EditorState) -> bool {
    state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                return h.edit ~= nil
            "#,
        )
        .eval()
        .expect("read edit state")
}

fn file_mtime_parts(path: &std::path::Path) -> (u64, u32) {
    let modified = std::fs::metadata(path)
        .expect("metadata")
        .modified()
        .expect("modified");
    let since_epoch = modified
        .duration_since(std::time::UNIX_EPOCH)
        .expect("test mtimes are post-epoch");
    (since_epoch.as_secs(), since_epoch.subsec_nanos())
}

// ---------------------------------------------------------------------------
// Bullet 1 --- rename via name edit + commit
// ---------------------------------------------------------------------------

#[test]
fn wdired_rename_via_name_edit_then_commit_produces_rename_syscall() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("old.txt"), b"contents").expect("write old");

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();
    open_dired_on(&mut state, &path_str);

    // Enter wdired edit mode.
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // The line for "old.txt" has the name beginning at byte 39 of
    // the line. Replace the entire name segment ([line_start+39,
    // eol)) with "new.txt".
    state
        .lua_host
        .eval(
            Some("rename-edit"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local mark = h.edit.line_start_marks[1]
                local ls = mark:get()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local eol = h.buf:len()
                local name_start = ls + NAME_START
                h.buf:replace(name_start, eol, "new.txt")
            "#,
        )
        .expect("rename edit");

    // Commit. The command schedules a pmacs.async; poll the
    // handle's last_commit_outcome to detect completion.
    state
        .lua_host
        .eval(
            Some("wd-commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert_eq!(outcome, "ok", "commit must succeed; got: {outcome}");
    // The file is now at the new path; the old path no longer exists.
    assert!(
        dir.path().join("new.txt").exists(),
        "after wdired-commit, new.txt must exist on disk"
    );
    assert!(
        !dir.path().join("old.txt").exists(),
        "after wdired-commit, old.txt must be gone from disk"
    );

    // The buffer reflects the post-commit listing.
    let text = active_buffer_text(&mut state);
    assert!(
        text.contains("new.txt") && !text.contains(" old.txt"),
        "post-commit dired buffer must list new.txt only; got: {text}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 2 --- invalid perms char rejected at intercept_edit
// ---------------------------------------------------------------------------

#[test]
fn wdired_invalid_perms_char_rejected_at_intercept_layer() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write file");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());

    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Replace one perm char with 'z' (not in the rwx alphabet). We
    // use replace (not insert) because the perms column is fixed-
    // width: an insert would shift the column and is rejected for
    // a different reason --- the alphabet check is for length-
    // preserving same-column replaces.
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                -- Position 1 is the first perms char; replace it with 'z'.
                local ok, err = pcall(function()
                    h.buf:replace(ls + 1, ls + 2, "z")
                end)
                assert(not ok, "intercept must reject 'z' in perms")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("intercept error eval");
    assert!(
        msg.contains("perms column") && msg.contains("'r', 'w', 'x', and '-'"),
        "rejection must name the rwx alphabet; got: {msg}"
    );

    // Buffer is unchanged: file mode unchanged on disk because no
    // chmod ran (we never reached commit).
    let mode_before = std::fs::metadata(dir.path().join("file.txt"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    // Run wdired-abandon to clean up, then verify nothing happened.
    state
        .lua_host
        .eval(
            Some("abandon"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-abandon")"#,
        )
        .expect("abandon");
    let mode_after = std::fs::metadata(dir.path().join("file.txt"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode_before, mode_after, "no chmod should have fired");
}

// ---------------------------------------------------------------------------
// Bullet 3 --- size column is read-only and rejected
// ---------------------------------------------------------------------------

#[test]
fn wdired_size_column_edits_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"hello").expect("write file");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());

    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Try to insert into the size column. Per the layout, size starts
    // at relative offset 11 (PERMS_END + 1).
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                local ok, err = pcall(function() h.buf:insert(ls + 13, "9") end)
                assert(not ok, "intercept must reject size-column edits")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("size-column eval");
    assert!(
        msg.contains("read-only"),
        "rejection must name the read-only column; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 4 --- external change between edit and commit produces refresh
// ---------------------------------------------------------------------------

#[test]
fn wdired_external_change_at_commit_aborts_with_refresh_guidance() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"a").expect("write a");
    std::fs::write(dir.path().join("b.txt"), b"b").expect("write b");

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();
    open_dired_on(&mut state, &path_str);

    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Edit a.txt's name to a_new.txt.
    state
        .lua_host
        .eval(
            Some("name-edit"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local ls1 = h.edit.line_start_marks[1]:get()
                local ls2 = h.edit.line_start_marks[2]:get()
                -- a.txt is on line 1; name region is [ls1 + NAME_START, ls2 - 1).
                local name_start = ls1 + NAME_START
                local eol = ls2 - 1  -- position of the trailing '\n'
                h.buf:replace(name_start, eol, "a_new.txt")
            "#,
        )
        .expect("name-edit");

    // Externally rename b.txt -> b_external.txt while the user is
    // mid-edit. This is the "concurrent external file change"
    // scenario.
    std::fs::rename(dir.path().join("b.txt"), dir.path().join("b_external.txt"))
        .expect("external rename");

    // Try to commit: should refuse with refresh-prompt guidance.
    state
        .lua_host
        .eval(
            Some("commit-attempt"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("failed: "),
        "commit must fail when external changes happened; got: {outcome}"
    );
    assert!(
        outcome.contains("external"),
        "error must say 'external'; got: {outcome}"
    );
    assert!(
        outcome.contains("wdired-abandon"),
        "error must point at wdired-abandon; got: {outcome}"
    );

    // a.txt was NOT renamed (commit aborted before any syscalls).
    assert!(
        dir.path().join("a.txt").exists(),
        "a.txt must remain since commit aborted"
    );
    assert!(
        !dir.path().join("a_new.txt").exists(),
        "a_new.txt must not exist since commit aborted"
    );
}

// ---------------------------------------------------------------------------
// Regressions: read-only column edges
// ---------------------------------------------------------------------------

#[test]
fn wdired_kind_char_edit_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                local ok, err = pcall(function() h.buf:replace(ls, ls + 1, "d") end)
                assert(not ok, "kind-char edit must be rejected")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("kind-char eval");
    assert!(
        msg.contains("read-only"),
        "rejection must name read-only: {msg}"
    );
}

#[test]
fn wdired_mtime_column_edit_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                -- Mtime is at relative offset 22..38; pick 25.
                local ok, err = pcall(function() h.buf:insert(ls + 25, "X") end)
                assert(not ok, "mtime edit must be rejected")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("mtime eval");
    assert!(msg.contains("read-only"), "must name read-only: {msg}");
}

#[test]
fn wdired_newline_insert_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                -- Try to insert a newline in the editable name region;
                -- still rejected because newlines split lines.
                local ok, err = pcall(function() h.buf:insert(ls + NAME_START + 1, "\n") end)
                assert(not ok, "newline insert must be rejected")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("newline eval");
    assert!(
        msg.contains("newline"),
        "rejection must name newline: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 1 companion --- chmod via perms edit + commit produces chmod syscall
// ---------------------------------------------------------------------------

#[test]
fn wdired_chmod_via_perms_edit_then_commit_changes_mode_on_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("file.txt");
    std::fs::write(&file, b"x").expect("write");
    // Set a known starting mode so we can detect the change.
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("chmod 0o644");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());

    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Replace the perms column "rw-r--r--" (0o644) with "rwxr-x---"
    // (0o750). The perms region is [line_start + 1, line_start + 10).
    state
        .lua_host
        .eval(
            Some("chmod-edit"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                h.buf:replace(ls + 1, ls + 10, "rwxr-x---")
            "#,
        )
        .expect("chmod edit");

    state
        .lua_host
        .eval(
            Some("wd-commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert_eq!(outcome, "ok", "commit must succeed; got: {outcome}");
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o750,
        "chmod via wdired-commit must set mode 0o750; got 0o{mode:o}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 1 companion --- empty name rejected at commit
// ---------------------------------------------------------------------------

#[test]
fn wdired_commit_rejects_empty_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Replace the name with empty.
    state
        .lua_host
        .eval(
            Some("blank-name"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local ls = h.edit.line_start_marks[1]:get()
                h.buf:replace(ls + NAME_START, h.buf:len(), "")
            "#,
        )
        .expect("blank-name edit");

    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);
    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("failed: "),
        "empty name must be rejected; got: {outcome}"
    );
    assert!(
        outcome.contains("empty"),
        "error must name 'empty'; got: {outcome}"
    );
    // File on disk untouched.
    assert!(
        dir.path().join("file.txt").exists(),
        "file.txt must remain since commit was rejected"
    );
}

// Slashed-name companion --- a basename containing '/' is forbidden
// (would escape the directory). Sibling of the empty-name test.
#[test]
fn wdired_commit_rejects_slashed_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    state
        .lua_host
        .eval(
            Some("slashed"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local ls = h.edit.line_start_marks[1]:get()
                h.buf:replace(ls + NAME_START, h.buf:len(), "foo/bar")
            "#,
        )
        .expect("slashed-edit");

    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);
    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("failed: "),
        "slashed name must be rejected; got: {outcome}"
    );
    assert!(
        outcome.contains("'/'") || outcome.contains("directory separator"),
        "error must mention slash / directory separator: {outcome}"
    );
    // Disk untouched: original file still there, no foo/ subdir created.
    assert!(dir.path().join("file.txt").exists(), "file.txt must remain");
    assert!(!dir.path().join("foo").exists(), "foo/ must not be created");
    assert!(
        !dir.path().join("foo").join("bar").exists(),
        "foo/bar must not be created"
    );
}

// ---------------------------------------------------------------------------
// Navigation rejected while editing
// ---------------------------------------------------------------------------

#[test]
fn wdired_navigation_commands_blocked_while_editing() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    for cmd in [
        "pmacs-dired.parent",
        "pmacs-dired.sort-name",
        "pmacs-dired.sort-mtime",
        "pmacs-dired.sort-size",
    ] {
        let probe = format!(
            r#"
                local ok, err = pcall(function() pmacs.command.invoke("{cmd}") end)
                assert(not ok, "{cmd} must be rejected while editing")
                return tostring(err)
            "#
        );
        let msg: String = state
            .lua_host
            .lua()
            .load(&probe)
            .eval()
            .unwrap_or_else(|e| panic!("nav-block eval failed for {cmd}: {e}"));
        assert!(
            msg.contains("wdired-edit") && msg.contains("commit") && msg.contains("abandon"),
            "rejection must point at commit/abandon; got from {cmd}: {msg}"
        );
    }
}

// ---------------------------------------------------------------------------
// wdired-abandon restores the buffer to its pre-edit text
// ---------------------------------------------------------------------------

#[test]
fn wdired_abandon_restores_pre_edit_buffer_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());

    let before = active_buffer_text(&mut state);

    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");
    state
        .lua_host
        .eval(
            Some("scribble"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local ls = h.edit.line_start_marks[1]:get()
                h.buf:replace(ls + NAME_START, h.buf:len(), "scribbled.txt")
            "#,
        )
        .expect("scribble");
    state
        .lua_host
        .eval(
            Some("abandon"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-abandon")"#,
        )
        .expect("abandon");

    let after = active_buffer_text(&mut state);
    assert_eq!(
        before, after,
        "wdired-abandon must restore the pre-edit text byte-for-byte"
    );
    // File on disk untouched.
    assert!(
        dir.path().join("file.txt").exists(),
        "file.txt must be intact after abandon"
    );
    assert!(
        !dir.path().join("scribbled.txt").exists(),
        "no rename should have happened"
    );
}

// ---------------------------------------------------------------------------
// wdired-edit twice errors instead of double-attaching
// ---------------------------------------------------------------------------

#[test]
fn wdired_edit_twice_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("first wdired-edit");
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local ok, err = pcall(function()
                    pmacs.command.invoke("pmacs-dired.wdired-edit")
                end)
                assert(not ok, "second wdired-edit must error")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("second-edit eval");
    assert!(
        msg.contains("already in wdired-edit mode"),
        "second wdired-edit error must say already in mode: {msg}"
    );
}

// ---------------------------------------------------------------------------
// parse_perm_string: positional validation (lib-style test through seam)
// ---------------------------------------------------------------------------

#[test]
fn wdired_parse_perm_string_positional_validation() {
    let (state, _c, _u) = editor_with_dired();
    let lua = state.lua_host.lua();
    // Valid: standard 0o755.
    let mode: i64 = lua
        .load(
            r#"
                local m, _ = require("pmacs-dired")._test.parse_perm_string("rwxr-xr-x")
                return m
            "#,
        )
        .eval()
        .expect("valid 755");
    assert_eq!(mode, 0o755);

    // Invalid: char in wrong position.
    let invalid: String = lua
        .load(
            r#"
                local m, err = require("pmacs-dired")._test.parse_perm_string("xrwr-xr-x")
                if m then return "valid" else return tostring(err) end
            "#,
        )
        .eval()
        .expect("xr in pos 0");
    assert!(
        invalid.contains("invalid perms char"),
        "wrong-position char must surface: {invalid}"
    );

    // Wrong length.
    let wrong_len: String = lua
        .load(
            r#"
                local m, err = require("pmacs-dired")._test.parse_perm_string("rwxr-xr--")
                if m == nil then return tostring(err) else return tostring(m) end
            "#,
        )
        .eval()
        .expect("9-char with - in last pos is valid actually");
    // "rwxr-xr--" is 0o744 (owner=7, group=4, other=4), valid.
    let _ = wrong_len;

    // Bad length (8 chars).
    let too_short: String = lua
        .load(
            r#"
                local m, err = require("pmacs-dired")._test.parse_perm_string("rwxr-xr-")
                if m then return "ok" else return tostring(err) end
            "#,
        )
        .eval()
        .expect("8 chars");
    assert!(
        too_short.contains("9 chars"),
        "must complain length: {too_short}"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 Finding High1 --- escaped names round-trip without spurious renames
// ---------------------------------------------------------------------------
//
// A file whose real name contains a `\n` byte renders as `weird\nname.txt`
// (escape_displayable handles control chars and the backslash itself).
// Before the unescape step, commit would parse the rendered text as a NEW
// name with a literal backslash + 'n', and fire a rename to that new name.
// After the fix, a no-op commit is genuinely no-op.

#[test]
fn wdired_newline_in_filename_round_trips_without_spurious_rename() {
    let dir = tempfile::tempdir().expect("tempdir");
    let weird_name = "weird\nname.txt";
    std::fs::write(dir.path().join(weird_name), b"x").expect("write weird");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // No edits. Just commit. Must succeed without renaming the file.
    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert_eq!(outcome, "ok", "no-op commit must succeed; got: {outcome}");

    // The file with a real newline in its name must still exist.
    assert!(
        dir.path().join(weird_name).exists(),
        "file with newline-in-name must still exist after no-op commit"
    );
    // The literal "weird\\nname.txt" (a name containing a backslash + 'n')
    // must NOT exist on disk --- that would mean we mistook the rendered
    // escape for the real name and fired a spurious rename.
    assert!(
        !dir.path().join("weird\\nname.txt").exists(),
        "escape-as-real-name file must not have been created"
    );
}

// Backslash-only name: tests the same round-trip path through a different
// escape (`\\`), and verifies unescape_displayable directly via the seam.
#[test]
fn wdired_unescape_displayable_round_trips_known_escapes() {
    let (state, _c, _u) = editor_with_dired();
    let lua = state.lua_host.lua();

    // Round-trips for each recognized escape.
    let result: String = lua
        .load(
            r#"
                local te = require("pmacs-dired")._test
                local cases = {
                    { input = "plain.txt",       expect = "plain.txt"        },
                    { input = "a\\nb",           expect = "a\nb"             },
                    { input = "a\\rb",           expect = "a\rb"             },
                    { input = "a\\tb",           expect = "a\tb"             },
                    { input = "a\\\\b",          expect = "a\\b"             },
                    { input = "x\\x00y",         expect = "x\0y"             },
                    { input = "z\\xFFw",         expect = "z\255w"           },
                }
                for i, c in ipairs(cases) do
                    local out, err = te.unescape_displayable(c.input)
                    if out ~= c.expect then
                        return ("case " .. i .. " input=" .. c.input ..
                                " got=" .. tostring(out) ..
                                " want=" .. tostring(c.expect) ..
                                " err=" .. tostring(err))
                    end
                end
                return "ok"
            "#,
        )
        .eval()
        .expect("unescape eval");
    assert_eq!(result, "ok", "all escape round-trips must succeed");

    // Bad escapes are rejected.
    for bad in ["bad\\q", "bad\\", "bad\\xZZ", "bad\\x1"] {
        let probe = format!(
            r#"
                local te = require("pmacs-dired")._test
                local out, err = te.unescape_displayable({bad:?})
                if out == nil then return tostring(err) end
                return "accepted unexpectedly"
            "#
        );
        let msg: String = lua
            .load(&probe)
            .eval()
            .unwrap_or_else(|e| panic!("bad-escape eval failed for {bad}: {e}"));
        assert!(
            !msg.starts_with("accepted"),
            "must reject bad escape '{bad}'; got: {msg}"
        );
    }
}

// ---------------------------------------------------------------------------
// Pass-2 Finding High2 --- duplicate final-name rejection (overwrite case)
// ---------------------------------------------------------------------------
//
// Renaming a -> b where b is unchanged in the same dired listing would have
// the kernel's rename() silently overwrite b. The commit planner must catch
// this before any syscall fires.

#[test]
fn wdired_duplicate_final_name_rejected_pre_syscall() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"A-content").expect("write a");
    std::fs::write(dir.path().join("b.txt"), b"B-content").expect("write b");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Sort is "name" by default, so a.txt is line 1 and b.txt is line 2.
    // Edit a.txt's name to "b.txt": collides with the unchanged b.txt.
    state
        .lua_host
        .eval(
            Some("collide"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local ls1 = h.edit.line_start_marks[1]:get()
                local ls2 = h.edit.line_start_marks[2]:get()
                local eol1 = ls2 - 1  -- '\n' between line 1 and line 2
                h.buf:replace(ls1 + NAME_START, eol1, "b.txt")
            "#,
        )
        .expect("collide-edit");

    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("failed: "),
        "collision must fail commit; got: {outcome}"
    );
    assert!(
        outcome.contains("collide") || outcome.contains("final name"),
        "error must describe collision: {outcome}"
    );
    // No syscall ran: both files still in their original state.
    assert!(dir.path().join("a.txt").exists(), "a.txt must remain");
    assert!(dir.path().join("b.txt").exists(), "b.txt must remain");
    let a_content = std::fs::read(dir.path().join("a.txt")).unwrap();
    assert_eq!(a_content, b"A-content", "a.txt content unchanged");
    let b_content = std::fs::read(dir.path().join("b.txt")).unwrap();
    assert_eq!(b_content, b"B-content", "b.txt content unchanged");
}

// Swap case: a -> b, b -> a. Final names are unique (no duplicate-final-name
// rejection), but a direct rename(a, b) would destroy b. The two-phase
// temp-name detour makes this safe.
#[test]
fn wdired_name_swap_via_two_phase_rename_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"A-content").expect("write a");
    std::fs::write(dir.path().join("b.txt"), b"B-content").expect("write b");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Same-length swaps. Replacing same-length keeps the gravity-right
    // line-start marks at their original byte positions.
    state
        .lua_host
        .eval(
            Some("swap"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local ls1 = h.edit.line_start_marks[1]:get()
                local ls2 = h.edit.line_start_marks[2]:get()
                local eol1 = ls2 - 1
                local eol2 = h.buf:len()
                -- a.txt (line 1) -> "b.txt"
                h.buf:replace(ls1 + NAME_START, eol1, "b.txt")
                -- b.txt (line 2) -> "a.txt"
                h.buf:replace(ls2 + NAME_START, eol2, "a.txt")
            "#,
        )
        .expect("swap-edit");

    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert_eq!(outcome, "ok", "swap commit must succeed; got: {outcome}");

    // Disk shows swapped names with original contents.
    let a_after = std::fs::read(dir.path().join("a.txt")).unwrap();
    let b_after = std::fs::read(dir.path().join("b.txt")).unwrap();
    assert_eq!(
        a_after, b"B-content",
        "a.txt now holds B's original content"
    );
    assert_eq!(
        b_after, b"A-content",
        "b.txt now holds A's original content"
    );

    // No leftover temp files.
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    for e in &entries {
        assert!(
            !e.contains(".pmacs-wdired-tmp-"),
            "no temp files must remain; got: {entries:?}"
        );
    }
    assert_eq!(
        entries.len(),
        2,
        "only 2 entries expected; got: {entries:?}"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 Finding High3 --- wdired-abandon refreshes from disk
// ---------------------------------------------------------------------------
//
// The wdired-commit error guidance points users at wdired-abandon to "refresh
// the listing from disk". Abandon must therefore actually re-read the
// directory, not just repaint from the cached snapshot.

#[test]
fn wdired_abandon_refreshes_listing_from_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("orig.txt"), b"x").expect("write");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Externally rename orig.txt -> renamed.txt while the user is in
    // wdired-edit mode --- the precise scenario abandon should resync.
    std::fs::rename(dir.path().join("orig.txt"), dir.path().join("renamed.txt"))
        .expect("external rename");

    // Abandon: schedules an async refresh. Pump until the buffer
    // shows the post-rename listing.
    state
        .lua_host
        .eval(
            Some("abandon"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-abandon")"#,
        )
        .expect("abandon");
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .load(
                r"
                    local buf = pmacs.window.buffer()
                    local text = buf:slice(0, buf:len())
                    return text:find('renamed.txt', 1, true) ~= nil
                ",
            )
            .eval::<bool>()
            .unwrap_or(false)
    });

    let text = active_buffer_text(&mut state);
    assert!(
        text.contains("renamed.txt"),
        "buffer must show post-disk listing after abandon; got: {text}"
    );
    assert!(
        !text.contains("orig.txt"),
        "buffer must not show pre-rename name after abandon; got: {text}"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 Finding Med4 --- detect_external_changes catches mode change
// ---------------------------------------------------------------------------
//
// The previous implementation compared only count + name set. An external
// chmod, truncate, or kind switch slipped through. This test pins the mode
// case; size / mtime / kind would all surface analogously.

#[test]
fn wdired_external_mode_change_at_commit_aborts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let f = dir.path().join("file.txt");
    std::fs::write(&f, b"x").expect("write");
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).expect("chmod 0o644");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // External chmod between snapshot and commit. No user edit; commit
    // is intent-empty but external-change detection must abort it.
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).expect("external chmod");

    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("failed: "),
        "external mode change must fail commit; got: {outcome}"
    );
    assert!(
        outcome.contains("mode change") && outcome.contains("file.txt"),
        "error must describe the external mode change on file.txt: {outcome}"
    );
    // File mode on disk reflects the external chmod, not the snapshot.
    let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "external chmod must remain on disk");
}

#[test]
fn wdired_external_same_size_same_second_rewrite_aborts() {
    for _attempt in 0..25 {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("file.txt");
        std::fs::write(&f, b"aa").expect("write initial");

        let (mut state, _c, _u) = editor_with_dired();
        open_dired_on(&mut state, &dir.path().display().to_string());
        state
            .lua_host
            .eval(
                Some("wd-edit"),
                r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
            )
            .expect("wdired-edit");

        let before = file_mtime_parts(&f);
        std::fs::write(&f, b"bb").expect("same-size rewrite");
        let after = file_mtime_parts(&f);
        if before.0 != after.0 || before.1 == after.1 {
            continue;
        }

        state
            .lua_host
            .eval(
                Some("commit"),
                r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
            )
            .expect("commit eval");
        pump_until_commit_done(&mut state);

        let outcome = read_commit_outcome(&mut state);
        assert!(
            outcome.starts_with("failed: "),
            "same-second rewrite must fail commit; got: {outcome}"
        );
        assert!(
            outcome.contains("mtime change") && outcome.contains("file.txt"),
            "error must describe external mtime change on file.txt: {outcome}"
        );
        return;
    }
    panic!("could not produce same-second rewrite with distinct nanoseconds");
}

// ---------------------------------------------------------------------------
// Pass-2 Finding Med5 --- inserts in perms rejected (length-changing)
// ---------------------------------------------------------------------------
//
// Inserting a char into the perms column would shift every subsequent column
// and break the fixed-width layout the commit parser relies on. The
// intercept must reject the insert at edit time, not let it through and
// fail later at parse time.

#[test]
fn wdired_perms_column_insert_rejected_at_intercept() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Insert a valid alphabet char ('r') into the middle of the perms
    // column. Bytes are valid; the rejection is for the length change.
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                local ok, err = pcall(function()
                    h.buf:insert(ls + 3, "r")
                end)
                assert(not ok, "insert in perms must be rejected")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("perms-insert eval");
    assert!(
        msg.contains("fixed-width") || msg.contains("name column"),
        "rejection must explain fixed-width / name-only inserts; got: {msg}"
    );

    // Same check for delete inside perms: must also be rejected.
    let del_msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ls = h.edit.line_start_marks[1]:get()
                local ok, err = pcall(function()
                    h.buf:replace(ls + 1, ls + 2, "")  -- delete one perms char
                end)
                assert(not ok, "delete in perms must be rejected")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("perms-delete eval");
    assert!(
        del_msg.contains("fixed-width") || del_msg.contains("preserve"),
        "rejection must explain fixed-width / length preservation; got: {del_msg}"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 Finding Med6 --- symlink target edits rejected at commit
// ---------------------------------------------------------------------------
//
// Symlink target editing is out of scope for v0.1 wdired. Previously the
// commit silently stripped the rendered " -> target" suffix and produced no
// op when the user edited only the target --- their edit was discarded with
// no signal. Now an edited target text different from the snapshot's
// rendering aborts the commit explicitly.

#[test]
fn wdired_symlink_target_edit_rejected_at_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("real.txt"), b"contents").expect("write target");
    std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link"))
        .expect("create symlink");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Find the symlink line and overwrite its name region with a
    // different target text but the same basename. The commit must
    // refuse: target edits are not supported in v0.1.
    state
        .lua_host
        .eval(
            Some("target-edit"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local symlink_idx
                for i, e in ipairs(h.edit.snapshot) do
                    if e.kind == "symlink" then symlink_idx = i; break end
                end
                assert(symlink_idx, "symlink must be in the snapshot")
                local ls = h.edit.line_start_marks[symlink_idx]:get()
                local line_end
                if symlink_idx < #h.edit.line_start_marks then
                    line_end = h.edit.line_start_marks[symlink_idx + 1]:get() - 1
                else
                    line_end = h.buf:len()
                end
                -- Replace the entire name region. The basename "link"
                -- is unchanged but the target portion now reads
                -- "elsewhere", which doesn't match the snapshot's
                -- escape_displayable(real.txt) -- different bytes.
                h.buf:replace(ls + NAME_START, line_end, "link -> elsewhere")
            "#,
        )
        .expect("target-edit");

    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("failed: "),
        "symlink target edit must fail commit; got: {outcome}"
    );
    assert!(
        outcome.contains("symlink target"),
        "error must say 'symlink target': {outcome}"
    );

    // Disk symlink unchanged.
    let target = std::fs::read_link(dir.path().join("link")).expect("readlink");
    assert_eq!(
        target,
        dir.path().join("real.txt"),
        "symlink target must not have been changed on disk"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 review High1 --- chmod on a symlink line is rejected at intercept
// ---------------------------------------------------------------------------
//
// Per src/fs.rs:349, `chmod(2)` follows symlinks: chmodding the displayed
// link line would silently mutate the *target* file's mode, while a refresh
// would then show the link's unchanged lstat perms (commonly 0o777). That
// asymmetry is too sharp an edge to leave editable in v0.1 wdired.

#[test]
fn wdired_symlink_perms_edit_rejected_at_intercept() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("real.txt"), b"contents").expect("write target");
    std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link"))
        .expect("create symlink");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Find the symlink line and try to replace one perms char. Same-length
    // replace + valid alphabet would normally pass --- the rejection here
    // is specifically because the line is a symlink.
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local symlink_idx
                for i, e in ipairs(h.edit.snapshot) do
                    if e.kind == "symlink" then symlink_idx = i; break end
                end
                assert(symlink_idx, "symlink must be in the snapshot")
                local ls = h.edit.line_start_marks[symlink_idx]:get()
                -- Position 1 is the first perms char of this line.
                local ok, err = pcall(function()
                    h.buf:replace(ls + 1, ls + 2, "r")
                end)
                assert(not ok, "perms replace on symlink line must be rejected")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("symlink-perms eval");
    assert!(
        msg.contains("symlink") && msg.contains("perms column"),
        "rejection must mention symlink + perms column: {msg}"
    );
    assert!(
        msg.contains("chmod follows symlinks"),
        "rejection must explain why (chmod follows symlinks): {msg}"
    );

    // Disk untouched: target file's mode and the link itself unchanged.
    let target_mode = std::fs::metadata(dir.path().join("real.txt"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let link_target =
        std::fs::read_link(dir.path().join("link")).expect("readlink after rejection");
    assert_eq!(
        link_target,
        dir.path().join("real.txt"),
        "symlink unchanged"
    );
    // We didn't set an explicit mode on real.txt, so we don't assert a
    // specific number; we just confirm the rejection path didn't fire a
    // chmod (the commit was never reached).
    let _ = target_mode;
}

// ---------------------------------------------------------------------------
// Pass-2 review High2 --- decoded names containing NUL are rejected
// ---------------------------------------------------------------------------
//
// unescape_displayable accepts \x00, but POSIX filenames cannot contain NUL.
// Without this check, a chmod might fire on the source path before the
// rename syscall fails at the kernel boundary --- a partial commit. Reject
// at the planner.

#[test]
fn wdired_commit_rejects_nul_in_decoded_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Replace name with literal "bad\x00name" --- escape_displayable would
    // *render* such a real name as "bad\\x00name", and the user typing
    // exactly that produces "bad<NUL>name" via unescape. Reject pre-syscall.
    state
        .lua_host
        .eval(
            Some("nul-name"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local NAME_START = require("pmacs-dired")._test.NAME_START
                local ls = h.edit.line_start_marks[1]:get()
                h.buf:replace(ls + NAME_START, h.buf:len(), "bad\\x00name")
            "#,
        )
        .expect("nul-name edit");

    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("failed: "),
        "NUL in decoded name must fail commit; got: {outcome}"
    );
    assert!(outcome.contains("NUL"), "error must mention NUL: {outcome}");
    // Original file unchanged.
    assert!(
        dir.path().join("file.txt").exists(),
        "file.txt must remain since commit was rejected"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 review Med3 --- symlink names containing literal " -> " round-trip
// ---------------------------------------------------------------------------
//
// A symlink whose basename literally contains " -> " renders as
// `<name with arrow> -> <target>`. The naive "split at first ' -> '" decode
// would mistake everything after the first arrow for the target text and
// flag a no-op commit as a target edit. Suffix-stripping fixes this.

#[test]
fn wdired_symlink_basename_with_arrow_substring_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("real.txt"), b"contents").expect("write target");
    // Create a symlink whose basename literally contains " -> ".
    let weird_link_name = "a -> b";
    std::os::unix::fs::symlink(
        dir.path().join("real.txt"),
        dir.path().join(weird_link_name),
    )
    .expect("create symlink with arrow in name");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // No-op commit. Must succeed, no rename, link unchanged.
    state
        .lua_host
        .eval(
            Some("commit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert_eq!(
        outcome, "ok",
        "no-op commit on weirdly-named symlink must succeed; got: {outcome}"
    );
    // Symlink with arrow-in-name still exists, still pointing at real.txt.
    let link_target = std::fs::read_link(dir.path().join(weird_link_name))
        .expect("link with arrow in name still readable");
    assert_eq!(
        link_target,
        dir.path().join("real.txt"),
        "symlink target unchanged"
    );
}

// ---------------------------------------------------------------------------
// Pass-2 review Med4 --- syscall success and refresh failure are distinct
// ---------------------------------------------------------------------------
//
// If all chmod / rename ops landed but the post-commit navigate_to fails
// (e.g., the parent dir vanished between the last syscall and the
// refresh), the disk changes are real --- reporting "failed:" would
// mislead the user. The new outcome shape is "applied; refresh failed: ...".
//
// We exercise the split by injecting a synthetic failure into
// `pmacs.fs.read_dir`'s *second* call within wdired-commit: the first
// is `detect_external_changes` (must pass to reach the syscall phase),
// the second is the post-commit `navigate_to` refresh (forced to fail).
// A no-op commit (nothing to chmod / rename) reaches the refresh
// without touching disk, so the test is deterministic --- no real
// syscalls race the hook.

#[test]
fn wdired_refresh_failure_after_apply_reports_applied_distinctly() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("only.txt"), b"x").expect("write only");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    // Hook pmacs.fs.read_dir: first call (external-change detection)
    // delegates to the real impl, second + later calls return a future
    // whose :await() raises a synthetic failure.
    state
        .lua_host
        .eval(
            Some("install-readdir-hook"),
            r#"
                local real = pmacs.fs.read_dir
                local calls = 0
                pmacs.fs.read_dir = function(path)
                    calls = calls + 1
                    if calls >= 2 then
                        return setmetatable({}, {
                            __index = {
                                await = function()
                                    error("synthetic refresh failure")
                                end,
                            },
                        })
                    end
                    return real(path)
                end
            "#,
        )
        .expect("hook install");

    // No-op commit. Validation + planning succeed (no chmod / rename
    // ops since nothing changed); detect_external_changes reads disk
    // (call #1, real). do_wdired_commit returns cleanly. The async
    // wrapper then calls navigate_to (call #2, forced failure) ---
    // outcome must be "applied; refresh failed: ...".
    state
        .lua_host
        .eval(
            Some("commit-noop"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("applied; refresh failed: "),
        "syscall success + refresh failure must use applied-prefix outcome; got: {outcome}"
    );
    assert!(
        outcome.contains("synthetic refresh failure"),
        "outcome must surface the underlying refresh error: {outcome}"
    );

    // Disk untouched (no-op commit). The original file is still there.
    assert!(
        dir.path().join("only.txt").exists(),
        "no-op commit must not have touched disk"
    );
}

#[test]
fn wdired_commit_rejects_second_invocation_while_pending() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("only.txt"), b"x").expect("write only");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local ok1, err1 = pcall(function()
                    pmacs.command.invoke("pmacs-dired.wdired-commit")
                end)
                assert(ok1, "first commit should schedule cleanly: " .. tostring(err1))
                local ok2, err2 = pcall(function()
                    pmacs.command.invoke("pmacs-dired.wdired-commit")
                end)
                assert(not ok2, "second commit while first is pending must be rejected")
                return tostring(err2)
            "#,
        )
        .eval()
        .expect("double commit eval");
    assert!(
        msg.contains("already in progress"),
        "second commit error must explain in-progress commit; got: {msg}"
    );

    pump_until_commit_done(&mut state);
    let outcome = read_commit_outcome(&mut state);
    assert_eq!(
        outcome, "ok",
        "first commit should still complete; got: {outcome}"
    );
}

#[test]
fn wdired_mid_batch_rename_failure_reports_partial_and_refreshes() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"a").expect("write a");
    std::fs::write(dir.path().join("b.txt"), b"b").expect("write b");

    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());
    state
        .lua_host
        .eval(
            Some("wd-edit"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-edit")"#,
        )
        .expect("wdired-edit");

    edit_wdired_names(&mut state, &[("a.txt", "aa.txt"), ("b.txt", "bb.txt")]);
    install_second_rename_failure_hook(&mut state);

    state
        .lua_host
        .eval(
            Some("commit-partial"),
            r#"pmacs.command.invoke("pmacs-dired.wdired-commit")"#,
        )
        .expect("commit eval");
    pump_until_commit_done(&mut state);

    let outcome = read_commit_outcome(&mut state);
    assert!(
        outcome.starts_with("partially applied: "),
        "mid-batch syscall failure must report partial application; got: {outcome}"
    );
    assert!(
        outcome.contains("synthetic mid-batch rename failure") && outcome.contains("refreshed"),
        "outcome must include original failure and refresh status; got: {outcome}"
    );

    assert!(
        !wdired_edit_active(&mut state),
        "partial application must leave wdired-edit mode"
    );

    assert!(
        !dir.path().join("a.txt").exists(),
        "first rename should have moved a.txt to a temp name before the injected failure"
    );
    assert!(
        dir.path().join("b.txt").exists(),
        "second rename should not have run the real syscall"
    );
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        entries.iter().any(|e| e.contains(".pmacs-wdired-tmp-")),
        "partial failure should leave the already-moved temp name visible; got: {entries:?}"
    );
}

// ---------------------------------------------------------------------------
// M8.4 audit finding 2 --- paint() must clear the painting flag on error
// ---------------------------------------------------------------------------
//
// The `painting` flag is a bypass-flag pattern (M6.9 finding 7 shape) that
// tells the package's wdired intercept to let paint()'s own writes pass
// through without re-validating each as a user edit. If `:replace` raises
// while the flag is set, the flag must still clear --- otherwise the next
// user edit would silently bypass the intercept and corrupt the buffer
// without validation. The audit fix wraps the buffer write in pcall.
//
// To exercise the failure path deterministically, attach a *second*
// intercept that always raises. The package's intercept (when present)
// honors the painting flag; a foreign intercept does not, so it fires
// during paint() and triggers the error path.

#[test]
fn dired_paint_clears_painting_flag_on_buffer_replace_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());

    // Sort once to confirm the baseline (no error, paint clean).
    state
        .lua_host
        .eval(
            Some("baseline-sort"),
            r#"pmacs.command.invoke("pmacs-dired.sort-name")"#,
        )
        .expect("baseline sort");

    // Attach a foreign intercept that always raises. This intercept
    // doesn't know about the package's `painting` flag, so it fires
    // even when paint() is in progress.
    state
        .lua_host
        .eval(
            Some("attach-failing-intercept"),
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                _G.M8_4_FOREIGN_INTERCEPT = pmacs.buffer.add_intercept(
                    h.buf,
                    function(op) error("synthetic intercept failure") end
                )
            "#,
        )
        .expect("attach intercept");

    // Trigger paint via a sort command. Paint flips painting=true,
    // calls :replace, the foreign intercept raises, the buffer write
    // errors. The package's pcall wrap should restore painting=false
    // before re-raising.
    let painting_state: bool = state
        .lua_host
        .lua()
        .load(
            r#"
                local h = require("pmacs-dired")._test.active_handle()
                local ok, err = pcall(function()
                    pmacs.command.invoke("pmacs-dired.sort-mtime")
                end)
                assert(not ok, "sort must propagate the synthetic failure")
                -- The flag must be cleared (false or nil --- both count).
                return h.painting == true
            "#,
        )
        .eval()
        .expect("paint-flag eval");
    assert!(
        !painting_state,
        "painting flag must clear after a paint() error; if it stays \
         set, subsequent user edits silently bypass the intercept"
    );

    // Detach the foreign intercept so the editor cleanup is clean.
    state
        .lua_host
        .eval(
            Some("detach-foreign-intercept"),
            "pmacs.buffer.remove_intercept(_G.M8_4_FOREIGN_INTERCEPT)",
        )
        .expect("detach intercept");
}

// Companion to the previous test, pinning the *eager-evaluation* case the
// initial pcall scope missed: Lua evaluates pcall's argument list before
// entering the protected scope, so `pcall(fn, a, b, buf:len())` would
// invoke `buf:len()` outside the pcall. If `:len()` raises (e.g., because
// the underlying buffer was removed via `pmacs.buffer.remove` between
// paint() entries), the painting flag would skip past its clear-step.
// The corrected fix wraps the *whole* op in a closure: `pcall(function()
// handle.buf:replace(0, handle.buf:len(), text) end)`. This test forces a
// `:len()` failure by stubbing `handle.buf` with a Lua table that errors
// on :len; the flag must still clear.
#[test]
fn dired_paint_clears_painting_flag_on_len_error_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());

    // Stub handle.buf so :len() raises before :replace is reached.
    // We invoke paint() directly via the test seam (not through a
    // command) to avoid `active_handle()`'s identity check, which
    // matches handles by `h.buf == active_buffer` --- swapping
    // h.buf for a stub table would break that lookup and the
    // command would short-circuit before reaching paint().
    // We restore h.buf before the assertions so editor teardown is clean.
    let painting_state: bool = state
        .lua_host
        .lua()
        .load(
            r#"
                local te = require("pmacs-dired")._test
                local h = te.active_handle()
                local original_buf = h.buf
                h.buf = setmetatable({}, {
                    __index = function(_, k)
                        if k == "len" then
                            return function() error("synthetic len failure") end
                        end
                        if k == "replace" then
                            return function() error("should not reach replace") end
                        end
                        return original_buf[k]
                    end,
                })
                local ok, err = pcall(te.paint, h)
                local stuck = (h.painting == true)
                h.buf = original_buf
                assert(not ok, "paint must propagate the synthetic len failure")
                return stuck
            "#,
        )
        .eval()
        .expect("paint-flag-on-len-error eval");
    assert!(
        !painting_state,
        "painting flag must clear even when :len() fails before :replace; \
         eager-evaluation of pcall arguments would otherwise skip the clear-step"
    );
}

#[test]
fn dired_active_handle_is_cleared_when_active_buffer_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("file.txt"), b"x").expect("write");
    let (mut state, _c, _u) = editor_with_dired();
    open_dired_on(&mut state, &dir.path().display().to_string());

    let active_cleared: bool = state
        .lua_host
        .lua()
        .load(
            r#"
                local te = require("pmacs-dired")._test
                local h = te.active_handle()
                assert(h ~= nil, "dired handle should exist before remove")
                pmacs.buffer.remove(h.buf)
                return te.active_handle() == nil
            "#,
        )
        .eval()
        .expect("active-handle-after-remove eval");
    assert!(
        active_cleared,
        "active_handle must not return a stale handle after its buffer is removed"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
