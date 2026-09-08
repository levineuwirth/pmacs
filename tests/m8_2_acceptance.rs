// tests/m8_2_acceptance.rs --- T M8.2 dired-class acceptance.

//! Acceptance for the M8.2 deliverable: a Lua-only directory-view
//! package, installed via `install_local`, exercising the dev-loop
//! infrastructure shipped in M8.1 end-to-end.
//!
//! The package source-of-record for this test lives in-tree at
//! `tests/fixtures/pmacs-dired/`. A development copy also exists
//! alongside this repo at `~/Repos/util/pmacs-dired/`, but the
//! fixture is canonical: the test must remain reproducible from a
//! fresh clone of *this* repo with no sibling checkouts.
//!
//! The fixture is structurally external (separate manifest, loaded
//! via `install_local`); only its on-disk location is internal.
//! That preserves the M8 acceptance claim that dired-class fits
//! the buffer-and-views universality story without core support.
//!
//! Spec acceptance bullets being pinned:
//!
//! * Opening a directory of 10000 entries renders within 200 ms.
//! * Subdirectories navigate with Enter; parent with Backspace.
//! * Sort modes switch via commands, regenerate the buffer
//!   deterministically.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use pmacs::editor::EditorState;
use pmacs::frontend::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pmacs::lua_bindings::PackageInstallOverride;
use pmacs::protocol::FrontendId;
use tempfile::TempDir;

/// Construct a synthetic keyboard event for tests that exercise
/// the editor's key-dispatch path. Mirrors the helper inside
/// `editor.rs`'s test module.
fn plain_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

/// Resolve the on-disk path to the M8.2 dired package source.
/// Points at the in-tree fixture so the test is reproducible from
/// a fresh clone with no sibling checkouts.
fn dired_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests").join("fixtures").join("pmacs-dired")
}

/// Pump the editor's async tick until `predicate` returns true or
/// the deadline fires. Mirrors the helper in `m4`/`m8_1` — the
/// dired package's `open()` flow goes through `pmacs.async(...)`
/// → `pmacs.fs.read_dir(...):await()`, which only settles on a
/// `tick_async()` cycle.
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

/// Spin up an editor with package install overrides pointing at a
/// fresh temp dir. The dired package is then `install_local`-ed
/// from its sibling source path.
///
/// `EditorState::new()` sets the init-complete flag during startup
/// (the integration-test build doesn't get the `cfg(test)` guard
/// that lib tests do), so we reopen the init phase before
/// `install_local`. The `reopen_init_phase_for_testing` helper is
/// the documented test escape hatch.
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

/// Read the active buffer's full text.
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
// T M8.2 --- header + per-entry shape
// ---------------------------------------------------------------------------

