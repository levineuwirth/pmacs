// tests/m8_1_acceptance.rs --- T M8.1 filesystem worker primitives.

//! Acceptance tests for the M8.1 deliverable: a worker-dispatched
//! `pmacs.fs.*` surface that packages can use without reaching into
//! the Rust core. The dired-class package (T M8.2) is the immediate
//! consumer; magit-class (T M8.6) and outline-class (T M8.10) add to
//! the case load.
//!
//! Each test drives a real `EditorState` so the dispatch goes through
//! the production wiring: `pmacs._async._dispatch_fs_*` raw bindings,
//! the worker pool, the bus reply path, and `tick_async`'s
//! settle-detection loop. The Lua-side coroutine + `:await()`
//! mechanics from M3 carry the result back to the test caller.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;

/// Drive `tick_async` until `predicate` returns `true` or the deadline
/// passes. Mirrors the helper in `tests/m4_acceptance.rs` so each
/// test file owns its pump (avoids cross-test private-helper
/// dependencies).
fn pump_until<F: Fn(&EditorState) -> bool>(state: &mut EditorState, predicate: F) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate(state) {
        assert!(
            Instant::now() < deadline,
            "async pump deadline exceeded after 2s"
        );
        state.tick_async();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Spin up a fresh editor with no file open. The caller drives the
/// fs API entirely from Lua chunks via `lua_host.eval`.
fn fresh_editor() -> EditorState {
    EditorState::new()
}

/// Run a Lua chunk to completion (no `:await`); returns the chunk's
/// return value as `T`. Convenience for chunks that don't dispatch
/// async work.
fn eval_sync<T: mlua::FromLuaMulti>(state: &mut EditorState, chunk: &str) -> T {
    state
        .lua_host
        .lua()
        .load(chunk)
        .eval::<T>()
        .expect("eval sync chunk")
}

// ---------------------------------------------------------------------------
// T M8.1 --- pmacs.fs.read_dir returns one entry per child with lstat
// ---------------------------------------------------------------------------

#[test]
fn read_dir_returns_one_entry_per_child_with_lstat_metadata() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.txt"), b"hello").expect("write");
    std::fs::write(dir.path().join("b.txt"), b"world!").expect("write");
    std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");

    let mut state = fresh_editor();
    let path_str = dir.path().display().to_string();

    // Kick off pmacs.async-spawned coroutine that calls
    // pmacs.fs.read_dir(...):await() and stashes the result on a
    // global so the test can inspect it after pumping.
    let chunk = format!(
        r#"
        _G.M8_1_RESULT = nil
        _G.M8_1_ERR = nil
        pmacs.async(function()
            local ok, value = pcall(function()
                return pmacs.fs.read_dir("{path_str}"):await()
            end)
            if ok then
                _G.M8_1_RESULT = value
            else
                _G.M8_1_ERR = tostring(value)
            end
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);

    pump_until(&mut state, |s| {
        let lua = s.lua_host.lua();
        lua.globals().get::<bool>("M8_1_RESULT").is_ok()
            && lua
                .globals()
                .get::<mlua::Value>("M8_1_RESULT")
                .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
            || lua
                .globals()
                .get::<mlua::Value>("M8_1_ERR")
                .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
    });

    // Pull the entries back into Rust as a Vec<(name, kind, size)>
    // for shape assertions.
    let probe = r#"
        if _G.M8_1_ERR then return { error = _G.M8_1_ERR } end
        local out = {}
        for i, e in ipairs(_G.M8_1_RESULT) do
            out[i] = {
                name = e.name,
                kind = e.kind,
                size = e.size,
                has_mtime = type(e.mtime) == "number",
                has_mtime_nsec = type(e.mtime_nsec) == "number",
                has_mode = type(e.mode) == "number",
                has_target = e.symlink_target ~= nil,
            }
        end
        return out
    "#;
    let entries: mlua::Table = state.lua_host.lua().load(probe).eval().expect("probe");
    if let Ok(err) = entries.get::<String>("error") {
        panic!("read_dir errored: {err}");
    }
    let len: usize = entries.len().map(|n| n as usize).unwrap_or(0);
    assert_eq!(len, 3, "expected 3 entries, got {len}");

    // Names are filesystem-iteration-ordered; gather and sort.
    let mut by_name: std::collections::BTreeMap<String, (String, i64)> =
        std::collections::BTreeMap::new();
    for i in 1..=len {
        let row: mlua::Table = entries.get(i).unwrap();
        let name: String = row.get("name").unwrap();
        let kind: String = row.get("kind").unwrap();
        let size: i64 = row.get("size").unwrap();
        let has_mtime: bool = row.get("has_mtime").unwrap();
        let has_mtime_nsec: bool = row.get("has_mtime_nsec").unwrap();
        assert!(has_mtime, "{name} must include numeric mtime seconds");
        assert!(
            has_mtime_nsec,
            "{name} must include numeric mtime nanoseconds"
        );
        by_name.insert(name, (kind, size));
    }
    let a = by_name.get("a.txt").expect("a.txt entry");
    assert_eq!(a.0, "file");
    assert_eq!(a.1, 5);
    let b = by_name.get("b.txt").expect("b.txt entry");
    assert_eq!(b.0, "file");
    assert_eq!(b.1, 6);
    let s = by_name.get("subdir").expect("subdir entry");
    assert_eq!(s.0, "dir");
}

// ---------------------------------------------------------------------------
// T M8.1 --- symlink target is its own field, not collapsed into name
// ---------------------------------------------------------------------------

#[test]
fn read_dir_reports_symlinks_with_separate_target_field() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("real.txt"), b"x").expect("write");
    symlink("real.txt", dir.path().join("link")).expect("symlink");

    let mut state = fresh_editor();
    let path_str = dir.path().display().to_string();

    let chunk = format!(
        r#"
        _G.RESULT = nil
        pmacs.async(function()
            _G.RESULT = pmacs.fs.read_dir("{path_str}"):await()
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<mlua::Value>("RESULT")
            .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
    });

    let probe = r#"
        local link, real
        for _, e in ipairs(_G.RESULT) do
            if e.name == "link" then link = e end
            if e.name == "real.txt" then real = e end
        end
        return {
            link_kind = link and link.kind,
            link_target = link and link.symlink_target,
            real_kind = real and real.kind,
            real_target = real and real.symlink_target,
        }
    "#;
    let row: mlua::Table = state.lua_host.lua().load(probe).eval().expect("probe");
    assert_eq!(
        row.get::<String>("link_kind").unwrap(),
        "symlink",
        "link entry must report kind=symlink"
    );
    assert_eq!(
        row.get::<String>("link_target").unwrap(),
        "real.txt",
        "link entry must carry the target via symlink_target"
    );
    assert_eq!(
        row.get::<String>("real_kind").unwrap(),
        "file",
        "real.txt is not a symlink"
    );
    assert!(
        matches!(
            row.get::<mlua::Value>("real_target").unwrap(),
            mlua::Value::Nil
        ),
        "non-symlinks must not carry symlink_target"
    );
}

