//! T M10.10 — Frontend-side CRDT replica state.
//!
//! `BufferMirror` is the frontend's mirror of the instance's
//! authoritative buffer state. M10.10 makes the frontend a CRDT
//! replica (departing from the M10.5–M10.9 "transport sink" posture)
//! so that local edits can be applied optimistically before the
//! daemon's confirmation round-trips.
//!
//! # Lifecycle
//!
//! - **Bootstrap.** On `SessionEstablished` the daemon sends one
//!   `InstanceMessage::BufferSnapshot` per active buffer. The
//!   frontend calls [`BufferMirror::init_from_snapshot`] for each;
//!   each buffer's mirror starts at the same CRDT state as the
//!   instance.
//! - **Local edits.** A typed character (per the text-input
//!   predicate in `attach.rs`) becomes a `CrdtState::insert` /
//!   `CrdtState::delete` call on the relevant mirror. The mirror
//!   produces a `CrdtOp` payload that the frontend sends upstream as
//!   `FrontendEvent::CrdtOp`. The mirror has already applied the op
//!   locally — the upstream send is for the daemon and other
//!   frontends.
//! - **Remote ops.** Incoming `InstanceMessage::CrdtOp` is routed
//!   here via [`BufferMirror::apply_remote_op`]. The op-source
//!   `FrontendId` determines whether it's the receiving frontend's
//!   own echo (no-op-on-echo, Q4) or a true remote op that needs
//!   integration plus repaint (Q5).
//! - **Mid-session buffer creation.** A new buffer surfaces via a
//!   subsequent `BufferSnapshot`. [`BufferMirror::is_ready`] gates
//!   optimistic-apply per buffer; pre-bootstrap typing falls through
//!   to the v0.1 round-trip path (Refinement 4).
//!
//! # Threading
//!
//! Single-thread owner (frontend's main loop). `CrdtState` is
//! `Send`-but-not-`Sync` per `crdt.rs`'s docstring; the mirror lives
//! on the main thread, the reader thread only delivers `InstanceMessage`
//! via mpsc. No `Mutex` wrapping.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::buffer::BufferId;
use crate::crdt::{CrdtState, peer_id_from_frontend};
use crate::protocol::FrontendId;
use loro::LoroError;
use unicode_width::UnicodeWidthChar;

/// Errors returned by [`BufferMirror`] operations.
///
/// Two invariant-violation variants surface caller mistakes honestly:
/// `NotReady` means a method was called on a buffer that hasn't had a
/// snapshot applied; `AlreadyInitialized` means
/// [`init_from_snapshot`](BufferMirror::init_from_snapshot) was called
/// twice for the same buffer. The third variant wraps `loro::LoroError`
/// for genuine CRDT-layer failures.
#[derive(Debug)]
pub enum BufferMirrorError {
    /// `apply_local_insert`/`apply_local_delete`/`apply_remote_op` was
    /// called for a buffer that hasn't received a snapshot yet. The
    /// caller should consult [`is_ready`](BufferMirror::is_ready) and
    /// fall through to the v0.1 round-trip path
    /// (Refinement 4: pre-bootstrap graceful degradation).
    NotReady(BufferId),
    /// [`init_from_snapshot`](BufferMirror::init_from_snapshot) was
    /// called for a buffer that already has state. Silently replacing
    /// would discard any optimistically-applied local edits — data
    /// loss. The caller (the dispatcher event loop) should treat this
    /// as a daemon-side bug and surface the error rather than retry.
    AlreadyInitialized(BufferId),
    /// Underlying CRDT operation failed. Includes invalid byte
    /// offsets, mid-codepoint positions, and op-bytes that don't
    /// decode.
    Loro(LoroError),
}

impl From<LoroError> for BufferMirrorError {
    fn from(e: LoroError) -> Self {
        Self::Loro(e)
    }
}

impl fmt::Display for BufferMirrorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReady(id) => {
                write!(f, "buffer {id:?} has no CRDT snapshot applied yet")
            }
            Self::AlreadyInitialized(id) => {
                write!(f, "buffer {id:?} already has a CRDT snapshot applied")
            }
            Self::Loro(e) => write!(f, "CRDT operation failed: {e}"),
        }
    }
}

impl std::error::Error for BufferMirrorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Loro(e) => Some(e),
            Self::NotReady(_) | Self::AlreadyInitialized(_) => None,
        }
    }
}

/// Per-frontend collection of CRDT-replica states keyed by `BufferId`.
///
/// A `BufferMirror` is created once per attach session, owned by the
/// frontend's main loop. It holds one [`CrdtState`] per active buffer
/// the daemon has snapshotted to this frontend, plus the byte-position
/// cursor for each buffer (authoritatively updated from
/// `InstanceMessage::CursorByte`).
pub struct BufferMirror {
    /// The frontend's assigned id — used to derive the loro `peer_id`
    /// for newly-initialized `CrdtState`s. Stable for the session.
    peer_id: u64,
    /// One CRDT replica per buffer the frontend currently mirrors.
    states: HashMap<BufferId, CrdtState>,
    /// Per-buffer cursor byte position. Authoritative source for the
    /// optimistic-apply path's insert/delete position arguments.
    ///
    /// Updated by [`set_cursor_byte_pos`](BufferMirror::set_cursor_byte_pos)
    /// when an `InstanceMessage::CursorByte` arrives from the daemon.
    /// Absent until the first such message arrives for a given
    /// `BufferId` — callers (the optimistic-apply path) treat absence
    /// the same as `is_ready` false and fall through to v0.1
    /// round-trip per Refinement 4.
    cursors: HashMap<BufferId, usize>,
    /// The buffer the most recent `InstanceMessage::CursorByte`
    /// described — the daemon's authoritative "active buffer for this
    /// frontend" signal. The optimistic-apply path routes keystrokes
    /// to this buffer.
    ///
    /// `None` until the first `CursorByte` arrives (pre-bootstrap
    /// state). The optimistic predicate falls through to v0.1
    /// round-trip when this is `None` per Refinement 4 graceful
    /// degradation.
    active_buffer: Option<BufferId>,
    /// T M10.10 post-audit-round-4 F22 + F23 — buffers whose cursor
    /// position is **non-authoritative** until the next `CursorByte`
    /// arrives from the daemon. Set when:
    ///
    /// - [`apply_remote_op`](Self::apply_remote_op) integrates a
    ///   remote CRDT op into the buffer's content. The mirror's
    ///   cursor for the buffer doesn't auto-adjust with right-
    ///   gravity, so it may now point at the wrong byte relative to
    ///   the new content (F23).
    /// - [`mark_cursor_stale`](Self::mark_cursor_stale) is called by
    ///   the attach loop after sending a `FrontendEvent::Key` that
    ///   the daemon will process — Key events can move the cursor
    ///   (motion keys, Enter, Tab, mid-line edits, etc.) and the
    ///   mirror has no way to predict the new position locally
    ///   (F22).
    ///
    /// Cleared when
    /// [`set_cursor_byte_pos`](Self::set_cursor_byte_pos) is called
    /// (the daemon's authoritative `CursorByte` arrived).
    ///
    /// The optimistic-apply orchestrator gates on
    /// [`is_cursor_fresh`](Self::is_cursor_fresh) and round-trips
    /// when stale, preventing "the mirror cursor advanced via
    /// optimistic apply against content the daemon's cursor was
    /// already past" coherence bugs.
    stale_cursors: HashSet<BufferId>,
}

