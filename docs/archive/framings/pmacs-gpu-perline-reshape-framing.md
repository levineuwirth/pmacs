# pmacs-gpu per-line incremental reshape — framing pass

Date: 2026-06-10. The per-keystroke latency floor after the
optimistic-typing arc: every keystroke runs `reshape()` — full
visible-slice chunk rebuild + `set_rich_text` (resets every
`BufferLine`'s shape cache) + `shape_until_scroll` (re-shapes every
visible line with `Shaping::Advanced`).

## Verified cosmic-text facts (0.18.2, vendored source)

- `Buffer::lines` is `pub Vec<BufferLine>`; replacing one element with
  `BufferLine::new(text, ending, attrs_list, shaping)` leaves the
  other lines' shape caches intact, and `shape_until_scroll` re-shapes
  only the fresh line (`shape_opt: Cached::Empty`).
- `set_rich_text` splits lines via `BidiParagraphs`, which strips the
  trailing paragraph separator from every yielded line in BOTH the
  ASCII fast path and the `BidiInfo` general path, yields **no
  trailing empty line** for text ending in `\n`, and assigns
  `LineEnding::default()` (= `Lf`) to every line including the last.
  Attr spans are added only when they differ from the defaults.
- Parity hazard: separators other than `\n` (`\r`, U+0085, U+2028,
  U+2029) split lines in the full path; a surgically built line
  containing one would diverge.

## Q#R1 — surgery vs full rebuild

**Stance: per-line surgery for the single-line edit (the keystroke
case), full `reshape()` for everything else.** Fallback conditions
(any ⇒ full): slice origin moved; line count changed (Enter,
multi-line delete); inserted text contains `\n`; edited line outside
the shaped slice (except: an edit entirely *past* the slice end
changes no visible line — update `view_range` and redraw only); the
rebuilt line's projected text contains a non-`\n` paragraph
separator; more than one edit in the batch.

Equivalence invariant: the surgically built line's (text, attr spans)
must equal what a full `set_rich_text` over the slice would produce
for that line. Achieved by deriving both from one shared chunk
function (`clipped_chunks_for_range`) — the full path calls it with
the slice range, surgery with the line's content range — and pinned
by a pure unit test (full-walk output split at line boundaries ==
concatenated per-line walks).

## Q#R2 — the M-2 hit map under surgery

The pointer hit map (projected runs + projected line starts) is
derived from the full chunk walk. **Stance: make it lazy** — surgery
marks it dirty; `hit_test_source_byte` rebuilds it on demand from the
same shared chunk function. Clicks are rare relative to keystrokes,
and the rebuild is an O(slice) byte walk (no shaping), microseconds.
Deriving from the same inputs keeps the map consistent with the
shaped buffer by construction.

## Q#R3 — scope

Only the local text-delta apply path (`apply_loro_text_delta_batches`
callers: optimistic edits + incoming CrdtOps) takes the surgery path.
StyleSpans/Decorations/adornment arrivals, scroll, resize, snapshot
keep full reshape — they change content across the slice and arrive
at far lower cadence (parse-settle / server-response rate).

## Predicted findings (categorical bets)

1. **Parity miss** in some attrs/boundary case the unit test doesn't
   cover (most likely: adornment anchored exactly at a line edge) —
   surfaces as one mis-styled line until the next full reshape.
2. **Staleness leak**: some consumer besides the hit map silently
   depended on full-reshape side effects (`view_range` freshness,
   redraw requests) — surfaces as a paint lag.
3. The win is real but the *remaining* per-keystroke cost shifts to
   glyphon `prepare` (full-buffer glyph pass per redraw), capping the
   perceived improvement.

## Session plan

Single session: shared chunk fn refactor → surgery + fallbacks →
lazy hit map → pure parity tests. Manual validation: type mid-file
(fast burst), Enter (fallback), type on a line with inlay hints,
click after typing (lazy map), peer-edit while typing.
