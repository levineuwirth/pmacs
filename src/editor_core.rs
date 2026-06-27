// editor_core.rs --- Mutable world state shared between Rust and Lua.

//! [`EditorCore`] is the editor's world state: a buffer registry, a
//! window tree, a focused window, file metadata, and the
//! minibuffer. Lives behind a `Rc<RefCell<...>>` so the Lua-bound
//! primitives in [`crate::lua_bindings`] (`pmacs.editor.*`,
//! `pmacs.window.*`) can mutate it from inside command bodies
//! invoked through [`crate::lua::LuaHost::invoke_command`].
//!
//! # Window model (T M2.8)
//!
//! Buffers live in [`BufferRegistry`]. Each [`Window`] points at one
//! by [`BufferId`] and owns its own cursor / view-top / goal-column /
//! [`TextView`]. The [`Layout`] tree maps the cell grid to per-window
//! viewport rectangles. A single [`WindowId`] is "active": every
//! `pmacs.editor.*` primitive operates on it; cursor and edits in
//! the run loop dispatch through it.
//!
//! When the active buffer mutates, [`EditorCore::apply_active_edit`]
//! notifies *every* window whose `buffer_id` matches the active
//! window's --- two windows on the same buffer keep their layout
//! caches synchronized.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, BufferId, EditOp};
use crate::file_io::{FileMeta, save_atomic};
use crate::lua_bindings::SharedRegistry;
use crate::minibuffer::Minibuffer;
use crate::protocol::FrontendId;
use crate::rope::Edit;
use crate::rope::{Position, Range};
use crate::text_view::TextView;
use crate::view::{DisplayCoord, View};
use crate::window::{FrontendView, Layout, Orientation, Window, WindowId};

/// T M10.10 post-audit-round-3 F16 — origin of a queued CRDT op.
///
/// Records **whether the originating frontend already applied the
/// op to its local mirror**, which determines whether the broadcast
/// sweep should exclude that frontend.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum CrdtOpOrigin {
    /// A replica frontend's `FrontendEvent::CrdtOp` path applied the
    /// op to its local mirror before sending. Broadcast must exclude
    /// that frontend (it would double-apply otherwise — see
    /// `BufferMirror::apply_local_insert` /
    /// `apply_local_delete` and `optimistic::apply_incoming_crdt_op`'s
    /// echo-skip rule).
    OptimisticReplica(FrontendId),
    /// Daemon-side mutation (a `FrontendEvent::Key` round-trip, a
    /// Lua-driven edit, a fallback path) generated the op. No
    /// frontend has applied it locally; broadcast to every replica
    /// frontend, including the one whose `Key` event drove the
    /// daemon path (its mirror is otherwise stale).
    DaemonKey,
}

/// The world state mutated by editor commands.
pub struct EditorCore {
    /// Shared buffer registry. The registry is the canonical owner
    /// of every buffer; windows reference buffers by [`BufferId`].
    pub registry: SharedRegistry,
    /// All windows, keyed by id for stable iteration. `WindowId`s
    /// are globally unique across all frontends; each
    /// [`FrontendView`] in `views` references a subset via its
    /// `Layout`.
    pub windows: BTreeMap<WindowId, Window>,
    /// T M10.8 — per-frontend views. Each attached frontend has its
    /// own `Layout` (split tree) + `active: WindowId`. Buffers are
    /// shared via `registry`; cursors / `view_top`s live in the
    /// per-frontend `Window` instances.
    ///
    /// Invariant: `FrontendId::LOCAL` always has an entry. The
    /// in-process editor uses this view; daemon-attached frontends
    /// register additional entries on attach (M10.8 Day 3 wires
    /// the per-attach registration via the dispatcher; Day 2 ships
    /// a fallback-to-LOCAL accessor so single-frontend tests pass
    /// before per-attach registration lands).
    pub views: HashMap<FrontendId, FrontendView>,
    /// One-line message shown in the status line.
    pub status: String,
    /// True iff the editor should exit at the next iteration.
    pub quit: bool,
    /// Universal minibuffer (T M2.7).
    pub minibuffer: Minibuffer,
    /// The frontend that produced the most recent input event
    /// dispatched to this core (T M5.4). v0.1 has a single frontend
    /// per instance, so this stays at [`FrontendId::LOCAL`] in
    /// practice; the field is load-bearing for v0.3 multi-frontend
    /// (multi-window, multi-user) where each input event must be
    /// attributable to its source frontend.
    pub active_frontend: FrontendId,
    /// T M10.8 Day 4 — pending CRDT ops queue.
    ///
    /// Each [`CrdtOpOrigin`] entry records both **what** to broadcast
    /// and **who already applied it locally** (the sender-exclusion
    /// signal). The dispatcher drains the queue per-tick and
    /// broadcasts each op to multi-frontend sessions with
    /// `crdt_replica` negotiated.
    ///
    /// # M10.10 post-audit-round-3 F16: origin tagging
    ///
    /// Sender exclusion depends on **whether the originating
    /// frontend already applied the op to its local mirror**:
    ///
    /// - [`CrdtOpOrigin::OptimisticReplica`] — a replica frontend's
    ///   `FrontendEvent::CrdtOp` path applied the op to its mirror
    ///   before sending. Broadcast must exclude that frontend so it
    ///   doesn't double-apply.
    /// - [`CrdtOpOrigin::DaemonKey`] — daemon-side mutation (a
    ///   `FrontendEvent::Key` round-trip, a Lua-driven edit, etc.)
    ///   generated the op. No frontend's mirror has applied it
    ///   locally; broadcast must include every replica frontend
    ///   *including* the active one. Without this, the
    ///   active frontend's mirror would silently drift from daemon
    ///   state after every fallback / Key-path edit.
    pub pending_crdt_ops: Vec<(CrdtOpOrigin, BufferId, crate::rope::CrdtOp)>,
    /// T M4.5 L1 — bounded jump ring. Cross-file navigation
    /// (`go-to-definition`, references, symbol jumps) pushes the
    /// pre-jump `(BufferId, Position)` here before moving the cursor;
    /// `M-,` (`jump_back`) pops the most recent entry and restores
    /// it. Bounded at [`Self::JUMP_RING_CAP`]: the oldest entry is
    /// evicted when full, so a long navigation session can't grow
    /// this without limit. Entries naming a now-removed buffer are
    /// skipped on pop (stale-handle safe, mirrors the registry's
    /// `Missing` contract).
    pub jump_ring: Vec<(BufferId, Position)>,
}

impl EditorCore {
    /// A fresh core with one window on a `*scratch*` buffer.
    #[must_use]
    pub fn new(registry: SharedRegistry) -> Self {
        let buffer_id = registry.borrow_mut().create("*scratch*");
        let text_view = {
            let r = registry.borrow();
            let buf = r.get(buffer_id).expect("just-created scratch buffer");
            TextView::new(buf)
        };
        let id = WindowId::next();
        let window = Window::new(id, buffer_id, text_view);
        let mut windows = BTreeMap::new();
        windows.insert(id, window);
        let mut views = HashMap::new();
        views.insert(
            FrontendId::LOCAL,
            FrontendView {
                layout: Layout::single(id),
                active: id,
            },
        );
        Self {
            registry,
            windows,
            views,
            status: String::new(),
            quit: false,
            minibuffer: Minibuffer::new(),
            active_frontend: FrontendId::LOCAL,
            pending_crdt_ops: Vec::new(),
            jump_ring: Vec::new(),
        }
    }

    /// Build a core from raw bytes under `name`. Used by tests.
    /// Replaces the scratch buffer's content; the active window is
    /// retained.
    #[must_use]
    pub fn from_bytes(registry: SharedRegistry, name: impl Into<String>, bytes: &[u8]) -> Self {
        let mut core = Self::new(registry);
        let id = core.active_window().buffer_id;
        let new_id = {
            let mut reg = core.registry.borrow_mut();
            let new_id = reg.create_from_bytes(name, bytes);
            // Replace the active window's buffer with the new one.
            let _ = reg.remove(id);
            new_id
        };
        let text_view = {
            let reg = core.registry.borrow();
            TextView::new(reg.get(new_id).unwrap())
        };
        let aw = core.active_window_mut();
        aw.buffer_id = new_id;
        aw.text_view = text_view;
        aw.cursor = 0;
        aw.view_top = 0;
        aw.goal_col = None;
        core
    }

    // ---- accessors ---------------------------------------------------------