// ---------------------------------------------------------------------------
// T M8.1 --- supersede semantics: second read_dir under the same key
// cancels the first
// ---------------------------------------------------------------------------

#[test]
fn read_dir_supersede_cancels_in_flight_predecessor() {
    // A directory with enough entries that the cancel-poll boundary
    // is visible. Realistically supersede latency is bounded by
    // worker dispatch + a single readdir loop iteration; we just
    // need the first call to be in flight when the second comes in.
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..256 {
        std::fs::write(dir.path().join(format!("f{i}")), b"").expect("write");
    }

    let mut state = fresh_editor();
    let path_str = dir.path().display().to_string();

    let chunk = format!(
        r#"
        _G.A_STATUS = nil
        _G.B_RESULT = nil
        local h_a = pmacs.fs.read_dir("{path_str}", {{ supersede = "dired" }})
        pmacs.async(function()
            local ok, err = pcall(function() return h_a:await() end)
            if ok then
                _G.A_STATUS = "ok"
            elseif type(err) == "table" and err.tag == "cancelled" then
                _G.A_STATUS = "cancelled"
            else
                _G.A_STATUS = "other:" .. tostring(err)
            end
        end)
        local h_b = pmacs.fs.read_dir("{path_str}", {{ supersede = "dired" }})
        pmacs.async(function()
            _G.B_RESULT = h_b:await()
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);

    pump_until(&mut state, |s| {
        let lua = s.lua_host.lua();
        lua.globals()
            .get::<mlua::Value>("A_STATUS")
            .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
            && lua
                .globals()
                .get::<mlua::Value>("B_RESULT")
                .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
    });

    let a_status: String = state.lua_host.lua().globals().get("A_STATUS").unwrap();
    assert_eq!(
        a_status, "cancelled",
        "first read_dir must be superseded; got {a_status}"
    );
    let b_len: i64 = state
        .lua_host
        .lua()
        .load("return #_G.B_RESULT")
        .eval()
        .unwrap();
    assert_eq!(b_len, 256, "second read_dir should return all 256 entries");
}

// ---------------------------------------------------------------------------
// T M8.1 --- read_dir on a missing path surfaces a structured error
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// T M8.1 --- stat returns metadata for a single path
// ---------------------------------------------------------------------------

#[test]
fn stat_returns_lstat_metadata_for_a_single_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("hello.txt");
    std::fs::write(&p, b"hello world").expect("write");

    let mut state = fresh_editor();
    let path_str = p.display().to_string();

    let chunk = format!(
        r#"
        _G.STAT = nil
        pmacs.async(function()
            _G.STAT = pmacs.fs.stat("{path_str}"):await()
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<mlua::Value>("STAT")
            .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
    });

    let probe = r"
        return {
            name = _G.STAT.name,
            kind = _G.STAT.kind,
            size = _G.STAT.size,
            has_target = _G.STAT.symlink_target ~= nil,
        }
    ";
    let row: mlua::Table = state.lua_host.lua().load(probe).eval().expect("probe");
    assert_eq!(row.get::<String>("name").unwrap(), "hello.txt");
    assert_eq!(row.get::<String>("kind").unwrap(), "file");
    assert_eq!(row.get::<i64>("size").unwrap(), 11);
    assert!(!row.get::<bool>("has_target").unwrap());
}