impl BufferMirror {
    /// Construct a mirror tied to `frontend_id`. The mirror starts
    /// with no buffers; [`init_from_snapshot`](Self::init_from_snapshot)
    /// adds them as `BufferSnapshot` messages arrive.
    #[must_use]
    pub fn new(frontend_id: FrontendId) -> Self {
        Self {
            peer_id: peer_id_from_frontend(frontend_id),
            states: HashMap::new(),
            cursors: HashMap::new(),
            active_buffer: None,
            stale_cursors: HashSet::new(),
        }
    }

    /// Mark the cursor for `buffer_id` as non-authoritative until
    /// the next `CursorByte` arrives. The optimistic-apply
    /// orchestrator round-trips while stale.
    ///
    /// Called by the attach loop after sending a `FrontendEvent::Key`
    /// to the daemon — the daemon's command pipeline may move the
    /// cursor (motion, Enter/Tab, mid-line edits) in ways the mirror
    /// can't predict locally. See post-audit-round-4 F22.
    pub fn mark_cursor_stale(&mut self, buffer_id: BufferId) {
        self.stale_cursors.insert(buffer_id);
    }

    /// Returns true when the mirror's cursor for `buffer_id`
    /// reflects the daemon's authoritative position (i.e. no
    /// `apply_remote_op` or `mark_cursor_stale` has run since the
    /// last `CursorByte`).
    ///
    /// The orchestrator consults this before generating a local
    /// `CrdtOp`. Stale → round-trip.
    #[must_use]
    pub fn is_cursor_fresh(&self, buffer_id: BufferId) -> bool {
        !self.stale_cursors.contains(&buffer_id)
    }

    /// The buffer the daemon most recently signaled as active for
    /// this frontend (via `InstanceMessage::CursorByte`). The
    /// optimistic-apply path routes keystrokes to this buffer.
    ///
    /// Returns `None` until the first `CursorByte` has been applied.
    /// Callers should treat `None` the same as `is_ready` false and
    /// fall through to v0.1 round-trip.
    #[must_use]
    pub fn active_buffer(&self) -> Option<BufferId> {
        self.active_buffer
    }

    /// This frontend's loro `peer_id`. Stable for the session; used
    /// by the keystroke-handling path to construct `CrdtOp` wire
    /// payloads.
    #[must_use]
    pub fn peer_id(&self) -> u64 {
        self.peer_id
    }

    /// Byte length of the character ending at the current cursor in
    /// `buffer_id`. Used for delete-back: removing one character
    /// from the optimistic mirror means removing this many bytes
    /// ending at the cursor.
    ///
    /// Returns:
    /// - `Some(n)` with the UTF-8 byte length of the previous
    ///   character (1 for ASCII, 2–4 for non-ASCII Unicode).
    /// - `None` if the buffer isn't ready, the cursor isn't tracked,
    ///   or the cursor is at position 0 (nothing before it to delete).
    ///
    /// The char-aware byte count matters: loro rejects mid-codepoint
    /// deletes (`crdt.rs::q1_sub_check_3_mid_codepoint_rejection_on_delete`).
    /// A naive 1-byte-retreat would error on multi-byte characters
    /// — using this helper produces a valid CRDT op.
    #[must_use]
    pub fn prev_char_len(&self, buffer_id: BufferId) -> Option<usize> {
        let cursor = self.cursors.get(&buffer_id).copied()?;
        if cursor == 0 {
            return None;
        }
        let content = self.states.get(&buffer_id)?.materialize_string();
        content[..cursor].chars().last().map(char::len_utf8)
    }

