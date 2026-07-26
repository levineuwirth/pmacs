//! Terminal wire types (Vterm Stage 3, protocol v19).
//!
//! A terminal is inherently a cell protocol: its identity buffer is an
//! empty CRDT anchor, so a semantic frontend that receives only
//! `BufferSnapshot` has nothing to draw. [`TerminalFrame`] carries the
//! complete visible grid instead — cells, cursor, per-view selection and
//! scroll state, and sanitized process metadata.
//!
//! Every limit needed to validate a frame lives here rather than in the
//! `pmacs` crate, so the daemon's pre-emission check and a frontend's
//! post-decode check run the SAME policy. [`TerminalFrame::validate`] is
//! that single structural policy. A second implementation of these rules
//! in a frontend is a bug, not a convenience.
//!
//! Glyph column width and wide-continuation topology moved to
//! [`crate::wire_grid`] in bottom-panel Stage 2B, which is now the
//! crate's only non-test `unicode-width` use: those rules are shared
//! with [`crate::panel::PanelFrame`]. The 512 per-axis PTY caps,
//! metadata, selection spans, and the `at_bottom`/`scroll_offset`
//! coupling stay here, because a panel does not inherit them.

use crate::cell::{Cell, CellCoord, CellSize};
use crate::ids::BufferId;

#[cfg(test)]
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------
// Shared limits
// ---------------------------------------------------------------------------

/// Maximum terminal rows accepted at creation, resize, or on the wire.
pub const MAX_TERMINAL_ROWS: u16 = 512;

/// Maximum terminal columns accepted at creation, resize, or on the wire.
pub const MAX_TERMINAL_COLS: u16 = 512;

/// Maximum visible terminal cells accepted at creation, resize, or on
/// the wire.
pub const MAX_TERMINAL_VISIBLE_CELLS: usize = 262_144;

/// Maximum UTF-8 bytes retained in one terminal grapheme cluster.
pub const MAX_TERMINAL_GRAPHEME_BYTES: usize = 256;

/// Shared cap for terminal title and process-outcome metadata.
pub const MAX_TERMINAL_METADATA_BYTES: usize = 1_024;

/// Aggregate cap on glyph bytes carried by one [`TerminalFrame`].
///
/// The visible-cell bound alone does not bound a frame's encoded size:
/// every cell may legally carry a [`MAX_TERMINAL_GRAPHEME_BYTES`]
/// cluster, and `262_144 * 256` is 64 MiB — four times the transport's
/// [`crate::MAX_FRAME_BYTES`]. Raising the transport cap would widen the
/// allocation ceiling for every pre-handshake and established message on
/// every connection, so v19 bounds the terminal payload instead. The
/// protocol test `maximum_legal_terminal_frame_encodes_below_the_transport_cap`
/// measures the largest legal frame this bound admits and pins it below
/// the unchanged 16 MiB cap.
pub const MAX_TERMINAL_FRAME_GLYPH_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

/// Process outcome published with a terminal frame.
///
/// `Signaled` and `Crashed` text is sanitized metadata subject to
/// [`MAX_TERMINAL_METADATA_BYTES`] and the control-character rule; it is
/// display text, never a host control effect.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TerminalProcessState {
    /// Child is running, or termination has only been requested.
    Running,
    /// Child exited with a status code.
    Exited(i32),
    /// Child was terminated by a sanitized symbolic signal.
    Signaled(String),
    /// Supervision failed after the session was published.
    Crashed(String),
}

/// One selected span on one visible terminal row.
///
/// `start_col` is inclusive, `end_col` exclusive. A frame carries at most
/// one span per row, in strictly increasing row order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TerminalSelectionSpan {
    /// Visible row.
    pub row: u32,
    /// Inclusive starting column.
    pub start_col: u32,
    /// Exclusive ending column.
    pub end_col: u32,
}

