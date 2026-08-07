// text_view.rs --- The plain-text View. ASCII, UTF-8, wide characters.

//! Plain-text view: maps rope bytes to display cells, line-aware.
//!
//! Implements the [`crate::view::View`] trait for the common case: a buffer
//! of UTF-8 text rendered as one row per line. Holds an incremental line
//! index keyed by byte offset; updates the index in place on each edit.
//!
//! # Width
//!
//! Display columns are computed via [`unicode_width`]. ASCII codepoints
//! are 1 column; CJK and similar wide characters are 2 columns (the second
//! column is filled with [`Glyph::Continuation`] in the cell grid).
//! Zero-width combining marks are skipped --- a future grapheme-aware pass
//! (M2+) will attach them to the preceding cell as a [`Glyph::Cluster`].
//!
//! # Threading
//!
//! Main thread only. The view is held inside a [`Buffer`], which is itself
//! main-only.

use crate::buffer::{Buffer, BufferError};
use crate::cell::{Cell, CellCoord, CellGrid, Glyph, Style};
use crate::display_width::{advance_char, valid_prefix_width};
use crate::rope::{Edit, Position};
use crate::view::{DisplayCoord, LayoutCtx, View, Viewport, WrapMode};

// ---------------------------------------------------------------------------
// Tuning
// ---------------------------------------------------------------------------

/// Line-prefix lengths up to this many bytes are decoded on the stack in
/// [`TextView::pos_to_display`]; longer prefixes fall back to a heap buffer.
const STACK_CAP: usize = 256;

/// Trailing marker painted after a collapsed region's head line (Arc 6
/// Stage 2, Q#FD13). One column wide, so it never disturbs layout.
pub const FOLD_ELLIPSIS: char = '…';

// ---------------------------------------------------------------------------
// TextView
// ---------------------------------------------------------------------------

/// View that renders a buffer as plain UTF-8 text, one buffer line per row.
pub struct TextView {
    /// Byte offsets of each line's first byte. `line_offsets[0] == 0`
    /// always; `line_offsets.last()` is the start of the final line.
    /// `line_offsets.len()` equals the number of lines (not the number of
    /// newlines: a trailing newline produces one extra empty line).
    line_offsets: Vec<u64>,
}

impl TextView {
    /// Construct a `TextView` for `buf`. Walks the buffer once to build
    /// the line index. Threading: main thread only.
    #[must_use]
    pub fn new(buf: &Buffer) -> Self {
        let mut v = Self {
            line_offsets: vec![0],
        };
        v.rebuild_lines_from(buf, 0);
        v
    }

