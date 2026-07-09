// desktop.rs --- session desktop-save (Arc 3 phase 2).

//! Serialize the open file buffers + window layout + per-window
//! positions so a session survives a restart. Emacs `desktop.el`,
//! opt-in via `pmacs.session.desktop_mode(true)`.
//!
//! This module owns the whole feature: the serde mirror types (the core
//! window enums are not serde and stay that way), the SHA-256 session
//! key, the save-side snapshot, and the restore orchestration that opens
//! files, prunes/rebuilds windows, and fires `buffer.after-load`.
//! [`save_session`] / [`restore_session`] take the `&Lua` that carries
//! the editor's `SharedCore` / `StateDir` / `LocalInstanceInfo`
//! app-data, so they run identically from a `pmacs.session.*` binding
//! and from the `RunLocal` startup trigger.
//!
//! Framing: docs/desktop-save-framing.md.

use std::collections::HashMap;
use std::path::Path;

use mlua::Lua;
use serde::{Deserialize, Serialize};

use crate::buffer::BufferId;
use crate::editor_core::EditorCore;
use crate::hash::sha256_hex;
use crate::lua_bindings::{LocalInstanceInfo, SharedCore, StateDir, fire_after_load_hook};
use crate::protocol::FrontendId;
use crate::text_view::TextView;
use crate::window::{FrontendView, Layout, LayoutNode, Orientation, Window, WindowId};

/// Bump when the on-disk shape changes incompatibly. Restore ignores a
/// desktop whose `version` it does not recognize.
pub const DESKTOP_VERSION: u32 = 1;

/// A serializable snapshot of a session: every open file buffer, the
/// window layout, and which leaf was focused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedDesktop {
    /// Format version ([`DESKTOP_VERSION`]).
    pub version: u32,
    /// The [`session_key`] this was saved under; restore refuses a
    /// mismatch (defense in depth — the filename already encodes it).
    pub session_key: String,
    /// **Every** open file buffer (visible or hidden), so a file opened
    /// then switched away from survives restore — not just layout
    /// leaves.
    pub buffers: Vec<SavedBuffer>,
    /// The window layout tree.
    pub root: SavedNode,
    /// Preorder index (into the surviving leaf sequence) of the focused
    /// leaf. Resolved with a nearest-neighbor fallback if the focused
    /// leaf did not survive (Q#DS10).
    pub active_leaf: usize,
}

/// One open file buffer. Contents are never saved (Q#DS6); `modified`
/// only drives the restore-time "unsaved changes" warning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedBuffer {
    /// Absolute-or-relative path exactly as the buffer holds it.
    pub path: String,
    /// Whether the buffer had unsaved edits at save time.
    pub modified: bool,
}

/// Mirror of [`LayoutNode`] — a `Leaf` carries the window's file +
/// position; a `Split` carries orientation, weights, and children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SavedNode {
    /// A window showing a file.
    Leaf(SavedLeaf),
    /// A proportional split.
    Split {
        /// Split axis.
        orientation: SavedOrientation,
        /// Per-child weights (same length as `children`).
        weights: Vec<u32>,
        /// Children in display order.
        children: Vec<SavedNode>,
    },
}

/// A restored window: which file, and where the cursor / viewport sit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLeaf {
    /// File path (always a file buffer — scratch/special are dropped).
    pub path: String,
    /// Cursor byte offset.
    pub cursor: u64,
    /// First visible source line.
    pub view_top: usize,
}

/// Serde mirror of [`Orientation`] (which is not itself serde).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedOrientation {
    /// Children stacked top-to-bottom.
    Horizontal,
    /// Children side-by-side.
    Vertical,
}

impl From<Orientation> for SavedOrientation {
    fn from(o: Orientation) -> Self {
        match o {
            Orientation::Horizontal => SavedOrientation::Horizontal,
            Orientation::Vertical => SavedOrientation::Vertical,
        }
    }
}

impl From<SavedOrientation> for Orientation {
    fn from(o: SavedOrientation) -> Self {
        match o {
            SavedOrientation::Horizontal => Orientation::Horizontal,
            SavedOrientation::Vertical => Orientation::Vertical,
        }
    }
}

/// The state-store key a session's desktop is saved under (Q#DS5).
///
/// `name.<sha256hex>` keyed on the instance/socket name when set, else
/// `cwd.<sha256hex>` keyed on the working directory (Emacs's
/// per-directory model). Hashing both uniformly sidesteps odd
/// characters and satisfies the `pmacs.state` key charset (a raw `:` or
/// `/` would be rejected); the `desktop/` prefix is added by the store
/// key, not here.
#[must_use]
pub fn session_key(instance_name: Option<&str>, working_directory: &str) -> String {
    match instance_name {
        Some(name) => format!("name.{}", sha256_hex(name)),
        None => format!("cwd.{}", sha256_hex(working_directory)),
    }
}