/// A complete visible terminal grid for one frontend/window view.
///
/// This is a whole-grid replacement, never a delta. Empty is NOT a clear
/// sentinel: valid dimensions are nonzero and `cells.len()` equals the
/// checked area exactly, so a receiver that decodes a valid frame always
/// has a complete picture.
///
/// `screen_generation` describes the published screen/title generation
/// only. Selection, scroll, viewport, and process state change WITHOUT
/// advancing it, so a producer must compare the complete ordered payload
/// — not that counter — when deciding whether to send.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TerminalFrame {
    /// Identity buffer whose session this frame projects.
    pub buffer_id: BufferId,
    /// Visible grid dimensions in cells.
    pub size: CellSize,
    /// Row-major visible cells; exactly `size.area()` entries.
    pub cells: Vec<Cell>,
    /// Visible child cursor, or `None` when hidden.
    pub cursor: Option<CellCoord>,
    /// Sanitized child title.
    pub title: Option<String>,
    /// Published terminal screen/title generation.
    pub screen_generation: u64,
    /// Per-row selection spans, strictly increasing by row.
    pub selection: Vec<TerminalSelectionSpan>,
    /// Physical retained rows between this viewport and the live tail.
    pub scroll_offset: u32,
    /// Whether this view reaches the live tail; always `scroll_offset == 0`.
    pub at_bottom: bool,
    /// Operating-system process id for this session generation.
    pub pid: u32,
    /// Latest observed process state.
    pub process: TerminalProcessState,
}

/// Why a [`TerminalFrame`] is not structurally valid.
///
/// Validation is atomic: the frame is rejected whole. A receiver retains
/// its previous valid frame rather than applying any part of this one.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TerminalFrameError {
    /// Rows or columns are zero or above the shared bounds.
    #[error("terminal size {rows}x{cols} is outside 1..={max_rows}x1..={max_cols}")]
    Size {
        /// Declared rows.
        rows: u32,
        /// Declared columns.
        cols: u32,
        /// Shared row bound.
        max_rows: u32,
        /// Shared column bound.
        max_cols: u32,
    },
    /// The checked area exceeds the shared visible-cell bound.
    #[error("terminal area {area} exceeds the visible-cell bound {max}")]
    Area {
        /// Checked `rows * cols`.
        area: usize,
        /// Shared visible-cell bound.
        max: usize,
    },
    /// `cells.len()` disagrees with the declared area.
    #[error("terminal frame carries {actual} cells for a {expected}-cell area")]
    CellCount {
        /// Declared area.
        expected: usize,
        /// Supplied cell count.
        actual: usize,
    },
    /// The cursor lies outside the declared grid.
    #[error("terminal cursor ({row},{col}) is outside the {rows}x{cols} grid")]
    Cursor {
        /// Cursor row.
        row: u32,
        /// Cursor column.
        col: u32,
        /// Declared rows.
        rows: u32,
        /// Declared columns.
        cols: u32,
    },
    /// Title or process metadata is too long or carries control characters.
    #[error("terminal {field} metadata is invalid: {reason}")]
    Metadata {
        /// Which metadata field failed.
        field: &'static str,
        /// Why it failed.
        reason: &'static str,
    },
    /// A cell's glyph is not a legal terminal glyph.
    #[error("terminal cell {index} has an invalid glyph: {reason}")]
    Glyph {
        /// Row-major cell index.
        index: usize,
        /// Why the glyph failed.
        reason: &'static str,
    },
    /// A cell carries a frontend attachment, which terminals never use.
    #[error("terminal cell {index} carries an attachment")]
    Attachment {
        /// Row-major cell index.
        index: usize,
    },
    /// Aggregate glyph bytes exceed [`MAX_TERMINAL_FRAME_GLYPH_BYTES`].
    #[error("terminal frame glyph bytes exceed the aggregate bound {max}")]
    GlyphBudget {
        /// Shared aggregate bound.
        max: usize,
    },
    /// A selection span is empty, out of order, duplicated, or out of bounds.
    #[error("terminal selection span {index} is invalid: {reason}")]
    Selection {
        /// Index into `selection`.
        index: usize,
        /// Why the span failed.
        reason: &'static str,
    },
    /// `at_bottom` disagrees with `scroll_offset == 0`.
    #[error("terminal at_bottom {at_bottom} disagrees with scroll_offset {scroll_offset}")]
    BottomState {
        /// Declared bottom state.
        at_bottom: bool,
        /// Declared scroll offset.
        scroll_offset: u32,
    },
}

