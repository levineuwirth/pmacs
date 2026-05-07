// buffer.rs --- Rope + identity + attached views + per-buffer state.

//! Buffers: the unit of editable content.
//!
//! Implements the buffer contract from spec §3.2 and the edit flow from
//! spec §3.5. A buffer owns a [`Rope`], a list of attached views, a name,
//! a modified flag, and undo/redo stacks. It coordinates rather than
//! computes: cursors, line indices, and selection state live in views.
//!
//! # Edit flow
//!
//! On [`Buffer::apply_edit`]:
//! 1. Each attached view's `intercept_edit` runs in registration order,
//!    possibly rewriting the operation.
//! 2. The rope edit is applied, producing a new rope and an [`Edit`]
//!    description.
//! 3. The buffer swaps in the new rope. The old rope is pushed onto the
//!    undo stack; the redo stack is cleared (any forward edit forks the
//!    history).
//! 4. Each view's `on_edit` is called with the [`Edit`] description.
//!
//! # Re-entry
//!
//! Views never hold a back-pointer to the buffer (spec §2.6). The buffer
//! passes itself in to each callback. Internally, the buffer temporarily
//! moves its view list out of `self` before iterating, so the views can
//! observe `&Buffer` while the buffer's own `&mut self` is held.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::rope::{Edit, Position, Range, Rope, RopeError};
use crate::view::{InterceptContext, View};

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Opaque, per-process identifier for a buffer.
///
/// The internal representation is private (R22): callers cannot reach for
/// `.0`; construction goes through [`BufferId::next`].
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct BufferId(u64);

impl BufferId {
    /// Allocate a fresh [`BufferId`] from the process-wide counter.
    ///
    /// Threading: any thread.
    #[must_use]
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Inspect the raw value. Useful for logging and FFI.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Rebuild an ID from a raw value for crate-internal references that
    /// persist an already-issued buffer identity in generated text.
    #[must_use]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Opaque, per-buffer identifier for an attached view.
///
/// Identity is scoped to the buffer that issued it; two different buffers
/// may both hand out a `ViewId(0)`. View IDs are returned by
/// [`Buffer::attach_view`] and accepted by [`Buffer::detach_view`].
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ViewId(u64);

impl ViewId {
    /// Inspect the raw value. Useful for logging and FFI.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Opaque, per-buffer identifier for a position mark.
///
/// Marks are owned by a [`Buffer`] and move through edits according to
/// their gravity. They are intentionally not process-global: a
/// `MarkId(0)` from one buffer has no meaning in another buffer.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct MarkId(u64);

impl MarkId {
    /// Inspect the raw value. Useful for logging and FFI.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Which side of an insertion/replacement a mark sticks to.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum MarkGravity {
    /// Stay before bytes inserted exactly at the mark.
    Left,
    /// Move after bytes inserted exactly at the mark.
    Right,
}

#[derive(Copy, Clone, Debug)]
struct Mark {
    pos: Position,
    gravity: MarkGravity,
}

// ---------------------------------------------------------------------------
// Edit operations
// ---------------------------------------------------------------------------

/// A pending edit before it reaches the rope.
///
/// Carries a borrow of the bytes to insert; the borrow only needs to live
/// for the duration of the [`Buffer::apply_edit`] call (the rope copies
/// bytes into its leaf chunks).
#[derive(Debug)]
pub enum EditOp<'a> {
    /// Insert `bytes` at byte position `pos`.
    Insert {
        /// Byte position. Must satisfy `pos <= rope.len()`.
        pos: Position,
        /// Bytes to insert.
        bytes: &'a [u8],
    },
    /// Delete the byte range `[range.start, range.end)`.
    Delete {
        /// Range of bytes to remove.
        range: Range,
    },
    /// Replace the byte range `[range.start, range.end)` with `bytes`.
    Replace {
        /// Range of bytes to replace.
        range: Range,
        /// Replacement bytes.
        bytes: &'a [u8],
    },
}

// ---------------------------------------------------------------------------
// Buffer
// ---------------------------------------------------------------------------

/// One entry in the undo (or redo) stack.
struct UndoEntry {
    /// Pre-edit rope. Cheap to retain: persistent rope, structural sharing.
    rope: Rope,
    /// Description of the edit that produced the current rope from this
    /// entry's rope. Used to broadcast a precise inverse edit on undo.
    edit: EditDescription,
}

/// A reduced [`Edit`] descriptor used by the undo stack.
///
/// Stored separately from `Edit` because `Edit` carries a `new_rope` that
/// duplicates `UndoEntry::rope`; we keep only the deltas here.
#[derive(Copy, Clone, Debug)]
struct EditDescription {
    /// Range in the *pre-edit* rope that was affected.
    pre_range: Range,
    /// Number of bytes inserted at `pre_range.start` to produce the
    /// post-edit rope.
    inserted_len: u64,
}

/// The unit of editable content: rope + identity + views + undo.
///
/// # Threading
///
/// Main thread only. The buffer holds `Box<dyn View>` trait objects whose
/// methods take `&mut self`; the buffer is single-owner. Workers receive
/// rope snapshots via [`Buffer::snapshot_rope`], not buffer references.
pub struct Buffer {
    id: BufferId,
    rope: Rope,
    name: String,
    is_modified: bool,
    /// Monotonic counter bumped by every successful forward edit, undo,
    /// and redo. Used by the editor to detect "did this command modify
    /// the buffer?" without reaching into the rope. LSP `did_change`
    /// notifications carry this as the document version.
    revision: u64,
    /// `(id, view)` pairs in attach order. Views are stored as trait
    /// objects (R32) because the set of view types is open --- Lua
    /// packages will define new ones.
    views: Vec<(ViewId, Box<dyn View>)>,
    /// Per-buffer counter for [`ViewId`] allocation.
    next_view_id: u64,
    /// Buffer-relative marks. Kept as a small vector because current
    /// consumers create a handful per buffer; if this grows into
    /// thousands, this can become an indexed table without changing
    /// the public API.
    marks: Vec<(MarkId, Mark)>,
    /// Per-buffer counter for [`MarkId`] allocation.
    next_mark_id: u64,
    /// Undo stack. Most recent entry on top.
    undo: Vec<UndoEntry>,
    /// Redo stack. Cleared by any forward edit.
    redo: Vec<UndoEntry>,
    /// True while an edit is in flight on this buffer (T M7.4).
    /// Set by [`Buffer::begin_edit`], cleared by [`Buffer::end_edit`].
    /// A re-entrant `apply_edit` / `apply_edit_skip_intercepts` while
    /// the flag is set returns [`BufferError::ConcurrentEdit`] rather
    /// than mutating the rope mid-intercept; cross-buffer re-entry
    /// is unaffected.
    editing_in_progress: bool,
}

impl Buffer {
    /// Construct an empty buffer with the given identity and name.
    ///
    /// Threading: main thread only.
    #[must_use]
    pub fn new(id: BufferId, name: impl Into<String>) -> Self {
        Self::from_rope(id, name, Rope::new())
    }

