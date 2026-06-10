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

use std::path::{Path, PathBuf};

use crate::file_io::FileMeta;
use crate::rope::{Edit, Position, Range, Rope, RopeError};
use crate::view::{InterceptContext, View};

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Re-export of `pmacs_protocol::BufferId` (moved there in session 1
/// of the `pmacs-gpu` arc — see `docs/pmacs-gpu-design.md`). Existing
/// `crate::buffer::BufferId` import paths continue to resolve through
/// this re-export; new consumers (`pmacs-gpu`, debug tools) should
/// depend on `pmacs-protocol` directly.
pub use pmacs_protocol::BufferId;

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
    /// Path this buffer is bound to on disk, if any (T M4.5 L1:
    /// relocated here from `EditorCore` so cross-file navigation can
    /// keep each buffer's identity straight — the v0.1 single-file
    /// `EditorCore.file_path` shortcut no longer holds once multiple
    /// files are open). `None` for scratch / unsaved buffers.
    file_path: Option<PathBuf>,
    /// Filesystem metadata captured at the last successful load/save,
    /// used for external-change detection. Relocated alongside
    /// [`Self::file_path`].
    file_meta: Option<FileMeta>,
    /// True while an edit is in flight on this buffer (T M7.4).
    /// Set by [`Buffer::begin_edit`], cleared by [`Buffer::end_edit`].
    /// A re-entrant `apply_edit` / `apply_edit_skip_intercepts` while
    /// the flag is set returns [`BufferError::ConcurrentEdit`] rather
    /// than mutating the rope mid-intercept; cross-buffer re-entry
    /// is unaffected.
    editing_in_progress: bool,
    /// Optional CRDT-backed state (T M10.2). When `Some`, every
    /// successful edit (forward, undo, or redo) is also applied to
    /// the CRDT, keeping the invariant `rope contents ≡ CRDT
    /// projection` at all times. Set at construction via
    /// [`Buffer::new_with_crdt`] / [`Buffer::from_bytes_with_crdt`]
    /// or attached to an existing buffer via
    /// [`Buffer::upgrade_to_crdt`]; never cleared (per the M10.2
    /// "Option set at construction, not toggled later" rule).
    ///
    /// Workers consume the rope projection via
    /// [`Buffer::snapshot_rope`] and never see the CRDT directly,
    /// per the rope-projection redirect (M10.1, §sec:m10-crdt-choice).
    #[cfg(feature = "crdt")]
    crdt: Option<crate::crdt::CrdtState>,
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
            file_path: None,
            file_meta: None,
            editing_in_progress: false,
            #[cfg(feature = "crdt")]
            crdt: None,
        }
    }

    /// The path this buffer is bound to on disk, if any.
    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// Bind (or unbind, with `None`) this buffer to a disk path.
    pub fn set_file_path(&mut self, path: Option<PathBuf>) {
        self.file_path = path;
    }

    /// Filesystem metadata from the last load/save, if any.
    #[must_use]
    pub fn file_meta(&self) -> Option<&FileMeta> {
        self.file_meta.as_ref()
    }

    /// Record filesystem metadata (after a successful load/save).
    pub fn set_file_meta(&mut self, meta: Option<FileMeta>) {
        self.file_meta = meta;
    }

    /// Construct an empty CRDT-backed buffer.
    ///
    /// `peer_id` identifies this frontend's edits in the CRDT op
    /// stream; M10.4's per-frontend undo and M10.5's wire-protocol
    /// op messages consume it. Threading: main thread only.
    ///
    /// The `Option<CrdtState>` is set here and never toggled
    /// afterward (M10.2 "set at construction, not later" rule).
    /// To attach a CRDT to an existing rope-only buffer, use
    /// [`Buffer::upgrade_to_crdt`].
    #[cfg(feature = "crdt")]
    pub fn new_with_crdt(
        id: BufferId,
        name: impl Into<String>,
        peer_id: u64,
    ) -> Result<Self, BufferError> {
        let mut buf = Self::new(id, name);
        buf.crdt = Some(crate::crdt::CrdtState::new(peer_id)?);
        Ok(buf)
    }

    /// Construct a CRDT-backed buffer seeded with the given bytes.
    ///
    /// The bytes are loaded into the rope (byte-faithful) and into
    /// the CRDT (UTF-8-normalized via `String::from_utf8_lossy`,
    /// matching [`crate::crdt::CrdtState::from_bytes`]). For valid
    /// UTF-8 input the two are identical; for ill-formed input the
    /// CRDT loses ill-formed-byte detail to U+FFFD replacement
    /// while the rope retains the original bytes — a documented
    /// divergence the v0.1 `from_bytes` already accepted.
    ///
    /// Threading: main thread only.
    #[cfg(feature = "crdt")]
    pub fn from_bytes_with_crdt(
        id: BufferId,
        name: impl Into<String>,
        bytes: &[u8],
        peer_id: u64,
    ) -> Result<Self, BufferError> {
        let mut buf = Self::from_bytes(id, name, bytes);
        buf.crdt = Some(crate::crdt::CrdtState::from_bytes(peer_id, bytes)?);
        Ok(buf)
    }

    /// Attach a CRDT to an existing rope-only buffer.
    ///
    /// Materializes a fresh `CrdtState` seeded from the buffer's
    /// current rope contents. Existing intercepts, marks, views,
    /// undo stack, revision, and `is_modified` flag are preserved.
    /// The buffer's `BufferId` is unchanged so existing references
    /// stay valid.
    ///
    /// **Undo-history loss**: Pre-upgrade entries in the v0.1 undo
    /// stack (and redo stack) are cleared explicitly during the
    /// upgrade. Post-upgrade undo routes through loro's `UndoManager`,
    /// which has no knowledge of pre-upgrade edits. Users wishing to
    /// preserve undo history should attach collaboration before
    /// making edits, or accept that mid-session collaboration loses
    /// prior undo state. A v0.2+ refinement preserving v0.1 history
    /// alongside `UndoManager` is feasible but out of scope for v1.0
    /// (the synthesis from v0.1 entries → CRDT ops is structurally
    /// problematic since the pre-upgrade ops have no `peer_id` to
    /// attribute to `UndoManager`).
    ///
    /// Used by M10.8 (multi-frontend instance state) when a v0.1
    /// frontend's buffer is promoted to CRDT-backed at attach time
    /// because a v1.0 frontend has joined the session. M10.2 ships
    /// the API surface; the M10.8 caller wires invocation.
    ///
    /// Returns an error if the CRDT was already attached (the
    /// "set once" rule); callers should check
    /// [`Buffer::is_crdt_backed`] if uncertain.
    ///
    /// Threading: main thread only.
    #[cfg(feature = "crdt")]
    pub fn upgrade_to_crdt(&mut self, peer_id: u64) -> Result<(), BufferError> {
        if self.crdt.is_some() {
            // Already CRDT-backed. The "set once" rule rejects re-
            // attachment; callers should not invoke this on an
            // already-upgraded buffer.
            return Err(BufferError::CrdtRejected {
                reason: "buffer is already CRDT-backed".to_owned(),
            });
        }
        // Materialize CRDT state from the current rope. The rope's
        // bytes are read in chunks to avoid one large allocation
        // (matters for the 10MB+ case the M10.1 audit measured at
        // 92ms cold-path materialization).
        let rope_len = self.rope.len();
        let mut bytes = vec![0u8; rope_len as usize];
        if rope_len > 0 {
            self.rope.slice(0, rope_len, &mut bytes);
        }
        self.crdt = Some(crate::crdt::CrdtState::from_bytes(peer_id, &bytes)?);
        // M10.4 reframe: clear v0.1 undo/redo stacks on upgrade. The
        // pre-upgrade entries can't be replayed through UndoManager
        // (no peer_id attribution); leaving them in self.undo would
        // make them unreachable through CRDT-mode undo (which
        // bypasses self.undo). Clear explicitly + log so the data
        // loss is visible. v0.2+ may revisit (preserve alongside
        // UndoManager, route undo to v0.1 stack first then switch).
        if !self.undo.is_empty() || !self.redo.is_empty() {
            // The buffer-registry / Lua-binding layer wraps this in
            // a user-facing notification; the log here is for
            // developer audit. eprintln intentionally for visibility
            // at upgrade time without taking a dep on pmacs's error
            // surface from inside the rope/buffer layer.
            eprintln!(
                "Buffer {} ({:?}): upgrade_to_crdt clearing {} undo + {} redo entries; \
                 v0.1 history is not preserved across CRDT mode upgrade. See M10.4 audit doc \
                 for the v0.2+ refinement path.",
                self.name,
                self.id,
                self.undo.len(),
                self.redo.len()
            );
        }
        self.undo.clear();
        self.redo.clear();
        Ok(())
    }

    /// Whether this buffer is CRDT-backed.
    ///
    /// Threading: main thread only.
    #[cfg(feature = "crdt")]
    #[must_use]
    pub fn is_crdt_backed(&self) -> bool {
        self.crdt.is_some()
    }

    /// Read-only access to the CRDT state.
    ///
    /// Used by the consistency property test
    /// (`rope ≡ CRDT projection`) and by T M10.10's daemon-side
    /// `BufferSnapshot` export: the dispatcher calls
    /// `crdt_state().export_snapshot()` on each active buffer to
    /// bootstrap a newly-attaching frontend's `BufferMirror`.
    ///
    /// Workers continue to consume the rope projection per M10.1's
    /// redirect; CRDT access is main-thread-only and limited to the
    /// snapshot-export + wire-protocol paths.
    #[cfg(feature = "crdt")]
    pub fn crdt_state(&self) -> Option<&crate::crdt::CrdtState> {
        self.crdt.as_ref()
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

    /// T M10.10 Finding 3 — apply a remotely-produced CRDT op to
    /// this buffer.
    ///
    /// Used by the daemon's `FrontendEvent::CrdtOp` handler when a
    /// replica frontend forwards a CRDT op. The flow:
    ///
    /// 1. `crdt.import_updates_with_text_deltas(op_bytes)` integrates
    ///    the remote op and captures Loro's text projection delta.
    /// 2. Apply the common single-insert shape directly to the rope.
    ///    Deletes and compound updates conservatively fall back to a
    ///    post-import materialization + contiguous diff.
    /// 3. Apply the rope stages (rope mutation + mark adjustment +
    ///    revision bump + modified flag + `on_edit` broadcast).
    ///    Skips the CRDT-application stage (already done in step 2)
    ///    AND the undo push (remote ops aren't locally undoable per
    ///    M10.4's per-peer undo design — loro's `UndoManager` tracks
    ///    history).
    ///
    /// # Why the diff-then-EditOp shape
    ///
    /// `on_edit` subscribers (`TextView`'s line cache, syntax
    /// highlighter, overlay style maps, marks) require Edit
    /// descriptions to maintain their incremental state. A naive
    /// "replace rope wholesale" approach (set `self.rope =
    /// new_materialized`) bypasses these subscribers — caches go
    /// stale, marks lose translation. The single-Replace `EditOp`
    /// preserves incremental updates throughout.
    ///
    /// # Limitations / v0.2+ work
    ///
    /// - Compound CRDT ops carrying multiple inserts/deletes
    ///   collapse to one Replace covering the whole changed region.
    ///   For M10.10's per-keystroke-op flow this is the actual shape
    ///   (one insert OR one delete per op); compound ops appearing
    ///   in v0.2+ would land here as a single coarse Replace,
    ///   acceptable but less efficient than per-sub-op processing.
    /// - Marks within the replaced range are still subject to the
    ///   existing mark-adjustment-for-Replace logic; if mark
    ///   semantics need finer-grained handling for multi-op CRDT
    ///   updates, v0.2+ may migrate marks to loro cursor primitives.
    ///
    /// # Errors
    ///
    /// - Returns `BufferError::CrdtRejected` if the buffer isn't
    ///   CRDT-backed (caller should upgrade first per
    ///   `send_buffer_snapshots`/`ensure_active_buffer_crdt_backed`).
    /// - Returns `BufferError::CrdtRejected` if `import_updates`
    ///   fails (e.g., malformed op bytes from a buggy peer).
    ///
    /// Returns `Ok(None)` if the imported op produced no actual
    /// content change (rare; could happen for already-integrated
    /// ops in a CRDT-redundant edge case).
    #[cfg(feature = "crdt")]
    pub fn apply_remote_crdt_op(&mut self, op_bytes: &[u8]) -> Result<Option<Edit>, BufferError> {
        if self.editing_in_progress {
            return Err(BufferError::ConcurrentEdit {
                id: self.id,
                name: self.name.clone(),
            });
        }
        let Some(crdt) = self.crdt.as_ref() else {
            return Err(BufferError::CrdtRejected {
                reason: "remote CrdtOp requires CRDT-backed buffer; daemon \
                         should call upgrade_to_crdt first"
                    .to_owned(),
            });
        };

        // Integrate the remote op and capture Loro's projection diff.
        // Optimistic GUI typing produces one Insert delta, so handle
        // that shape without copying or materializing the document.
        let text_deltas = crdt
            .import_updates_with_text_deltas(op_bytes)
            .map_err(|e| BufferError::CrdtRejected {
                reason: format!("import_updates: {e:?}"),
            })?;
        if let Some((unicode_pos, inserted)) = single_remote_text_insert(&text_deltas)
            && let Some(byte_pos) = crdt.unicode_to_utf8_pos(unicode_pos)
        {
            let byte_pos = byte_pos as Position;
            let mut views = std::mem::take(&mut self.views);
            let result =
                self.run_remote_rope_stages(&mut views, byte_pos, byte_pos, inserted.as_bytes());
            self.views = views;
            return result.map(Some);
        }

        // Conservative fallback for deletes, compound updates, and
        // already-integrated ops. The rope is still the pre-import
        // projection, so it remains the source for the old bytes.
        let old_len = self.rope.len();
        let mut old_bytes = vec![0u8; old_len as usize];
        if old_len > 0 {
            self.rope.slice(0, old_len, &mut old_bytes);
        }
        let new_content = crdt.materialize_string();
        let new_bytes = new_content.as_bytes();

        // Compute common prefix/suffix at byte level, then
        // **back off to UTF-8 char boundaries** in both strings.
        //
        // # Post-audit-round-4 F25: char-boundary alignment
        //
        // Naively splitting on byte equality can land mid-codepoint
        // for compound CRDT updates that change a single character.
        // Example: 'é' (`0xC3 0xA9`) → 'è' (`0xC3 0xA8`). Byte
        // prefix = 1; range_start = 1 puts the rope edit's start
        // inside the first codepoint, so the resulting `inserted`
        // slice (`[0xA8]`) is not valid UTF-8 and downstream
        // consumers (TextView line indexing, char-aware cursor
        // motion) get an invalid byte stream.
        //
        // Fix: after computing byte-level prefix and suffix, walk
        // both bounds outward (decreasing prefix, decreasing suffix)
        // until they sit on char boundaries in BOTH old_str and
        // new_str. `str::is_char_boundary(n)` is the standard test.
        // The result: `range_start..range_end` always covers
        // complete codepoints in both old and new content; the
        // `inserted` slice is always a valid UTF-8 substring.
        let old_str =
            std::str::from_utf8(&old_bytes).expect("rope content is UTF-8 by project invariant");
        let new_str = new_content.as_str();

        let mut prefix = old_bytes
            .iter()
            .zip(new_bytes.iter())
            .take_while(|(a, b)| a == b)
            .count();
        while prefix > 0 && !old_str.is_char_boundary(prefix) {
            prefix -= 1;
        }
        // Cap suffix so prefix and suffix don't overlap on either side.
        let max_suffix = (old_bytes.len() - prefix).min(new_bytes.len() - prefix);
        let mut suffix = old_bytes
            .iter()
            .rev()
            .zip(new_bytes.iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
            .min(max_suffix);
        while suffix > 0
            && (!old_str.is_char_boundary(old_bytes.len() - suffix)
                || !new_str.is_char_boundary(new_bytes.len() - suffix))
        {
            suffix -= 1;
        }

        if prefix + suffix == old_bytes.len() && prefix + suffix == new_bytes.len() {
            // Pre- and post-content are identical — import was a
            // no-op (already-integrated op, or content-equivalent
            // concurrent edit). Skip rope mutation; return None to
            // signal "nothing changed."
            return Ok(None);
        }

        let range_start = prefix as Position;
        let range_end = (old_bytes.len() - suffix) as Position;
        let inserted = &new_bytes[prefix..new_bytes.len() - suffix];

        // Apply rope stages without re-applying to CRDT
        // (CRDT was applied above in step 2) and without undo push
        // (remote ops aren't locally undoable per M10.4).
        let mut views = std::mem::take(&mut self.views);
        let result = self.run_remote_rope_stages(&mut views, range_start, range_end, inserted);
        self.views = views;
        result.map(Some)
    }

    /// T M10.10 post-audit-round-4 F26 — verify that importing the
    /// remote update `bytes` would attribute every new op to
    /// `expected_peer_id`. Forked-import; doesn't mutate the buffer.
    ///
    /// Returns `Ok(())` on match (or non-CRDT buffer — caller's
    /// other validations gate that case). Returns `Err(actual)` for
    /// the first peer mismatch found.
    ///
    /// The daemon's `validate_remote_crdt_op` calls this after the
    /// other identity / scope checks to ensure the loro-internal
    /// peer attribution agrees with the wire wrapper's
    /// `op.peer_id` (and therefore with the authenticated source).
    #[cfg(feature = "crdt")]
    pub fn validate_remote_op_peer_ids(
        &self,
        expected_peer_id: u64,
        bytes: &[u8],
    ) -> Result<(), u64> {
        if let Some(crdt) = self.crdt.as_ref() {
            crdt.validate_update_peer_ids(expected_peer_id, bytes)
        } else {
            // Non-CRDT buffer: the apply will fail downstream with a
            // clearer error. Nothing to validate here.
            Ok(())
        }
    }

    /// T M10.10 — rope-stages-only path for remote CRDT ops. Mirrors
    /// `run_rope_edit_and_broadcast`'s stages 2–4 but skips CRDT
    /// application (already done) and undo push (remote ops aren't
    /// locally undoable). Always called from
    /// [`apply_remote_crdt_op`](Self::apply_remote_crdt_op).
    #[cfg(feature = "crdt")]
    fn run_remote_rope_stages(
        &mut self,
        views: &mut [(ViewId, Box<dyn View>)],
        range_start: Position,
        range_end: Position,
        inserted: &[u8],
    ) -> Result<Edit, BufferError> {
        // Stage 2: rope edit (single Replace covering the diff).
        let edit = self.rope.replace(range_start, range_end, inserted)?;

        // Stage 3: state update (mark adjustment + revision bump +
        // modified flag; no undo push for remote ops).
        let pre_range = edit.range;
        let inserted_len = edit.inserted_len;
        self.rope = edit.new_rope.clone();
        self.adjust_marks_for_edit(pre_range, inserted_len);
        self.is_modified = true;
        self.revision = self.revision.wrapping_add(1);

        // Stage 4: broadcast on_edit so views update incrementally
        // (TextView line cache, syntax highlighter, overlays, etc.).
        for (_, view) in views.iter_mut() {
            view.on_edit(self, &edit)?;
        }

        Ok(edit)
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

    /// T M10.2: apply an `EditOp` to the CRDT, return the lossy-
    /// normalized byte payload (if any) so the rope can mirror it,
    /// AND (Day 3) the wire-format CRDT op bytes for the originating
    /// edit so the resulting `Edit::crdt_op` carries them.
    ///
    /// Per Q2 (defense-in-depth): the CRDT op runs first; on failure
    /// the rope is untouched. The bytes are converted to UTF-8 via
    /// `from_utf8_lossy` (matching `CrdtState::from_bytes`); for valid
    /// UTF-8 inputs this is a no-op, for ill-formed bytes the CRDT
    /// sees U+FFFD substitution and the rope must too (to preserve
    /// the `rope ≡ CRDT projection` invariant).
    ///
    /// Day 3 addition: the version-capture / export idiom captures
    /// pre-version BEFORE applying ops, then exports the delta AFTER.
    /// This produces the wire-format bytes for THIS edit's ops (one
    /// op for Insert/Delete, two ops for Replace). The bytes are
    /// what M10.5 (wire protocol) sends across the network.
    ///
    /// Returns `(normalized_bytes, crdt_op)`:
    ///
    /// * `normalized_bytes`: `Some` if the rope must mirror lossy-
    ///   converted bytes (UTF-8 normalization happened); `None` if
    ///   the original bytes round-trip cleanly OR for delete-only ops.
    /// * `crdt_op`: `Some` carrying `peer_id` + wire bytes if any
    ///   CRDT op was applied; `None` for true no-op edits (Q5
    ///   detection path: pre-checked at the `EditOp` level so true
    ///   no-ops skip the CRDT path entirely; this function isn't
    ///   invoked for them).
    #[cfg(feature = "crdt")]
    fn apply_to_crdt_then_normalize_bytes(
        crdt: &crate::crdt::CrdtState,
        op: &EditOp<'_>,
    ) -> Result<CrdtRoutingResult, BufferError> {
        // Capture the pre-edit oplog frontier so the post-edit export
        // returns exactly the ops produced by THIS edit. Loro's
        // transactional model gives a consistent before/after pair.
        let pre_version = crdt.version();

        let normalized: Option<Vec<u8>> = match op {
            EditOp::Insert { pos, bytes } => {
                if bytes.is_empty() {
                    // Pre-checked at the caller (no-op detection),
                    // but defensive-return-None-here in case a future
                    // caller forgets.
                    return Ok((None, None));
                }
                let s = String::from_utf8_lossy(bytes);
                crdt.insert(*pos as usize, &s)?;
                if matches!(s, std::borrow::Cow::Borrowed(_)) {
                    None
                } else {
                    Some(s.into_owned().into_bytes())
                }
            }
            EditOp::Delete { range } => {
                if range.is_empty() {
                    return Ok((None, None));
                }
                crdt.delete(range.start as usize, range.len() as usize)?;
                None
            }
            EditOp::Replace { range, bytes } => {
                // Two CRDT ops (no splice_utf8 in loro 1.12 per the
                // morning audit). Order: delete, then insert. If
                // delete succeeds and insert fails, the CRDT is
                // mid-transaction (range deleted but replacement
                // not inserted) and the rope is unchanged. This is
                // an invariant violation; loro's insert is
                // expected to succeed if the position is valid
                // (which it is by construction). Treat insert
                // failure here as a bug worth surfacing.
                if !range.is_empty() {
                    crdt.delete(range.start as usize, range.len() as usize)?;
                }
                if bytes.is_empty() {
                    // Replace { non-empty range, empty bytes } is
                    // semantically a delete; the delete above already
                    // ran, no further op is needed. Fall through to
                    // exporting the delta below.
                    None
                } else {
                    let s = String::from_utf8_lossy(bytes);
                    crdt.insert(range.start as usize, &s)?;
                    if matches!(s, std::borrow::Cow::Borrowed(_)) {
                        None
                    } else {
                        Some(s.into_owned().into_bytes())
                    }
                }
            }
        };

        // Export the wire bytes for the delta produced by the ops
        // above. This is the `crdt_op` field on the resulting Edit.
        let bytes = crdt.export_updates_since(&pre_version)?;
        let crdt_op = Box::new(crate::rope::CrdtOp {
            peer_id: crdt.peer_id(),
            bytes,
        });
        Ok((normalized, Some(crdt_op)))
    }

    fn run_rope_edit_and_broadcast(
        &mut self,
        views: &mut [(ViewId, Box<dyn View>)],
        current: &EditOp<'_>,
    ) -> Result<Edit, BufferError> {
        // T M10.2: CRDT routing (Q2 defense-in-depth ordering — CRDT
        // first, then rope; if CRDT errors, abort before rope mutation).
        // The byte → str conversion uses `from_utf8_lossy` per the
        // documented divergence: ill-formed bytes become U+FFFD in the
        // CRDT and (under v0.1 byte-permissive rope) would diverge. To
        // keep the invariant `rope ≡ CRDT projection`, the rope ALSO
        // sees the lossy bytes when CRDT mode is active. v0.1 mode
        // (CRDT off) is unchanged.
        #[cfg(feature = "crdt")]
        let (lossy_owned, captured_crdt_op): (
            Option<Vec<u8>>,
            Option<Box<crate::rope::CrdtOp>>,
        ) = match (&self.crdt, is_no_op_edit(current)) {
            (Some(crdt), false) => Self::apply_to_crdt_then_normalize_bytes(crdt, current)?,
            _ => (None, None),
        };

        // Stage 2: rope edit. In CRDT mode, the EditOp's byte payload
        // is replaced by the lossy-normalized version so the rope
        // matches the CRDT projection (the invariant the proptest
        // pins).
        #[cfg(feature = "crdt")]
        let normalized: Option<EditOp<'_>> = lossy_owned.as_deref().map(|bytes| match current {
            EditOp::Insert { pos, .. } => EditOp::Insert { pos: *pos, bytes },
            EditOp::Replace { range, .. } => EditOp::Replace {
                range: *range,
                bytes,
            },
            EditOp::Delete { range } => EditOp::Delete { range: *range },
        });
        #[cfg(feature = "crdt")]
        let current = normalized.as_ref().unwrap_or(current);

        #[cfg_attr(not(feature = "crdt"), allow(unused_mut))]
        let mut edit = match current {
            EditOp::Insert { pos, bytes } => self.rope.insert(*pos, bytes)?,
            EditOp::Delete { range } => self.rope.delete(range.start, range.end)?,
            EditOp::Replace { range, bytes } => self.rope.replace(range.start, range.end, bytes)?,
        };

        // T M10.2 Day 3: populate the Edit's crdt_op field with the
        // wire-format bytes captured by the CRDT routing above.
        // Mutation pattern: rope returns Edit with crdt_op = None;
        // Buffer mutates the field before the Edit is returned to
        // any consumer. The "moment of partial construction" is
        // internal to apply_edit; consumers always see fully-
        // constructed Edits. Don't refactor this away under
        // "Edits should be immutable" reasoning — the alternative
        // is double-allocation per edit.
        #[cfg(feature = "crdt")]
        {
            edit.crdt_op = captured_crdt_op;
        }

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
        // T M10.4: in CRDT mode, loro's UndoManager tracks undo
        // history; the v0.1 self.undo stack is bypassed (would grow
        // unboundedly otherwise since nothing pops it in CRDT mode).
        // The redo stack is similarly unused in CRDT mode.
        #[cfg(feature = "crdt")]
        let in_crdt_mode = self.crdt.is_some();
        #[cfg(not(feature = "crdt"))]
        let in_crdt_mode = false;
        if in_crdt_mode {
            // CRDT mode: loro's UndoManager tracks history; drop the
            // old rope (in v0.1 it's owned by the pushed UndoEntry,
            // in CRDT mode it's released here).
            drop(old_rope);
        } else {
            self.undo.push(UndoEntry {
                rope: old_rope,
                edit: EditDescription {
                    pre_range,
                    inserted_len,
                },
            });
            self.redo.clear();
        }
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
        // T M10.4: in CRDT mode, route through loro's UndoManager via
        // the materialize-and-replace path (Day 1 morning audit
        // decision — path (a)). Inverse ops are produced as proper
        // CRDT ops by UndoManager, interacting with concurrent remote
        // ops via CRDT convergence rules.
        #[cfg(feature = "crdt")]
        if self.crdt.is_some() {
            return self.undo_crdt_mode();
        }

        // v0.1 mode: pop the saved UndoEntry, swap the rope back,
        // push onto redo stack.
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
            crdt_op: None,
        };
        self.broadcast_on_edit(&inverse_edit)?;
        Ok(inverse_edit)
    }

    /// T M10.4: CRDT-mode undo via loro's `UndoManager`.
    ///
    /// Materialize-and-replace path (Day 1 morning audit decision).
    /// Inverse ops are produced by `UndoManager` as proper CRDT ops
    /// (not synthetic Replace as M10.2's path did); they interact
    /// with concurrent remote ops via CRDT convergence rules.
    ///
    /// The Edit description is derived via [`derive_replacement_edit`]:
    /// longest-common-prefix + longest-common-suffix trim against the
    /// pre-undo rope. This produces a minimal `(range, inserted_len)`
    /// covering exactly the bytes that changed, so marks adjust
    /// correctly and tree-sitter's incremental parse stays
    /// incremental. Cost: O(min(`old_len`, `new_len`)) byte compare via
    /// rope chunks; sub-ms at typical edit sizes.
    #[cfg(feature = "crdt")]
    fn undo_crdt_mode(&mut self) -> Result<Edit, BufferError> {
        // Extract everything we need from `self.crdt` before mutating
        // `self.rope` / `self.marks` / `self.revision` etc., to
        // avoid a borrow-checker conflict between the immutable
        // crdt-ref and the upcoming `&mut self` method calls.
        let (new_text, bytes, peer_id, can_undo_after) = {
            let crdt = self.crdt.as_ref().expect("checked");
            let pre_version = crdt.version();
            let undid = crdt.undo()?;
            if !undid {
                return Err(BufferError::NothingToUndo);
            }
            let new_text = crdt.materialize_string();
            let bytes = crdt.export_updates_since(&pre_version)?;
            (new_text, bytes, crdt.peer_id(), crdt.can_undo())
        };
        let new_rope = crate::rope::Rope::from_bytes(new_text.as_bytes());
        let (range, inserted_len) = derive_replacement_edit(&self.rope, &new_rope);
        self.rope = new_rope.clone();
        self.adjust_marks_for_edit(range, inserted_len);
        self.revision = self.revision.wrapping_add(1);
        // is_modified stays true while there's still anything in the
        // CRDT's undo stack (i.e. local edits not yet at the buffer's
        // saved baseline). Matches v0.1 mode's `!self.undo.is_empty()`
        // semantics translated to CrdtState's bookkeeping.
        self.is_modified = can_undo_after;

        let inverse_edit = Edit {
            new_rope,
            range,
            inserted_len,
            crdt_op: Some(Box::new(crate::rope::CrdtOp { peer_id, bytes })),
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
        // T M10.4: in CRDT mode, route through loro's UndoManager.
        #[cfg(feature = "crdt")]
        if self.crdt.is_some() {
            return self.redo_crdt_mode();
        }

        // v0.1 mode: pop the saved redo entry, swap the rope forward.
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
            crdt_op: None,
        };
        self.broadcast_on_edit(&inverse_edit)?;
        Ok(inverse_edit)
    }

    /// T M10.4: CRDT-mode redo via loro's `UndoManager`.
    ///
    /// Symmetric to [`Self::undo_crdt_mode`]; same materialize-and-
    /// replace path.
    #[cfg(feature = "crdt")]
    fn redo_crdt_mode(&mut self) -> Result<Edit, BufferError> {
        let (new_text, bytes, peer_id) = {
            let crdt = self.crdt.as_ref().expect("checked");
            let pre_version = crdt.version();
            let redid = crdt.redo()?;
            if !redid {
                return Err(BufferError::NothingToRedo);
            }
            let new_text = crdt.materialize_string();
            let bytes = crdt.export_updates_since(&pre_version)?;
            (new_text, bytes, crdt.peer_id())
        };
        let new_rope = crate::rope::Rope::from_bytes(new_text.as_bytes());
        let (range, inserted_len) = derive_replacement_edit(&self.rope, &new_rope);
        self.rope = new_rope.clone();
        self.adjust_marks_for_edit(range, inserted_len);
        self.revision = self.revision.wrapping_add(1);
        self.is_modified = true;

        let inverse_edit = Edit {
            new_rope,
            range,
            inserted_len,
            crdt_op: Some(Box::new(crate::rope::CrdtOp { peer_id, bytes })),
        };
        self.broadcast_on_edit(&inverse_edit)?;
        Ok(inverse_edit)
    }

    /// T M10.4: per-frontend undo for a specific attached frontend.
    ///
    /// **M10.11 architecture-record:** the M10.4 framing predicted
    /// that this method would dispatch by `frontend_id` to a
    /// `HashMap<FrontendId, UndoManager>` on the buffer. M10.11's
    /// Day 2 verification surfaced that loro's `UndoManager` binds
    /// to one peer at construction (`src/crdt.rs:60-65`,
    /// `loro-internal/src/undo.rs:572-672`) — you can't maintain
    /// per-peer `UndoManager` instances on a single doc. The
    /// CRDT-native per-frontend undo path lives on the **frontend**
    /// side: each `BufferMirror` holds its own `CrdtState` whose
    /// `UndoManager` is bound to that frontend's `peer_id` (see
    /// `BufferMirror::apply_local_undo` and
    /// `optimistic::frontend_event_for_keystroke`'s
    /// `OptimisticAction::Undo` arm). The frontend produces an
    /// inverse `CrdtOp` and the daemon imports it as an ordinary
    /// update. The daemon-side `Buffer::undo` (this method's
    /// no-arg sibling) remains the daemon-peer-only undo path —
    /// used for Lua-driven daemon-side edits and the v0.1 single-
    /// frontend mode.
    ///
    /// This method therefore routes `frontend_id` arguments to
    /// `Self::undo` directly: there is no per-frontend dispatch to
    /// do at the buffer level. The signature is preserved for any
    /// callers that were threading a frontend id; behavior is
    /// unchanged from the M10.4 single-frontend semantics.
    ///
    /// Threading: main thread only.
    pub fn undo_for(
        &mut self,
        _frontend_id: crate::protocol::FrontendId,
    ) -> Result<Edit, BufferError> {
        // Per the M10.11 architecture record above: per-frontend
        // undo lives frontend-side via BufferMirror's peer-bound
        // UndoManager. Daemon-side undo is daemon-peer-scoped.
        self.undo()
    }

    /// T M10.4: symmetric to [`Self::undo_for`]. Same architecture
    /// record applies: per-frontend redo lives frontend-side.
    pub fn redo_for(
        &mut self,
        _frontend_id: crate::protocol::FrontendId,
    ) -> Result<Edit, BufferError> {
        self.redo()
    }

    // T M10.4: `sync_crdt_for_history_swap` removed. M10.2 Day 2's
    // synthetic-Replace path produced inverse ops attributed to the
    // editing peer that looked like fresh edits to the CRDT (not
    // semantic undos). M10.4 replaces this with loro's UndoManager
    // (see `undo_crdt_mode` / `redo_crdt_mode` above), which produces
    // proper inverse ops that interact correctly with concurrent
    // remote edits per the M10.4 acceptance: "B's edit lands on
    // whatever surrounding text remains."

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

