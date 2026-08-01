//! Desktop-save acceptance (Arc 3 phase 2): save the open file buffers,
//! window layout, and per-window positions, then restore them into a
//! fresh editor. Driven through the `pmacs.session` bindings and the
//! real Lua surface; fixtures use the split bindings plus direct core
//! access.
//!
//! Each test injects a private tempdir `StateDir` (integration tests
//! link the lib without `cfg(test)`), so nothing touches a developer's
//! real state dir. The session key is cwd-based, so a save in one
//! editor and a restore in another (same process) agree on the key.
//!
//! Framing: `docs/desktop-save-framing.md`.

use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use pmacs::window::{LayoutNode, WindowId};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Shared tempdir so a save and a restore in two editors use one state
/// store. Unique per test.
fn fresh_state_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-desktop-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn editor(state_dir: &std::path::Path) -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
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

fn save(s: &EditorState) -> bool {
    pmacs::desktop::save_session(s.lua_host.lua()).unwrap()
}

fn restore(s: &mut EditorState) {
    pmacs::desktop::restore_session(s.lua_host.lua()).unwrap();
}

/// The LOCAL frontend's leaves in preorder as `(path, cursor, view_top)`.
fn leaves(s: &EditorState) -> Vec<(String, u64, usize)> {
    let core = s.core.borrow();
    let view = core.views.get(&FrontendId::LOCAL).unwrap();
    let mut ids = Vec::new();
    collect(&view.layout.root, &mut ids);
    let reg = core.registry.borrow();
    ids.into_iter()
        .map(|id| {
            let w = core.windows.get(&id).unwrap();
            let path = reg
                .get(w.buffer_id)
                .unwrap()
                .file_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            (path, w.cursor, w.view_top)
        })
        .collect()
}

fn collect(node: &LayoutNode, out: &mut Vec<WindowId>) {
    match node {
        LayoutNode::Leaf(id) => out.push(*id),
        LayoutNode::Split { children, .. } => {
            for c in children {
                collect(c, out);
            }
        }
    }
}

/// The root split's weights, if the root is a split.
fn root_weights(s: &EditorState) -> Option<Vec<u32>> {
    let core = s.core.borrow();
    match &core.views.get(&FrontendId::LOCAL).unwrap().layout.root {
        LayoutNode::Split { weights, .. } => Some(weights.clone()),
        LayoutNode::Leaf(_) => None,
    }
}

/// All file-buffer paths in the registry (visible or hidden).
fn buffer_paths(s: &EditorState) -> Vec<String> {
    let core = s.core.borrow();
    let reg = core.registry.borrow();
    reg.ids()
        .iter()
        .filter_map(|&id| {
            reg.get(id)
                .ok()?
                .file_path()
                .map(|p| p.display().to_string())
        })
        .collect()
}