/// The `pmacs.state` key (under a `desktop/` subdir) for a session key.
#[must_use]
pub fn desktop_state_key(session_key: &str) -> String {
    format!("desktop/{session_key}")
}

/// Build the serializable layout tree from a core [`LayoutNode`],
/// resolving each leaf window to a [`SavedLeaf`] (returning `None` for a
/// non-file leaf, which is dropped and its split collapsed).
///
/// Returns the mirror node plus the **surviving leaf window-ids in
/// preorder** — the caller uses that list to compute `active_leaf`
/// (with the Q#DS10 fallback). Returns `None` when nothing survives.
pub fn build_saved_node(
    node: &LayoutNode,
    resolve: &impl Fn(WindowId) -> Option<SavedLeaf>,
    surviving: &mut Vec<WindowId>,
) -> Option<SavedNode> {
    match node {
        LayoutNode::Leaf(id) => resolve(*id).map(|leaf| {
            surviving.push(*id);
            SavedNode::Leaf(leaf)
        }),
        LayoutNode::Split {
            orientation,
            weights,
            children,
        } => {
            let mut kept: Vec<(SavedNode, u32)> = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if let Some(saved) = build_saved_node(child, resolve, surviving) {
                    let w = weights.get(i).copied().unwrap_or(1).max(1);
                    kept.push((saved, w));
                }
            }
            match kept.len() {
                0 => None,
                // A split with a single surviving child collapses to
                // that child (the sibling that carried the other pane is
                // gone), so the tree never has a one-child split.
                1 => Some(kept.into_iter().next().unwrap().0),
                _ => {
                    let (nodes, weights): (Vec<_>, Vec<_>) = kept.into_iter().unzip();
                    Some(SavedNode::Split {
                        orientation: (*orientation).into(),
                        weights,
                        children: nodes,
                    })
                }
            }
        }
    }
}