    /// Number of lines in the buffer, as understood by this view.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.line_offsets.len()
    }

    /// First-byte offset of `line`, or `None` if `line` is out of range.
    #[must_use]
    pub fn line_offset(&self, line: usize) -> Option<u64> {
        self.line_offsets.get(line).copied()
    }

    /// Index of the line containing byte `offset`.
    ///
    /// For an offset equal to a line's first byte, returns that line. For an
    /// offset past the buffer end, returns the last line.
    #[must_use]
    pub fn line_at_offset(&self, offset: u64) -> usize {
        match self.line_offsets.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// Rebuild line offsets from `start_line` onward by re-scanning the
    /// buffer from `line_offsets[start_line]` to the buffer end. Lines
    /// before `start_line` are left untouched.
    fn rebuild_lines_from(&mut self, buf: &Buffer, start_line: usize) {
        let start_offset = self.line_offsets[start_line];
        self.line_offsets.truncate(start_line + 1);

        let rope = buf.snapshot_rope();
        let buf_end = rope.len();
        if start_offset >= buf_end {
            return;
        }

        let mut pos = start_offset;
        for chunk in rope.chunks(start_offset, buf_end) {
            for (i, b) in chunk.iter().enumerate() {
                if *b == b'\n' {
                    self.line_offsets.push(pos + i as u64 + 1);
                }
            }
            pos += chunk.len() as u64;
        }
    }

    /// Length in bytes of `line` (excluding the trailing `\n` if any). Used
    /// by render and by tests; `None` if `line` is out of range.
    #[must_use]
    pub fn line_len(&self, buf: &Buffer, line: usize) -> Option<u64> {
        let start = *self.line_offsets.get(line)?;
        let raw_end = self
            .line_offsets
            .get(line + 1)
            .copied()
            .unwrap_or(buf.len());
        // If a newline terminates this line, exclude it.
        if line + 1 < self.line_count() && raw_end > start {
            Some(raw_end - start - 1)
        } else {
            Some(raw_end - start)
        }
    }

    /// Read a line's bytes (excluding the trailing newline) into a fresh
    /// `Vec<u8>`. Lines are usually small; the copy is acceptable.
    fn read_line_bytes(&self, buf: &Buffer, line: usize) -> Vec<u8> {
        let Some(line_start) = self.line_offsets.get(line).copied() else {
            return Vec::new();
        };
        let Some(line_len) = self.line_len(buf, line) else {
            return Vec::new();
        };
        let mut out = vec![0u8; line_len as usize];
        if !out.is_empty() {
            buf.snapshot_rope()
                .slice(line_start, line_start + line_len, &mut out);
        }
        out
    }

    /// Which visual row of `line` holds byte `within` (relative to the
    /// line's start), at `max_cols` columns under character wrap.
    ///
    /// Total: any offset is legal, and one past the line's end lands on
    /// its last row. `max_cols == 0` yields row 0 rather than looping.
    /// Which visual row of `line` holds byte `within`, discarding the
    /// column. Thin wrapper over [`Self::place_of_byte`].
    fn row_of_byte(&self, buf: &Buffer, line: usize, within: u64, max_cols: u32) -> u32 {
        self.place_of_byte(buf, line, within, max_cols).0
    }

    /// Where byte `within` (relative to `line`'s start) sits under
    /// character wrap, as `(visual row, column)`.
    ///
    /// Total: any offset is legal, and one past the line's end lands
    /// just after its last character. A byte **inside** a multi-byte
    /// codepoint yields that codepoint's own place — the same
    /// projection `valid_prefix_width` performs on the unwrapped path,
    /// so the interior-byte contract is unchanged by wrapping.
    fn place_of_byte(&self, buf: &Buffer, line: usize, within: u64, max_cols: u32) -> (u32, u32) {
        if max_cols == 0 {
            return (0, 0);
        }
        let bytes = self.read_line_bytes(buf, line);
        let Ok(s) = std::str::from_utf8(&bytes) else {
            return (0, 0);
        };
        let (mut row, mut col, mut seen) = (0u32, 0u32, 0u64);
        for ch in s.chars() {
            if seen >= within {
                break;
            }
            let (start_row, start_col, end_row, end_col) =
                advance_wrapped(row, col, ch, max_cols, true);
            seen += ch.len_utf8() as u64;
            if seen > within {
                // `within` fell inside this character: project to the
                // character's own start, which is where it is drawn.
                return (start_row, start_col);
            }
            row = end_row;
            col = end_col;
        }
        // A position that lands exactly on a row boundary belongs to
        // column 0 of the NEXT row, not one past the end of the last
        // one (framing §7: the wrap position is owned downstream). The
        // downstream cell always exists; `(row, max_cols)` does not.
        if col >= max_cols {
            (row.saturating_add(1), 0)
        } else {
            (row, col)
        }
    }

    /// Byte offset (relative to `line`'s start) at visual row `sub_row`,
    /// column `col`, under character wrap — the inverse of
    /// [`Self::place_of_byte`].
    ///
    /// Rounds forward to the next character boundary when the column
    /// lands inside a wide glyph, matching the unwrapped
    /// `display_to_pos`. A row past the line's height clamps to the
    /// line's end.
    fn byte_at_place(
        &self,
        buf: &Buffer,
        line: usize,
        sub_row: u32,
        col: u32,
        max_cols: u32,
    ) -> u64 {
        let bytes = self.read_line_bytes(buf, line);
        let Ok(s) = std::str::from_utf8(&bytes) else {
            return 0;
        };
        if max_cols == 0 {
            return 0;
        }
        let (mut row, mut c, mut walked) = (0u32, 0u32, 0u64);
        for ch in s.chars() {
            let (start_row, start_col, end_row, end_col) =
                advance_wrapped(row, c, ch, max_cols, true);
            if start_row > sub_row || (start_row == sub_row && start_col >= col) {
                return walked;
            }
            walked += ch.len_utf8() as u64;
            row = end_row;
            c = end_col;
        }
        walked
    }

    /// Paint one source line and report how many grid rows it used.
    ///
    /// `first_row` is where the line begins in the viewport; `skip_rows`
    /// drops that many of the line's own leading visual rows, which is
    /// non-zero only for the first line when the byte anchor sits partway
    /// down it (framing Q#LL6).
    ///
    /// Under [`WrapMode::Truncate`] this returns 1 and walks exactly as
    /// the pre-wrap renderer did — the identity case the staging rests on.
    fn paint_line(
        &self,
        buf: &Buffer,
        line: usize,
        viewport: Viewport<'_>,
        cells: &mut CellGrid<'_>,
        place: LinePlacement,
    ) -> u32 {
        let LinePlacement {
            first_row,
            skip_rows,
            is_fold_head,
        } = place;
        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let origin = viewport.cell_origin;
        let wrapping = viewport.wrap == WrapMode::Wrap;

        // A zero-width content area has no cell to paint into. Bail
        // before the walk rather than inside it: under `Wrap` the first
        // `col >= max_cols` test is true immediately, so the walk would
        // advance a row and then index column 0 of a zero-width grid.
        // Reachable whenever the gutter consumes the window's width.
        if max_cols == 0 {
            return 1;
        }
        let line_bytes = self.read_line_bytes(buf, line);
        let Ok(s) = std::str::from_utf8(&line_bytes) else {
            return 1;
        };

        // `sub_row` counts this line's own visual rows. The grid row is
        // derived and is `None` while still skipping, or once past the
        // viewport's bottom.
        let grid_row = |sub: u32| -> Option<u32> {
            let r = first_row.checked_add(sub.checked_sub(skip_rows)?)?;
            (r < max_rows).then_some(origin.row + r)
        };
        let put = |cells: &mut CellGrid<'_>, sub: u32, col: u32, glyph: Glyph| {
            if let Some(row) = grid_row(sub) {
                let cell = cells.at(CellCoord::new(row, origin.col + col));
                cell.glyph = glyph;
                cell.style = Style::default();
                cell.attachment = None;
            }
        };

        let (mut sub_row, mut col) = (0u32, 0u32);
        for ch in s.chars() {
            if !wrapping && col >= max_cols {
                break;
            }
            let (start_row, start_col, end_row, end_col) =
                advance_wrapped(sub_row, col, ch, max_cols, wrapping);
            if start_row >= skip_rows && grid_row(start_row).is_none() {
                sub_row = start_row;
                break;
            }
            if ch == '\t' {
                for c in start_col..end_col {
                    put(cells, start_row, c, Glyph::Char(' '));
                }
            } else if end_col > start_col || start_row > sub_row {
                put(cells, start_row, start_col, Glyph::Char(ch));
                if end_col.saturating_sub(start_col) == 2 && start_col + 1 < max_cols {
                    put(cells, start_row, start_col + 1, Glyph::Continuation);
                }
            }
            sub_row = end_row;
            col = end_col;
        }

        // The head of a collapsed region carries a trailing ellipsis in
        // the CONTENT area (Q#FD13/FD20): the authoritative,
        // layout-neutral fold indicator, present in every gutter state,
        // clipped like any long line.
        if is_fold_head {
            for marker in [' ', FOLD_ELLIPSIS] {
                if col >= max_cols {
                    if !wrapping {
                        break;
                    }
                    sub_row += 1;
                    col = 0;
                    if sub_row >= skip_rows && grid_row(sub_row).is_none() {
                        break;
                    }
                }
                put(cells, sub_row, col, Glyph::Char(marker));
                col += 1;
            }
        }

        sub_row.saturating_sub(skip_rows) + 1
    }
}