    /// T M10.10 (post-audit Finding 5) — is the cursor at end-of-
    /// line **and safe for visual delete-back paint**?
    ///
    /// Stricter than [`cursor_at_end_of_line`](Self::cursor_at_end_of_line):
    /// requires three additional invariants on top of end-of-line:
    /// (a) cursor not at position 0 (something to delete);
    /// (b) previous char is not `\n` (a line-join requires `MoveUp` +
    /// repaint of the previous line's tail, which the single-column
    /// erase can't represent — round-1 Finding 9);
    /// (c) previous char's rendered display width is exactly 1
    /// column (post-audit-round-3 F20 — wide chars, tabs, combining
    /// marks, and other zero-width controls all break the single-
    /// column-erase paint sequence's column accounting).
    ///
    /// # F20: width-1 invariant
    ///
    /// The paint sequence `MoveLeft(1), Print(' '), MoveLeft(1)`
    /// assumes the cell to erase is exactly one terminal column
    /// wide. `TextView` renders wide chars (`UnicodeWidthChar::width
    /// == 2`) into two cells (base + Continuation), tabs into a
    /// variable run of spaces aligned to the next tab stop, and
    /// zero-width combining marks as clusters attached to the
    /// previous cell. Erasing one of these with our one-column
    /// paint leaves a half-painted cell or a column off.
    ///
    /// The check uses `UnicodeWidthChar::width(prev_char) ==
    /// Some(1)`, which excludes:
    /// - wide chars (`Some(2)`)
    /// - control characters (`None` or `Some(0)`)
    /// - zero-width combining marks (`Some(0)`)
    /// - tabs (`None`)
    ///
    /// Returns:
    /// - `Some(true)` if optimistic delete-back paint is safe (all
    ///   four invariants hold).
    /// - `Some(false)` if optimistic delete-back paint is unsafe
    ///   (any invariant fails).
    /// - `None` if the buffer isn't ready.
    #[must_use]
    pub fn cursor_at_end_of_line_safe_for_delete_back(&self, buffer_id: BufferId) -> Option<bool> {
        let cursor = self.cursors.get(&buffer_id).copied()?;
        let content = self.states.get(&buffer_id)?.materialize_string();
        let bytes = content.as_bytes();
        if cursor == 0 {
            return Some(false); // nothing before cursor to delete
        }
        let at_end_of_line = if cursor >= bytes.len() {
            true
        } else {
            bytes[cursor] == b'\n'
        };
        if !at_end_of_line {
            return Some(false);
        }
        // Previous char must not be a newline — otherwise this is a
        // line-join operation that the single-column-erase paint
        // sequence can't represent.
        let prev_char = content[..cursor].chars().last()?;
        if prev_char == '\n' {
            return Some(false);
        }
        // F20: previous char's rendered width must be exactly 1
        // column. Wide chars, tabs, combining marks all fail this
        // check and fall through to v0.1 round-trip.
        Some(UnicodeWidthChar::width(prev_char) == Some(1))
    }

    /// T M10.10 Day 3 step 5 Path β — is the cursor at the end of
    /// its current line?
    ///
    /// Used by the visual-optimistic-paint gate: end-of-line typing
    /// can paint optimistically (`queue!(out, Print(c))` matches the
    /// cell the daemon's `CellDelta` will eventually carry, so no
    /// flicker). Mid-line typing would shift cells right of cursor
    /// in the daemon's render, which the frontend's single-Print
    /// can't match without view layout — falls through to no
    /// optimistic visual paint.
    ///
    /// Returns:
    /// - `Some(true)` if the cursor is positioned at the end of its
    ///   line (either at the buffer's end, or immediately before a
    ///   `\n`).
    /// - `Some(false)` if the cursor is in the middle of its line.
    /// - `None` if the buffer isn't ready or the cursor isn't
    ///   tracked.
    ///
    /// Detection: the byte at `cursor_byte_pos` is either past the
    /// buffer's end OR is a `\n`. Both cases mean "nothing on this
    /// line right of the cursor" — optimistic Print fits.
    #[must_use]
    pub fn cursor_at_end_of_line(&self, buffer_id: BufferId) -> Option<bool> {
        let cursor = self.cursors.get(&buffer_id).copied()?;
        let content = self.states.get(&buffer_id)?.materialize_string();
        let bytes = content.as_bytes();
        if cursor >= bytes.len() {
            // Cursor at or past the buffer's end — always end-of-line.
            return Some(true);
        }
        // `\n` at cursor position means cursor is at the end of the
        // line that precedes the newline.
        Some(bytes[cursor] == b'\n')
    }

    /// Byte length of the character at the current cursor in
    /// `buffer_id`. Used for delete-forward: removing one character
    /// at the cursor means removing this many bytes.
    ///
    /// Returns:
    /// - `Some(n)` with the UTF-8 byte length of the next character.
    /// - `None` if the buffer isn't ready, the cursor isn't tracked,
    ///   or the cursor is at the end of the buffer.
    ///
    /// Same char-boundary rationale as
    /// [`prev_char_len`](Self::prev_char_len).
    #[must_use]
    pub fn next_char_len(&self, buffer_id: BufferId) -> Option<usize> {
        let cursor = self.cursors.get(&buffer_id).copied()?;
        let content = self.states.get(&buffer_id)?.materialize_string();
        if cursor >= content.len() {
            return None;
        }
        content[cursor..].chars().next().map(char::len_utf8)
    }

    /// Get the cursor byte position for `buffer_id`. Returns `None`
    /// until the first `InstanceMessage::CursorByte` for that buffer
    /// has been applied via
    /// [`set_cursor_byte_pos`](Self::set_cursor_byte_pos).
    ///
    /// The optimistic-apply path consults this before generating a
    /// local `CrdtOp`. Absence means cursor position is unknown for
    /// this buffer; the keystroke should fall through to the v0.1
    /// `FrontendEvent::Key` round-trip path (Refinement 4 graceful
    /// degradation).
    #[must_use]
    pub fn cursor_byte_pos(&self, buffer_id: BufferId) -> Option<usize> {
        self.cursors.get(&buffer_id).copied()
    }

    /// Set the cursor byte position for `buffer_id` authoritatively.
    /// Called when an `InstanceMessage::CursorByte { buffer_id,
    /// byte_pos }` arrives from the daemon.
    ///
    /// Per the broadened `CursorByte` semantics (Day 3 step 3b
    /// composition-check resolution), this method also marks
    /// `buffer_id` as the active buffer for the optimistic-apply
    /// path. `CursorByte` represents "active buffer + cursor in it,"
    /// not just "cursor moved," so the active-buffer update is the
    /// natural pairing.
    ///
    /// Overwrites any prior cursor for `buffer_id` (including
    /// optimistically-advanced positions). This is the
    /// authoritative-update path; optimistic advances via
    /// [`advance_cursor`](Self::advance_cursor) and
    /// [`retreat_cursor`](Self::retreat_cursor) yield to the next
    /// `CursorByte` from the daemon.
    pub fn set_cursor_byte_pos(&mut self, buffer_id: BufferId, byte_pos: usize) {
        self.cursors.insert(buffer_id, byte_pos);
        self.active_buffer = Some(buffer_id);
        // F22 + F23: the daemon's authoritative cursor byte position
        // re-grounds the mirror cursor. Any prior staleness from a
        // pending Key round-trip or unaccounted remote-op
        // right-gravity is resolved.
        self.stale_cursors.remove(&buffer_id);
    }