// ---------------------------------------------------------------------------
// T M8.1 --- rename moves a file on disk
// ---------------------------------------------------------------------------

#[test]
fn rename_moves_a_file_and_settles_with_unit_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let from = dir.path().join("a.txt");
    let to = dir.path().join("b.txt");
    std::fs::write(&from, b"x").expect("write");

    let mut state = fresh_editor();
    let from_s = from.display().to_string();
    let to_s = to.display().to_string();

    let chunk = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            pmacs.fs.rename("{from_s}", "{to_s}"):await()
            _G.DONE = true
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    assert!(!from.exists(), "source must be gone after rename");
    assert!(to.exists(), "target must exist after rename");
}

// ---------------------------------------------------------------------------
// T M8.1 --- chmod replaces permission bits
// ---------------------------------------------------------------------------

#[test]
fn chmod_replaces_permission_bits() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path().join("f.txt");
    std::fs::write(&p, b"x").expect("write");
    // Start at 0o644 so the test sees a real change.
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).expect("seed perms");

    let mut state = fresh_editor();
    let path_str = p.display().to_string();
    let chunk = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            pmacs.fs.chmod("{path_str}", 0x1a4):await()  -- 0o644
            pmacs.fs.chmod("{path_str}", 0x180):await()  -- 0o600
            _G.DONE = true
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    let bits = std::fs::metadata(&p).unwrap().permissions().mode() & 0o7777;
    assert_eq!(
        bits, 0o600,
        "expected 0o600 after second chmod; got 0o{bits:o}"
    );
}

// ---------------------------------------------------------------------------
// T M8.1 --- remove deletes both files and empty directories
// ---------------------------------------------------------------------------

#[test]
fn remove_deletes_files_and_empty_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let f = dir.path().join("f.txt");
    let d = dir.path().join("empty");
    std::fs::write(&f, b"x").expect("write");
    std::fs::create_dir(&d).expect("mkdir");

    let mut state = fresh_editor();
    let f_s = f.display().to_string();
    let d_s = d.display().to_string();
    let chunk = format!(
        r#"
        _G.DONE = nil
        pmacs.async(function()
            pmacs.fs.remove("{f_s}"):await()
            pmacs.fs.remove("{d_s}"):await()
            _G.DONE = true
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<bool>("DONE")
            .unwrap_or(false)
    });

    assert!(!f.exists(), "file removed");
    assert!(!d.exists(), "empty dir removed");
}