/// Where a line goes in the viewport, for [`TextView::paint_line`].
#[derive(Copy, Clone, Debug)]
struct LinePlacement {
    /// Grid row (viewport-relative) where this line begins.
    first_row: u32,
    /// Leading visual rows of the line to drop, non-zero only for the
    /// first line when the byte anchor sits partway down it.
    skip_rows: u32,
    /// Whether the line heads a collapsed region and owes an ellipsis.
    is_fold_head: bool,
}

/// Where a character is drawn, and where it leaves the cursor, under
/// character wrap.
///
/// **This is the wrap rule and it exists exactly once.** Both
/// [`TextView::row_of_byte`] and [`TextView::paint_line`] go through it,
/// because they must agree perfectly: the first decides which visual row
/// the viewport's byte anchor sits on, the second decides which row the
/// text is drawn on. Two copies that drifted by one row would scroll the
/// buffer to a position it does not render — a defect with no local
/// symptom, and the exact shape this lane keeps finding.
///
/// Returns `(start_row, start_col, end_row, end_col)`: where the
/// character itself goes (it may already have moved to the next row),
/// and where the following character resumes.
///
/// With `wrapping == false` the row never advances and the column
/// arithmetic is the pre-wrap walk's own, unchanged.
fn advance_wrapped(
    row: u32,
    col: u32,
    ch: char,
    max_cols: u32,
    wrapping: bool,
) -> (u32, u32, u32, u32) {
    // The break belongs to the character that could not fit, so it is
    // taken before drawing rather than after the previous glyph.
    let (row, col) = if wrapping && col >= max_cols {
        (row.saturating_add(1), 0)
    } else {
        (row, col)
    };
    if ch == '\t' {
        // A tab fills to the row's end and stops; it never spans a wrap.
        // Column 0 of the next row is itself a tab stop, so alignment
        // survives the break rather than being approximated — carrying
        // the remaining pad across would put the next character at a
        // column the tab-stop arithmetic never chose.
        return (row, col, row, advance_char(col, ch).min(max_cols));
    }
    let width = advance_char(col, ch) - col;
    if width == 0 {
        // Combining mark or other zero-width control: M1.5 skips; M2+
        // will attach it to the previous cell as `Glyph::Cluster`.
        return (row, col, row, col);
    }
    // `max_cols >= 2` is the whole of the narrow-viewport policy: a
    // double-width glyph moves to the next row only when the next row
    // could actually hold it. At one column it never can, so moving
    // would insert a blank row before every wide character and paint it
    // clipped anyway — a single CJK glyph would render on row 1 with
    // row 0 left empty. Below two columns a wide glyph is clipped in
    // place, which is what `Truncate` does at the edge for the same
    // reason: there is no better row to move it to.
    if wrapping && width == 2 && max_cols >= 2 && col + 1 >= max_cols {
        // A double-width glyph with a single cell left moves to the next
        // row whole rather than being split across the break.
        //
        // `Truncate` keeps its existing behavior instead — lead cell
        // painted, continuation omitted. That is arguably worse, and
        // changing it is not this lane's to make: `Truncate` must stay
        // byte-identical.
        let r = row.saturating_add(1);
        return (r, 0, r, 2.min(max_cols));
    }
    (row, col, row, col + width)
}

impl View for TextView {
    fn on_edit(&mut self, buf: &Buffer, edit: &Edit) -> Result<(), BufferError> {
        let start_line = self.line_at_offset(edit.range.start);
        self.rebuild_lines_from(buf, start_line);
        Ok(())
    }

    fn pos_to_display(&self, buf: &Buffer, pos: Position, ctx: LayoutCtx) -> Option<DisplayCoord> {
        if pos > buf.len() {
            return None;
        }
        let row_idx = self.line_at_offset(pos);
        let line_start = self.line_offsets[row_idx];
        if ctx.wrapping() {
            let (sub_row, col) = self.place_of_byte(buf, row_idx, pos - line_start, ctx.cols);
            return Some(DisplayCoord::wrapped(row_idx as u32, sub_row, col));
        }
        // Everything below is the pre-wrap path, unchanged.

        // Slice [line_start, pos) and sum the display widths of any complete
        // codepoints inside. Bytes that look like UTF-8 continuation bytes
        // outside a complete codepoint are skipped (the position falls inside
        // a multi-byte codepoint; we treat the codepoint's column as the
        // answer, which means trimming the in-progress bytes).
        let take = (pos - line_start) as usize;
        if take == 0 {
            return Some(DisplayCoord::new(row_idx as u32, 0));
        }
        // Copy [line_start, pos) into a stack buffer for the common short-line
        // case, hitting the heap only for unusually long prefixes. This removes
        // the per-call allocation that previously ran on every cursor move.
        let mut stack_buf = [0u8; STACK_CAP];
        let mut heap_buf: Vec<u8>;
        let bytes: &mut [u8] = if take <= STACK_CAP {
            &mut stack_buf[..take]
        } else {
            heap_buf = vec![0u8; take];
            &mut heap_buf
        };
        buf.snapshot_rope().slice(line_start, pos, bytes);
        let col = valid_prefix_width(bytes);
        Some(DisplayCoord::new(row_idx as u32, col))
    }