    /// Advance the cursor for `buffer_id` by `n` bytes (used after a
    /// local optimistic insert). No-op if the cursor isn't tracked
    /// yet (the optimistic-apply path's contract is that
    /// [`cursor_byte_pos`](Self::cursor_byte_pos) returned `Some`
    /// before the apply, so the cursor is guaranteed to exist by the
    /// time we're advancing — but we don't panic on the contract
    /// violation).
    pub fn advance_cursor(&mut self, buffer_id: BufferId, n: usize) {
        if let Some(pos) = self.cursors.get_mut(&buffer_id) {
            *pos = pos.saturating_add(n);
        }
    }

    /// Retreat the cursor for `buffer_id` by `n` bytes (used after a
    /// local optimistic delete-back). Saturating: cursor clamps at 0
    /// rather than wrapping.
    pub fn retreat_cursor(&mut self, buffer_id: BufferId, n: usize) {
        if let Some(pos) = self.cursors.get_mut(&buffer_id) {
            *pos = pos.saturating_sub(n);
        }
    }

    /// True if the mirror has a CRDT state for `buffer_id`. The
    /// frontend's optimistic-apply predicate consults this before
    /// generating a local `CrdtOp`; false → fall through to v0.1
    /// round-trip (Refinement 4 graceful degradation).
    ///
    /// Returns false for `BufferId`s the mirror has never received a
    /// snapshot for — including buffers that exist on the instance
    /// but whose `BufferSnapshot` hasn't arrived at this frontend yet
    /// (the "buffer just got created elsewhere" case from
    /// Refinement 6).
    #[must_use]
    pub fn is_ready(&self, buffer_id: BufferId) -> bool {
        self.states.contains_key(&buffer_id)
    }

    /// Initialize the mirror for `buffer_id` from a daemon-sent CRDT
    /// snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`BufferMirrorError::AlreadyInitialized`] if the buffer
    /// already has state. Silently replacing would discard any
    /// optimistically-applied local edits made between the two
    /// snapshots — that's data loss; this method refuses the
    /// double-init explicitly. A daemon bug (sending two snapshots
    /// for the same buffer) is the most likely cause; the caller
    /// should log and surface the error rather than retry blindly.
    ///
    /// Returns [`BufferMirrorError::Loro`] if `CrdtState::new` or
    /// `import_snapshot` fail. A failed init leaves the mirror
    /// without an entry for `buffer_id`.
    pub fn init_from_snapshot(
        &mut self,
        buffer_id: BufferId,
        snapshot: &[u8],
    ) -> Result<(), BufferMirrorError> {
        if self.states.contains_key(&buffer_id) {
            return Err(BufferMirrorError::AlreadyInitialized(buffer_id));
        }
        let state = CrdtState::new(self.peer_id)?;
        state.import_snapshot(snapshot)?;
        self.states.insert(buffer_id, state);
        Ok(())
    }

    /// Apply a local insertion to the mirror for `buffer_id`.
    /// Returns the op's wire-format bytes so the frontend can wrap
    /// them in `FrontendEvent::CrdtOp` and send upstream.
    ///
    /// # Errors
    ///
    /// - [`BufferMirrorError::NotReady`] if the buffer hasn't received
    ///   a snapshot. Callers should check
    ///   [`is_ready`](Self::is_ready) first and fall through to the
    ///   v0.1 round-trip path on false.
    /// - [`BufferMirrorError::Loro`] if the loro `insert` fails
    ///   (e.g., mid-codepoint position).
    pub fn apply_local_insert(
        &mut self,
        buffer_id: BufferId,
        pos: usize,
        text: &str,
    ) -> Result<Vec<u8>, BufferMirrorError> {
        let state = self
            .states
            .get_mut(&buffer_id)
            .ok_or(BufferMirrorError::NotReady(buffer_id))?;
        let version_before = state.version();
        state.insert(pos, text)?;
        state.export_updates_since(&version_before).map_err(|e| {
            BufferMirrorError::Loro(LoroError::DecodeError(
                format!("export_updates: {e:?}").into(),
            ))
        })
    }

    /// Apply a local deletion to the mirror for `buffer_id`.
    /// Returns the op's wire-format bytes; same contract as
    /// [`apply_local_insert`](Self::apply_local_insert).
    ///
    /// # Errors
    ///
    /// As for [`apply_local_insert`](Self::apply_local_insert).
    pub fn apply_local_delete(
        &mut self,
        buffer_id: BufferId,
        pos: usize,
        len: usize,
    ) -> Result<Vec<u8>, BufferMirrorError> {
        let state = self
            .states
            .get_mut(&buffer_id)
            .ok_or(BufferMirrorError::NotReady(buffer_id))?;
        let version_before = state.version();
        state.delete(pos, len)?;
        state.export_updates_since(&version_before).map_err(|e| {
            BufferMirrorError::Loro(LoroError::DecodeError(
                format!("export_updates: {e:?}").into(),
            ))
        })
    }