    /// Construct a buffer holding the given bytes.
    ///
    /// Convenience over `from_rope(id, name, Rope::from_bytes(bytes))`,
    /// used by file load. Threading: main thread only.
    #[must_use]
    pub fn from_bytes(id: BufferId, name: impl Into<String>, bytes: &[u8]) -> Self {
        Self::from_rope(id, name, Rope::from_bytes(bytes))
    }

    /// Construct a buffer wrapping an existing rope.
    ///
    /// Threading: main thread only.
    #[must_use]
    pub fn from_rope(id: BufferId, name: impl Into<String>, rope: Rope) -> Self {
        Self {
            id,
            rope,
            name: name.into(),
            is_modified: false,
            revision: 0,
            views: Vec::new(),
            next_view_id: 0,
            marks: Vec::new(),
            next_mark_id: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            editing_in_progress: false,
        }
    }

    /// This buffer's identifier.
    ///
    /// Threading: main thread only (entire `Buffer` API is main-only).
    #[must_use]
    pub fn id(&self) -> BufferId {
        self.id
    }

    /// This buffer's name. Typically a file path or a synthetic label
    /// like `*scratch*`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set the buffer's name. Used by save-as and rename operations.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Whether the buffer has been modified since the last save / load.
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.is_modified
    }

    /// Monotonic edit counter. Bumped by every successful forward edit,
    /// undo, and redo. The active-buffer revision delta across a key
    /// dispatch is the editor's "did this command edit the buffer?"
    /// signal; LSP wiring uses it as the `textDocument/didChange`
    /// document version.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Mark the buffer as unmodified. Called after a successful save.
    pub fn mark_clean(&mut self) {
        self.is_modified = false;
    }

    /// Total length of the buffer in bytes.
    #[must_use]
    pub fn len(&self) -> Position {
        self.rope.len()
    }