/// T M10.2 Day 3 type alias: the routing result tuple — lossy-
/// normalized bytes (if any) plus the captured CRDT op (if any).
/// Factored out to silence clippy's `type_complexity`.
#[cfg(feature = "crdt")]
type CrdtRoutingResult = (Option<Vec<u8>>, Option<Box<crate::rope::CrdtOp>>);

/// Recognize the hot-path projection delta produced by one remote
/// insertion. Loro's retain/delete lengths use Unicode scalar offsets;
/// the caller converts the insertion point through the post-import
/// text container before applying the UTF-8 bytes to the rope.
#[cfg(feature = "crdt")]
fn single_remote_text_insert(deltas: &[Vec<loro::TextDelta>]) -> Option<(usize, &str)> {
    let [delta] = deltas else {
        return None;
    };
    let mut cursor = 0usize;
    let mut found = None;
    for op in delta {
        match op {
            loro::TextDelta::Retain { retain, .. } => {
                cursor = cursor.checked_add(*retain)?;
            }
            loro::TextDelta::Insert { insert, .. } if insert.is_empty() => {}
            loro::TextDelta::Insert { insert, .. } => {
                if found.is_some() {
                    return None;
                }
                found = Some((cursor, insert.as_str()));
                cursor = cursor.checked_add(insert.chars().count())?;
            }
            loro::TextDelta::Delete { delete } if *delete == 0 => {}
            loro::TextDelta::Delete { .. } => return None,
        }
    }
    found
}