    /// Optimistically undo this frontend's last edit on `buffer_id`,
    /// returning the inverse op's wire-format bytes (to be broadcast
    /// as a `FrontendEvent::CrdtOp`) on success.
    ///
    /// Loro's `UndoManager` is bound to the doc's `peer_id` at
    /// construction (see `src/crdt.rs:60-65`, `"Local-only"`: undoes
    /// the bound peer's most recent change). Each frontend's
    /// `BufferMirror` holds a per-buffer `CrdtState` whose
    /// `UndoManager` is bound to this frontend's `peer_id`, so calling
    /// `state.undo()` reverses *this frontend's* most recent edit
    /// regardless of concurrent remote activity — exactly M10.4's
    /// per-frontend undo property. The inverse op is exported and
    /// returned for broadcast; the daemon imports it as an ordinary
    /// CRDT update (no daemon-side `UndoManager` involvement).
    ///
    /// This is the CRDT-native per-frontend undo path (M10.11 P1).
    /// The daemon-side `Buffer::undo` remains the daemon-peer-only
    /// undo path (vestigial from single-frontend mode + still used
    /// for Lua-driven daemon-side edits); frontends route `Ctrl-4`
    /// through this method via `optimistic::frontend_event_for_keystroke`.
    ///
    /// # Cursor staleness
    ///
    /// Undo can change content at arbitrary positions relative to
    /// the cursor — the inverse of an insert at position 17 deletes
    /// bytes at position 17, but the local cursor may be at 42
    /// (after subsequent edits). The cursor for this buffer is
    /// marked stale on successful undo; the daemon's next
    /// `CursorByte` re-grounds it. Optimistic-apply round-trips
    /// while stale.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(bytes))` — the undo succeeded and produced an
    ///   inverse op. The caller should send this as a
    ///   `FrontendEvent::CrdtOp`.
    /// - `Ok(None)` — nothing to undo on this frontend's local
    ///   replica (the `UndoManager`'s stack is empty). Caller should
    ///   round-trip the original keystroke; daemon's `buffer.undo`
    ///   may have its own daemon-peer ops to undo (Lua-driven
    ///   edits), so the Key path remains the right fallback.
    /// - `Err(BufferMirrorError::NotReady)` — buffer hasn't received
    ///   a snapshot yet (bootstrap window). Caller should round-trip.
    /// - `Err(BufferMirrorError::Loro)` — loro's undo or export
    ///   failed. Caller should round-trip.
    pub fn apply_local_undo(
        &mut self,
        buffer_id: BufferId,
    ) -> Result<Option<Vec<u8>>, BufferMirrorError> {
        let state = self
            .states
            .get_mut(&buffer_id)
            .ok_or(BufferMirrorError::NotReady(buffer_id))?;
        let version_before = state.version();
        let did_undo = state.undo()?;
        if !did_undo {
            return Ok(None);
        }
        let bytes = state.export_updates_since(&version_before).map_err(|e| {
            BufferMirrorError::Loro(LoroError::DecodeError(
                format!("export_updates: {e:?}").into(),
            ))
        })?;
        // Content changed at arbitrary positions; cursor needs to be
        // re-grounded by the daemon's next `CursorByte`. Same shape as
        // `apply_remote_op`'s post-content-change handling below.
        self.stale_cursors.insert(buffer_id);
        Ok(Some(bytes))
    }

    /// Apply a remote op (received via `InstanceMessage::CrdtOp`) to
    /// the mirror for `buffer_id`.
    ///
    /// # Echo filtering — caller's responsibility
    ///
    /// **Filter rule:** call this method only when the broadcast's
    /// source `FrontendId` differs from this frontend's assigned
    /// `FrontendId`. Ops whose source matches the local frontend are
    /// echoes of locally-applied edits; passing them here would
    /// double-apply (the local op was already integrated by
    /// `apply_local_insert`/`apply_local_delete` at keystroke time).
    ///
    /// The mirror layer doesn't know about `FrontendId` or session
    /// identity — it operates on op bytes only. The attach loop in
    /// `attach.rs` performs the `FrontendId` comparison before invoking
    /// this method.
    ///
    /// # Errors
    ///
    /// - [`BufferMirrorError::NotReady`] if no snapshot has been
    ///   applied for `buffer_id`.
    /// - [`BufferMirrorError::Loro`] if the op bytes don't decode.
    ///   A per-op failure doesn't corrupt the mirror; the caller can
    ///   log and continue.
    pub fn apply_remote_op(
        &mut self,
        buffer_id: BufferId,
        op_bytes: &[u8],
    ) -> Result<(), BufferMirrorError> {
        let state = self
            .states
            .get_mut(&buffer_id)
            .ok_or(BufferMirrorError::NotReady(buffer_id))?;
        state
            .import_updates(op_bytes)
            .map_err(BufferMirrorError::Loro)?;
        // F23 — content changed; the mirror's cursor for this buffer
        // doesn't auto-adjust with right-gravity, so it may now point
        // at the wrong byte relative to the new content. Mark stale
        // until the daemon's next `CursorByte` re-grounds it. The
        // optimistic-apply orchestrator round-trips while stale.
        self.stale_cursors.insert(buffer_id);
        Ok(())
    }

    /// Materialize the current text of `buffer_id` as a String.
    /// Returns None if the buffer isn't ready.
    #[must_use]
    pub fn materialize(&self, buffer_id: BufferId) -> Option<String> {
        self.states
            .get(&buffer_id)
            .map(CrdtState::materialize_string)
    }