    fn display_to_pos(
        &self,
        buf: &Buffer,
        coord: DisplayCoord,
        ctx: LayoutCtx,
    ) -> Option<Position> {
        let row = coord.row as usize;
        if row >= self.line_count() {
            return None;
        }
        let line_start = self.line_offsets[row];
        if ctx.wrapping() {
            let within = self.byte_at_place(buf, row, coord.sub_row, coord.col, ctx.cols);
            return Some(line_start + within);
        }
        // Everything below is the pre-wrap path, unchanged.
        let line_bytes = self.read_line_bytes(buf, row);
        let s = std::str::from_utf8(&line_bytes).ok()?;

        let mut walked_cols: u32 = 0;
        let mut walked_bytes: usize = 0;
        for (byte_idx, ch) in s.char_indices() {
            if walked_cols >= coord.col {
                walked_bytes = byte_idx;
                return Some(line_start + walked_bytes as u64);
            }
            walked_cols = advance_char(walked_cols, ch);
            walked_bytes = byte_idx + ch.len_utf8();
        }
        // Past the line's last codepoint: clamp to the line's visible end.
        Some(line_start + walked_bytes as u64)
    }

    fn render(&mut self, buf: &Buffer, viewport: Viewport<'_>, cells: &mut CellGrid<'_>) {
        // Arc 6 Stage 2 (Q#FD13): row `r` shows the `r`-th VISIBLE source
        // line at or after `view_top`; a collapsed region's lines are
        // skipped entirely and the rows below shift up. Without a fold
        // map this walk is the pre-folding `start_line + row_offset`
        // identity. Folding is deliberately not an overlay — overlays
        // repaint cells, they cannot delete rows.
        let folds = viewport.folds.filter(|m| !m.is_identity());
        // `view_top` is clamped backward before the frame, but a caller
        // that hands us a hidden start still gets its head.
        let start_line = {
            let raw = self.line_at_offset(viewport.buffer_start);
            folds.map_or(raw, |m| m.visible_head_of(raw))
        };
        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let origin = viewport.cell_origin;

        // Under `Wrap` one source line can own several rows, so the row
        // walk is no longer the line walk and `row_offset` is carried
        // rather than iterated. Clearing moves up front for the same
        // reason — a row's occupant is not known until the line reaching
        // it has been laid out — and every row is still blanked exactly
        // once, as before.
        for row_offset in 0..max_rows {
            let cell_row = origin.row + row_offset;
            for col in 0..max_cols {
                *cells.at(CellCoord::new(cell_row, origin.col + col)) = Cell::default();
            }
        }

        // Visual rows of the first line to skip. `buffer_start` may sit
        // partway down a wrapped line — the reason the anchor is a byte
        // and not a row index (framing Q#LL6). Always 0 under `Truncate`,
        // where a line owns exactly one row.
        let mut skip_rows = if viewport.wrap == WrapMode::Wrap {
            let line_start = self.line_offsets.get(start_line).copied().unwrap_or(0);
            self.row_of_byte(
                buf,
                start_line,
                viewport.buffer_start.saturating_sub(line_start),
                max_cols,
            )
        } else {
            0
        };

        let mut row_offset: u32 = 0;
        let mut line = start_line;
        while row_offset < max_rows && line < self.line_count() {
            let this_line = line;
            line = folds.map_or(this_line + 1, |m| m.next_visible(this_line));
            let used = self.paint_line(
                buf,
                this_line,
                viewport,
                cells,
                LinePlacement {
                    first_row: row_offset,
                    skip_rows,
                    is_fold_head: folds.is_some_and(|m| m.is_head(this_line)),
                },
            );
            skip_rows = 0;
            // A line always advances the row cursor, even when it painted
            // nothing (invalid UTF-8, an empty line): otherwise the walk
            // would re-enter the same grid row forever.
            row_offset += used.max(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{BufferId, EditOp};
    use crate::cell::CellSize;
    use crate::rope::Range;
    use crate::view::WrapMode;
    use proptest::prelude::*;

    fn buf_with(content: &[u8]) -> Buffer {
        Buffer::from_bytes(BufferId::next(), "test", content)
    }

    fn attached(content: &[u8]) -> (Buffer, TextView) {
        let buf = buf_with(content);
        let view = TextView::new(&buf);
        (buf, view)
    }

    // ----- line index -----

    #[test]
    fn empty_buffer_has_one_line() {
        let (_buf, view) = attached(b"");
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.line_offset(0), Some(0));
    }

    #[test]
    fn single_line_no_newline() {
        let (_buf, view) = attached(b"hello");
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.line_offset(0), Some(0));
    }

    #[test]
    fn three_lines() {
        let (buf, view) = attached(b"alpha\nbeta\ngamma");
        assert_eq!(view.line_count(), 3);
        assert_eq!(view.line_offset(0), Some(0));
        assert_eq!(view.line_offset(1), Some(6));
        assert_eq!(view.line_offset(2), Some(11));
        assert_eq!(view.line_len(&buf, 0), Some(5)); // "alpha"
        assert_eq!(view.line_len(&buf, 1), Some(4)); // "beta"
        assert_eq!(view.line_len(&buf, 2), Some(5)); // "gamma" (no trailing \n)
    }

    #[test]
    fn trailing_newline_creates_empty_line() {
        // "a\nb\n" -> three lines: "a", "b", "" (the third is empty after the
        // last newline).
        let (buf, view) = attached(b"a\nb\n");
        assert_eq!(view.line_count(), 3);
        assert_eq!(view.line_offset(2), Some(4));
        assert_eq!(view.line_len(&buf, 2), Some(0));
    }

    #[test]
    fn line_at_offset_walks_correctly() {
        let (_buf, view) = attached(b"abc\ndef\nghi");
        // line 0: bytes 0..4, line 1: 4..8, line 2: 8..11
        assert_eq!(view.line_at_offset(0), 0);
        assert_eq!(view.line_at_offset(3), 0);
        assert_eq!(view.line_at_offset(4), 1);
        assert_eq!(view.line_at_offset(7), 1);
        assert_eq!(view.line_at_offset(8), 2);
        assert_eq!(view.line_at_offset(11), 2);
    }

    // ----- incremental update on edit -----

    #[test]
    fn insert_within_a_line_keeps_other_offsets_pointer_stable() {
        let mut buf = buf_with(b"alpha\nbeta\ngamma");
        let mut view = TextView::new(&buf);
        // Capture the spine before the edit.
        let before: Vec<u64> = view.line_offsets.clone();

        // Insert one byte inside line 1 ("beta"); lines 0..=1 unchanged in
        // start offset, line 2 shifts by +1.
        let edit = buf
            .apply_edit(EditOp::Insert {
                pos: 8,
                bytes: b"X",
            })
            .unwrap();
        view.on_edit(&buf, &edit).unwrap();

        assert_eq!(view.line_offsets[0], before[0]);
        assert_eq!(view.line_offsets[1], before[1]);
        assert_eq!(view.line_offsets[2], before[2] + 1);
    }

    #[test]
    fn insert_a_newline_splits_a_line() {
        let mut buf = buf_with(b"abcdef");
        let mut view = TextView::new(&buf);
        let edit = buf
            .apply_edit(EditOp::Insert {
                pos: 3,
                bytes: b"\n",
            })
            .unwrap();
        view.on_edit(&buf, &edit).unwrap();
        assert_eq!(view.line_count(), 2);
        assert_eq!(view.line_offset(0), Some(0));
        assert_eq!(view.line_offset(1), Some(4));
    }

    #[test]
    fn delete_a_newline_merges_lines() {
        let mut buf = buf_with(b"abc\ndef");
        let mut view = TextView::new(&buf);
        let edit = buf
            .apply_edit(EditOp::Delete {
                range: Range::new(3, 4),
            })
            .unwrap();
        view.on_edit(&buf, &edit).unwrap();
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.line_offset(0), Some(0));
    }

