# pmacs-gpu — viewport-scoped rendering & scroll (framing)

**Status: framing pass; pre-implementation. Urgent.** Editing a large
file in pmacs-gpu is unusably slow because rendering is O(file) per
keystroke. This milestone makes it O(visible) and adds scrolling. Same
framing discipline as the Phase B framing: Q-decisions committed before
code, fact-checked first.

## Why this exists (the perf problem, precisely)

Per keystroke today, for a 240 KB file:

- **GPU**: `reshape()` rebuilds rich text from the *entire*
  `current_text` — `projected_rich_chunks` walks all 240 KB, then
  `set_rich_text` creates a `BufferLine` per source line (~25k). It
  fires on every edit, and one keystroke produces 2–3 reshapes (the
  `CrdtOp`, `StyleSpans`, and `Decorations` frames each trigger one).
- **GPU Viewport**: pmacs-gpu declares `visible: { 0, text_len }` — the
  whole file (`main.rs` `BufferSnapshot`/`CrdtOp` arms). So the producer
  is asked to style the whole file.
- **Daemon**: on each edit the generation bumps, the `StyleGate`
  recomputes, and `scoped_style_spans` runs the tree-sitter highlight
  query over the whole bundle (it clips the *result* to the viewport
  but runs the query whole-file).

The GPU's O(file) `set_rich_text` + `projected_rich_chunks` dominate and
run per edit. Editing is hundreds of ms to seconds per character.

## Goal

Make the GPU per-edit cost **O(visible lines)**, not O(file), and add
line-based scrolling so the whole file is reachable. Correctness (text,
styling, caret, edits, CRDT convergence) is preserved.

## Approach: render only the visible slice (stance committed)

cosmic-text offers a native `Buffer::set_scroll` + `shape_until_scroll`
path, but that only makes *shaping* lazy — `set_rich_text` and
`projected_rich_chunks` would still process the whole rope. So that path
does **not** fix the dominant cost.

**Stance: feed cosmic-text only the visible byte slice.** The GPU keeps
the whole rope in `current_text` (the CRDT replica is authoritative and
small to hold), but builds chunks and calls `set_rich_text` over
`current_text[vstart..vend]` — the byte range of the visible source
lines. Everything cosmic-text touches is then O(visible): chunk build,
`BufferLine` creation, shaping, layout. Scrolling re-slices and
re-shapes; the buffer always renders from its own top (no cosmic-text
scroll offset needed).

The cost is **coordinate rebasing**: spans / decorations / caret /
presence arrive in whole-file byte coordinates; the slice starts at
file byte `vstart`, so each offset maps to slice byte `offset - vstart`,
and only the portion intersecting `[vstart, vend)` is rendered. This is
the part to get exactly right — and the QB3 lesson applies (glyph
offsets are line-relative; `line_byte_offsets` is computed on the
slice).

## Contract inheritance

Pixel-pure instance preserved: the GPU still sends only a byte-range
`Viewport` and byte-anchored input. The producer already clips
`StyleSpans` / `Decorations` to `vp.visible` (verified), so a scoped
viewport immediately cuts wire volume and GPU span processing with no
producer change required.

## Forced decisions

### Q#S1 — scroll unit: line-based

**Scroll position is a source-line index (`scroll_top`), not a pixel
offset.** The visible slice is `[line_start(scroll_top),
line_start(scroll_top + visible_lines + overscan))`. Line-based scroll
makes the slice always start on a line boundary (cosmic-text splits
`BufferLine`s on `\n`, so a mid-line slice would corrupt the first
line) and matches how `estimated_visible_lines` already works.
Pixel-smooth scroll is a later refinement.

### Q#S2 — what drives scroll: keep the caret visible

**The cursor stays on screen.** `scroll_top` is adjusted whenever a
`CursorByte` (own cursor) would fall outside the visible line range:
scroll just enough to bring the cursor's line to the nearest visible
edge (+ a small margin). This is the only scroll trigger session-1
needs — it makes arrow/PageUp/PageDown navigation work without a
separate scroll command:

