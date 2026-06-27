# Regex in-buffer search — framing pass

Date: 2026-06-27. Extends the merged incremental isearch (PR #70):
`C-s`/`C-r` drive a smart-case **substring** search over a per-buffer
`SearchStore`, highlighted live in both frontends. This adds **regex**
matching alongside the literal path. Decided up front (user):

- **Both** a dedicated regex entry (`C-M-s` / `C-M-r`) **and** a
  mid-search toggle (`M-r`, Emacs `isearch-toggle-regexp`).
- **Multi-line** matches (a pattern may span newlines).

## Survey facts (anchors — what shipped in #70)

- `find_all(haystack, query) -> Vec<ByteRange>` (search.rs): smart-case
  ASCII substring, non-overlapping. The one matcher today.
- `EditorCore::SearchSession { query, origin, forward }` +
  `search_begin/input_char/backspace/step/finish/recompute`. Recompute
  runs `find_all` over an `O(1)` rope snapshot, writes the store,
  refocuses from origin, moves the cursor.
- Input is intercepted in the shared `dispatch_key` →
  `dispatch_search_key` (`SearchKey::from_chord`), so TUI-local and
  daemon (GUI round-trip) keystrokes drive the same core.
- TUI `SearchView` overlay washes matches; it assumes **single-line**
  matches ("the query carries no newline") — the one place multi-line
  breaks.
- GPU washes via `SearchMatch`/`SearchMatchActive` decorations through
  `push_glyph_extent_rects`, which **already fans a byte range across
  visual lines** (multi-line selections use the same path) — so the
  GPU needs **no** rendering change for multi-line.
- `InstanceMessage::SearchPrompt { buffer_id, query, active, total }`
  (protocol v9) carries the query + readout to the GUI status band.
- `regex` 1.12.3 is already in `Cargo.lock` (transitive); promote to a
  direct dependency.

## Q#RX1 — the matcher

**Stance: a sibling `find_all_regex(haystack, pattern) -> Option<Vec<ByteRange>>`
on `regex::bytes::Regex` over the whole buffer.** `Some(matches)` for a
valid pattern (possibly empty), `None` for an invalid one (so the
caller can show `[invalid]` rather than `[no match]`). `find_iter`
gives leftmost non-overlapping byte ranges directly; **zero-width
matches are filtered** (start == end — `a*`, `^`, anchors — wash
nothing and would spam). Multi-line falls out for free: the regex runs
over the whole byte slice, so `foo\n\s*bar` matches across the newline.
`.` keeps its default (no `\n`); the user opts into dotall with `(?s)`.

## Q#RX2 — smart-case for regex

**Stance: the same uppercase heuristic, via an `(?i)` prefix.**
Case-insensitive unless the *pattern string* contains an uppercase
ASCII letter, implemented by compiling `(?i){pattern}` vs `{pattern}`.
Accepted imprecision: an uppercase letter inside an escape/class (`\D`,
`[A-Z]`) trips case-sensitivity — same spirit as the literal path's
"any uppercase ⇒ exact", and a regex author writing `[A-Z]` plausibly
wants case to matter anyway. Documented, not solved, in v1.

## Q#RX3 — session mode & toggle

**Stance: a `regex: bool` on `SearchSession`; `recompute` dispatches.**
`search_begin(forward, regex)` records the mode; `recompute` calls
`find_all_regex` (regex) or `find_all` (literal). `M-r` →
`SearchKey::ToggleRegex` → `search_toggle_regex()` flips the flag and
re-runs recompute on the same query (re-anchored from origin). The
session tracks an `invalid` bool (last recompute's pattern failed to
compile) so the prompt can distinguish invalid from zero-match.

## Q#RX4 — multi-line TUI wash

**Stance: `SearchView` iterates the rows each match spans, mirroring
`paint_local_selection`.** Per match `[start, end)`: for each display
row from `line(start)` to `line(end)`, wash that row's clipped slice
(`paint_start = max(start, line_start)`, `paint_end = min(end,
line_end)`), mapping to display cols with the existing
`byte_range_to_display_cols`. Single-line matches (every literal match,
and most regex matches) hit exactly one row — no behavior change there.
The GPU is untouched (Q#RX-anchor: `push_glyph_extent_rects` already
multi-lines).

## Q#RX5 — entry keys & both frontends

**Stance: `C-M-s`/`C-M-r` start a regex search; `M-r` toggles within
any search.** Daemon keymap binds `C-M-s` → `search.forward-regex`,
`C-M-r` → `search.backward-regex` (Lua → `ed.search_start(forward,
regex=true)`); `M-r` is handled *inside* `dispatch_search_key` (a
`SearchKey`, not a global binding — it is only meaningful mid-search).
GUI: `is_search_entry_chord` also forwards `C-M-s`/`C-M-r` (Ctrl+Alt);
`M-r` already round-trips via the intercept path once a search is
running, so it needs no GUI change. The TUI/GUI prompt reads
**`Regex I-search:`** in regex mode and **`[invalid]`** when the
pattern won't compile.

## Q#RX6 — the wire (GUI)

**Stance: extend `SearchPrompt` with `regex: bool` + `invalid: bool`,
protocol v10.** `SearchPrompt` is new in v9 (this is the first
encoding change to it), so the additive-but-encoding-changing rule
bumps `PROTOCOL_VERSION` 9 → 10 and grows `SUPPORTED` to
`[6,7,8,9,10]`, daemon-gated per session exactly like v9. The producer
fills both from the active `SearchSession`; the GUI renders the regex
label + invalid indicator from them.

## Predicted findings (categorical bets)

1. **Multi-line TUI wash correctness** (headline): row-by-row clipping
   at line boundaries — off-by-one on the trailing newline / a match
   ending exactly at a line end / an empty middle line must wash
   cleanly, not bleed a column or skip a row.
2. **Invalid-regex incremental states**: typing `foo(` passes through
   invalid mid-keystroke constantly; recompute must degrade to "no
   matches + `[invalid]`", never panic or surface a raw regex error,
   and recover the instant the pattern compiles again.
3. **Smart-case heuristic × escapes/classes**: `\D`, `[A-Z]` flip
   case-sensitivity under the simple "any uppercase" rule — acceptable,
   noted.
4. **Zero-width / pathological matches**: `a*`, `^`, `$` produce empty
   or per-position matches; filter empties, and lean on `find_iter`'s
   non-overlapping advance so a catastrophic pattern can't loop.
5. **Regex compile per keystroke**: a fresh compile each character is
   fine for interactive small patterns; no cache in v1 (note it).

## Session plan

Four green commits (mirrors the #70 arc: core → TUI → GUI):

1. This framing doc.
2. `find_all_regex` (multi-line, smart-case, `Option` for invalid) +
   `regex` direct dep + unit tests.
3. TUI: `SearchSession.regex` + `invalid` + `search_begin(fwd, regex)`
   + `search_toggle_regex` + `SearchKey::ToggleRegex` (`M-r`) +
   `search.forward-regex`/`backward-regex` commands + `C-M-s`/`C-M-r`
   bindings + `Regex I-search:` / `[invalid]` prompt + multi-line
   `SearchView` wash + tests.
4. GUI: protocol v10 (`SearchPrompt` + `regex`/`invalid`), producer
   fill, GUI prompt label + indicator, `C-M-s`/`C-M-r` entry chords.

Manual validation gate as usual (regex highlight live, multi-line
pattern washes across rows, `M-r` toggles, invalid pattern shows
`[invalid]`, both frontends).