/// T M10.4: derive a fine-grained `(range, inserted_len)` Edit
/// description for the change from `old_rope` to `new_rope` via
/// longest-common-prefix + longest-common-suffix trim.
///
/// CRDT-mode undo/redo materializes the post-undo rope via the CRDT
/// projection (path (a) from Day 1 morning's audit). A coarse Edit
/// description `(0..old_len, new_len)` would break mark positions
/// (every mark would be moved through a full-replace) and force
/// tree-sitter to re-parse the whole document. This helper computes
/// the minimal Edit description by trimming matching prefix +
/// suffix from both ropes.
///
/// Correctness: assumes the change is a single contiguous edit
/// (which `UndoManager`.undo / .redo always produces — each undo
/// reverses one logical `apply_edit` op). For multi-edit changes
/// (concurrent remote ops applied during the undo, hypothetically)
/// the derived Edit description is still correct in the sense that
/// applying it to `old_rope` produces `new_rope` — the change just
/// covers a wider range.
///
/// Cost: O(min(`old_len`, `new_len`)) byte comparison via rope
/// chunk iteration. At 1MB doc size with one-keystroke undo, the
/// prefix walk hits the divergence point within microseconds; same
/// for the suffix walk.
#[cfg(feature = "crdt")]
fn derive_replacement_edit(old_rope: &Rope, new_rope: &Rope) -> (Range, u64) {
    let old_len = old_rope.len();
    let new_len = new_rope.len();
    if old_len == 0 && new_len == 0 {
        return (Range::new(0, 0), 0);
    }
    // Longest common prefix.
    let mut prefix = 0u64;
    let max_prefix = old_len.min(new_len);
    let chunk = 4096u64.min(max_prefix);
    while prefix < max_prefix {
        let n = chunk.min(max_prefix - prefix);
        let mut a = vec![0u8; n as usize];
        let mut b = vec![0u8; n as usize];
        old_rope.slice(prefix, prefix + n, &mut a);
        new_rope.slice(prefix, prefix + n, &mut b);
        let mismatch = a.iter().zip(b.iter()).position(|(x, y)| x != y);
        if let Some(off) = mismatch {
            prefix += off as u64;
            break;
        }
        prefix += n;
    }
    // Longest common suffix (bounded so we don't overlap with prefix).
    let mut suffix = 0u64;
    let max_suffix = (old_len - prefix).min(new_len - prefix);
    while suffix < max_suffix {
        let n = chunk.min(max_suffix - suffix);
        let mut a = vec![0u8; n as usize];
        let mut b = vec![0u8; n as usize];
        old_rope.slice(old_len - suffix - n, old_len - suffix, &mut a);
        new_rope.slice(new_len - suffix - n, new_len - suffix, &mut b);
        let mismatch = a.iter().rev().zip(b.iter().rev()).position(|(x, y)| x != y);
        if let Some(off) = mismatch {
            suffix += off as u64;
            break;
        }
        suffix += n;
    }
    let range = Range::new(prefix, old_len - suffix);
    let inserted_len = new_len - prefix - suffix;
    (range, inserted_len)
}

