//! `CrdtOp` — moved from `pmacs::rope` in session 1 of the `pmacs-gpu`
//! arc. Carried on the `InstanceMessage::CrdtOp` /
//! `FrontendEvent::CrdtOp` wire variants when an attached session
//! negotiated `crdt_replica: true`.
//!
//! The type is unconditional (not `#[cfg]`-gated) — the comment on
//! the original `pmacs::rope::CrdtOp` explained why: "Always present
//! (not `#[cfg]`-gated) to avoid feature-flag proliferation through
//! every Edit consumer." Keeping the same shape here. The `crdt`
//! feature on the parent `pmacs` crate gates loro and the actual
//! application of CRDT ops; the wire-type definition stays compiled
//! unconditionally so consumers (`pmacs-gpu`, debug tools) don't have
//! to mirror the feature flag to handle a wire-level variant they
//! may never see.

/// T M10.2 Day 3: CRDT-op metadata carried by `Edit` in CRDT mode.
///
/// Two fields:
///
/// * `peer_id` — the producing-frontend identity. M10.4's per-frontend
///   undo reads this as the "is this op mine?" filter; saves the
///   consumer from parsing the op bytes to extract identity.
/// * `bytes` — wire-format serialization of the CRDT ops produced by
///   the originating edit, as returned by loro's
///   `ExportMode::updates_owned(pre_version)`. M10.5+ sends these
///   over the wire; receiving frontends import them via loro's
///   `import` to apply on their local CRDT.
///
/// Constructed by `Buffer::apply_edit` (and `undo` / `redo`) in CRDT
/// mode; rope's edit constructors set `Edit::crdt_op` to `None` and
/// the Buffer wraps after the rope returns.
///
/// T M10.5: serde derives added so this type can be the payload of
/// `InstanceMessage::CrdtOp` and `FrontendEvent::CrdtOp` on the wire.
/// `bytes` is opaque to the protocol layer — it's loro's incremental-
/// update format; the receiving end's `CrdtState::import_updates`
/// decodes it.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CrdtOp {
    /// Producing frontend's identity (loro `PeerID`).
    pub peer_id: u64,
    /// Wire-format op bytes (loro `ExportMode::updates_owned` output).
    pub bytes: Vec<u8>,
}