#[test]
fn dired_open_renders_header_and_one_line_per_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"hello").expect("write a");
    std::fs::write(dir.path().join("b.txt"), b"world!").expect("write b");
    std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();

    let open = format!(
        r#"
        _G.M8_2_DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{path_str}")
            _G.M8_2_DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open"), &open)
        .unwrap_or_else(|e| panic!("dired.open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("M8_2_DONE")
            .unwrap_or(false)
    });

    let text = active_buffer_text(&mut state);
    let lines: Vec<&str> = text.lines().collect();
    // 1 header + 3 entries.
    assert_eq!(
        lines.len(),
        4,
        "expected 1 header + 3 entries, got {} lines: {text}",
        lines.len()
    );

    // Header is the path with trailing colon.
    assert_eq!(lines[0], format!("{path_str}:"));

    // Each entry line starts with a kind char then 9 perm chars
    // then a space. Subdir lines start with 'd', files with '-'.
    let mut saw_subdir = false;
    let mut saw_a = false;
    let mut saw_b = false;
    for line in &lines[1..] {
        assert!(line.len() > 11, "entry line too short: {line:?}");
        if line.contains(" subdir") {
            assert!(
                line.starts_with('d'),
                "subdir line should start with 'd': {line}"
            );
            saw_subdir = true;
        }
        if line.contains(" a.txt") {
            assert!(
                line.starts_with('-'),
                "a.txt line should start with '-': {line}"
            );
            assert!(line.contains(" 5 "), "size 5 should appear: {line}");
            saw_a = true;
        }
        if line.contains(" b.txt") {
            assert!(
                line.starts_with('-'),
                "b.txt line should start with '-': {line}"
            );
            assert!(line.contains(" 6 "), "size 6 should appear: {line}");
            saw_b = true;
        }
    }
    assert!(
        saw_subdir && saw_a && saw_b,
        "all three entries must render"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- 10K entries render under 200 ms
// ---------------------------------------------------------------------------

/// Open ten thousand entries through the async path; asserts the
/// render and returns the wall clock from `open()` to settle.
fn open_10k_entries() -> Duration {
    // Build a directory of 10K small files. The fixture creation
    // itself isn't fast (10K syscalls), so we measure only the
    // dired open() call, not setup.
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..10_000 {
        std::fs::write(dir.path().join(format!("f{i:05}")), b"").expect("write fixture entry");
    }

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();

    let open = format!(
        r#"
        _G.M8_2_DONE = nil
        _G.M8_2_OPEN_START = nil
        _G.M8_2_OPEN_END = nil
        pmacs.async(function()
            _G.M8_2_OPEN_START = os.clock()
            require("pmacs-dired").open("{path_str}")
            _G.M8_2_OPEN_END = os.clock()
            _G.M8_2_DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open10k"), &open)
        .unwrap_or_else(|e| panic!("dired.open(10K) failed: {e}"));

    let outer_start = Instant::now();
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("M8_2_DONE")
            .unwrap_or(false)
    });
    let outer_elapsed = outer_start.elapsed();

    // The pump-loop measurement includes our 2ms sleeps between
    // tick_async cycles. The async-runtime budget for the
    // read_dir reply is bounded by the worker pool's dispatch
    // latency plus the actual lstat-loop cost. For the spec's
    // 200ms ceiling we use the wall-clock from open() to pump
    // settle, which the inner os.clock() reading also captures
    // for cross-checking.
    // Sanity: the buffer has 1 header + 10000 entry lines.
    let text = active_buffer_text(&mut state);
    let line_count = text.lines().count();
    assert_eq!(
        line_count, 10_001,
        "10K entries + 1 header = 10001 lines; got {line_count}"
    );
    outer_elapsed
}

#[test]
fn dired_open_renders_10k_entries() {
    let elapsed = open_10k_entries();
    eprintln!("dired.open rendered 10K entries in {elapsed:?}");
}

#[test]
#[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
fn dired_open_renders_10k_entries_under_200ms() {
    let outer_elapsed = open_10k_entries();
    assert!(
        outer_elapsed < Duration::from_millis(200),
        "M8.2 spec: 10K entries must render within 200ms; took {outer_elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- sort modes regenerate the buffer deterministically
// ---------------------------------------------------------------------------

#[test]
fn dired_sort_commands_regenerate_buffer_deterministically() {
    // Three files with deliberately disjoint name/size ordering so
    // each sort mode produces a distinct expected order. mtime is
    // hard to control deterministically without sleep; we rely on
    // the name + size cases.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a"), b"1234567890").expect("a"); // 10 bytes
    std::fs::write(dir.path().join("b"), b"x").expect("b"); // 1 byte
    std::fs::write(dir.path().join("c"), b"yy").expect("c"); // 2 bytes

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();

    // Open and verify default sort (by name).
    let open = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{path_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("sort-open"), &open)
        .unwrap_or_else(|e| panic!("open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    let by_name_order = entry_filenames_in_buffer(&mut state);
    assert_eq!(
        by_name_order,
        vec!["a", "b", "c"],
        "default sort should be by name ascending"
    );

    // Sort by size (largest first): a (10), c (2), b (1).
    state
        .lua_host
        .eval(
            Some("sort-size"),
            r#"pmacs.command.invoke("pmacs-dired.sort-size")"#,
        )
        .unwrap_or_else(|e| panic!("sort-size command failed: {e}"));
    let by_size_order = entry_filenames_in_buffer(&mut state);
    assert_eq!(
        by_size_order,
        vec!["a", "c", "b"],
        "sort-size should produce largest first"
    );

    // Re-sort by name and verify we're back to alphabetical.
    state
        .lua_host
        .eval(
            Some("sort-name"),
            r#"pmacs.command.invoke("pmacs-dired.sort-name")"#,
        )
        .unwrap_or_else(|e| panic!("sort-name command failed: {e}"));
    let by_name_again = entry_filenames_in_buffer(&mut state);
    assert_eq!(
        by_name_again,
        vec!["a", "b", "c"],
        "re-sort by name should be deterministic"
    );
}

/// Extract entry filenames in display order from the active dired
/// buffer. Skips the header (line 0) and pulls the trailing token
/// of each entry line (filename) for comparison.
fn entry_filenames_in_buffer(state: &mut EditorState) -> Vec<String> {
    let text = active_buffer_text(state);
    text.lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().last().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------
// T M8.2 --- parent-directory navigation rebuilds the listing
// ---------------------------------------------------------------------------

#[test]
fn dired_parent_command_navigates_up_one_level() {
    // Open dired on a subdirectory, run the parent command, verify
    // the buffer now lists the parent's contents.
    let root = tempfile::tempdir().expect("tempdir");
    let child = root.path().join("child");
    std::fs::create_dir(&child).expect("mkdir child");
    std::fs::write(child.join("inner.txt"), b"i").expect("write inner");
    std::fs::write(root.path().join("sibling.txt"), b"s").expect("write sibling");

    let (mut state, _c, _u) = editor_with_dired();
    let child_str = child.display().to_string();

    let open = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{child_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-child"), &open)
        .unwrap_or_else(|e| panic!("open child failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    // Sanity: buffer shows child's contents.
    assert!(
        active_buffer_text(&mut state).contains("inner.txt"),
        "child's listing should contain inner.txt"
    );

    // Run the parent command. It dispatches another async work,
    // so we pump again.
    state
        .lua_host
        .eval(
            Some("parent"),
            r#"
                _G.DONE2 = nil
                pmacs.command.invoke("pmacs-dired.parent")
                pmacs.async(function()
                    -- Yield a tick so the parent-command's nested
                    -- pmacs.async has time to settle. We rely on
                    -- pmacs.workers.sleep as the canonical "yield
                    -- one tick" helper.
                    pmacs.workers.sleep(20):await()
                    _G.DONE2 = true
                end)
            "#,
        )
        .unwrap_or_else(|e| panic!("parent command failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE2")
            .unwrap_or(false)
    });

    let after = active_buffer_text(&mut state);
    let root_str = root.path().display().to_string();
    assert!(
        after.starts_with(&format!("{root_str}:")),
        "after parent, header should be the root path; got: {}",
        after.lines().next().unwrap_or("")
    );
    assert!(
        after.contains("sibling.txt"),
        "parent listing should include sibling.txt"
    );
    assert!(
        after.contains(" child"),
        "parent listing should include the original child as an entry"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- Enter on a subdirectory navigates into it
// ---------------------------------------------------------------------------
//
// Pins the spec bullet "Subdirectories navigate with Enter". The
// header advertises this acceptance, and the open-line command
// already implemented it; this test exercises the path end-to-end
// (cursor positioning + command invocation + buffer rebuild).

#[test]
fn dired_open_line_navigates_into_subdirectory_under_cursor() {
    // Parent has exactly one entry --- the subdirectory --- so we
    // know cursor line 1 is the subdir without depending on sort
    // semantics or file listing order.
    let root = tempfile::tempdir().expect("tempdir");
    let only = root.path().join("only_subdir");
    std::fs::create_dir(&only).expect("mkdir only_subdir");
    std::fs::write(only.join("inside.txt"), b"present").expect("write inside");

    let (mut state, _c, _u) = editor_with_dired();
    let root_str = root.path().display().to_string();

    // Open dired on the parent.
    let open = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{root_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-parent"), &open)
        .unwrap_or_else(|e| panic!("open parent failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    // Sanity: parent listing has only_subdir on line 1 (line 0 is
    // the header).
    let before = active_buffer_text(&mut state);
    let lines: Vec<&str> = before.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "parent should have 1 header + 1 entry; got {}: {before}",
        lines.len()
    );
    assert!(
        lines[1].contains("only_subdir"),
        "entry line 1 must be only_subdir: {}",
        lines[1]
    );

    // Position the cursor on entry line 1, then invoke open-line.
    // The pmacs.editor primitives are `move_line_start` + a loop of
    // `move_down`. We move to top via repeated move_up, then exactly
    // one move_down to land on line 1.
    state
        .lua_host
        .eval(
            Some("enter-on-subdir"),
            r#"
                while pmacs.editor.cursor_line() > 0 do
                    pmacs.editor.move_up()
                end
                pmacs.editor.move_line_start()
                pmacs.editor.move_down()
                _G.DONE2 = nil
                pmacs.command.invoke("pmacs-dired.open-line")
                pmacs.async(function()
                    -- The open-line dir branch dispatches another
                    -- pmacs.async; yield once so its read_dir reply
                    -- settles before we read the buffer.
                    pmacs.workers.sleep(20):await()
                    _G.DONE2 = true
                end)
            "#,
        )
        .unwrap_or_else(|e| panic!("enter-on-subdir failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE2")
            .unwrap_or(false)
    });

    // Buffer must now show the subdirectory's listing.
    let after = active_buffer_text(&mut state);
    let only_str = only.display().to_string();
    assert!(
        after.starts_with(&format!("{only_str}:")),
        "after Enter on subdir, header should be the subdir path; got: {}",
        after.lines().next().unwrap_or("")
    );
    assert!(
        after.contains("inside.txt"),
        "subdir listing should include inside.txt; got: {after}"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- Enter on a file errors with a clear message (stub posture)
// ---------------------------------------------------------------------------
//
// The v0.1 dired package can't open files directly --- there's no
// buffer-from-file primitive yet --- so the open-line command for
// non-dir entries must error explicitly rather than silently no-op.
// Pins the stub-posture contract: error names the workaround.

#[test]
fn dired_open_line_on_file_errors_with_actionable_message() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a_file.txt"), b"content").expect("write file");

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();

    let open = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{path_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-files"), &open)
        .unwrap_or_else(|e| panic!("open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    // Position the cursor on entry line 1 and invoke. open-line
    // should raise (caught via pcall here so we can inspect the
    // message).
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                while pmacs.editor.cursor_line() > 0 do
                    pmacs.editor.move_up()
                end
                pmacs.editor.move_line_start()
                pmacs.editor.move_down()
                local ok, err = pcall(function()
                    pmacs.command.invoke("pmacs-dired.open-line")
                end)
                assert(not ok, "open-line on file should have errored")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("file-branch error eval");
    assert!(
        msg.contains("buffer-from-file"),
        "error must name the missing capability: {msg}"
    );
    assert!(
        msg.contains("C-x C-f") || msg.contains("file-open"),
        "error must point at a workaround: {msg}"
    );
    assert!(
        msg.contains("a_file.txt"),
        "error must include the offending path: {msg}"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- relative paths to dired.open are rejected up front
// ---------------------------------------------------------------------------

#[test]
fn dired_open_rejects_relative_paths() {
    let (state, _c, _u) = editor_with_dired();
    let msg: String = state
        .lua_host
        .lua()
        .load(
            r#"
                local ok, err = pcall(function()
                    require("pmacs-dired").open("./relative")
                end)
                assert(not ok, "relative path should reject")
                return tostring(err)
            "#,
        )
        .eval()
        .expect("relative-path eval");
    assert!(
        msg.contains("absolute"),
        "error must call out the absolute requirement: {msg}"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- newlines in filenames don't break the line-per-entry invariant
// ---------------------------------------------------------------------------
//
// POSIX filenames may contain `\n` and `\r`. If render_entry passed
// such names through verbatim the buffer's one-line-per-entry
// contract --- which the wdired layer (M8.3) and cursor-line ->
// entry mapping both depend on --- would silently break. Verifies
// that `escape_displayable` keeps each entry on a single line.

#[test]
fn dired_escapes_newlines_in_filenames() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Create a file with a literal newline in its name. POSIX
    // permits this; ext4 happily accepts it.
    let nasty_name = "evil\nname.txt";
    std::fs::write(dir.path().join(nasty_name), b"x").expect("write nasty");
    std::fs::write(dir.path().join("normal.txt"), b"y").expect("write normal");

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();

    let open = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{path_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-nasty"), &open)
        .unwrap_or_else(|e| panic!("open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    let text = active_buffer_text(&mut state);
    let line_count = text.lines().count();
    // 1 header + 2 entries = 3 lines. Without escaping, the nasty
    // entry would render as two physical lines, giving 4.
    assert_eq!(
        line_count, 3,
        "newline-in-name must not split into multiple buffer lines; got {line_count} lines:\n{text}"
    );
    assert!(
        text.contains("evil\\nname.txt"),
        "newline must render as the escaped sequence \\n: {text}"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- escape_displayable disambiguates literal `\` from real newlines
// ---------------------------------------------------------------------------
//
// Wdired (M8.3) will parse rendered entry text to detect line
// edits, so the rendering must be a one-to-one encoding: a
// filename containing the literal two-character sequence `\` `n`
// must not look identical to one containing an actual newline.
// We escape literal backslashes first, then the control chars,
// matching `ls --quoting=c`. This test exercises the test seam's
// `escape_displayable` directly because constructing a filename
// containing a literal backslash-n in a portable way and reading
// it back through pmacs.fs is brittle (some filesystems normalize
// surrogate pairs); the function-level test is the load-bearing
// guarantee.

#[test]
fn dired_escape_displayable_round_trip_is_unambiguous() {
    let (state, _c, _u) = editor_with_dired();
    let result: mlua::Table = state
        .lua_host
        .lua()
        .load(
            r#"
                local esc = require("pmacs-dired")._test.escape_displayable
                return {
                    -- Real newline -> two-char escape "\n"
                    real_newline = esc("a\nb"),
                    -- Literal "\n" (two chars) -> "\\n" (escaped)
                    literal_backslash_n = esc("a\\nb"),
                    -- Real CR
                    real_cr = esc("a\rb"),
                    -- Other C0 controls use visible hex escapes
                    c0_soh = esc("a" .. string.char(1) .. "b"),
                    c0_us = esc("a" .. string.char(31) .. "b"),
                    -- Literal "\\" -> "\\\\"
                    only_backslash = esc("\\"),
                    -- ASCII passes through
                    ascii_only = esc("plain.txt"),
                }
            "#,
        )
        .eval()
        .expect("escape eval");

    let real_newline: String = result.get("real_newline").unwrap();
    let literal_bs_n: String = result.get("literal_backslash_n").unwrap();
    let real_cr: String = result.get("real_cr").unwrap();
    let c0_soh: String = result.get("c0_soh").unwrap();
    let c0_us: String = result.get("c0_us").unwrap();
    let only_backslash: String = result.get("only_backslash").unwrap();
    let ascii: String = result.get("ascii_only").unwrap();

    // The whole point: the two encodings must differ.
    assert_ne!(
        real_newline, literal_bs_n,
        "real newline and literal \\n must encode distinctly; both -> {real_newline}"
    );
    assert_eq!(
        real_newline, "a\\nb",
        "real newline -> a\\nb; got {real_newline}"
    );
    assert_eq!(
        literal_bs_n, "a\\\\nb",
        "literal \\n -> a\\\\nb; got {literal_bs_n}"
    );
    assert_eq!(real_cr, "a\\rb");
    assert_eq!(c0_soh, "a\\x01b");
    assert_eq!(c0_us, "a\\x1Fb");
    assert_eq!(only_backslash, "\\\\");
    assert_eq!(ascii, "plain.txt");
}

// ---------------------------------------------------------------------------
// T M8.2 --- initial open is atomic on read failure
// ---------------------------------------------------------------------------
//
// `open(path)` must not create an empty dired buffer or switch windows
// before it knows `path` can be read. Navigation already has the
// read-then-commit guarantee; initial open needs the same property.

#[test]
fn dired_open_failure_leaves_editor_unchanged() {
    let root = tempfile::tempdir().expect("tempdir");
    let missing = root.path().join("__missing__");
    let missing_str = missing.display().to_string();

    let (mut state, _c, _u) = editor_with_dired();
    let before_text = active_buffer_text(&mut state);

    let open = format!(
        r#"
        _G.OPEN_FAIL_DONE = nil
        pmacs.async(function()
            local before_buf = pmacs.window.buffer()
            local before_count = #pmacs.buffer.list()
            local ok, err = pcall(function()
                require("pmacs-dired").open("{missing_str}")
            end)
            _G.OPEN_FAIL_OK = ok
            _G.OPEN_FAIL_ERR = tostring(err)
            _G.OPEN_FAIL_BUFFER_UNCHANGED = (pmacs.window.buffer() == before_buf)
            _G.OPEN_FAIL_COUNT_UNCHANGED = (#pmacs.buffer.list() == before_count)
            _G.OPEN_FAIL_DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-failure-atomic"), &open)
        .unwrap_or_else(|e| panic!("open failure eval failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("OPEN_FAIL_DONE")
            .unwrap_or(false)
    });

    let ok: bool = state
        .lua_host
        .lua()
        .globals()
        .get("OPEN_FAIL_OK")
        .expect("OPEN_FAIL_OK");
    assert!(!ok, "open on missing path must fail");
    let buffer_unchanged: bool = state
        .lua_host
        .lua()
        .globals()
        .get("OPEN_FAIL_BUFFER_UNCHANGED")
        .expect("OPEN_FAIL_BUFFER_UNCHANGED");
    assert!(
        buffer_unchanged,
        "active buffer must not change on failed open"
    );
    let count_unchanged: bool = state
        .lua_host
        .lua()
        .globals()
        .get("OPEN_FAIL_COUNT_UNCHANGED")
        .expect("OPEN_FAIL_COUNT_UNCHANGED");
    assert!(count_unchanged, "failed open must not create a buffer");
    assert_eq!(
        before_text,
        active_buffer_text(&mut state),
        "failed open must leave active buffer bytes unchanged"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- navigation is atomic on read failure (read-then-commit)
// ---------------------------------------------------------------------------
//
// If `pmacs.fs.read_dir(target):await()` fails for any reason
// (path doesn't exist, EACCES, etc.), the dired buffer must
// remain showing the previous listing rather than landing in a
// half-updated state where the header points at the new path but
// the body still shows the old entries. The wdired layer (M8.3)
// and the cursor-line -> entry mapping both rely on the
// header-and-body always agreeing.

#[test]
fn dired_navigation_failure_leaves_handle_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("alive.txt"), b"present").expect("write alive");

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();

    let open = format!(
        r#"
        _G.DONE = nil
        _G.D_HANDLE = nil
        pmacs.async(function()
            _G.D_HANDLE = require("pmacs-dired").open("{path_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-atomic"), &open)
        .unwrap_or_else(|e| panic!("open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    let before = active_buffer_text(&mut state);
    assert!(
        before.contains("alive.txt"),
        "pre-navigation buffer must show alive.txt; got: {before}"
    );

    // Try to navigate to a path that definitely doesn't exist.
    // Use a temp-tree subpath that hasn't been created --- read_dir
    // returns ENOENT, the await fails with the structured error,
    // and the test seam's navigate_to should leave handle.path /
    // .entries / the buffer unchanged.
    let bogus = format!("{path_str}/__no_such_dir__");
    let nav = format!(
        r#"
        _G.NAV_OK = nil
        _G.NAV_ERR = nil
        _G.NAV_DONE = nil
        local h = _G.D_HANDLE
        local path_before = h.path
        local entries_before = #h.entries
        local navigate_to = require("pmacs-dired")._test.navigate_to
        pmacs.async(function()
            local ok, err = pcall(navigate_to, h, "{bogus}")
            _G.NAV_OK = ok
            _G.NAV_ERR = tostring(err)
            -- After failure, handle must be unchanged.
            _G.NAV_PATH_UNCHANGED = (h.path == path_before)
            _G.NAV_ENTRIES_UNCHANGED = (#h.entries == entries_before)
            _G.NAV_DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("nav-atomic"), &nav)
        .unwrap_or_else(|e| panic!("nav eval failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("NAV_DONE")
            .unwrap_or(false)
    });

    let nav_ok: bool = state
        .lua_host
        .lua()
        .globals()
        .get("NAV_OK")
        .expect("NAV_OK");
    assert!(
        !nav_ok,
        "navigate_to to non-existent path must error (got success)"
    );
    let path_ok: bool = state
        .lua_host
        .lua()
        .globals()
        .get("NAV_PATH_UNCHANGED")
        .expect("NAV_PATH_UNCHANGED");
    assert!(
        path_ok,
        "handle.path must be unchanged after failed navigation"
    );
    let entries_ok: bool = state
        .lua_host
        .lua()
        .globals()
        .get("NAV_ENTRIES_UNCHANGED")
        .expect("NAV_ENTRIES_UNCHANGED");
    assert!(
        entries_ok,
        "handle.entries must be unchanged after failed navigation"
    );

    let after = active_buffer_text(&mut state);
    assert_eq!(
        before, after,
        "buffer must be byte-identical after a failed navigation"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- sort-mtime sorts newest-first
// ---------------------------------------------------------------------------
//
// The pmacs-dired.sort-mtime command was advertised but not
// directly tested in the first pass. Construct three files with
// controlled mtimes, invoke the command, verify the rendered
// order is newest-first.

#[test]
fn dired_sort_mtime_orders_newest_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Create files in a known order, then explicitly stamp their
    // mtimes to make the test independent of filesystem timestamp
    // resolution / ordering during creation.
    let oldest = dir.path().join("c_oldest");
    let middle = dir.path().join("a_middle");
    let newest = dir.path().join("b_newest");
    for p in [&oldest, &middle, &newest] {
        std::fs::write(p, b"x").expect("write");
    }
    let now = SystemTime::now();
    std::fs::File::options()
        .write(true)
        .open(&oldest)
        .unwrap()
        .set_modified(now - Duration::from_mins(2))
        .expect("set mtime oldest");
    std::fs::File::options()
        .write(true)
        .open(&middle)
        .unwrap()
        .set_modified(now - Duration::from_mins(1))
        .expect("set mtime middle");
    std::fs::File::options()
        .write(true)
        .open(&newest)
        .unwrap()
        .set_modified(now)
        .expect("set mtime newest");

    let (mut state, _c, _u) = editor_with_dired();
    let path_str = dir.path().display().to_string();

    let open = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{path_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("mtime-open"), &open)
        .unwrap_or_else(|e| panic!("open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    state
        .lua_host
        .eval(
            Some("sort-mtime"),
            r#"pmacs.command.invoke("pmacs-dired.sort-mtime")"#,
        )
        .unwrap_or_else(|e| panic!("sort-mtime failed: {e}"));
    let order = entry_filenames_in_buffer(&mut state);
    assert_eq!(
        order,
        vec!["b_newest", "a_middle", "c_oldest"],
        "sort-mtime must list newest first"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- RET / Backspace key dispatch invokes the bound commands
// ---------------------------------------------------------------------------
//
// The other navigation tests invoke commands directly via
// pmacs.command.invoke, which masks regressions in the actual
// keybinding path (M.open's pmacs.keymap.bind calls). This test
// dispatches synthetic key events through EditorState::dispatch_key,
// the same path the running editor uses.

#[test]
fn dired_ret_and_backspace_keybindings_navigate() {
    let root = tempfile::tempdir().expect("tempdir");
    let only = root.path().join("only_subdir");
    std::fs::create_dir(&only).expect("mkdir");
    std::fs::write(only.join("inner.txt"), b"in").expect("write inner");

    let (mut state, _c, _u) = editor_with_dired();
    let root_str = root.path().display().to_string();
    let only_str = only.display().to_string();

    let open = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{root_str}")
            _G.DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("open-keybind"), &open)
        .unwrap_or_else(|e| panic!("open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    // Position cursor on the entry line (line 1).
    state
        .lua_host
        .eval(
            Some("cursor-to-entry"),
            r"
                while pmacs.editor.cursor_line() > 0 do
                    pmacs.editor.move_up()
                end
                pmacs.editor.move_line_start()
                pmacs.editor.move_down()
            ",
        )
        .expect("cursor positioning");

    // Press RET. The keybinding routes to pmacs-dired.open-line,
    // which dispatches a pmacs.async navigation. Wait for the
    // header to flip to the subdir path.
    state.dispatch_key(FrontendId::LOCAL, plain_key(KeyCode::Enter));
    let target_header = format!("{only_str}:");
    pump_until(&mut state, |s| {
        let lua = s.lua_host.lua();
        match lua
            .load(
                r"
                    local buf = pmacs.window.buffer()
                    return buf:slice(0, buf:len())
                ",
            )
            .eval::<String>()
        {
            Ok(text) => text.starts_with(&target_header),
            Err(_) => false,
        }
    });
    assert!(
        active_buffer_text(&mut state).contains("inner.txt"),
        "after RET the subdir's inner.txt should be visible"
    );

    // Press Backspace. Should navigate back to the root via the
    // pmacs-dired.parent binding.
    state.dispatch_key(FrontendId::LOCAL, plain_key(KeyCode::Backspace));
    let root_header = format!("{root_str}:");
    pump_until(&mut state, |s| {
        let lua = s.lua_host.lua();
        match lua
            .load(
                r"
                    local buf = pmacs.window.buffer()
                    return buf:slice(0, buf:len())
                ",
            )
            .eval::<String>()
        {
            Ok(text) => text.starts_with(&root_header),
            Err(_) => false,
        }
    });
    assert!(
        active_buffer_text(&mut state).contains("only_subdir"),
        "after Backspace the root's only_subdir entry should be visible"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- pmacs-dired itself reloads post-init
// ---------------------------------------------------------------------------
//
// The package claims reload safety, and the runtime supports command
// unregistering from post-init unload hooks. Pin the actual fixture so
// a future package-local cleanup regression can't hide behind the
// generic M8.1d lifecycle tests.

#[test]
fn dired_package_reload_is_safe_after_init_complete() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("after_reload.txt"), b"ok").expect("write");
    let path_str = dir.path().display().to_string();

    let (mut state, _c, _u) = editor_with_dired();
    state.lua_host.set_init_complete();

    state
        .lua_host
        .eval(
            Some("reload-dired"),
            r#"
                pmacs.packages.reload("pmacs-dired")
                assert(pmacs.command.exists("pmacs-dired.open-line"))
                assert(pmacs.command.exists("pmacs-dired.parent"))
                assert(pmacs.command.exists("pmacs-dired.sort-name"))
                assert(pmacs.command.exists("pmacs-dired.sort-mtime"))
                assert(pmacs.command.exists("pmacs-dired.sort-size"))
            "#,
        )
        .unwrap_or_else(|e| panic!("pmacs-dired reload failed: {e}"));

    let open = format!(
        r#"
        _G.RELOAD_OPEN_DONE = nil
        pmacs.async(function()
            require("pmacs-dired").open("{path_str}")
            _G.RELOAD_OPEN_DONE = true
        end)
    "#
    );
    state
        .lua_host
        .eval(Some("reload-dired-open"), &open)
        .unwrap_or_else(|e| panic!("post-reload open failed: {e}"));
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("RELOAD_OPEN_DONE")
            .unwrap_or(false)
    });

    assert!(
        active_buffer_text(&mut state).contains("after_reload.txt"),
        "fresh dired buffer should still open after package reload"
    );
}

// ---------------------------------------------------------------------------
// T M8.2 --- pmacs-dired source size under 1500 lines
// ---------------------------------------------------------------------------

#[test]
fn dired_source_size_under_audit_ceiling() {
    let init_path = dired_package_path().join("init.lua");
    let src = std::fs::read_to_string(&init_path).expect("read init.lua");
    let lines = src.lines().count();
    assert!(
        lines < 1500,
        "M8.2 spec: dired source under 1500 lines; got {lines}"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
