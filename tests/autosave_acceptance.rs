//! Autosave + crash-recovery acceptance (Arc 3 phase 3).
//!
//! Each test injects a private tempdir `StateDir` (integration tests link
//! the lib without `cfg(test)`), so nothing touches a developer's real
//! state dir. Sweeps are driven directly rather than through the timer,
//! so the 1-second interval floor never slows the suite.
//!
//! Framing: `docs/autosave-recovery-framing.md`.

use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fresh_state_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-autosave-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn editor(state_dir: &std::path::Path) -> EditorState {
    let s = EditorState::new();
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host
        .lua()
        .set_app_data(StateDir(state_dir.to_path_buf()));
    s
}

fn write_file(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.display().to_string()
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Force a sweep; returns how many buffers were written.
fn sweep(s: &EditorState) -> i64 {
    let (written, _blocked): (i64, i64) = eval(s, "return pmacs.autosave.sweep()");
    written
}

/// Force a sweep; returns `(written, blocked)`.
fn sweep2(s: &EditorState) -> (i64, i64) {
    eval(s, "return pmacs.autosave.sweep()")
}

fn recovered(s: &EditorState, path: &str) -> Vec<u8> {
    let b: mlua::String = eval(
        s,
        &format!("return pmacs.autosave._recover_bytes({path:?})"),
    );
    b.as_bytes().to_vec()
}

fn status(s: &EditorState, path: &str) -> String {
    eval(s, &format!("return pmacs.autosave._status({path:?})"))
}

/// Open a file and dirty it by `n` inserted bytes at the front.
fn open_and_dirty(s: &EditorState, path: &str, text: &str) {
    exec(
        s,
        &format!("pmacs.buffer.find_or_open({path:?}); pmacs.window.buffer():insert(0, {text:?})"),
    );
}

#[test]
fn sweep_writes_recovery_for_a_modified_file_buffer() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "a.txt", "on disk\n");
    open_and_dirty(&s, &f, "unsaved ");
    assert_eq!(sweep(&s), 1, "one modified file buffer written");
    assert_eq!(status(&s, &f), "fresh");

    // The recovery contents are the buffer's, not the file's.
    let bytes: mlua::String = eval(&s, &format!("return pmacs.autosave._recover_bytes({f:?})"));
    assert_eq!(&*bytes.as_bytes(), b"unsaved on disk\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn envelope_round_trips_non_utf8_contents() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "bin.dat", "");
    // 0xff is invalid UTF-8; the envelope reads bytes, not a String.
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({f:?}); pmacs.window.buffer():insert(0, '\\255\\n\\0a')"
        ),
    );
    assert_eq!(sweep(&s), 1);
    let bytes: mlua::String = eval(&s, &format!("return pmacs.autosave._recover_bytes({f:?})"));
    assert_eq!(&*bytes.as_bytes(), &[0xffu8, b'\n', 0x00, b'a'][..]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sweep_skips_clean_scratch_and_unchanged_buffers() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "a.txt", "hello\n");

    // A clean file buffer + the scratch buffer: nothing to write.
    exec(&s, &format!("pmacs.buffer.find_or_open({f:?})"));
    assert_eq!(sweep(&s), 0, "clean buffer and scratch are skipped");

    // Dirty it → one write. Sweeping again with no further edit → zero
    // (the (path_hash, revision) skip cache).
    exec(&s, "pmacs.window.buffer():insert(0, 'x')");
    assert_eq!(sweep(&s), 1);
    assert_eq!(sweep(&s), 0, "unchanged since last copy → no rewrite");

    // Another edit → written again.
    exec(&s, "pmacs.window.buffer():insert(0, 'y')");
    assert_eq!(sweep(&s), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn path_change_writes_new_key_and_discards_the_old() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let old = write_file(&dir, "old.txt", "body\n");
    let new = dir.join("new.txt").display().to_string();
    open_and_dirty(&s, &old, "dirty ");
    assert_eq!(sweep(&s), 1);
    assert_eq!(status(&s, &old), "fresh");

    // Rename WITHOUT editing the buffer — what an LSP WorkspaceEdit
    // rename does: the file moves on disk (preserving mtime/size) and the
    // buffer keeps its BufferId *and* its revision, only its path changes.
    std::fs::rename(&old, &new).unwrap();
    {
        let id = s.core.borrow().active_buffer_id();
        s.core
            .borrow_mut()
            .set_buffer_path(id, Some(PathBuf::from(&new)));
    }
    // A revision-only cache would skip this write, never create the
    // recovery under the new key, and orphan the old one.
    assert_eq!(sweep(&s), 1, "path change forces a rewrite");
    assert_eq!(status(&s, &new), "fresh", "new key written");
    assert_eq!(status(&s, &old), "none", "old key discarded");
    let bytes: mlua::String = eval(
        &s,
        &format!("return pmacs.autosave._recover_bytes({new:?})"),
    );
    assert_eq!(&*bytes.as_bytes(), b"dirty body\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn new_file_buffer_is_swept_with_null_origin_and_recovers() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    // A `[new file]`: a path with no file on disk, so no `file_meta`.
    // Lua's find_or_open *errors* on a missing path, so this is built the
    // way argv `pmacs draft.txt` does — an empty buffer with a path.
    let missing = dir.join("draft.txt");
    exec(
        &s,
        "_G.nb = pmacs.buffer.create('draft.txt'); pmacs.window.switch_buffer(_G.nb)",
    );
    {
        let id = s.core.borrow().active_buffer_id();
        s.core
            .borrow_mut()
            .set_buffer_path(id, Some(missing.clone()));
    }
    // Typing into it is what makes it modified (and worth recovering).
    exec(&s, "pmacs.window.buffer():insert(0, 'unsaved draft')");
    let p = missing.display().to_string();
    assert_eq!(sweep(&s), 1, "a new-file buffer is swept");
    // origin is null → fresh while the file is still absent.
    assert_eq!(status(&s, &p), "fresh");
    let bytes: mlua::String = eval(&s, &format!("return pmacs.autosave._recover_bytes({p:?})"));
    assert_eq!(&*bytes.as_bytes(), b"unsaved draft");

    // Someone creates the file meanwhile → stale, never auto-offered.
    std::fs::write(&missing, b"theirs").unwrap();
    assert_eq!(status(&s, &p), "stale");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn external_change_makes_recovery_stale() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "a.txt", "original\n");
    open_and_dirty(&s, &f, "mine ");
    assert_eq!(sweep(&s), 1);
    assert_eq!(status(&s, &f), "fresh");

    // Someone else edits the file on disk.
    std::fs::write(&f, b"theirs, quite different\n").unwrap();
    assert_eq!(status(&s, &f), "stale", "never auto-offered");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn corrupt_recovery_is_typed_quiet_and_discardable() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "a.txt", "body\n");
    exec(&s, &format!("pmacs.buffer.find_or_open({f:?})"));

    // Plant a malformed envelope under the right key.
    let key = pmacs::autosave::key_for(std::path::Path::new(&f));
    pmacs::state::write_private(&dir, &key, b"garbage without a newline").unwrap();
    assert_eq!(status(&s, &f), "corrupt");

    // The aggregate report must not error or announce it.
    let (fresh, corrupt): (Vec<String>, i64) = eval(&s, "return pmacs.autosave._pending()");
    assert!(fresh.is_empty(), "corrupt is never offered");
    assert_eq!(corrupt, 1, "counted separately");

    // And it is discardable.
    exec(&s, &format!("pmacs.autosave._discard({f:?})"));
    assert_eq!(status(&s, &f), "none");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pending_aggregates_and_names_a_single_file() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let mut paths = Vec::new();
    for i in 0..3 {
        let f = write_file(&dir, &format!("f{i}.txt"), "body\n");
        open_and_dirty(&s, &f, "x");
        paths.push(f);
    }
    assert_eq!(sweep(&s), 3);
    let (fresh, corrupt): (Vec<String>, i64) = eval(&s, "return pmacs.autosave._pending()");
    assert_eq!(fresh.len(), 3, "all three reported in ONE call");
    assert_eq!(corrupt, 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sweep_never_overwrites_unclaimed_crash_recovery() {
    let dir = fresh_state_dir();
    // Session 1 crashes with unsaved work: a recovery copy is on disk.
    let s1 = editor(&dir);
    let f = write_file(&dir, "a.txt", "on disk\n");
    open_and_dirty(&s1, &f, "CRASH WORK ");
    assert_eq!(sweep(&s1), 1);
    let crash_copy = recovered(&s1, &f);
    assert_eq!(&crash_copy, b"CRASH WORK on disk\n");

    // Session 2 reopens the file (on-disk contents) and edits it BEFORE
    // running recover-file. Sweeping must NOT clobber the crash copy.
    let s2 = editor(&dir);
    exec(&s2, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s2, "pmacs.window.buffer():insert(0, 'new edits ')");
    let (written, blocked) = sweep2(&s2);
    assert_eq!(written, 0, "must not write over unclaimed crash data");
    assert_eq!(blocked, 1, "the sweep is blocked and reported");
    assert_eq!(
        recovered(&s2, &f),
        crash_copy,
        "the crash recovery survives intact"
    );
    assert_eq!(status(&s2, &f), "fresh", "still offered to the user");

    // Once recover-file adopts it, autosave resumes for that path.
    exec(&s2, "pmacs.autosave._adopt(pmacs.window.buffer())");
    // Adopt records the copy at the buffer's *current* revision, so the
    // very next sweep sees no change; an edit makes it write again.
    exec(&s2, "pmacs.window.buffer():insert(0, 'more ')");
    let (written, blocked) = sweep2(&s2);
    assert_eq!((written, blocked), (1, 0), "adopted → sweeps again");
    assert_eq!(recovered(&s2, &f), b"more new edits on disk\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saving_without_recovering_preserves_unclaimed_crash_data() {
    let dir = fresh_state_dir();
    // Session 1 crashes with unsaved work.
    let s1 = editor(&dir);
    let f = write_file(&dir, "a.txt", "on disk\n");
    open_and_dirty(&s1, &f, "CRASH WORK ");
    assert_eq!(sweep(&s1), 1);
    let crash_copy = recovered(&s1, &f);

    // Session 2 reopens, edits, and SAVES — without ever recovering or
    // discarding. The save must not destroy the crash copy: only
    // recover-file (adopt) or discard-recovery may release it.
    let s2 = editor(&dir);
    exec(&s2, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s2, "pmacs.window.buffer():insert(0, 'new ')");
    exec(&s2, "pmacs.command.invoke('buffer.save')");
    assert_ne!(
        status(&s2, &f),
        "none",
        "saving must not delete unclaimed crash data"
    );
    assert_eq!(
        recovered(&s2, &f),
        crash_copy,
        "the crash recovery survives a save"
    );
    // It is now stale (the file changed on disk), so it is never
    // auto-offered — but it is still there to recover or discard.
    assert_eq!(status(&s2, &f), "stale");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn killing_without_recovering_preserves_unclaimed_crash_data() {
    let dir = fresh_state_dir();
    let s1 = editor(&dir);
    let f = write_file(&dir, "a.txt", "on disk\n");
    open_and_dirty(&s1, &f, "CRASH ");
    assert_eq!(sweep(&s1), 1);
    let crash_copy = recovered(&s1, &f);

    let s2 = editor(&dir);
    exec(&s2, &format!("_G.b = pmacs.buffer.find_or_open({f:?})"));
    exec(&s2, "pmacs.buffer.kill(_G.b)");
    assert_eq!(recovered(&s2, &f), crash_copy, "kill preserves it too");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn recover_then_kill_retires_the_adopted_recovery() {
    let dir = fresh_state_dir();
    let s1 = editor(&dir);
    let f = write_file(&dir, "a.txt", "on disk\n");
    open_and_dirty(&s1, &f, "crash ");
    assert_eq!(sweep(&s1), 1);

    // Reopen, recover, then kill immediately — before any save or sweep.
    // The removal callback fires after the buffer is gone, so the only
    // way to find the copy is the entry `_adopt` recorded for its id.
    let s2 = editor(&dir);
    exec(&s2, &format!("_G.b = pmacs.buffer.find_or_open({f:?})"));
    exec(
        &s2,
        &format!(
            "
            local bytes = pmacs.autosave._recover_bytes({f:?})
            local b = pmacs.window.buffer()
            b:replace(0, b:len(), bytes)
            pmacs.autosave._adopt(b)
            "
        ),
    );
    exec(&s2, "pmacs.buffer.kill(_G.b)");
    assert_eq!(
        status(&s2, &f),
        "none",
        "an adopted recovery is retired on kill, not left to be re-offered"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn discard_recovery_lets_the_next_sweep_reprotect_immediately() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "a.txt", "body\n");
    open_and_dirty(&s, &f, "mine ");
    assert_eq!(sweep(&s), 1);
    assert_eq!(sweep(&s), 0, "unchanged → skipped");

    // Explicitly discard while the buffer is still dirty. The next sweep
    // must re-create protection at once: a stale skip-cache entry would
    // leave the buffer unprotected until its next edit.
    exec(&s, &format!("pmacs.autosave._discard({f:?})"));
    assert_eq!(status(&s, &f), "none");
    assert_eq!(sweep(&s), 1, "protection restored without needing an edit");
    assert_eq!(recovered(&s, &f), b"mine body\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn discarding_an_unclaimed_recovery_unblocks_the_sweep() {
    let dir = fresh_state_dir();
    let s1 = editor(&dir);
    let f = write_file(&dir, "a.txt", "on disk\n");
    open_and_dirty(&s1, &f, "crash ");
    assert_eq!(sweep(&s1), 1);

    let s2 = editor(&dir);
    exec(&s2, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s2, "pmacs.window.buffer():insert(0, 'mine ')");
    assert_eq!(sweep2(&s2), (0, 1), "blocked");

    exec(&s2, &format!("pmacs.autosave._discard({f:?})"));
    assert_eq!(sweep2(&s2), (1, 0), "discarded → sweeps again");
    assert_eq!(recovered(&s2, &f), b"mine on disk\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn killing_a_new_file_buffer_gcs_its_recovery() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    // A `[new file]` fires no after-load, so no per-buffer removal
    // callback is registered — the sweep-time GC is the backstop.
    let missing = dir.join("draft.txt");
    exec(
        &s,
        "_G.nb = pmacs.buffer.create('draft.txt'); pmacs.window.switch_buffer(_G.nb)",
    );
    {
        let id = s.core.borrow().active_buffer_id();
        s.core
            .borrow_mut()
            .set_buffer_path(id, Some(missing.clone()));
    }
    exec(&s, "pmacs.window.buffer():insert(0, 'draft')");
    let p = missing.display().to_string();
    assert_eq!(sweep(&s), 1);
    assert_eq!(status(&s, &p), "fresh");

    exec(&s, "pmacs.buffer.kill(_G.nb)");
    // The next sweep GCs the dead buffer's recovery copy.
    sweep(&s);
    assert_eq!(
        status(&s, &p),
        "none",
        "killed new-file buffer is cleaned up"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saving_after_a_rename_removes_the_recovery_written_under_the_old_path() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let old = write_file(&dir, "old.txt", "body\n");
    let new = dir.join("new.txt").display().to_string();
    open_and_dirty(&s, &old, "dirty ");
    assert_eq!(sweep(&s), 1, "recovery written under the OLD key");

    // Rename, then save — without an intervening sweep. A path-captured
    // cleanup would delete the new key and leave the old one behind.
    std::fs::rename(&old, &new).unwrap();
    {
        let id = s.core.borrow().active_buffer_id();
        s.core
            .borrow_mut()
            .set_buffer_path(id, Some(PathBuf::from(&new)));
    }
    exec(&s, "pmacs.command.invoke('buffer.save')");
    assert_eq!(status(&s, &old), "none", "old key removed");
    assert_eq!(status(&s, &new), "none", "new key removed");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_pre_existing_lax_autosave_dir_is_tightened() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = fresh_state_dir();
        // Someone (an older pmacs, or the user) left autosave/ at 0755.
        let autosave_dir = dir.join("autosave");
        std::fs::create_dir_all(&autosave_dir).unwrap();
        std::fs::set_permissions(&autosave_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let s = editor(&dir);
        let f = write_file(&dir, "a.txt", "body\n");
        open_and_dirty(&s, &f, "secret ");
        assert_eq!(sweep(&s), 1);

        let dmode = std::fs::metadata(&autosave_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700, "a lax autosave dir is tightened, not left");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn tick_reports_recoveries_once_aggregated() {
    let dir = fresh_state_dir();
    // Seed three recovery copies, then "crash" and reopen the files.
    let s1 = editor(&dir);
    let mut paths = Vec::new();
    for i in 0..3 {
        let f = write_file(&dir, &format!("f{i}.txt"), "body\n");
        open_and_dirty(&s1, &f, "x");
        paths.push(f);
    }
    assert_eq!(sweep(&s1), 3);

    let s2 = editor(&dir);
    for f in &paths {
        exec(&s2, &format!("pmacs.buffer.find_or_open({f:?})"));
    }
    // Each `after-load` only raises a flag; the tick does the reporting,
    // so three loads collapse into ONE aggregate message.
    exec(&s2, "pmacs.hook.run('process.after-tick')");
    let status = s2.core.borrow().status.clone();
    assert!(
        status.contains("3 files have autosave recovery"),
        "one aggregated message, not three: {status:?}"
    );

    // A second tick does not re-report (the flag was cleared).
    s2.core.borrow_mut().status.clear();
    exec(&s2, "pmacs.hook.run('process.after-tick')");
    assert!(s2.core.borrow().status.is_empty(), "no repeat report");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn tick_names_the_file_when_exactly_one_is_recoverable() {
    let dir = fresh_state_dir();
    let s1 = editor(&dir);
    let f = write_file(&dir, "solo.txt", "body\n");
    open_and_dirty(&s1, &f, "x");
    assert_eq!(sweep(&s1), 1);

    let s2 = editor(&dir);
    exec(&s2, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s2, "pmacs.hook.run('process.after-tick')");
    let status = s2.core.borrow().status.clone();
    assert!(
        status.contains("solo.txt") && status.contains("recover-file"),
        "single recovery names the file: {status:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_and_kill_delete_the_recovery_copy() {
    let dir = fresh_state_dir();
    let s = editor(&dir);

    // Clean save deletes it (buffer.after-save).
    let f = write_file(&dir, "a.txt", "body\n");
    open_and_dirty(&s, &f, "x");
    assert_eq!(sweep(&s), 1);
    exec(&s, "pmacs.command.invoke('buffer.save')");
    assert_eq!(status(&s, &f), "none", "clean save retires the recovery");

    // Kill deletes it (per-buffer on_removed registered at after-load).
    let g = write_file(&dir, "b.txt", "body\n");
    exec(&s, &format!("_G.gb = pmacs.buffer.find_or_open({g:?})"));
    exec(&s, "pmacs.window.buffer():insert(0, 'x')");
    assert_eq!(sweep(&s), 1);
    assert_eq!(status(&s, &g), "fresh");
    exec(&s, "pmacs.buffer.kill(_G.gb)");
    assert_eq!(status(&s, &g), "none", "kill retires the recovery");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn recover_file_installs_contents_fires_after_edit_and_leaves_modified() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "a.txt", "on disk\n");
    open_and_dirty(&s, &f, "recovered ");
    assert_eq!(sweep(&s), 1);

    // Simulate the crash-then-reopen: a fresh editor over the same store,
    // opening the file whose on-disk contents are the OLD ones.
    let s2 = editor(&dir);
    exec(
        &s2,
        r#"
        _G.after_edit = 0
        pmacs.hook.add("buffer.after-edit", function() _G.after_edit = _G.after_edit + 1 end)
        "#,
    );
    exec(&s2, &format!("pmacs.buffer.find_or_open({f:?})"));
    assert_eq!(status(&s2, &f), "fresh");
    // The buffer still holds the on-disk contents (no silent substitution).
    let before: mlua::String = eval(
        &s2,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    assert_eq!(&*before.as_bytes(), b"on disk\n");

    // Drive recover-file's accept path directly (the command opens a
    // minibuffer; we exercise what its on_accept does).
    exec(
        &s2,
        &format!(
            r#"
            local bytes = pmacs.autosave._recover_bytes({f:?})
            local b = pmacs.window.buffer()
            b:replace(0, b:len(), bytes)
            pmacs.hook.run("buffer.after-edit")
            "#
        ),
    );
    let after: mlua::String = eval(
        &s2,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    assert_eq!(&*after.as_bytes(), b"recovered on disk\n");
    let fired: i64 = eval(&s2, "return _G.after_edit");
    assert!(
        fired >= 1,
        "after-edit fired so LSP/syntax see the recovery"
    );
    let modified: bool = eval(&s2, "return pmacs.window.buffer():is_modified()");
    assert!(
        modified,
        "recovered buffer is dirty; user must save to keep"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn recovery_files_are_private_0600_under_a_0700_dir() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = fresh_state_dir();
        let s = editor(&dir);
        let f = write_file(&dir, "secret.txt", "");
        open_and_dirty(&s, &f, "unsaved secret");
        assert_eq!(sweep(&s), 1);

        let key = pmacs::autosave::key_for(std::path::Path::new(&f));
        let file = dir.join(&key);
        let fmode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(fmode, 0o600, "recovery file holds unsaved contents");
        let dmode = std::fs::metadata(dir.join("autosave"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn interval_is_a_validated_getter_setter_and_enable_gates_the_sweep() {
    let dir = fresh_state_dir();
    let s = editor(&dir);

    let default_ms: i64 = eval(&s, "return pmacs.autosave.interval_ms()");
    assert_eq!(default_ms, 30000, "Emacs's auto-save-timeout");

    let set: i64 = eval(&s, "return pmacs.autosave.interval_ms(60000)");
    assert_eq!(set, 60000);
    let read_back: i64 = eval(&s, "return pmacs.autosave.interval_ms()");
    assert_eq!(read_back, 60000, "change takes effect immediately");

    // Floats floor; bad values error.
    let floored: i64 = eval(&s, "return pmacs.autosave.interval_ms(1500.9)");
    assert_eq!(floored, 1500);
    for bad in ["'soon'", "0", "999", "-1", "{}"] {
        let ok: bool = eval(
            &s,
            &format!("return (pcall(pmacs.autosave.interval_ms, {bad}))"),
        );
        assert!(!ok, "interval_ms({bad}) must be rejected");
    }
    // A rejected set leaves the previous value intact.
    let still: i64 = eval(&s, "return pmacs.autosave.interval_ms()");
    assert_eq!(still, 1500);

    // enable(false) makes sweep a no-op even with a dirty buffer.
    let f = write_file(&dir, "a.txt", "body\n");
    open_and_dirty(&s, &f, "x");
    exec(&s, "pmacs.autosave.enable(false)");
    assert_eq!(sweep(&s), 0, "disabled → no sweep");
    assert_eq!(status(&s, &f), "none");
    exec(&s, "pmacs.autosave.enable(true)");
    assert_eq!(sweep(&s), 1, "re-enabled → sweeps");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn before_quit_sweeps_synchronously_without_vetoing() {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, "a.txt", "body\n");
    open_and_dirty(&s, &f, "unsaved ");
    // Not swept yet.
    assert_eq!(status(&s, &f), "none");

    // before-quit is short-circuit: a `true` result means "not vetoed".
    let not_vetoed: bool = eval(&s, "return pmacs.hook.run('editor.before-quit')");
    assert!(not_vetoed, "autosave must never veto quit");
    assert_eq!(
        status(&s, &f),
        "fresh",
        "quitting with unsaved changes leaves a recovery copy"
    );
    std::fs::remove_dir_all(&dir).ok();
}
