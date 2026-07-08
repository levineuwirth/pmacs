//! Persistence phase 1 acceptance (Arc 3): the `pmacs.state` confined
//! store, saveplace (cursor restored on reopen), and recentf (MRU
//! record + dedup + picker), driven through the real Lua surface.
//!
//! Integration tests link the lib without `cfg(test)`, so
//! `EditorState::new()` configures the state dir from the environment.
//! Each test **overrides that with a private tempdir** (via the
//! `StateDir` app-data) before touching any file, so the suite never
//! reads or writes a developer's real `~/.local/state/pmacs`.
//!
//! Framing: docs/persistence-framing.md.

use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use std::path::PathBuf;

/// A fresh editor whose state dir is a private, empty tempdir (unique
/// per call so parallel tests never share or wipe each other's dirs).
fn editor_with_state_dir() -> (EditorState, PathBuf) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-persist-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("mk state tempdir");
    let s = EditorState::new();
    // Override whatever startup configured with our tempdir.
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    (s, dir)
}

/// Write a real file under `dir` and return its path string.
fn write_file(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write test file");
    p.display().to_string()
}

#[test]
fn state_round_trips_and_rejects_escapes() {
    let (s, dir) = editor_with_state_dir();
    let out: (bool, Option<String>, bool, bool) = s
        .lua_host
        .lua()
        .load(
            r#"
            local wrote = pmacs.state.write("recentf", "a\nb\n")
            local back = pmacs.state.read("recentf")
            -- Confinement: an escaping key must error (pcall → false).
            local esc_ok = pcall(pmacs.state.write, "../escape", "x")
            local abs_ok = pcall(pmacs.state.read, "/etc/passwd")
            return wrote, back, esc_ok, abs_ok
            "#,
        )
        .eval()
        .expect("state round-trip");
    assert!(out.0, "write returned true");
    assert_eq!(
        out.1.as_deref(),
        Some("a\nb\n"),
        "read returns what was written"
    );
    assert!(!out.2, "`../escape` key is rejected");
    assert!(!out.3, "absolute key is rejected");
    // And it actually landed under our tempdir, nowhere else.
    assert!(dir.join("recentf").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn state_is_inert_when_unconfigured() {
    let s = EditorState::new();
    // Simulate no state dir (the cfg(test) lib case, or no HOME).
    s.lua_host.lua().remove_app_data::<StateDir>();
    let (avail, wrote, read): (bool, bool, Option<String>) = s
        .lua_host
        .lua()
        .load(
            r#"
            return pmacs.state.available(),
                   pmacs.state.write("recentf", "should not persist"),
                   pmacs.state.read("recentf")
            "#,
        )
        .eval()
        .expect("inert state");
    assert!(!avail, "unconfigured → not available");
    assert!(!wrote, "write is a no-op (returns false)");
    assert_eq!(read, None, "read returns nil");
}

#[test]
fn recentf_records_dedups_and_orders_mru() {
    let (s, dir) = editor_with_state_dir();
    let a = write_file(&dir, "a.rs", "fn a() {}\n");
    let b = write_file(&dir, "b.rs", "fn b() {}\n");
    // Open a, then b, then a again — MRU should be [a, b].
    for path in [&a, &b, &a] {
        s.lua_host
            .lua()
            .load(format!("pmacs.buffer.find_or_open({path:?})"))
            .exec()
            .expect("open file");
    }
    let list: Vec<String> = s
        .lua_host
        .lua()
        .load("return pmacs.recentf.list()")
        .eval()
        .expect("recentf list");
    assert_eq!(list, vec![a.clone(), b.clone()], "MRU-first, deduped");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saveplace_restores_cursor_on_reopen() {
    let (s, dir) = editor_with_state_dir();
    let f = write_file(&dir, "place.rs", "line0\nline1\nline2\nline3\n");
    // Open, move to byte 12 ("line2"), save (before-save records the
    // place), then KILL the buffer — so the reopen is a fresh load
    // (buffer.after-load), the cross-session path saveplace targets.
    let cursor: i64 = s
        .lua_host
        .lua()
        .load(format!(
            r#"
            local b = pmacs.buffer.find_or_open({f:?})
            pmacs.editor.goto_byte(12)
            pmacs.command.invoke("buffer.save")
            pmacs.buffer.kill(b)
            -- Reopen from scratch: after-load fires → saveplace restores.
            pmacs.buffer.find_or_open({f:?})
            return pmacs.editor.cursor()
            "#
        ))
        .eval()
        .expect("place + save + kill + reopen");
    assert_eq!(cursor, 12, "saveplace restored the cursor byte on reload");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saveplace_can_be_disabled() {
    let (s, dir) = editor_with_state_dir();
    let f = write_file(&dir, "off.rs", "aaaa\nbbbb\ncccc\n");
    let cursor: i64 = s
        .lua_host
        .lua()
        .load(format!(
            r#"
            pmacs.saveplace.enable(false)
            local b = pmacs.buffer.find_or_open({f:?})
            pmacs.editor.goto_byte(10)
            pmacs.command.invoke("buffer.save")
            pmacs.buffer.kill(b)
            pmacs.buffer.find_or_open({f:?})
            return pmacs.editor.cursor()
            "#
        ))
        .eval()
        .expect("disabled saveplace flow");
    assert_eq!(cursor, 0, "disabled saveplace leaves the cursor at the top");
    std::fs::remove_dir_all(&dir).ok();
}
