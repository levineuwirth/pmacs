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

use unicode_width::UnicodeWidthChar;

use crate::buffer::{Buffer, BufferError};
use crate::cell::{Cell, CellCoord, CellGrid, Glyph, Style};
use crate::rope::{Edit, Position};
use crate::view::{DisplayCoord, View, Viewport};

// ---------------------------------------------------------------------------
// Tuning
// ---------------------------------------------------------------------------

/// Tab stop width in display columns. A `\t` advances to the next column
/// that is a multiple of this value.
const TAB_WIDTH: u32 = 8;

/// Line-prefix lengths up to this many bytes are decoded on the stack in
/// [`TextView::pos_to_display`]; longer prefixes fall back to a heap buffer.
const STACK_CAP: usize = 256;

/// Display width of `ch` when drawn starting at column `current_col`.
///
/// Tabs expand to the next [`TAB_WIDTH`]-aligned column, so they need the
/// running column to compute width. Everything else delegates to
/// [`UnicodeWidthChar`]: control characters return 0 (skipped by the
/// caller), printable characters return 1, wide characters return 2.
fn char_display_width(ch: char, current_col: u32) -> u32 {
    if ch == '\t' {
        TAB_WIDTH - (current_col % TAB_WIDTH)
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0) as u32
    }
}

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
}

impl View for TextView {
    fn on_edit(&mut self, buf: &Buffer, edit: &Edit) -> Result<(), BufferError> {
        let start_line = self.line_at_offset(edit.range.start);
        self.rebuild_lines_from(buf, start_line);
        Ok(())
    }

    fn pos_to_display(&self, buf: &Buffer, pos: Position) -> Option<DisplayCoord> {
        if pos > buf.len() {
            return None;
        }
        let row_idx = self.line_at_offset(pos);
        let line_start = self.line_offsets[row_idx];

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
        // If `pos` fell inside a multi-byte codepoint, keep only the bytes up to
        // the last complete codepoint. `valid_up_to()` gives that boundary in
        // one step, replacing the old pop-one-byte-and-revalidate loop. (Only
        // trailing bytes can be invalid here, since the slice is a prefix of
        // valid UTF-8 cut at `pos`.)
        let s = match std::str::from_utf8(bytes) {
            Ok(valid) => valid,
            Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap(),
        };
        let mut col: u32 = 0;
        for ch in s.chars() {
            col += char_display_width(ch, col);
        }
        Some(DisplayCoord::new(row_idx as u32, col))
    }

    fn display_to_pos(&self, buf: &Buffer, coord: DisplayCoord) -> Option<Position> {
        let row = coord.row as usize;
        if row >= self.line_count() {
            return None;
        }
        let line_start = self.line_offsets[row];
        let line_bytes = self.read_line_bytes(buf, row);
        let s = std::str::from_utf8(&line_bytes).ok()?;

        let mut walked_cols: u32 = 0;
        let mut walked_bytes: usize = 0;
        for (byte_idx, ch) in s.char_indices() {
            if walked_cols >= coord.col {
                walked_bytes = byte_idx;
                return Some(line_start + walked_bytes as u64);
            }
            walked_cols += char_display_width(ch, walked_cols);
            walked_bytes = byte_idx + ch.len_utf8();
        }
        // Past the line's last codepoint: clamp to the line's visible end.
        Some(line_start + walked_bytes as u64)
    }

