// desktop.rs --- session desktop-save (Arc 3 phase 2).

//! Serialize the open file buffers + window layout + per-window
//! positions so a session survives a restart. Emacs `desktop.el`,
//! opt-in via `pmacs.session.desktop_mode(true)` for the in-process
//! editor.
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
//! Under a daemon (E10, D16) the desktop is an attached window's, not
//! the daemon's own `LOCAL` view's: one snapshot per daemon and
//! [`FrontendKind`], keyed by [`frontend_session_key`], written by the
//! daemon whenever a window of that kind detaches, for every attached
//! window at shutdown, and by `desktop-save` in a window, the last
//! written winning ([`save_frontend`]). A GPU window that attaches
//! naming no file, with no view retained for it, is built from the
//! semantic kind's desktop ([`restore_for_attach`], E10.2), and
//! `desktop-restore` in any window restores its kind's into it.
//!
//! Framing: docs/archive/framings/desktop-save-framing.md.

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
    /// First visible display column — horizontal scroll (Stage 4).
    ///
    /// **`#[serde(default)]` is load-bearing, not tidiness.** Nothing
    /// else in this file carries it, so without it serde would REJECT
    /// every desktop written before this field existed: a missing field
    /// is a deserialization error, not a zero. That is why no
    /// `DESKTOP_VERSION` bump is needed — and why removing this
    /// attribute would silently orphan every user's saved desktop.
    ///
    /// The reverse direction needs nothing: an older binary meets an
    /// unknown field, which serde ignores absent `deny_unknown_fields`
    /// (this file sets none).
    #[serde(default)]
    pub view_left: u32,
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

/// The kind of window attached to a daemon, the second half of the key
/// a daemon keeps a desktop under (E10, D16): the daemon's identity
/// ([`session_key`]) and this. A window of one kind never reopens what a
/// window of the other left, since the two show a desktop differently
/// (the GPU frontend shows its active window alone until the window tree
/// reaches the wire).
///
/// Decided from the negotiated projection that picks a session's render
/// state, `semantic_render`, which is the only frontend fact the attach
/// carries: the wire names no frontend kind, and E10 adds none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrontendKind {
    /// A grid session, which the daemon paints cell by cell (the
    /// attached TUI).
    Grid,
    /// A semantic session, which lays its text out itself (the GPU
    /// frontend).
    Semantic,
}

impl FrontendKind {
    /// The kind of a session that negotiated `semantic_render`.
    #[must_use]
    pub fn of_session(semantic_render: bool) -> Self {
        if semantic_render {
            Self::Semantic
        } else {
            Self::Grid
        }
    }

    /// The kind's part of a snapshot's key; also how it is named to the
    /// user.
    #[must_use]
    pub fn key_part(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Semantic => "semantic",
        }
    }
}

