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

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::buffer::{Buffer, BufferId, EditOp};
use crate::file_io::{FileMeta, save_atomic};
use crate::lua_bindings::SharedRegistry;
use crate::minibuffer::Minibuffer;
use crate::protocol::FrontendId;
use crate::rope::Edit;
use crate::rope::{Position, Range};
use crate::text_view::TextView;
use crate::view::{DisplayCoord, View};
use crate::window::{Layout, Orientation, Window, WindowId};

/// The world state mutated by editor commands.
pub struct EditorCore {
    /// Shared buffer registry. The registry is the canonical owner
    /// of every buffer; windows reference buffers by [`BufferId`].
    pub registry: SharedRegistry,
    /// Open windows, keyed by id for stable iteration.
    pub windows: BTreeMap<WindowId, Window>,
    /// Window tree mapping the cell grid to windows.
    pub layout: Layout,
    /// The focused window. `pmacs.editor.*` primitives target it.
    pub active: WindowId,
    /// One-line message shown in the status line.
    pub status: String,
    /// True iff the editor should exit at the next iteration.
    pub quit: bool,
    /// File backing the active window's buffer, if any.
    ///
    /// Kept on the core (rather than per-buffer) for v0.1 single-
    /// file workflows. Multi-file open will need this on the buffer
    /// itself or in a side table; for now, M2.8 doesn't exercise the
    /// distinction.
    pub file_path: Option<PathBuf>,
    /// File metadata at the last load or save.
    pub file_meta: Option<FileMeta>,
    /// Universal minibuffer (T M2.7).
    pub minibuffer: Minibuffer,
    /// The frontend that produced the most recent input event
    /// dispatched to this core (T M5.4). v0.1 has a single frontend
    /// per instance, so this stays at [`FrontendId::LOCAL`] in
    /// practice; the field is load-bearing for v0.3 multi-frontend
    /// (multi-window, multi-user) where each input event must be
    /// attributable to its source frontend.
    pub active_frontend: FrontendId,
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
        Self {
            registry,
            windows,
            layout: Layout::single(id),
            active: id,
            status: String::new(),
            quit: false,
            file_path: None,
            file_meta: None,
            minibuffer: Minibuffer::new(),
            active_frontend: FrontendId::LOCAL,
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

    /// Reference the active [`Window`].
    #[must_use]
    pub fn active_window(&self) -> &Window {
        self.windows
            .get(&self.active)
            .expect("active window present")
    }

    /// Mutably reference the active [`Window`].
    pub fn active_window_mut(&mut self) -> &mut Window {
        self.windows
            .get_mut(&self.active)
            .expect("active window present")
    }

    /// [`BufferId`] of the active window's buffer.
    #[must_use]
    pub fn active_buffer_id(&self) -> BufferId {
        self.active_window().buffer_id
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
        self.registry.borrow().get(id).map(Buffer::len).unwrap_or(0)
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
        let Some(path) = self.file_path.clone() else {
            self.status = "no file (M1: open a file from argv)".into();
            return false;
        };
        let id = self.active_buffer_id();
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
                self.file_meta = Some(meta);
                if let Ok(buf) = self.registry.borrow_mut().get_mut(id) {
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
            }
            Err(_) => self.status = "nothing to redo".into(),
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
        self.layout.split_window(self.active, orientation, new_id);
        new_id
    }

    /// Move focus to the next window in iteration order.
    pub fn focus_next(&mut self) {
        self.active = self.layout.focus_next(self.active);
    }

    /// Move focus to the previous window in iteration order.
    pub fn focus_prev(&mut self) {
        self.active = self.layout.focus_prev(self.active);
    }

    /// Close the active window (unless it's the only one). Returns
    /// false if there's only one window left.
    pub fn close_active(&mut self) -> bool {
        if self.windows.len() <= 1 {
            return false;
        }
        let target = self.active;
        self.layout.close_window(target);
        self.windows.remove(&target);
        // Pick an adjacent window as the new focus.
        self.active = *self
            .layout
            .iter_ids()
            .first()
            .expect("at least one window remains");
        true
    }

    /// Close every window except the active one.
    pub fn close_others(&mut self) {
        let keep = self.active;
        self.layout.keep_only(keep);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

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

    #[test]
    fn split_active_creates_a_second_window_on_same_buffer() {
        let mut s = fresh();
        let original = s.active;
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
        let other = s
            .windows
            .keys()
            .find(|id| **id != s.active)
            .copied()
            .unwrap();
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
        let a = s.active;
        let _b = s.split_active(Orientation::Vertical, true);
        let _c = s.split_active(Orientation::Horizontal, true);
        // Splits don't move focus; `a` is still active.
        assert_eq!(s.active, a);
        let order = s.layout.iter_ids();
        assert_eq!(order.len(), 3);
        // Walking N times wraps back to the original.
        for _ in 0..3 {
            s.focus_next();
        }
        assert_eq!(s.active, a);
    }
}
