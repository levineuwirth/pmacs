//! T M10.9 — color + label palette for other-frontend cursor overlays.
//!
//! # Contract
//!
//! Each attached frontend gets one slot from a fixed palette. The
//! palette has [`PALETTE_LEN`] entries, mapping slot index → distinct
//! terminal color. Slots are assigned per Unix uid in the daemon
//! (`crate::daemon::DaemonState::color_slot_for_uid`); same uid
//! across reconnect → same slot (the M10.9 spec's "stable across
//! reconnect within a session" criterion).
//!
//! # Wire-format note
//!
//! The slot index → color mapping is not on the wire. The daemon
//! paints overlay cells directly into the recipient's grid using
//! the resolved color; frontends just render the resulting
//! `CellDelta`. Changing the palette is a daemon-only visual update,
//! not a protocol change.

use crate::cell::Color;
use crate::protocol::FrontendId;

/// Number of distinct slots in the palette.
///
/// 8 colors covers the typical multi-frontend deployment (2–4
/// attached frontends) with headroom. Beyond [`PALETTE_LEN`]
/// distinct uids, slot assignment wraps; two uids may share a
/// color (the same shape as a hash collision).
pub const PALETTE_LEN: usize = 8;

/// 8-color palette. Indexed by slot (0..[`PALETTE_LEN`]).
///
/// Chosen for visibility against typical terminal backgrounds
/// (both light and dark themes). All entries are explicit Rgb
/// tuples so the rendering is theme-independent — the palette
/// doesn't rely on terminal palette overrides.
pub const PALETTE: [Color; PALETTE_LEN] = [
    Color::Rgb(0x00, 0xB7, 0xC3), // cyan
    Color::Rgb(0xC2, 0x4F, 0xC2), // magenta
    Color::Rgb(0x3A, 0xA0, 0x4F), // green
    Color::Rgb(0xD8, 0xA0, 0x10), // gold
    Color::Rgb(0x4A, 0x90, 0xE2), // blue
    Color::Rgb(0xE0, 0x50, 0x50), // red
    Color::Rgb(0xB0, 0xB0, 0xB0), // gray
    Color::Rgb(0xA8, 0x70, 0x30), // brown
];

/// Resolve a slot index to a `Color`. Wraps via modulo for
/// safety, though the daemon should never produce out-of-range
/// slots.
#[must_use]
pub fn color_for_slot(slot: u8) -> Color {
    PALETTE[(slot as usize) % PALETTE_LEN]
}

/// Label character for a `FrontendId`.
///
/// Returns `Some('A'..'Z')` for FrontendId(2)..FrontendId(27)
/// (daemon-attached frontends start at 2 because FrontendId(1)
/// is reserved for the in-process TUI). Returns `None` for
/// FrontendId(28) and beyond — labels are an aid, not a
/// requirement, and the colored cursor cell still distinguishes
/// the frontend.
///
/// v0.2+ may add username-based labels when user-identity
/// infrastructure lands. The v1.0 contract caps labels at 26 and
/// degrades gracefully past that.
#[must_use]
pub fn label_for_frontend_id(fid: FrontendId) -> Option<char> {
    let raw = fid.0;
    if (2..=27).contains(&raw) {
        // FrontendId(2) → 'A', FrontendId(3) → 'B', etc.
        let idx = u8::try_from(raw - 2).ok()?;
        Some((b'A' + idx) as char)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_has_documented_length() {
        assert_eq!(PALETTE.len(), PALETTE_LEN);
    }

    #[test]
    fn color_for_slot_returns_palette_entry() {
        for slot in 0..PALETTE_LEN as u8 {
            let color = color_for_slot(slot);
            assert_eq!(color, PALETTE[slot as usize]);
        }
    }

    #[test]
    fn color_for_slot_wraps_modulo() {
        assert_eq!(color_for_slot(0), color_for_slot(PALETTE_LEN as u8));
        assert_eq!(color_for_slot(1), color_for_slot(PALETTE_LEN as u8 + 1));
    }

    #[test]
    fn label_for_first_daemon_attached_is_a() {
        assert_eq!(label_for_frontend_id(FrontendId(2)), Some('A'));
    }

    #[test]
    fn label_for_z_boundary() {
        assert_eq!(label_for_frontend_id(FrontendId(27)), Some('Z'));
    }

    #[test]
    fn label_beyond_z_is_none() {
        assert_eq!(label_for_frontend_id(FrontendId(28)), None);
        assert_eq!(label_for_frontend_id(FrontendId(100)), None);
    }

    #[test]
    fn label_for_local_is_none() {
        // FrontendId(1) is LOCAL; never gets a label (it's the
        // in-process TUI, not a peer that needs to be distinguished).
        assert_eq!(label_for_frontend_id(FrontendId::LOCAL), None);
    }

    #[test]
    fn label_for_zero_is_none() {
        // Defensive — FrontendId(0) shouldn't exist but handle
        // gracefully.
        assert_eq!(label_for_frontend_id(FrontendId(0)), None);
    }
}