/// Given the full preorder leaf list (before dropping) with a survived
/// flag, and the focused window, return the `active_leaf` index into
/// the *surviving* sequence — the focused leaf if it survived, else the
/// nearest surviving preorder neighbor (later preferred), else 0
/// (Q#DS10). `surviving_ids` is the preorder list of leaves that
/// survived, in the same order they appear in `full`.
#[must_use]
pub fn resolve_active_leaf(
    full: &[(WindowId, bool)],
    surviving_ids: &[WindowId],
    focused: WindowId,
) -> usize {
    // Direct hit: focused survived.
    if let Some(i) = surviving_ids.iter().position(|&id| id == focused) {
        return i;
    }
    // Focused was dropped: find its position in the full preorder list,
    // then the nearest survivor (scan right, then left).
    let Some(fpos) = full.iter().position(|&(id, _)| id == focused) else {
        return 0;
    };
    let neighbor = full
        .iter()
        .enumerate()
        .filter(|(_, (_, survived))| *survived)
        .min_by_key(|(pos, _)| {
            // Prefer later leaves on ties: right distance rounds down.
            let dist = pos.abs_diff(fpos);
            (dist, i32::from(*pos < fpos))
        })
        .map(|(_, (id, _))| *id);
    neighbor
        .and_then(|id| surviving_ids.iter().position(|&s| s == id))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Save / restore orchestration (driven from `pmacs.session.*` and the
// RunLocal startup trigger; both hand us the `&Lua` that carries the
// SharedCore / StateDir / LocalInstanceInfo app-data).
// ---------------------------------------------------------------------------

/// True when this process is a daemon (multi-frontend). Desktop
/// save/restore is local-only in v1 (Q#DS9); this is the **reliable**
/// enforcement — the `DaemonMode` marker is set right after the daemon's
/// `EditorState::new()`, so it is present for every save/restore that
/// can run after startup (the before-quit hook, manual commands, direct
/// binding calls), even though `init.lua` runs before it is set. Both
/// `save_session` and `restore_session` no-op when it holds.
fn is_daemon(lua: &Lua) -> bool {
    lua.app_data_ref::<crate::lua_bindings::DaemonMode>()
        .is_some()
}

/// The desktop session key for this process (`cwd.<hash>` in local mode).
fn session_key_from_lua(lua: &Lua) -> String {
    match lua
        .app_data_ref::<LocalInstanceInfo>()
        .map(|i| i.build_identity())
    {
        Some(id) => session_key(id.instance_name.as_deref(), &id.working_directory),
        None => session_key(None, ""),
    }
}

/// Snapshot the LOCAL frontend as a [`SavedDesktop`] (Q#DS2). `None`
/// when no file window survives (nothing worth saving).
#[must_use]
pub fn snapshot(core: &EditorCore, session_key: String) -> Option<SavedDesktop> {
    let view = core.views.get(&FrontendId::LOCAL)?;
    let focused = view.active;
    let reg = core.registry.borrow();

    let resolve = |wid: WindowId| -> Option<SavedLeaf> {
        let win = core.windows.get(&wid)?;
        let path = reg.get(win.buffer_id).ok()?.file_path()?;
        Some(SavedLeaf {
            path: path.display().to_string(),
            cursor: win.cursor,
            view_top: win.view_top,
        })
    };

    let mut surviving = Vec::new();
    let root = build_saved_node(&view.layout.root, &resolve, &mut surviving)?;

    let full: Vec<(WindowId, bool)> = view
        .layout
        .iter_ids()
        .into_iter()
        .map(|id| (id, surviving.contains(&id)))
        .collect();
    let active_leaf = resolve_active_leaf(&full, &surviving, focused);

    let buffers = reg
        .ids()
        .iter()
        .filter_map(|&id| {
            let b = reg.get(id).ok()?;
            let path = b.file_path()?;
            Some(SavedBuffer {
                path: path.display().to_string(),
                modified: b.is_modified(),
            })
        })
        .collect();

    Some(SavedDesktop {
        version: DESKTOP_VERSION,
        session_key,
        buffers,
        root,
        active_leaf,
    })
}

/// Serialize the current session to the desktop state file (Q#DS1).
/// `Ok(false)` when the state dir is unconfigured or nothing is worth
/// saving. Never fails hard for the before-quit path (Q#DS8) — the
/// caller may ignore the error.
///
/// # Errors
/// A state-write / serialization failure (surfaced for manual save).
pub fn save_session(lua: &Lua) -> Result<bool, String> {
    if is_daemon(lua) {
        return Ok(false); // local-only in v1 (Q#DS9)
    }
    let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
        return Ok(false);
    };
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();
    let key = session_key_from_lua(lua);
    let snap = snapshot(&core.borrow(), key);
    let Some(snap) = snap else {
        return Ok(false);
    };
    let json = serde_json::to_string(&snap).map_err(|e| e.to_string())?;
    let state_key = desktop_state_key(&snap.session_key);
    crate::state::write(&base, &state_key, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Rebuild the LOCAL session from the desktop state file (Q#DS3). A
/// no-op when no desktop is saved for this key / version / session
/// mismatches. Fires `buffer.after-load` via `lua` with each restored
/// leaf active.
///
/// # Errors
/// Parse / state-read failures; a missing individual file collapses its
/// leaf rather than failing the whole restore.
pub fn restore_session(lua: &Lua) -> Result<(), String> {
    if is_daemon(lua) {
        return Ok(()); // local-only in v1 (Q#DS9)
    }
    let key = session_key_from_lua(lua);
    let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
        return Ok(());
    };
    let state_key = desktop_state_key(&key);
    let Some(json) = crate::state::read(&base, &state_key).map_err(|e| e.to_string())? else {
        return Ok(()); // no desktop saved
    };
    let saved: SavedDesktop = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if saved.version != DESKTOP_VERSION || saved.session_key != key {
        return Ok(()); // unrecognized / wrong session
    }
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();
    let modified = restore_into(&core, &saved, || fire_after_load_hook(lua));
    if modified > 0 {
        core.borrow_mut().status =
            format!("desktop restored; {modified} buffer(s) had unsaved changes when saved");
    }
    Ok(())
}

/// One restored leaf window, carried from the tree build to the
/// activate-then-fire pass (Q#DS3).
struct RestoreLeaf {
    window: WindowId,
    cursor: u64,
    view_top: usize,
}

/// Do the structural rebuild: open buffers, prune the old LOCAL layout,
/// build the new tree + windows, install the view, then fire
/// `buffer.after-load` (via `fire_after_load`) with each newly-loaded
/// leaf active and apply the exact per-leaf `cursor`/`view_top` afterward
/// (desktop wins over saveplace, Q#DS3). Returns the count of buffers
/// that were modified at save time (for the Q#DS6 warning).
///
/// Takes `&SharedCore` (not a borrow) so it can release the core borrow
/// around each hook fire — `buffer.after-load` re-enters `pmacs.editor.*`
/// which re-borrows the core.
pub fn restore_into(
    core: &SharedCore,
    saved: &SavedDesktop,
    mut fire_after_load: impl FnMut(),
) -> usize {
    // (2) Open every buffer up front (hidden ones survive), keyed by the
    // raw saved path. A missing file is absent → its leaves collapse.
    let mut modified_count = 0usize;
    let mut opened: HashMap<String, (BufferId, bool)> = HashMap::new();
    for sb in &saved.buffers {
        if sb.modified {
            modified_count += 1;
        }
        if let Ok(res) = core.borrow_mut().get_or_load_buffer(Path::new(&sb.path)) {
            opened.insert(sb.path.clone(), res);
        }
    }

    // (3-5) Build + prune + install the new LOCAL view.
    let mut leaves: Vec<RestoreLeaf> = Vec::new();
    let mut save_slots: Vec<Option<WindowId>> = Vec::new();
    let active_wid = {
        let mut c = core.borrow_mut();
        let old_ids = c.views.get(&FrontendId::LOCAL).map(|v| v.layout.iter_ids());
        let Some(root) =
            build_restore_node(&mut c, &saved.root, &opened, &mut leaves, &mut save_slots)
        else {
            return modified_count; // nothing survived → keep current session
        };
        // Prune every window of the old LOCAL layout (not just scratch)
        // so none linger orphaned in `core.windows`.
        if let Some(old_ids) = old_ids {
            for id in old_ids {
                c.windows.remove(&id);
            }
        }
        let active = pick_active_window(&save_slots, saved.active_leaf)
            .or_else(|| leaves.first().map(|l| l.window));
        let Some(active) = active else {
            return modified_count;
        };
        c.active_frontend = FrontendId::LOCAL;
        c.views.insert(
            FrontendId::LOCAL,
            FrontendView {
                layout: Layout { root },
                active,
            },
        );
        active
    };

    // (5) activate-then-fire, once **per leaf** (per window). after-load
    // must observe the restored leaf as active (saveplace/recentf/syntax/
    // LSP read active state). Firing per leaf — not per buffer — gives
    // each pane its own per-window overlay (syntax attaches to the active
    // window), while LSP's `attach_buffer` is idempotent, so the same
    // file in two panes still attaches LSP once but syntax to both.
    // Writing the exact per-leaf cursor/view_top *after* the hook lets
    // desktop win over saveplace.
    //
    // NOTE (Q#DS3, hidden buffers): a restored buffer with NO leaf (open
    // but hidden) is loaded into the registry but does not fire
    // after-load here — it attaches syntax on first visit (after-switch)
    // and LSP when it is next shown/opened. Registry-only in v1.
    for leaf in &leaves {
        core.borrow_mut().set_active_window_id(leaf.window);
        fire_after_load();
        let mut c = core.borrow_mut();
        if let Some(win) = c.windows.get_mut(&leaf.window) {
            win.cursor = leaf.cursor;
            win.view_top = leaf.view_top;
        }
    }
    core.borrow_mut().set_active_window_id(active_wid);
    modified_count
}

/// Recursively rebuild a [`LayoutNode`] from a [`SavedNode`], creating a
/// `Window` in `core.windows` per surviving leaf. A leaf whose file
/// failed to open (absent from `opened`) is dropped and its split
/// collapsed — mirroring the save-side collapse.
fn build_restore_node(
    core: &mut EditorCore,
    node: &SavedNode,
    opened: &HashMap<String, (BufferId, bool)>,
    leaves: &mut Vec<RestoreLeaf>,
    save_slots: &mut Vec<Option<WindowId>>,
) -> Option<LayoutNode> {
    match node {
        SavedNode::Leaf(leaf) => {
            let Some(&(buffer_id, _newly)) = opened.get(&leaf.path) else {
                save_slots.push(None); // file missing → leaf collapses
                return None;
            };
            let text_view = {
                let reg = core.registry.borrow();
                TextView::new(reg.get(buffer_id).ok()?)
            };
            let buf_len = core
                .registry
                .borrow()
                .get(buffer_id)
                .map_or(0, crate::buffer::Buffer::len);
            let cursor = leaf.cursor.min(buf_len);
            let view_top = leaf.view_top.min(text_view.line_count().saturating_sub(1));
            let wid = WindowId::next();
            let mut win = Window::new(wid, buffer_id, text_view);
            win.cursor = cursor;
            win.view_top = view_top;
            core.windows.insert(wid, win);
            leaves.push(RestoreLeaf {
                window: wid,
                cursor,
                view_top,
            });
            save_slots.push(Some(wid));
            Some(LayoutNode::Leaf(wid))
        }
        SavedNode::Split {
            orientation,
            weights,
            children,
        } => {
            let mut kept: Vec<(LayoutNode, u32)> = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if let Some(n) = build_restore_node(core, child, opened, leaves, save_slots) {
                    kept.push((n, weights.get(i).copied().unwrap_or(1).max(1)));
                }
            }
            match kept.len() {
                0 => None,
                1 => Some(kept.into_iter().next().unwrap().0),
                _ => {
                    let (nodes, ws): (Vec<_>, Vec<_>) = kept.into_iter().unzip();
                    Some(LayoutNode::Split {
                        orientation: (*orientation).into(),
                        weights: ws,
                        children: nodes,
                    })
                }
            }
        }
    }
}

/// Resolve the focused window from the save-time `active_leaf` index
/// against the restore survivors: direct hit, else nearest surviving
/// preorder neighbor (later preferred) — Q#DS10.
fn pick_active_window(slots: &[Option<WindowId>], want: usize) -> Option<WindowId> {
    if let Some(Some(w)) = slots.get(want) {
        return Some(*w);
    }
    (0..slots.len())
        .filter(|&i| slots[i].is_some())
        .min_by_key(|&i| (i.abs_diff(want), usize::from(i < want)))
        .and_then(|i| slots[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_distinguishes_name_and_cwd() {
        let by_name = session_key(Some("work"), "/home/u/proj");
        let by_cwd = session_key(None, "/home/u/proj");
        assert!(by_name.starts_with("name."));
        assert!(by_cwd.starts_with("cwd."));
        assert_ne!(by_name, by_cwd);
        // Deterministic + charset-safe (state key validates it).
        assert_eq!(by_name, session_key(Some("work"), "/elsewhere"));
        assert!(crate::state::validate_name(&desktop_state_key(&by_name)).is_ok());
        assert!(crate::state::validate_name(&desktop_state_key(&by_cwd)).is_ok());
    }

    #[test]
    fn saved_desktop_json_round_trips() {
        let d = SavedDesktop {
            version: DESKTOP_VERSION,
            session_key: "cwd.abc".into(),
            buffers: vec![SavedBuffer {
                path: "/a.rs".into(),
                modified: true,
            }],
            root: SavedNode::Split {
                orientation: SavedOrientation::Vertical,
                weights: vec![2, 1],
                children: vec![
                    SavedNode::Leaf(SavedLeaf {
                        path: "/a.rs".into(),
                        cursor: 10,
                        view_top: 2,
                    }),
                    SavedNode::Leaf(SavedLeaf {
                        path: "/b.rs".into(),
                        cursor: 0,
                        view_top: 0,
                    }),
                ],
            },
            active_leaf: 1,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<SavedDesktop>(&json).unwrap(), d);
    }

    fn leaf(path: &str) -> SavedLeaf {
        SavedLeaf {
            path: path.into(),
            cursor: 0,
            view_top: 0,
        }
    }

    #[test]
    fn split_with_one_survivor_collapses() {
        // A 2-way split where only the first child is a file leaf →
        // collapses to that leaf, no one-child split.
        let (a, b) = (WindowId::next(), WindowId::next());
        let node = LayoutNode::Split {
            orientation: Orientation::Vertical,
            weights: vec![1, 1],
            children: vec![LayoutNode::Leaf(a), LayoutNode::Leaf(b)],
        };
        let resolve = |id: WindowId| (id == a).then(|| leaf("/a.rs"));
        let mut surviving = Vec::new();
        let saved = build_saved_node(&node, &resolve, &mut surviving).unwrap();
        assert_eq!(saved, SavedNode::Leaf(leaf("/a.rs")));
        assert_eq!(surviving, vec![a]);
    }

    #[test]
    fn active_leaf_falls_back_to_neighbor() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        // b was focused but dropped; survivors are [a, c] in preorder.
        let full = vec![(a, true), (b, false), (c, true)];
        let surviving = vec![a, c];
        // Nearest neighbor to b (pos 1) preferring later → c (index 1).
        assert_eq!(resolve_active_leaf(&full, &surviving, b), 1);
        // Direct hit still works.
        assert_eq!(resolve_active_leaf(&full, &surviving, a), 0);
    }
}