    /// T M10.8 — the active frontend's view (layout + active window).
    ///
    /// **Day 2 transitional behavior**: if `active_frontend` has no
    /// registered view (the daemon-attached frontend case before Day
    /// 3's dispatcher refactor wires `register_frontend_view`), fall
    /// back to `FrontendId::LOCAL`'s view. The invariant "LOCAL
    /// always has a view" is enforced by the constructor.
    #[must_use]
    pub fn active_view(&self) -> &FrontendView {
        self.views.get(&self.active_frontend).unwrap_or_else(|| {
            self.views.get(&FrontendId::LOCAL).expect(
                "invariant: FrontendId::LOCAL always has a registered FrontendView; \
                 populated by EditorCore::new and never removed",
            )
        })
    }

    /// Mutable view of the active frontend's [`FrontendView`].
    ///
    /// Same fallback semantics as [`active_view`].
    pub fn active_view_mut(&mut self) -> &mut FrontendView {
        // Choose the key first to avoid borrowing `self.views`
        // twice with overlapping lifetimes (the fallback path).
        let key = if self.views.contains_key(&self.active_frontend) {
            self.active_frontend
        } else {
            FrontendId::LOCAL
        };
        self.views.get_mut(&key).expect(
            "invariant: FrontendId::LOCAL always has a registered FrontendView; \
             populated by EditorCore::new and never removed",
        )
    }

    /// The active frontend's window-split tree.
    #[must_use]
    pub fn active_layout(&self) -> &Layout {
        &self.active_view().layout
    }

    /// Mutable access to the active frontend's window-split tree.
    pub fn active_layout_mut(&mut self) -> &mut Layout {
        &mut self.active_view_mut().layout
    }

    /// `WindowId` of the active frontend's focused window.
    #[must_use]
    pub fn active_window_id(&self) -> WindowId {
        self.active_view().active
    }

    /// Set the active frontend's focused window.
    pub fn set_active_window_id(&mut self, id: WindowId) {
        self.active_view_mut().active = id;
    }

    /// Reference the active [`Window`] — the window currently
    /// focused in the active frontend's view.
    #[must_use]
    pub fn active_window(&self) -> &Window {
        let id = self.active_window_id();
        self.windows
            .get(&id)
            .expect("active window present in core.windows")
    }

    /// Mutably reference the active [`Window`].
    pub fn active_window_mut(&mut self) -> &mut Window {
        let id = self.active_window_id();
        self.windows
            .get_mut(&id)
            .expect("active window present in core.windows")
    }

    /// Reference a specific frontend's active [`Window`].
    ///
    /// Returns `None` if `fid` has no registered view (no fallback —
    /// callers explicitly asking about a specific frontend get a
    /// truthful answer about whether that frontend has state).
    #[must_use]
    pub fn active_window_for(&self, fid: FrontendId) -> Option<&Window> {
        let view = self.views.get(&fid)?;
        self.windows.get(&view.active)
    }

    /// Mutably reference a specific frontend's active [`Window`].
    pub fn active_window_mut_for(&mut self, fid: FrontendId) -> Option<&mut Window> {
        let win_id = self.views.get(&fid)?.active;
        self.windows.get_mut(&win_id)
    }

    /// T M10.8 — register a `FrontendView` for `fid`. Called by the
    /// daemon on attach (Day 3 dispatcher work). Day 2's fallback
    /// path makes this optional; Day 3 makes it required.
    pub fn register_frontend_view(&mut self, fid: FrontendId, view: FrontendView) {
        self.views.insert(fid, view);
    }

    /// T M10.8 — drop a frontend's view on detach. The frontend's
    /// windows remain in `self.windows` until explicit cleanup (M10.x
    /// may add per-detach window pruning); for M10.8 they're
    /// orphaned but accessible by id (matches v0.1 behavior where
    /// closing a window left others intact).
    pub fn unregister_frontend_view(&mut self, fid: FrontendId) {
        self.views.remove(&fid);
    }

    /// [`BufferId`] of the active window's buffer.
    #[must_use]
    pub fn active_buffer_id(&self) -> BufferId {
        self.active_window().buffer_id
    }

    /// Path bound to the active window's buffer, if any. T M4.5 L1:
    /// replaces the old `EditorCore.file_path` field — it now lives
    /// per-buffer so cross-file navigation keeps each buffer's
    /// identity straight.
    #[must_use]
    pub fn active_buffer_path(&self) -> Option<PathBuf> {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .ok()
            .and_then(|b| b.file_path().map(Path::to_path_buf))
    }

    /// Filesystem metadata recorded for the active window's buffer.
    #[must_use]
    pub fn active_file_meta(&self) -> Option<FileMeta> {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .ok()
            .and_then(|b| b.file_meta().cloned())
    }

    /// Bind a path (and clear metadata) on a specific buffer. Used by
    /// file open / `pmacs.buffer.from_file`.
    ///
    /// The path is normalized to an absolute, lexically-clean form
    /// first ([`normalize_buffer_path`]). This is the single seam
    /// every buffer identity flows through (CLI open, Lua find-file,
    /// `WorkspaceEdit` rename ops), so doing it here keeps the invariant
    /// "a buffer's `file_path` is always absolute" — which the LSP
    /// layer relies on to build a resolvable `file:///…` URI (a
    /// relative or `~`-prefixed path produced `file://ipc.cpp`, which
    /// clangd rejected with `-32602 unresolvable URI`) and which
    /// cross-file navigation relies on for buffer-identity matching.
    pub fn set_buffer_path(&mut self, id: BufferId, path: Option<PathBuf>) {
        let path = path.map(normalize_buffer_path);
        if let Ok(b) = self.registry.borrow_mut().get_mut(id) {
            b.set_file_path(path);
        }
    }

    /// Record filesystem metadata on a specific buffer.
    pub fn set_buffer_meta(&mut self, id: BufferId, meta: Option<FileMeta>) {
        if let Ok(b) = self.registry.borrow_mut().get_mut(id) {
            b.set_file_meta(meta);
        }
    }

    /// Cursor of the active window (compatibility shim for callers
    /// migrated from pre-M2.8 code).
    #[must_use]
    pub fn cursor(&self) -> Position {
        self.active_window().cursor
    }

    /// `view_top` of the active window.
    #[must_use]
    pub fn view_top(&self) -> usize {
        self.active_window().view_top
    }

    /// Active buffer's byte length.
    #[must_use]
    pub fn active_buffer_len(&self) -> u64 {
        let id = self.active_buffer_id();
        self.registry.borrow().get(id).map_or(0, Buffer::len)
    }

    /// Active buffer's name. Returns an owned String to release the
    /// registry borrow promptly.
    #[must_use]
    pub fn active_buffer_name(&self) -> String {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .map(|b| b.name().to_owned())
            .unwrap_or_default()
    }

    /// Returns true iff the active buffer has unsaved modifications.
    #[must_use]
    pub fn active_buffer_is_modified(&self) -> bool {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .is_ok_and(Buffer::is_modified)
    }

    /// 0-based line index containing the active window's cursor.
    #[must_use]
    pub fn cursor_line(&self) -> usize {
        let aw = self.active_window();
        aw.text_view.line_at_offset(aw.cursor)
    }

    /// Move the active window's cursor to the start of a 0-based line.
    /// Out-of-range line numbers clamp to the last line.
    pub fn move_to_line(&mut self, line: usize) {
        let line_count = self.active_window().text_view.line_count().max(1);
        let target_line = line.min(line_count - 1);
        let target = self
            .active_window()
            .text_view
            .line_offset(target_line)
            .unwrap_or_else(|| self.active_buffer_len());
        let aw = self.active_window_mut();
        aw.cursor = target;
        aw.goal_col = None;
    }

    // ---- jump ring (T M4.5 L1) ---------------------------------------------

    /// Bound on [`Self::jump_ring`]. Large enough for a deep
    /// cross-file dig (definition → definition → references …),
    /// small enough that a stuck loop can't grow memory unbounded.
    pub const JUMP_RING_CAP: usize = 64;

    /// Record the active window's current `(buffer, cursor)` as a
    /// jump origin. Call this *before* moving the cursor on a
    /// navigation action (go-to-definition, references, symbol jump)
    /// so `M-,` can return here.
    ///
    /// When the ring is at [`Self::JUMP_RING_CAP`], the oldest
    /// origin is evicted (front drop) — the user keeps the most
    /// recent trail, which is the one they're likely to unwind.
    pub fn push_jump(&mut self) {
        let entry = (self.active_buffer_id(), self.cursor());
        if self.jump_ring.len() >= Self::JUMP_RING_CAP {
            self.jump_ring.remove(0);
        }
        self.jump_ring.push(entry);
    }

