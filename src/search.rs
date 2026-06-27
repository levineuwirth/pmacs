// search.rs --- in-buffer incremental search store + match primitive.

//! Per-buffer in-buffer search state and the smart-case substring
//! matcher that fills it. Mirrors [`crate::diag::DiagnosticStore`]:
//! a cheaply-cloneable shared store written by the search session /
//! navigation commands and read by the decorations producer
//! ([`crate::semantic_render`]) and the TUI [`SearchView`]
//! (`crate::search_view`-equivalent — lives here for v1).
//!
//! # Why per-buffer (not per-window)
//!
//! Matches are a function of buffer *content*, so they live per
//! buffer, like diagnostics — not per window like the selection. The
//! active-match index is navigation state kept on the store; v1
//! accepts that two windows showing the same buffer share the active
//! highlight (the same tradeoff diagnostics make).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pmacs_protocol::ByteRange;

use crate::buffer::BufferId;

/// One buffer's search state: the resolved query, its matches (byte
/// ranges, ascending and non-overlapping), and the active index.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    query: String,
    matches: Vec<ByteRange>,
    active: usize,
}

impl SearchState {
    /// The query these matches were computed for.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// All matches, ascending by start.
    #[must_use]
    pub fn matches(&self) -> &[ByteRange] {
        &self.matches
    }

    /// The active match's range, or `None` when there are no matches.
    #[must_use]
    pub fn active_match(&self) -> Option<ByteRange> {
        self.matches.get(self.active).copied()
    }

    /// The active match's index, or `None` when there are no matches.
    #[must_use]
    pub fn active_index(&self) -> Option<usize> {
        (!self.matches.is_empty()).then_some(self.active)
    }

    /// Number of matches.
    #[must_use]
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// True when there are no matches.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }
}

/// Per-buffer in-buffer-search store. Mirrors
/// [`crate::diag::DiagnosticStore`]: the search session and the
/// `search.next` / `search.previous` commands write it; the
/// decorations producer and the TUI search overlay read it.
///
/// **Staleness (M11.8 model).** An edit marks the buffer's matches
/// stale: their byte positions describe pre-edit text, so the
/// producer / overlay suppress them until the next [`Self::set`]
/// re-runs the search against the current content.
#[derive(Default)]
pub struct SearchStore {
    by_buffer: HashMap<BufferId, SearchState>,
    stale: HashSet<BufferId>,
}

impl SearchStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace `buffer_id`'s query + matches, clearing the stale flag.
    /// An empty query or no matches drops the entry entirely. The
    /// active index is preserved across re-search (clamped into the
    /// new match set) so live typing doesn't reset the focused match.
    pub fn set(&mut self, buffer_id: BufferId, query: impl Into<String>, matches: Vec<ByteRange>) {
        let query = query.into();
        self.stale.remove(&buffer_id);
        if query.is_empty() || matches.is_empty() {
            self.by_buffer.remove(&buffer_id);
            return;
        }
        let active = self
            .by_buffer
            .get(&buffer_id)
            .map_or(0, |s| s.active)
            .min(matches.len() - 1);
        self.by_buffer.insert(
            buffer_id,
            SearchState {
                query,
                matches,
                active,
            },
        );
    }

    /// Drop a buffer's search state (e.g. on cancel / accept-and-end).
    pub fn clear(&mut self, buffer_id: BufferId) {
        self.by_buffer.remove(&buffer_id);
        self.stale.remove(&buffer_id);
    }

    /// The buffer's search state, or `None` if it has none.
    #[must_use]
    pub fn for_buffer(&self, buffer_id: BufferId) -> Option<&SearchState> {
        self.by_buffer.get(&buffer_id)
    }

    /// Mark a buffer's matches stale (document edited since the search
    /// ran). No-op for a buffer with no search state.
    pub fn mark_stale(&mut self, buffer_id: BufferId) {
        if self.by_buffer.contains_key(&buffer_id) {
            self.stale.insert(buffer_id);
        }
    }

    /// `true` iff the buffer's matches are stale.
    #[must_use]
    pub fn is_stale(&self, buffer_id: BufferId) -> bool {
        self.stale.contains(&buffer_id)
    }

    /// Step the active match forward or backward, wrapping. Returns
    /// the new active match's range, or `None` when the buffer has no
    /// matches.
    pub fn step(&mut self, buffer_id: BufferId, forward: bool) -> Option<ByteRange> {
        let s = self.by_buffer.get_mut(&buffer_id)?;
        let n = s.matches.len();
        if n == 0 {
            return None;
        }
        s.active = if forward {
            (s.active + 1) % n
        } else {
            (s.active + n - 1) % n
        };
        s.matches.get(s.active).copied()
    }

    /// Focus the first match at or after `byte` (wrapping to the first
    /// match when all matches precede `byte`). Used on entry to focus
    /// the match nearest the cursor. Returns the focused range.
    pub fn focus_from(&mut self, buffer_id: BufferId, byte: u64) -> Option<ByteRange> {
        let s = self.by_buffer.get_mut(&buffer_id)?;
        if s.matches.is_empty() {
            return None;
        }
        let idx = s.matches.iter().position(|m| m.start >= byte).unwrap_or(0);
        s.active = idx;
        s.matches.get(idx).copied()
    }
}

