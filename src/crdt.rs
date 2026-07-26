//! T M10.2: CRDT-backed buffer state.
//!
//! This module implements the wrapper layer that mediates between the
//! editor's existing rope contract and the underlying CRDT library
//! (loro 1.12, selected in M10.1).
//!
//! # Architecture (per M10.1 Decision section, §sec:m10-crdt-choice)
//!
//! The CRDT lives on the main thread. Workers consume a *rope
//! projection* materialized from the CRDT state via the existing
//! [`crate::rope::Rope`] type and the existing
//! [`crate::buffer::Buffer::snapshot_rope`] API. Workers never see
//! the CRDT directly; the rope-projection redirect preserves the
//! v0.1 worker contract.
//!
//! Two cost-model paths:
//!
//! * **Hot path**: per-edit incremental projection updates. Each
//!   CRDT op produces an [`crate::rope::Edit`] description that
//!   updates the rope projection at O(log n) via the existing rope
//!   edit path. This is the cost shape the editor already pays.
//! * **Cold path**: attach-time / catastrophic-divergence full
//!   re-materialization. O(n); paid once per attach, dispatched to a
//!   foreground worker for documents above ~1MB.
//!
//! # Feature gating
//!
//! The whole module is gated behind the `crdt` Cargo feature so v0.1
//! builds carry zero CRDT overhead — the `loro` dependency isn't
//! pulled in, no field on the [`crate::buffer::Buffer`] struct
//! layout, no branch on `apply_edit`. v1.0 builds enable `crdt`.
//!
//! # Day 1 scope
//!
//! M10.2 Day 1 builds the minimal `CrdtState` wrapper: insert/delete
//! operations against a [`loro::LoroDoc`], a `from_bytes` constructor
//! that seeds a `CrdtState` from existing rope contents, and a
//! [`CrdtState::materialize_string`] projection extractor. Subsequent days wire this
//! into [`crate::buffer::Buffer`] and add the per-edit incremental
//! propagation, the optional `crdt_op` field on [`crate::rope::Edit`],
//! and the convergence proptest.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use loro::{
    ContainerTrait, ExportMode, LoroDoc, LoroEncodeError, LoroResult, TextDelta, UndoManager,
    VersionVector,
};

type TextDeltaBatches = Arc<Mutex<Vec<Vec<TextDelta>>>>;
type TextDeltaSubscription = (TextDeltaBatches, Arc<AtomicBool>, loro::Subscription);

/// The CRDT-backed buffer state.
///
/// Owns a [`LoroDoc`] with a single text container (named `"body"`).
/// Edit operations route through the loro APIs; the editor consumes
/// projections via [`Self::materialize_string`] for the rope materialization.
///
/// `Send`-but-not-`Sync`: a single-threaded owner (the main thread)
/// holds it; workers consume the rope projection, not the CRDT. The
/// `Send` bound is needed for the M10.2 Day 7 cold-path attach
/// (foreground worker materializes the initial projection).
pub struct CrdtState {
    doc: LoroDoc,
    /// Text projection deltas captured only while a remote import is
    /// active. Keeping the subscription alive avoids registering and
    /// dropping one callback for every typed character.
    text_delta_batches: TextDeltaBatches,
    text_delta_capture_enabled: Arc<AtomicBool>,
    _text_delta_subscription: loro::Subscription,
    /// T M10.4: per-peer undo machinery. Bound to `doc`'s `peer_id`
    /// at construction; produces inverse ops attributed to that peer.
    ///
    /// Loro's `UndoManager`:
    /// - Local-only: undoes the bound peer's most recent change, not
    ///   the document's most recent change (the M10.4 collaborative
    ///   semantics that "undo my edits, not theirs" is provided by
    ///   loro's underlying design, not by pmacs).
    /// - Inverse ops interact with concurrent remote ops via the
    ///   CRDT's normal convergence rules — the M10.4 acceptance
    ///   criterion "B's edit lands on whatever surrounding text
    ///   remains" is loro's intrinsic behavior.
    /// - Default max undo steps: 100. Pmacs raises this to `10_000` to
    ///   match v0.1's effectively-unbounded undo stack semantics.
    ///
    /// `UndoManager` is `!Send + !Sync` internally; `CrdtState` is
    /// main-thread-only, matching the M10.1 rope-projection-redirect
    /// constraint (workers consume the rope, not the CRDT).
    undo: std::cell::RefCell<UndoManager>,
}

