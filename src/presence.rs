//! T M10.6 — Per-tick presence broadcast.
//!
//! # Contract
//!
//! `SessionRegistry` is the daemon-side bookkeeping for the
//! `InstanceMessage::PresenceUpdate` flow:
//!
//! - Per-session **negotiated protocol version** so the per-tick
//!   sweep can filter v0.1 recipients (presence didn't exist on the
//!   v0.1 wire — the variant lives in the v1.0+ enum and v1 sessions
//!   must never receive it).
//! - Per-source **last-broadcast snapshot** so the sweep can
//!   short-circuit when nothing changed since the last tick. This is
//!   the coalescing implementation: rapid cursor movement between
//!   sweeps produces one broadcast carrying the *final* state, not
//!   N broadcasts carrying intermediate values.
//!
//! # Equality discipline
//!
//! `PresenceSnapshot`'s `PartialEq` is exactly the wire-representation
//! equality: two snapshots compare equal iff the
//! `InstanceMessage::PresenceUpdate`s built from them would serialize
//! to identical bytes. The flat shape ([`Position`] = u64;
//! `Option<SelectionSnapshot>` is a flat pair of u64s) makes this
//! property structural — no internal state can affect equality
//! without also changing the wire bytes.
//!
//! If a future field is added to `PresenceSnapshot` or
//! `SelectionSnapshot` that does NOT affect the wire (e.g., a
//! daemon-internal annotation), the derive(PartialEq) needs
//! revisiting — otherwise the sweep emits spurious broadcasts on
//! changes the wire would not encode.
//!
//! # M10.6 single-frontend behavior
//!
//! The daemon is single-frontend in M10.6 — only one session is
//! registered at a time, and the sweep's sender-exclusion + v2-
//! recipient filter produces an empty broadcast list. The call site
//! exists; the data flow is wired; the recipient list is structurally
//! empty until M10.8 enables multi-attach.
//!
//! # M10.7 / M10.8 forward-pointers
//!
//! - M10.7 tightens recipient filtering to also require
//!   `crdt_replica`/`multi_frontend` capability bits, not just v2
//!   protocol. The current sweep takes only the negotiated version;
//!   M10.7 will extend the session-state type to carry capabilities
//!   and the sweep will consult both.
//! - M10.8 wires the multi-frontend session dispatcher. The tracker's
//!   `register_session` / `unregister_session` API will be called per
//!   attach/detach; the sweep's broadcast list becomes non-empty.

use std::collections::HashMap;

use crate::buffer::BufferId;
use crate::protocol::{FrontendId, InstanceMessage, NegotiatedCapabilities, SelectionSnapshot};
use crate::rope::Position;

/// T M10.7 — per-session daemon-internal state.
///
/// One entry per attached session, keyed by `FrontendId` in the
/// tracker's `sessions` map. M10.7 ships with two fields; future
/// milestones append fields with sensible defaults — e.g., M10.8 may
/// add per-session view state, M11 may add per-session keymap
/// overlays. Append-only growth keeps existing call sites valid.
///
/// **Module-location note**: `SessionState` lives in `presence.rs`
/// today because that's where M10.6 introduced the per-session
/// tracking. M10.8 will likely want this type accessible from a
/// session-routing module as well; relocating to a neutral location
/// (e.g., `src/session.rs`) is M10.8's call. M10.7 leaves it here
/// with this note.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SessionState {
    /// The protocol version negotiated during the handshake. Always
    /// a member of `SUPPORTED_PROTOCOL_VERSIONS` (the daemon checks
    /// before constructing this). v0.1 frontends produce `1`; v1.0+
    /// frontends produce `2`.
    pub negotiated_protocol_version: u32,
    /// The capability bits negotiated during the handshake. v0.1
    /// frontends always end up with all bits `false` (their wire
    /// format does not carry capability fields; `#[serde(default)]`
    /// produces `false` on the daemon side).
    pub negotiated_capabilities: NegotiatedCapabilities,
    /// T M10.9 — color palette slot for this session's overlay
    /// rendering. Daemon assigns at attach time based on the
    /// connecting peer's Unix uid (`SO_PEERCRED`); same uid across
    /// reconnect → same slot. Slot resolves to a `Color` via
    /// [`crate::overlay_color::color_for_slot`].
    pub color_slot: u8,
}

