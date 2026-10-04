//! Hover and signature popups at the caret (E8, protocol v26).
//!
//! A popup is the daemon's decision that a short text surface floats at
//! a byte of the buffer the receiving frontend shows: the language
//! server's hover documentation for the symbol under the caret, or the
//! signature of the call the caret sits in with its active parameter
//! marked. The daemon owns the popup's life (what opens it, what
//! dismisses it, which window it belongs to); a frontend owns every
//! pixel and cell of it, including how its lines wrap and, on the GPU,
//! how far it is scrolled.
//!
//! Presence is explicit, as for the panel band: [`PopupPayload::Absent`]
//! is authoritative and is sent when the popup closes, because a
//! receiver keeps its last popup and silence would leave it on screen.
//!
//! # The bound
//!
//! Hover text is arbitrary server markdown: unbounded length, code
//! blocks, sometimes a screenful of trait bounds. The wire carries at
//! most [`MAX_POPUP_LINES`] lines, each at most [`MAX_POPUP_LINE_BYTES`],
//! together at most [`MAX_POPUP_TEXT_BYTES`]. The producer cuts what the
//! server sent to fit and says how much it left out in
//! [`PopupFrame::omitted_lines`], so a frontend can say so rather than
//! end the text silently; the whole text stays reachable in the
//! `*lsp-help*` buffer. A frame over the bound is not cut by a
//! receiver: [`PopupFrame::validate`] rejects it whole.

use crate::ids::BufferId;

/// Lowest negotiated protocol version that carries the popup family:
/// [`crate::InstanceMessage::Popup`], daemon to frontend.
///
/// One constant rather than a literal at each gate, as
/// [`crate::PANEL_MIN_VERSION`] is: the daemon's write filter, the
/// producer's peer flag, and the GPU frontend's apply and paint gates
/// all read this definition, so they cannot drift apart and leave one
/// side trusting a wire the other never negotiated. The family has no
/// inbound event --- a click or a wheel turn inside a popup is the
/// frontend's own --- so there is no receiver-side gate at the daemon.
///
/// Distinct from [`crate::ADVERTISED_PROTOCOL_VERSION`], which stays at
/// the compatibility baseline permanently: a session reaches this
/// version by the frontend's `AttachRequest` counter-offer.
pub const POPUP_MIN_VERSION: u32 = 26;

/// Most lines a popup frame carries.
pub const MAX_POPUP_LINES: usize = 256;

/// Most bytes one popup line carries.
pub const MAX_POPUP_LINE_BYTES: usize = 4096;

/// Most bytes of text one popup frame carries, summed over its lines.
pub const MAX_POPUP_TEXT_BYTES: usize = 32 * 1024;

/// What a popup shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PopupKind {
    /// The server's hover documentation for the symbol at the anchor.
    Hover,
    /// The signature of the call around the caret.
    Signature,
}

/// The active parameter of a signature popup: bytes `start..end` of
/// `lines[line]`, both on character boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PopupActiveRange {
    /// Index into [`PopupFrame::lines`].
    pub line: u32,
    /// First byte of the parameter in that line.
    pub start: u32,
    /// One past its last byte.
    pub end: u32,
}

/// One popup, as the receiving frontend should show it.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PopupFrame {
    /// Buffer the anchor indexes. A frontend showing another buffer
    /// draws nothing.
    pub buffer_id: BufferId,
    /// The byte the popup floats at: the start of the hovered symbol,
    /// or the caret a signature was asked for.
    pub anchor_byte: u64,
    /// What the popup shows.
    pub kind: PopupKind,
    /// The text, one entry per logical line, none containing a control
    /// character; a frontend wraps them to its own width.
    pub lines: Vec<String>,
    /// The active parameter, for a [`PopupKind::Signature`] only.
    pub active_range: Option<PopupActiveRange>,
    /// Lines the producer did not ship because the text passed the
    /// bound. Zero when `lines` is the whole text.
    pub omitted_lines: u32,
}

/// Explicit popup presence.
///
/// `Absent` is authoritative rather than implied by silence, and is
/// duplicate-suppressed like any payload.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PopupPayload {
    /// A popup is open and this is its content.
    Present(PopupFrame),
    /// No popup is open; clear any retained one.
    Absent,
}