impl CrdtState {
    /// Construct an empty CRDT state.
    ///
    /// The `peer_id` is the producing-frontend identity for ops this
    /// state generates. M10.4 (per-frontend undo) consumes this as
    /// the "is this op mine?" filter; M10.5 (wire protocol) embeds it
    /// into broadcast `CrdtOp` messages.
    pub fn new(peer_id: u64) -> LoroResult<Self> {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer_id)?;
        // Ensure the "body" text container exists by creating the
        // handle. Loro creates containers lazily on first access; the
        // explicit get here ensures the container is registered before
        // any read or write.
        let _ = doc.get_text("body");
        let (text_delta_batches, text_delta_capture_enabled, text_delta_subscription) =
            Self::subscribe_text_deltas(&doc);
        let undo = Self::create_undo_manager(&doc);
        Ok(Self {
            doc,
            text_delta_batches,
            text_delta_capture_enabled,
            _text_delta_subscription: text_delta_subscription,
            undo: std::cell::RefCell::new(undo),
        })
    }

    fn subscribe_text_deltas(doc: &LoroDoc) -> TextDeltaSubscription {
        let text = doc.get_text("body");
        let batches = Arc::new(Mutex::new(Vec::<Vec<TextDelta>>::new()));
        let capture_enabled = Arc::new(AtomicBool::new(false));
        let captured_batches = Arc::clone(&batches);
        let captured_enabled = Arc::clone(&capture_enabled);
        let subscription = doc.subscribe(
            &text.id(),
            Arc::new(move |event| {
                if !captured_enabled.load(Ordering::Relaxed) {
                    return;
                }
                let mut guard = captured_batches
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for event in event.events {
                    if let Some(delta) = event.diff.as_text()
                        && !delta.is_empty()
                    {
                        guard.push(delta.clone());
                    }
                }
            }),
        );
        (batches, capture_enabled, subscription)
    }

    /// T M10.4: construct a fresh `UndoManager` bound to the given doc.
    /// Extracted as a helper because `from_bytes` constructs it AFTER
    /// the initial seed insert (so the seed isn't observable as an
    /// undoable op), while `new` constructs it before any ops happen
    /// (same effect).
    fn create_undo_manager(doc: &LoroDoc) -> UndoManager {
        let mut undo = UndoManager::new(doc);
        // Raise max undo steps from loro's default 100 to 10_000.
        // v0.1's undo stack is effectively unbounded; 10k is the
        // realistic ceiling for human-driven editing and gives users
        // ample undo depth without unreasonable memory cost.
        undo.set_max_undo_steps(10_000);
        undo
    }

    /// Construct a CRDT state seeded with the given bytes.
    ///
    /// Used by the cold-path attach: an existing rope's contents are
    /// loaded into a fresh CRDT state. The result has one initial op
    /// (the bulk insert at byte 0) attributed to `peer_id`.
    pub fn from_bytes(peer_id: u64, bytes: &[u8]) -> LoroResult<Self> {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer_id)?;
        let _ = doc.get_text("body");
        if !bytes.is_empty() {
            // Loro's text API takes &str; we accept `&[u8]` and decode
            // as UTF-8 with replacement to match the existing rope's
            // permissive byte-level semantics. Files that aren't valid
            // UTF-8 round-trip via the projection but lose the
            // ill-formed-byte detail; v0.1 has the same limitation
            // for any operation that reads bytes as text.
            let text = String::from_utf8_lossy(bytes);
            doc.get_text("body").insert(0, &text)?;
        }
        // T M10.4: construct UndoManager AFTER the initial seed
        // insert so the seed is not observable as an undoable op.
        // The buffer's starting contents (from file load, scratch
        // initial text, etc.) shouldn't be undoable from the user's
        // perspective; only post-construction edits are.
        let (text_delta_batches, text_delta_capture_enabled, text_delta_subscription) =
            Self::subscribe_text_deltas(&doc);
        let undo = Self::create_undo_manager(&doc);
        Ok(Self {
            doc,
            text_delta_batches,
            text_delta_capture_enabled,
            _text_delta_subscription: text_delta_subscription,
            undo: std::cell::RefCell::new(undo),
        })
    }

    /// Return the producing-frontend identity for this state.
    pub fn peer_id(&self) -> u64 {
        self.doc.peer_id()
    }

    /// Insert `s` at byte position `pos` in the body text.
    ///
    /// Routes to loro's byte-native `insert_utf8` per the M10.2 Day 2
    /// morning audit (Q1 verification). Mid-codepoint positions are
    /// rejected with a clear error from loro; callers should ensure
    /// `pos` aligns to a UTF-8 codepoint boundary.
    pub fn insert(&self, pos: usize, s: &str) -> LoroResult<()> {
        self.doc.get_text("body").insert_utf8(pos, s)
    }

    /// Delete `len` bytes starting at byte position `pos`. Same
    /// codepoint-boundary requirement as [`Self::insert`].
    pub fn delete(&self, pos: usize, len: usize) -> LoroResult<()> {
        self.doc.get_text("body").delete_utf8(pos, len)
    }

    /// Length of the body text in bytes (UTF-8 length).
    pub fn len_utf8(&self) -> usize {
        self.doc.get_text("body").len_utf8()
    }

    /// Length of the body text in unicode code points. Used by tests
    /// that assert on codepoint-vs-byte distinctions; production
    /// callers should prefer [`Self::len_utf8`] which matches the
    /// rope's byte-length contract.
    pub fn len_unicode(&self) -> usize {
        self.doc.get_text("body").len_unicode()
    }

    /// Materialize the rope projection as a `String`.
    ///
    /// O(n). Day 1 ships the trivial projection (full string
    /// extraction); Day 2's Buffer integration replaces this with the
    /// per-edit-incremental path for hot-path edits and reserves
    /// this method for the cold-path attach.
    ///
    /// Named `materialize_string` rather than `to_string` to avoid
    /// shadowing the [`std::fmt::Display`]-derived `to_string`
    /// (clippy `inherent_to_string`); semantics are explicit at the
    /// call site (it's a projection materialization, not a display
    /// conversion).
    pub fn materialize_string(&self) -> String {
        self.doc.get_text("body").to_string()
    }

    /// T M10.2 Day 3: capture the current oplog frontier.
    ///
    /// Used as the `from` argument to a subsequent
    /// [`Self::export_updates_since`] call to capture the wire bytes
    /// for ops produced between the two version captures. Cheap; the
    /// version vector is a small data structure that loro maintains
    /// alongside its op log.
    pub fn version(&self) -> VersionVector {
        self.doc.oplog_vv()
    }

    /// T M11.2 — the oplog version projected to a single monotonic
    /// scalar: the sum of every peer's op counter in the version
    /// vector.
    ///
    /// This is the `generation` anchor for the semantic projection
    /// (`InstanceMessage::StyleSpans::generation`). A loro counter is
    /// per-peer non-decreasing and only ever grows as ops accrue, so
    /// the sum is non-decreasing for the document as a whole — a
    /// frontend can compare a received `generation` against the one
    /// it computed locally and discard styling that predates an edit
    /// it already applied optimistically. It is deliberately *not* a
    /// causal clock: equal scalars do not imply equal states across
    /// divergent replicas. It is only ever compared against itself on
    /// one replica (the frontend's own mirror vs. the instance's
    /// authoritative doc), where it is monotone, which is all the
    /// staleness check needs.
    #[must_use]
    pub fn version_scalar(&self) -> u64 {
        self.doc
            .oplog_vv()
            .values()
            .map(|counter| u64::try_from(*counter).unwrap_or(0))
            .sum()
    }

    /// T M10.2 Day 3: export wire-format bytes for ops added since
    /// `from`.
    ///
    /// Used by [`crate::buffer::Buffer`] to capture the per-edit op
    /// delta that populates [`crate::rope::Edit::crdt_op`]. The bytes
    /// are loro's incremental-update format; M10.5 (wire protocol)
    /// sends them across the wire to remote frontends, which import
    /// via [`Self::import_snapshot`] (or its updates-shaped variant).
    ///
    /// The capture-apply-export idiom is:
    /// 1. `let pre = state.version();`
    /// 2. apply ops via [`Self::insert`] / [`Self::delete`] / etc.
    /// 3. `let bytes = state.export_updates_since(&pre)?;`
    ///
    /// Loro's transactional model gives a consistent before/after
    /// pair: the version captured before the op does not include the
    /// op's effect, and the export from that version captures
    /// exactly the ops that were applied after the capture (the
    /// per-edit delta the M10.2 Day 3 framing requires).
    ///
    /// Returns empty bytes if no ops have been applied since `from`
    /// (no-op case detection is the caller's responsibility — empty
    /// bytes still has loro's structural overhead, so byte-length
    /// alone isn't a reliable empty-check; the caller pre-checks
    /// the `EditOp` variants instead).
    pub fn export_updates_since(&self, from: &VersionVector) -> Result<Vec<u8>, LoroEncodeError> {
        self.doc.export(ExportMode::updates(from))
    }

    /// Export a wire-format snapshot of the entire CRDT state.
    ///
    /// Used by M10.5 wire-protocol serialization for full-state-sync
    /// to reconnecting frontends. Loro's snapshot is run-encoded
    /// (~0.8% of source at 1MB+ per the M10.1 measurements), so the
    /// wire bandwidth cost is small.
    pub fn export_snapshot(&self) -> Result<Vec<u8>, LoroEncodeError> {
        self.doc.export(ExportMode::Snapshot)
    }

    /// Import a wire-format snapshot into this state.
    ///
    /// Inverse of [`Self::export_snapshot`]. Used by M10.5 attach
    /// flow: a frontend receives a snapshot from the instance and
    /// constructs its local state from it.
    pub fn import_snapshot(&self, bytes: &[u8]) -> LoroResult<()> {
        self.doc.import(bytes).map(|_| ())
    }

    /// T M10.2 Day 4: import incremental update bytes from a remote
    /// peer.
    ///
    /// Inverse of [`Self::export_updates_since`]. Used by M10.5's
    /// wire-protocol layer when a frontend receives a `CrdtOp`
    /// message broadcast by the instance — the bytes are the delta
    /// the originating frontend produced; this method merges them
    /// into the local CRDT.
    ///
    /// Loro's underlying `doc.import` accepts both full snapshots
    /// AND incremental updates (the format is universal); the
    /// separate method exists to make call sites self-documenting:
    ///
    /// * [`Self::import_snapshot`] — full-state replacement (attach
    ///   path); call site signals "I'm receiving the whole state."
    /// * [`Self::import_updates`] — partial delta merge (per-edit
    ///   path); call site signals "I'm receiving incremental ops."
    ///
    /// Loro's CRDT semantics handle the merge: concurrent ops from
    /// different peers converge regardless of import order, which is
    /// the property M10.2 Day 4's convergence proptest verifies.
    pub fn import_updates(&self, bytes: &[u8]) -> LoroResult<()> {
        self.doc.import(bytes).map(|_| ())
    }

    /// Import remote updates and capture Loro's text projection deltas.
    ///
    /// The import callback runs synchronously before `doc.import`
    /// returns. Buffer's hot path uses the captured single-insert shape
    /// to update its rope projection without materializing the whole
    /// document.
    pub fn import_updates_with_text_deltas(&self, bytes: &[u8]) -> LoroResult<Vec<Vec<TextDelta>>> {
        self.text_delta_batches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.text_delta_capture_enabled
            .store(true, Ordering::Relaxed);
        let import_result = self.doc.import(bytes).map(|_| ());
        self.text_delta_capture_enabled
            .store(false, Ordering::Relaxed);
        import_result?;
        let mut guard = self
            .text_delta_batches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(std::mem::take(&mut *guard))
    }

    /// Convert a Unicode scalar offset in the current text projection
    /// to its UTF-8 byte offset.
    #[must_use]
    pub fn unicode_to_utf8_pos(&self, pos: usize) -> Option<usize> {
        self.doc.get_text("body").convert_pos(
            pos,
            loro::cursor::PosType::Unicode,
            loro::cursor::PosType::Bytes,
        )
    }

    /// T M10.10 post-audit-round-4 F26 — validate that importing
    /// the wire bytes `bytes` would attribute every new op to
    /// `expected_peer_id`.
    ///
    /// Imports into a forked clone so the live state isn't mutated.
    /// Compares the oplog version vector before and after import:
    /// every peer whose counter advances must be `expected_peer_id`,
    /// or the bytes carry ops produced under a different identity
    /// than the wire wrapper's `op.peer_id`.
    ///
    /// Returns:
    /// - `Ok(())` — every advancing peer matches `expected_peer_id`,
    ///   OR the loro decoder rejected the bytes (the real import
    ///   will surface the same decode error to the caller; we don't
    ///   reject on validation grounds here).
    /// - `Err(actual)` — the first peer whose counter advanced and
    ///   doesn't match `expected_peer_id`. The caller (the daemon's
    ///   `validate_remote_crdt_op`) treats this as a protocol
    ///   violation and drops the op.
    ///
    /// Cost: doubles the import cost. Loro's `fork()` is shallow;
    /// the real import reapplies. Acceptable for v1.0 throughput;
    /// v0.2+ may add a peek API to loro that avoids the second
    /// import.
    pub fn validate_update_peer_ids(&self, expected_peer_id: u64, bytes: &[u8]) -> Result<(), u64> {
        let fork = self.doc.fork();
        let before = fork.oplog_vv();
        if fork.import(bytes).is_err() {
            // Decode error: let the real import surface it.
            return Ok(());
        }
        let after = fork.oplog_vv();
        for (peer, after_counter) in after.iter() {
            let before_counter = before.get(peer).copied().unwrap_or(0);
            if *after_counter > before_counter && *peer != expected_peer_id {
                return Err(*peer);
            }
        }
        Ok(())
    }

    /// T M10.4: undo the bound peer's most recent change.
    ///
    /// Returns `true` if an undo was performed, `false` if there was
    /// nothing to undo (the undo stack was empty). Inverse ops are
    /// applied to the doc; the projection (`materialize_string`)
    /// reflects the post-undo state immediately. Callers needing the
    /// wire-format bytes for the inverse should use the
    /// `version()` → `undo()` → `export_updates_since()` pattern,
    /// mirroring the `apply_edit` path.
    ///
    /// Loro's `UndoManager`:
    /// - Affects only the bound peer's ops; remote ops are unchanged
    /// - Inverse interacts with concurrent remote ops via CRDT
    ///   convergence (M10.4 acceptance: "B's edit lands on
    ///   whatever surrounding text remains")
    ///
    /// `&self` (not `&mut self`) via interior mutability: the
    /// underlying `UndoManager` needs `&mut` but pmacs's call sites
    /// hold `CrdtState` by reference. `RefCell` gates this safely;
    /// the main-thread-only constraint means there's no contention.
    pub fn undo(&self) -> LoroResult<bool> {
        self.undo.borrow_mut().undo()
    }

    /// T M10.4: redo the most-recently-undone change by the bound peer.
    ///
    /// Symmetric to [`Self::undo`]. Returns `true` if a redo was
    /// performed, `false` if the redo stack was empty.
    pub fn redo(&self) -> LoroResult<bool> {
        self.undo.borrow_mut().redo()
    }

    /// T M10.4: whether the bound peer has anything to undo.
    pub fn can_undo(&self) -> bool {
        self.undo.borrow().can_undo()
    }

    /// T M10.4: whether the bound peer has anything to redo.
    pub fn can_redo(&self) -> bool {
        self.undo.borrow().can_redo()
    }

    /// T M10.4: record an undo checkpoint.
    ///
    /// Pmacs's `apply_edit` semantics is "each successful forward edit
    /// is its own undo unit." Loro's `UndoManager` groups ops into
    /// undo units by merge interval; default 0 means no merging,
    /// which matches pmacs's per-edit semantics naturally. The
    /// explicit checkpoint method is exposed for v0.2+ batch-op
    /// coalescing (Day 7 mitigation B) where multiple CRDT ops
    /// should group into one undo unit.
    pub fn record_checkpoint(&self) -> LoroResult<()> {
        self.undo.borrow_mut().record_new_checkpoint()
    }

    /// Discard the bound peer's undo and redo history, keeping the
    /// document itself untouched.
    ///
    /// Loro's `UndoManager` exposes no `clear`, but it does not need
    /// one: a manager records only what happens **after** it is
    /// constructed. [`Self::from_bytes`] already relies on exactly
    /// that property to keep the seed insert out of undo. Replacing
    /// the manager with a fresh one bound to the same doc therefore
    /// leaves nothing to undo, and drops the old manager's retained
    /// stacks with it.
    ///
    /// Used by [`crate::buffer::Buffer::set_generated_contents`], whose
    /// contract is that a generated buffer accumulates no history
    /// across refreshes. Marking the buffer read-only would stop the
    /// history being *replayed*, but not being *retained* — a panel
    /// refreshed on a timer would grow without bound.
    pub fn clear_undo_history(&self) {
        *self.undo.borrow_mut() = Self::create_undo_manager(&self.doc);
    }
}