/// Bounds a terminal frame enforces on its cell grid.
///
/// The per-axis caps are the PTY-specific half of the split: a panel
/// frame shares every other rule but not these, because a panel is
/// sized by the frontend's surface rather than by a pty window size.
const TERMINAL_GRID_LIMITS: crate::wire_grid::WireGridLimits = crate::wire_grid::WireGridLimits {
    max_rows: MAX_TERMINAL_ROWS as u32,
    max_cols: MAX_TERMINAL_COLS as u32,
    max_visible_cells: MAX_TERMINAL_VISIBLE_CELLS,
    max_glyph_bytes: MAX_TERMINAL_FRAME_GLYPH_BYTES,
};

/// Map a shared wire-grid failure onto this message's error type.
///
/// The variants and their text are unchanged by the Stage 2B factoring:
/// every existing terminal-frame assertion still observes exactly what
/// it observed before.
fn terminal_grid_error(error: crate::wire_grid::WireGridError) -> TerminalFrameError {
    use crate::wire_grid::WireGridError;
    match error {
        WireGridError::Size {
            rows,
            cols,
            max_rows,
            max_cols,
        } => TerminalFrameError::Size {
            rows,
            cols,
            max_rows,
            max_cols,
        },
        WireGridError::Area { area, max } => TerminalFrameError::Area { area, max },
        WireGridError::CellCount { expected, actual } => {
            TerminalFrameError::CellCount { expected, actual }
        }
        WireGridError::Cursor {
            row,
            col,
            rows,
            cols,
        } => TerminalFrameError::Cursor {
            row,
            col,
            rows,
            cols,
        },
        WireGridError::Glyph { index, reason } => TerminalFrameError::Glyph { index, reason },
        WireGridError::Attachment { index } => TerminalFrameError::Attachment { index },
        WireGridError::GlyphBudget { max } => TerminalFrameError::GlyphBudget { max },
    }
}

impl TerminalFrame {
    /// Check every structural rule a terminal frame must satisfy.
    ///
    /// This is the single policy used by the daemon before emission and
    /// by a frontend after decode. It is pure: a rejected frame mutates
    /// nothing, so callers get atomic rejection for free.
    pub fn validate(&self) -> Result<(), TerminalFrameError> {
        crate::wire_grid::validate_wire_grid(
            self.size,
            &self.cells,
            self.cursor,
            TERMINAL_GRID_LIMITS,
        )
        .map_err(terminal_grid_error)?;
        if let Some(title) = &self.title {
            validate_metadata("title", title)?;
        }
        match &self.process {
            TerminalProcessState::Signaled(text) => validate_metadata("signal", text)?,
            TerminalProcessState::Crashed(text) => validate_metadata("crash", text)?,
            TerminalProcessState::Running | TerminalProcessState::Exited(_) => {}
        }
        self.validate_selection()?;
        if self.at_bottom != (self.scroll_offset == 0) {
            return Err(TerminalFrameError::BottomState {
                at_bottom: self.at_bottom,
                scroll_offset: self.scroll_offset,
            });
        }
        Ok(())
    }

    /// One nonempty in-bounds span per row, strictly increasing by row.
    fn validate_selection(&self) -> Result<(), TerminalFrameError> {
        let mut previous_row: Option<u32> = None;
        for (index, span) in self.selection.iter().enumerate() {
            if span.row >= self.size.rows {
                return Err(TerminalFrameError::Selection {
                    index,
                    reason: "row is outside the frame",
                });
            }
            if span.start_col >= span.end_col {
                return Err(TerminalFrameError::Selection {
                    index,
                    reason: "span is empty or reversed",
                });
            }
            if span.end_col > self.size.cols {
                return Err(TerminalFrameError::Selection {
                    index,
                    reason: "span extends past the final column",
                });
            }
            if previous_row.is_some_and(|previous| previous >= span.row) {
                return Err(TerminalFrameError::Selection {
                    index,
                    reason: "rows are not strictly increasing",
                });
            }
            previous_row = Some(span.row);
        }
        Ok(())
    }
}