/// T M10.2 Day 3: pre-check an `EditOp` for the no-op case so the
/// CRDT path can be skipped entirely (Q5 detection: pre-check the
/// `EditOp` variants explicitly; truly empty edits skip the CRDT
/// path; partially-empty Replace variants delegate to insert or
/// delete semantics and are NOT no-ops).
///
/// Truly no-op cases:
/// * `Insert { bytes: empty }` — inserts nothing
/// * `Delete { range: empty }` — deletes nothing
/// * `Replace { range: empty, bytes: empty }` — neither deletes nor inserts
///
/// `Replace` with non-empty range OR non-empty bytes is NOT a no-op:
/// it has actual semantic effect (delete-only or insert-only or
/// both) that the CRDT must observe to keep the rope ≡ projection
/// invariant. The rope path's own no-op short-circuit handles the
/// truly-empty cases AFTER the rope mutation runs; this helper lets
/// the CRDT path skip the round-trip BEFORE the rope runs.
#[cfg(feature = "crdt")]
fn is_no_op_edit(op: &EditOp<'_>) -> bool {
    match op {
        EditOp::Insert { bytes, .. } => bytes.is_empty(),
        EditOp::Delete { range } => range.is_empty(),
        EditOp::Replace { range, bytes } => range.is_empty() && bytes.is_empty(),
    }
}

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
    /// The CRDT-backed buffer mode rejected an op (T M10.2). Reaches
    /// the caller when loro returns an error during edit routing — the
    /// most common case is a mid-codepoint position that loro's
    /// `insert_utf8` / `delete_utf8` rejects (per the M10.2 Day 2
    /// morning audit). Per the Q2 defense-in-depth ordering, the
    /// CRDT op is attempted before the rope mutation, so this error
    /// leaves the rope unchanged.
    #[cfg(feature = "crdt")]
    #[error("CRDT edit rejected: {reason}")]
    CrdtRejected {
        /// Human-readable reason from loro. Surfaced verbatim.
        reason: String,
    },
}

