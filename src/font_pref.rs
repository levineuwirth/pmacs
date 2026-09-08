//! The global GPU font preference (Arc 4 stage 2, framing Q#F3,
//! `docs/archive/framings/gpu-set-font-framing.md`).
//!
//! One daemon-side preference — family name and/or size — written by
//! `pmacs.gpu.set_font` and read by the `semantic_render` producer,
//! which relays it to GPU-capable peers as the bufferless
//! `InstanceMessage::FontFacts` at protocol v17. The daemon relays a
//! PREFERENCE: it never learns metrics, advances, or what resolves
//! (the no-pixels invariant); the frontend owns resolution and every
//! pixel consequence.

use std::sync::{Arc, Mutex};

/// Shared handle, mirroring [`crate::highlight::ThemeHandle`]'s
/// shape: the Lua setter writes it, per-session producers read it.
pub type FontPrefHandle = Arc<Mutex<FontPref>>;

/// The preference itself. `None` per axis means "the frontend's
/// built-in default" — a REAL, always-shipped state, never inferred
/// from silence (the Q#TH7 authoritative-per-attachment lesson).
#[derive(Debug, Default)]
pub struct FontPref {
    /// Font family name to resolve frontend-locally, or `None` for
    /// the frontend's default family query.
    pub family: Option<String>,
    /// Size in HUNDREDTHS of a logical pixel (1600 = 16.0), already
    /// validated and quantized by the Lua boundary (range-check the
    /// original value first, then nearest-hundredth via round —
    /// framing Q#F2). `u32` matches the wire, which derives `Eq`.
    pub size_centi_px: Option<u32>,
    /// Monotonic mutation counter, increment-only from its prior
    /// value on every successful `set_font` (the Q#TH6 lesson). The
    /// producer's `Option`-seeded gate compares this one `u64` per
    /// tick.
    pub epoch: u64,
}

/// Fresh all-default preference behind a new handle.
#[must_use]
pub fn new_handle() -> FontPrefHandle {
    Arc::new(Mutex::new(FontPref::default()))
}