    fn render(&mut self, buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        let start_line = self.line_at_offset(viewport.buffer_start);
        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let origin = viewport.cell_origin;

        for row_offset in 0..max_rows {
            let line = start_line + row_offset as usize;
            let cell_row = origin.row + row_offset;

            // Always clear the visible row first so previous content does
            // not bleed when the buffer shrinks past this row.
            for col in 0..max_cols {
                *cells.at(CellCoord::new(cell_row, origin.col + col)) = Cell::default();
            }
            if line >= self.line_count() {
                continue;
            }

            let line_bytes = self.read_line_bytes(buf, line);
            let Ok(s) = std::str::from_utf8(&line_bytes) else {
                continue;
            };

            let mut col: u32 = 0;
            for ch in s.chars() {
                if col >= max_cols {
                    break;
                }
                if ch == '\t' {
                    // Expand to the next TAB_WIDTH-aligned column with spaces.
                    let pad = char_display_width(ch, col);
                    for _ in 0..pad {
                        if col >= max_cols {
                            break;
                        }
                        let cell = cells.at(CellCoord::new(cell_row, origin.col + col));
                        cell.glyph = Glyph::Char(' ');
                        cell.style = Style::default();
                        cell.attachment = None;
                        col += 1;
                    }
                    continue;
                }
                let width = UnicodeWidthChar::width(ch).unwrap_or(0) as u32;
                if width == 0 {
                    // Combining mark or other zero-width control: M1.5
                    // skips; M2+ will attach to the previous cell as
                    // Glyph::Cluster.
                    continue;
                }
                let cell = cells.at(CellCoord::new(cell_row, origin.col + col));
                cell.glyph = Glyph::Char(ch);
                cell.style = Style::default();
                cell.attachment = None;
                if width == 2 && col + 1 < max_cols {
                    let cont = cells.at(CellCoord::new(cell_row, origin.col + col + 1));
                    cont.glyph = Glyph::Continuation;
                    cont.style = Style::default();
                    cont.attachment = None;
                }
                col += width;
            }
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
        assert_eq!(view.pos_to_display(&buf, 0), Some(DisplayCoord::new(0, 0)));
        assert_eq!(view.pos_to_display(&buf, 5), Some(DisplayCoord::new(0, 5)));
        assert_eq!(view.pos_to_display(&buf, 6), Some(DisplayCoord::new(1, 0)));
        assert_eq!(view.pos_to_display(&buf, 11), Some(DisplayCoord::new(1, 5)));
        assert_eq!(view.pos_to_display(&buf, 12), None);
    }

    #[test]
    fn ascii_display_to_pos_basic() {
        let (buf, view) = attached(b"hello\nworld");
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 0)), Some(0));
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 5)), Some(5));
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(1, 0)), Some(6));
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(1, 5)), Some(11));
        // Past the visible end of a line: clamps.
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 10)), Some(5));
        // Past the last line: None.
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(2, 0)), None);
    }

    #[test]
    fn multibyte_utf8_widths() {
        // "héllo" with é = U+00E9 (2 bytes UTF-8, 1 column wide).
        // Bytes: h(0x68) é(0xC3 0xA9) l(0x6C) l(0x6C) o(0x6F) -> 6 bytes total.
        let (buf, view) = attached("héllo".as_bytes());
        assert_eq!(buf.len(), 6);
        // Position 0 -> col 0
        assert_eq!(view.pos_to_display(&buf, 0), Some(DisplayCoord::new(0, 0)));
        // Position 1 (just after 'h') -> col 1
        assert_eq!(view.pos_to_display(&buf, 1), Some(DisplayCoord::new(0, 1)));
        // Position 3 (just after 'é') -> col 2
        assert_eq!(view.pos_to_display(&buf, 3), Some(DisplayCoord::new(0, 2)));
        // Position 6 (end) -> col 5
        assert_eq!(view.pos_to_display(&buf, 6), Some(DisplayCoord::new(0, 5)));
    }

    #[test]
    fn wide_cjk_widths() {
        // "中文" each codepoint is 3 bytes UTF-8 and 2 columns wide.
        let (buf, view) = attached("中文".as_bytes());
        assert_eq!(buf.len(), 6);
        assert_eq!(view.pos_to_display(&buf, 0), Some(DisplayCoord::new(0, 0)));
        assert_eq!(view.pos_to_display(&buf, 3), Some(DisplayCoord::new(0, 2)));
        assert_eq!(view.pos_to_display(&buf, 6), Some(DisplayCoord::new(0, 4)));
    }

    #[test]
    fn display_to_pos_jumps_over_wide_chars() {
        let (buf, view) = attached("中a".as_bytes());
        // "中" is 2 cols wide, 3 bytes. "a" is 1 col, 1 byte. Total 4 bytes, 3 cols.
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 0)), Some(0));
        // Asking for col 1 lands inside the wide char; we round to the next
        // codepoint boundary (col 2's start position).
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 2)), Some(3));
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 3)), Some(4));
    }

    proptest! {
        // Inverse property on ASCII text (incl. tabs): every codepoint-
        // aligned position round-trips through pos -> display -> pos.
        #[test]
        fn ascii_pos_display_inverse(content in r"[a-z\t\n]{0,200}", offset in 0usize..=200) {
            let buf = buf_with(content.as_bytes());
            let view = TextView::new(&buf);
            let pos = (offset as u64).min(buf.len());
            if let Some(disp) = view.pos_to_display(&buf, pos) {
                let back = view.display_to_pos(&buf, disp);
                prop_assert_eq!(back, Some(pos));
            }
        }
    }

    // ----- tabs -----

    #[test]
    fn tab_at_start_advances_to_column_8() {
        let (buf, view) = attached(b"\tx");
        // Position 0 (before tab) -> col 0
        assert_eq!(view.pos_to_display(&buf, 0), Some(DisplayCoord::new(0, 0)));
        // Position 1 (after tab) -> col 8
        assert_eq!(view.pos_to_display(&buf, 1), Some(DisplayCoord::new(0, 8)));
        // Position 2 (after 'x') -> col 9
        assert_eq!(view.pos_to_display(&buf, 2), Some(DisplayCoord::new(0, 9)));
    }

    #[test]
    fn tab_in_middle_pads_to_next_stop() {
        // "ab\tcd": after 'b' col is 2, tab pads to col 8, 'c' at col 8.
        let (buf, view) = attached(b"ab\tcd");
        assert_eq!(view.pos_to_display(&buf, 0), Some(DisplayCoord::new(0, 0)));
        assert_eq!(view.pos_to_display(&buf, 2), Some(DisplayCoord::new(0, 2)));
        assert_eq!(view.pos_to_display(&buf, 3), Some(DisplayCoord::new(0, 8)));
        assert_eq!(view.pos_to_display(&buf, 4), Some(DisplayCoord::new(0, 9)));
        assert_eq!(view.pos_to_display(&buf, 5), Some(DisplayCoord::new(0, 10)));
    }

    #[test]
    fn tab_aligned_input_advances_full_width() {
        // 8 chars then tab: tab pads from col 8 to col 16 (a full TAB_WIDTH).
        let (buf, view) = attached(b"01234567\tx");
        assert_eq!(view.pos_to_display(&buf, 8), Some(DisplayCoord::new(0, 8)));
        assert_eq!(view.pos_to_display(&buf, 9), Some(DisplayCoord::new(0, 16)));
    }

    #[test]
    fn display_to_pos_inside_tab_rounds_to_next_codepoint() {
        // "\tx": col 0..8 are the tab; col 5 (inside the tab) should
        // round to byte 1 (the start of 'x').
        let (buf, view) = attached(b"\tx");
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 0)), Some(0));
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 5)), Some(1));
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 8)), Some(1));
        assert_eq!(view.display_to_pos(&buf, DisplayCoord::new(0, 9)), Some(2));
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
            },
            &mut grid,
        );
        assert_eq!(storage[0].glyph, Glyph::Char('中'));
        assert_eq!(storage[1].glyph, Glyph::Continuation);
        assert_eq!(storage[2].glyph, Glyph::Char('a'));
    }
}