    // ----- pos <-> display -----

    #[test]
    fn ascii_pos_to_display_basic() {
        let (buf, view) = attached(b"hello\nworld");
        assert_eq!(
            view.pos_to_display(&buf, 0, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 0))
        );
        assert_eq!(
            view.pos_to_display(&buf, 5, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 5))
        );
        assert_eq!(
            view.pos_to_display(&buf, 6, LayoutCtx::truncated()),
            Some(DisplayCoord::new(1, 0))
        );
        assert_eq!(
            view.pos_to_display(&buf, 11, LayoutCtx::truncated()),
            Some(DisplayCoord::new(1, 5))
        );
        assert_eq!(view.pos_to_display(&buf, 12, LayoutCtx::truncated()), None);
    }

    #[test]
    fn ascii_display_to_pos_basic() {
        let (buf, view) = attached(b"hello\nworld");
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 0), LayoutCtx::truncated()),
            Some(0)
        );
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 5), LayoutCtx::truncated()),
            Some(5)
        );
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(1, 0), LayoutCtx::truncated()),
            Some(6)
        );
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(1, 5), LayoutCtx::truncated()),
            Some(11)
        );
        // Past the visible end of a line: clamps.
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 10), LayoutCtx::truncated()),
            Some(5)
        );
        // Past the last line: None.
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(2, 0), LayoutCtx::truncated()),
            None
        );
    }

    #[test]
    fn multibyte_utf8_widths() {
        // "héllo" with é = U+00E9 (2 bytes UTF-8, 1 column wide).
        // Bytes: h(0x68) é(0xC3 0xA9) l(0x6C) l(0x6C) o(0x6F) -> 6 bytes total.
        let (buf, view) = attached("héllo".as_bytes());
        assert_eq!(buf.len(), 6);
        // Position 0 -> col 0
        assert_eq!(
            view.pos_to_display(&buf, 0, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 0))
        );
        // Position 1 (just after 'h') -> col 1
        assert_eq!(
            view.pos_to_display(&buf, 1, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 1))
        );
        // Position 3 (just after 'é') -> col 2
        assert_eq!(
            view.pos_to_display(&buf, 3, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 2))
        );
        // Position 6 (end) -> col 5
        assert_eq!(
            view.pos_to_display(&buf, 6, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 5))
        );
    }

    #[test]
    fn wide_cjk_widths() {
        // "中文" each codepoint is 3 bytes UTF-8 and 2 columns wide.
        let (buf, view) = attached("中文".as_bytes());
        assert_eq!(buf.len(), 6);
        assert_eq!(
            view.pos_to_display(&buf, 0, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 0))
        );
        assert_eq!(
            view.pos_to_display(&buf, 3, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 2))
        );
        assert_eq!(
            view.pos_to_display(&buf, 6, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 4))
        );
    }

    #[test]
    fn display_to_pos_jumps_over_wide_chars() {
        let (buf, view) = attached("中a".as_bytes());
        // "中" is 2 cols wide, 3 bytes. "a" is 1 col, 1 byte. Total 4 bytes, 3 cols.
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 0), LayoutCtx::truncated()),
            Some(0)
        );
        // Asking for col 1 lands inside the wide char; we round to the next
        // codepoint boundary (col 2's start position).
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 2), LayoutCtx::truncated()),
            Some(3)
        );
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 3), LayoutCtx::truncated()),
            Some(4)
        );
    }

    proptest! {
        // Inverse property on ASCII text (incl. tabs): every codepoint-
        // aligned position round-trips through pos -> display -> pos.
        #[test]
        fn ascii_pos_display_inverse(content in r"[a-z\t\n]{0,200}", offset in 0usize..=200) {
            let buf = buf_with(content.as_bytes());
            let view = TextView::new(&buf);
            let pos = (offset as u64).min(buf.len());
            if let Some(disp) = view.pos_to_display(&buf, pos, LayoutCtx::truncated()) {
                let back = view.display_to_pos(&buf, disp, LayoutCtx::truncated());
                prop_assert_eq!(back, Some(pos));
            }
        }
    }

    // ----- tabs -----

    #[test]
    fn tab_at_start_advances_to_column_8() {
        let (buf, view) = attached(b"\tx");
        // Position 0 (before tab) -> col 0
        assert_eq!(
            view.pos_to_display(&buf, 0, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 0))
        );
        // Position 1 (after tab) -> col 8
        assert_eq!(
            view.pos_to_display(&buf, 1, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 8))
        );
        // Position 2 (after 'x') -> col 9
        assert_eq!(
            view.pos_to_display(&buf, 2, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 9))
        );
    }

    #[test]
    fn tab_in_middle_pads_to_next_stop() {
        // "ab\tcd": after 'b' col is 2, tab pads to col 8, 'c' at col 8.
        let (buf, view) = attached(b"ab\tcd");
        assert_eq!(
            view.pos_to_display(&buf, 0, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 0))
        );
        assert_eq!(
            view.pos_to_display(&buf, 2, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 2))
        );
        assert_eq!(
            view.pos_to_display(&buf, 3, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 8))
        );
        assert_eq!(
            view.pos_to_display(&buf, 4, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 9))
        );
        assert_eq!(
            view.pos_to_display(&buf, 5, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 10))
        );
    }

    #[test]
    fn tab_aligned_input_advances_full_width() {
        // 8 chars then tab: the protocol tab stop advances col 8 to col 16.
        let (buf, view) = attached(b"01234567\tx");
        assert_eq!(
            view.pos_to_display(&buf, 8, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 8))
        );
        assert_eq!(
            view.pos_to_display(&buf, 9, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 16))
        );
    }

    #[test]
    fn display_to_pos_inside_tab_rounds_to_next_codepoint() {
        // "\tx": col 0..8 are the tab; col 5 (inside the tab) should
        // round to byte 1 (the start of 'x').
        let (buf, view) = attached(b"\tx");
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 0), LayoutCtx::truncated()),
            Some(0)
        );
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 5), LayoutCtx::truncated()),
            Some(1)
        );
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 8), LayoutCtx::truncated()),
            Some(1)
        );
        assert_eq!(
            view.display_to_pos(&buf, DisplayCoord::new(0, 9), LayoutCtx::truncated()),
            Some(2)
        );
    }

    /// Under `wrap`, `pos_to_display` reports the visual row **within**
    /// the source line, and `row` stays the source line.
    #[test]
    fn wrapped_coords_keep_row_as_the_source_line() {
        let (buf, view) = attached(b"abcdefghij\nzz");
        let ctx = LayoutCtx {
            cols: 4,
            wrap: WrapMode::Wrap,
        };
        // 'e' is byte 4: line 0, second visual row, column 0.
        assert_eq!(
            view.pos_to_display(&buf, 4, ctx),
            Some(DisplayCoord::wrapped(0, 1, 0))
        );
        // The second SOURCE line is still row 1, not a visual row index.
        assert_eq!(
            view.pos_to_display(&buf, 11, ctx),
            Some(DisplayCoord::wrapped(1, 0, 0)),
            "row is the source line; a redefinition would have made this 3"
        );
    }

    /// The wrap point belongs to column 0 of the next visual row, never
    /// one past the end of the previous one (framing §7).
    #[test]
    fn the_wrap_point_is_owned_by_the_next_row() {
        let (buf, view) = attached(b"abcdefgh");
        let ctx = LayoutCtx {
            cols: 4,
            wrap: WrapMode::Wrap,
        };
        assert_eq!(
            view.pos_to_display(&buf, 4, ctx),
            Some(DisplayCoord::wrapped(0, 1, 0)),
            "byte 4 is the start of row 1, not (row 0, col 4)"
        );
        // The two DISTINCT adjacent codepoints across the break map
        // distinctly: 'd' at (0,0,3) and 'e' at (0,1,0).
        assert_eq!(
            view.pos_to_display(&buf, 3, ctx),
            Some(DisplayCoord::wrapped(0, 0, 3))
        );
    }

    /// Round trip is identity on every cursor boundary of a wrapped
    /// line, and projection inside a multi-byte codepoint — the
    /// contract framing §7 settled.
    #[test]
    fn wrapped_round_trip_is_identity_on_boundaries() {
        let text = "abc\tde中fghijklmno";
        let (buf, view) = attached(text.as_bytes());
        for cols in [3_u32, 4, 5, 9] {
            let ctx = LayoutCtx {
                cols,
                wrap: WrapMode::Wrap,
            };
            for (byte, _) in text.char_indices() {
                let coord = view
                    .pos_to_display(&buf, byte as u64, ctx)
                    .expect("in range");
                let back = view.display_to_pos(&buf, coord, ctx).expect("in range");
                assert_eq!(
                    back, byte as u64,
                    "cols={cols}: boundary {byte} did not round trip (via {coord:?})"
                );
            }
        }
    }

    /// An interior byte projects to its codepoint's start, exactly as on
    /// the unwrapped path — wrapping does not change that contract.
    #[test]
    fn wrapped_interior_bytes_project_to_the_codepoint_start() {
        let text = "ab中cd";
        let (buf, view) = attached(text.as_bytes());
        let ctx = LayoutCtx {
            cols: 4,
            wrap: WrapMode::Wrap,
        };
        // '中' starts at byte 2 and is three bytes long.
        let at_start = view.pos_to_display(&buf, 2, ctx);
        for interior in [3_u64, 4] {
            assert_eq!(
                view.pos_to_display(&buf, interior, ctx),
                at_start,
                "byte {interior} is inside the codepoint starting at 2"
            );
        }
        // ...and the projection is idempotent.
        let coord = at_start.expect("in range");
        let back = view.display_to_pos(&buf, coord, ctx).expect("in range");
        assert_eq!(back, 2);
        assert_eq!(view.pos_to_display(&buf, back, ctx), at_start);
    }

    /// The identity control: with `truncated()` the mapping is exactly
    /// what it was before wrapping existed.
    #[test]
    fn truncate_coords_are_unchanged() {
        let (buf, view) = attached(b"abcdefghij");
        assert_eq!(
            view.pos_to_display(&buf, 6, LayoutCtx::truncated()),
            Some(DisplayCoord::new(0, 6)),
            "no sub_row, and the column is the whole prefix width"
        );
    }

    // -----------------------------------------------------------------
    // Line wrapping (QoL Stage 3, docs/long-lines-framing.md)
    // -----------------------------------------------------------------

    /// Render `text` into a `rows` x `cols` grid and return the glyph of
    /// every cell, row-major.
    fn render_grid(text: &[u8], rows: u32, cols: u32, wrap: WrapMode) -> Vec<Glyph> {
        render_grid_from(text, rows, cols, wrap, 0)
    }

    /// As [`render_grid`], but starting the viewport at byte `start` —
    /// which under `Wrap` may sit partway down a wrapped line.
    fn render_grid_from(
        text: &[u8],
        rows: u32,
        cols: u32,
        wrap: WrapMode,
        start: u64,
    ) -> Vec<Glyph> {
        let (buf, mut view) = attached(text);
        let n = (rows * cols) as usize;
        let mut storage = vec![Cell::default(); n];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: cols,
            size: CellSize::new(rows, cols),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: start,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(rows, cols),
                gutter_w: 0,
                folds: None,
                wrap,
            },
            &mut grid,
        );
        storage.iter().map(|c| c.glyph.clone()).collect()
    }

    fn row_text(glyphs: &[Glyph], cols: u32, row: u32) -> String {
        glyphs
            .iter()
            .skip((row * cols) as usize)
            .take(cols as usize)
            .map(|g| match g {
                Glyph::Char(c) => *c,
                _ => ' ',
            })
            .collect()
    }

    /// A viewport too narrow to hold a wide glyph must not insert a
    /// blank row before it.
    ///
    /// The wrap rule moves a double-width glyph to the next row when it
    /// will not fit in the cells left. At one column it never fits
    /// there either, so moving would leave row 0 empty and paint the
    /// glyph clipped on row 1 — worse than clipping it in place.
    #[test]
    fn a_one_column_viewport_does_not_shove_wide_glyphs_down() {
        let g = render_grid("中x".as_bytes(), 3, 1, WrapMode::Wrap);
        assert_eq!(
            g[0],
            Glyph::Char('中'),
            "the wide glyph belongs on row 0, clipped, not row 1"
        );
    }

    /// A zero-width content area paints nothing and does not panic.
    ///
    /// Reachable when the line-number gutter consumes the whole window.
    /// Under `Wrap` the walk's first `col >= max_cols` test is true
    /// immediately, so without an explicit bail it advances a row and
    /// then indexes column 0 of a zero-width grid.
    #[test]
    fn a_zero_width_viewport_paints_nothing() {
        let (buf, mut view) = attached(b"abc\ndef");
        let mut storage: Vec<Cell> = Vec::new();
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: 0,
            size: CellSize::new(4, 0),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(4, 0),
                gutter_w: 0,
                folds: None,
                wrap: WrapMode::Wrap,
            },
            &mut grid,
        );
        assert!(storage.is_empty(), "nothing to paint, and nothing painted");
    }

    /// The reported defect: a line wider than the window is readable.
    #[test]
    fn a_long_line_continues_on_the_following_rows() {
        let g = render_grid(b"abcdefghij", 4, 4, WrapMode::Wrap);
        assert_eq!(row_text(&g, 4, 0), "abcd");
        assert_eq!(row_text(&g, 4, 1), "efgh");
        assert_eq!(row_text(&g, 4, 2), "ij  ");
        assert_eq!(
            row_text(&g, 4, 3),
            "    ",
            "no fifth row of a ten-char line"
        );
    }

    /// The identity control: the same input under `Truncate` is exactly
    /// what the pre-wrap renderer produced.
    #[test]
    fn truncate_still_clips_at_the_edge() {
        let g = render_grid(b"abcdefghij", 4, 4, WrapMode::Truncate);
        assert_eq!(row_text(&g, 4, 0), "abcd");
        assert_eq!(
            row_text(&g, 4, 1),
            "    ",
            "one row per line, remainder clipped"
        );
    }

    /// Wrapping is per source line: a second line starts a new row
    /// rather than continuing the first.
    #[test]
    fn each_source_line_starts_its_own_row() {
        let g = render_grid(
            b"abcde
xy",
            4,
            4,
            WrapMode::Wrap,
        );
        assert_eq!(row_text(&g, 4, 0), "abcd");
        assert_eq!(row_text(&g, 4, 1), "e   ");
        assert_eq!(row_text(&g, 4, 2), "xy  ");
    }

    /// A double-width glyph with one cell left moves to the next row
    /// whole, rather than being split or half-painted.
    #[test]
    fn a_wide_char_that_does_not_fit_moves_down_whole() {
        // 3 columns: "ab" fills 0..2, leaving one cell — too narrow for
        // the 2-column CJK glyph.
        let g = render_grid("ab中".as_bytes(), 3, 3, WrapMode::Wrap);
        assert_eq!(row_text(&g, 3, 0), "ab ", "the odd cell stays blank");
        assert_eq!(
            g[3],
            Glyph::Char('中'),
            "the glyph starts the next row instead of straddling"
        );
        assert_eq!(g[4], Glyph::Continuation, "and keeps its continuation cell");
    }

    /// A tab fills to the row's end and stops; it never spans a wrap.
    /// Column 0 of the next row is itself a tab stop, so alignment
    /// survives the break.
    #[test]
    fn a_tab_fills_to_the_row_end_and_stops() {
        let g = render_grid(b"ab	z", 4, 4, WrapMode::Wrap);
        assert_eq!(row_text(&g, 4, 0), "ab  ", "tab pads to the edge");
        assert_eq!(
            row_text(&g, 4, 1),
            "z   ",
            "and the next char resumes at col 0"
        );
    }

    /// The byte anchor may sit partway down a wrapped line, which is
    /// why `view_top`'s sub-line component is a byte (framing Q#LL6).
    #[test]
    fn the_viewport_can_start_partway_down_a_wrapped_line() {
        // Byte 4 is 'e', the first character of the second visual row.
        let g = render_grid_from(b"abcdefghij", 2, 4, WrapMode::Wrap, 4);
        assert_eq!(row_text(&g, 4, 0), "efgh", "the first row is skipped");
        assert_eq!(row_text(&g, 4, 1), "ij  ");
    }

    /// `row_of_byte` and the painter must agree, or the viewport scrolls
    /// to a row the renderer does not draw. They share `advance_wrapped`
    /// precisely so this cannot drift; the witness pins it anyway.
    #[test]
    fn the_anchor_row_matches_where_the_text_is_painted() {
        let text = "ab	cd中efghij";
        let (buf, view) = attached(text.as_bytes());
        for cols in [3_u32, 4, 5, 8] {
            for (byte, _) in text.char_indices() {
                let row = view.row_of_byte(&buf, 0, byte as u64, cols);
                // Anchoring the viewport at that byte must put the
                // character on the viewport's FIRST row.
                let g = render_grid_from(text.as_bytes(), 3, cols, WrapMode::Wrap, byte as u64);
                let full = render_grid(text.as_bytes(), 12, cols, WrapMode::Wrap);
                let expect = row_text(&full, cols, row);
                assert_eq!(
                    row_text(&g, cols, 0),
                    expect,
                    "cols={cols} byte={byte}: anchor row {row} is not the row painted first"
                );
            }
        }
    }

    #[test]
    fn render_expands_tabs_to_spaces() {
        let (buf, mut view) = attached(b"\tx");
        let mut storage = vec![Cell::default(); 16];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: 16,
            size: CellSize::new(1, 16),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(1, 16),
                gutter_w: 0,
                folds: None,
                wrap: WrapMode::Truncate,
            },
            &mut grid,
        );
        // Cells 0..8 are spaces (the expanded tab), cell 8 is 'x'.
        for (i, cell) in storage.iter().enumerate().take(8) {
            assert_eq!(cell.glyph, Glyph::Char(' '), "cell {i} should be space");
        }
        assert_eq!(storage[8].glyph, Glyph::Char('x'));
    }

    #[test]
    fn render_tab_in_middle_pads_correctly() {
        // "ab\tcd" → cells: a, b, ' ', ' ', ' ', ' ', ' ', ' ', c, d
        let (buf, mut view) = attached(b"ab\tcd");
        let mut storage = vec![Cell::default(); 16];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: 16,
            size: CellSize::new(1, 16),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(1, 16),
                gutter_w: 0,
                folds: None,
                wrap: WrapMode::Truncate,
            },
            &mut grid,
        );
        assert_eq!(storage[0].glyph, Glyph::Char('a'));
        assert_eq!(storage[1].glyph, Glyph::Char('b'));
        for (i, cell) in storage.iter().enumerate().take(8).skip(2) {
            assert_eq!(cell.glyph, Glyph::Char(' '), "cell {i} should be space");
        }
        assert_eq!(storage[8].glyph, Glyph::Char('c'));
        assert_eq!(storage[9].glyph, Glyph::Char('d'));
    }

    // ----- render -----

    #[test]
    fn render_writes_ascii_glyphs() {
        let (buf, mut view) = attached(b"abc\nde");
        let mut storage = vec![Cell::default(); 5 * 5];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: 5,
            size: CellSize::new(5, 5),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(5, 5),
                gutter_w: 0,
                folds: None,
                wrap: WrapMode::Truncate,
            },
            &mut grid,
        );
        assert_eq!(storage[0].glyph, Glyph::Char('a'));
        assert_eq!(storage[1].glyph, Glyph::Char('b'));
        assert_eq!(storage[2].glyph, Glyph::Char('c'));
        // Past 'c': default (space) cell.
        assert_eq!(storage[3].glyph, Glyph::Char(' '));
        // Row 1: "de"
        assert_eq!(storage[5].glyph, Glyph::Char('d'));
        assert_eq!(storage[6].glyph, Glyph::Char('e'));
        // Row 2 onward: empty (buffer has only 2 lines).
        assert_eq!(storage[10].glyph, Glyph::Char(' '));
    }

    #[test]
    fn render_wide_char_writes_continuation() {
        let (buf, mut view) = attached("中a".as_bytes());
        let mut storage = vec![Cell::default(); 5];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: 5,
            size: CellSize::new(1, 5),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(1, 5),
                gutter_w: 0,
                folds: None,
                wrap: WrapMode::Truncate,
            },
            &mut grid,
        );
        assert_eq!(storage[0].glyph, Glyph::Char('中'));
        assert_eq!(storage[1].glyph, Glyph::Continuation);
        assert_eq!(storage[2].glyph, Glyph::Char('a'));
    }
}