    /// Pop the most recent jump origin and move there. Returns
    /// `true` if a jump was performed.
    ///
    /// Stale entries — a recorded buffer that has since been removed
    /// from the registry — are skipped (the loop keeps popping until
    /// it finds a live target or the ring empties), so a jump-back
    /// never lands on a missing buffer. The restored cursor is
    /// clamped to the (possibly now shorter) buffer length.
    pub fn jump_back(&mut self) -> bool {
        while let Some((bid, pos)) = self.jump_ring.pop() {
            if !self.registry.borrow().contains(bid) {
                continue;
            }
            if self.active_buffer_id() != bid && self.switch_active_buffer(bid).is_err() {
                continue;
            }
            let clamped = pos.min(self.active_buffer_len());
            let aw = self.active_window_mut();
            aw.cursor = clamped;
            aw.goal_col = None;
            return true;
        }
        false
    }

    // ---- editing primitives ------------------------------------------------

    /// Apply `op` to the active buffer; notify every window
    /// displaying that buffer. Returns the new buffer length.
    ///
    /// # Errors
    ///
    /// Returns a stringified error on buffer or view failure.
    pub fn apply_active_edit(&mut self, op: EditOp<'_>) -> Result<u64, String> {
        let buffer_id = self.active_buffer_id();
        let mut reg = self.registry.borrow_mut();
        let buffer = reg.get_mut(buffer_id).map_err(|e| e.to_string())?;
        let edit = buffer.apply_edit(op).map_err(|e| e.to_string())?;
        for win in self.windows.values_mut() {
            if win.buffer_id == buffer_id {
                let _ = win.text_view.on_edit(buffer, &edit);
                for overlay in &mut win.overlays {
                    let _ = overlay.on_edit(buffer, &edit);
                }
            }
        }
        // T M10.8 Day 4 — capture CRDT op (if the buffer was in
        // CRDT mode and produced one) for the dispatcher to
        // broadcast on the next tick.
        //
        // M10.10 post-audit-round-3 F16: this is the **daemon-side**
        // mutation path (e.g. `FrontendEvent::Key` round-trip,
        // Lua-driven edit, fallback). The source frontend's mirror
        // has NOT applied this op locally; the queued origin is
        // [`CrdtOpOrigin::DaemonKey`] so the broadcast sweep includes
        // every replica (no sender exclusion).
        if let Some(crdt_op) = edit.crdt_op.as_ref() {
            self.pending_crdt_ops
                .push((CrdtOpOrigin::DaemonKey, buffer_id, (**crdt_op).clone()));
        }
        Ok(edit.new_rope.len())
    }

    /// Notify every window displaying `buffer_id` that the buffer was
    /// just edited externally — used by code paths that mutate a buffer
    /// without going through [`Self::apply_active_edit`] (the most
    /// notable one being [`crate::lua::LuaHost::append_to_errors_buffer`],
    /// which writes to `*errors*` from inside Lua callbacks).
    ///
    /// Without this notification, any window currently displaying the
    /// edited buffer would keep a stale [`crate::text_view::TextView`]
    /// line cache, causing later cursor motions to land at offsets the
    /// view cannot map back to display coordinates.
    pub fn notify_buffer_edit(&mut self, buffer_id: BufferId, edit: &Edit) {
        let reg = self.registry.borrow();
        let Ok(buffer) = reg.get(buffer_id) else {
            return;
        };
        for win in self.windows.values_mut() {
            if win.buffer_id == buffer_id {
                let _ = win.text_view.on_edit(buffer, edit);
                for overlay in &mut win.overlays {
                    let _ = overlay.on_edit(buffer, edit);
                }
            }
        }
    }

    /// Force every window currently showing `buffer_id` to rebuild
    /// its [`TextView`] from scratch.
    ///
    /// Used by code paths that rewrite a buffer end-to-end without
    /// emitting a useful [`Edit`] (the help renderer issues a
    /// delete-all + insert pair on `*help*`; `*buffer-list*` is
    /// regenerated from scratch on every C-x C-b). Calling
    /// [`Self::notify_buffer_edit`] for each step works but is more
    /// fiddly; rebuild is simpler and still O(buffer length) which is
    /// what an end-to-end rewrite cost anyway.
    ///
    /// Cursor and `view_top` are clamped to the new buffer extent so
    /// they don't dangle past the end after a shrinking rewrite.
    pub fn rebuild_views_for(&mut self, buffer_id: BufferId) {
        let reg = self.registry.borrow();
        let Ok(buffer) = reg.get(buffer_id) else {
            return;
        };
        let len = buffer.len();
        for win in self.windows.values_mut() {
            if win.buffer_id == buffer_id {
                win.text_view = TextView::new(buffer);
                if win.cursor > len {
                    win.cursor = len;
                }
                let max_top = win.text_view.line_count().saturating_sub(1);
                if win.view_top > max_top {
                    win.view_top = max_top;
                }
            }
        }
    }