    /// True iff the buffer holds zero bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rope.is_empty()
    }

    /// Take a rope snapshot.
    ///
    /// O(1) (an `Arc` bump). The returned [`Rope`] is independent of the
    /// buffer: subsequent edits to the buffer do not affect it. Use this to
    /// hand a consistent view to a worker (M3+).
    ///
    /// Threading: main thread only (requires `&self`); the result is
    /// `Send + Sync` and may be passed across threads.
    #[must_use]
    pub fn snapshot_rope(&self) -> Rope {
        self.rope.snapshot()
    }

    /// Number of attached views.
    #[must_use]
    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// Iterate over the attached views' IDs in attach order.
    pub fn view_ids(&self) -> impl Iterator<Item = ViewId> + '_ {
        self.views.iter().map(|(id, _)| *id)
    }

    /// Attach a view. Returns the freshly allocated [`ViewId`].
    pub fn attach_view(&mut self, view: Box<dyn View>) -> ViewId {
        let id = ViewId(self.next_view_id);
        self.next_view_id += 1;
        self.views.push((id, view));
        id
    }

    /// Detach the view with the given ID, returning ownership to the
    /// caller. Returns `None` if `id` is not attached.
    pub fn detach_view(&mut self, id: ViewId) -> Option<Box<dyn View>> {
        let idx = self.views.iter().position(|(v, _)| *v == id)?;
        Some(self.views.remove(idx).1)
    }

    /// Create a mark at byte position `pos`.
    ///
    /// The position must be inside the current buffer (`pos <= len`).
    /// The returned ID is scoped to this buffer.
    pub fn create_mark(
        &mut self,
        pos: Position,
        gravity: MarkGravity,
    ) -> Result<MarkId, BufferError> {
        if pos > self.len() {
            return Err(BufferError::Rope(RopeError::OutOfBounds {
                pos,
                len: self.len(),
            }));
        }
        let id = MarkId(self.next_mark_id);
        self.next_mark_id += 1;
        self.marks.push((id, Mark { pos, gravity }));
        Ok(id)
    }

    /// Current byte position of a mark, or `None` if it has been removed.
    #[must_use]
    pub fn mark_pos(&self, id: MarkId) -> Option<Position> {
        self.marks
            .iter()
            .find_map(|(mark_id, mark)| (*mark_id == id).then_some(mark.pos))
    }

    /// Move an existing mark to `pos`.
    ///
    /// Returns `Ok(false)` for an unknown mark ID. Out-of-bounds
    /// positions are errors and leave the mark unchanged.
    pub fn set_mark(&mut self, id: MarkId, pos: Position) -> Result<bool, BufferError> {
        if pos > self.len() {
            return Err(BufferError::Rope(RopeError::OutOfBounds {
                pos,
                len: self.len(),
            }));
        }
        let Some((_, mark)) = self.marks.iter_mut().find(|(mark_id, _)| *mark_id == id) else {
            return Ok(false);
        };
        mark.pos = pos;
        Ok(true)
    }

    /// Remove a mark. Returns `true` if the mark existed.
    pub fn remove_mark(&mut self, id: MarkId) -> bool {
        let Some(idx) = self.marks.iter().position(|(mark_id, _)| *mark_id == id) else {
            return false;
        };
        self.marks.remove(idx);
        true
    }

    /// Take all attached views out of the buffer, returning ownership
    /// to the caller (T M7.4).
    ///
    /// Pair with [`Buffer::restore_views`]. While the views are taken
    /// out, the buffer's view list is empty: `attach_view` calls
    /// during this window land in the empty list and will be
    /// preserved by `restore_views`.
    ///
    /// Used by the Lua bindings to run the intercept chain with the
    /// registry borrow released, so an intercept body may safely
    /// re-enter the buffer API on any buffer (including this one,
    /// modulo the `editing_in_progress` gate).
    pub fn take_views(&mut self) -> Vec<(ViewId, Box<dyn View>)> {
        std::mem::take(&mut self.views)
    }

    /// Restore previously-taken views.
    ///
    /// Views attached during the take/restore window are preserved
    /// and ordered after the restored set. Use case: a Lua intercept
    /// body on buffer A calls `pmacs.buffer.add_intercept(A, ...)` to
    /// install another intercept; the new view should sit after the
    /// existing chain so the existing chain still runs first on
    /// future edits.
    pub fn restore_views(&mut self, mut original: Vec<(ViewId, Box<dyn View>)>) {
        let new_additions = std::mem::take(&mut self.views);
        original.extend(new_additions);
        self.views = original;
    }

    /// Mark the buffer as mid-edit (T M7.4). Pairs with [`Buffer::end_edit`].
    ///
    /// Returns [`BufferError::ConcurrentEdit`] if a previous
    /// `begin_edit` is unmatched. The Lua bindings call this at the
    /// start of the three-phase edit flow so that a re-entrant Lua
    /// call into the same buffer's `apply_edit` /
    /// `apply_edit_skip_intercepts` surfaces a typed error rather
    /// than silently corrupting state.
    pub fn begin_edit(&mut self) -> Result<(), BufferError> {
        if self.editing_in_progress {
            return Err(BufferError::ConcurrentEdit {
                id: self.id,
                name: self.name.clone(),
            });
        }
        self.editing_in_progress = true;
        Ok(())
    }

    /// Clear the mid-edit flag set by [`Buffer::begin_edit`].
    /// Idempotent. Lua bindings call this at the end of the edit
    /// flow, before the final `apply_edit_skip_intercepts`.
    pub fn end_edit(&mut self) {
        self.editing_in_progress = false;
    }

    /// Whether the buffer is currently mid-edit (T M7.4).
    /// Useful for diagnostic tooling; the in-process flow's
    /// re-entrancy check happens inside `apply_edit` itself.
    #[must_use]
    pub fn editing_in_progress(&self) -> bool {
        self.editing_in_progress
    }

    /// Apply an edit.
    ///
    /// Walks the intercept-edit chain in attach order; applies the
    /// (possibly rewritten) operation to the rope; pushes the previous
    /// rope onto the undo stack and clears redo; broadcasts the edit to
    /// each view's `on_edit`. Returns the [`Edit`] description.
    ///
    /// On error the buffer is left in its pre-edit state and the undo
    /// stack is unchanged.
    ///
    /// # Re-entrancy (T M7.4)
    ///
    /// In-process Rust callers run intercepts under the same `&mut Buffer`
    /// borrow that owns the apply --- no re-entry path exists, so the
    /// `editing_in_progress` flag is not set by this method (it is set
    /// only by [`Buffer::begin_edit`], which the Lua bindings use to gate
    /// same-buffer re-entry). A caller that does `b.apply_edit(...)`
    /// while another `apply_edit` is on the stack for the same `b`
    /// would already fail at `&mut` aliasing in safe Rust.
    ///
    /// Threading: main thread only.
    pub fn apply_edit(&mut self, op: EditOp<'_>) -> Result<Edit, BufferError> {
        if self.editing_in_progress {
            return Err(BufferError::ConcurrentEdit {
                id: self.id,
                name: self.name.clone(),
            });
        }
        // Take views out so the loop body can borrow `&self` while iterating.
        // The buffer is left view-less only for the duration of this call;
        // panics during it would leave an empty view list (acceptable: views
        // are held by `Box`, no resource leak).
        let mut views = std::mem::take(&mut self.views);
        let result = self.apply_edit_inner(&mut views, op);
        // Restore views even on error.
        self.views = views;
        result
    }

    /// Apply an edit, skipping the intercept chain.
    ///
    /// Used by the Lua bindings (T M7.4) after they have run intercepts
    /// out-of-band with the registry borrow released. Behaves like
    /// [`Buffer::apply_edit`] from "rope edit" onward: rope mutation,
    /// undo bookkeeping, modified flag, revision bump, and `on_edit`
    /// broadcast all happen here.
    ///
    /// In-process Rust callers should use [`Buffer::apply_edit`]
    /// instead --- this primitive exists for the case where the
    /// caller has already evaluated the intercept chain and has the
    /// final [`EditOp`] in hand.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "by-value mirrors apply_edit's signature; the Lua bindings build a fresh EditOp per call"
    )]
    pub fn apply_edit_skip_intercepts(&mut self, op: EditOp<'_>) -> Result<Edit, BufferError> {
        let mut views = std::mem::take(&mut self.views);
        let result = self.run_rope_edit_and_broadcast(&mut views, &op);
        self.views = views;
        result
    }

    fn apply_edit_inner(
        &mut self,
        views: &mut [(ViewId, Box<dyn View>)],
        op: EditOp<'_>,
    ) -> Result<Edit, BufferError> {
        // Stage 1: intercept chain.
        let mut current = op;
        let ctx = InterceptContext::snapshot(self);
        for (_, view) in views.iter_mut() {
            current = view.intercept_edit(&ctx, current)?;
        }

        // Stages 2-4: rope edit + state update + broadcast.
        self.run_rope_edit_and_broadcast(views, &current)
    }

    fn run_rope_edit_and_broadcast(
        &mut self,
        views: &mut [(ViewId, Box<dyn View>)],
        current: &EditOp<'_>,
    ) -> Result<Edit, BufferError> {
        // Stage 2: rope edit.
        let edit = match current {
            EditOp::Insert { pos, bytes } => self.rope.insert(*pos, bytes)?,
            EditOp::Delete { range } => self.rope.delete(range.start, range.end)?,
            EditOp::Replace { range, bytes } => self.rope.replace(range.start, range.end, bytes)?,
        };

        // No-op: empty insert, empty-range delete, or replace-empty-with-
        // empty all roundtrip with no actual change. Skip undo bookkeeping
        // and don't mark the buffer modified --- the spec's edit flow
        // describes "successful edits" pushing onto undo, and a no-op
        // doesn't count.
        if edit.range.is_empty() && edit.inserted_len == 0 {
            // We still broadcast on_edit so views don't miss a deliberate
            // no-op (e.g. callers that count the call). The rope is
            // unchanged, so we don't swap it.
            for (_, view) in views.iter_mut() {
                view.on_edit(self, &edit)?;
            }
            return Ok(edit);
        }

        // Stage 3: state update.
        let pre_range = edit.range;
        let inserted_len = edit.inserted_len;
        let old_rope = std::mem::replace(&mut self.rope, edit.new_rope.clone());
        self.adjust_marks_for_edit(pre_range, inserted_len);
        self.undo.push(UndoEntry {
            rope: old_rope,
            edit: EditDescription {
                pre_range,
                inserted_len,
            },
        });
        self.redo.clear();
        self.is_modified = true;
        self.revision = self.revision.wrapping_add(1);

        // Stage 4: broadcast.
        for (_, view) in views.iter_mut() {
            view.on_edit(self, &edit)?;
        }

        Ok(edit)
    }

    /// Undo the most recent edit.
    ///
    /// Returns the [`Edit`] description of the inverse change. The most
    /// recent forward edit is moved from the undo stack to the redo stack.
    /// On error (nothing to undo), the buffer is unchanged.
    ///
    /// Threading: main thread only.
    pub fn undo(&mut self) -> Result<Edit, BufferError> {
        let entry = self.undo.pop().ok_or(BufferError::NothingToUndo)?;

        // The pre-edit rope held in `entry.rope` becomes current. The
        // inverse edit affects the post-state's range
        // `[pre_range.start, pre_range.start + inserted_len)` and produces
        // `pre_range.len()` bytes (the bytes originally at `pre_range`).
        let inverse_pre_range = Range::new(
            entry.edit.pre_range.start,
            entry.edit.pre_range.start + entry.edit.inserted_len,
        );
        let inverse_inserted_len = entry.edit.pre_range.len();

        let new_rope = entry.rope.clone();
        let old_rope = std::mem::replace(&mut self.rope, new_rope.clone());
        self.adjust_marks_for_edit(inverse_pre_range, inverse_inserted_len);
        self.redo.push(UndoEntry {
            rope: old_rope,
            edit: EditDescription {
                pre_range: inverse_pre_range,
                inserted_len: inverse_inserted_len,
            },
        });
        self.is_modified = !self.undo.is_empty();
        self.revision = self.revision.wrapping_add(1);

        let inverse_edit = Edit {
            new_rope,
            range: inverse_pre_range,
            inserted_len: inverse_inserted_len,
        };
        self.broadcast_on_edit(&inverse_edit)?;
        Ok(inverse_edit)
    }

    /// Redo a previously undone edit.
    ///
    /// Symmetric to [`Buffer::undo`]. The redo stack is cleared by any
    /// forward edit, so `redo` is only meaningful immediately after a
    /// sequence of `undo`s.
    ///
    /// Threading: main thread only.
    pub fn redo(&mut self) -> Result<Edit, BufferError> {
        let entry = self.redo.pop().ok_or(BufferError::NothingToRedo)?;

        let inverse_pre_range = Range::new(
            entry.edit.pre_range.start,
            entry.edit.pre_range.start + entry.edit.inserted_len,
        );
        let inverse_inserted_len = entry.edit.pre_range.len();

        let new_rope = entry.rope.clone();
        let old_rope = std::mem::replace(&mut self.rope, new_rope.clone());
        self.adjust_marks_for_edit(inverse_pre_range, inverse_inserted_len);
        self.undo.push(UndoEntry {
            rope: old_rope,
            edit: EditDescription {
                pre_range: inverse_pre_range,
                inserted_len: inverse_inserted_len,
            },
        });
        self.is_modified = true;
        self.revision = self.revision.wrapping_add(1);

        let inverse_edit = Edit {
            new_rope,
            range: inverse_pre_range,
            inserted_len: inverse_inserted_len,
        };
        self.broadcast_on_edit(&inverse_edit)?;
        Ok(inverse_edit)
    }

    fn broadcast_on_edit(&mut self, edit: &Edit) -> Result<(), BufferError> {
        let mut views = std::mem::take(&mut self.views);
        let result = (|| {
            for (_, view) in &mut views {
                view.on_edit(self, edit)?;
            }
            Ok(())
        })();
        self.views = views;
        result
    }

    fn adjust_marks_for_edit(&mut self, range: Range, inserted_len: u64) {
        let start = range.start;
        let end = range.end;
        let old_len = range.len();
        let new_end = start.saturating_add(inserted_len);

        for (_, mark) in &mut self.marks {
            let pos = mark.pos;
            mark.pos = if pos < start {
                pos
            } else if pos > end {
                pos - old_len + inserted_len
            } else if pos == start {
                if old_len == 0 && mark.gravity == MarkGravity::Right {
                    new_end
                } else if old_len == 0 {
                    start
                } else {
                    match mark.gravity {
                        MarkGravity::Left => start,
                        MarkGravity::Right => new_end,
                    }
                }
            } else {
                match mark.gravity {
                    MarkGravity::Left => start,
                    MarkGravity::Right => new_end,
                }
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by [`Buffer`] operations.
#[derive(Debug, thiserror::Error)]
pub enum BufferError {
    /// The underlying rope rejected the operation.
    #[error("rope error: {0}")]
    Rope(#[from] RopeError),
    /// `undo` was called with an empty undo stack.
    #[error("nothing to undo")]
    NothingToUndo,
    /// `redo` was called with an empty redo stack.
    #[error("nothing to redo")]
    NothingToRedo,
    /// A view's `intercept_edit` rejected the operation with a typed
    /// reason. Used by the M6.4 REPL package's Lua intercept to surface
    /// "this region is read-only" without coopting an unrelated rope
    /// error variant; available to any future view that wants to
    /// reject with a human-readable message.
    #[error("intercept rejected the edit: {reason}")]
    Intercepted {
        /// Human-readable reason. Surfaced verbatim to the user.
        reason: String,
    },
    /// A re-entrant edit was attempted on a buffer that is already
    /// mid-edit (T M7.4). The most common path: a Lua intercept body
    /// running on buffer A called `A:insert(...)` or similar.
    /// Cross-buffer re-entry (`A`'s intercept editing `B`) is allowed
    /// and does not surface this error.
    ///
    /// The message names a workaround per the project convention.
    #[error(
        "buffer `{name}` (id {id:?}) is already being edited; \
         re-entrant edits on the same buffer are not supported. \
         To compose with the current edit, return a transformed table \
         from this intercept; to schedule a follow-up edit, register an \
         on-edit hook that runs after the current edit completes, or \
         edit a different buffer."
    )]
    ConcurrentEdit {
        /// The buffer's identifier.
        id: BufferId,
        /// The buffer's name, for diagnostics.
        name: String,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::View;
    use std::sync::Mutex;

    fn fresh() -> Buffer {
        Buffer::new(BufferId::next(), "*scratch*")
    }

    fn collect(buf: &Buffer) -> Vec<u8> {
        let mut out = vec![0u8; buf.len() as usize];
        if !out.is_empty() {
            buf.snapshot_rope().slice(0, buf.len(), &mut out);
        }
        out
    }

    // A view that records every callback for assertions.
    #[derive(Default)]
    struct RecorderView {
        events: std::sync::Arc<Mutex<Vec<RecorderEvent>>>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum RecorderEvent {
        Intercept { pre_len: u64 },
        OnEdit { post_len: u64, inserted: u64 },
    }

    impl View for RecorderView {
        fn intercept_edit<'a>(
            &mut self,
            ctx: &crate::view::InterceptContext,
            op: EditOp<'a>,
        ) -> Result<EditOp<'a>, BufferError> {
            self.events.lock().unwrap().push(RecorderEvent::Intercept {
                pre_len: ctx.buf_len,
            });
            Ok(op)
        }
        fn on_edit(&mut self, buf: &Buffer, edit: &Edit) -> Result<(), BufferError> {
            self.events.lock().unwrap().push(RecorderEvent::OnEdit {
                post_len: buf.len(),
                inserted: edit.inserted_len,
            });
            Ok(())
        }
    }

    // A view that rewrites every Insert into Insert with reversed bytes.
    // Used to confirm intercept_edit runs *before* the rope edit and
    // its rewrite is what reaches the rope.
    struct ReverseInsertView;
    impl View for ReverseInsertView {
        fn intercept_edit<'a>(
            &mut self,
            _ctx: &crate::view::InterceptContext,
            op: EditOp<'a>,
        ) -> Result<EditOp<'a>, BufferError> {
            // Cannot return EditOp with owned bytes given the lifetime
            // constraint; instead pass through and use Replace with the
            // same range. Demonstration uses a different transform:
            // unconditionally reject deletes.
            match op {
                EditOp::Delete { .. } => Err(BufferError::NothingToUndo),
                other => Ok(other),
            }
        }
    }

    // ----- identity / construction -----

    #[test]
    fn buffer_id_is_unique() {
        let a = BufferId::next();
        let b = BufferId::next();
        assert_ne!(a, b);
    }

    #[test]
    fn new_is_empty_and_clean() {
        let b = fresh();
        assert!(b.is_empty());
        assert!(!b.is_modified());
        assert_eq!(b.name(), "*scratch*");
        assert_eq!(b.view_count(), 0);
    }

    #[test]
    fn from_bytes_preserves_content() {
        let b = Buffer::from_bytes(BufferId::next(), "hello.txt", b"hello world");
        assert_eq!(b.len(), 11);
        assert_eq!(collect(&b), b"hello world");
        assert!(!b.is_modified());
    }

    // ----- view attach / detach -----

    #[test]
    fn attach_and_detach_view() {
        let mut b = fresh();
        let id = b.attach_view(Box::new(RecorderView::default()));
        assert_eq!(b.view_count(), 1);
        let detached = b.detach_view(id).expect("present");
        let _ = detached;
        assert_eq!(b.view_count(), 0);
        assert!(b.detach_view(id).is_none());
    }

    // ----- edit flow -----

    #[test]
    fn apply_insert_edit_updates_state() {
        let mut b = fresh();
        let edit = b
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"abc",
            })
            .unwrap();
        assert_eq!(edit.range, Range::new(0, 0));
        assert_eq!(edit.inserted_len, 3);
        assert_eq!(b.len(), 3);
        assert!(b.is_modified());
        assert_eq!(collect(&b), b"abc");
    }

    #[test]
    fn marks_apply_insertion_gravity() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abcd");
        let left = b.create_mark(2, MarkGravity::Left).unwrap();
        let right = b.create_mark(2, MarkGravity::Right).unwrap();

        b.apply_edit(EditOp::Insert {
            pos: 2,
            bytes: b"XX",
        })
        .unwrap();

        assert_eq!(b.mark_pos(left), Some(2));
        assert_eq!(b.mark_pos(right), Some(4));
    }

    #[test]
    fn marks_shift_and_clamp_through_delete() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abcdef");
        let before = b.create_mark(1, MarkGravity::Right).unwrap();
        let inside = b.create_mark(3, MarkGravity::Left).unwrap();
        let after = b.create_mark(5, MarkGravity::Right).unwrap();

        b.apply_edit(EditOp::Delete {
            range: Range::new(2, 4),
        })
        .unwrap();

        assert_eq!(b.mark_pos(before), Some(1));
        assert_eq!(b.mark_pos(inside), Some(2));
        assert_eq!(b.mark_pos(after), Some(3));
    }

    #[test]
    fn marks_follow_undo_and_redo() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abcd");
        let mark = b.create_mark(3, MarkGravity::Right).unwrap();

        b.apply_edit(EditOp::Insert {
            pos: 1,
            bytes: b"XX",
        })
        .unwrap();
        assert_eq!(b.mark_pos(mark), Some(5));

        b.undo().unwrap();
        assert_eq!(b.mark_pos(mark), Some(3));

        b.redo().unwrap();
        assert_eq!(b.mark_pos(mark), Some(5));
    }

    #[test]
    fn intercept_runs_before_on_edit_and_before_rope_mutation() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"hi");
        let view = RecorderView::default();
        let events = view.events.clone();
        b.attach_view(Box::new(view));

        let _ = b
            .apply_edit(EditOp::Insert {
                pos: 2,
                bytes: b"!",
            })
            .unwrap();

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        // intercept runs first, observing pre-edit rope (len 2).
        assert_eq!(events[0], RecorderEvent::Intercept { pre_len: 2 });
        // on_edit runs after, observing post-edit rope (len 3) with the
        // edit description.
        assert_eq!(
            events[1],
            RecorderEvent::OnEdit {
                post_len: 3,
                inserted: 1
            }
        );
    }

    #[test]
    fn intercept_can_reject_edit() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abc");
        b.attach_view(Box::new(ReverseInsertView));

        // Delete is rejected by the view.
        let err = b.apply_edit(EditOp::Delete {
            range: Range::new(0, 1),
        });
        assert!(err.is_err());
        // Buffer state unchanged.
        assert_eq!(b.len(), 3);
        assert!(!b.is_modified());
        // Undo stack unchanged.
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    }

    #[test]
    fn delete_and_replace() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"hello world");
        b.apply_edit(EditOp::Delete {
            range: Range::new(5, 11),
        })
        .unwrap();
        assert_eq!(collect(&b), b"hello");
        b.apply_edit(EditOp::Replace {
            range: Range::new(0, 5),
            bytes: b"HELLO",
        })
        .unwrap();
        assert_eq!(collect(&b), b"HELLO");
    }

    // ----- undo / redo -----

    #[test]
    fn undo_round_trips_to_original() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"original");
        b.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"X",
        })
        .unwrap();
        b.apply_edit(EditOp::Insert {
            pos: 4,
            bytes: b"Y",
        })
        .unwrap();
        b.apply_edit(EditOp::Delete {
            range: Range::new(2, 5),
        })
        .unwrap();
        assert_ne!(collect(&b), b"original");

        b.undo().unwrap();
        b.undo().unwrap();
        b.undo().unwrap();
        assert_eq!(collect(&b), b"original");
        assert!(!b.is_modified());

        // Nothing left to undo.
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    }

    #[test]
    fn redo_replays_undone_edit() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abc");
        b.apply_edit(EditOp::Insert {
            pos: 3,
            bytes: b"def",
        })
        .unwrap();
        assert_eq!(collect(&b), b"abcdef");
        b.undo().unwrap();
        assert_eq!(collect(&b), b"abc");
        b.redo().unwrap();
        assert_eq!(collect(&b), b"abcdef");
        assert!(b.is_modified());
    }

    #[test]
    fn empty_insert_is_a_noop() {
        // Inserting zero bytes must not mark the buffer modified or push
        // anything onto the undo stack.
        let mut b = fresh();
        b.apply_edit(EditOp::Insert { pos: 0, bytes: b"" }).unwrap();
        assert!(!b.is_modified());
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    }

    #[test]
    fn empty_range_delete_is_a_noop() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abc");
        b.apply_edit(EditOp::Delete {
            range: Range::new(1, 1),
        })
        .unwrap();
        assert!(!b.is_modified());
        assert_eq!(collect(&b), b"abc");
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    }

    #[test]
    fn replace_empty_with_empty_is_a_noop() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abc");
        b.apply_edit(EditOp::Replace {
            range: Range::new(2, 2),
            bytes: b"",
        })
        .unwrap();
        assert!(!b.is_modified());
        assert_eq!(collect(&b), b"abc");
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    }

    #[test]
    fn forward_edit_clears_redo() {
        let mut b = Buffer::from_bytes(BufferId::next(), "test", b"abc");
        b.apply_edit(EditOp::Insert {
            pos: 3,
            bytes: b"d",
        })
        .unwrap();
        b.undo().unwrap();
        assert!(b.redo().is_ok());
        // Set up redo state again.
        b.undo().unwrap();
        // Forward edit clears redo.
        b.apply_edit(EditOp::Insert {
            pos: 3,
            bytes: b"X",
        })
        .unwrap();
        assert!(matches!(b.redo(), Err(BufferError::NothingToRedo)));
    }

    #[test]
    fn random_edit_then_full_undo_recovers_original() {
        // Reuses the rope's fuzz pattern but at the buffer level: any
        // arbitrary sequence must undo to the starting bytes exactly.
        let mut b = Buffer::from_bytes(
            BufferId::next(),
            "test",
            &(0..512u32).map(|i| (i % 251) as u8).collect::<Vec<_>>(),
        );
        let original = collect(&b);

        let mut rng_state: u64 = 0x1234_5678;
        let mut rng = || {
            rng_state = rng_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (rng_state >> 33) as u32
        };

        let mut edit_count = 0usize;
        for _ in 0..200 {
            let len = b.len();
            match rng() % 3 {
                0 => {
                    let pos = u64::from(rng()) % (len + 1);
                    let n = (rng() % 32 + 1) as usize;
                    let bytes: Vec<u8> = (0..n)
                        .map(|i| (rng() as u8).wrapping_add(i as u8))
                        .collect();
                    b.apply_edit(EditOp::Insert { pos, bytes: &bytes }).unwrap();
                    edit_count += 1;
                }
                1 if len > 0 => {
                    let s = u64::from(rng()) % len;
                    let e = s + u64::from(rng()) % (len - s + 1).max(1);
                    let e = e.min(len);
                    if s < e {
                        b.apply_edit(EditOp::Delete {
                            range: Range::new(s, e),
                        })
                        .unwrap();
                        edit_count += 1;
                    }
                }
                _ if len > 0 => {
                    let s = u64::from(rng()) % len;
                    let e = s + u64::from(rng()) % (len - s + 1).max(1);
                    let e = e.min(len);
                    if s < e {
                        let n = (rng() % 16) as usize;
                        let bytes: Vec<u8> = (0..n)
                            .map(|i| (rng() as u8).wrapping_add(i as u8))
                            .collect();
                        b.apply_edit(EditOp::Replace {
                            range: Range::new(s, e),
                            bytes: &bytes,
                        })
                        .unwrap();
                        edit_count += 1;
                    }
                }
                _ => {}
            }
        }

        for _ in 0..edit_count {
            b.undo().unwrap();
        }
        assert_eq!(collect(&b), original);
        assert!(!b.is_modified());
    }
}