/// Why a [`PopupFrame`] is not structurally valid.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PopupFrameError {
    /// A present popup with no lines; closing is [`PopupPayload::Absent`].
    #[error("popup carries no lines")]
    Empty,
    /// More lines than [`MAX_POPUP_LINES`].
    #[error("popup carries {lines} lines, over the bound {max}")]
    Lines {
        /// Lines carried.
        lines: usize,
        /// The bound.
        max: usize,
    },
    /// A line longer than [`MAX_POPUP_LINE_BYTES`].
    #[error("popup line {index} is {bytes} bytes, over the bound {max}")]
    LineBytes {
        /// The line.
        index: usize,
        /// Its length.
        bytes: usize,
        /// The bound.
        max: usize,
    },
    /// More text than [`MAX_POPUP_TEXT_BYTES`].
    #[error("popup text is {bytes} bytes, over the bound {max}")]
    TextBytes {
        /// Bytes carried.
        bytes: usize,
        /// The bound.
        max: usize,
    },
    /// A line carries a control character (a newline included: lines
    /// are already split).
    #[error("popup line {index} carries a control character")]
    Control {
        /// The line.
        index: usize,
    },
    /// The active range is outside its line, empty, or splits a
    /// character.
    #[error("popup active range is not a character span of its line")]
    ActiveRange,
    /// A hover carries an active range, which only a signature has.
    #[error("a hover popup carries an active range")]
    HoverActiveRange,
}

impl PopupFrame {
    /// Check every structural rule a popup frame must satisfy.
    ///
    /// Pure: a rejected frame mutates nothing, so a receiver can drop
    /// it whole.
    pub fn validate(&self) -> Result<(), PopupFrameError> {
        if self.lines.is_empty() {
            return Err(PopupFrameError::Empty);
        }
        if self.lines.len() > MAX_POPUP_LINES {
            return Err(PopupFrameError::Lines {
                lines: self.lines.len(),
                max: MAX_POPUP_LINES,
            });
        }
        let mut total = 0usize;
        for (index, line) in self.lines.iter().enumerate() {
            if line.len() > MAX_POPUP_LINE_BYTES {
                return Err(PopupFrameError::LineBytes {
                    index,
                    bytes: line.len(),
                    max: MAX_POPUP_LINE_BYTES,
                });
            }
            if line.chars().any(char::is_control) {
                return Err(PopupFrameError::Control { index });
            }
            total += line.len();
        }
        if total > MAX_POPUP_TEXT_BYTES {
            return Err(PopupFrameError::TextBytes {
                bytes: total,
                max: MAX_POPUP_TEXT_BYTES,
            });
        }
        if let Some(range) = self.active_range {
            if self.kind == PopupKind::Hover {
                return Err(PopupFrameError::HoverActiveRange);
            }
            let line = self
                .lines
                .get(range.line as usize)
                .ok_or(PopupFrameError::ActiveRange)?;
            let (start, end) = (range.start as usize, range.end as usize);
            if start >= end
                || end > line.len()
                || !line.is_char_boundary(start)
                || !line.is_char_boundary(end)
            {
                return Err(PopupFrameError::ActiveRange);
            }
        }
        Ok(())
    }
}