/// Cheaply-cloneable shared handle, mirroring
/// [`crate::diag::SharedDiagStore`].
pub type SharedSearchStore = Arc<Mutex<SearchStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedSearchStore {
    Arc::new(Mutex::new(SearchStore::new()))
}

/// Smart-case substring search over `haystack` bytes for `query`:
/// case-insensitive unless `query` contains an uppercase character,
/// in which case it is case-sensitive. Returns non-overlapping
/// matches as byte ranges, ascending. An empty query yields no
/// matches.
///
/// **ASCII case folding.** Case-insensitivity folds ASCII letters
/// only (`eq_ignore_ascii_case`), which keeps byte offsets exact
/// (ASCII upper/lower are the same byte length). Non-ASCII bytes
/// compare exactly, so a query with non-ASCII letters matches those
/// case-sensitively — acceptable for v1 (code search is
/// overwhelmingly ASCII); a full-Unicode fold is a later refinement.
#[must_use]
pub fn find_all(haystack: &[u8], query: &str) -> Vec<ByteRange> {
    let q = query.as_bytes();
    if q.is_empty() || haystack.len() < q.len() {
        return Vec::new();
    }
    let case_sensitive = query.chars().any(char::is_uppercase);
    let matches_at = |i: usize| {
        haystack[i..i + q.len()].iter().zip(q).all(|(&h, &needle)| {
            if case_sensitive {
                h == needle
            } else {
                h.eq_ignore_ascii_case(&needle)
            }
        })
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= haystack.len() {
        if matches_at(i) {
            out.push(ByteRange {
                start: i as u64,
                end: (i + q.len()) as u64,
            });
            i += q.len(); // non-overlapping
        } else {
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// TUI view
// ---------------------------------------------------------------------------

use crate::buffer::Buffer;
use crate::cell::{CellCoord, CellGrid, Color, Style};
use crate::overlay::merge_styles;
use crate::view::{View, Viewport};

/// Background style applied to a non-active search match (Q#SR4) —
/// black-on-yellow so the highlighted text reads on any theme.
fn match_style() -> Style {
    Style {
        bg: Color::Indexed(3), // yellow
        fg: Color::Indexed(0), // black
        ..Style::default()
    }
}

/// Background style for the active match — brighter yellow so it
/// stands out from the lazy matches as you step through.
fn active_match_style() -> Style {
    Style {
        bg: Color::Indexed(11), // bright yellow
        fg: Color::Indexed(0),
        ..Style::default()
    }
}

/// TUI overlay that washes search matches in the visible region,
/// mirroring [`crate::diag::DiagnosticView`]: snapshot the store under
/// the lock, skip while stale, map each match's byte range to display
/// columns, and merge the highlight style into those cells. Matches
/// are single-line (the minibuffer query carries no newline), so each
/// maps to one row.
pub struct SearchView {
    store: SharedSearchStore,
}

impl SearchView {
    /// Construct a view reading `store` for whichever buffer the host
    /// window is showing. The view keys on the *rendered* buffer
    /// ([`Buffer::id`]) rather than a fixed id, so a single attached
    /// instance keeps highlighting correctly even if the window
    /// switches buffers (the store is per-buffer; a buffer with no
    /// search entry simply paints nothing).
    #[must_use]
    pub fn new(store: SharedSearchStore) -> Self {
        Self { store }
    }
}

impl View for SearchView {
    fn kind(&self) -> &'static str {
        "search"
    }

    fn render(&mut self, buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        let buffer_id = buf.id();
        // Snapshot the matches under the lock, release immediately
        // (same discipline as DiagnosticView).
        let (matches, active): (Vec<ByteRange>, Option<ByteRange>) = {
            let guard = self.store.lock().expect("search store mutex poisoned");
            if guard.is_stale(buffer_id) {
                return;
            }
            match guard.for_buffer(buffer_id) {
                Some(s) => (s.matches().to_vec(), s.active_match()),
                None => return,
            }
        };
        if matches.is_empty() {
            return;
        }

        let source: Vec<u8> = {
            let mut bytes = vec![0u8; buf.len() as usize];
            if !bytes.is_empty() {
                buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
            }
            bytes
        };
        let line_offsets = crate::diag::compute_line_offsets(&source);
        let start_line_buf =
            crate::diag::line_at_offset(&line_offsets, viewport.buffer_start as u32);
        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let cell_origin = viewport.cell_origin;

        for m in &matches {
            let line = crate::diag::line_at_offset(&line_offsets, m.start as u32);
            if line < start_line_buf {
                continue;
            }
            let row_offset = line - start_line_buf;
            if row_offset >= max_rows {
                break;
            }
            let line_start = line_offsets[line as usize];
            let line_end = line_offsets
                .get(line as usize + 1)
                .copied()
                .unwrap_or(source.len() as u32);
            let line_end_no_nl = if line_end > line_start
                && source.get(line_end as usize - 1).copied() == Some(b'\n')
            {
                line_end - 1
            } else {
                line_end
            };
            let line_bytes = &source[line_start as usize..line_end_no_nl as usize];
            let within_start = (m.start as u32).saturating_sub(line_start) as usize;
            let within_end = (m.end as u32).saturating_sub(line_start) as usize;
            let (start_col, end_col) =
                crate::diag::byte_range_to_display_cols(line_bytes, within_start, within_end);
            if end_col <= start_col {
                continue;
            }
            let style = if Some(*m) == active {
                active_match_style()
            } else {
                match_style()
            };
            let cell_row = cell_origin.row + row_offset;
            let clamped_start = start_col.min(max_cols);
            let clamped_end = end_col.min(max_cols);
            for col in clamped_start..clamped_end {
                let cell = cells.at(CellCoord::new(cell_row, cell_origin.col + col));
                cell.style = merge_styles(cell.style, style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: u64, end: u64) -> ByteRange {
        ByteRange { start, end }
    }

    #[test]
    fn find_all_is_case_insensitive_for_lowercase_queries() {
        // "fn" matches "fn" and "Fn" and "FN" when the query is all
        // lowercase (smart-case).
        let hay = b"fn Fn FN fnord";
        assert_eq!(
            find_all(hay, "fn"),
            vec![r(0, 2), r(3, 5), r(6, 8), r(9, 11)]
        );
    }

    #[test]
    fn find_all_is_case_sensitive_when_query_has_uppercase() {
        // "Fn" (has uppercase) matches only "Fn".
        let hay = b"fn Fn FN";
        assert_eq!(find_all(hay, "Fn"), vec![r(3, 5)]);
    }

    #[test]
    fn find_all_matches_are_non_overlapping() {
        // "aa" in "aaaa": [0,2) and [2,4), not [1,3).
        assert_eq!(find_all(b"aaaa", "aa"), vec![r(0, 2), r(2, 4)]);
    }

    #[test]
    fn find_all_empty_query_and_too_short_haystack() {
        assert!(find_all(b"hello", "").is_empty());
        assert!(find_all(b"hi", "hello").is_empty());
        assert!(find_all(b"", "x").is_empty());
    }

    #[test]
    fn store_set_clamps_active_and_clears_on_empty() {
        let mut s = SearchStore::new();
        let bid = BufferId::next();
        s.set(bid, "x", vec![r(0, 1), r(4, 5), r(8, 9)]);
        // Step to the last match, then a shorter re-search clamps the
        // active index instead of pointing past the end.
        s.step(bid, true);
        s.step(bid, true);
        assert_eq!(s.for_buffer(bid).unwrap().active_index(), Some(2));
        s.set(bid, "x", vec![r(0, 1)]);
        assert_eq!(s.for_buffer(bid).unwrap().active_index(), Some(0));
        // Empty query drops the entry.
        s.set(bid, "", vec![]);
        assert!(s.for_buffer(bid).is_none());
    }

    #[test]
    fn store_step_wraps_both_directions() {
        let mut s = SearchStore::new();
        let bid = BufferId::next();
        s.set(bid, "x", vec![r(0, 1), r(4, 5), r(8, 9)]);
        assert_eq!(s.step(bid, true), Some(r(4, 5)));
        assert_eq!(s.step(bid, true), Some(r(8, 9)));
        assert_eq!(s.step(bid, true), Some(r(0, 1)), "wraps to first");
        assert_eq!(s.step(bid, false), Some(r(8, 9)), "wraps back to last");
    }

    #[test]
    fn store_focus_from_picks_match_at_or_after_cursor() {
        let mut s = SearchStore::new();
        let bid = BufferId::next();
        s.set(bid, "x", vec![r(2, 3), r(10, 11), r(20, 21)]);
        assert_eq!(s.focus_from(bid, 5), Some(r(10, 11)));
        assert_eq!(s.for_buffer(bid).unwrap().active_index(), Some(1));
        // Past the last match → wrap to the first.
        assert_eq!(s.focus_from(bid, 99), Some(r(2, 3)));
    }

    #[test]
    fn search_view_washes_matches_and_distinguishes_active() {
        use crate::cell::{Cell, CellGrid, CellSize};
        use crate::view::Viewport;

        let store = make_shared_store();
        let bid = BufferId::next();
        let mut buf = Buffer::new(bid, "test.txt");
        buf.apply_edit(crate::buffer::EditOp::Insert {
            pos: 0,
            bytes: b"lo lo lo\n",
        })
        .expect("seed");
        store
            .lock()
            .unwrap()
            .set(bid, "lo", find_all(b"lo lo lo\n", "lo"));

        let mut view = SearchView::new(store.clone());
        let mut backing = vec![Cell::default(); 10];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 10,
            size: CellSize::new(1, 10),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(1, 10),
            },
            &mut grid,
        );

        // Match 0 [0,2) is active (bright yellow), matches at [3,5) and
        // [6,8) are lazy (yellow); the spaces between carry no bg.
        assert_eq!(grid.get(CellCoord::new(0, 0)).style.bg, Color::Indexed(11));
        assert_eq!(grid.get(CellCoord::new(0, 1)).style.bg, Color::Indexed(11));
        assert_eq!(grid.get(CellCoord::new(0, 2)).style.bg, Color::Default);
        assert_eq!(grid.get(CellCoord::new(0, 3)).style.bg, Color::Indexed(3));
        assert_eq!(grid.get(CellCoord::new(0, 6)).style.bg, Color::Indexed(3));

        // Stale store paints nothing.
        store.lock().unwrap().mark_stale(bid);
        let mut backing2 = vec![Cell::default(); 10];
        let mut grid2 = CellGrid {
            cells: &mut backing2,
            stride: 10,
            size: CellSize::new(1, 10),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(1, 10),
            },
            &mut grid2,
        );
        assert_eq!(
            grid2.get(CellCoord::new(0, 0)).style.bg,
            Color::Default,
            "stale store washes nothing"
        );
    }

    #[test]
    fn store_staleness_tracks_per_buffer() {
        let mut s = SearchStore::new();
        let bid = BufferId::next();
        s.set(bid, "x", vec![r(0, 1)]);
        assert!(!s.is_stale(bid));
        s.mark_stale(bid);
        assert!(s.is_stale(bid));
        // A fresh set clears stale.
        s.set(bid, "x", vec![r(0, 1)]);
        assert!(!s.is_stale(bid));
        // mark_stale is a no-op for a buffer with no search state.
        let other = BufferId::next();
        s.mark_stale(other);
        assert!(!s.is_stale(other));
    }
}