#[test]
fn remove_of_nonempty_directory_surfaces_failed_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let d = dir.path().join("populated");
    std::fs::create_dir(&d).expect("mkdir");
    std::fs::write(d.join("child"), b"x").expect("write child");

    let mut state = fresh_editor();
    let d_s = d.display().to_string();
    let chunk = format!(
        r#"
        _G.STATUS = nil
        _G.MSG = nil
        pmacs.async(function()
            local ok, err = pcall(function()
                return pmacs.fs.remove("{d_s}"):await()
            end)
            if ok then
                _G.STATUS = "ok"
            else
                _G.STATUS = (type(err) == "table" and err.tag) or "raw"
                _G.MSG = (type(err) == "table" and err.message) or tostring(err)
            end
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<mlua::Value>("STATUS")
            .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
    });

    let status: String = state.lua_host.lua().globals().get("STATUS").unwrap();
    assert_eq!(
        status, "failed",
        "non-empty dir remove must surface failed; got {status}"
    );
    let msg: String = state.lua_host.lua().globals().get("MSG").unwrap();
    assert!(msg.contains("populated"), "error must name the path: {msg}");
    assert!(d.exists(), "directory must still exist after failed remove");
}

#[test]
fn read_dir_on_missing_path_returns_failed_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing: PathBuf = dir.path().join("does-not-exist");

    let mut state = fresh_editor();
    let path_str = missing.display().to_string();

    let chunk = format!(
        r#"
        _G.STATUS = nil
        _G.MESSAGE = nil
        pmacs.async(function()
            local ok, err = pcall(function()
                return pmacs.fs.read_dir("{path_str}"):await()
            end)
            if ok then
                _G.STATUS = "ok"
            else
                if type(err) == "table" then
                    _G.STATUS = err.tag or "table"
                    _G.MESSAGE = err.message
                else
                    _G.STATUS = "raw"
                    _G.MESSAGE = tostring(err)
                end
            end
        end)
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);
    pump_until(&mut state, |s| {
        s.lua_host
            .lua()
            .globals()
            .get::<mlua::Value>("STATUS")
            .is_ok_and(|v| !matches!(v, mlua::Value::Nil))
    });

    let status: String = state.lua_host.lua().globals().get("STATUS").unwrap();
    assert_eq!(
        status, "failed",
        "missing-path read_dir must surface tag = 'failed'; got {status}"
    );
    let message: String = state.lua_host.lua().globals().get("MESSAGE").unwrap();
    assert!(
        message.contains("does-not-exist"),
        "error message must name the offending path; got {message}"
    );
}

#[test]
fn fs_watch_reports_file_change_and_can_cancel() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("watched.txt");
    std::fs::write(&path, b"one").expect("write initial");

    let mut state = fresh_editor();
    let path_str = path.display().to_string();
    let chunk = format!(
        r#"
        _G.WATCH_EVENTS = {{}}
        _G.WATCH = pmacs.fs.watch("{path_str}", function(event)
            WATCH_EVENTS[#WATCH_EVENTS + 1] = event.kind
        end, {{ interval_ms = 5 }})
    "#,
    );
    eval_sync::<()>(&mut state, &chunk);

    // The watcher establishes its baseline snapshot asynchronously (an
    // in-flight fs.stat). `pmacs._async.pending_count() == 1` cannot
    // distinguish that in-flight baseline read from the steady-state
    // poll sleep, so a single post-gate write can race the baseline,
    // be absorbed into it, and never produce an event (the flake this
    // replaces). Instead, rewrite the file with distinct-length
    // content on every pump iteration: whatever snapshot the watcher
    // captured as its baseline, a subsequent distinct write
    // necessarily differs from it (size differs, so it holds even on
    // coarse-mtime filesystems) and fires a "changed" event.
    let writes = std::cell::Cell::new(0u32);
    pump_until(&mut state, |s| {
        let n = writes.get() + 1;
        writes.set(n);
        std::fs::write(&path, "v".repeat(n as usize + 1)).expect("rewrite changed");
        let count: i64 = s
            .lua_host
            .lua()
            .load("return #WATCH_EVENTS")
            .eval()
            .expect("event count");
        count >= 1
    });

    let first: String = state
        .lua_host
        .lua()
        .load("return WATCH_EVENTS[1]")
        .eval()
        .expect("first watch event");
    assert_eq!(first, "changed");

    eval_sync::<()>(&mut state, "WATCH:cancel()");
    let cancelled: bool = state
        .lua_host
        .lua()
        .load("return WATCH:is_cancelled()")
        .eval()
        .expect("cancelled");
    assert!(cancelled, "watch handle must report cancellation");
}