impl SessionState {
    /// Convenience constructor used by tests and the daemon's
    /// handshake path. Takes the version, capabilities, and color
    /// slot.
    #[must_use]
    pub fn new(
        negotiated_protocol_version: u32,
        negotiated_capabilities: NegotiatedCapabilities,
        color_slot: u8,
    ) -> Self {
        Self {
            negotiated_protocol_version,
            negotiated_capabilities,
            color_slot,
        }
    }
}

/// One source frontend's presence at a tick boundary.
///
/// Equality is wire-equality — two snapshots compare equal iff they
/// would serialize to identical [`InstanceMessage::PresenceUpdate`]
/// bytes. See module docs for the discipline this implies.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PresenceSnapshot {
    /// Buffer the source frontend's cursor is in.
    pub buffer_id: BufferId,
    /// Byte offset of the source frontend's cursor within `buffer_id`.
    pub cursor: Position,
    /// Active selection range, if any.
    pub selection: Option<SelectionSnapshot>,
}

/// One outbound presence broadcast: which session receives which message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BroadcastEntry {
    /// The session receiving the message. Sender exclusion is the
    /// tracker's responsibility — the recipient is never the source.
    pub recipient: FrontendId,
    /// The wire message to send. Always
    /// [`InstanceMessage::PresenceUpdate`] in M10.6.
    pub message: InstanceMessage,
}

/// Per-tick presence diff + broadcast routing.
///
/// One instance lives on the daemon's per-attach path. M10.6's
/// single-frontend deployment means at most one session is registered
/// at a time; M10.8 generalizes to multiple sessions.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    /// Per-source last-broadcast snapshot. A source is present here
    /// iff at least one sweep has emitted (or considered emitting) a
    /// broadcast for it. `None` (absent) means "no prior state" —
    /// the first sweep observes a change.
    last_broadcast: HashMap<FrontendId, PresenceSnapshot>,
    /// Per-session daemon-internal state — negotiated protocol
    /// version + negotiated capability bits. M10.7 widened this from
    /// a bare `u32` version to the richer `SessionState` once
    /// capability negotiation became load-bearing. Recipients are
    /// filtered on `negotiated_capabilities.multi_frontend` (M10.7);
    /// v0.1 sessions naturally fail the filter because their
    /// declared bit defaults to `false` and the AND with any
    /// instance bit is `false`.
    sessions: HashMap<FrontendId, SessionState>,
}

impl SessionRegistry {
    /// Fresh tracker with no sessions and no prior broadcasts.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a session with its negotiated state (T M10.7).
    ///
    /// Called on attach after both the version-check predicate
    /// (`is_supported_protocol_version`) and the capability
    /// negotiation (`negotiate_capabilities`) have accepted the
    /// request. The `SessionState` carries the negotiated version
    /// AND the negotiated capability bits — M10.7 widened this from
    /// the M10.6 bare-`u32` shape.
    pub fn register_session(&mut self, frontend_id: FrontendId, state: SessionState) {
        self.sessions.insert(frontend_id, state);
    }

    /// Unregister a session on detach. Drops both the session entry
    /// and any last-broadcast state for that frontend so a future
    /// re-attach starts with no prior state.
    pub fn unregister_session(&mut self, frontend_id: FrontendId) {
        self.sessions.remove(&frontend_id);
        self.last_broadcast.remove(&frontend_id);
    }

    /// The negotiated state for `frontend_id`, or `None` if no
    /// session is registered for it.
    #[must_use]
    pub fn session_state(&self, frontend_id: FrontendId) -> Option<SessionState> {
        self.sessions.get(&frontend_id).copied()
    }

