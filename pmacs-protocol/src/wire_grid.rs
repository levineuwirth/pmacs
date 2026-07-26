//! Shared cell-grid validation for every wire message that carries a
//! rectangular grid of [`Cell`]s.
//!
//! Bottom-panel Stage 2B (Q#BP15) factors this out of
//! [`crate::terminal`], which was the only such message until
//! [`crate::panel::PanelFrame`] arrived. The split follows the boundary
//! the framing names:
//!
//! - **Shared** — the checked area, the visible-cell bound, the cell
//!   count, cursor bounds, glyph legality, wide-continuation topology,
//!   the aggregate glyph-byte budget, and the attachment rejection.
//! - **Terminal-only** — the 512 per-axis PTY caps, title/process
//!   metadata, selection spans, and the `at_bottom == (scroll_offset ==
//!   0)` coupling.
//!
//! The per-axis caps are a [`WireGridLimits`] parameter rather than a
//! constant precisely because a panel does not inherit them: a 4K
//! surface at a small font is legitimately wider than 512 columns, and
//! the area bound is what keeps the encoding inside the transport
//! budget.
//!
//! The attachment rejection is deliberately **shared**, not
//! terminal-only, even though its terminal-side message reads "which
//! terminals never use". Panels render no attachments either, so
//! rejecting them here fails closed for both; classifying it as
//! terminal-only would let a panel ship a cell no frontend can paint.

use crate::cell::{Cell, CellCoord, CellSize, Glyph};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Aggregate glyph-byte ceiling shared by every wire grid.
///
/// A grid at the visible-cell bound where every cell carries a maximum
/// cluster would exceed the transport frame limit; this keeps the
/// encoded size bounded independently of the per-cell rule.
pub const MAX_WIRE_GRID_GLYPH_BYTES: usize = 8 * 1024 * 1024;

/// Per-cell grapheme-cluster byte ceiling shared by every wire grid.
pub const MAX_WIRE_GRID_GRAPHEME_BYTES: usize = 256;

/// Visible-cell ceiling shared by every wire grid.
///
/// This is the transport-safety bound, not a per-message policy: it is
/// what keeps `rows * cols * per-cell` inside the transport frame limit,
/// so both the terminal and the panel answer to it even though they
/// carry different per-axis caps.
pub const MAX_WIRE_GRID_VISIBLE_CELLS: usize = 262_144;

/// Bounds a particular wire grid enforces.
///
/// `max_rows` / `max_cols` are per-message policy. `max_visible_cells`
/// is the shared area bound and is what actually keeps the encoding
/// inside the transport budget.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct WireGridLimits {
    /// Inclusive row ceiling.
    pub max_rows: u32,
    /// Inclusive column ceiling.
    pub max_cols: u32,
    /// Inclusive `rows * cols` ceiling.
    pub max_visible_cells: usize,
    /// Inclusive aggregate glyph-byte ceiling.
    pub max_glyph_bytes: usize,
}

/// Why a wire grid is not structurally valid.
///
/// Callers map these onto their own message-specific error types so
/// existing wire errors keep their exact variants and text.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WireGridError {
    /// Rows or columns are zero or above this grid's bounds.
    Size {
        /// Declared rows.
        rows: u32,
        /// Declared columns.
        cols: u32,
        /// Row bound in force.
        max_rows: u32,
        /// Column bound in force.
        max_cols: u32,
    },
    /// The checked area exceeds the visible-cell bound.
    Area {
        /// Checked `rows * cols`.
        area: usize,
        /// Bound in force.
        max: usize,
    },
    /// `cells.len()` disagrees with the declared area.
    CellCount {
        /// Declared area.
        expected: usize,
        /// Supplied cell count.
        actual: usize,
    },
    /// The cursor lies outside the declared grid.
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
    /// A cell's glyph is not legal in a wire grid.
    Glyph {
        /// Row-major cell index.
        index: usize,
        /// Why the glyph failed.
        reason: &'static str,
    },
    /// A cell carries a frontend attachment, which no wire grid uses.
    Attachment {
        /// Row-major cell index.
        index: usize,
    },
    /// Aggregate glyph bytes exceed the budget.
    GlyphBudget {
        /// Bound in force.
        max: usize,
    },
}

/// Declared cell area, checked against this grid's bounds.
///
/// Separate from [`validate_wire_grid`] because callers need the area
/// before they have cells to check against it.
pub fn checked_area(size: CellSize, limits: WireGridLimits) -> Result<usize, WireGridError> {
    let rows = size.rows;
    let cols = size.cols;
    if rows == 0 || cols == 0 || rows > limits.max_rows || cols > limits.max_cols {
        return Err(WireGridError::Size {
            rows,
            cols,
            max_rows: limits.max_rows,
            max_cols: limits.max_cols,
        });
    }
    // `checked_mul` rather than a bound-derived assumption: a panel's
    // axis ceilings are large enough that the product genuinely can
    // overflow, which the terminal's 512x512 could not.
    let area = rows
        .checked_mul(cols)
        .and_then(|area| usize::try_from(area).ok())
        .ok_or(WireGridError::Area {
            area: usize::MAX,
            max: limits.max_visible_cells,
        })?;
    if area > limits.max_visible_cells {
        return Err(WireGridError::Area {
            area,
            max: limits.max_visible_cells,
        });
    }
    Ok(area)
}