#[cfg(feature = "crdt")]
impl From<loro::LoroError> for BufferError {
    fn from(e: loro::LoroError) -> Self {
        BufferError::CrdtRejected {
            reason: e.to_string(),
        }
    }
}

#[cfg(feature = "crdt")]
impl From<loro::LoroEncodeError> for BufferError {
    fn from(e: loro::LoroEncodeError) -> Self {
        BufferError::CrdtRejected {
            reason: format!("CRDT encode failed: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::View;
    use std::sync::Mutex;

    // T M10.2 Day 5: `fresh()` helper removed — every test that used
    // it has been converted to `dual_mode_test!` and uses the
    // injected `make`/`make_bytes` factories instead. v0.1-only tests
    // construct via `Buffer::new` / `Buffer::from_bytes` directly;
    // CRDT-only tests use `Buffer::new_with_crdt` / `from_bytes_with_crdt`.

    // -----------------------------------------------------------------
    // T M10.2 Day 5 — dual-mode test harness.
    //
    // `dual_mode_test!` generates two `#[test]` entries (`<name>::v01`
    // and `<name>::crdt`) from one test body. Each entry runs the body
    // with a different buffer factory:
    //
    //   * `make(name)` → empty buffer
    //   * `make_bytes(name, bytes)` → buffer seeded with bytes
    //
    // The body picks whichever factory it needs; tests that only want
    // the empty case ignore `make_bytes` (and vice versa) via `_`-
    // prefixed names. The CRDT entry is `#[cfg(feature = "crdt")]`,
    // so v0.1 builds get only the v0.1 test.
    //
    // Why per-mode `#[test]` entries vs one test running both modes:
    // failure messages identify which mode failed without ambiguity.
    // `<name>::v01` and `<name>::crdt` show up as distinct entries in
    // the test runner's output.
    //
    // Day 5 converts the existing buffer tests that exercise the
    // apply_edit / undo / redo / intercept paths. Day 6 classifies
    // any divergences this surfaces.
    // -----------------------------------------------------------------
    macro_rules! dual_mode_test {
        ($name:ident, |$make:ident, $make_bytes:ident| $body:block) => {
            mod $name {
                use super::*;

                #[test]
                fn v01() {
                    let $make = |n: &str| Buffer::new(BufferId::next(), n.to_owned());
                    let $make_bytes = |n: &str, bytes: &[u8]| {
                        Buffer::from_bytes(BufferId::next(), n.to_owned(), bytes)
                    };
                    $body
                }

                #[cfg(feature = "crdt")]
                #[test]
                fn crdt() {
                    let $make = |n: &str| {
                        Buffer::new_with_crdt(BufferId::next(), n.to_owned(), 1)
                            .expect("CRDT-mode buffer construction failed")
                    };
                    let $make_bytes = |n: &str, bytes: &[u8]| {
                        Buffer::from_bytes_with_crdt(BufferId::next(), n.to_owned(), bytes, 1)
                            .expect("CRDT-mode from_bytes failed")
                    };
                    $body
                }
            }
        };
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

    dual_mode_test!(new_is_empty_and_clean, |make, _make_bytes| {
        let b = make("*scratch*");
        assert!(b.is_empty());
        assert!(!b.is_modified());
        assert_eq!(b.name(), "*scratch*");
        assert_eq!(b.view_count(), 0);
    });

    dual_mode_test!(from_bytes_preserves_content, |_make, make_bytes| {
        let b = make_bytes("hello.txt", b"hello world");
        assert_eq!(b.len(), 11);
        assert_eq!(collect(&b), b"hello world");
        assert!(!b.is_modified());
    });

    // ----- view attach / detach -----

    dual_mode_test!(attach_and_detach_view, |make, _make_bytes| {
        let mut b = make("*scratch*");
        let id = b.attach_view(Box::new(RecorderView::default()));
        assert_eq!(b.view_count(), 1);
        let detached = b.detach_view(id).expect("present");
        let _ = detached;
        assert_eq!(b.view_count(), 0);
        assert!(b.detach_view(id).is_none());
    });

    // ----- edit flow -----

    dual_mode_test!(apply_insert_edit_updates_state, |make, _make_bytes| {
        let mut b = make("*scratch*");
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
    });

    dual_mode_test!(marks_apply_insertion_gravity, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abcd");
        let left = b.create_mark(2, MarkGravity::Left).unwrap();
        let right = b.create_mark(2, MarkGravity::Right).unwrap();

        b.apply_edit(EditOp::Insert {
            pos: 2,
            bytes: b"XX",
        })
        .unwrap();

        assert_eq!(b.mark_pos(left), Some(2));
        assert_eq!(b.mark_pos(right), Some(4));
    });

    dual_mode_test!(marks_shift_and_clamp_through_delete, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abcdef");
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
    });

    dual_mode_test!(marks_follow_undo_and_redo, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abcd");
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
    });

    dual_mode_test!(
        intercept_runs_before_on_edit_and_before_rope_mutation,
        |_make, make_bytes| {
            let mut b = make_bytes("test", b"hi");
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
    );

    dual_mode_test!(intercept_can_reject_edit, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abc");
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
    });

    dual_mode_test!(delete_and_replace, |_make, make_bytes| {
        let mut b = make_bytes("test", b"hello world");
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
    });

    // ----- undo / redo -----

    dual_mode_test!(undo_round_trips_to_original, |_make, make_bytes| {
        let mut b = make_bytes("test", b"original");
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
    });

    dual_mode_test!(redo_replays_undone_edit, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abc");
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
    });

    dual_mode_test!(empty_insert_is_a_noop, |make, _make_bytes| {
        // Inserting zero bytes must not mark the buffer modified or push
        // anything onto the undo stack.
        let mut b = make("*scratch*");
        b.apply_edit(EditOp::Insert { pos: 0, bytes: b"" }).unwrap();
        assert!(!b.is_modified());
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    });

