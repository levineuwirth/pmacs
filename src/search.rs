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
    /// matches — or when they are stale (Q#AI8 fail-closed): stale
    /// ranges were computed against pre-edit text, and stepping
    /// through them would teleport the cursor to offsets that no
    /// longer exist. A live search un-sticks on the next pattern
    /// keystroke ([`Self::set`] clears staleness).
    pub fn step(&mut self, buffer_id: BufferId, forward: bool) -> Option<ByteRange> {
        if self.stale.contains(&buffer_id) {
            return None;
        }
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

/// Smart-case regex search over `haystack` bytes for `pattern` (Q#RX1).
///
/// Returns `Some(matches)` — leftmost, non-overlapping, ascending byte
/// ranges — for a valid pattern (possibly empty), or `None` when
/// `pattern` fails to compile. The `Option` lets the caller tell an
/// *invalid* pattern (show `[invalid]`) apart from a valid one with no
/// matches (a failing search), which a flat `Vec` could not.
///
/// **Smart-case (Q#RX2).** Case-insensitive unless `pattern` contains
/// an uppercase letter, applied by compiling `(?i)` ahead of the
/// pattern. An uppercase letter inside an escape or class (`\D`,
/// `[A-Z]`) trips case-sensitivity — the same coarse rule the literal
/// path uses.
///
/// **Multi-line (Q#RX1)** falls out for free: the regex runs over the
/// whole byte slice, so an explicit `\n` (or `(?s).`) spans lines. `.`
/// keeps its default of not matching `\n`.
///
/// **Zero-width matches** (`a*`, `^`, `$`, anchors) are filtered — they
/// wash nothing and would otherwise flood the match list. The `regex`
/// crate's linear-time engine makes a pathological pattern slow at
/// worst, never catastrophic.
#[must_use]
pub fn find_all_regex(haystack: &[u8], pattern: &str) -> Option<Vec<ByteRange>> {
    if pattern.is_empty() {
        return Some(Vec::new());
    }
    let re = compile_search_regex(pattern)?;
    let matches = re
        .find_iter(haystack)
        .filter(|m| m.end() > m.start())
        .map(|m| ByteRange {
            start: m.start() as u64,
            end: m.end() as u64,
        })
        .collect();
    Some(matches)
}

/// Compile `pattern` with the same smart-case rule the search paths use
/// (case-insensitive unless the pattern has an uppercase letter, via a
/// `(?i)` prefix), or `None` if it fails to compile. Shared by
/// [`find_all_regex`] and the query-replace session (which caches the
/// compiled engine for the whole run — Q#QR2).
#[must_use]
pub fn compile_search_regex(pattern: &str) -> Option<regex::bytes::Regex> {
    let case_insensitive = !pattern.chars().any(char::is_uppercase);
    if case_insensitive {
        regex::bytes::Regex::new(&format!("(?i){pattern}"))
    } else {
        regex::bytes::Regex::new(pattern)
    }
    .ok()
}

/// The first smart-case literal match of `query` in `haystack` at or
/// after byte `start` (Q#QR2: query-replace's forward step). Same
/// case-folding as [`find_all`]. An empty query, or `start` past the
/// last possible match, yields `None`.
#[must_use]
pub fn find_first_from(haystack: &[u8], query: &str, start: usize) -> Option<ByteRange> {
    let q = query.as_bytes();
    if q.is_empty() || start > haystack.len() || haystack.len() - start < q.len() {
        return None;
    }
    let case_sensitive = query.chars().any(char::is_uppercase);
    let mut i = start;
    while i + q.len() <= haystack.len() {
        let hit = haystack[i..i + q.len()].iter().zip(q).all(|(&h, &n)| {
            if case_sensitive {
                h == n
            } else {
                h.eq_ignore_ascii_case(&n)
            }
        });
        if hit {
            return Some(ByteRange {
                start: i as u64,
                end: (i + q.len()) as u64,
            });
        }
        i += 1;
    }
    None
}

/// The first non-zero-width match of the pre-compiled `re` in
/// `haystack` at or after byte `start` (Q#QR2). Uses `find_at` so the
/// engine keeps look-around context (`\b`, `^`) correct at the seam,
/// and skips zero-width matches (`a*`, anchors) by advancing one byte —
/// a zero-width hit never moves `next_from`, so it would otherwise
/// loop.
#[must_use]
pub fn find_first_regex_from(
    haystack: &[u8],
    re: &regex::bytes::Regex,
    start: usize,
) -> Option<ByteRange> {
    let mut pos = start;
    while pos <= haystack.len() {
        let m = re.find_at(haystack, pos)?;
        if m.end() > m.start() {
            return Some(ByteRange {
                start: m.start() as u64,
                end: m.end() as u64,
            });
        }
        // Zero-width match: step past it to guarantee progress.
        pos = m.start() + 1;
    }
    None
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
            let style = if Some(*m) == active {
                active_match_style()
            } else {
                match_style()
            };
            // A regex match may span multiple lines (Q#RX4); wash each
            // row's clipped slice, mirroring the selection renderer.
            // Single-line matches (every literal match) touch one row.
            let first_line = crate::diag::line_at_offset(&line_offsets, m.start as u32);
            // Matches are sorted ascending, so once one starts below the
            // viewport every later one does too — stop.
            if first_line >= start_line_buf.saturating_add(max_rows) {
                break;
            }
            let last_byte = m.end.saturating_sub(1).max(m.start) as u32;
            let last_line = crate::diag::line_at_offset(&line_offsets, last_byte);
            for line in first_line..=last_line {
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
                // Clip the match to this line's content (newline excluded
                // so a multi-line match doesn't wash a phantom trailing
                // cell).
                let paint_start = (m.start as u32).max(line_start);
                let paint_end = (m.end as u32).min(line_end_no_nl);
                if paint_start >= paint_end {
                    continue;
                }
                let line_bytes = &source[line_start as usize..line_end_no_nl as usize];
                let within_start = (paint_start - line_start) as usize;
                let within_end = (paint_end - line_start) as usize;
                let (start_col, end_col) =
                    crate::diag::byte_range_to_display_cols(line_bytes, within_start, within_end);
                if end_col <= start_col {
                    continue;
                }
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

    // ---- regex matcher (Q#RX1) -----------------------------------------

    #[test]
    fn find_all_regex_matches_pattern() {
        // \d+ over "a1 bb 23 c" → "1" and "23".
        assert_eq!(
            find_all_regex(b"a1 bb 23 c", r"\d+"),
            Some(vec![r(1, 2), r(6, 8)])
        );
    }

    #[test]
    fn find_all_regex_is_smart_case() {
        // Lowercase pattern folds case (matches "Foo", "foo", "FOO").
        assert_eq!(
            find_all_regex(b"Foo foo FOO", "foo"),
            Some(vec![r(0, 3), r(4, 7), r(8, 11)])
        );
        // An uppercase letter in the pattern makes it case-sensitive.
        assert_eq!(find_all_regex(b"Foo foo FOO", "Foo"), Some(vec![r(0, 3)]));
    }

    #[test]
    fn find_all_regex_spans_newlines() {
        // An explicit \n in the pattern matches across the line break.
        assert_eq!(
            find_all_regex(b"foo\n  bar", r"foo\n\s*bar"),
            Some(vec![r(0, 9)])
        );
        // Plain `.` does NOT cross the newline (default, not dotall).
        assert_eq!(find_all_regex(b"a\nb", "a.b"), Some(vec![]));
        // ...but `(?s)` opts into dotall.
        assert_eq!(find_all_regex(b"a\nb", "(?s)a.b"), Some(vec![r(0, 3)]));
    }

    #[test]
    fn find_all_regex_invalid_pattern_is_none() {
        // Unbalanced group — the incremental-typing case (`foo(`).
        assert_eq!(find_all_regex(b"foo(", "foo("), None);
        // A valid pattern with zero matches is Some(empty), distinct
        // from invalid.
        assert_eq!(find_all_regex(b"abc", "zzz"), Some(vec![]));
    }

    #[test]
    fn find_all_regex_filters_zero_width_and_empty() {
        // `a*` matches empty at non-'a' positions; only the non-empty
        // runs survive the zero-width filter.
        assert_eq!(find_all_regex(b"baab", "a*"), Some(vec![r(1, 3)]));
        // An empty pattern yields no matches (not one-per-position).
        assert_eq!(find_all_regex(b"abc", ""), Some(vec![]));
    }

    // ---- find_first_from (query-replace forward step, Q#QR2) ---------------

    #[test]
    fn find_first_from_scans_forward() {
        assert_eq!(find_first_from(b"a.a.a", "a", 0), Some(r(0, 1)));
        // Start past the first hit → the next one.
        assert_eq!(find_first_from(b"a.a.a", "a", 1), Some(r(2, 3)));
        assert_eq!(find_first_from(b"a.a.a", "a", 3), Some(r(4, 5)));
        // No match at/after start.
        assert_eq!(find_first_from(b"a.a.a", "a", 5), None);
        assert_eq!(find_first_from(b"abc", "z", 0), None);
        // Empty query never matches.
        assert_eq!(find_first_from(b"abc", "", 0), None);
    }

    #[test]
    fn find_first_from_is_smart_case() {
        // Lowercase query folds case; uppercase query is exact.
        assert_eq!(find_first_from(b"xFoo", "foo", 0), Some(r(1, 4)));
        assert_eq!(find_first_from(b"xFoo", "Foo", 0), Some(r(1, 4)));
        assert_eq!(find_first_from(b"xfoo", "Foo", 0), None);
    }

    #[test]
    fn find_first_from_does_not_reloop_on_growing_replacement() {
        // The a→aa shape: after replacing the 'a' at 0 with "aa", the
        // next search must start PAST the replacement (byte 2), not
        // re-match the inserted text. Simulated here by starting the
        // scan at the replacement end.
        assert_eq!(find_first_from(b"aa_a", "a", 2), Some(r(3, 4)));
    }

    #[test]
    fn find_first_regex_from_scans_and_skips_zero_width() {
        let re = compile_search_regex("a+").unwrap();
        assert_eq!(find_first_regex_from(b"_aa_a", &re, 0), Some(r(1, 3)));
        assert_eq!(find_first_regex_from(b"_aa_a", &re, 3), Some(r(4, 5)));
        assert_eq!(find_first_regex_from(b"_aa_a", &re, 5), None);
        // Zero-width pattern `x*` never yields a match (all filtered),
        // and crucially terminates rather than looping.
        let z = compile_search_regex("x*").unwrap();
        assert_eq!(find_first_regex_from(b"abc", &z, 0), None);
    }

    #[test]
    fn compile_search_regex_smart_case_and_invalid() {
        // Lowercase → case-insensitive.
        let re = compile_search_regex("foo").unwrap();
        assert!(re.is_match(b"FOO"));
        // Uppercase → case-sensitive.
        let re = compile_search_regex("Foo").unwrap();
        assert!(!re.is_match(b"foo"));
        // Invalid pattern → None (the session refuses to start).
        assert!(compile_search_regex("(unclosed").is_none());
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
                gutter_w: 0,
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
                gutter_w: 0,
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
    fn search_view_washes_a_multiline_match_per_row() {
        use crate::cell::{Cell, CellGrid, CellSize};
        use crate::view::Viewport;

        let store = make_shared_store();
        let bid = BufferId::next();
        let mut buf = Buffer::new(bid, "t.txt");
        // "foo\nbar\nbaz": match [0,7) = "foo\nbar" spans lines 0–1.
        buf.apply_edit(crate::buffer::EditOp::Insert {
            pos: 0,
            bytes: b"foo\nbar\nbaz",
        })
        .expect("seed");
        store.lock().unwrap().set(bid, r"foo\nbar", vec![r(0, 7)]);

        let (rows, cols) = (3u32, 10u32);
        let mut backing = vec![Cell::default(); (rows * cols) as usize];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: cols,
            size: CellSize::new(rows, cols),
        };
        SearchView::new(store.clone()).render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(rows, cols),
                gutter_w: 0,
            },
            &mut grid,
        );

        // Row 0 "foo" and row 1 "bar" both wash (the active match's
        // bright color); the newline cells and row 2 "baz" do not.
        for col in 0..3 {
            assert_eq!(
                grid.get(CellCoord::new(0, col)).style.bg,
                Color::Indexed(11),
                "row 0 col {col} should wash"
            );
            assert_eq!(
                grid.get(CellCoord::new(1, col)).style.bg,
                Color::Indexed(11),
                "row 1 col {col} should wash"
            );
        }
        assert_eq!(
            grid.get(CellCoord::new(0, 3)).style.bg,
            Color::Default,
            "the newline cell past 'foo' is not washed"
        );
        assert_eq!(
            grid.get(CellCoord::new(2, 0)).style.bg,
            Color::Default,
            "row 2 'baz' is outside the match"
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

    #[test]
    fn step_fails_closed_while_stale() {
        // Q#AI8: stale ranges were computed against pre-edit text;
        // stepping through them would teleport the cursor to offsets
        // that no longer exist.
        let mut s = SearchStore::new();
        let bid = BufferId::next();
        s.set(bid, "x", vec![r(0, 1), r(4, 5)]);
        assert!(s.step(bid, true).is_some(), "fresh matches step");
        s.mark_stale(bid);
        assert!(s.step(bid, true).is_none(), "stale matches do not");
        // A re-run (`set`) clears staleness and stepping resumes.
        s.set(bid, "x", vec![r(0, 1), r(4, 5)]);
        assert!(s.step(bid, true).is_some(), "fresh set un-sticks stepping");
    }
}