    /// Save the active buffer to its backing file. Returns `true` on
    /// successful write; `false` if no path is associated, the buffer
    /// could not be read, or the atomic save failed. Callers (the
    /// `buffer.save` Lua command) use the return value to gate
    /// `buffer.after-save` firing.
    pub fn save(&mut self) -> bool {
        let id = self.active_buffer_id();
        let Some(path) = self.active_buffer_path() else {
            self.status = "no file (M1: open a file from argv)".into();
            return false;
        };
        let len_and_bytes = {
            let reg = self.registry.borrow();
            let buffer = match reg.get(id) {
                Ok(b) => b,
                Err(e) => {
                    self.status = format!("save failed: {e}");
                    return false;
                }
            };
            let len = buffer.len();
            let mut content = vec![0u8; len as usize];
            if len > 0 {
                buffer.snapshot_rope().slice(0, len, &mut content);
            }
            (len, content)
        };
        let (_, content) = len_and_bytes;
        match save_atomic(&path, &content) {
            Ok(meta) => {
                if let Ok(buf) = self.registry.borrow_mut().get_mut(id) {
                    buf.set_file_meta(Some(meta));
                    buf.mark_clean();
                }
                self.status = format!("saved {}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("save failed: {e}");
                false
            }
        }
    }

    /// Move the cursor by one codepoint to the left. No-op at start.
    pub fn move_left(&mut self) {
        let cursor = self.active_window().cursor;
        if cursor == 0 {
            self.active_window_mut().goal_col = None;
            return;
        }
        let new = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            prev_codepoint(buffer, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor by one codepoint to the right. No-op at end.
    pub fn move_right(&mut self) {
        let cursor = self.active_window().cursor;
        let new = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            if cursor >= buffer.len() {
                cursor
            } else {
                next_codepoint(buffer, cursor)
            }
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor up one line, preserving display column.
    pub fn move_up(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let goal_col = self.active_window().goal_col;
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let coord = self
                .active_window()
                .text_view
                .pos_to_display(buffer, cursor)
                .unwrap_or_default();
            if coord.row == 0 {
                return;
            }
            let goal = goal_col.unwrap_or(coord.col);
            let target = DisplayCoord::new(coord.row - 1, goal);
            let new_pos = self
                .active_window()
                .text_view
                .display_to_pos(buffer, target);
            (goal, new_pos)
        };
        let (goal, new_pos) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
    }

    /// Move the cursor down one line, preserving display column.
    pub fn move_down(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let goal_col = self.active_window().goal_col;
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let coord = self
                .active_window()
                .text_view
                .pos_to_display(buffer, cursor)
                .unwrap_or_default();
            let next_row = coord.row + 1;
            if (next_row as usize) >= self.active_window().text_view.line_count() {
                return;
            }
            let goal = goal_col.unwrap_or(coord.col);
            let target = DisplayCoord::new(next_row, goal);
            let new_pos = self
                .active_window()
                .text_view
                .display_to_pos(buffer, target);
            (goal, new_pos)
        };
        let (goal, new_pos) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
    }

    /// Move to the start of the current line.
    pub fn move_line_start(&mut self) {
        let cursor = self.active_window().cursor;
        let new = {
            let aw = self.active_window();
            let line = aw.text_view.line_at_offset(cursor);
            aw.text_view.line_offset(line).unwrap_or(cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor forward by one word.
    ///
    /// Skips runs of non-word characters, then a run of word
    /// characters. Word characters are alphanumerics plus `_`, the
    /// Emacs default. Multi-byte characters are handled correctly:
    /// `is_word` runs after a full UTF-8 codepoint is decoded.
    pub fn move_word_right(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            forward_word(buffer, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor backward by one word. Mirror of
    /// [`Self::move_word_right`].
    pub fn move_word_left(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            backward_word(buffer, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Select the word at the active cursor. Returns `false` when the
    /// cursor is not on a word character.
    pub fn select_word_at_cursor(&mut self) -> bool {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let range = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else {
                return false;
            };
            word_range_at(buffer, cursor)
        };
        let Some((start, end)) = range else {
            return false;
        };
        let aw = self.active_window_mut();
        aw.selection = Some(crate::window::Selection { anchor: start });
        aw.cursor = end;
        aw.goal_col = None;
        true
    }

    /// Select the whole line at the active cursor, trailing newline
    /// included — the convention that makes consecutive triple-click
    /// lines abut (Q#M4). The cursor lands at the selection end (the
    /// start of the next line). No-op when the buffer is gone.
    pub fn select_line_at_cursor(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let (start, end) = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else {
                return;
            };
            let view = &self.active_window().text_view;
            let line = view.line_at_offset(cursor);
            let start = view.line_offset(line).unwrap_or(0);
            let end = view.line_offset(line + 1).unwrap_or_else(|| buffer.len());
            (start, end)
        };
        let aw = self.active_window_mut();
        aw.selection = Some(crate::window::Selection { anchor: start });
        aw.cursor = end;
        aw.goal_col = None;
    }

    /// Move the cursor forward to the next paragraph break.
    ///
    /// A paragraph break is a blank line (empty or whitespace-only).
    /// If the cursor is currently in a paragraph, the cursor lands at
    /// the start of the first blank line after it. If the cursor is
    /// already on a blank line, blanks are skipped first, then the
    /// next blank line is found. Lands at the end of the buffer when
    /// there are no further paragraph breaks. Mirrors GNU Emacs's
    /// (and Doom's) `forward-paragraph` semantics.
    pub fn move_paragraph_down(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            forward_paragraph(buffer, &aw.text_view, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor backward to the previous paragraph break.
    /// Mirror of [`Self::move_paragraph_down`].
    pub fn move_paragraph_up(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            backward_paragraph(buffer, &aw.text_view, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor down by approximately one screenful, scrolling
    /// `view_top` to match. The step is the active window's last
    /// rendered viewport height minus one (so the user keeps a line
    /// of context); falls back to a sane default before the first
    /// frame has rendered.
    pub fn move_page_down(&mut self) {
        let step = self.page_step();
        let cursor = self.active_window().cursor;
        let view_top = self.active_window().view_top;
        let id = self.active_buffer_id();
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let coord = aw
                .text_view
                .pos_to_display(buffer, cursor)
                .unwrap_or_default();
            let max_line = aw.text_view.line_count().saturating_sub(1) as u32;
            let goal_col = aw.goal_col.unwrap_or(coord.col);
            let target_row = (coord.row + step).min(max_line);
            let target = DisplayCoord::new(target_row, goal_col);
            let new_pos = aw.text_view.display_to_pos(buffer, target);
            (goal_col, new_pos, view_top.saturating_add(step as usize))
        };
        let (goal, new_pos, new_top) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
        // Also nudge view_top; render's scroll-into-view will clamp
        // and align further if needed.
        let max_top = aw.text_view.line_count().saturating_sub(1);
        aw.view_top = new_top.min(max_top);
    }

    /// Move the cursor up by approximately one screenful. Mirror of
    /// [`Self::move_page_down`].
    pub fn move_page_up(&mut self) {
        let step = self.page_step();
        let cursor = self.active_window().cursor;
        let view_top = self.active_window().view_top;
        let id = self.active_buffer_id();
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let coord = aw
                .text_view
                .pos_to_display(buffer, cursor)
                .unwrap_or_default();
            let goal_col = aw.goal_col.unwrap_or(coord.col);
            let target_row = coord.row.saturating_sub(step);
            let target = DisplayCoord::new(target_row, goal_col);
            let new_pos = aw.text_view.display_to_pos(buffer, target);
            (goal_col, new_pos, view_top.saturating_sub(step as usize))
        };
        let (goal, new_pos, new_top) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
        aw.view_top = new_top;
    }

    /// Number of lines a "page" advances. Uses the active window's
    /// last rendered viewport height minus one (one line of context
    /// at the seam, like Emacs's `next-screen-context-lines`),
    /// clamped to a sensible default for headless tests where no
    /// frame has rendered.
    fn page_step(&self) -> u32 {
        const DEFAULT_PAGE: u32 = 20;
        let rows = self.active_window().last_visible_rows;
        if rows >= 2 { rows - 1 } else { DEFAULT_PAGE }
    }

    /// Move to the end of the current line (before any trailing newline).
    pub fn move_line_end(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let line = aw.text_view.line_at_offset(cursor);
            let Some(start) = aw.text_view.line_offset(line) else {
                return;
            };
            let len = aw.text_view.line_len(buffer, line).unwrap_or(0);
            start + len
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Insert a single character at the cursor.
    pub fn insert_char(&mut self, ch: char) {
        self.active_window_mut().goal_col = None;
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let bytes = s.as_bytes();
        let pos = self.active_window().cursor;
        if let Err(e) = self.apply_active_edit(EditOp::Insert { pos, bytes }) {
            self.status = format!("insert failed: {e}");
            return;
        }
        self.active_window_mut().cursor += bytes.len() as u64;
    }

    /// CUA type-over: insert `ch`, replacing the active region if one
    /// exists. With a region this is a *single* `EditOp::Replace` — one
    /// undo step — rather than the former `delete_region()` +
    /// `insert_char()` pair, which recorded two. With no region it
    /// delegates to [`Self::insert_char`] (a plain insert). The cursor
    /// lands just past the inserted bytes and any selection is cleared.
    pub fn insert_char_over_region(&mut self, ch: char) {
        let Some((lo, hi)) = self.active_region() else {
            self.insert_char(ch);
            return;
        };
        self.active_window_mut().goal_col = None;
        let mut buf = [0u8; 4];
        let bytes = ch.encode_utf8(&mut buf).as_bytes();
        if let Err(e) = self.apply_active_edit(EditOp::Replace {
            range: Range { start: lo, end: hi },
            bytes,
        }) {
            self.status = format!("replace failed: {e}");
            return;
        }
        let aw = self.active_window_mut();
        aw.cursor = lo + bytes.len() as u64;
        aw.selection = None;
    }

    /// Delete the codepoint immediately before the cursor.
    pub fn backspace(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        if cursor == 0 {
            return;
        }
        let prev = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            prev_codepoint(buffer, cursor)
        };
        let range = Range::new(prev, cursor);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
            return;
        }
        self.active_window_mut().cursor = prev;
    }

    /// Delete the codepoint at the cursor (forward delete).
    pub fn delete_forward(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        let id = self.active_buffer_id();
        let next = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            if cursor >= buffer.len() {
                return;
            }
            next_codepoint(buffer, cursor)
        };
        let range = Range::new(cursor, next);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
        }
    }

    /// Delete from the cursor backward to the start of the previous
    /// word. The CUA-style `Ctrl+Backspace`. No-op at start-of-buffer.
    /// Mirrors [`Self::backspace`] but the deleted range is the gap
    /// between the cursor and where [`Self::move_word_left`] would
    /// land.
    pub fn delete_word_backward(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        if cursor == 0 {
            return;
        }
        let new = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            backward_word(buffer, cursor)
        };
        if new == cursor {
            return;
        }
        let range = Range::new(new, cursor);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
            return;
        }
        self.active_window_mut().cursor = new;
    }

    /// Delete from the cursor forward to the end of the next word. The
    /// CUA-style `Ctrl+Delete`. No-op at end-of-buffer. Mirrors
    /// [`Self::delete_forward`] over the gap from the cursor to where
    /// [`Self::move_word_right`] would land.
    pub fn delete_word_forward(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        let id = self.active_buffer_id();
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            if cursor >= buffer.len() {
                return;
            }
            forward_word(buffer, cursor)
        };
        if new == cursor {
            return;
        }
        let range = Range::new(cursor, new);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
        }
    }

    /// Undo the most recent edit on the active buffer; clamp the
    /// active window's cursor to the new length and notify all
    /// windows on this buffer.
    pub fn undo(&mut self) {
        self.active_window_mut().goal_col = None;
        let buffer_id = self.active_buffer_id();
        let edit = {
            let mut reg = self.registry.borrow_mut();
            let Ok(buffer) = reg.get_mut(buffer_id) else {
                return;
            };
            buffer.undo()
        };
        match edit {
            Ok(edit) => {
                let reg = self.registry.borrow();
                let Ok(buffer) = reg.get(buffer_id) else {
                    return;
                };
                for win in self.windows.values_mut() {
                    if win.buffer_id == buffer_id {
                        let _ = win.text_view.on_edit(buffer, &edit);
                        let max = buffer.len();
                        if win.cursor > max {
                            win.cursor = max;
                        }
                    }
                }
                drop(reg);
                // Post-audit-round-5 F27: undo on a CRDT-backed
                // buffer produces a crdt_op that must broadcast to
                // every replica frontend (including the one whose
                // command triggered the undo — its BufferMirror has
                // no other way to converge with the post-undo state).
                self.queue_daemon_origin_crdt_op(buffer_id, &edit);
            }
            Err(_) => self.status = "nothing to undo".into(),
        }
    }

    /// Redo the most recently undone edit on the active buffer.
    pub fn redo(&mut self) {
        self.active_window_mut().goal_col = None;
        let buffer_id = self.active_buffer_id();
        let edit = {
            let mut reg = self.registry.borrow_mut();
            let Ok(buffer) = reg.get_mut(buffer_id) else {
                return;
            };
            buffer.redo()
        };
        match edit {
            Ok(edit) => {
                let reg = self.registry.borrow();
                let Ok(buffer) = reg.get(buffer_id) else {
                    return;
                };
                for win in self.windows.values_mut() {
                    if win.buffer_id == buffer_id {
                        let _ = win.text_view.on_edit(buffer, &edit);
                        let max = buffer.len();
                        if win.cursor > max {
                            win.cursor = max;
                        }
                    }
                }
                drop(reg);
                // Post-audit-round-5 F27 — same as undo above.
                self.queue_daemon_origin_crdt_op(buffer_id, &edit);
            }
            Err(_) => self.status = "nothing to redo".into(),
        }
    }

    /// T M10.10 post-audit-round-5 F27 + F28 — queue a CRDT op
    /// produced by a daemon-origin edit (undo/redo via core, Lua
    /// bindings, command pipeline) for broadcast.
    ///
    /// Pushes into `pending_crdt_ops` with
    /// [`CrdtOpOrigin::DaemonKey`] semantics: the broadcast sweep
    /// includes every replica frontend (no sender exclusion). The
    /// originating frontend's `BufferMirror` has not applied the op
    /// locally — only the daemon's authoritative buffer has — so
    /// the source's mirror needs the broadcast just like every
    /// other replica.
    ///
    /// No-op when the edit doesn't carry a `crdt_op` (the buffer
    /// wasn't CRDT-backed at the time of the edit). Callers can
    /// invoke this unconditionally after any daemon-origin
    /// `apply_*` that returns an `Edit`; non-CRDT buffers pay no
    /// cost beyond the early return.
    pub fn queue_daemon_origin_crdt_op(&mut self, buffer_id: BufferId, edit: &Edit) {
        if let Some(crdt_op) = edit.crdt_op.as_ref() {
            self.pending_crdt_ops
                .push((CrdtOpOrigin::DaemonKey, buffer_id, (**crdt_op).clone()));
        }
    }

    // ---- window operations -------------------------------------------------

    /// Split the active window. Returns the new window's id.
    /// `same_buffer` controls whether the new window opens on the
    /// active buffer (Emacs default) or a fresh `*scratch*` buffer.
    pub fn split_active(&mut self, orientation: Orientation, same_buffer: bool) -> WindowId {
        let active_buf = self.active_buffer_id();
        let (buffer_id, text_view) = if same_buffer {
            let reg = self.registry.borrow();
            let buf = reg.get(active_buf).expect("active buffer present");
            (active_buf, TextView::new(buf))
        } else {
            let mut reg = self.registry.borrow_mut();
            let new_id = reg.create("*scratch*");
            let buf = reg.get(new_id).unwrap();
            (new_id, TextView::new(buf))
        };
        let new_id = WindowId::next();
        let new_window = Window::new(new_id, buffer_id, text_view);
        self.windows.insert(new_id, new_window);
        let active = self.active_window_id();
        self.active_layout_mut()
            .split_window(active, orientation, new_id);
        new_id
    }

    /// Move focus to the next window in iteration order.
    pub fn focus_next(&mut self) {
        let active = self.active_window_id();
        let next = self.active_layout().focus_next(active);
        self.set_active_window_id(next);
    }

    /// Move focus to the previous window in iteration order.
    pub fn focus_prev(&mut self) {
        let active = self.active_window_id();
        let prev = self.active_layout().focus_prev(active);
        self.set_active_window_id(prev);
    }

    /// Close the active window (unless it's the only one). Returns
    /// false if there's only one window left.
    pub fn close_active(&mut self) -> bool {
        if self.windows.len() <= 1 {
            return false;
        }
        let target = self.active_window_id();
        self.active_layout_mut().close_window(target);
        self.windows.remove(&target);
        // Pick an adjacent window as the new focus.
        let next = *self
            .active_layout()
            .iter_ids()
            .first()
            .expect("at least one window remains");
        self.set_active_window_id(next);
        true
    }

    /// Close every window except the active one.
    pub fn close_others(&mut self) {
        let keep = self.active_window_id();
        self.active_layout_mut().keep_only(keep);
        self.windows.retain(|id, _| *id == keep);
    }

    // ---- selection / region (T M2.12) --------------------------------------

    /// Active region of the active window, as `(lo, hi)` byte
    /// positions, or `None` if no region is set or it is empty.
    #[must_use]
    pub fn active_region(&self) -> Option<(Position, Position)> {
        self.active_window().region()
    }

    /// Begin a selection at `anchor` on the active window.
    pub fn begin_selection(&mut self, anchor: Position) {
        self.active_window_mut().selection = Some(crate::window::Selection { anchor });
    }

    /// Drop any active selection on the active window.
    pub fn clear_selection(&mut self) {
        self.active_window_mut().selection = None;
    }

    /// Delete the active region (if any) from the active buffer and
    /// move the cursor to the deletion's start. No-op if there is no
    /// region. Returns the new buffer length.
    ///
    /// # Errors
    ///
    /// Returns the same stringified error shape as
    /// [`Self::apply_active_edit`] if the underlying delete fails.
    pub fn delete_region(&mut self) -> Result<u64, String> {
        let Some((lo, hi)) = self.active_region() else {
            return Ok(self.active_buffer_len());
        };
        let new_len = self.apply_active_edit(EditOp::Delete {
            range: Range { start: lo, end: hi },
        })?;
        let aw = self.active_window_mut();
        aw.cursor = lo;
        aw.selection = None;
        aw.goal_col = None;
        Ok(new_len)
    }

    /// Safely remove `buffer_id` from the registry. Any window that
    /// was displaying it is redirected to a fallback buffer (`*scratch*`,
    /// created on demand) so window state never refers to a missing id.
    ///
    /// # Errors
    ///
    /// Returns an error string when `buffer_id` is the only buffer in
    /// the registry (the registry must remain non-empty), or when the
    /// id doesn't resolve.
    pub fn kill_buffer(&mut self, buffer_id: BufferId) -> Result<(), String> {
        {
            let reg = self.registry.borrow();
            if !reg.contains(buffer_id) {
                return Err(format!("buffer {buffer_id:?} not found"));
            }
            if reg.len() <= 1 {
                return Err("cannot kill the last remaining buffer".into());
            }
        }
        let fallback = {
            let mut reg = self.registry.borrow_mut();
            match reg.find_by_name("*scratch*") {
                Some(id) if id != buffer_id => id,
                _ => {
                    let candidate = reg.ids().iter().copied().find(|id| *id != buffer_id);
                    match candidate {
                        Some(id) => id,
                        None => reg.create("*scratch*"),
                    }
                }
            }
        };
        {
            let reg = self.registry.borrow();
            let buf = reg.get(fallback).map_err(|e| e.to_string())?;
            for win in self.windows.values_mut() {
                if win.buffer_id == buffer_id {
                    win.buffer_id = fallback;
                    win.text_view = TextView::new(buf);
                    win.overlays.clear();
                    win.cursor = 0;
                    win.selection = None;
                    win.view_top = 0;
                    win.goal_col = None;
                }
            }
        }
        self.registry
            .borrow_mut()
            .remove(buffer_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Switch the active window to a different buffer, allocating a
    /// fresh [`TextView`] for it.
    pub fn switch_active_buffer(&mut self, buffer_id: BufferId) -> Result<(), String> {
        let text_view = {
            let reg = self.registry.borrow();
            let buf = reg.get(buffer_id).map_err(|e| e.to_string())?;
            TextView::new(buf)
        };
        let aw = self.active_window_mut();
        aw.buffer_id = buffer_id;
        aw.text_view = text_view;
        // Overlays were keyed to the previous buffer's coordinates;
        // dropping them is safer than carrying through coordinates
        // that no longer mean anything. Callers that want to preserve
        // an overlay across buffer switches re-register after.
        aw.overlays.clear();
        aw.cursor = 0;
        aw.selection = None;
        aw.view_top = 0;
        aw.goal_col = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Codepoint navigation
// ---------------------------------------------------------------------------

/// Return the byte position of the codepoint immediately before `pos`.
fn prev_codepoint(buf: &Buffer, pos: Position) -> Position {
    if pos == 0 {
        return 0;
    }
    let rope = buf.snapshot_rope();
    let mut p = pos - 1;
    while p > 0 {
        let b = rope.byte_at(p).unwrap_or(0);
        if (b & 0xC0) != 0x80 {
            return p;
        }
        p -= 1;
    }
    0
}

/// Return the byte position of the codepoint immediately after `pos`.
fn next_codepoint(buf: &Buffer, pos: Position) -> Position {
    let len = buf.len();
    if pos >= len {
        return len;
    }
    let rope = buf.snapshot_rope();
    let lead = rope.byte_at(pos).unwrap_or(0);
    let advance = utf8_codepoint_len(lead);
    (pos + advance as u64).min(len)
}

fn utf8_codepoint_len(lead: u8) -> usize {
    if lead < 0xC0 {
        1
    } else if lead < 0xE0 {
        2
    } else if lead < 0xF0 {
        3
    } else {
        4
    }
}

/// Decode the codepoint starting at `pos`. Returns `(char, advance)`
/// where `advance` is the number of bytes the codepoint consumed.
/// `None` if `pos` is past the buffer end or the bytes there are not
/// valid UTF-8.
fn char_at(buf: &Buffer, pos: Position) -> Option<(char, u64)> {
    let rope = buf.snapshot_rope();
    if pos >= rope.len() {
        return None;
    }
    let lead = rope.byte_at(pos)?;
    let len = utf8_codepoint_len(lead);
    let mut bytes = [0u8; 4];
    for (i, slot) in bytes.iter_mut().take(len).enumerate() {
        *slot = rope.byte_at(pos + i as u64).unwrap_or(0);
    }
    let s = std::str::from_utf8(&bytes[..len]).ok()?;
    let ch = s.chars().next()?;
    Some((ch, len as u64))
}

/// Whether `c` counts as a word character. Matches the Emacs default:
/// alphanumerics plus underscore. Punctuation and whitespace are
/// separators.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Forward-word semantics: skip non-word characters, then skip word
/// characters, returning the resulting position.
fn forward_word(buf: &Buffer, mut pos: Position) -> Position {
    let len = buf.len();
    // Skip non-word.
    while pos < len {
        let Some((ch, advance)) = char_at(buf, pos) else {
            break;
        };
        if is_word_char(ch) {
            break;
        }
        pos += advance;
    }
    // Skip word.
    while pos < len {
        let Some((ch, advance)) = char_at(buf, pos) else {
            break;
        };
        if !is_word_char(ch) {
            break;
        }
        pos += advance;
    }
    pos
}

fn word_range_at(buf: &Buffer, pos: Position) -> Option<(Position, Position)> {
    let (ch, ch_len) = char_at(buf, pos)?;
    if !is_word_char(ch) {
        return None;
    }
    // Walk back from just *past* the char under the cursor, not from
    // `pos` itself: `backward_word(pos)` at a word's FIRST character
    // sees the non-word char before it, skips it, and crosses into
    // the previous word — double-clicking the 'w' of "llo world"
    // would select "llo world". From `pos + ch_len` the char behind
    // is this word's own first char, so the walk stops at its start.
    let start = backward_word(buf, pos.saturating_add(ch_len));
    let end = forward_word(buf, pos);
    (start < end).then_some((start, end))
}

/// True iff `line` is empty or contains only ASCII whitespace.
/// Used by paragraph motion: a blank line is a paragraph break.
fn line_is_blank(buf: &Buffer, view: &TextView, line: usize) -> bool {
    let Some(start) = view.line_offset(line) else {
        return true;
    };
    let Some(len) = view.line_len(buf, line) else {
        return true;
    };
    if len == 0 {
        return true;
    }
    let rope = buf.snapshot_rope();
    for chunk in rope.chunks(start, start + len) {
        if chunk.iter().any(|b| !b.is_ascii_whitespace()) {
            return false;
        }
    }
    true
}

/// Forward-paragraph: skip blank lines if currently on one, then
/// scan forward until the first blank line; return the position at
/// the start of that line, or the buffer end.
fn forward_paragraph(buf: &Buffer, view: &TextView, pos: Position) -> Position {
    let total = view.line_count();
    if total == 0 {
        return pos;
    }
    let cur_line = view.line_at_offset(pos);
    let starting_blank = line_is_blank(buf, view, cur_line);
    let mut line = cur_line.saturating_add(1);
    if starting_blank {
        while line < total && line_is_blank(buf, view, line) {
            line += 1;
        }
    }
    while line < total {
        if line_is_blank(buf, view, line) {
            return view.line_offset(line).unwrap_or(pos);
        }
        line += 1;
    }
    buf.len()
}

/// Backward-paragraph: mirror of [`forward_paragraph`].
fn backward_paragraph(buf: &Buffer, view: &TextView, pos: Position) -> Position {
    if pos == 0 {
        return 0;
    }
    let cur_line = view.line_at_offset(pos);
    if cur_line == 0 {
        return 0;
    }
    let starting_blank = line_is_blank(buf, view, cur_line);
    let mut line = cur_line - 1;
    if starting_blank {
        loop {
            if !line_is_blank(buf, view, line) {
                break;
            }
            if line == 0 {
                return view.line_offset(0).unwrap_or(0);
            }
            line -= 1;
        }
    }
    loop {
        if line_is_blank(buf, view, line) {
            return view.line_offset(line).unwrap_or(0);
        }
        if line == 0 {
            return 0;
        }
        line -= 1;
    }
}

/// Backward-word semantics: step back over non-word characters, then
/// step back over word characters.
fn backward_word(buf: &Buffer, mut pos: Position) -> Position {
    // Step back over non-word characters.
    while pos > 0 {
        let prev = prev_codepoint(buf, pos);
        let Some((ch, _)) = char_at(buf, prev) else {
            break;
        };
        if is_word_char(ch) {
            break;
        }
        pos = prev;
    }
    // Step back over word characters.
    while pos > 0 {
        let prev = prev_codepoint(buf, pos);
        let Some((ch, _)) = char_at(buf, prev) else {
            break;
        };
        if !is_word_char(ch) {
            break;
        }
        pos = prev;
    }
    pos
}

/// Normalize a buffer path to an absolute, lexically-clean form:
///
/// 1. expand a leading `~` / `~/…` against `$HOME`,
/// 2. join onto the process cwd if still relative,
/// 3. fold `.` / `..` purely lexically.
///
/// No filesystem access and no symlink resolution (unlike
/// [`std::fs::canonicalize`]): the result is correct for a
/// not-yet-created "[new file]" buffer and never silently rewrites a
/// path's on-disk identity. Every step is best-effort — if `$HOME`
/// or the cwd is unavailable the path is returned as far as it could
/// be resolved rather than panicking.
fn normalize_buffer_path(path: PathBuf) -> PathBuf {
    let path = expand_tilde(path);
    let abs = if path.is_absolute() {
        path
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path)
    } else {
        path
    };
    lexical_normalize(&abs)
}

/// Expand a leading `~` (whole component only) using `$HOME`. A bare
/// `~` becomes `$HOME`; `~/x` becomes `$HOME/x`. `~user` is left
/// untouched (no passwd lookup). Returns the input unchanged if it
/// has no leading `~`, isn't valid UTF-8, or `$HOME` is unset.
fn expand_tilde(path: PathBuf) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path;
    };
    if s == "~" {
        return std::env::var_os("HOME").map_or(path, PathBuf::from);
    }
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return Path::new(&home).join(rest);
    }
    path
}

/// Fold `.` and `..` components without touching the filesystem.
/// `..` pops a preceding normal segment; against the root (or a
/// Windows prefix) it is dropped, since you cannot ascend past it.
fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut stack: Vec<Component> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => stack.push(Component::ParentDir),
            },
            c => stack.push(c),
        }
    }
    let mut out = PathBuf::new();
    for c in stack {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn lexical_normalize_folds_dot_and_dotdot() {
        assert_eq!(
            lexical_normalize(Path::new("/a/./b/../c")),
            PathBuf::from("/a/c")
        );
        // `..` cannot ascend past the root.
        assert_eq!(
            lexical_normalize(Path::new("/../../x")),
            PathBuf::from("/x")
        );
        // Already clean ⇒ unchanged (keeps tempdir paths stable so
        // the LSP acceptance tests' exact-path asserts still hold).
        assert_eq!(
            lexical_normalize(Path::new("/tmp/quickshell/ipc.cpp")),
            PathBuf::from("/tmp/quickshell/ipc.cpp")
        );
    }

    #[test]
    fn expand_tilde_only_at_leading_component() {
        // `~user` (no passwd lookup) and a non-leading `~` are left
        // exactly as-is, independent of `$HOME`.
        assert_eq!(
            expand_tilde(PathBuf::from("~bob/x")),
            PathBuf::from("~bob/x")
        );
        assert_eq!(expand_tilde(PathBuf::from("a/~/b")), PathBuf::from("a/~/b"));
        // With `$HOME` set (the case in any normal test environment)
        // a leading `~` / `~/…` expands against its real value.
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_tilde(PathBuf::from("~")), PathBuf::from(&home));
            assert_eq!(
                expand_tilde(PathBuf::from("~/src/ipc.cpp")),
                Path::new(&home).join("src/ipc.cpp")
            );
        }
    }

    #[test]
    fn normalize_buffer_path_yields_absolute() {
        // A relative path becomes absolute (joined onto cwd) — this
        // is exactly what made clangd reject `file://ipc.cpp`.
        let p = normalize_buffer_path(PathBuf::from("ipc.cpp"));
        assert!(p.is_absolute(), "expected absolute, got {p:?}");
        assert!(p.ends_with("ipc.cpp"));
    }

    fn fresh() -> EditorCore {
        let reg: SharedRegistry =
            Rc::new(RefCell::new(crate::buffer_registry::BufferRegistry::new()));
        EditorCore::new(reg)
    }

    fn from_bytes(bytes: &[u8]) -> EditorCore {
        let reg: SharedRegistry =
            Rc::new(RefCell::new(crate::buffer_registry::BufferRegistry::new()));
        EditorCore::from_bytes(reg, "test", bytes)
    }

    #[test]
    fn insert_advances_cursor() {
        let mut s = from_bytes(b"");
        s.insert_char('h');
        s.insert_char('i');
        assert_eq!(s.cursor(), 2);
        assert_eq!(s.active_buffer_len(), 2);
    }

    #[test]
    fn backspace_undoes_insertion() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 3;
        s.backspace();
        assert_eq!(s.cursor(), 2);
        assert_eq!(s.active_buffer_len(), 2);
    }

    #[test]
    fn cursor_navigation_left_right() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 0;
        s.move_right();
        assert_eq!(s.cursor(), 1);
        s.move_right();
        s.move_right();
        s.move_right();
        assert_eq!(s.cursor(), 3);
        s.move_left();
        s.move_left();
        s.move_left();
        s.move_left();
        assert_eq!(s.cursor(), 0);
    }

    #[test]
    fn cursor_navigation_up_down_preserves_column() {
        let mut s = from_bytes(b"abcdef\nghi\njklmno");
        s.active_window_mut().cursor = 4;
        s.move_down();
        assert_eq!(s.cursor(), 10);
        s.move_down();
        assert_eq!(s.cursor(), 15);
        s.move_up();
        s.move_up();
        assert_eq!(s.cursor(), 4);
    }

    #[test]
    fn line_start_and_end() {
        let mut s = from_bytes(b"hello\nworld");
        s.active_window_mut().cursor = 8;
        s.move_line_start();
        assert_eq!(s.cursor(), 6);
        s.move_line_end();
        assert_eq!(s.cursor(), 11);
    }

    #[test]
    fn undo_clamps_cursor_to_buffer_len() {
        let mut s = from_bytes(b"");
        s.insert_char('a');
        s.insert_char('b');
        assert_eq!(s.cursor(), 2);
        s.undo();
        assert_eq!(s.cursor(), 1);
        s.undo();
        assert_eq!(s.cursor(), 0);
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 3;
        s.delete_forward();
        assert_eq!(s.active_buffer_len(), 3);
    }

    #[test]
    fn delete_word_backward_removes_previous_word_to_cursor() {
        // Cursor sits at end-of-buffer; deletes back through "world".
        let mut s = from_bytes(b"hello world");
        s.active_window_mut().cursor = 11;
        s.delete_word_backward();
        // `backward_word` lands at the start of the word ("world"
        // begins at byte 6), so we delete bytes 6..11.
        assert_eq!(s.cursor(), 6);
        assert_eq!(s.active_buffer_len(), 6);
    }

    #[test]
    fn delete_word_backward_at_start_of_buffer_is_noop() {
        let mut s = from_bytes(b"hello");
        s.active_window_mut().cursor = 0;
        s.delete_word_backward();
        assert_eq!(s.cursor(), 0);
        assert_eq!(s.active_buffer_len(), 5);
    }

    #[test]
    fn delete_word_forward_removes_next_word_from_cursor() {
        let mut s = from_bytes(b"hello world");
        s.active_window_mut().cursor = 0;
        s.delete_word_forward();
        // `forward_word` lands at the end of the first word (byte 5);
        // delete bytes 0..5. Cursor stays where it was.
        assert_eq!(s.cursor(), 0);
        assert_eq!(s.active_buffer_len(), 6);
    }

    #[test]
    fn delete_word_forward_at_end_of_buffer_is_noop() {
        let mut s = from_bytes(b"hello");
        s.active_window_mut().cursor = 5;
        s.delete_word_forward();
        assert_eq!(s.cursor(), 5);
        assert_eq!(s.active_buffer_len(), 5);
    }

    #[test]
    fn multibyte_navigation() {
        let mut s = from_bytes("héllo".as_bytes());
        s.active_window_mut().cursor = 0;
        s.move_right();
        assert_eq!(s.cursor(), 1);
        s.move_right();
        assert_eq!(s.cursor(), 3);
        s.move_right();
        assert_eq!(s.cursor(), 4);
        s.move_left();
        s.move_left();
        assert_eq!(s.cursor(), 1);
    }

    #[test]
    fn save_with_no_path_announces_via_status() {
        let mut s = fresh();
        s.save();
        assert!(s.status.contains("no file"));
    }

    /// T M10.8 — pins the Day 2 transitional fallback behavior in
    /// [`EditorCore::active_view`].
    ///
    /// **Day 2 → Day 3 transition contract**: while the dispatcher
    /// thread is being wired (Day 3 work), the daemon may set
    /// `active_frontend` to a daemon-attached `FrontendId` whose
    /// `FrontendView` hasn't been registered yet. The fallback to
    /// `FrontendId::LOCAL`'s view keeps single-frontend behavior
    /// observable.
    ///
    /// **Day 3 cleanup**: once
    /// [`EditorCore::register_frontend_view`] is invariantly called
    /// before any event dispatch, this test flips to assert "every
    /// `active_frontend` has its own registered view, no fallback
    /// ever activates." Until then, the fallback is the bridge.
    #[test]
    fn active_view_falls_back_to_local_when_active_frontend_unregistered() {
        let mut s = fresh();
        // Default active_frontend is LOCAL → no fallback yet.
        assert_eq!(s.active_frontend, FrontendId::LOCAL);
        let local_active_window = s.active_view().active;

        // Simulate the Day 2 transitional state: a daemon-attached
        // frontend's id is set as active, but no FrontendView is
        // registered for it (Day 3 work).
        s.active_frontend = FrontendId(42);
        assert!(!s.views.contains_key(&FrontendId(42)));

        // Fallback activates: active_view() returns LOCAL's view.
        let fallback_view = s.active_view();
        assert_eq!(
            fallback_view.active, local_active_window,
            "Day 2 fallback: active_view() returns LOCAL's view when active_frontend has no entry"
        );

        // Same for active_window().
        let win = s.active_window();
        assert_eq!(win.id, local_active_window);
    }

    #[test]
    fn active_view_for_explicit_fid_returns_none_when_unregistered() {
        // T M10.8 — explicit-fid lookups don't fall back. Callers
        // explicitly asking about a specific frontend get a truthful
        // None when that frontend has no state, distinguishing
        // "active by default" from "actually has its own view."
        let s = fresh();
        assert!(s.active_window_for(FrontendId(42)).is_none());
        assert!(s.active_window_for(FrontendId::LOCAL).is_some());
    }

    #[test]
    fn register_and_unregister_frontend_view() {
        // T M10.8 — the lifecycle API the dispatcher uses on attach
        // and detach. Wiring lives in `daemon.rs`; this test pins
        // the EditorCore-side semantics.
        let mut s = fresh();
        let fid = FrontendId(7);
        assert!(s.active_window_for(fid).is_none());

        // Build a view referencing the existing scratch window so
        // we don't need a fresh window allocation in this test.
        let local_view = s.views[&FrontendId::LOCAL].clone();
        s.register_frontend_view(fid, local_view);
        assert!(s.active_window_for(fid).is_some());

        // Unregister drops the entry; explicit lookup returns None.
        s.unregister_frontend_view(fid);
        assert!(s.active_window_for(fid).is_none());

        // LOCAL invariant survives unrelated register/unregister.
        assert!(s.views.contains_key(&FrontendId::LOCAL));
    }

    #[test]
    fn split_active_creates_a_second_window_on_same_buffer() {
        let mut s = fresh();
        let original = s.active_window_id();
        let new_id = s.split_active(Orientation::Vertical, true);
        assert_ne!(new_id, original);
        assert_eq!(s.windows.len(), 2);
        // Same buffer.
        assert_eq!(s.windows[&original].buffer_id, s.windows[&new_id].buffer_id);
    }

    #[test]
    fn edit_in_one_window_propagates_through_buffer_to_the_other() {
        let mut s = from_bytes(b"abc");
        let _new = s.split_active(Orientation::Vertical, true);
        // Insert via the active window.
        s.active_window_mut().cursor = 3;
        s.insert_char('X');
        // Buffer length is now 4; the *other* window shares the
        // same buffer, so its text view sees the same length.
        assert_eq!(s.active_buffer_len(), 4);
        // The other window's text_view has the same line count,
        // confirming on_edit fired.
        let active = s.active_window_id();
        let other = s.windows.keys().find(|id| **id != active).copied().unwrap();
        assert_eq!(s.windows[&other].text_view.line_count(), 1);
    }

    #[test]
    fn close_active_falls_back_to_remaining_window() {
        let mut s = fresh();
        s.split_active(Orientation::Horizontal, true);
        assert_eq!(s.windows.len(), 2);
        assert!(s.close_active());
        assert_eq!(s.windows.len(), 1);
    }

    #[test]
    fn close_active_refuses_when_only_one_window() {
        let mut s = fresh();
        assert!(!s.close_active());
        assert_eq!(s.windows.len(), 1);
    }

    #[test]
    fn focus_next_round_robins() {
        let mut s = fresh();
        let a = s.active_window_id();
        let _b = s.split_active(Orientation::Vertical, true);
        let _c = s.split_active(Orientation::Horizontal, true);
        // Splits don't move focus; `a` is still active.
        assert_eq!(s.active_window_id(), a);
        let order = s.active_layout().iter_ids();
        assert_eq!(order.len(), 3);
        // Walking N times wraps back to the original.
        for _ in 0..3 {
            s.focus_next();
        }
        assert_eq!(s.active_window_id(), a);
    }

    // ------------------------------------------------------------------
    // F27 / F28 (post-audit-round-5) — daemon-origin CRDT ops are
    // queued on `pending_crdt_ops` so they reach all replicas.
    // ------------------------------------------------------------------

    /// Helper: upgrade the active buffer to CRDT-backed under the
    /// LOCAL peer id (mirrors what the daemon does at attach time
    /// for replica sessions).
    #[cfg(feature = "crdt")]
    fn upgrade_active_to_crdt(s: &mut EditorCore) {
        let buffer_id = s.active_buffer_id();
        let mut reg = s.registry.borrow_mut();
        let buf = reg.get_mut(buffer_id).expect("active buffer present");
        buf.upgrade_to_crdt(crate::crdt::peer_id_from_frontend(
            crate::protocol::FrontendId::LOCAL,
        ))
        .expect("upgrade");
    }

    /// F27 — undo on a CRDT-backed buffer queues the resulting
    /// CRDT op for broadcast.
    #[cfg(feature = "crdt")]
    #[test]
    fn undo_on_crdt_buffer_queues_crdt_op_for_broadcast_f27() {
        let mut s = from_bytes(b"abc");
        upgrade_active_to_crdt(&mut s);
        // Apply an edit so there's something to undo. apply_active_edit
        // also pushes a DaemonKey-origin op.
        s.apply_active_edit(crate::buffer::EditOp::Insert {
            pos: 3,
            bytes: b"X",
        })
        .expect("edit");
        let queued_after_edit = s.pending_crdt_ops.len();
        assert!(queued_after_edit >= 1, "edit must queue a CRDT op");

        // Drain to isolate the undo's queueing.
        s.pending_crdt_ops.clear();
        s.undo();

        assert!(
            !s.pending_crdt_ops.is_empty(),
            "F27: undo on a CRDT-backed buffer must queue a CRDT op for broadcast"
        );
        // Origin must be DaemonKey (broadcast-to-all-replicas).
        let (origin, _, _) = &s.pending_crdt_ops[0];
        assert!(
            matches!(origin, CrdtOpOrigin::DaemonKey),
            "F27: undo's CRDT op must be queued with DaemonKey origin (broadcast to all replicas including active frontend)"
        );
    }

    /// F27 — redo on a CRDT-backed buffer queues the resulting
    /// CRDT op for broadcast.
    #[cfg(feature = "crdt")]
    #[test]
    fn redo_on_crdt_buffer_queues_crdt_op_for_broadcast_f27() {
        let mut s = from_bytes(b"abc");
        upgrade_active_to_crdt(&mut s);
        s.apply_active_edit(crate::buffer::EditOp::Insert {
            pos: 3,
            bytes: b"X",
        })
        .expect("edit");
        s.undo();
        s.pending_crdt_ops.clear();
        s.redo();
        assert!(
            !s.pending_crdt_ops.is_empty(),
            "F27: redo on a CRDT-backed buffer must queue a CRDT op for broadcast"
        );
        let (origin, _, _) = &s.pending_crdt_ops[0];
        assert!(matches!(origin, CrdtOpOrigin::DaemonKey));
    }

    /// F27 — undo on a non-CRDT buffer is a no-op for the broadcast
    /// queue (the buffer produced no `crdt_op` on the Edit).
    #[test]
    fn undo_on_non_crdt_buffer_does_not_queue_crdt_op_f27() {
        let mut s = from_bytes(b"abc");
        s.apply_active_edit(crate::buffer::EditOp::Insert {
            pos: 3,
            bytes: b"X",
        })
        .expect("edit");
        // Non-CRDT — apply_active_edit's pending push is a no-op
        // (Edit::crdt_op is None). Confirm precondition then undo.
        assert!(s.pending_crdt_ops.is_empty());
        s.undo();
        assert!(
            s.pending_crdt_ops.is_empty(),
            "F27: undo on a non-CRDT buffer must not produce a phantom queue entry"
        );
    }

    // ---- jump ring (T M4.5 L1) -----------------------------------------

    #[test]
    fn jump_back_returns_false_on_empty_ring() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 2;
        assert!(!s.jump_back(), "empty ring must not move the cursor");
        assert_eq!(s.cursor(), 2);
    }

    #[test]
    fn push_then_jump_back_restores_cursor() {
        let mut s = from_bytes(b"line one\nline two\nline three");
        s.active_window_mut().cursor = 3;
        s.push_jump();
        s.active_window_mut().cursor = 20;
        assert!(s.jump_back());
        assert_eq!(s.cursor(), 3);
        // Ring is now empty; a second pop is a no-op.
        assert!(!s.jump_back());
    }

    #[test]
    fn jump_back_clamps_to_shortened_buffer() {
        let mut s = from_bytes(b"abcdefghij");
        s.active_window_mut().cursor = 9;
        s.push_jump();
        // Truncate the buffer so the recorded position is past EOF.
        s.apply_active_edit(crate::buffer::EditOp::Delete {
            range: Range::new(2, 10),
        })
        .expect("delete");
        assert!(s.jump_back());
        assert_eq!(
            s.cursor(),
            s.active_buffer_len(),
            "stale position must clamp to the current buffer length"
        );
    }

    #[test]
    fn jump_ring_is_bounded_and_evicts_oldest() {
        let mut s = from_bytes(b"0123456789");
        for i in 0..(EditorCore::JUMP_RING_CAP + 10) {
            s.active_window_mut().cursor = (i % 10) as u64;
            s.push_jump();
        }
        assert_eq!(
            s.jump_ring.len(),
            EditorCore::JUMP_RING_CAP,
            "ring must stay bounded at JUMP_RING_CAP"
        );
    }

    #[test]
    fn jump_back_skips_removed_buffer() {
        let mut s = from_bytes(b"original");
        // Record a jump on a second buffer, then remove that buffer.
        let doomed = s.registry.borrow_mut().create_from_bytes("doomed", b"x");
        s.switch_active_buffer(doomed).expect("switch");
        s.active_window_mut().cursor = 1;
        s.push_jump();
        // Switch back and record a live origin too.
        let original = *s.registry.borrow().ids().first().expect("original id");
        s.switch_active_buffer(original).expect("switch back");
        s.active_window_mut().cursor = 4;
        s.push_jump();
        s.active_window_mut().cursor = 0;
        // Drop the doomed buffer: its ring entry is now stale.
        s.registry.borrow_mut().remove(doomed).expect("remove");
        // First pop lands on the live `original` origin.
        assert!(s.jump_back());
        assert_eq!(s.active_buffer_id(), original);
        assert_eq!(s.cursor(), 4);
        // Next pop would be the stale `doomed` entry — skipped, ring empties.
        assert!(!s.jump_back());
    }
}
