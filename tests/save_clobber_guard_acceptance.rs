//! `EditorCore::save()` must not silently overwrite a file that changed on
//! disk since the buffer read it.
//!
//! Before this guard, pmacs wrote unconditionally and only *then* recorded
//! the new `FileMeta` — so another editor's (or a `git checkout`'s) writes
//! were destroyed without a word. The comparison seam
//! (`FileMeta: PartialEq`, `file_io::current_meta`) already existed and no
//! caller used it.

use pmacs::editor::EditorState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn tempdir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "pmacs-clobber-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn write(p: &std::path::Path, body: &str) {
    std::fs::write(p, body).unwrap();
}

fn read(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// Open `path` and dirty the buffer.
fn open_and_dirty(s: &EditorState, path: &str) {
    exec(
        s,
        &format!("pmacs.buffer.find_or_open({path:?}); pmacs.window.buffer():insert(0, 'mine ')"),
    );
}

#[test]
fn save_refuses_to_clobber_a_file_changed_on_disk() {
    let dir = tempdir();
    let f = dir.join("a.txt");
    write(&f, "original\n");
    let fs = f.display().to_string();

    let s = EditorState::new_with_roots(&crate::iso::roots());
    open_and_dirty(&s, &fs);

    // Another writer lands between our read and our save.
    write(&f, "THEIRS -- do not destroy\n");

    let saved: bool = eval(&s, "return pmacs.editor.save()");
    assert!(!saved, "save must refuse");
    assert_eq!(
        read(&f),
        "THEIRS -- do not destroy\n",
        "their content is intact"
    );
    assert!(
        status(&s).contains("changed on disk") && status(&s).contains("save-anyway"),
        "the refusal says what happened and how to override: {:?}",
        status(&s)
    );
    // The buffer keeps its unsaved edits — nothing was lost on our side.
    let modified: bool = eval(&s, "return pmacs.window.buffer():is_modified()");
    assert!(modified);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_save_command_does_not_fire_after_save_when_it_refuses() {
    let dir = tempdir();
    let f = dir.join("a.txt");
    write(&f, "original\n");
    let fs = f.display().to_string();

    let s = EditorState::new_with_roots(&crate::iso::roots());
    open_and_dirty(&s, &fs);
    exec(
        &s,
        "_G.after = 0; pmacs.hook.add('buffer.after-save', function() _G.after = _G.after + 1 end)",
    );
    write(&f, "theirs\n");

    exec(&s, "pmacs.command.invoke('buffer.save')");
    let after: i64 = eval(&s, "return _G.after");
    assert_eq!(after, 0, "after-save must not fire on a refused save");
    assert_eq!(read(&f), "theirs\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_anyway_overwrites_deliberately_and_resyncs_meta() {
    let dir = tempdir();
    let f = dir.join("a.txt");
    write(&f, "original\n");
    let fs = f.display().to_string();

    let s = EditorState::new_with_roots(&crate::iso::roots());
    open_and_dirty(&s, &fs);
    write(&f, "theirs\n");
    assert!(!eval::<bool>(&s, "return pmacs.editor.save()"));

    // The user looked and decided their buffer wins.
    exec(&s, "pmacs.command.invoke('buffer.save-anyway')");
    assert_eq!(read(&f), "mine original\n", "overwritten on purpose");

    // The buffer re-syncs to the file it just wrote, so an immediate
    // ordinary save is allowed again.
    exec(&s, "pmacs.window.buffer():insert(0, 'more ')");
    assert!(
        eval::<bool>(&s, "return pmacs.editor.save()"),
        "meta was refreshed by the forced save"
    );
    assert_eq!(read(&f), "more mine original\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unchanged_file_saves_normally_and_repeatedly() {
    let dir = tempdir();
    let f = dir.join("a.txt");
    write(&f, "original\n");
    let fs = f.display().to_string();

    let s = EditorState::new_with_roots(&crate::iso::roots());
    open_and_dirty(&s, &fs);
    assert!(eval::<bool>(&s, "return pmacs.editor.save()"));
    assert_eq!(read(&f), "mine original\n");

    // A save updates our recorded meta, so the next one is not a false
    // positive — the guard must not trip on our own writes.
    exec(&s, "pmacs.window.buffer():insert(0, 'again ')");
    assert!(eval::<bool>(&s, "return pmacs.editor.save()"));
    assert_eq!(read(&f), "again mine original\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_deleted_file_is_recreated_not_refused() {
    let dir = tempdir();
    let f = dir.join("a.txt");
    write(&f, "original\n");
    let fs = f.display().to_string();

    let s = EditorState::new_with_roots(&crate::iso::roots());
    open_and_dirty(&s, &fs);
    // Nothing on disk to clobber, so recreating it is not data loss.
    std::fs::remove_file(&f).unwrap();
    assert!(
        eval::<bool>(&s, "return pmacs.editor.save()"),
        "a vanished file is recreated, not refused"
    );
    assert_eq!(read(&f), "mine original\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_new_file_buffer_refuses_once_someone_else_creates_the_file() {
    let dir = tempdir();
    let missing = dir.join("draft.txt");

    let s = EditorState::new_with_roots(&crate::iso::roots());
    // The argv `[new file]` shape: a path with nothing on disk, no meta.
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
    exec(&s, "pmacs.window.buffer():insert(0, 'my draft')");

    // While we drafted, someone created the file. We have never seen its
    // contents, so writing over them is exactly the clobber we refuse.
    write(&missing, "theirs\n");
    assert!(!eval::<bool>(&s, "return pmacs.editor.save()"));
    assert_eq!(read(&missing), "theirs\n");

    // With the file still absent it would have saved cleanly.
    std::fs::remove_file(&missing).unwrap();
    assert!(eval::<bool>(&s, "return pmacs.editor.save()"));
    assert_eq!(read(&missing), "my draft");
    std::fs::remove_dir_all(&dir).ok();
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