/// Check every structural rule shared by wire grids.
///
/// Pure: a rejected grid mutates nothing, so callers get atomic
/// rejection for free.
pub fn validate_wire_grid(
    size: CellSize,
    cells: &[Cell],
    cursor: Option<CellCoord>,
    limits: WireGridLimits,
) -> Result<(), WireGridError> {
    let area = checked_area(size, limits)?;
    if cells.len() != area {
        return Err(WireGridError::CellCount {
            expected: area,
            actual: cells.len(),
        });
    }
    if let Some(cursor) = cursor
        && (cursor.row >= size.rows || cursor.col >= size.cols)
    {
        return Err(WireGridError::Cursor {
            row: cursor.row,
            col: cursor.col,
            rows: size.rows,
            cols: size.cols,
        });
    }
    validate_cells(size, cells, limits)
}

/// Glyph legality, wide-continuation topology, and the glyph budget.
fn validate_cells(
    size: CellSize,
    cells: &[Cell],
    limits: WireGridLimits,
) -> Result<(), WireGridError> {
    let cols = size.cols as usize;
    let mut glyph_bytes = 0usize;
    // Columns still owed to the preceding wide lead on this row.
    let mut pending_continuation = false;
    for (index, cell) in cells.iter().enumerate() {
        if cell.attachment.is_some() {
            return Err(WireGridError::Attachment { index });
        }
        let col = index % cols;
        if col == 0 && pending_continuation {
            // A wide lead in the final column would have to be completed
            // on the next row, which is not a footprint a cell grid can
            // express.
            return Err(WireGridError::Glyph {
                index: index - 1,
                reason: "wide glyph has no continuation column on its row",
            });
        }
        match &cell.glyph {
            Glyph::Continuation => {
                if !pending_continuation {
                    return Err(WireGridError::Glyph {
                        index,
                        reason: "continuation without a preceding wide glyph",
                    });
                }
                pending_continuation = false;
            }
            Glyph::Char(ch) => {
                if pending_continuation {
                    return Err(WireGridError::Glyph {
                        index,
                        reason: "wide glyph is not followed by its continuation",
                    });
                }
                let width = char_display_width(*ch).ok_or(WireGridError::Glyph {
                    index,
                    reason: "glyph is a control or zero-width character",
                })?;
                glyph_bytes = add_glyph_bytes(glyph_bytes, ch.len_utf8(), limits)?;
                pending_continuation = width == 2;
            }
            Glyph::Cluster(bytes) => {
                if pending_continuation {
                    return Err(WireGridError::Glyph {
                        index,
                        reason: "wide glyph is not followed by its continuation",
                    });
                }
                let width = cluster_display_width(bytes, index)?;
                glyph_bytes = add_glyph_bytes(glyph_bytes, bytes.len(), limits)?;
                pending_continuation = width == 2;
            }
        }
    }
    if pending_continuation {
        return Err(WireGridError::Glyph {
            index: cells.len() - 1,
            reason: "wide glyph has no continuation column on its row",
        });
    }
    Ok(())
}

/// Column width of a leading `Char` glyph, or `None` when it cannot lead.
pub(crate) fn char_display_width(ch: char) -> Option<usize> {
    if ch.is_control() {
        return None;
    }
    match UnicodeWidthChar::width(ch) {
        Some(1) => Some(1),
        Some(2) => Some(2),
        _ => None,
    }
}

/// Column width of a leading `Cluster` glyph.
///
/// Width is clamped into `1..=2` exactly as the terminal screen clamps it
/// when it writes the cluster: a base plus combining marks may measure
/// wider than two columns, and the screen occupies two. Clamping in one
/// place and measuring in another is how a frame that renders correctly
/// gets rejected on the wire.
fn cluster_display_width(bytes: &[u8], index: usize) -> Result<usize, WireGridError> {
    if bytes.is_empty() {
        return Err(WireGridError::Glyph {
            index,
            reason: "cluster is empty",
        });
    }
    if bytes.len() > MAX_WIRE_GRID_GRAPHEME_BYTES {
        return Err(WireGridError::Glyph {
            index,
            reason: "cluster exceeds the per-cluster byte limit",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| WireGridError::Glyph {
        index,
        reason: "cluster is not valid UTF-8",
    })?;
    if text.chars().any(char::is_control) {
        return Err(WireGridError::Glyph {
            index,
            reason: "cluster carries a control character",
        });
    }
    let width = UnicodeWidthStr::width(text);
    if width == 0 {
        return Err(WireGridError::Glyph {
            index,
            reason: "cluster occupies no columns",
        });
    }
    Ok(width.min(2))
}

/// Accumulate glyph bytes against the aggregate budget.
fn add_glyph_bytes(
    total: usize,
    add: usize,
    limits: WireGridLimits,
) -> Result<usize, WireGridError> {
    let next = total.checked_add(add).ok_or(WireGridError::GlyphBudget {
        max: limits.max_glyph_bytes,
    })?;
    if next > limits.max_glyph_bytes {
        return Err(WireGridError::GlyphBudget {
            max: limits.max_glyph_bytes,
        });
    }
    Ok(next)
}