    /// Number of registered sessions. Useful for tests + diagnostics.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// T M10.9 — gather `OtherPresence` entries for the overlay
    /// renderer to paint into `recipient`'s grid.
    ///
    /// Returns `(source, snapshot, color_slot)` triples for every
    /// registered session whose:
    /// - `source != recipient` (sender exclusion)
    /// - `negotiated_capabilities.multi_frontend` is true (same
    ///   filter as `sweep`; v0.1 sessions don't broadcast presence)
    /// - has a `last_broadcast` entry (i.e., has produced a snapshot
    ///   the sweep observed)
    ///
    /// The recipient itself doesn't need a `multi_frontend`
    /// capability check — the caller only invokes this for
    /// recipients that will RECEIVE overlays, which by definition
    /// means they're multi-capable.
    #[must_use]
    pub fn other_presences_for(
        &self,
        recipient: FrontendId,
    ) -> Vec<crate::overlay_paint::OtherPresence> {
        let mut out = Vec::new();
        for (&source, state) in &self.sessions {
            if source == recipient {
                continue;
            }
            if !state.negotiated_capabilities.multi_frontend {
                continue;
            }
            if let Some(&snapshot) = self.last_broadcast.get(&source) {
                out.push(crate::overlay_paint::OtherPresence {
                    frontend_id: source,
                    snapshot,
                    color_slot: state.color_slot,
                });
            }
        }
        out
    }

    /// Sweep: given per-source current snapshots, return the
    /// broadcasts to send this tick.
    ///
    /// For each `(source, snapshot)`:
    /// 1. If `snapshot == last_broadcast[source]`, no change → no
    ///    broadcast for this source.
    /// 2. Otherwise update `last_broadcast[source]` and emit one
    ///    [`BroadcastEntry`] per recipient where recipient is
    ///    registered, recipient != source (sender exclusion), and
    ///    the recipient's negotiated version is `>= 2` (v0.1 filter).
    ///
    /// Coalescing is structural: the sweep is called once per tick;
    /// multiple cursor movements between sweeps are observed as one
    /// snapshot (the final state). The N-moves-coalesce-to-1
    /// property follows from the sweep cadence, not from any
    /// timestamp / counter inside the snapshot.
    ///
    /// In M10.6 single-frontend deployments: the recipient list is
    /// structurally empty (sender exclusion with no other sessions),
    /// so the returned vec is always empty even if the snapshot
    /// changed. The construct-then-fan-out shape is preserved so
    /// M10.8 doesn't restructure the code; only the recipient set
    /// grows.
    pub fn sweep(&mut self, current: &[(FrontendId, PresenceSnapshot)]) -> Vec<BroadcastEntry> {
        let mut out = Vec::new();
        for (source, snapshot) in current {
            let changed = self
                .last_broadcast
                .get(source)
                .is_none_or(|prev| prev != snapshot);
            if !changed {
                continue;
            }
            self.last_broadcast.insert(*source, *snapshot);
            // Build the wire message once per source; the recipient
            // list may be empty (M10.6 single-frontend), in which
            // case the message is constructed but never serialized.
            // M10.8 enables non-empty recipient lists.
            let message = InstanceMessage::PresenceUpdate {
                frontend_id: *source,
                buffer_id: snapshot.buffer_id,
                cursor: snapshot.cursor,
                selection: snapshot.selection,
            };
            for (&recipient, state) in &self.sessions {
                if recipient == *source {
                    continue;
                }
                // T M10.7: filter on the negotiated capability bit.
                // M10.6 filtered on `version >= 2`; M10.7 tightens to
                // `multi_frontend = true`. v0.1 sessions naturally
                // fail the filter (their declared bit defaults to
                // false → AND with instance is false). v1.0 sessions
                // that declined `multi_frontend` during negotiation
                // also fail — they opted into single-frontend mode.
                if !state.negotiated_capabilities.multi_frontend {
                    continue;
                }
                out.push(BroadcastEntry {
                    recipient,
                    message: message.clone(),
                });
            }
        }
        out
    }