/// T M10.3: map a [`crate::protocol::FrontendId`] to the loro `PeerID`
/// (u64) used as the producing-frontend identity in CRDT ops.
///
/// The mapping is **identity**: `FrontendId(n)` maps to `n`. Both
/// types are `u64` by design (`FrontendId(pub u64)`, loro `PeerID`
/// is `u64`-wrapped). The single invariant the mapping must
/// preserve is non-zero:
///
/// * `FrontendId::LOCAL = FrontendId(1)` — the v0.1 default; non-zero
/// * Multi-frontend allocations from M5's counter start above `LOCAL`
/// * Loro's `PeerID` is internally `NonZeroU64` (default feature) — zero
///   would panic at `LoroDoc::set_peer_id`
///
/// The identity mapping preserves the non-zero invariant by
/// construction: every `FrontendId` produced by pmacs's frontend
/// machinery is non-zero, and `FrontendId(0)` is reserved as the
/// "no frontend" sentinel (not allocated to any real attach). The
/// mapping is a thin wrapper rather than implicit conversion so
/// call sites are explicit about which value-space they're in:
/// the wire-protocol layer uses `FrontendId`; the CRDT layer uses
/// `PeerID`; the boundary is this function.
///
/// M10.4's per-frontend undo reads this back from `CrdtOp.peer_id`
/// to decide whose ops to undo; M10.5's wire protocol embeds it
/// in `InstanceMessage::CrdtOp` broadcasts.
#[must_use]
pub fn peer_id_from_frontend(frontend_id: crate::protocol::FrontendId) -> u64 {
    // The non-zero invariant is a precondition; callers must not
    // pass FrontendId(0). Debug-assertion guards the assumption;
    // in release builds we trust callers and the identity mapping
    // returns 0 which loro will reject with a clear error at the
    // CrdtState::new boundary.
    debug_assert!(
        frontend_id.0 != 0,
        "FrontendId(0) is the no-frontend sentinel; loro PeerID requires non-zero \
         (NonZeroU64 internally). Caller passed a sentinel-valued FrontendId, \
         which is not a real frontend identity."
    );
    frontend_id.0
}