    /// UTF-8 byte length of `buffer_id`'s current content. Returns
    /// None if not ready. The frontend uses this for cursor-bounds
    /// checks before generating delete-forward ops.
    #[must_use]
    pub fn len_utf8(&self, buffer_id: BufferId) -> Option<usize> {
        self.states.get(&buffer_id).map(CrdtState::len_utf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_snapshot(seed_peer_id: u64, initial_text: &str) -> Vec<u8> {
        // Build a CrdtState on a synthetic peer, seed text, export.
        // Models what the daemon's CRDT-aware buffer does at attach
        // time.
        let state = CrdtState::new(seed_peer_id).expect("new");
        state.insert(0, initial_text).expect("seed insert");
        state.export_snapshot().expect("export")
    }

    #[test]
    fn fresh_mirror_has_no_buffers_and_is_not_ready_for_any() {
        let m = BufferMirror::new(FrontendId(7));
        let some_id = BufferId::next();
        assert!(!m.is_ready(some_id));
        assert!(m.materialize(some_id).is_none());
        assert!(m.len_utf8(some_id).is_none());
    }

    #[test]
    fn is_ready_returns_false_for_buffer_never_snapshotted() {
        // Verifies the "buffer just got created on another frontend;
        // this frontend hasn't received the snapshot yet" path from
        // Refinement 6. A BufferId the mirror has never heard of
        // returns false, not a panic.
        let mut m = BufferMirror::new(FrontendId(2));
        let known = BufferId::next();
        let unknown = BufferId::next();
        m.init_from_snapshot(known, &fresh_snapshot(99, "x"))
            .expect("init known");
        assert!(m.is_ready(known));
        assert!(!m.is_ready(unknown));
    }

    #[test]
    fn init_from_snapshot_makes_buffer_ready() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        let snap = fresh_snapshot(99, "hello");
        m.init_from_snapshot(id, &snap).expect("init");

        assert!(m.is_ready(id));
        assert_eq!(m.materialize(id).as_deref(), Some("hello"));
        assert_eq!(m.len_utf8(id), Some("hello".len()));
    }

    #[test]
    fn init_from_snapshot_twice_errors_already_initialized() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "first"))
            .expect("init 1");

        let err = m
            .init_from_snapshot(id, &fresh_snapshot(99, "second"))
            .expect_err("second init must error");
        match err {
            BufferMirrorError::AlreadyInitialized(b) => assert_eq!(b, id),
            other => panic!("expected AlreadyInitialized, got {other:?}"),
        }
        // The first snapshot's state is preserved — the second-init
        // attempt did not corrupt it.
        assert_eq!(m.materialize(id).as_deref(), Some("first"));
    }

    #[test]
    fn local_insert_modifies_mirror_and_returns_op_bytes() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        let snap = fresh_snapshot(99, "abc");
        m.init_from_snapshot(id, &snap).expect("init");

        let op = m.apply_local_insert(id, 3, "X").expect("insert");
        assert!(!op.is_empty(), "op bytes must be non-empty");
        assert_eq!(m.materialize(id).as_deref(), Some("abcX"));
    }

    #[test]
    fn local_delete_modifies_mirror_and_returns_op_bytes() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        let snap = fresh_snapshot(99, "abcde");
        m.init_from_snapshot(id, &snap).expect("init");

        let op = m.apply_local_delete(id, 1, 2).expect("delete");
        assert!(!op.is_empty());
        assert_eq!(m.materialize(id).as_deref(), Some("ade"));
    }

    #[test]
    fn remote_op_applies_to_mirror() {
        // Build a snapshot from peer 99, then synthesize a remote
        // op also produced by peer 99 (simulates the receiving
        // frontend hasn't typed anything and another peer's edit
        // arrives).
        let donor = CrdtState::new(99).expect("donor new");
        donor.insert(0, "abc").expect("donor seed");
        let snap = donor.export_snapshot().expect("snap");
        let v_before = donor.version();
        donor.insert(3, "Y").expect("donor edit");
        let op_bytes = donor
            .export_updates_since(&v_before)
            .expect("export updates");

        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &snap).expect("init");
        m.apply_remote_op(id, &op_bytes).expect("apply remote");

        assert_eq!(m.materialize(id).as_deref(), Some("abcY"));
    }

    #[test]
    fn apply_local_insert_on_unready_buffer_errors_not_ready() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        let err = m.apply_local_insert(id, 0, "x").expect_err("must error");
        match err {
            BufferMirrorError::NotReady(b) => assert_eq!(b, id),
            other => panic!("expected NotReady, got {other:?}"),
        }
        assert!(!m.is_ready(id));
    }

    #[test]
    fn apply_local_delete_on_unready_buffer_errors_not_ready() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        let err = m.apply_local_delete(id, 0, 1).expect_err("must error");
        assert!(matches!(err, BufferMirrorError::NotReady(b) if b == id));
    }

    #[test]
    fn apply_remote_op_on_unready_buffer_errors_not_ready() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        let err = m
            .apply_remote_op(id, &[0xCA, 0xFE])
            .expect_err("must error");
        assert!(matches!(err, BufferMirrorError::NotReady(b) if b == id));
    }

    // -----------------------------------------------------------------
    // Cursor tracking (T M10.10 Finding 2 — CursorByte wire variant).
    // -----------------------------------------------------------------

    #[test]
    fn cursor_byte_pos_returns_none_before_any_cursor_byte_received() {
        let m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        assert!(m.cursor_byte_pos(id).is_none());
    }

    #[test]
    fn cursor_byte_pos_returns_none_for_unknown_buffer_even_when_others_have_cursors() {
        let mut m = BufferMirror::new(FrontendId(2));
        let known = BufferId::next();
        let unknown = BufferId::next();
        m.set_cursor_byte_pos(known, 42);
        assert_eq!(m.cursor_byte_pos(known), Some(42));
        assert!(m.cursor_byte_pos(unknown).is_none());
    }

    #[test]
    fn set_cursor_byte_pos_overwrites_prior_value() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 10);
        m.set_cursor_byte_pos(id, 25);
        assert_eq!(m.cursor_byte_pos(id), Some(25));
    }

    #[test]
    fn advance_cursor_increments_existing_position() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 5);
        m.advance_cursor(id, 3);
        assert_eq!(m.cursor_byte_pos(id), Some(8));
    }

    #[test]
    fn advance_cursor_on_unknown_buffer_is_noop() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.advance_cursor(id, 5);
        assert!(m.cursor_byte_pos(id).is_none());
    }

    #[test]
    fn retreat_cursor_decrements_existing_position() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 10);
        m.retreat_cursor(id, 3);
        assert_eq!(m.cursor_byte_pos(id), Some(7));
    }

    #[test]
    fn retreat_cursor_saturates_at_zero() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 2);
        m.retreat_cursor(id, 10);
        assert_eq!(m.cursor_byte_pos(id), Some(0));
    }

    #[test]
    fn authoritative_cursor_update_overwrites_optimistic_advance() {
        // Models the daemon-correction path: frontend optimistically
        // advanced cursor; daemon's authoritative CursorByte arrives
        // and overwrites with the daemon's source-of-truth value.
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 5);
        m.advance_cursor(id, 1); // optimistic: 5 → 6
        m.set_cursor_byte_pos(id, 7); // daemon says actual is 7
        assert_eq!(m.cursor_byte_pos(id), Some(7));
    }

    // -----------------------------------------------------------------
    // active_buffer tracking (broadened CursorByte semantics).
    // -----------------------------------------------------------------

    #[test]
    fn active_buffer_is_none_before_any_cursor_byte_received() {
        let m = BufferMirror::new(FrontendId(2));
        assert!(m.active_buffer().is_none());
    }

    #[test]
    fn set_cursor_byte_pos_updates_active_buffer() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 5);
        assert_eq!(m.active_buffer(), Some(id));
    }

    #[test]
    fn active_buffer_tracks_most_recent_cursor_byte_buffer() {
        // Broadened CursorByte semantics: each CursorByte represents
        // "active buffer + cursor in it". Buffer switches are
        // reflected as active_buffer changes.
        let mut m = BufferMirror::new(FrontendId(2));
        let a = BufferId::next();
        let b = BufferId::next();
        m.set_cursor_byte_pos(a, 10);
        assert_eq!(m.active_buffer(), Some(a));
        m.set_cursor_byte_pos(b, 0);
        assert_eq!(m.active_buffer(), Some(b));
        // Prior cursor for `a` is preserved (per-buffer cursors are
        // independent); only the active-buffer pointer switches.
        assert_eq!(m.cursor_byte_pos(a), Some(10));
        assert_eq!(m.cursor_byte_pos(b), Some(0));
    }

    // -----------------------------------------------------------------
    // prev_char_len / next_char_len — char-boundary-aware byte counts.
    // -----------------------------------------------------------------

    #[test]
    fn prev_char_len_ascii_is_one_byte() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5); // cursor at end
        assert_eq!(m.prev_char_len(id), Some(1));
    }

    #[test]
    fn prev_char_len_multibyte_unicode_is_2_to_4_bytes() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        // 'é' is 2 bytes (U+00E9: C3 A9); '中' is 3 bytes (E4 B8 AD).
        m.init_from_snapshot(id, &fresh_snapshot(99, "aé中"))
            .expect("init");
        // Cursor right after '中' (the 3-byte char).
        // "aé中" = 1 + 2 + 3 = 6 bytes.
        m.set_cursor_byte_pos(id, 6);
        assert_eq!(m.prev_char_len(id), Some(3));
        // Cursor right after 'é' (the 2-byte char).
        m.set_cursor_byte_pos(id, 3);
        assert_eq!(m.prev_char_len(id), Some(2));
        // Cursor right after 'a' (1-byte ASCII).
        m.set_cursor_byte_pos(id, 1);
        assert_eq!(m.prev_char_len(id), Some(1));
    }

    #[test]
    fn prev_char_len_at_position_zero_is_none() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "x"))
            .expect("init");
        m.set_cursor_byte_pos(id, 0);
        assert!(m.prev_char_len(id).is_none());
    }

    #[test]
    fn prev_char_len_without_cursor_is_none() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "x"))
            .expect("init");
        // No CursorByte received → no cursor tracked.
        assert!(m.prev_char_len(id).is_none());
    }

    #[test]
    fn next_char_len_ascii_is_one_byte() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        m.set_cursor_byte_pos(id, 0);
        assert_eq!(m.next_char_len(id), Some(1));
    }

    #[test]
    fn next_char_len_multibyte_unicode_is_2_to_4_bytes() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        // "é中a" = 2 + 3 + 1 = 6 bytes.
        m.init_from_snapshot(id, &fresh_snapshot(99, "é中a"))
            .expect("init");
        m.set_cursor_byte_pos(id, 0); // before 'é'
        assert_eq!(m.next_char_len(id), Some(2));
        m.set_cursor_byte_pos(id, 2); // before '中'
        assert_eq!(m.next_char_len(id), Some(3));
        m.set_cursor_byte_pos(id, 5); // before 'a'
        assert_eq!(m.next_char_len(id), Some(1));
    }

    #[test]
    fn next_char_len_at_end_is_none() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "abc"))
            .expect("init");
        m.set_cursor_byte_pos(id, 3);
        assert!(m.next_char_len(id).is_none());
    }

    #[test]
    fn peer_id_matches_frontend_derivation() {
        let m = BufferMirror::new(FrontendId(42));
        assert_eq!(m.peer_id(), 42);
    }

    // -----------------------------------------------------------------
    // cursor_at_end_of_line — Path β end-of-line predicate.
    // -----------------------------------------------------------------

    #[test]
    fn cursor_at_end_of_line_is_true_at_buffer_end() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5);
        assert_eq!(m.cursor_at_end_of_line(id), Some(true));
    }

    #[test]
    fn cursor_at_end_of_line_is_false_mid_line() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello world"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5); // between 'hello' and ' world'
        assert_eq!(m.cursor_at_end_of_line(id), Some(false));
    }

    #[test]
    fn cursor_at_end_of_line_is_true_before_newline() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "foo\nbar"))
            .expect("init");
        m.set_cursor_byte_pos(id, 3); // immediately before '\n'
        assert_eq!(m.cursor_at_end_of_line(id), Some(true));
    }

    #[test]
    fn cursor_at_end_of_line_is_false_at_line_start_with_content() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "foo\nbar"))
            .expect("init");
        m.set_cursor_byte_pos(id, 4); // start of "bar" line
        assert_eq!(m.cursor_at_end_of_line(id), Some(false));
    }

    #[test]
    fn cursor_at_end_of_line_is_true_on_empty_line() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "foo\n\nbar"))
            .expect("init");
        m.set_cursor_byte_pos(id, 4); // empty line between foo and bar
        assert_eq!(m.cursor_at_end_of_line(id), Some(true));
    }

    // -----------------------------------------------------------------
    // cursor_at_end_of_line_safe_for_delete_back — Finding 5 narrower
    // predicate that excludes line-joining backspaces.
    // -----------------------------------------------------------------

    #[test]
    fn safe_for_delete_back_true_when_prev_char_is_not_newline() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5); // cursor after 'o'
        assert_eq!(m.cursor_at_end_of_line_safe_for_delete_back(id), Some(true));
    }

    #[test]
    fn safe_for_delete_back_false_when_prev_char_is_newline() {
        // Cursor at byte position 4 (start of empty line after "foo\n").
        // bytes[4] is past end → end-of-line=true.
        // But prev char is '\n' — backspace would join lines.
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "foo\n"))
            .expect("init");
        m.set_cursor_byte_pos(id, 4);
        // Original predicate says yes (cursor at end-of-line)
        assert_eq!(m.cursor_at_end_of_line(id), Some(true));
        // Stricter predicate says no (would be line-join)
        assert_eq!(
            m.cursor_at_end_of_line_safe_for_delete_back(id),
            Some(false)
        );
    }

    #[test]
    fn safe_for_delete_back_false_when_cursor_at_zero() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "abc"))
            .expect("init");
        m.set_cursor_byte_pos(id, 0);
        assert_eq!(
            m.cursor_at_end_of_line_safe_for_delete_back(id),
            Some(false)
        );
    }

    #[test]
    fn safe_for_delete_back_false_mid_line() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello world"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5);
        assert_eq!(
            m.cursor_at_end_of_line_safe_for_delete_back(id),
            Some(false)
        );
    }

    #[test]
    fn cursor_at_end_of_line_is_none_when_buffer_not_ready() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.set_cursor_byte_pos(id, 5);
        assert!(m.cursor_at_end_of_line(id).is_none());
    }

    // ------------------------------------------------------------------
    // F20 (post-audit-round-3) — width-aware safe-for-delete-back
    // predicate. The paint sequence `MoveLeft(1), Print(' '),
    // MoveLeft(1)` is column-accurate only when the previous char
    // renders to exactly one column. Wide chars, tabs, and zero-
    // width combining marks break that invariant.
    // ------------------------------------------------------------------

    #[test]
    fn safe_for_delete_back_false_for_wide_prev_char() {
        // CJK ideograph 漢 has UnicodeWidthChar::width == Some(2).
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "漢"))
            .expect("init");
        // "漢" is 3 UTF-8 bytes; cursor at end-of-line.
        m.set_cursor_byte_pos(id, "漢".len());
        assert_eq!(
            m.cursor_at_end_of_line_safe_for_delete_back(id),
            Some(false),
            "F20: wide-char delete-back must not be optimistically painted"
        );
    }

    #[test]
    fn safe_for_delete_back_false_for_tab_prev_char() {
        // Tab has UnicodeWidthChar::width == None — paint sequence
        // can't account for variable tab expansion width.
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "x\t"))
            .expect("init");
        m.set_cursor_byte_pos(id, "x\t".len());
        assert_eq!(
            m.cursor_at_end_of_line_safe_for_delete_back(id),
            Some(false),
            "F20: tab delete-back must not be optimistically painted"
        );
    }

    #[test]
    fn safe_for_delete_back_false_for_combining_mark_prev_char() {
        // "a" + U+0301 (COMBINING ACUTE ACCENT, width 0). The
        // combining mark attaches to "a"'s cell; erasing it as if
        // it were a width-1 cell would clear "a"'s glyph.
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        let s = "a\u{0301}";
        m.init_from_snapshot(id, &fresh_snapshot(99, s))
            .expect("init");
        m.set_cursor_byte_pos(id, s.len());
        assert_eq!(
            m.cursor_at_end_of_line_safe_for_delete_back(id),
            Some(false),
            "F20: combining-mark delete-back must not be optimistically painted"
        );
    }

    // ------------------------------------------------------------------
    // F22 + F23 (post-audit-round-4) — mirror-cursor freshness
    // invariant. The mirror cursor must round-trip after any event
    // that may have desynced it from the daemon's authoritative
    // cursor.
    // ------------------------------------------------------------------

    #[test]
    fn cursor_starts_fresh_after_init() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        // Fresh by default; no CursorByte has arrived yet but no
        // staleness-inducing event has happened either.
        assert!(m.is_cursor_fresh(id));
    }

    #[test]
    fn mark_cursor_stale_makes_fresh_false() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5);
        assert!(m.is_cursor_fresh(id));
        m.mark_cursor_stale(id);
        assert!(
            !m.is_cursor_fresh(id),
            "F22: mark_cursor_stale must make is_cursor_fresh return false"
        );
    }

    #[test]
    fn set_cursor_byte_pos_clears_staleness() {
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5);
        m.mark_cursor_stale(id);
        assert!(!m.is_cursor_fresh(id));
        // Daemon's authoritative CursorByte re-grounds the mirror.
        m.set_cursor_byte_pos(id, 7);
        assert!(
            m.is_cursor_fresh(id),
            "F22: a fresh CursorByte from the daemon must clear the stale flag"
        );
    }

    #[test]
    fn apply_remote_op_marks_cursor_stale() {
        // F23: a remote CRDT op changes content; the mirror's cursor
        // doesn't right-gravity-adjust, so it may now point at the
        // wrong byte. Must be marked stale until the daemon's next
        // CursorByte arrives.
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "hello"))
            .expect("init");
        m.set_cursor_byte_pos(id, 5);
        assert!(m.is_cursor_fresh(id));

        // Build a remote op against a peer-99 replica that inserts
        // "X" at position 0. The export contains an update we can
        // import into our mirror.
        let peer = CrdtState::new(99).expect("peer");
        peer.import_snapshot(&fresh_snapshot(99, "hello"))
            .expect("peer init");
        let v0 = peer.version();
        peer.insert(0, "X").expect("peer insert");
        let op_bytes = peer.export_updates_since(&v0).expect("export");

        m.apply_remote_op(id, &op_bytes).expect("apply remote");
        assert!(
            !m.is_cursor_fresh(id),
            "F23: apply_remote_op must mark the cursor stale (right-gravity not done locally)"
        );
    }

    #[test]
    fn safe_for_delete_back_true_for_plain_ascii_prev_char() {
        // Regression: width-1 ASCII is the typical case and must
        // still return true.
        let mut m = BufferMirror::new(FrontendId(2));
        let id = BufferId::next();
        m.init_from_snapshot(id, &fresh_snapshot(99, "abc"))
            .expect("init");
        m.set_cursor_byte_pos(id, 3);
        assert_eq!(m.cursor_at_end_of_line_safe_for_delete_back(id), Some(true));
    }
}