    /// T M10.8 Day 4 — broadcast one `InstanceMessage::CrdtOp` to
    /// every registered session that negotiated `crdt_replica: true`,
    /// excluding the source.
    ///
    /// Cadence differs from [`Self::sweep`]: presence sweeps run
    /// once per tick; CRDT-op broadcasts run once per edit event
    /// that produced a `Edit::crdt_op` payload. Keeping them as
    /// separate methods reflects the cadence difference and avoids
    /// awkward "one of these arguments is the presence, the other
    /// is the CRDT op" signature overloads.
    ///
    /// Sender exclusion: when `exclude` is `Some(fid)`, that frontend
    /// is filtered out (it already applied the op via its local
    /// mirror — see M10.10 post-audit-round-3 F16 / `CrdtOpOrigin`).
    /// When `exclude` is `None`, every `crdt_replica`-capable
    /// recipient receives the op including any frontend that may
    /// have driven the daemon's mutation via `FrontendEvent::Key`
    /// (whose mirror is otherwise stale). Recipient filter:
    /// `negotiated_capabilities.crdt_replica == true`.
    pub fn broadcast_crdt_op(
        &self,
        exclude: Option<FrontendId>,
        buffer_id: BufferId,
        op: crate::rope::CrdtOp,
    ) -> Vec<BroadcastEntry> {
        let mut out = Vec::new();
        let message = InstanceMessage::CrdtOp { buffer_id, op };
        for (&recipient, state) in &self.sessions {
            if Some(recipient) == exclude {
                continue;
            }
            if !state.negotiated_capabilities.crdt_replica {
                continue;
            }
            out.push(BroadcastEntry {
                recipient,
                message: message.clone(),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(buffer_id: BufferId, cursor: Position) -> PresenceSnapshot {
        PresenceSnapshot {
            buffer_id,
            cursor,
            selection: None,
        }
    }

    /// Session state for a v2 frontend that negotiated multi-frontend
    /// capability — the "presence-eligible recipient" case.
    /// Color slot 0 is the test default (palette[0]).
    fn multi_session() -> SessionState {
        SessionState::new(
            2,
            NegotiatedCapabilities {
                multi_frontend: true,
                crdt_replica: false,
            },
            0,
        )
    }

    /// Session state for a v0.1 frontend — `multi_frontend` defaults
    /// to false (the v0.1 wire format doesn't carry the field;
    /// `#[serde(default)]` produces false on the daemon side). This
    /// is the "presence filtered out" recipient case.
    fn legacy_session() -> SessionState {
        SessionState::new(
            1,
            NegotiatedCapabilities {
                multi_frontend: false,
                crdt_replica: false,
            },
            0,
        )
    }

    #[test]
    fn new_is_empty() {
        let t = SessionRegistry::new();
        assert_eq!(t.session_count(), 0);
        assert_eq!(t.session_state(FrontendId(2)), None);
    }

    #[test]
    fn register_and_unregister_session() {
        let mut t = SessionRegistry::new();
        t.register_session(FrontendId(2), multi_session());
        assert_eq!(t.session_count(), 1);
        assert_eq!(
            t.session_state(FrontendId(2))
                .map(|s| s.negotiated_protocol_version),
            Some(2)
        );
        t.unregister_session(FrontendId(2));
        assert_eq!(t.session_count(), 0);
        assert_eq!(t.session_state(FrontendId(2)), None);
    }

    #[test]
    fn sweep_excludes_sender_in_single_frontend() {
        // T M10.6 acceptance — sender exclusion. A multi-frontend
        // session sees its own cursor move; sweep produces no
        // broadcast because the only recipient candidate is the
        // sender itself.
        let mut t = SessionRegistry::new();
        let fid = FrontendId(2);
        t.register_session(fid, multi_session());
        let buf = BufferId::next();
        let out = t.sweep(&[(fid, snap(buf, 10))]);
        assert!(
            out.is_empty(),
            "sender excluded — single-frontend sweep should produce no broadcast, got {out:?}"
        );
    }

    #[test]
    fn sweep_excludes_recipient_without_multi_frontend_capability() {
        // T M10.7 (was M10.6 v1-filter test) — recipients without
        // negotiated multi_frontend are filtered out. v0.1 sessions
        // naturally fail because their declared bit defaults to
        // false; v1.0 sessions that declined multi_frontend during
        // negotiation also fail (they opted into single-frontend
        // mode).
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let legacy_recipient = FrontendId(3);
        t.register_session(src, multi_session());
        t.register_session(legacy_recipient, legacy_session());
        let buf = BufferId::next();
        let out = t.sweep(&[(src, snap(buf, 10))]);
        assert!(
            out.is_empty(),
            "recipient without multi_frontend filtered out — got {out:?}"
        );
    }

    #[test]
    fn sweep_broadcasts_to_multi_frontend_recipient() {
        // T M10.6/7 acceptance — multi-frontend source + multi-
        // frontend recipient: one message. This is the M10.8 case;
        // the tracker handles it correctly even though M10.6/7's
        // daemon doesn't admit multiple sessions.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient = FrontendId(3);
        t.register_session(src, multi_session());
        t.register_session(recipient, multi_session());
        let buf = BufferId::next();
        let out = t.sweep(&[(src, snap(buf, 10))]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].recipient, recipient);
        match &out[0].message {
            InstanceMessage::PresenceUpdate {
                frontend_id,
                cursor,
                ..
            } => {
                assert_eq!(*frontend_id, src);
                assert_eq!(*cursor, 10);
            }
            other => panic!("expected PresenceUpdate, got {other:?}"),
        }
    }

    #[test]
    fn sweep_suppresses_when_snapshot_unchanged() {
        // T M10.6 acceptance — diff suppression. Second sweep with
        // identical snapshot produces no broadcast even though
        // recipients exist.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient = FrontendId(3);
        t.register_session(src, multi_session());
        t.register_session(recipient, multi_session());
        let buf = BufferId::next();
        let s = snap(buf, 10);
        let out1 = t.sweep(&[(src, s)]);
        assert_eq!(out1.len(), 1, "first sweep broadcasts");
        let out2 = t.sweep(&[(src, s)]);
        assert!(
            out2.is_empty(),
            "second sweep with unchanged snapshot is suppressed, got {out2:?}"
        );
    }

    #[test]
    fn sweep_coalesces_intermediate_moves_to_final_state() {
        // T M10.6 acceptance — coalescing. The sweep is called once
        // per tick, observing the snapshot at the moment of the
        // sweep. Multiple cursor moves between sweeps appear to the
        // tracker as one snapshot change (from prev to final). The
        // coalescing-to-1 property is structural: the daemon calls
        // sweep once per tick, regardless of how many cursor moves
        // happened during the tick.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient = FrontendId(3);
        t.register_session(src, multi_session());
        t.register_session(recipient, multi_session());
        let buf = BufferId::next();

        // Tick 1: snapshot at cursor=10. First sweep broadcasts.
        let out1 = t.sweep(&[(src, snap(buf, 10))]);
        assert_eq!(out1.len(), 1);

        // Between ticks 1 and 2: cursor moves 10 → 20 → 30 → 99
        // (intermediate moves happen, but no sweep). Tick 2 observes
        // the final snapshot (cursor=99) only.
        let out2 = t.sweep(&[(src, snap(buf, 99))]);
        assert_eq!(out2.len(), 1, "tick 2 broadcasts once");
        match &out2[0].message {
            InstanceMessage::PresenceUpdate { cursor, .. } => {
                assert_eq!(
                    *cursor, 99,
                    "broadcast carries final snapshot value (99), not any intermediate (20, 30, …)"
                );
            }
            other => panic!("expected PresenceUpdate, got {other:?}"),
        }
    }

    #[test]
    fn sweep_first_tick_is_change() {
        // T M10.6 — the first tick after a session registers has no
        // prior state in last_broadcast. The diff treats absent-prior
        // as "changed" so the initial cursor position is broadcast
        // to any registered multi-frontend recipients.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient = FrontendId(3);
        t.register_session(src, multi_session());
        t.register_session(recipient, multi_session());
        let buf = BufferId::next();
        let out = t.sweep(&[(src, snap(buf, 0))]);
        assert_eq!(out.len(), 1, "first sweep broadcasts initial state");
    }

    #[test]
    fn unregister_clears_last_broadcast() {
        // T M10.6 — re-attaching after unregister starts fresh. The
        // last_broadcast entry is dropped on unregister so the next
        // sweep after re-register sees absent-prior and broadcasts.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient = FrontendId(3);
        t.register_session(src, multi_session());
        t.register_session(recipient, multi_session());
        let buf = BufferId::next();

        // First attach: broadcast initial state.
        let out1 = t.sweep(&[(src, snap(buf, 10))]);
        assert_eq!(out1.len(), 1);

        // Detach + re-attach: state cleared.
        t.unregister_session(src);
        t.register_session(src, multi_session());

        // First sweep after re-attach: still cursor=10, but the
        // last_broadcast was cleared on unregister, so the diff says
        // "changed" and we broadcast.
        let out2 = t.sweep(&[(src, snap(buf, 10))]);
        assert_eq!(
            out2.len(),
            1,
            "re-attach with same cursor still broadcasts (last_broadcast was cleared)"
        );
    }

    #[test]
    fn sweep_handles_selection_diff() {
        // T M10.6 — selection change alone (cursor unchanged) is a
        // diff and triggers broadcast. Equality is wire-equality, so
        // selection: None vs Some(anchor=cursor=10) compare unequal.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient = FrontendId(3);
        t.register_session(src, multi_session());
        t.register_session(recipient, multi_session());
        let buf = BufferId::next();

        let without_sel = PresenceSnapshot {
            buffer_id: buf,
            cursor: 10,
            selection: None,
        };
        let with_sel = PresenceSnapshot {
            buffer_id: buf,
            cursor: 10,
            selection: Some(SelectionSnapshot {
                anchor: 5,
                active: 10,
            }),
        };

        let _ = t.sweep(&[(src, without_sel)]);
        let out = t.sweep(&[(src, with_sel)]);
        assert_eq!(out.len(), 1, "selection-only change broadcasts");
    }

    #[test]
    fn sweep_multiple_sources_produce_independent_broadcasts() {
        // T M10.6 — when M10.8 enables multiple multi-frontend
        // sources, each source's snapshot diff is independent.
        // Per-tick sweep emits one broadcast per (source, recipient)
        // pair where the source's snapshot changed.
        let mut t = SessionRegistry::new();
        let a = FrontendId(2);
        let b = FrontendId(3);
        let c = FrontendId(4);
        t.register_session(a, multi_session());
        t.register_session(b, multi_session());
        t.register_session(c, multi_session());
        let buf = BufferId::next();

        // Tick 1: A at 10, B at 20. Both change (no prior). C is a
        // recipient of both A and B (and a sender to A and B, but
        // C didn't move so no broadcast originates from C).
        let out = t.sweep(&[(a, snap(buf, 10)), (b, snap(buf, 20))]);
        // From A: broadcasts to B and C (2 entries).
        // From B: broadcasts to A and C (2 entries).
        // Total: 4 broadcasts.
        assert_eq!(
            out.len(),
            4,
            "two sources × two recipients each = 4 entries"
        );

        // Tick 2: A unchanged, B moved to 25.
        let out2 = t.sweep(&[(a, snap(buf, 10)), (b, snap(buf, 25))]);
        // From A: unchanged, no broadcast.
        // From B: changed, broadcasts to A and C.
        assert_eq!(out2.len(), 2, "only B's change produces broadcasts");
    }

    #[test]
    fn sweep_with_no_sources_is_noop() {
        // Defensive — empty current list is the trivial case (no
        // attached frontends moved this tick). The sweep returns
        // empty without touching last_broadcast.
        let mut t = SessionRegistry::new();
        t.register_session(FrontendId(2), multi_session());
        let out = t.sweep(&[]);
        assert!(out.is_empty());
    }

    // T M10.8 Day 4 — broadcast_crdt_op filter matrix.

    /// Session state for a v2 frontend that negotiated `crdt_replica` capability.
    fn crdt_session() -> SessionState {
        SessionState::new(
            2,
            NegotiatedCapabilities {
                multi_frontend: true,
                crdt_replica: true,
            },
            0,
        )
    }

    /// Session state for a v2 frontend that opted out of `crdt_replica`.
    fn no_crdt_session() -> SessionState {
        SessionState::new(
            2,
            NegotiatedCapabilities {
                multi_frontend: true,
                crdt_replica: false,
            },
            0,
        )
    }

    fn dummy_crdt_op() -> crate::rope::CrdtOp {
        crate::rope::CrdtOp {
            peer_id: 7,
            bytes: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }
    }

    #[test]
    fn broadcast_crdt_op_excludes_sender() {
        // M10.8 acceptance criterion 1 (sender exclusion): a source
        // doesn't receive its own CRDT op back.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        t.register_session(src, crdt_session());
        let out = t.broadcast_crdt_op(Some(src), BufferId::next(), dummy_crdt_op());
        assert!(
            out.is_empty(),
            "sender excluded; single-session broadcast empty: {out:?}"
        );
    }

    #[test]
    fn broadcast_crdt_op_routes_to_crdt_capable_recipient() {
        // M10.8 acceptance criterion 3 (capability filter): a
        // recipient that negotiated crdt_replica receives.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient = FrontendId(3);
        t.register_session(src, crdt_session());
        t.register_session(recipient, crdt_session());
        let buf = BufferId::next();
        let out = t.broadcast_crdt_op(Some(src), buf, dummy_crdt_op());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].recipient, recipient);
        match &out[0].message {
            InstanceMessage::CrdtOp {
                buffer_id,
                op: crate::rope::CrdtOp { peer_id, bytes },
            } => {
                assert_eq!(*buffer_id, buf);
                assert_eq!(*peer_id, 7);
                assert_eq!(bytes, &vec![0xDE, 0xAD, 0xBE, 0xEF]);
            }
            other => panic!("expected CrdtOp, got {other:?}"),
        }
    }

    #[test]
    fn broadcast_crdt_op_filters_recipient_without_crdt_replica() {
        // M10.8 acceptance criterion 3: recipient with
        // `crdt_replica: false` doesn't receive.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let recipient_no_crdt = FrontendId(3);
        t.register_session(src, crdt_session());
        t.register_session(recipient_no_crdt, no_crdt_session());
        let out = t.broadcast_crdt_op(Some(src), BufferId::next(), dummy_crdt_op());
        assert!(
            out.is_empty(),
            "recipient with crdt_replica=false filtered out: {out:?}"
        );
    }

    #[test]
    fn broadcast_crdt_op_filters_legacy_recipient() {
        // v0.1 sessions have `crdt_replica: false` by default — the
        // M10.5 wire format doesn't carry the field and
        // `#[serde(default)]` produces false. They're naturally
        // filtered out.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let legacy_recipient = FrontendId(3);
        t.register_session(src, crdt_session());
        t.register_session(legacy_recipient, legacy_session());
        let out = t.broadcast_crdt_op(Some(src), BufferId::next(), dummy_crdt_op());
        assert!(out.is_empty(), "legacy recipient filtered out: {out:?}");
    }

    #[test]
    fn broadcast_crdt_op_routes_to_multiple_recipients() {
        // M10.8 multi-attach case: one source, two crdt-capable
        // recipients → 2 broadcasts.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        let r1 = FrontendId(3);
        let r2 = FrontendId(4);
        t.register_session(src, crdt_session());
        t.register_session(r1, crdt_session());
        t.register_session(r2, crdt_session());
        let out = t.broadcast_crdt_op(Some(src), BufferId::next(), dummy_crdt_op());
        assert_eq!(out.len(), 2);
        let recipients: std::collections::HashSet<_> = out.iter().map(|e| e.recipient).collect();
        assert!(recipients.contains(&r1));
        assert!(recipients.contains(&r2));
    }

    #[test]
    fn broadcast_crdt_op_no_recipients_is_noop() {
        // Source is the only session; broadcast empty.
        let mut t = SessionRegistry::new();
        let src = FrontendId(2);
        t.register_session(src, crdt_session());
        let out = t.broadcast_crdt_op(Some(src), BufferId::next(), dummy_crdt_op());
        assert!(out.is_empty());
    }

    /// M10.10 post-audit-round-3 F16: `exclude = None` broadcasts
    /// to **all** crdt-capable replicas including the frontend
    /// whose `Key` event drove the edit (its mirror is stale
    /// otherwise).
    #[test]
    fn broadcast_crdt_op_none_exclude_reaches_all_replicas() {
        let mut t = SessionRegistry::new();
        let a = FrontendId(2);
        let b = FrontendId(3);
        t.register_session(a, crdt_session());
        t.register_session(b, crdt_session());
        let out = t.broadcast_crdt_op(None, BufferId::next(), dummy_crdt_op());
        let recipients: std::collections::HashSet<_> = out.iter().map(|e| e.recipient).collect();
        assert!(
            recipients.contains(&a) && recipients.contains(&b),
            "F16: None-exclude must include the active frontend whose mirror is stale"
        );
        assert_eq!(out.len(), 2);
    }
}