/// Build a two-pane vertical split: left shows A, right shows B, with
/// the given root weights and per-pane cursors. Returns `(a_path, b_path)`.
fn build_two_pane(
    s: &EditorState,
    dir: &std::path::Path,
    weights: [u32; 2],
    ca: u64,
    cb: u64,
) -> (String, String) {
    let a = write_file(dir, "a.txt", "aaaa\nbbbb\ncccc\ndddd\neeee\n");
    let b = write_file(dir, "b.txt", "1111\n2222\n3333\n4444\n5555\n");
    exec(s, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(s, "pmacs.window.split_vertical()");
    // Focus the new (right) pane and open B there.
    exec(s, "pmacs.window.focus_next()");
    exec(s, &format!("pmacs.buffer.find_or_open({b:?})"));
    // Set weights + cursors directly (the split API is 1:1 only).
    let mut core = s.core.borrow_mut();
    if let LayoutNode::Split { weights: w, .. } = &mut core.active_layout_mut().root {
        *w = weights.to_vec();
    }
    let ids: Vec<WindowId> = core.views[&FrontendId::LOCAL].layout.iter_ids();
    core.windows.get_mut(&ids[0]).unwrap().cursor = ca;
    core.windows.get_mut(&ids[1]).unwrap().cursor = cb;
    (a, b)
}

#[test]
fn round_trips_layout_weights_buffers_and_cursors() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let (a, b) = build_two_pane(&src, &dir, [3, 1], 5, 12);
    assert!(save(&src), "a desktop was written");

    // Fresh editor, same state store → restore.
    let mut dst = editor(&dir);
    restore(&mut dst);

    assert_eq!(
        root_weights(&dst).as_deref(),
        Some(&[3, 1][..]),
        "weights round-trip"
    );
    let ls = leaves(&dst);
    assert_eq!(ls.len(), 2, "two panes");
    assert_eq!(ls[0], (a, 5, 0), "left pane = A @ cursor 5");
    assert_eq!(ls[1], (b, 12, 0), "right pane = B @ cursor 12");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hidden_buffer_survives_restore() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let a = write_file(&dir, "a.txt", "aaaa\n");
    let b = write_file(&dir, "b.txt", "bbbb\n");
    // Open A, then B in the SAME window → A is now hidden but live.
    exec(&src, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(&src, &format!("pmacs.buffer.find_or_open({b:?})"));
    assert!(save(&src));

    let mut dst = editor(&dir);
    restore(&mut dst);
    let mut paths = buffer_paths(&dst);
    paths.sort();
    assert!(paths.contains(&a), "hidden buffer A restored");
    assert!(paths.contains(&b), "visible buffer B restored");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn after_load_fires_with_the_restored_leaf_active() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let (_a, _b) = build_two_pane(&src, &dir, [1, 1], 0, 0);
    assert!(save(&src));

    // Restore in a fresh editor with a probe on buffer.after-load that
    // records the ACTIVE file path each time it fires. If the wrong
    // buffer were active, the recorded paths would not match.
    let mut dst = editor(&dir);
    exec(
        &dst,
        r#"
        _G.seen = {}
        pmacs.hook.add("buffer.after-load", function()
          _G.seen[#_G.seen + 1] = pmacs.editor.file_path()
        end)
        "#,
    );
    restore(&mut dst);
    let seen: Vec<String> = dst.lua_host.lua().load("return _G.seen").eval().unwrap();
    // Two distinct files, each observed active exactly when its
    // after-load fired.
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        2,
        "after-load fired once per buffer, active-correct: {seen:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn same_file_in_two_panes_keeps_distinct_positions_and_fires_per_pane() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let a = write_file(&dir, "a.txt", "aaaa\nbbbb\ncccc\ndddd\n");
    // Two panes both showing A, different cursors.
    exec(&src, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(&src, "pmacs.window.split_vertical()");
    {
        let mut core = src.core.borrow_mut();
        let ids: Vec<WindowId> = core.views[&FrontendId::LOCAL].layout.iter_ids();
        core.windows.get_mut(&ids[0]).unwrap().cursor = 2;
        core.windows.get_mut(&ids[1]).unwrap().cursor = 15;
    }
    assert!(save(&src));

    let mut dst = editor(&dir);
    // after-load must fire once PER PANE (not once per buffer) so each
    // window gets its own per-window overlay (syntax attaches to the
    // active window; LSP attach is idempotent).
    exec(
        &dst,
        "_G.fires = 0; pmacs.hook.add('buffer.after-load', function() _G.fires = _G.fires + 1 end)",
    );
    restore(&mut dst);
    let fires: i64 = dst.lua_host.lua().load("return _G.fires").eval().unwrap();
    assert_eq!(
        fires, 2,
        "after-load fires once per pane for the same buffer"
    );
    let ls = leaves(&dst);
    assert_eq!(ls.len(), 2);
    assert_eq!(ls[0], (a.clone(), 2, 0));
    assert_eq!(ls[1], (a, 15, 0));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn daemon_mode_disables_save_and_restore() {
    let dir = fresh_state_dir();
    // Seed a desktop from a normal (non-daemon) editor.
    let src = editor(&dir);
    let a = write_file(&dir, "a.txt", "aaaa\n");
    exec(&src, &format!("pmacs.buffer.find_or_open({a:?})"));
    assert!(save(&src));

    // A daemon editor must not save or restore (local-only, Q#DS9) —
    // the Rust gate holds regardless of what init did.
    let mut daemon = editor(&dir);
    daemon
        .lua_host
        .lua()
        .set_app_data(pmacs::lua_bindings::DaemonMode);
    assert!(!save(&daemon), "daemon save is a no-op");
    restore(&mut daemon);
    assert!(
        leaves(&daemon).iter().all(|(p, _, _)| p != &a),
        "daemon restore is a no-op"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn disabling_desktop_mode_unarms_restore() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let a = write_file(&dir, "a.txt", "aaaa\n");
    exec(&src, &format!("pmacs.buffer.find_or_open({a:?})"));
    assert!(save(&src));

    // Enable then disable desktop_mode → the startup restore must NOT
    // fire (arming is a boolean; disable unarms).
    let mut dst = editor(&dir);
    exec(
        &dst,
        "pmacs.session.desktop_mode(true); pmacs.session.desktop_mode(false)",
    );
    dst.restore_desktop_if_armed(false);
    assert!(
        leaves(&dst).iter().all(|(p, _, _)| p != &a),
        "disabled desktop_mode leaves restore unarmed"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_file_collapses_and_focus_falls_back() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let (a, b) = build_two_pane(&src, &dir, [1, 1], 0, 0);
    // The right pane (B) was focused at save (focus_next moved there).
    assert!(save(&src));
    // Delete B before restore → its leaf collapses; focus must fall
    // back to a surviving leaf (A), not crash.
    std::fs::remove_file(&b).unwrap();

    let mut dst = editor(&dir);
    restore(&mut dst);
    let ls = leaves(&dst);
    assert_eq!(ls.len(), 1, "only A survives");
    assert_eq!(ls[0].0, a);
    // Active window is a real, surviving window.
    let core = dst.core.borrow();
    let active = core.views[&FrontendId::LOCAL].active;
    assert!(core.windows.contains_key(&active), "active window survives");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn restore_leaves_no_orphan_windows() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    build_two_pane(&src, &dir, [1, 1], 0, 0);
    assert!(save(&src));

    let mut dst = editor(&dir);
    // dst starts with 1 scratch window; after restore only the rebuilt
    // leaves should remain in core.windows.
    restore(&mut dst);
    let core = dst.core.borrow();
    let leaf_ids: std::collections::HashSet<_> = core.views[&FrontendId::LOCAL]
        .layout
        .iter_ids()
        .into_iter()
        .collect();
    assert_eq!(
        core.windows.len(),
        leaf_ids.len(),
        "no orphan windows linger in core.windows"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn session_keys_scope_by_name_vs_cwd() {
    let dir = fresh_state_dir();
    // Save under an instance name.
    let src_named = editor(&dir);
    src_named.lua_host.set_instance_name(Some("work".into()));
    let a = write_file(&dir, "a.txt", "aaaa\n");
    exec(&src_named, &format!("pmacs.buffer.find_or_open({a:?})"));
    assert!(save(&src_named));

    // A cwd-keyed (nameless) editor must NOT see the named desktop.
    let mut dst_cwd = editor(&dir);
    dst_cwd.lua_host.set_instance_name(None);
    restore(&mut dst_cwd);
    assert!(
        leaves(&dst_cwd).iter().all(|(p, _, _)| p != &a),
        "cwd session does not restore the name session's desktop"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn modified_buffer_warns_on_restore() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let a = write_file(&dir, "a.txt", "aaaa\nbbbb\n");
    exec(&src, &format!("pmacs.buffer.find_or_open({a:?})"));
    // Dirty the buffer (an edit) so it saves as modified.
    exec(&src, "pmacs.window.buffer():insert(0, 'x')");
    assert!(save(&src));

    let mut dst = editor(&dir);
    restore(&mut dst);
    let status = dst.core.borrow().status.clone();
    assert!(
        status.contains("unsaved changes"),
        "restore warns about modified buffers: {status:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_gate_respects_file_arg_and_arming() {
    let dir = fresh_state_dir();
    let src = editor(&dir);
    let a = write_file(&dir, "a.txt", "aaaa\n");
    exec(&src, &format!("pmacs.buffer.find_or_open({a:?})"));
    assert!(save(&src));

    // Not armed → no restore even with no file arg.
    let mut d1 = editor(&dir);
    d1.restore_desktop_if_armed(false);
    assert!(
        leaves(&d1).iter().all(|(p, _, _)| p != &a),
        "unarmed: no restore"
    );

    // Armed + a file arg → still no restore (Q#DS7).
    let mut d2 = editor(&dir);
    exec(&d2, "pmacs.session.arm_restore()");
    d2.restore_desktop_if_armed(true);
    assert!(
        leaves(&d2).iter().all(|(p, _, _)| p != &a),
        "armed + file arg: no restore"
    );

    // Armed + no file arg → restore.
    let mut d3 = editor(&dir);
    exec(&d3, "pmacs.session.arm_restore()");
    d3.restore_desktop_if_armed(false);
    assert!(
        leaves(&d3).iter().any(|(p, _, _)| p == &a),
        "armed + no file: restores"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