/// The session key a daemon keeps `kind`'s desktop under: the daemon's
/// own [`session_key`] with the kind after a dot, so it never meets the
/// in-process editor's desktop for the same directory or name, which is
/// the bare key, and passes the `pmacs.state` charset.
#[must_use]
pub fn frontend_session_key(daemon_key: &str, kind: FrontendKind) -> String {
    format!("{daemon_key}.{}", kind.key_part())
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

/// True when this process is a daemon (multi-frontend). The daemon's own
/// `LOCAL` view is never saved or restored (Q#DS9: it has a layout per
/// attached frontend and none of its own); since E10 an attached
/// window's desktop is, under its [`FrontendKind`]. This is the
/// **reliable** test — the `DaemonMode` marker is set right after the
/// daemon's `EditorState::new()`, so it is present for every
/// save/restore that can run after startup (the before-quit hook, manual
/// commands, direct binding calls), even though `init.lua` runs before
/// it is set.
fn is_daemon(lua: &Lua) -> bool {
    lua.app_data_ref::<crate::lua_bindings::DaemonMode>()
        .is_some()
}

/// The desktop session key for this process: `cwd.<hash>` in local mode,
/// and under a daemon the daemon's own identity (its `--socket NAME`, or
/// its working directory), which [`frontend_session_key`] extends with a
/// kind (D16).
fn session_key_from_lua(lua: &Lua) -> String {
    match lua
        .app_data_ref::<LocalInstanceInfo>()
        .map(|i| i.build_identity())
    {
        Some(id) => session_key(id.instance_name.as_deref(), &id.working_directory),
        None => session_key(None, ""),
    }
}

/// Whose desktop a `pmacs.session` call saves or restores, and the
/// session key it lives under: `LOCAL`'s under the session key in local
/// mode; under a daemon, the acting frontend's under its kind (E10), and
/// nobody's when the acting frontend has no kind, which is the daemon's
/// own `LOCAL`.
fn acting_target(lua: &Lua, core: &EditorCore) -> Option<(FrontendId, String)> {
    let key = session_key_from_lua(lua);
    if !is_daemon(lua) {
        return Some((FrontendId::LOCAL, key));
    }
    let fid = core.active_frontend;
    let kind = core.frontend_kinds.get(&fid)?;
    Some((fid, frontend_session_key(&key, *kind)))
}

/// Snapshot frontend `fid`'s view as a [`SavedDesktop`] (Q#DS2; E10.1
/// for an attached window, `LOCAL` in the in-process editor). `None`
/// when `fid` has no view, or no file window survives (nothing worth
/// saving).
#[must_use]
pub fn snapshot(core: &EditorCore, fid: FrontendId, session_key: String) -> Option<SavedDesktop> {
    let view = core.views.get(&fid)?;
    let focused = view.active;
    let reg = core.registry.borrow();

    let resolve = |wid: WindowId| -> Option<SavedLeaf> {
        let win = core.windows.get(&wid)?;
        // Bottom-panel arc (Q#BP10): side windows are transient display
        // policy, never desktop state. Dropping the leaf here makes the
        // existing single-surviving-child collapse remove the root
        // wrapper too, so the saved tree is the document tree exactly —
        // no `SavedLeaf` shape change and no `DESKTOP_VERSION` bump.
        if win.is_side() {
            return None;
        }
        let path = reg.get(win.buffer_id).ok()?.file_path()?;
        Some(SavedLeaf {
            path: path.display().to_string(),
            cursor: win.cursor,
            view_top: win.view_top,
            view_left: win.view_left,
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

/// Serialize the current session to the desktop state file (Q#DS1):
/// `LOCAL`'s in local mode, and under a daemon the acting window's,
/// under its kind (E10.1, `desktop-save` in a window). `Ok(false)` when
/// the state dir is unconfigured, nothing is worth saving, or under a
/// daemon the acting frontend is its own `LOCAL`; a desktop already
/// saved is left as it was. Never fails hard for the before-quit path
/// (Q#DS8) — the caller may ignore the error.
///
/// # Errors
/// A state-write / serialization failure (surfaced for manual save).
pub fn save_session(lua: &Lua) -> Result<bool, String> {
    let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
        return Ok(false);
    };
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();
    let snap = {
        let core = core.borrow();
        acting_target(lua, &core).and_then(|(fid, key)| snapshot(&core, fid, key))
    };
    let Some(snap) = snap else {
        return Ok(false);
    };
    write_snapshot(&base, &snap)?;
    Ok(true)
}

/// E10.1 --- the daemon's own write of attached window `fid`'s desktop,
/// at its detach and at shutdown: its view under its kind's key, so the
/// next fresh window of that kind reopens it, whichever window of the
/// kind was written last winning (D16's last-detached-wins). A window
/// that leaves nothing worth saving (no file window) removes its kind's
/// desktop instead, for the same reason: it is the last one written, and
/// what it had is a fresh window. `Ok(true)` when a desktop was written;
/// `Ok(false)` when one was removed or there is nothing to do (no state
/// dir, or `fid` has no kind, being no attached window).
///
/// # Errors
/// A state-write, removal or serialization failure; the daemon reports
/// it and carries on, as the before-quit save does (Q#DS8).
pub fn save_frontend(lua: &Lua, fid: FrontendId) -> Result<bool, String> {
    let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
        return Ok(false);
    };
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();
    let core = core.borrow();
    let Some(kind) = core.frontend_kinds.get(&fid).copied() else {
        return Ok(false);
    };
    let key = frontend_session_key(&session_key_from_lua(lua), kind);
    if let Some(snap) = snapshot(&core, fid, key.clone()) {
        write_snapshot(&base, &snap)?;
        return Ok(true);
    }
    crate::state::remove(&base, &desktop_state_key(&key)).map_err(|e| e.to_string())?;
    Ok(false)
}

/// Write `snap` to its key's state file.
fn write_snapshot(base: &Path, snap: &SavedDesktop) -> Result<(), String> {
    let json = serde_json::to_string(snap).map_err(|e| e.to_string())?;
    let state_key = desktop_state_key(&snap.session_key);
    crate::state::write(base, &state_key, json.as_bytes()).map_err(|e| e.to_string())
}

/// Rebuild the current session from the desktop state file (Q#DS3):
/// `LOCAL`'s in local mode, and under a daemon the acting window's, from
/// its kind's desktop (E10.2, `desktop-restore` in a window). A no-op
/// when no desktop is saved for this key / version / session
/// mismatches, or under a daemon the acting frontend is its own `LOCAL`.
/// Fires `buffer.after-load` via `lua` with each restored leaf active.
///
/// # Errors
/// Parse / state-read failures; a missing individual file collapses its
/// leaf rather than failing the whole restore.
pub fn restore_session(lua: &Lua) -> Result<(), String> {
    let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
        return Ok(());
    };
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();
    let target = acting_target(lua, &core.borrow());
    let Some((fid, key)) = target else {
        return Ok(());
    };
    let Some(saved) = read_snapshot(&base, &key)? else {
        return Ok(());
    };
    let Some(restored) = build_restored(&core, &saved) else {
        return Ok(()); // nothing survived → keep the current session
    };
    install_restored(&mut core.borrow_mut(), fid, &restored);
    settle_restored(&core, fid, &restored, || fire_after_load_hook(lua));
    warn_modified(&core, restored.modified);
    Ok(())
}

/// E10.2 --- the layout a fresh view of a window of `kind` is built
/// from (`build_fresh_frontend_view`, an attach that named no target):
/// the daemon's desktop for the kind, its buffers opened and its
/// windows made, not yet in any view. `Ok(None)` when the daemon keeps
/// no desktop for the kind, or none of its files opens.
///
/// # Errors
/// A state-read or parse failure; the daemon builds the view as if
/// there were no desktop, and says why.
pub fn restore_for_attach(lua: &Lua, kind: FrontendKind) -> Result<Option<Restored>, String> {
    let Some(base) = lua.app_data_ref::<StateDir>().map(|d| d.0.clone()) else {
        return Ok(None);
    };
    let core = lua
        .app_data_ref::<SharedCore>()
        .ok_or("no editor core")?
        .clone();
    let key = frontend_session_key(&session_key_from_lua(lua), kind);
    let Some(saved) = read_snapshot(&base, &key)? else {
        return Ok(None);
    };
    Ok(build_restored(&core, &saved))
}

/// The desktop saved under `key`, if one is, in a version this build
/// reads, and saved under that key.
fn read_snapshot(base: &Path, key: &str) -> Result<Option<SavedDesktop>, String> {
    let state_key = desktop_state_key(key);
    let Some(json) = crate::state::read(base, &state_key).map_err(|e| e.to_string())? else {
        return Ok(None); // no desktop saved
    };
    let saved: SavedDesktop = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if saved.version != DESKTOP_VERSION || saved.session_key != key {
        return Ok(None); // unrecognized / wrong session
    }
    Ok(Some(saved))
}

/// The Q#DS6 warning, when a restore brought back a file that had
/// unsaved changes at save time.
fn warn_modified(core: &SharedCore, modified: usize) {
    if modified > 0 {
        core.borrow_mut().status =
            format!("desktop restored; {modified} buffer(s) had unsaved changes when saved");
    }
}

/// One restored leaf window, carried from the tree build to the
/// activate-then-fire pass (Q#DS3).
struct RestoreLeaf {
    window: WindowId,
    cursor: u64,
    view_top: usize,
    view_left: u32,
}

/// A desktop rebuilt into the core: its buffers open and its windows in
/// `core.windows`, but in no view yet. [`install_restored`] puts it in
/// place of a frontend's existing view (the commands, and the in-process
/// editor's startup); the daemon's `build_fresh_frontend_view` makes a
/// new view of it (E10.2). Either way [`settle_restored`] then runs
/// `buffer.after-load` for each leaf.
pub struct Restored {
    root: LayoutNode,
    active: WindowId,
    leaves: Vec<RestoreLeaf>,
    modified: usize,
}

impl Restored {
    /// The restored window tree.
    #[must_use]
    pub fn layout(&self) -> Layout {
        Layout {
            root: self.root.clone(),
        }
    }

    /// The restored focus (Q#DS10's fallback applied).
    #[must_use]
    pub fn active(&self) -> WindowId {
        self.active
    }

    /// How many files this restore brought back that had unsaved
    /// changes at save time (Q#DS6). A buffer already open is not
    /// counted: its unsaved text is still in the editor, which under a
    /// daemon is the usual case (E10.2).
    #[must_use]
    pub fn modified(&self) -> usize {
        self.modified
    }
}

/// Open every buffer `saved` names (hidden ones survive) and build its
/// window tree in `core.windows`. `None` when no leaf's file opens,
/// having made no window.
///
/// Takes `&SharedCore` (not a borrow) so each load borrows and releases.
fn build_restored(core: &SharedCore, saved: &SavedDesktop) -> Option<Restored> {
    // (2) Open every buffer up front, keyed by the raw saved path. A
    // missing file is absent → its leaves collapse.
    let mut modified = 0usize;
    let mut opened: HashMap<String, (BufferId, bool)> = HashMap::new();
    for sb in &saved.buffers {
        let loaded = core.borrow_mut().get_or_load_buffer(Path::new(&sb.path));
        let already_open = matches!(loaded, Ok((_, false)));
        if sb.modified && !already_open {
            modified += 1;
        }
        if let Ok(res) = loaded {
            opened.insert(sb.path.clone(), res);
        }
    }

    // (3-4) Build the tree and its windows.
    let mut leaves: Vec<RestoreLeaf> = Vec::new();
    let mut save_slots: Vec<Option<WindowId>> = Vec::new();
    let root = build_restore_node(
        &mut core.borrow_mut(),
        &saved.root,
        &opened,
        &mut leaves,
        &mut save_slots,
    )?;
    let active = pick_active_window(&save_slots, saved.active_leaf)
        .or_else(|| leaves.first().map(|l| l.window))?;
    Some(Restored {
        root,
        active,
        leaves,
        modified,
    })
}

/// (5) Put `restored` in place of frontend `fid`'s view. Its document
/// windows go (E6d.5's rule: a window another view still shows is
/// kept); a side window stays, with the panel wrapper it sat in around
/// the restored tree, since a panel is transient display policy and no
/// part of a desktop (Q#BP10), and its terminal or listing is still
/// running. The view keeps its projection, panel capability and frame
/// geometry, which are the frontend's and not the desktop's; its
/// document viewport goes with its document. A no-op for a frontend with
/// no view.
fn install_restored(core: &mut EditorCore, fid: FrontendId, restored: &Restored) {
    let Some(old) = core.views.get(&fid).cloned() else {
        return;
    };
    let side = old
        .layout
        .side_leaf(|id| core.windows.get(&id).is_some_and(Window::is_side));
    let still_shown: std::collections::HashSet<WindowId> = core
        .views
        .iter()
        .filter(|(other, _)| **other != fid)
        .flat_map(|(_, view)| view.layout.iter_ids())
        .collect();
    for id in old.layout.iter_ids() {
        if Some(id) != side && !still_shown.contains(&id) {
            core.windows.remove(&id);
        }
    }
    let root = match (side, &old.layout.root) {
        (
            Some(side),
            LayoutNode::Split {
                orientation: Orientation::Horizontal,
                weights,
                children,
            },
        ) if children.len() == 2 && matches!(children[1], LayoutNode::Leaf(id) if id == side) => {
            LayoutNode::Split {
                orientation: Orientation::Horizontal,
                weights: weights.clone(),
                children: vec![restored.root.clone(), LayoutNode::Leaf(side)],
            }
        }
        (Some(side), _) => {
            let mut layout = restored.layout();
            layout.install_side_leaf(side);
            layout.root
        }
        (None, _) => restored.root.clone(),
    };
    core.views.insert(
        fid,
        FrontendView {
            layout: Layout { root },
            active: restored.active,
            fold_projection: old.fold_projection,
            panel_capable: old.panel_capable,
            frame_geometry: old.frame_geometry,
            panel_hidden: side.is_some() && old.panel_hidden,
            document_viewport: None,
        },
    );
}

/// (5) activate-then-fire, once **per leaf** (per window), with `fid`
/// the acting frontend and its view holding `restored`. after-load
/// must observe the restored leaf as active (saveplace/recentf/syntax/
/// LSP read active state). Firing per leaf — not per buffer — gives
/// each pane its own per-window overlay (syntax attaches to the active
/// window), while LSP's `attach_buffer` is idempotent, so the same
/// file in two panes still attaches LSP once but syntax to both.
/// Writing the exact per-leaf `cursor` and `view_top` *after* the hook lets
/// desktop win over saveplace.
///
/// NOTE (Q#DS3, hidden buffers): a restored buffer with NO leaf (open
/// but hidden) is loaded into the registry but does not fire
/// after-load here — it attaches syntax on first visit (after-switch)
/// and LSP when it is next shown/opened. Registry-only in v1.
///
/// Takes `&SharedCore` (not a borrow) so it can release the core borrow
/// around each hook fire — `buffer.after-load` re-enters `pmacs.editor.*`
/// which re-borrows the core. A leaf a hook has taken out of the view is
/// skipped, so the view's focus is always one of its own windows.
pub fn settle_restored(
    core: &SharedCore,
    fid: FrontendId,
    restored: &Restored,
    mut fire_after_load: impl FnMut(),
) {
    let shows = |core: &EditorCore, id: WindowId| {
        core.views
            .get(&fid)
            .is_some_and(|view| view.layout.iter_ids().contains(&id))
    };
    for leaf in &restored.leaves {
        {
            let mut c = core.borrow_mut();
            c.active_frontend = fid;
            if !shows(&c, leaf.window) {
                continue;
            }
            c.set_active_window_id(leaf.window);
        }
        fire_after_load();
        let mut c = core.borrow_mut();
        if let Some(win) = c.windows.get_mut(&leaf.window) {
            win.cursor = leaf.cursor;
            win.view_top = leaf.view_top;
            // Re-applied for the same reason `cursor` and `view_top`
            // are: `buffer.after-load` can move the window, and the
            // desktop is authoritative over whatever a hook (saveplace)
            // did (Q#DS3).
            win.view_left = leaf.view_left;
        }
    }
    let mut c = core.borrow_mut();
    c.active_frontend = fid;
    if shows(&c, restored.active) {
        c.set_active_window_id(restored.active);
    }
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
            // Not clamped, unlike `view_top` against the line count.
            // There is no cheap column bound (it would mean measuring
            // the widest visible line), and none is needed: the
            // horizontal follow pass moves the offset to the cursor on
            // the first frame, so a stale value from a since-shortened
            // file corrects itself rather than persisting.
            win.view_left = leaf.view_left;
            core.windows.insert(wid, win);
            leaves.push(RestoreLeaf {
                window: wid,
                cursor,
                view_top,
                view_left: leaf.view_left,
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

    /// E10 --- a daemon keeps one desktop per kind, apart from each other
    /// and from the in-process editor's for the same directory or name,
    /// under keys the state store accepts.
    #[test]
    fn a_kinds_key_is_its_own_and_valid() {
        for daemon_key in [session_key(Some("work"), "/x"), session_key(None, "/x")] {
            let grid = frontend_session_key(&daemon_key, FrontendKind::Grid);
            let semantic = frontend_session_key(&daemon_key, FrontendKind::Semantic);
            assert_ne!(grid, semantic);
            assert_ne!(grid, daemon_key);
            assert_ne!(semantic, daemon_key);
            assert!(semantic.starts_with(&daemon_key));
            assert!(crate::state::validate_name(&desktop_state_key(&grid)).is_ok());
            assert!(crate::state::validate_name(&desktop_state_key(&semantic)).is_ok());
        }
        assert_eq!(FrontendKind::of_session(true), FrontendKind::Semantic);
        assert_eq!(FrontendKind::of_session(false), FrontendKind::Grid);
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
                        view_left: 0,
                    }),
                    SavedNode::Leaf(SavedLeaf {
                        path: "/b.rs".into(),
                        cursor: 0,
                        view_top: 0,
                        view_left: 0,
                    }),
                ],
            },
            active_leaf: 1,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<SavedDesktop>(&json).unwrap(), d);
    }

    /// A desktop written **before** `view_left` existed must still load
    /// (Stage 4, framing Q#HS5 — a condition of that approval, not a
    /// nicety).
    ///
    /// This is a literal v1 document, not one produced by serializing
    /// the current struct: a generated fixture would gain the field and
    /// prove nothing. `SavedLeaf` carries no other `#[serde(default)]`,
    /// so without that attribute serde treats the missing field as an
    /// **error** and every saved desktop in the wild stops loading —
    /// which is exactly why no `DESKTOP_VERSION` bump was needed and why
    /// deleting the attribute must fail here rather than in the field.
    #[test]
    fn a_desktop_saved_before_horizontal_scroll_still_loads() {
        let v1 = r#"{
            "version": 1,
            "session_key": "cwd.abc",
            "buffers": [{"path": "/a.rs", "modified": false}],
            "root": {"Leaf": {"path": "/a.rs", "cursor": 7, "view_top": 3}},
            "active_leaf": 0
        }"#;
        let saved: SavedDesktop = serde_json::from_str(v1)
            .expect("a pre-Stage-4 desktop must load, not error on a missing field");
        let SavedNode::Leaf(leaf) = &saved.root else {
            panic!("expected a single leaf");
        };
        assert_eq!(leaf.cursor, 7, "the fields that existed are unchanged");
        assert_eq!(leaf.view_top, 3);
        assert_eq!(
            leaf.view_left, 0,
            "and the new one restores unscrolled rather than erroring"
        );
    }

    fn leaf(path: &str) -> SavedLeaf {
        SavedLeaf {
            path: path.into(),
            cursor: 0,
            view_top: 0,
            view_left: 0,
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