#[cfg(test)]
mod m10_3_tests {
    use super::*;
    use crate::protocol::FrontendId;

    #[test]
    fn peer_id_from_frontend_is_identity_for_local() {
        assert_eq!(peer_id_from_frontend(FrontendId::LOCAL), 1);
    }

    #[test]
    fn peer_id_from_frontend_is_identity_for_arbitrary_ids() {
        for raw in [2u64, 3, 100, 1_000_000, u64::MAX] {
            assert_eq!(peer_id_from_frontend(FrontendId(raw)), raw);
        }
    }

    #[test]
    fn peer_id_from_frontend_round_trips_through_crdt_state() {
        // The identity mapping is round-trip-safe: a CrdtState
        // constructed with peer_id_from_frontend(fid) returns the
        // same fid back when its peer_id is read.
        let fid = FrontendId(42);
        let state = CrdtState::new(peer_id_from_frontend(fid)).expect("new");
        assert_eq!(state.peer_id(), 42);
        assert_eq!(state.peer_id(), fid.0);
    }
}

// ---------------------------------------------------------------------------
// Smoke tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_empty() {
        let s = CrdtState::new(1).expect("new");
        assert_eq!(s.len_unicode(), 0);
        assert_eq!(s.materialize_string(), "");
        assert_eq!(s.peer_id(), 1);
    }

    #[test]
    fn version_scalar_is_monotonic_non_decreasing() {
        // T M11.2 — the semantic projection's `generation` anchor.
        // Empty doc is 0; each applied op only grows the scalar; a
        // no-op delete does not shrink it.
        let s = CrdtState::new(1).expect("new");
        assert_eq!(s.version_scalar(), 0, "empty doc has generation 0");

        s.insert(0, "hello").expect("insert");
        let g1 = s.version_scalar();
        assert!(g1 > 0, "an applied op must advance the generation");

        s.insert(5, " world").expect("insert");
        let g2 = s.version_scalar();
        assert!(g2 >= g1, "generation must not decrease across ops");

        s.delete(0, 1).expect("delete");
        let g3 = s.version_scalar();
        assert!(
            g3 >= g2,
            "a delete is still an op — the version vector only grows"
        );
    }

    #[test]
    fn insert_round_trip_ascii() {
        let s = CrdtState::new(1).expect("new");
        s.insert(0, "hello").expect("insert");
        assert_eq!(s.materialize_string(), "hello");
        s.insert(5, " world").expect("insert at end");
        assert_eq!(s.materialize_string(), "hello world");
        s.insert(5, ",").expect("insert in middle");
        assert_eq!(s.materialize_string(), "hello, world");
    }

    #[test]
    fn delete_removes_range() {
        let s = CrdtState::new(1).expect("new");
        s.insert(0, "hello, world").expect("seed");
        s.delete(5, 2).expect("delete ', '");
        assert_eq!(s.materialize_string(), "helloworld");
    }

    #[test]
    fn from_bytes_seeds_initial_content() {
        let s = CrdtState::from_bytes(42, b"the quick brown fox").expect("from_bytes");
        assert_eq!(s.materialize_string(), "the quick brown fox");
        assert_eq!(s.peer_id(), 42);
        // Subsequent edits compose with the seed.
        s.insert(19, " jumps").expect("append");
        assert_eq!(s.materialize_string(), "the quick brown fox jumps");
    }

    #[test]
    fn from_bytes_empty_yields_empty_state() {
        let s = CrdtState::from_bytes(7, b"").expect("from_bytes empty");
        assert_eq!(s.len_unicode(), 0);
        assert_eq!(s.materialize_string(), "");
    }

    #[test]
    fn snapshot_round_trips_through_export_import() {
        let source = CrdtState::new(1).expect("source");
        source.insert(0, "snapshot me").expect("seed");
        let bytes = source.export_snapshot().expect("export");
        // Independent state on a different peer imports the bytes
        // and observes the same content.
        let restored = CrdtState::new(2).expect("restored");
        restored.import_snapshot(&bytes).expect("import");
        assert_eq!(restored.materialize_string(), "snapshot me");
        // Restored state can continue editing under its own peer ID.
        restored.insert(11, "!").expect("post-restore edit");
        assert_eq!(restored.materialize_string(), "snapshot me!");
    }

    // -----------------------------------------------------------------
    // M10.2 Day 2 morning audit — Q1 sub-checks for byte-native loro.
    //
    // Findings recorded here so a future loro upgrade or wrapper change
    // doesn't silently shift the byte/unicode boundary semantics. Loro
    // 1.12 provides parallel utf8-suffixed methods (`insert_utf8`,
    // `delete_utf8`, `mark_utf8`) for byte-native operation rather
    // than a global `OffsetKind` config. The default `insert` / `delete`
    // are unicode-native; we use the `_utf8` variants exclusively in
    // the wrapper to match pmacs's byte-position rope API.
    //
    // Test string: "héllo" — h=0x68 (1B), é=0xC3 0xA9 (2B), l=0x6C
    // (1B), l=0x6C (1B), o=0x6F (1B). Total 6 bytes, 5 codepoints.

    #[test]
    fn q1_sub_check_1_byte_native_insertion() {
        // We don't (yet) expose `insert_utf8` on `CrdtState` — Day 2's
        // afternoon adds it. This test pins the underlying loro
        // behaviour we'll wrap.
        let state = CrdtState::new(1).expect("new");
        // Using the underlying loro doc directly to verify the byte-
        // native method exists and behaves as expected. The wrapper
        // method (`insert_bytes` / similar) will route to this.
        let text = state.doc.get_text("body");
        text.insert_utf8(0, "héllo").expect("seed");
        // Insert "X" at byte offset 3 (after "hé"). Byte-native:
        // result is "héXllo". Unicode-native: would be "hélXlo".
        text.insert_utf8(3, "X").expect("byte-native insert");
        let s = text.to_string();
        assert_eq!(
            s, "héXllo",
            "loro insert_utf8 must be byte-native (got {s:?})"
        );
    }

    #[test]
    fn q1_sub_check_2_byte_native_deletion() {
        let state = CrdtState::new(1).expect("new");
        let text = state.doc.get_text("body");
        text.insert_utf8(0, "héllo world").expect("seed");
        // Delete bytes [1, 3) — the é (2-byte codepoint). Byte-native:
        // "hllo world". Unicode-native (codepoints [1, 3) = "él"):
        // "hlo world". The two interpretations differ unambiguously.
        text.delete_utf8(1, 2).expect("byte-native delete");
        let s = text.to_string();
        assert_eq!(s, "hllo world", "loro delete_utf8 must be byte-native");
    }

    #[test]
    fn q1_sub_check_3_mid_codepoint_rejection_on_delete() {
        // Trying to delete a range that splits a multi-byte codepoint
        // must fail rather than silently corrupt. The é at bytes 1..3:
        // delete_utf8(2, 1) starts mid-codepoint and should error.
        let state = CrdtState::new(1).expect("new");
        let text = state.doc.get_text("body");
        text.insert_utf8(0, "héllo").expect("seed");
        let r = text.delete_utf8(2, 1);
        assert!(
            r.is_err(),
            "delete_utf8 with mid-codepoint range must reject (got {r:?})"
        );
        // The buffer should be unchanged after the rejected delete.
        assert_eq!(text.to_string(), "héllo");
    }

    #[test]
    fn q1_sub_check_3_mid_codepoint_on_insert() {
        // insert_utf8 at a mid-codepoint position. Behaviour pinned;
        // either reject or split-with-replacement. The wrapper layer
        // can normalize whichever it is to a uniform error shape, but
        // we need to know which one loro picks.
        let state = CrdtState::new(1).expect("new");
        let text = state.doc.get_text("body");
        text.insert_utf8(0, "héllo").expect("seed");
        let r = text.insert_utf8(2, "X");
        match r {
            Ok(()) => {
                let s = text.to_string();
                // Document the observed behaviour rather than asserting
                // a specific one — the wrapper layer adapts.
                eprintln!("loro insert_utf8 at mid-codepoint accepted: result = {s:?}");
                // Sanity: result must not be UB-shaped; should still
                // be valid UTF-8 and contain at least the inserted X.
                assert!(s.contains('X'));
            }
            Err(e) => {
                eprintln!("loro insert_utf8 at mid-codepoint rejected: {e}");
                assert_eq!(text.to_string(), "héllo");
            }
        }
    }

    #[test]
    fn q1_sub_check_cursor_position_default_kind() {
        // get_cursor takes a unicode position by default and
        // get_cursor_pos returns AbsolutePosition.pos in some kind.
        // We need to know which kind so the wrapper can route
        // correctly.
        use loro::cursor::Side;
        let state = CrdtState::new(1).expect("new");
        let text = state.doc.get_text("body");
        text.insert_utf8(0, "héllo").expect("seed");
        // Place a cursor with input pos=2 (unicode). Multi-byte case
        // makes the byte/unicode answer differ:
        //   unicode pos 2 = after "hé" (2 codepoints in)
        //   byte pos 2 = mid-é (between é's two bytes — illegal but
        //                       informative if loro uses it)
        let cursor = text.get_cursor(2, Side::Middle).expect("cursor");
        let pos = state.doc.get_cursor_pos(&cursor).expect("query");
        eprintln!(
            "get_cursor(2, Middle) on \"héllo\" -> current.pos = {} (unicode 2 = after 'hé', byte 2 = mid-é)",
            pos.current.pos
        );
        // Loro's default get_cursor / get_cursor_pos use unicode
        // positions per the docs; the wrapper layer will translate
        // when exposing cursor positions. Pin the expectation:
        assert_eq!(
            pos.current.pos, 2,
            "loro cursor positions are unicode by default (input pos round-trips)"
        );
    }

    #[test]
    fn from_bytes_handles_non_utf8_via_replacement() {
        // The byte 0xFF is not valid UTF-8. The editor's rope is
        // permissive at the byte level; the CRDT-backed mode loses
        // the ill-formed-byte detail (replaced with U+FFFD) per the
        // documented from_bytes caveat. Pin the behaviour.
        let s = CrdtState::from_bytes(1, b"ok\xFFok").expect("from_bytes");
        // The replacement-character round-trip preserves length-3
        // structure: "ok" + U+FFFD + "ok" — verify text.
        assert!(s.materialize_string().contains("ok"));
        assert!(s.materialize_string().contains('\u{FFFD}'));
    }

    // -----------------------------------------------------------------
    // T M10.2 Day 4 — multi-peer convergence smoke tests.
    //
    // Targeted tests for the canonical CRDT convergence claim:
    // concurrent ops from N peers, when merged in any order, produce
    // identical final states on every peer. The proptest version is
    // in `crdt::proptests` below; these smoke tests verify the
    // smallest interesting cases first so a proptest failure is
    // easier to debug.
    //
    // Per the Q2 design: `import_updates` is the per-edit-delta
    // path (Day 4 + M10.5); `import_snapshot` is the full-state
    // attach path. Both wrap loro's universal `doc.import`.
    // -----------------------------------------------------------------

    /// Helper: peer-to-peer sync. Each peer captures its updates since
    /// the empty version vector (which is "all my updates") and the
    /// other peer imports them. After this call, both peers should be
    /// converged.
    #[cfg(test)]
    fn sync_pair(a: &CrdtState, b: &CrdtState) {
        use loro::VersionVector;
        let zero = VersionVector::default();
        let a_updates = a.export_updates_since(&zero).expect("a export");
        let b_updates = b.export_updates_since(&zero).expect("b export");
        a.import_updates(&b_updates).expect("a import b");
        b.import_updates(&a_updates).expect("b import a");
    }

    #[test]
    fn two_peers_converge_with_concurrent_inserts_at_zero() {
        // Both peers start empty.
        // Peer A inserts "AAA" at position 0.
        // Peer B inserts "BBB" at position 0.
        // After bidirectional sync, both peers must hold the same
        // final string. Loro's tie-breaking yields one of "AAABBB"
        // or "BBBAAA" deterministically by peer ID; the test asserts
        // the peers agree, not a specific outcome.
        let a = CrdtState::new(1).expect("peer a");
        let b = CrdtState::new(2).expect("peer b");
        a.insert(0, "AAA").expect("a insert");
        b.insert(0, "BBB").expect("b insert");

        let pre_a = a.materialize_string();
        let pre_b = b.materialize_string();
        assert_eq!(pre_a, "AAA", "peer A sees only its own op pre-sync");
        assert_eq!(pre_b, "BBB", "peer B sees only its own op pre-sync");

        sync_pair(&a, &b);

        let post_a = a.materialize_string();
        let post_b = b.materialize_string();
        assert_eq!(
            post_a, post_b,
            "peers must converge after bidirectional sync"
        );
        // Sanity: both edits are represented in the converged state.
        assert!(post_a.contains("AAA"));
        assert!(post_a.contains("BBB"));
        assert_eq!(post_a.len(), 6, "no duplication, no loss");
    }

    #[test]
    fn two_peers_converge_independent_of_import_order() {
        use loro::VersionVector;
        // Apply identical ops to two peer pairs but in different
        // import orders; the converged states must match. This is
        // the smallest "different sync ordering, same final state"
        // smoke test — the canonical CRDT property.
        let make_pair = || {
            let p1 = CrdtState::new(1).unwrap();
            let p2 = CrdtState::new(2).unwrap();
            p1.insert(0, "hello").unwrap();
            p2.insert(0, "world").unwrap();
            (p1, p2)
        };

        // Ordering 1: peer 1 imports first, then peer 2.
        let (a1, a2) = make_pair();
        let zero = VersionVector::default();
        let a2_bytes = a2.export_updates_since(&zero).unwrap();
        a1.import_updates(&a2_bytes).unwrap();
        let a1_bytes = a1.export_updates_since(&zero).unwrap();
        a2.import_updates(&a1_bytes).unwrap();

        // Ordering 2: peer 2 imports first, then peer 1.
        let (b1, b2) = make_pair();
        let b1_bytes = b1.export_updates_since(&zero).unwrap();
        b2.import_updates(&b1_bytes).unwrap();
        let b2_bytes = b2.export_updates_since(&zero).unwrap();
        b1.import_updates(&b2_bytes).unwrap();

        // Both orderings produce the same final state across all peers.
        let final_state = a1.materialize_string();
        assert_eq!(a1.materialize_string(), final_state);
        assert_eq!(a2.materialize_string(), final_state);
        assert_eq!(b1.materialize_string(), final_state);
        assert_eq!(b2.materialize_string(), final_state);
    }

    // -----------------------------------------------------------------
    // T M10.2 Day 4 — convergence proptest.
    //
    // Generates N peers (2-4), each with a random sequence of
    // local ops, applied independently (each peer sees only its own
    // ops until sync). After full sync via one of five hand-picked
    // patterns, all peers must converge to the same projection
    // string.
    //
    // Five sync orderings:
    //   * Sequential — peer-0 → peer-1 → peer-2 → ... (each peer
    //     receives every prior peer's updates)
    //   * Star       — peer-0 receives all others' updates, then
    //     broadcasts the combined state
    //   * Pairwise   — peer-0 syncs with peer-1, then both sync
    //     with peer-2, etc.
    //   * DelayedJoin — first half of peers sync each other; second
    //     half joins later and pulls combined state
    //   * Reverse    — like Sequential but in reverse peer order
    //
    // Hand-picked patterns beat random permutations here: realistic
    // multi-frontend topologies have structural meaning that uniform
    // random permutation dilutes. Loro's convergence is proved
    // upstream; what's being tested is the wrapper's correctness
    // under realistic patterns.
    // -----------------------------------------------------------------
    mod proptests {
        use super::*;
        use loro::VersionVector;
        use proptest::prelude::*;

        const ALPHABET: &[&str] = &["a", "b", "c", " ", "\n"];

        #[derive(Clone, Debug)]
        enum PeerOp {
            Insert(usize, String),
            Delete(usize, usize),
            Replace(usize, usize, String),
        }

        #[derive(Clone, Debug)]
        enum SyncPattern {
            Sequential,
            Star,
            Pairwise,
            DelayedJoin,
            Reverse,
        }

        fn gen_payload() -> impl Strategy<Value = String> {
            prop::collection::vec(prop::sample::select(ALPHABET.to_vec()), 1..6)
                .prop_map(|parts| parts.concat())
        }

        fn gen_op() -> impl Strategy<Value = PeerOp> {
            prop_oneof![
                3 => (any::<u8>(), gen_payload()).prop_map(|(p, s)| PeerOp::Insert(p as usize, s)),
                2 => (any::<u8>(), any::<u8>()).prop_map(|(p, l)| PeerOp::Delete(p as usize, l as usize)),
                1 => (any::<u8>(), any::<u8>(), gen_payload())
                    .prop_map(|(p, l, s)| PeerOp::Replace(p as usize, l as usize, s)),
            ]
        }

        /// Apply an op to a peer, clamping positions to the peer's
        /// local state at op-generation time. The op's recorded
        /// position refers to that state; loro's CRDT handles
        /// translation when the op is applied on a peer with
        /// different local state. We're testing that this translation
        /// produces convergent results, not that loro rejects
        /// invalid positions — so the clamp ensures the generated
        /// ops are *valid for the local peer*; convergence handles
        /// the rest.
        fn apply_to_peer(peer: &CrdtState, op: &PeerOp) -> Result<(), loro::LoroError> {
            let len = peer.len_utf8();
            match op {
                PeerOp::Insert(pos, s) => {
                    let pos = (*pos).min(len);
                    peer.insert(pos, s)
                }
                PeerOp::Delete(pos, l) => {
                    let pos = (*pos).min(len);
                    let l = (*l).min(len.saturating_sub(pos));
                    if l == 0 {
                        return Ok(());
                    }
                    peer.delete(pos, l)
                }
                PeerOp::Replace(pos, l, s) => {
                    let pos = (*pos).min(len);
                    let l = (*l).min(len.saturating_sub(pos));
                    if l > 0 {
                        peer.delete(pos, l)?;
                    }
                    peer.insert(pos, s)
                }
            }
        }

        /// Run a sync pattern against a list of peers. After this
        /// call, every peer must hold the same converged state.
        ///
        /// Each pattern is implemented as a sequence of pairwise
        /// imports: each invocation transfers one peer's complete
        /// update set (since the empty version vector) to another
        /// peer. Loro's import handles concurrent merges via the
        /// underlying CRDT semantics; this driver just orchestrates
        /// who-imports-from-whom in the chosen topology.
        ///
        /// The explicit `for i in 0..n / for j in 0..n` index loops
        /// (vs `.iter().enumerate()`) make the topology semantics
        /// readable: "every peer imports from every other peer" is
        /// a 2D index pattern, not a transformation chain.
        #[allow(
            clippy::needless_range_loop,
            reason = "index loops express topology more clearly than iterator chains for the i != j cross-product pattern"
        )]
        fn run_sync(peers: &[CrdtState], pattern: &SyncPattern) {
            let zero = VersionVector::default();
            let n = peers.len();
            let exports: Vec<Vec<u8>> = peers
                .iter()
                .map(|p| p.export_updates_since(&zero).expect("export"))
                .collect();
            match pattern {
                SyncPattern::Sequential => {
                    // Each peer i imports from every prior peer 0..i,
                    // and forward-shares to peer i+1.
                    for i in 0..n {
                        for j in 0..n {
                            if i != j {
                                peers[i].import_updates(&exports[j]).expect("import");
                            }
                        }
                    }
                }
                SyncPattern::Reverse => {
                    // Same as sequential but processed in reverse.
                    for i in (0..n).rev() {
                        for j in (0..n).rev() {
                            if i != j {
                                peers[i].import_updates(&exports[j]).expect("import");
                            }
                        }
                    }
                }
                SyncPattern::Star => {
                    // peer 0 collects from all; then re-exports to all.
                    for j in 1..n {
                        peers[0].import_updates(&exports[j]).expect("import");
                    }
                    let combined = peers[0].export_updates_since(&zero).expect("re-export");
                    for i in 1..n {
                        peers[i].import_updates(&combined).expect("import");
                    }
                }
                SyncPattern::Pairwise => {
                    // Peer 0 syncs with 1, then (0,1) with 2, etc.
                    // Each step pulls the cumulative state to the
                    // joining peer.
                    for i in 1..n {
                        let combined = peers[i - 1]
                            .export_updates_since(&zero)
                            .expect("export combined");
                        peers[i].import_updates(&combined).expect("import");
                        // Reverse direction so the older peers also
                        // see the new one's ops.
                        let back = peers[i].export_updates_since(&zero).expect("back-export");
                        for j in 0..i {
                            peers[j].import_updates(&back).expect("back-import");
                        }
                    }
                }
                SyncPattern::DelayedJoin => {
                    // First half syncs each other; second half joins
                    // later and pulls combined state.
                    let half = n / 2;
                    if half >= 2 {
                        for i in 0..half {
                            for j in 0..half {
                                if i != j {
                                    peers[i].import_updates(&exports[j]).expect("first-half");
                                }
                            }
                        }
                    }
                    let combined = peers[0]
                        .export_updates_since(&zero)
                        .expect("first-half combined");
                    for i in half..n {
                        peers[i].import_updates(&combined).expect("delayed import");
                        // Late joiners also share with the first half.
                        let late = peers[i].export_updates_since(&zero).expect("late export");
                        for j in 0..half {
                            peers[j].import_updates(&late).expect("late-receive");
                        }
                    }
                    // Final round: everyone imports everyone (idempotent
                    // for already-merged ops).
                    let final_exports: Vec<Vec<u8>> = peers
                        .iter()
                        .map(|p| p.export_updates_since(&zero).expect("final-export"))
                        .collect();
                    for i in 0..n {
                        for j in 0..n {
                            if i != j {
                                peers[i].import_updates(&final_exports[j]).expect("final");
                            }
                        }
                    }
                }
            }
        }

        proptest! {
            // 32 cases for CI; bump locally (e.g. 256+) when validating
            // before declaring Day 4 done. 32 cases passing means "no
            // failure surfaced in this sample," not exhaustive proof.
            #![proptest_config(ProptestConfig::with_cases(32))]

            #[test]
            fn peers_converge_under_arbitrary_sync_order(
                peer_count in 2usize..=4,
                op_seqs in prop::collection::vec(
                    prop::collection::vec(gen_op(), 1..=15),
                    2..=4,
                ),
                sync_pattern in prop::sample::select(vec![
                    SyncPattern::Sequential,
                    SyncPattern::Reverse,
                    SyncPattern::Star,
                    SyncPattern::Pairwise,
                    SyncPattern::DelayedJoin,
                ]),
            ) {
                // The strategy may generate more op-seqs than peer_count;
                // truncate to peer_count so the sizes match.
                let op_seqs: Vec<_> = op_seqs.into_iter().take(peer_count).collect();
                // The strategy guarantees at least 2 op-seqs (vec size
                // bound), so peer_count<=op_seqs.len() may not hold if
                // peer_count is 4 but the inner vec generated only 2.
                // Re-derive the actual peer count from op_seqs.len().
                let actual_peer_count = op_seqs.len();

                // Create N peers with unique peer IDs (loro requires
                // NonZeroU64, so we use 1..=N).
                let peers: Vec<CrdtState> = (1..=actual_peer_count as u64)
                    .map(|id| CrdtState::new(id).expect("peer"))
                    .collect();

                // Each peer applies its own op sequence in isolation
                // (no cross-peer visibility yet). Some ops may be
                // benign no-ops after clamping; that's fine — the
                // convergence claim holds across any valid op set.
                for (peer, ops) in peers.iter().zip(op_seqs.iter()) {
                    for op in ops {
                        let _ = apply_to_peer(peer, op);
                    }
                }

                // Capture pre-sync states for diagnostic output on
                // failure. The shrinker output without these is hard
                // to interpret; with them, the wrapper bug (if any)
                // is visible.
                let pre_sync: Vec<String> = peers
                    .iter()
                    .map(CrdtState::materialize_string)
                    .collect();
                // Capture logical version vectors (not byte encodings)
                // — encoding may not canonicalize entry order, so
                // byte-comparison would false-positive divergence;
                // VersionVector has PartialEq that compares logical
                // content.
                let pre_versions: Vec<_> = peers.iter().map(CrdtState::version).collect();

                run_sync(&peers, &sync_pattern);

                let post_sync: Vec<String> = peers
                    .iter()
                    .map(CrdtState::materialize_string)
                    .collect();
                let post_versions: Vec<_> = peers.iter().map(CrdtState::version).collect();

                // Convergence: all peers' projections match peer 0's.
                for i in 1..actual_peer_count {
                    prop_assert_eq!(
                        &post_sync[0],
                        &post_sync[i],
                        "peers diverged after {:?} sync\n  \
                         peer 0 ops: {:?}\n  \
                         peer {} ops: {:?}\n  \
                         peer 0 pre-sync: {:?}\n  \
                         peer {} pre-sync: {:?}\n  \
                         peer 0 post-sync: {:?}\n  \
                         peer {} post-sync: {:?}\n  \
                         peer 0 pre-version: {:?}\n  \
                         peer {} pre-version: {:?}\n  \
                         peer 0 post-version: {:?}\n  \
                         peer {} post-version: {:?}",
                        sync_pattern,
                        op_seqs[0],
                        i, op_seqs[i],
                        pre_sync[0],
                        i, pre_sync[i],
                        post_sync[0],
                        i, post_sync[i],
                        pre_versions[0],
                        i, pre_versions[i],
                        post_versions[0],
                        i, post_versions[i]
                    );
                }

                // Also assert all peers' version vectors are identical
                // post-sync. This is stronger than projection-string
                // equality: if peers agree on the string but disagree
                // on the version vector, future remote ops might cause
                // them to diverge. Loro's CRDT contract guarantees
                // both; pin both.
                for i in 1..actual_peer_count {
                    prop_assert_eq!(
                        &post_versions[0],
                        &post_versions[i],
                        "peer version vectors diverged"
                    );
                }

                // Avoid the unused-variable warning when peer_count is
                // referenced only via op_seqs.len().
                let _ = peer_count;
            }
        }
    }

    // -----------------------------------------------------------------
    // T M10.4 — per-frontend undo acceptance tests.
    //
    // Five tests covering the spec's three criteria plus two extras
    // surfaced during the framing pass (concurrent-without-sync,
    // region-overlap):
    //
    //   1. (covered elsewhere) Single-frontend identical to v0.1
    //      — dual-mode buffer tests already verify this
    //   2. Concurrent inserts with intervening sync: A inserts / sync
    //      / B inserts / sync / A undoes / B's insert remains
    //   3. Concurrent inserts WITHOUT intervening sync: A inserts /
    //      B inserts (both unaware) / sync both ways / A undoes /
    //      B's insert remains
    //   4. B edits A's region: A inserts / sync / B edits within /
    //      sync / A undoes A's insert / verify loro's region-
    //      overlap behavior
    //   5. Redo symmetry: each pattern with redo applied, state
    //      returns to pre-undo
    // -----------------------------------------------------------------

    #[test]
    fn m10_4_concurrent_with_sync_a_undoes_b_remains() {
        use loro::VersionVector;
        // A inserts / sync / B inserts / sync / A undoes / verify
        // B's insert remains, A's insert is gone.
        let a = CrdtState::new(1).expect("A");
        let b = CrdtState::new(2).expect("B");

        // Round 1: A inserts "AA" at 0.
        a.insert(0, "AA").expect("A insert");
        // Sync A → B.
        let zero = VersionVector::default();
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(b.materialize_string(), "AA");

        // Round 2: B inserts "BB" at end (position 2).
        b.insert(b.len_utf8(), "BB").expect("B insert");
        // Sync B → A.
        a.import_updates(&b.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(a.materialize_string(), "AABB");
        assert_eq!(b.materialize_string(), "AABB");

        // A undoes its own insert. B's "BB" must remain.
        let undid = a.undo().expect("A undo");
        assert!(undid);
        let after_undo = a.materialize_string();
        assert_eq!(
            after_undo, "BB",
            "A's undo must remove only A's insert; B's must remain"
        );

        // Sync A's undo back to B; B's projection must match A's.
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(b.materialize_string(), "BB");
    }

    #[test]
    fn m10_4_concurrent_without_sync_a_undoes_b_remains() {
        use loro::VersionVector;
        // A inserts / B inserts (both unaware of each other) / sync
        // bidirectionally / A undoes / verify B's insert remains.
        let a = CrdtState::new(1).expect("A");
        let b = CrdtState::new(2).expect("B");

        // Both peers insert concurrently with no intervening sync.
        a.insert(0, "AAA").expect("A insert");
        b.insert(0, "BBB").expect("B insert");

        // Now sync bidirectionally.
        let zero = VersionVector::default();
        let a_bytes = a.export_updates_since(&zero).unwrap();
        let b_bytes = b.export_updates_since(&zero).unwrap();
        a.import_updates(&b_bytes).unwrap();
        b.import_updates(&a_bytes).unwrap();

        // Converged state contains both edits.
        assert_eq!(a.materialize_string(), b.materialize_string());
        let converged = a.materialize_string();
        assert!(converged.contains("AAA"));
        assert!(converged.contains("BBB"));
        assert_eq!(converged.len(), 6);

        // A undoes its insert. B's "BBB" must remain.
        a.undo().expect("A undo");
        let after_undo = a.materialize_string();
        assert_eq!(
            after_undo, "BBB",
            "A's undo removes A's insert; B's BBB remains regardless of sync order"
        );

        // Sync undo to B; convergence holds.
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(b.materialize_string(), "BBB");
    }

    #[test]
    fn m10_4_b_edits_a_region_then_a_undoes() {
        use loro::VersionVector;
        // A inserts "hello" / sync / B edits within (inserts "X" at
        // position 2, between "he" and "llo") / sync / A undoes /
        // verify loro's behavior for region-overlap undo.
        //
        // Loro's UndoManager produces an inverse op that removes the
        // bytes A originally inserted. B's "X" insert was at a
        // position WITHIN A's "hello" block; after A's undo, what
        // happens to B's "X"?
        //
        // CRDT semantics: B's "X" insert referenced A's "hello" block
        // structurally (insert-between-codepoints). When A's hello is
        // removed, B's X has no anchor — but it persists because
        // loro's tombstone preserves the insertion point.
        //
        // This test PINS whatever behavior loro produces; the spec
        // language "B's edit lands on whatever surrounding text
        // remains" is loro's intrinsic behavior, not ours to design.
        let a = CrdtState::new(1).expect("A");
        let b = CrdtState::new(2).expect("B");

        a.insert(0, "hello").expect("A insert");
        let zero = VersionVector::default();
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(b.materialize_string(), "hello");

        // B inserts "X" at position 2 ("he" + "X" + "llo" -> "heXllo").
        b.insert(2, "X").expect("B insert in middle");
        a.import_updates(&b.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(a.materialize_string(), "heXllo");
        assert_eq!(b.materialize_string(), "heXllo");

        // A undoes "hello". B's "X" remains; the surrounding "hello"
        // bytes attributed to A are removed.
        a.undo().expect("A undo");
        let after_undo = a.materialize_string();
        assert_eq!(
            after_undo, "X",
            "A's undo removes A's hello; B's X remains on surrounding text \
             (which is now empty)"
        );

        // Sync to B; convergence holds.
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(b.materialize_string(), "X");
    }

    #[test]
    fn m10_4_redo_symmetric_after_undo() {
        use loro::VersionVector;
        // Verify redo correctly reverses undo across the multi-peer
        // patterns. Same setup as test 2 (concurrent-with-sync); A
        // undoes / verifies / A redoes / verifies state returns to
        // pre-undo.
        let a = CrdtState::new(1).expect("A");
        let b = CrdtState::new(2).expect("B");
        a.insert(0, "AA").expect("A insert");
        let zero = VersionVector::default();
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        b.insert(b.len_utf8(), "BB").expect("B insert");
        a.import_updates(&b.export_updates_since(&zero).unwrap())
            .unwrap();
        let pre_undo = a.materialize_string();
        assert_eq!(pre_undo, "AABB");

        a.undo().expect("undo");
        assert_eq!(a.materialize_string(), "BB");

        a.redo().expect("redo");
        assert_eq!(
            a.materialize_string(),
            pre_undo,
            "redo must restore pre-undo state across multi-peer scenarios"
        );
    }

    #[test]
    fn m10_4_undo_is_local_only_across_peers() {
        // Verify that A.undo() doesn't affect B's local view.
        // (B's view only changes when A's inverse op is synced to B
        // via import_updates.) Pins the "local-only" undo semantics
        // loro's UndoManager promises.
        use loro::VersionVector;
        let a = CrdtState::new(1).expect("A");
        let b = CrdtState::new(2).expect("B");
        a.insert(0, "hello").expect("A insert");
        let zero = VersionVector::default();
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(b.materialize_string(), "hello");

        // A undoes. B's view is NOT updated until B explicitly imports.
        a.undo().expect("A undo");
        assert_eq!(a.materialize_string(), "");
        assert_eq!(
            b.materialize_string(),
            "hello",
            "B's view unchanged until B imports A's inverse op"
        );

        // Now B imports; convergence.
        b.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        assert_eq!(b.materialize_string(), "");
    }

    #[test]
    fn delayed_join_peer_converges_with_others() {
        use loro::VersionVector;
        // The cold-attach pattern: peers A and B exchange ops over
        // time, then peer C joins much later and syncs from both.
        // After C catches up, all three peers must converge.
        let a = CrdtState::new(1).expect("peer a");
        let b = CrdtState::new(2).expect("peer b");

        // Round 1: a and b exchange.
        a.insert(0, "round1-a ").unwrap();
        b.insert(0, "round1-b ").unwrap();
        sync_pair(&a, &b);

        // Round 2: more ops, more exchange.
        let a_len = a.len_utf8();
        let b_len = b.len_utf8();
        a.insert(a_len, "round2-a ").unwrap();
        b.insert(b_len, "round2-b ").unwrap();
        sync_pair(&a, &b);

        // Now peer C joins, importing from both a and b.
        let c = CrdtState::new(3).expect("peer c");
        let zero = VersionVector::default();
        c.import_updates(&a.export_updates_since(&zero).unwrap())
            .unwrap();
        c.import_updates(&b.export_updates_since(&zero).unwrap())
            .unwrap();

        // C's state matches a's and b's. All three converged.
        assert_eq!(c.materialize_string(), a.materialize_string());
        assert_eq!(c.materialize_string(), b.materialize_string());
    }
}
