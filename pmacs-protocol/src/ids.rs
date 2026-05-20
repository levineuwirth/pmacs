//! Identity and range types — moved from `pmacs::buffer`,
//! `pmacs::protocol`, and `pmacs::rope` in session 1 of the
//! `pmacs-gpu` arc. The originals re-export these names so internal
//! `pmacs` imports (`crate::buffer::BufferId`, `crate::rope::Position`,
//! etc.) keep working unchanged.

use std::sync::atomic::{AtomicU64, Ordering};

/// Opaque, per-process identifier for a buffer.
///
/// The internal representation is private (R22): callers cannot reach
/// for `.0`; construction goes through [`BufferId::next`].
///
/// T M10.5: `Serialize` / `Deserialize` derived so `BufferId` can be
/// the routing key on `InstanceMessage::CrdtOp` / `FrontendEvent::CrdtOp`.
/// The serialized form is the bare `u64` (transparent newtype).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize)]
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
    ///
    /// Was `pub(crate)` before the session-1 crate split; promoted to
    /// `pub` to remain reachable from `pmacs` after the move. Not
    /// stable API for external consumers — external callers should
    /// either round-trip via `serde` or accept that the constructor
    /// may change.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Opaque identifier for a frontend attached to an instance.
///
/// Every input event carries a `FrontendId`. v0.1 uses one ID per
/// instance ([`FrontendId::LOCAL`]); v0.3 generalizes to multi-frontend
/// (multi-window, multi-user) without a protocol break.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct FrontendId(pub u64);

impl FrontendId {
    /// The single frontend used in v0.1's local-attach mode.
    ///
    /// Future multi-frontend deployments allocate IDs from a counter
    /// starting after this value; the constant is reserved.
    pub const LOCAL: FrontendId = FrontendId(1);
}

/// Byte offset into a rope. Buffer-wide; cursor / selection / span
/// anchors all use this type. Type alias rather than newtype so
/// arithmetic on offsets (slice ranges, byte deltas) doesn't need
/// conversions.
pub type Position = u64;

/// Half-open byte range `[start, end)` into a buffer's rope, matching
/// the rope's own range convention.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ByteRange {
    /// Inclusive start byte offset.
    pub start: u64,
    /// Exclusive end byte offset.
    pub end: u64,
}