/// Where a popup of `size` goes beside an anchor inside a window of
/// `area`, as its top-left corner --- the one placement rule for popups
/// on both frontends (E7d.3, which E8 inherits rather than restating).
/// The GPU calls it in logical pixels and the grid in cells; the rule
/// does not care which.
///
/// `anchor` is `(x, top, bottom)`: a click is a point, so its top and
/// bottom coincide; a caret is its line's top and bottom. The popup goes
/// below the anchor when it fits there, else above it when it fits
/// there, else on the roomier side, and is then clamped into the window
/// on both axes; horizontally it starts at the anchor and is pulled left
/// until its right edge is inside. A popup larger than the window keeps
/// its top-left corner inside it, so its first row and left edge stay
/// on screen and the rest is clipped.
#[must_use]
pub fn place_popup(anchor: (f32, f32, f32), size: (f32, f32), area: (f32, f32)) -> (f32, f32) {
    let (ax, top, bottom) = anchor;
    let (w, h) = size;
    let (width, height) = area;
    let x = ax.min(width - w).max(0.0);
    let room_below = height - bottom;
    let room_above = top;
    // Above only when it fits there and not below, or when it fits
    // neither side and above is roomier; the clamp settles the rest.
    let above = h > room_below && (h <= room_above || room_above > room_below);
    let y = if above { top - h } else { bottom };
    (x, y.min(height - h).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(lines: &[&str]) -> PopupFrame {
        PopupFrame {
            buffer_id: BufferId::from_raw(1),
            anchor_byte: 0,
            kind: PopupKind::Signature,
            lines: lines.iter().map(|l| (*l).to_owned()).collect(),
            active_range: None,
            omitted_lines: 0,
        }
    }

    #[test]
    fn a_frame_at_every_bound_is_valid_and_one_past_each_is_not() {
        let at_lines = PopupFrame {
            lines: vec!["x".to_owned(); MAX_POPUP_LINES],
            ..frame(&[])
        };
        assert_eq!(at_lines.validate(), Ok(()));
        let past_lines = PopupFrame {
            lines: vec!["x".to_owned(); MAX_POPUP_LINES + 1],
            ..frame(&[])
        };
        assert_eq!(
            past_lines.validate(),
            Err(PopupFrameError::Lines {
                lines: MAX_POPUP_LINES + 1,
                max: MAX_POPUP_LINES
            })
        );
        let long = "y".repeat(MAX_POPUP_LINE_BYTES);
        assert_eq!(frame(&[&long]).validate(), Ok(()));
        let too_long = "y".repeat(MAX_POPUP_LINE_BYTES + 1);
        assert!(matches!(
            frame(&[&too_long]).validate(),
            Err(PopupFrameError::LineBytes { index: 0, .. })
        ));
        let lines = MAX_POPUP_TEXT_BYTES / MAX_POPUP_LINE_BYTES;
        let at_text = PopupFrame {
            lines: vec![long.clone(); lines],
            ..frame(&[])
        };
        assert_eq!(at_text.validate(), Ok(()));
        let past_text = PopupFrame {
            lines: [vec![long; lines], vec!["z".to_owned()]].concat(),
            ..frame(&[])
        };
        assert!(matches!(
            past_text.validate(),
            Err(PopupFrameError::TextBytes { .. })
        ));
    }

    #[test]
    fn an_empty_popup_a_control_character_and_a_bad_active_range_are_rejected() {
        assert_eq!(frame(&[]).validate(), Err(PopupFrameError::Empty));
        assert_eq!(
            frame(&["ok", "tab\there"]).validate(),
            Err(PopupFrameError::Control { index: 1 })
        );
        assert_eq!(
            frame(&["a\nb"]).validate(),
            Err(PopupFrameError::Control { index: 0 })
        );
        let with = |line, start, end| PopupFrame {
            active_range: Some(PopupActiveRange { line, start, end }),
            ..frame(&["fn f(é: u8)"])
        };
        assert_eq!(with(0, 5, 7).validate(), Ok(()), "é is two bytes");
        assert_eq!(with(0, 5, 6).validate(), Err(PopupFrameError::ActiveRange));
        assert_eq!(with(0, 5, 5).validate(), Err(PopupFrameError::ActiveRange));
        assert_eq!(with(0, 5, 99).validate(), Err(PopupFrameError::ActiveRange));
        assert_eq!(with(1, 0, 1).validate(), Err(PopupFrameError::ActiveRange));
        let hover = PopupFrame {
            kind: PopupKind::Hover,
            ..with(0, 0, 2)
        };
        assert_eq!(hover.validate(), Err(PopupFrameError::HoverActiveRange));
    }

    #[test]
    fn place_popup_flips_above_without_room_below_and_clamps_right() {
        // Room below: below the anchor's bottom.
        assert_eq!(
            place_popup((10.0, 5.0, 6.0), (20.0, 4.0), (80.0, 24.0)),
            (10.0, 6.0)
        );
        // No room below, room above: its bottom edge on the anchor's top.
        assert_eq!(
            place_popup((10.0, 20.0, 21.0), (20.0, 6.0), (80.0, 24.0)),
            (10.0, 14.0)
        );
        // No room right: pulled left to the window's edge.
        assert_eq!(
            place_popup((70.0, 5.0, 6.0), (20.0, 4.0), (80.0, 24.0)),
            (60.0, 6.0)
        );
        // Larger than the window: its top-left corner stays inside.
        assert_eq!(
            place_popup((70.0, 5.0, 6.0), (100.0, 40.0), (80.0, 24.0)),
            (0.0, 0.0)
        );
    }
}