- Arrow up/down past the edge → cursor moves (daemon) → `CursorByte`
  → auto-scroll follows.
- `PageUp`/`PageDown` → already forwarded; the daemon moves the cursor
  by a page → `CursorByte` → auto-scroll follows. (No GPU-local page
  math; the daemon owns cursor motion, Q#B3.)

Mouse-wheel / scrollbar scroll-without-cursor-move is a later add.

### Q#S3 — overscan: a small margin

Slice a few lines beyond the visible region (e.g. visible + 2) so a
1-line scroll doesn't always re-slice, and the bottom partial line
renders. Keep it small — overscan is wasted shaping.

### Q#S4 — rebasing rule: subtract `vstart`, computed once

A single `vstart` (file byte of the first visible line) rebases
everything: `slice = current_text[vstart..vend]`; a span/decoration/
caret at file byte `b` is at slice byte `b - vstart` and is rendered
only if `vstart <= b < vend`; `line_byte_offsets` is computed on
`slice`. One helper clips+rebases a `[start,end)` range to the slice
(returns `None` when disjoint). The caret and the background-wash
builders all route through it.

### Q#S5 — Viewport declaration: the visible range

The GPU declares `Viewport { visible: { vstart, vend } }` whenever the
slice changes (scroll or buffer switch), and **re-declares on scroll**
so the producer ships spans for what's on screen. Coalesce: only send
when `[vstart, vend)` actually changed. (Today only `BufferSnapshot`
declares a Viewport; the `CrdtOp` edit arm sends none — so the producer
keeps styling the last-declared range across edits, which is exactly
why the whole-file range is currently sticky.)

### Q#S6 — daemon whole-file highlight query: out of scope (noted)

`scoped_style_spans` runs the tree-sitter query over the whole bundle
even for a scoped viewport. After this milestone the GPU is O(visible)
and the wire is scoped, so the daemon query becomes the next bottleneck
on very large files — but it is a *separate* producer optimization
(query-by-range / incremental), deferred to its own task. The GPU
session does not depend on it.

## Predicted findings — categorical bets

| # | Bet | Category |
|---|---|---|
| S1 | Off-by-one / mid-line slicing at the bottom edge (partial last line, trailing newline) corrupts the visible text | Boundary-decomposition |
| S2 | Rebasing misses a coordinate space — a span or the caret rendered at `b` instead of `b - vstart`, or vice versa | Coordinate-space (recurrence of QB3) |
| S3 | Scroll re-declares Viewport every frame (not just on change), re-introducing per-frame churn | Cadence |
| S4 | Caret-follow scroll oscillates or lags by a frame when the cursor moves and the slice + caret update on different frames | Temporal-interaction |

Bet S2 is pre-flagged: the rebasing must be verified against the
cosmic-text source as QB3 taught, not assumed.

## Session plan

| Session | Work | Probe |
|---|---|---|
| **S1** | Visible-slice `reshape` (slice + clip/rebase spans, decorations, caret, washes) + `scroll_top` state + caret-follow auto-scroll + scoped Viewport (Q#S1–S5). | Open a 240 KB file: editing is snappy; arrow/PageUp/PageDown navigate the whole file with the caret staying visible; styling/caret/edits correct at any scroll position; TUI stays converged. |
| **S2+** | Mouse-wheel / scrollbar scroll; pixel-smooth scroll; daemon query-by-range (Q#S6). | Per-session. |

## Process rule (carried forward)

Touches the core render path — the area that has regressed before. Not
merged until visually confirmed on a large file: editing is fast, the
caret tracks, and styling is correct **after scrolling** (the rebasing
is only exercised once `vstart > 0`). CI-green is necessary, not
sufficient.

## Deliberately not committed

- Pixel-smooth (sub-line) scroll — line-based first.
- Mouse-wheel / scrollbar — S2.
- Daemon query-by-range — Q#S6, its own task.
- Horizontal scroll / no-wrap long lines — separate; pmacs-gpu has no
  soft wrap yet.