/// Length and control-character rules shared by title and process text.
fn validate_metadata(field: &'static str, text: &str) -> Result<(), TerminalFrameError> {
    if text.len() > MAX_TERMINAL_METADATA_BYTES {
        return Err(TerminalFrameError::Metadata {
            field,
            reason: "exceeds the metadata byte limit",
        });
    }
    if text.chars().any(char::is_control) {
        return Err(TerminalFrameError::Metadata {
            field,
            reason: "carries a control character",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // `Glyph` is no longer used by this module's production code — the
    // glyph rules moved to `crate::wire_grid` — but these tests still
    // construct frames cell by cell.
    use crate::cell::{Color, Glyph, Style, UnderlineStyle};
    use crate::message::InstanceMessage;
    use crate::transport::MAX_FRAME_BYTES;

    /// The style whose postcard encoding is as long as a legal `Style`
    /// gets: three truecolor components, a non-`None` underline, and
    /// every boolean set.
    fn maximal_style() -> Style {
        Style {
            fg: Color::Rgb(0xff, 0xee, 0xdd),
            bg: Color::Rgb(0x11, 0x22, 0x33),
            bold: true,
            italic: true,
            underline: UnderlineStyle::Dashed,
            reverse: true,
            underline_color: Color::Rgb(0x44, 0x55, 0x66),
        }
    }

    /// A legal one-column cluster of exactly `len` UTF-8 bytes.
    ///
    /// A combining mark is two bytes, so parity comes from the base:
    /// a one-byte space for odd lengths, a two-byte `é` for even ones.
    /// Width stays 1 no matter how many marks are appended.
    fn cluster_of_len(len: usize) -> Vec<u8> {
        assert!((1..=MAX_TERMINAL_GRAPHEME_BYTES).contains(&len));
        let mut text = String::with_capacity(len);
        if len % 2 == 1 {
            text.push(' ');
        } else {
            text.push('\u{e9}');
        }
        while text.len() < len {
            text.push('\u{301}');
        }
        assert_eq!(text.len(), len);
        assert_eq!(UnicodeWidthStr::width(text.as_str()), 1);
        text.into_bytes()
    }

    fn cell_with(glyph: Glyph) -> Cell {
        Cell {
            glyph,
            style: maximal_style(),
            attachment: None,
        }
    }

    /// A minimal valid frame: `rows * cols` single-space cells.
    fn frame(rows: u32, cols: u32) -> TerminalFrame {
        let area = (rows * cols) as usize;
        TerminalFrame {
            buffer_id: BufferId::from_raw(7),
            size: CellSize::new(rows, cols),
            cells: vec![cell_with(Glyph::Char(' ')); area],
            cursor: Some(CellCoord::new(0, 0)),
            title: Some("sh".into()),
            screen_generation: 3,
            selection: Vec::new(),
            scroll_offset: 0,
            at_bottom: true,
            pid: 4321,
            process: TerminalProcessState::Running,
        }
    }

    #[test]
    fn exact_shared_boundaries_are_accepted() {
        let mut f = frame(2, 4);
        // A wide lead plus its continuation, a maximal-length cluster,
        // and a maximal metadata string are all exactly legal.
        f.cells[0] = cell_with(Glyph::Char('\u{4e00}'));
        f.cells[1] = cell_with(Glyph::Continuation);
        f.cells[2] = cell_with(Glyph::Cluster(
            cluster_of_len(MAX_TERMINAL_GRAPHEME_BYTES).into_boxed_slice(),
        ));
        f.title = Some("t".repeat(MAX_TERMINAL_METADATA_BYTES));
        f.process = TerminalProcessState::Signaled("s".repeat(MAX_TERMINAL_METADATA_BYTES));
        f.cursor = Some(CellCoord::new(1, 3));
        f.selection = vec![
            TerminalSelectionSpan {
                row: 0,
                start_col: 0,
                end_col: 4,
            },
            TerminalSelectionSpan {
                row: 1,
                start_col: 3,
                end_col: 4,
            },
        ];
        assert_eq!(f.validate(), Ok(()));
    }

    #[test]
    #[allow(clippy::too_many_lines, reason = "one case per structural rule")]
    fn every_structural_violation_is_rejected() {
        // Each case mutates one axis of an otherwise valid frame, so a
        // failure names exactly the rule that stopped enforcing.
        let zero_rows = {
            let mut f = frame(1, 1);
            f.size = CellSize::new(0, 1);
            f
        };
        assert!(matches!(
            zero_rows.validate(),
            Err(TerminalFrameError::Size { .. })
        ));

        let over_rows = {
            let mut f = frame(1, 1);
            f.size = CellSize::new(u32::from(MAX_TERMINAL_ROWS) + 1, 1);
            f
        };
        assert!(matches!(
            over_rows.validate(),
            Err(TerminalFrameError::Size { .. })
        ));

        let over_area = {
            let mut f = frame(1, 1);
            f.size = CellSize::new(u32::from(MAX_TERMINAL_ROWS), u32::from(MAX_TERMINAL_COLS));
            f
        };
        // 512 x 512 is exactly the visible-cell bound, so this frame
        // fails on its cell count, not its area — the area bound is
        // reached only if a later limit change makes them disagree.
        assert!(matches!(
            over_area.validate(),
            Err(TerminalFrameError::CellCount { .. })
        ));

        let bad_count = {
            let mut f = frame(2, 2);
            f.cells.pop();
            f
        };
        assert!(matches!(
            bad_count.validate(),
            Err(TerminalFrameError::CellCount {
                expected: 4,
                actual: 3
            })
        ));

        let bad_cursor = {
            let mut f = frame(2, 2);
            f.cursor = Some(CellCoord::new(2, 0));
            f
        };
        assert!(matches!(
            bad_cursor.validate(),
            Err(TerminalFrameError::Cursor { .. })
        ));

        let long_title = {
            let mut f = frame(1, 1);
            f.title = Some("t".repeat(MAX_TERMINAL_METADATA_BYTES + 1));
            f
        };
        assert!(matches!(
            long_title.validate(),
            Err(TerminalFrameError::Metadata { field: "title", .. })
        ));

        let control_title = {
            let mut f = frame(1, 1);
            f.title = Some("ok\u{7}".into());
            f
        };
        assert!(matches!(
            control_title.validate(),
            Err(TerminalFrameError::Metadata { field: "title", .. })
        ));

        let long_process = {
            let mut f = frame(1, 1);
            f.process = TerminalProcessState::Crashed("c".repeat(MAX_TERMINAL_METADATA_BYTES + 1));
            f
        };
        assert!(matches!(
            long_process.validate(),
            Err(TerminalFrameError::Metadata { field: "crash", .. })
        ));

        let control_glyph = {
            let mut f = frame(1, 1);
            f.cells[0] = cell_with(Glyph::Char('\u{7}'));
            f
        };
        assert!(matches!(
            control_glyph.validate(),
            Err(TerminalFrameError::Glyph { index: 0, .. })
        ));

        let malformed_cluster = {
            let mut f = frame(1, 1);
            f.cells[0] = cell_with(Glyph::Cluster(vec![0xff, 0xfe].into_boxed_slice()));
            f
        };
        assert!(matches!(
            malformed_cluster.validate(),
            Err(TerminalFrameError::Glyph { index: 0, .. })
        ));

        let empty_cluster = {
            let mut f = frame(1, 1);
            f.cells[0] = cell_with(Glyph::Cluster(Vec::new().into_boxed_slice()));
            f
        };
        assert!(matches!(
            empty_cluster.validate(),
            Err(TerminalFrameError::Glyph { index: 0, .. })
        ));

        let overlong_cluster = {
            let mut f = frame(1, 1);
            let mut bytes = cluster_of_len(MAX_TERMINAL_GRAPHEME_BYTES);
            bytes.extend_from_slice("\u{301}".as_bytes());
            f.cells[0] = cell_with(Glyph::Cluster(bytes.into_boxed_slice()));
            f
        };
        assert!(matches!(
            overlong_cluster.validate(),
            Err(TerminalFrameError::Glyph { index: 0, .. })
        ));

        let orphan_continuation = {
            let mut f = frame(1, 2);
            f.cells[1] = cell_with(Glyph::Continuation);
            f.cells[0] = cell_with(Glyph::Char('a'));
            f
        };
        assert!(matches!(
            orphan_continuation.validate(),
            Err(TerminalFrameError::Glyph { index: 1, .. })
        ));

        let missing_continuation = {
            let mut f = frame(1, 2);
            f.cells[0] = cell_with(Glyph::Char('\u{4e00}'));
            f
        };
        assert!(matches!(
            missing_continuation.validate(),
            Err(TerminalFrameError::Glyph { index: 1, .. })
        ));

        // A wide lead in the final column has nowhere to put its
        // continuation: the next cell belongs to the next row.
        let wide_at_row_end = {
            let mut f = frame(2, 2);
            f.cells[1] = cell_with(Glyph::Char('\u{4e00}'));
            f
        };
        assert!(matches!(
            wide_at_row_end.validate(),
            Err(TerminalFrameError::Glyph { index: 1, .. })
        ));

        let attachment = {
            let mut f = frame(1, 1);
            f.cells[0].attachment = Some(crate::cell::Attachment::ImageCell {
                image_id: 1,
                sub_x: 0,
                sub_y: 0,
            });
            f
        };
        assert!(matches!(
            attachment.validate(),
            Err(TerminalFrameError::Attachment { index: 0 })
        ));

        let reversed_span = {
            let mut f = frame(1, 4);
            f.selection = vec![TerminalSelectionSpan {
                row: 0,
                start_col: 3,
                end_col: 1,
            }];
            f
        };
        assert!(matches!(
            reversed_span.validate(),
            Err(TerminalFrameError::Selection { index: 0, .. })
        ));

        let empty_span = {
            let mut f = frame(1, 4);
            f.selection = vec![TerminalSelectionSpan {
                row: 0,
                start_col: 2,
                end_col: 2,
            }];
            f
        };
        assert!(matches!(
            empty_span.validate(),
            Err(TerminalFrameError::Selection { index: 0, .. })
        ));

        let duplicate_row = {
            let mut f = frame(2, 4);
            f.selection = vec![
                TerminalSelectionSpan {
                    row: 1,
                    start_col: 0,
                    end_col: 1,
                },
                TerminalSelectionSpan {
                    row: 1,
                    start_col: 1,
                    end_col: 2,
                },
            ];
            f
        };
        assert!(matches!(
            duplicate_row.validate(),
            Err(TerminalFrameError::Selection { index: 1, .. })
        ));

        let out_of_order = {
            let mut f = frame(2, 4);
            f.selection = vec![
                TerminalSelectionSpan {
                    row: 1,
                    start_col: 0,
                    end_col: 1,
                },
                TerminalSelectionSpan {
                    row: 0,
                    start_col: 0,
                    end_col: 1,
                },
            ];
            f
        };
        assert!(matches!(
            out_of_order.validate(),
            Err(TerminalFrameError::Selection { index: 1, .. })
        ));

        let span_past_end = {
            let mut f = frame(1, 4);
            f.selection = vec![TerminalSelectionSpan {
                row: 0,
                start_col: 0,
                end_col: 5,
            }];
            f
        };
        assert!(matches!(
            span_past_end.validate(),
            Err(TerminalFrameError::Selection { index: 0, .. })
        ));

        let span_off_frame = {
            let mut f = frame(1, 4);
            f.selection = vec![TerminalSelectionSpan {
                row: 1,
                start_col: 0,
                end_col: 1,
            }];
            f
        };
        assert!(matches!(
            span_off_frame.validate(),
            Err(TerminalFrameError::Selection { index: 0, .. })
        ));

        let inconsistent_bottom = {
            let mut f = frame(1, 1);
            f.scroll_offset = 4;
            f.at_bottom = true;
            f
        };
        assert!(matches!(
            inconsistent_bottom.validate(),
            Err(TerminalFrameError::BottomState { .. })
        ));

        let inconsistent_top = {
            let mut f = frame(1, 1);
            f.scroll_offset = 0;
            f.at_bottom = false;
            f
        };
        assert!(matches!(
            inconsistent_top.validate(),
            Err(TerminalFrameError::BottomState { .. })
        ));
    }

    #[test]
    fn aggregate_glyph_budget_accepts_the_exact_bound_and_rejects_one_byte_over() {
        // Reaching the aggregate bound exactly requires the whole legal
        // grid — no smaller frame can spend 8 MiB of glyph — so the
        // boundary pair is built once and asserted in both directions.
        let (exact, over) = budget_boundary_frames();
        assert_eq!(exact.validate(), Ok(()));
        assert_eq!(
            over.validate(),
            Err(TerminalFrameError::GlyphBudget {
                max: MAX_TERMINAL_FRAME_GLYPH_BYTES
            })
        );
    }

    /// The exact-budget frame and its one-byte-over twin.
    ///
    /// Cluster lengths are chosen to maximize serialized length-prefix
    /// overhead: postcard spends two bytes on any length `>= 128`, so as
    /// many cells as the budget allows carry a 128-byte cluster and the
    /// rest carry the one-byte minimum, rather than packing the budget
    /// into a few maximal clusters that would each cost one prefix.
    fn budget_boundary_frames() -> (TerminalFrame, TerminalFrame) {
        /// Shortest cluster length postcard encodes with a two-byte
        /// length prefix.
        const WIDE_PREFIX_LEN: usize = 128;
        let rows = u32::from(MAX_TERMINAL_ROWS);
        let cols = u32::from(MAX_TERMINAL_COLS);
        let area = (rows * cols) as usize;
        assert_eq!(area, MAX_TERMINAL_VISIBLE_CELLS);

        let spare = MAX_TERMINAL_FRAME_GLYPH_BYTES - area;
        let wide_cells = spare / (WIDE_PREFIX_LEN - 1);
        let remainder = spare % (WIDE_PREFIX_LEN - 1);
        assert!(wide_cells + usize::from(remainder > 0) <= area);

        let wide = cluster_of_len(WIDE_PREFIX_LEN).into_boxed_slice();
        let single = cluster_of_len(1).into_boxed_slice();
        let mut cells = Vec::with_capacity(area);
        for index in 0..area {
            let glyph = if index < wide_cells {
                Glyph::Cluster(wide.clone())
            } else if index == wide_cells && remainder > 0 {
                Glyph::Cluster(cluster_of_len(remainder + 1).into_boxed_slice())
            } else {
                Glyph::Cluster(single.clone())
            };
            cells.push(cell_with(glyph));
        }

        let selection = (0..rows)
            .map(|row| TerminalSelectionSpan {
                row,
                start_col: 0,
                end_col: cols,
            })
            .collect();

        let exact = TerminalFrame {
            buffer_id: BufferId::from_raw(u64::MAX),
            size: CellSize::new(rows, cols),
            cells,
            cursor: Some(CellCoord::new(rows - 1, cols - 1)),
            title: Some("t".repeat(MAX_TERMINAL_METADATA_BYTES)),
            screen_generation: u64::MAX,
            selection,
            scroll_offset: u32::MAX,
            at_bottom: false,
            pid: u32::MAX,
            process: TerminalProcessState::Crashed("c".repeat(MAX_TERMINAL_METADATA_BYTES)),
        };

        let mut over = exact.clone();
        // One more byte of glyph, nothing else changed.
        let last = over.cells.len() - 1;
        over.cells[last] = cell_with(Glyph::Cluster(cluster_of_len(3).into_boxed_slice()));

        (exact, over)
    }

    #[test]
    fn maximum_legal_terminal_frame_encodes_below_the_transport_cap() {
        let (exact, _) = budget_boundary_frames();
        assert_eq!(exact.validate(), Ok(()));

        let mut glyph_bytes = 0usize;
        for cell in &exact.cells {
            glyph_bytes += match &cell.glyph {
                Glyph::Char(ch) => ch.len_utf8(),
                Glyph::Cluster(bytes) => bytes.len(),
                Glyph::Continuation => 0,
            };
        }
        assert_eq!(
            glyph_bytes, MAX_TERMINAL_FRAME_GLYPH_BYTES,
            "the measured fixture must spend the whole aggregate budget"
        );

        let msg = InstanceMessage::TerminalFrame(exact);
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        assert!(
            bytes.len() < MAX_FRAME_BYTES,
            "largest legal terminal frame encodes to {} bytes, at or above the \
             {MAX_FRAME_BYTES}-byte transport cap; the aggregate glyph bound no \
             longer keeps terminal traffic inside the existing transport limit",
            bytes.len()
        );
    }

    #[test]
    fn frames_round_trip_through_postcard() {
        let mut f = frame(3, 5);
        f.cells[0] = cell_with(Glyph::Char('\u{4e00}'));
        f.cells[1] = cell_with(Glyph::Continuation);
        f.cells[2] = cell_with(Glyph::Cluster(cluster_of_len(9).into_boxed_slice()));
        f.selection = vec![TerminalSelectionSpan {
            row: 2,
            start_col: 1,
            end_col: 4,
        }];
        f.scroll_offset = 6;
        f.at_bottom = false;
        f.process = TerminalProcessState::Exited(-1);
        assert_eq!(f.validate(), Ok(()));

        let msg = InstanceMessage::TerminalFrame(f);
        let bytes = postcard::to_allocvec(&msg).expect("encode");
        let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, msg);
    }
}