    dual_mode_test!(empty_range_delete_is_a_noop, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abc");
        b.apply_edit(EditOp::Delete {
            range: Range::new(1, 1),
        })
        .unwrap();
        assert!(!b.is_modified());
        assert_eq!(collect(&b), b"abc");
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    });

    dual_mode_test!(replace_empty_with_empty_is_a_noop, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abc");
        b.apply_edit(EditOp::Replace {
            range: Range::new(2, 2),
            bytes: b"",
        })
        .unwrap();
        assert!(!b.is_modified());
        assert_eq!(collect(&b), b"abc");
        assert!(matches!(b.undo(), Err(BufferError::NothingToUndo)));
    });

    dual_mode_test!(forward_edit_clears_redo, |_make, make_bytes| {
        let mut b = make_bytes("test", b"abc");
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
    });

    dual_mode_test!(
        random_edit_then_full_undo_recovers_original,
        |_make, make_bytes| {
            // Reuses the rope's fuzz pattern but at the buffer level: any
            // arbitrary sequence must undo to the starting bytes exactly.
            //
            // Seed bytes are ASCII-only (lower 7 bits) so the CRDT-mode
            // run doesn't trip on lossy UTF-8 normalization. The
            // v0.1-mode seed in the rope-level fuzz test uses 0..251
            // which includes ill-formed UTF-8; that's a rope-only
            // property and stays in `src/rope.rs`'s tests.
            let mut b = make_bytes(
                "test",
                &(0..512u32).map(|i| (i % 128) as u8).collect::<Vec<_>>(),
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
                        // ASCII-only bytes so CRDT mode's lossy-utf8
                        // normalization is a no-op (rope ≡ projection
                        // for the v0.1-byte-permissive AND CRDT-utf8-
                        // normalized paths).
                        let bytes: Vec<u8> = (0..n)
                            .map(|i| ((rng() & 0x7F) as u8).wrapping_add(i as u8) & 0x7F)
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
                                .map(|i| ((rng() & 0x7F) as u8).wrapping_add(i as u8) & 0x7F)
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
    );

    // -----------------------------------------------------------------
    // T M10.2 CRDT-backed buffer tests.
    //
    // Targeted tests for Day 2's routing work: rope ≡ CRDT projection
    // invariant must hold after apply_edit / undo / redo / arbitrary
    // sequences. Day 5+'s parameterized harness re-runs all of the
    // existing buffer tests against CRDT mode; Day 2's tests are
    // direct.
    // -----------------------------------------------------------------

    // -----------------------------------------------------------------
    // T M10.2 Day 6 — coverage-audit gap closures.
    //
    // Day 5's 17 dual-mode tests cover canonical edit semantics but
    // miss two non-canonical paths that the Day 6 coverage audit
    // surfaced as substantive M10.2 scope:
    //
    //   * `apply_edit_skip_intercepts` — the Lua-bindings path that
    //     bypasses the intercept chain. Routes through the same
    //     `run_rope_edit_and_broadcast` as `apply_edit`, which is
    //     where CRDT routing happens; in principle CRDT mode applies
    //     equally, but worth an explicit test to lock it in.
    //   * `begin_edit` / `end_edit` — the re-entrancy gate that
    //     surfaces `BufferError::ConcurrentEdit` on nested edits.
    //     Independent of CRDT mode but the gate's interaction with
    //     CRDT routing wasn't tested.
    //
    // Other gaps the audit surfaced but classified as deferred:
    //
    //   * Boundary edits (insert at 0, insert at end, delete entire
    //     buffer, replace covering whole) — implicitly covered by
    //     the random_edit fuzz and explicit tests. Adding dedicated
    //     boundary tests is cheap polish, not M10.2 scope.
    //   * Large edits (1MB+) — exercises the export-overhead cost
    //     model; Day 7's perf check covers these.
    //   * `set_name`, `mark_clean`, `editing_in_progress` getter —
    //     non-edit-path methods; behavior is identical across modes
    //     by construction. Not worth a dual-mode test each.
    // -----------------------------------------------------------------

    dual_mode_test!(
        apply_edit_skip_intercepts_routes_through_crdt,
        |_make, make_bytes| {
            // skip_intercepts bypasses the intercept chain but routes
            // through the same rope-edit-and-broadcast path. In CRDT
            // mode the CRDT routing still applies; the rope projection
            // and CRDT state must stay in sync as they do for `apply_edit`.
            let mut b = make_bytes("test", b"hello");
            // Attach a view whose intercept would reject any edit; the
            // skip_intercepts call must succeed because the intercept is
            // bypassed, demonstrating the path works.
            b.attach_view(Box::new(ReverseInsertView));
            let edit = b
                .apply_edit_skip_intercepts(EditOp::Delete {
                    range: Range::new(0, 1),
                })
                .expect("skip_intercepts bypasses the rejecting intercept");
            assert_eq!(edit.range, Range::new(0, 1));
            assert_eq!(edit.inserted_len, 0);
            assert_eq!(collect(&b), b"ello");
            assert!(b.is_modified());
        }
    );

    dual_mode_test!(
        begin_edit_blocks_reentrant_apply_edit,
        |make, _make_bytes| {
            // begin_edit sets the editing_in_progress flag; a subsequent
            // apply_edit returns ConcurrentEdit until end_edit. The flag
            // is independent of CRDT mode but the gate's interaction
            // with CRDT routing is worth pinning.
            let mut b = make("test");
            b.begin_edit().expect("first begin");
            assert!(b.editing_in_progress());

            let err = b.apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"X",
            });
            assert!(
                matches!(err, Err(BufferError::ConcurrentEdit { .. })),
                "expected ConcurrentEdit, got {err:?}"
            );

            // end_edit clears the flag; subsequent apply_edit succeeds.
            b.end_edit();
            assert!(!b.editing_in_progress());
            b.apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"X",
            })
            .expect("after end_edit");
            assert_eq!(collect(&b), b"X");
        }
    );

    dual_mode_test!(
        begin_edit_is_reentrant_safe_via_returned_error,
        |make, _make_bytes| {
            // Two consecutive begin_edit calls without an intervening
            // end_edit must return ConcurrentEdit on the second, not
            // double-set the flag. Pins the gate's idempotency.
            let mut b = make("*scratch*");
            b.begin_edit().expect("first begin");
            let r = b.begin_edit();
            assert!(matches!(r, Err(BufferError::ConcurrentEdit { .. })));
            // First begin's flag is still set; end_edit clears it.
            assert!(b.editing_in_progress());
            b.end_edit();
            assert!(!b.editing_in_progress());
        }
    );

    #[cfg(feature = "crdt")]
    fn rope_string(b: &Buffer) -> String {
        let mut bytes = vec![0u8; b.len() as usize];
        if !bytes.is_empty() {
            b.snapshot_rope().slice(0, b.len(), &mut bytes);
        }
        String::from_utf8(bytes).expect("rope contents must be UTF-8 in CRDT mode")
    }

    #[cfg(feature = "crdt")]
    fn assert_invariant(b: &Buffer) {
        let rope = rope_string(b);
        let crdt = b
            .crdt_state()
            .expect("CRDT-backed in this test")
            .materialize_string();
        assert_eq!(rope, crdt, "rope ≡ CRDT projection invariant violated");
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_apply_edit_keeps_invariant_basic() {
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*scratch*", 1).expect("crdt construction");
        b.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"hello",
        })
        .unwrap();
        assert_eq!(rope_string(&b), "hello");
        assert_invariant(&b);

        b.apply_edit(EditOp::Insert {
            pos: 5,
            bytes: b" world",
        })
        .unwrap();
        assert_eq!(rope_string(&b), "hello world");
        assert_invariant(&b);

        b.apply_edit(EditOp::Delete {
            range: Range::new(5, 6),
        })
        .unwrap();
        assert_eq!(rope_string(&b), "helloworld");
        assert_invariant(&b);

        b.apply_edit(EditOp::Replace {
            range: Range::new(0, 5),
            bytes: b"howdy",
        })
        .unwrap();
        assert_eq!(rope_string(&b), "howdyworld");
        assert_invariant(&b);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_undo_keeps_invariant() {
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*scratch*", 1).expect("crdt construction");
        b.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"hello",
        })
        .unwrap();
        b.apply_edit(EditOp::Insert {
            pos: 5,
            bytes: b" world",
        })
        .unwrap();
        assert_eq!(rope_string(&b), "hello world");
        assert_invariant(&b);

        b.undo().expect("undo");
        assert_eq!(rope_string(&b), "hello");
        assert_invariant(&b);

        b.undo().expect("undo");
        assert_eq!(rope_string(&b), "");
        assert_invariant(&b);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_undo_redo_keeps_invariant() {
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*scratch*", 1).expect("crdt construction");
        b.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"abcdef",
        })
        .unwrap();
        b.apply_edit(EditOp::Replace {
            range: Range::new(2, 4),
            bytes: b"XY",
        })
        .unwrap();
        assert_eq!(rope_string(&b), "abXYef");
        assert_invariant(&b);

        b.undo().expect("undo replace");
        assert_eq!(rope_string(&b), "abcdef");
        assert_invariant(&b);

        b.redo().expect("redo replace");
        assert_eq!(rope_string(&b), "abXYef");
        assert_invariant(&b);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_from_bytes_seeds_both_rope_and_crdt() {
        let b =
            Buffer::from_bytes_with_crdt(BufferId::next(), "*seeded*", b"the quick brown fox", 7)
                .expect("crdt seeded");
        assert_eq!(rope_string(&b), "the quick brown fox");
        assert_invariant(&b);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_upgrade_to_crdt_clears_v01_history() {
        // T M10.4 reframe: upgrade_to_crdt clears v0.1 undo/redo
        // stacks explicitly. The pre-upgrade entries can't be
        // replayed through UndoManager (no peer_id attribution); the
        // M10.4 audit doc records this as documented behavior with
        // a v0.2+ refinement path. This test pins the cleared-stack
        // behavior so future contributors don't reintroduce silent-
        // persist-but-unreachable semantics.
        let mut b = Buffer::from_bytes(BufferId::next(), "*upgrade*", b"initial");
        b.apply_edit(EditOp::Insert {
            pos: 7,
            bytes: b" content",
        })
        .unwrap();
        let pre_upgrade_rev = b.revision();
        assert!(!b.is_crdt_backed());
        assert_eq!(b.undo.len(), 1);

        b.upgrade_to_crdt(42).expect("upgrade");
        assert!(b.is_crdt_backed());
        // Content preserved.
        assert_eq!(rope_string(&b), "initial content");
        assert_invariant(&b);
        // Revision counter preserved (the rope contents themselves
        // didn't change at upgrade time; revision tracks rope-version
        // not history).
        assert_eq!(b.revision(), pre_upgrade_rev);
        // v0.1 undo/redo stacks cleared per M10.4 reframe.
        assert!(b.undo.is_empty());
        assert!(b.redo.is_empty());
        // CRDT-mode undo has nothing yet (the seeded content from
        // upgrade isn't an undoable edit — see CrdtState::from_bytes).
        let undo_result = b.undo();
        assert!(matches!(undo_result, Err(BufferError::NothingToUndo)));

        // Subsequent edits on the upgraded buffer keep the invariant
        // and ARE undoable via CRDT-mode undo.
        b.apply_edit(EditOp::Delete {
            range: Range::new(0, 8),
        })
        .unwrap();
        assert_eq!(rope_string(&b), "content");
        assert_invariant(&b);
        b.undo().expect("undo the delete");
        assert_eq!(rope_string(&b), "initial content");
        assert_invariant(&b);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_upgrade_rejects_double_attach() {
        let mut b = Buffer::new_with_crdt(BufferId::next(), "*twice*", 1).expect("first attach");
        let r = b.upgrade_to_crdt(2);
        assert!(matches!(r, Err(BufferError::CrdtRejected { .. })));
    }

    // ---------------------------------------------------------------
    // Consistency property test: rope ≡ CRDT projection holds under
    // arbitrary apply_edit + undo + redo sequences.
    //
    // Generators per the framing-pass methodology:
    //   - Insert / Delete / Replace at random aligned positions
    //   - Undo / Redo at ~5–10% probability each
    //   - Byte content from a small UTF-8 alphabet (avoids
    //     ill-formed-bytes drift; the wrapper handles those via
    //     from_utf8_lossy but the proptest is testing routing
    //     correctness, not normalization correctness)
    //   - Sequence length 50; 64 cases (proptest's default)
    // ---------------------------------------------------------------
    #[cfg(feature = "crdt")]
    mod proptests {
        use super::*;
        use proptest::prelude::*;

        // Codepoint-aligned generators. The mid-codepoint case is its
        // own test (`crdt_mid_codepoint_position_is_rejected_cleanly`);
        // the property test exercises the well-formed path so failures
        // surface routing bugs, not codepoint-alignment surprises.
        const ALPHABET: &[&str] = &["a", "b", "c", " ", "\n"];

        #[derive(Clone, Debug)]
        enum GenOp {
            Insert(usize, String),
            Delete(usize, usize),
            Replace(usize, usize, String),
            Undo,
            Redo,
        }

        fn gen_payload() -> impl Strategy<Value = String> {
            prop::collection::vec(prop::sample::select(ALPHABET.to_vec()), 0..6)
                .prop_map(|parts| parts.concat())
        }

        fn gen_op() -> impl Strategy<Value = GenOp> {
            // Weighted: forward edits dominate, undo/redo at ~5% each.
            prop_oneof![
                30 => (any::<u8>(), gen_payload()).prop_map(|(p, s)| GenOp::Insert(p as usize, s)),
                20 => (any::<u8>(), any::<u8>()).prop_map(|(p, l)| GenOp::Delete(p as usize, l as usize)),
                10 => (any::<u8>(), any::<u8>(), gen_payload())
                    .prop_map(|(p, l, s)| GenOp::Replace(p as usize, l as usize, s)),
                3  => Just(GenOp::Undo),
                3  => Just(GenOp::Redo),
            ]
        }

        // T M10.2 Day 3 helper: applies a `GenOp` and returns the
        // resulting Edit so the proptest can assert per-op shape.
        // Each op is best-effort: out-of-range positions are clamped
        // before dispatch so the proptest doesn't fail on benign rope
        // errors. Routing bugs (CRDT/rope drift) are what the post-
        // condition catches. Returns None on no-op short-circuits and
        // on history-stack-empty errors (NothingToUndo / NothingToRedo).
        fn apply_capturing(b: &mut Buffer, op: GenOp) -> Option<Edit> {
            let len = b.len() as usize;
            match op {
                GenOp::Insert(pos, s) => {
                    let pos = pos.min(len);
                    b.apply_edit(EditOp::Insert {
                        pos: pos as u64,
                        bytes: s.as_bytes(),
                    })
                    .ok()
                }
                GenOp::Delete(pos, l) => {
                    let pos = pos.min(len);
                    let l = l.min(len.saturating_sub(pos));
                    b.apply_edit(EditOp::Delete {
                        range: Range::new(pos as u64, (pos + l) as u64),
                    })
                    .ok()
                }
                GenOp::Replace(pos, l, s) => {
                    let pos = pos.min(len);
                    let l = l.min(len.saturating_sub(pos));
                    b.apply_edit(EditOp::Replace {
                        range: Range::new(pos as u64, (pos + l) as u64),
                        bytes: s.as_bytes(),
                    })
                    .ok()
                }
                GenOp::Undo => b.undo().ok(),
                GenOp::Redo => b.redo().ok(),
            }
        }

        proptest! {
            // Smaller proptest case count than the default (64) to keep
            // CI overhead modest; the per-op invariant check is the
            // load-bearing part, not the diversity of sequences.
            #![proptest_config(ProptestConfig::with_cases(32))]

            #[test]
            fn rope_matches_crdt_projection_after_arbitrary_edits(
                ops in prop::collection::vec(gen_op(), 1..50),
            ) {
                let mut b = Buffer::new_with_crdt(BufferId::next(), "*proptest*", 1)
                    .expect("crdt construction");
                for op in ops {
                    let op_repr = format!("{op:?}");
                    let edit = apply_capturing(&mut b, op);
                    // Per-op invariant check: catches drift the moment
                    // it happens, with the failing op visible in the
                    // shrinker output.
                    let rope = rope_string(&b);
                    let crdt = b.crdt_state().unwrap().materialize_string();
                    prop_assert_eq!(
                        &rope, &crdt,
                        "invariant violated after op {}: rope={:?} crdt={:?}",
                        op_repr, rope, crdt
                    );
                    // Day 3: crdt_op shape invariant.
                    // - real edits in CRDT mode populate crdt_op
                    // - no-op short-circuits leave crdt_op = None
                    // - history-stack-empty errors return None Edit
                    if let Some(edit) = edit {
                        let is_no_op_edit_result =
                            edit.range.is_empty() && edit.inserted_len == 0;
                        if is_no_op_edit_result {
                            prop_assert!(
                                edit.crdt_op.is_none(),
                                "no-op edit must have crdt_op = None ({})",
                                op_repr
                            );
                        } else {
                            prop_assert!(
                                edit.crdt_op.is_some(),
                                "non-no-op CRDT-mode edit must have crdt_op = Some ({})",
                                op_repr
                            );
                            let crdt_op = edit.crdt_op.as_ref().unwrap();
                            prop_assert_eq!(
                                crdt_op.peer_id, 1,
                                "peer_id must thread from CrdtState ({})",
                                op_repr
                            );
                            prop_assert!(
                                !crdt_op.bytes.is_empty(),
                                "wire bytes must be non-empty ({})",
                                op_repr
                            );
                        }
                    }
                }
            }

            // T M10.3: round-trip property. Arbitrary EditOp sequences
            // on Buffer A (peer_id 1) produce per-edit crdt_op bytes.
            // Replaying those bytes on a fresh CrdtState B (peer_id 2)
            // must produce a projection identical to A's. This is the
            // stronger property than Day 3's single-instance test:
            // proves that the wire-format bytes are independently
            // re-applicable on a remote CRDT instance, exercising the
            // path M10.5 will use for InstanceMessage::CrdtOp delivery.
            //
            // Excludes undo/redo from the gen — those produce synthetic
            // Replace ops that, when re-applied on B from base zero,
            // create a state inconsistent with A's history-swap
            // semantics. M10.5's actual wire protocol delivers undo
            // ops only when the originating peer's prior history is
            // already known to the receiver; the proptest scope is
            // forward edits only (insert / delete / replace).
            #[test]
            fn crdt_op_bytes_round_trip_via_remote_crdt_state(
                ops in prop::collection::vec(gen_op_forward_only(), 1..30),
            ) {
                let mut a = Buffer::new_with_crdt(BufferId::next(), "A", 1)
                    .expect("A construction");
                let receiver = crate::crdt::CrdtState::new(2)
                    .expect("receiver construction");

                for op in ops {
                    let op_repr = format!("{op:?}");
                    let edit = apply_capturing(&mut a, op);
                    if let Some(edit) = edit
                        && let Some(crdt_op) = edit.crdt_op.as_ref() {
                            // Apply the wire-format bytes to the
                            // receiver. Receiver projection must match
                            // A's projection after this.
                            receiver
                                .import_updates(&crdt_op.bytes)
                                .expect("receiver import");
                            let a_proj = a
                                .crdt_state()
                                .unwrap()
                                .materialize_string();
                            let b_proj = receiver.materialize_string();
                            prop_assert_eq!(
                                &a_proj, &b_proj,
                                "A and remote receiver diverged after op {}: \
                                 A={:?} B={:?}",
                                op_repr, a_proj, b_proj
                            );
                        }
                }
            }
        }

        /// T M10.3 generator: forward-only ops (no Undo/Redo). The
        /// round-trip proptest excludes history-nav because the
        /// synthetic-Replace ops undo/redo produce don't round-trip
        /// cleanly when replayed on a peer without the originating
        /// history. M10.5's wire protocol handles this; M10.3's
        /// scope is forward edits.
        fn gen_op_forward_only() -> impl Strategy<Value = GenOp> {
            prop_oneof![
                30 => (any::<u8>(), gen_payload()).prop_map(|(p, s)| GenOp::Insert(p as usize, s)),
                20 => (any::<u8>(), any::<u8>()).prop_map(|(p, l)| GenOp::Delete(p as usize, l as usize)),
                10 => (any::<u8>(), any::<u8>(), gen_payload())
                    .prop_map(|(p, l, s)| GenOp::Replace(p as usize, l as usize, s)),
            ]
        }
    }

    // ---------------------------------------------------------------
    // T M10.2 Day 3 — crdt_op population on Edit.
    // ---------------------------------------------------------------

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_op_is_none_in_v01_mode() {
        let mut b = Buffer::new(BufferId::next(), "*v01*");
        let edit = b
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"hello",
            })
            .unwrap();
        assert!(
            edit.crdt_op.is_none(),
            "v0.1 mode (no CRDT) must produce Edit with crdt_op = None"
        );
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_op_is_some_in_crdt_mode_for_real_edits() {
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*crdt*", 42).expect("crdt construction");
        let edit = b
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"hello",
            })
            .unwrap();
        let op = edit
            .crdt_op
            .as_ref()
            .expect("CRDT mode must populate crdt_op");
        assert_eq!(op.peer_id, 42, "peer_id must thread through");
        assert!(!op.bytes.is_empty(), "wire bytes must be non-empty");
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_op_is_none_for_no_op_edits_even_in_crdt_mode() {
        // Q5: truly empty edits skip the CRDT path entirely. The
        // returned Edit's crdt_op must be None — the rope's no-op
        // short-circuit doesn't reach the CRDT routing, so no op
        // bytes are captured.
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*crdt*", 1).expect("crdt construction");
        let edit = b.apply_edit(EditOp::Insert { pos: 0, bytes: b"" }).unwrap();
        assert!(edit.crdt_op.is_none(), "true no-op insert must yield None");
        let edit = b
            .apply_edit(EditOp::Delete {
                range: Range::new(0, 0),
            })
            .unwrap();
        assert!(edit.crdt_op.is_none(), "true no-op delete must yield None");
        let edit = b
            .apply_edit(EditOp::Replace {
                range: Range::new(0, 0),
                bytes: b"",
            })
            .unwrap();
        assert!(edit.crdt_op.is_none(), "true no-op replace must yield None");
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_op_carries_distinct_bytes_per_edit() {
        // The framing pass's verification target: apply two edits in
        // succession, verify each Edit's crdt_op contains exactly its
        // own delta (not the cumulative). Loro's transactional model
        // gives a consistent before/after pair via version capture.
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*twin*", 7).expect("crdt construction");
        let e1 = b
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"first",
            })
            .unwrap();
        let e2 = b
            .apply_edit(EditOp::Insert {
                pos: 5,
                bytes: b"-second",
            })
            .unwrap();
        let bytes1 = &e1.crdt_op.as_ref().unwrap().bytes;
        let bytes2 = &e2.crdt_op.as_ref().unwrap().bytes;
        assert_ne!(
            bytes1, bytes2,
            "each edit's crdt_op must carry its own delta, not cumulative state"
        );
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_op_populated_for_undo_and_redo() {
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*history*", 3).expect("crdt construction");
        b.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"hello",
        })
        .unwrap();
        let undo_edit = b.undo().expect("undo");
        assert!(
            undo_edit.crdt_op.is_some(),
            "undo's inverse Edit must carry the synthetic-Replace bytes"
        );
        assert_eq!(undo_edit.crdt_op.as_ref().unwrap().peer_id, 3);
        let redo_edit = b.redo().expect("redo");
        assert!(
            redo_edit.crdt_op.is_some(),
            "redo's inverse Edit must carry the synthetic-Replace bytes"
        );
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_op_imports_into_a_fresh_doc_to_reproduce_state() {
        // Day 3 acceptance verification (framing-pass risk #2 follow-
        // up): the wire-format bytes a single edit produces, when
        // imported into a fresh CRDT doc, should reproduce the
        // post-edit content. This is what M10.5's wire protocol will
        // rely on — receiving frontends import the bytes to apply
        // ops on their local CRDT.
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*wire*", 1).expect("crdt construction");
        // First edit: empty -> "hello".
        let e1 = b
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"hello",
            })
            .unwrap();
        // Second edit: -> "hello world".
        let e2 = b
            .apply_edit(EditOp::Insert {
                pos: 5,
                bytes: b" world",
            })
            .unwrap();
        // Replay both deltas into a fresh CRDT doc; result should
        // match the originating buffer's rope.
        let receiver = crate::crdt::CrdtState::new(99).unwrap();
        receiver
            .import_snapshot(&e1.crdt_op.as_ref().unwrap().bytes)
            .expect("import e1");
        receiver
            .import_snapshot(&e2.crdt_op.as_ref().unwrap().bytes)
            .expect("import e2");
        assert_eq!(
            receiver.materialize_string(),
            "hello world",
            "replaying the wire bytes on a fresh doc must reproduce the originating state"
        );
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn crdt_mid_codepoint_position_is_rejected_cleanly() {
        // Per the Q1 morning audit, loro rejects mid-codepoint
        // delete_utf8 / insert_utf8. The Buffer routing must surface
        // the rejection as a clean error and leave both rope and CRDT
        // untouched (the rope mutation never runs because Q2 ordering
        // applies CRDT first).
        let mut b =
            Buffer::new_with_crdt(BufferId::next(), "*midcp*", 1).expect("crdt construction");
        b.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: "héllo".as_bytes(),
        })
        .unwrap();
        assert_invariant(&b);

        // Try to delete starting mid-é (byte 2 of "héllo"). Should
        // surface as CrdtRejected; rope and CRDT both unchanged.
        let pre_rope = rope_string(&b);
        let r = b.apply_edit(EditOp::Delete {
            range: Range::new(2, 3),
        });
        assert!(
            matches!(r, Err(BufferError::CrdtRejected { .. })),
            "got {r:?}"
        );
        assert_eq!(rope_string(&b), pre_rope, "rope must be unchanged");
        assert_invariant(&b);
    }

    // -----------------------------------------------------------------
    // T M10.10 Finding 3 — apply_remote_crdt_op acceptance.
    // -----------------------------------------------------------------

    #[cfg(feature = "crdt")]
    #[test]
    fn apply_remote_crdt_op_integrates_op_and_keeps_invariant() {
        // Donor peer (frontend B simulated) produces an op against
        // an empty starting state. Receiver (daemon-side buffer)
        // applies the op via apply_remote_crdt_op.
        let donor = crate::crdt::CrdtState::new(2).expect("donor");
        let v_before = donor.version();
        donor.insert(0, "hello").expect("donor seed");
        let op_bytes = donor
            .export_updates_since(&v_before)
            .expect("export updates");

        // Receiver buffer starts empty under peer 1 (the daemon's
        // LOCAL peer id).
        let mut buf = Buffer::new_with_crdt(BufferId::next(), "*remote*", 1).expect("receiver buf");
        assert_eq!(rope_string(&buf), "");

        let edit = buf
            .apply_remote_crdt_op(&op_bytes)
            .expect("apply remote")
            .expect("non-empty edit");

        // Rope ≡ CRDT projection invariant after remote op.
        assert_eq!(rope_string(&buf), "hello");
        assert_invariant(&buf);
        // Edit's crdt_op stays None — remote op doesn't get re-broadcast.
        assert!(
            edit.crdt_op.is_none(),
            "remote-applied Edit must not carry crdt_op"
        );
        // Modified + revision bumped.
        assert!(buf.is_modified());
        assert!(buf.revision() > 0);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn apply_remote_crdt_op_on_non_crdt_buffer_errors() {
        let mut buf = Buffer::new(BufferId::next(), "*plain*");
        let result = buf.apply_remote_crdt_op(&[0x00, 0x01, 0x02]);
        assert!(
            matches!(result, Err(BufferError::CrdtRejected { .. })),
            "got {result:?}"
        );
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn apply_remote_crdt_op_invokes_on_edit_subscribers() {
        // Verify the Edit subscriber path is honored — TextView's
        // line cache + syntax highlighting + overlays all depend on
        // on_edit notifications. A simple recording View confirms
        // the broadcast fires.
        use crate::view::View;
        use std::cell::Cell;
        use std::rc::Rc;

        struct RecorderView {
            count: Rc<Cell<usize>>,
        }
        impl View for RecorderView {
            fn on_edit(&mut self, _buf: &Buffer, _edit: &Edit) -> Result<(), BufferError> {
                self.count.set(self.count.get() + 1);
                Ok(())
            }
        }

        let donor = crate::crdt::CrdtState::new(2).expect("donor");
        let v_before = donor.version();
        donor.insert(0, "abc").expect("donor seed");
        let op_bytes = donor.export_updates_since(&v_before).expect("export");

        let mut buf = Buffer::new_with_crdt(BufferId::next(), "*recorder*", 1).expect("buf");
        let count = Rc::new(Cell::new(0usize));
        buf.attach_view(Box::new(RecorderView {
            count: Rc::clone(&count),
        }));

        let _edit = buf.apply_remote_crdt_op(&op_bytes).expect("apply").unwrap();

        assert_eq!(count.get(), 1, "on_edit must fire for remote op");
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn apply_remote_crdt_op_insert_after_multibyte_char_uses_utf8_byte_position() {
        let donor = crate::crdt::CrdtState::new(2).expect("donor");
        donor.insert(0, "éx").expect("seed");

        let mut buf = Buffer::new_with_crdt(BufferId::next(), "*utf8-insert*", 1).expect("buf");
        let donor_snap = donor.export_snapshot().expect("snap");
        buf.crdt
            .as_ref()
            .expect("crdt")
            .import_snapshot(&donor_snap)
            .expect("init from snap");
        buf.rope = crate::rope::Rope::from_bytes("éx".as_bytes());

        let v_before = donor.version();
        donor.insert("é".len(), "!").expect("insert");
        let op_bytes = donor.export_updates_since(&v_before).expect("export");
        let edit = buf
            .apply_remote_crdt_op(&op_bytes)
            .expect("apply")
            .expect("non-empty edit");

        assert_eq!(rope_string(&buf), "é!x");
        assert_eq!(edit.range, Range::new("é".len() as u64, "é".len() as u64));
        assert_eq!(edit.inserted_len, 1);
        assert_invariant(&buf);
    }

    /// F25 (post-audit-round-4): a CRDT update that changes one
    /// codepoint into another with a shared leading UTF-8 byte
    /// must produce a char-boundary-aligned diff. Pre-fix, the
    /// byte-prefix walk landed mid-codepoint and the rope edit
    /// carried an invalid byte slice.
    ///
    /// Setup: receiver starts empty; donor inserts 'é'; receiver
    /// applies that first op (state → 'é'). Donor then deletes the
    /// 'é' and inserts 'è'; receiver applies that second op. The
    /// second op's diff path sees pre = 'é' (`0xC3 0xA9`) vs post
    /// = 'è' (`0xC3 0xA8`) — byte prefix 1, lands mid-codepoint.
    #[cfg(feature = "crdt")]
    #[test]
    fn apply_remote_crdt_op_preserves_utf8_boundaries_for_single_char_change_f25() {
        let donor = crate::crdt::CrdtState::new(2).expect("donor");
        let v0 = donor.version();
        donor.insert(0, "é").expect("donor seed");
        let seed_bytes = donor.export_updates_since(&v0).expect("seed export");
        let v_after_seed = donor.version();
        donor.delete(0, "é".len()).expect("donor delete");
        donor.insert(0, "è").expect("donor insert");
        let replace_bytes = donor
            .export_updates_since(&v_after_seed)
            .expect("replace export");

        let mut buf = Buffer::new_with_crdt(BufferId::next(), "*utf8*", 1).expect("buf");
        // Step 1: seed receiver with 'é'.
        let seed_edit = buf
            .apply_remote_crdt_op(&seed_bytes)
            .expect("apply seed")
            .expect("seed edit");
        assert_eq!(rope_string(&buf), "é");
        assert_eq!(seed_edit.range.start, 0);
        assert_eq!(seed_edit.range.end, 0);

        // Step 2: the F25 codepath. Apply the replace op WITHOUT
        // panicking on a mid-codepoint rope split.
        let replace_edit = buf
            .apply_remote_crdt_op(&replace_bytes)
            .expect("apply replace")
            .expect("replace edit");
        assert_eq!(rope_string(&buf), "è");
        // The Edit's range should cover the WHOLE codepoint (0..2),
        // not the byte-naive (1..2) that pre-fix would have produced.
        assert_eq!(
            replace_edit.range.start, 0,
            "F25: range_start must be char-boundary-aligned (0, not 1)"
        );
        assert_eq!(replace_edit.range.end, "é".len() as u64);
        assert_invariant(&buf);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn apply_remote_crdt_op_for_delete_produces_correct_diff() {
        // Donor inserts then deletes; the second op is the "delete"
        // we'll apply remotely. Replicates the multi-step convergence
        // path.
        let donor = crate::crdt::CrdtState::new(2).expect("donor");
        donor.insert(0, "hello world").expect("seed");

        let mut buf = Buffer::new_with_crdt(BufferId::next(), "*remote-del*", 1).expect("buf");
        // Sync receiver up to donor's initial state.
        let donor_snap = donor.export_snapshot().expect("snap");
        buf.crdt
            .as_ref()
            .expect("crdt")
            .import_snapshot(&donor_snap)
            .expect("init from snap");
        // Replace rope to match (test setup; production daemon side
        // doesn't do this manually).
        buf.rope = crate::rope::Rope::from_bytes(b"hello world");
        assert_eq!(rope_string(&buf), "hello world");

        // Donor deletes " world" (6 bytes from position 5).
        let v_before = donor.version();
        donor.delete(5, 6).expect("donor delete");
        let op_bytes = donor.export_updates_since(&v_before).expect("export");

        let _ = buf.apply_remote_crdt_op(&op_bytes).expect("apply").unwrap();
        assert_eq!(rope_string(&buf), "hello");
        assert_invariant(&buf);
    }
}
