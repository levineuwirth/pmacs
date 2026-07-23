# Folding Stage 2 — grid (daemon-rendered) collapse — framing (Arc 6)

**Revision 1 — 2026-07-23. Status: DRAFT for review.** Parent architecture
(`docs/folding-framing.md`, rev 5) is APPROVED and Stage 1 (the headless fold
engine) is MERGED as **#142** (canonical `main` @ `c49a8c7`). This doc reframes
**Stage 2** in detail off that base, per the parent's §8/§14 ("each stage is
re-framed after the prior stage lands"). It carries the four Stage-2 obligations
the parent named, plus the one architectural fact the Stage-2 scout surfaced that
the parent only sketched.

Numbering continues the parent's `Q#FD…` scheme from `Q#FD12`.

## 1. What Stage 2 ships

The grid TUI is **daemon-rendered**: the daemon walks a buffer's source lines
and paints a character grid it ships to the terminal client. Stage 1 built the
instance-side fold store and produces `FoldState` for GPU sessions, but **no
frontend renders a collapse yet** — the daemon grid path never consults the fold
store.

Stage 2 makes the **daemon grid renderer fold-aware**:

1. **Collapse** — omit each fold's hidden source lines; show the head line with a
   trailing ellipsis; rows below shift up.
2. **Gutter fold marker** — a fold glyph on the head-line row (parent §7).
3. **Fold-aware line numbers** — the `LineNumbers` family skips hidden lines, and
   relative/hybrid distance is measured across the collapse (parent §8).
4. **Fold-aware diagnostic signs** — a sign on a hidden line clamps to the fold's
   visible head-line row (parent §8, minor d).
5. **Fold-aware caret** — the caret never renders on a hidden line; it clamps to
   the fold head (parent §8, Q#FD3 render-time invariant).
6. **Fold-aware viewport/scroll** — `view_top`, paging, and auto-scroll count
   **visible** lines, not raw source lines (parent §8).
7. **Interactive-Lua-command unfold widening** — yank, query-replace, and
   comment-toggle unfold a fold at their edit point before the edit is visible
   (parent §8, R3-3), which is only observable once the TUI collapses.

**No wire schema changes and no protocol bump.** `FoldState` is already produced
(Stage 1) for semantic/GPU sessions; the TUI collapse is entirely daemon-side
(the vterm-Stage-2 shape). The GPU render path is untouched — that is Stage 3.
The store, the source, and the producer from Stage 1 are unchanged; Stage 2 only
*reads* the store in a new place (the daemon grid renderer) and *widens* one
pre-edit hook.

## 2. Ground truth (scouted 2026-07-23, `main` @ `c49a8c7`)

### 2.1 The grid render path is layered — and the collapse belongs deep in it

The parent framing (F1) located the collapse at "the daemon grid renderer,
`render_frame`." The scout refined this: `RenderState::render_frame`
(`src/instance_render.rs:98`) is a **diff shell** — it double-buffers cells and
emits `CellDelta`; it does **not** walk source lines. The source-line→grid-row
loop is two calls down:

```
RenderState::render_frame  (src/instance_render.rs:98)   — diff shell, no line walk
  → paint_frame            (src/editor.rs:2796)          — per-window composition; HAS state.fold_registry
      → window.text_view.render(buf, viewport, grid)     (src/editor.rs:2948)
          → TextView::render (src/text_view.rs:207)      — THE line→row loop
```

`TextView::render` is the primitive (`src/text_view.rs:213`):

```rust
for row_offset in 0..max_rows {
    let line = start_line + row_offset as usize;   // source line  (:214)
    let cell_row = origin.row + row_offset;         // grid row     (:215)
    ...
}
```

The mapping is **strictly identity** — `line = start_line + row_offset`,
contiguous +1, no skip, no wrap (long lines truncate at `max_cols`, they do not
wrap). This identity is repeated, un-abstracted, at **every** view_top-arithmetic
site:

| Site | file:line | assumes |
|---|---|---|
| render loop | `src/text_view.rs:214` | `line = view_top + row` |
| line-number gutter | `src/editor.rs:3215` | `buffer_line = view_top + r` |
| diagnostic signs | `src/diag.rs:563` | `row_offset = line − start_line` |
| caret grid row | `src/editor.rs:3044` | `grid_row = origin + (disp.row − view_top)` |
| click inverse | `src/editor.rs:2233` | `display_row = view_top + local_row` |
| auto-scroll clamp | `src/editor.rs:2866` | cursor row in `[view_top, view_top+rows)` |
| page/scroll | `src/editor_core.rs:1745`, `src/editor.rs:2264` | target = raw source row |

`Window.view_top: usize` (`src/window.rs:179`) is a **source-line index**.
`DisplayCoord`'s own doc (`src/view.rs:108`) anticipates that row/col "diverge
once virtual lines, wrapping, and inline expansions appear," but **nothing
implements a non-identity map today** — `TextView` is the only base view and its
`pos_to_display`/`display_to_pos` are identity. **Folding is the first
non-identity source-line↔display-row mapping in the TUI.**

**Consequence (the crux, Q#FD12).** Fold collapse cannot be a localized edit to
one loop, and it cannot be an overlay (overlays paint *on top of* laid-out rows;
they cannot delete rows or change the row count). It must be a **shared
fold-aware line map** that the render loop *and* all seven sites above consult.

### 2.2 The renderer can already reach the fold store

`fold_registry: SharedFoldRegistry` is a field on both `EditorCore`
(`src/editor_core.rs:223`) and `EditorState` (`src/editor.rs:110`).
`paint_frame` borrows `state`, so `state.fold_registry` + `window.buffer_id` are
in scope at the composition point (`src/editor.rs:2796`). The semantic producer
already reads it: `state.fold_registry.folds(buffer_id)`
(`src/semantic_render.rs:1433`). The **grid path reads it nowhere** yet — that is
the new consumer Stage 2 adds.

The store's read surface (`src/fold.rs`):
- `FoldRegistry::folds(buf) -> Vec<ByteRange>` (`:327`) — whole-buffer, sorted,
  empty when no store. The one-call read a renderer wants.
- `FoldStore::containing(p) -> Vec<ByteRange>` (`:171`) — byte-space containment,
  `(start, end]` (start-exclusive, end-inclusive per `src/fold.rs:11`).
- **There is NO line-space query** ("is source line N hidden?"). Only byte-space
  `containing` exists. Stage 2 must add the line-space view (§4).

A fold `ByteRange` is `start` = end-of-head-line content byte, `end` =
end-of-last-hidden-line content byte (parent §3/§5). So the byte→line conversion
is exact: `head_line = line_at_offset(start)`, hidden lines =
`head_line+1 ..= line_at_offset(end)`, head line and (for brace nodes) the closer
line stay visible. `ByteRange`'s struct doc (`pmacs-protocol/src/ids.rs:72`) calls
itself half-open `[start,end)`; the **fold engine overrides that to `(start,
end]`** — use the fold semantics, never the struct doc.

### 2.3 The gutter

- `LineNumberMode` (`pmacs-protocol/src/message.rs:1173`; `Off`/`Absolute`/
  `Relative`/`Hybrid`; default `Off`) is per-window (`Window.line_numbers`,
  `src/window.rs:188`). The number rule is
  `LineNumberMode::number_for(line, cursor_line)` (`:1197`) — and it measures
  relative distance as **`abs_diff` in raw source-line space**.
- The daemon computes the column in `paint_line_number_gutter`
  (`src/editor.rs:3177`): `buffer_line = view_top + r` (`:3215`) →
  `number_for(buffer_line, cursor_line)` (`:3224`); `cursor_line =
  line_at_offset(window.cursor)` (`:3189`).
- Gutter **width** is `decimal_digits(line_count) + 2` (`src/window.rs:216`),
  sized to the **absolute** line count and fixed per frame. Folding does not
  change `line_count`, so **the gutter width is stable** (no jitter) — a fold does
  not shrink the field.
- Diagnostic signs: `DiagnosticView::render` (`src/diag.rs:496`), `row_offset =
  line − start_line_buf` (`:563`), most-severe-per-row into `line_markers`
  (`:570`), painted by `paint_line_markers` (`:635`) with
  `DiagnosticSeverity::gutter_glyph()` (`:75`) at window col 0 (the gutter's
  leading cell). Diagnostics render **after** the number gutter (overlay order,
  `src/editor.rs:2943-2954`), so a sign is not erased by the digits.
- Gutter column order across `[origin.col .. origin.col+gutter_w)`:
  **[sign @ col 0 | right-aligned digits | content @ origin+gutter_w]**. Every
  painter reads `Viewport.gutter_w` (`src/view.rs:144`) so it stays gutter-blind.

### 2.4 The interactive-edit unfold seam (correcting the parent's premise)

Stage 1 wired the pre-edit unfold as `unfold_before_point_edit`
(`src/editor_core.rs:1847` — reads `active_window().cursor`, calls
`fold_registry.unfold_containing(id, point)`) called at the **top** of the six
`EditorCore` primitives (`insert_char` :1858, `insert_char_over_region` :1894,
`backspace` :1936, `delete_forward` :1958, `delete_word_backward` :1982,
`delete_word_forward` :2010). The shared `apply_active_edit`
(`src/editor_core.rs:1266`) they all call does **not** itself unfold.

The parent framing (R3-3) assumed yank/query-replace/comment-toggle all "mutate
through the Lua mutator path." **The scout corrected this:**

- **Yank (`C-y`)** and **query-replace** are **Rust `apply_active_edit`
  callers**, not Lua-mutator callers: yank →
  `clipboard_paste`→`insert_bytes_over_region`→`apply_active_edit`
  (`src/editor_core.rs:2544/2570`); query-replace →
  `query_replace_apply_current`→`apply_active_edit` (`:1129/1137`). They skip the
  six primitives, so they do **not** unfold today — but they are inherently
  **local active-frontend** edits (`apply_active_edit` is never the path for a
  remote/optimistic-CRDT apply).
- **Comment-toggle** (`comment.lua:172`, one `buf:replace`) and **yank-pop**
  (`killring.lua:356`) take the **Lua mutator path**: `add_mutation_methods`
  (`src/lua_bindings/mod.rs:1227`) → `run_managed_edit` (`:1347`) →
  `Buffer::apply_edit_skip_intercepts` → `EditorCore::notify_buffer_edit`
  (`src/editor_core.rs:1364`). This path has no unfold — **and it is shared with
  remote/optimistic-CRDT applies**, which are the parent's named Stage 3 concern.

The dispatcher stamps `active_frontend = frontend_id` **before** any command body
runs (`src/editor.rs:807`; also `src/daemon.rs:999`), so the acting frontend and
its point (`active_window().cursor`) are available throughout an interactive
command — the exact pair `unfold_before_point_edit` already reads. There is a
narrower one-shot provenance record (`TypedEditRecord`/`take_typed_edit`,
`src/editor_core.rs:161/2477`) used by auto-pairing, but it is codepoint-scoped
self-insert only and cannot carry these multi-byte edits — Stage 2 needs a
position-keyed signal, not that record.

### 2.5 Scroll/viewport commands all count raw lines; there is no recenter

`move_page_down`/`move_page_up` (`src/editor_core.rs:1745/1779`, step =
`last_visible_rows − 1`), `scroll_window` (mouse wheel, `src/editor.rs:2264`,
`SCROLL_LINES = 3`), and `move_to_line`/goto-line (`src/editor_core.rs:708`) all
compute targets in **raw source-line** space via `Window::text_view` and mutate a
raw-line `view_top`. There is **no `beginning/end-of-buffer` command** and **no
recenter** — recenter is explicitly deferred, blocked on viewport facts (the GPU
never consumes daemon `view_top`); confirmed in
`docs/editing-conveniences-framing.md:217` and the handoff §6. Stage 2 inherits
that deferral (recenter stays out).

## 3. The shared fold-aware line map (Q#FD12)

Stage 2's spine is one primitive, derived per window per frame from
`state.fold_registry.folds(buffer_id)` and the buffer's line offsets (already
computed for rendering). It answers, in **line space**:

- `is_hidden(line) -> bool` — is this source line inside some fold's hidden
  interval `head+1 ..= last_hidden`?
- `head_of(line) -> Option<line>` — if hidden, the visible head line of its
  (innermost) enclosing fold.
- `next_visible(line)` / `prev_visible(line)` — walk to the next/previous
  **visible** source line (skipping whole folds).
- `visible_between(a, b) -> usize` — count of visible lines in `[a, b)`, for
  relative distance and paging.
- `first_visible_at_or_after(line)` — clamp a candidate `view_top` to a visible
  line.

Implementation shape: convert `folds()` to a sorted, non-overlapping set of
**hidden-line intervals** (nested folds collapse to the union of their hidden
lines — a hidden line is hidden regardless of which fold owns it). Byte→line uses
the render path's existing line-offset table; the fold set is O(top-level blocks)
(parent B2), so the build is cheap (**Bet B4**). The map is a *derived per-frame
view*, not new stored state — the byte-range store in `src/fold.rs` stays the one
source of truth; nested/overlap normalization happens in the derivation.

**`view_top` stays a source-line index** (Q#FD12, **Bet B5**): it is only ever
clamped to a *visible* line (`first_visible_at_or_after`). Keeping it in
source-line space avoids a second coordinate space and preserves the saveplace
contract (`view_top` is persisted, `saveplace.lua:60`) and the existing
`_view_top`/`set_view_top` Lua surface. The visible-line *ordinal* is derived
where needed (line numbers, paging), never stored.

Home of the primitive: a Stage-2 render-side helper (e.g. `src/fold_view.rs` or a
function on the render path) that takes `&FoldStore`/`folds()` + line offsets and
yields the queries above. It is **not** put on `FoldStore` itself — the store is
byte-space and frontend-agnostic; the line-space view is a rendering concern. (A
thin convenience like `FoldStore::hidden_line_intervals(&line_offsets)` may live
in `fold.rs` if it keeps the byte→line conversion beside the containment
convention it must match; TBD in implementation, not load-bearing.)

## 4. Collapse rendering (Q#FD13)

**Threading.** `TextView::render` (`src/text_view.rs:207`) takes only `(buf,
viewport, cells)` and has no fold handle. The fold-aware line map is carried on
**`Viewport`** (widening it beside `gutter_w`) so that `TextView::render` *and*
every overlay painter (syntax, diagnostics, selection, search) see the **same**
map and their rows stay aligned. `paint_frame` builds the map (it holds
`state.fold_registry` + `window.buffer_id`) and installs it on the `Viewport` it
already constructs (`src/editor.rs:2932`). This mirrors how `gutter_w` is
threaded to keep painters gutter-blind (§2.3) — now they become fold-blind too,
consulting the map rather than each re-deriving it.

**The loop.** `TextView::render` advances over **visible** lines: row `r` shows
the `r`-th visible source line at/after `view_top` (via `next_visible`). Hidden
lines are skipped; rows past the last visible line clear as today.

**Head line + ellipsis.** A visible line that is a fold head renders its real
content, then a trailing ellipsis marker (` …`) in the **content area** after its
text (clipped like any long line). The ellipsis is the authoritative,
layout-neutral fold indicator.

**Overlays.** Syntax spans, diagnostics, selection washes, and search hits all
paint per row through `Viewport`; because they now read the same visible map, a
span/wash on a hidden line is simply not painted (its row does not exist), and a
span on a visible line lands on the right row. No overlay tries to delete a row —
the row set is already fold-correct by the time overlays run.

## 5. Gutter: line numbers + fold glyph (Q#FD14, Q#FD20)

**Line numbers (Q#FD14).** `paint_line_number_gutter` (`src/editor.rs:3177`)
walks visible lines (row `r` → the `r`-th visible line at/after `view_top`, same
map as §4):

- **Absolute** shows that visible line's raw `line + 1` — unchanged rule; hidden
  numbers simply do not appear, so the column jumps from the head's number to the
  first post-fold number with no gap-filling.
- **Relative / Hybrid** measure distance in **visible** lines:
  `visible_between(cursor_line, row_line)` (signed by direction), **not**
  `number_for`'s raw `abs_diff`. Concretely, the caller feeds `number_for` a
  *visible* distance (and, for Hybrid's cursor row, the raw `line + 1`). Absolute
  keeps needing the raw line, so the walk carries both the raw source line and the
  visible ordinal.

Gutter **width** is unchanged (`decimal_digits(line_count)+2`, §2.3) — sized to
the absolute count, so it neither jitters nor shrinks when a fold collapses.

**Fold glyph (Q#FD20).** Parent §7 promises a gutter fold marker. To honor it
with **zero layout change** and no contention: on a head-line row, draw a fold
marker in the gutter's **col-0 sign cell** *only when that row carries no
diagnostic sign*. A diagnostic clamped onto the head (Q#FD15) is higher-signal
("there is an error inside this fold") and wins col 0; otherwise the fold glyph
occupies it. This adds no gutter column and keeps the width rule intact. (A
dedicated fold column is deferred, §9 — it would widen the gutter and fight the
sign cell.)

## 6. Diagnostic signs on hidden lines (Q#FD15)

`DiagnosticView::render` (`src/diag.rs:496`), before recording a marker for a
source `line` (`:563/:570`): if `is_hidden(line)`, **remap it to `head_of(line)`**
(clamp to the visible head-line row) rather than dropping it. The existing
most-severe-per-row merge (`:570`) then makes the head row show the most severe
sign among the head's own diagnostics and every hidden line under its fold. Clamp
(not drop) preserves the "there is a problem inside this collapsed region" signal
— the reason a fold marker exists.

`DiagnosticView` reaches the map the same way `TextView` does — through
`Viewport` (§4) — so it needs no separate fold-registry handle and every overlay
stays uniformly fold-aware. Its resolved head row then routes through the same
visible map as the numbers, so sign and number agree on the row.

## 7. Caret, click, viewport, and motion (Q#FD16, Q#FD17, Q#FD18)

**Caret clamp (Q#FD16).** The caret grid-row (`src/editor.rs:3033-3044`): if the
logical cursor's source line `is_hidden`, render the caret at `head_of(line)`'s
row (then map through the visible map like any row). This satisfies the parent's
per-cursor render-time invariant (Q#FD3) — including the shared-store case where
another frontend folded a region around this frontend's cursor.

**Click inverse (Q#FD16).** `activate_and_position` (`src/editor.rs:2228`): a
click on grid row `k` maps to the `k`-th visible line (via the map), so a click
can never land the cursor on a hidden line.

**Viewport / scroll (Q#FD18).** `view_top` is clamped to a visible line
(`first_visible_at_or_after`) wherever it is set. Paging and wheel scroll count
**visible** rows: `move_page_down`/`move_page_up`
(`src/editor_core.rs:1745/1779`) advance `view_top` and the cursor by a screenful
of *visible* lines (walk `next_visible`/`prev_visible` `page_step` times);
`scroll_window` (`src/editor.rs:2264`) advances by visible lines; the
auto-scroll-to-cursor clamp (`src/editor.rs:2866`) keeps the cursor's *visible*
row within the viewport. `move_to_line`/goto-line targets a raw line; if that
line is hidden, `view_top` is set so the target's **head** is visible (Stage 2
does not auto-unfold on goto — revealing-by-unfold is the deferred search-reveal,
§9). Recenter stays deferred (§2.5).

**Cursor line-motion (Q#FD17) — SCOPE DECISION, recommendation: include.**
`next-line`/`prev-line` (and the arrow keys) today step to the same column on the
adjacent **source** line. Recommended for Stage 2: they step to the adjacent
**visible** line (`next_visible`/`prev_visible`), so a collapsed region is one
motion step and the cursor never *rests* on a hidden line — the caret clamp
(Q#FD16) becomes a backstop for the shared-fold case rather than the primary
guard, and the visible-line map is already built. This is small (it reuses the
map at the line-motion functions in `editor_core`) and makes the collapse feel
coherent. **Alternative (smaller Stage 2):** leave line-motion in raw source-line
space; rely solely on the render-time caret clamp; a down-arrow into a fold lands
the logical cursor on a hidden line (drawn clamped), and the next edit unfolds.
This is less code but a confusing "cursor entered the fold invisibly" UX. *This
is the one scope fork I most want your ruling on.*

## 8. Interactive-Lua-command unfold widening (Q#FD19)

The parent's Stage 2 obligation (R3-3): widen the pre-edit unfold beyond the six
`dispatch_key` primitives to the interactive Lua commands. Per §2.4 the targets
split across two funnels, and the widening must stay **local-interactive only** —
it must NOT unfold on the remote/optimistic-CRDT apply path (parent's Stage 3
"CRDT-origin unfold").

- **`apply_active_edit` funnel (yank, query-replace, and the six).** Move the
  pre-edit unfold to the **top of `apply_active_edit`** (`src/editor_core.rs:1266`),
  keyed on the active frontend's point (as `unfold_before_point_edit` already is).
  This subsumes the six primitives' individual calls (they are then **retired** —
  one funnel) and covers yank (`clipboard_paste`→`insert_bytes_over_region`) and
  query-replace (`query_replace_apply_current`) for free. `apply_active_edit` is
  never the remote-apply path, so this funnel is inherently local — no CRDT
  leakage.
- **Interactive Lua-mutator funnel (comment-toggle, yank-pop).** These reach
  `run_managed_edit` (`src/lua_bindings/mod.rs:1347`). Unfold there **only when the
  edit runs inside an interactive-command dispatch for the active frontend**, keyed
  on the requested edit's start position (`edit.range.start`). Gating on the
  interactive-command context is what keeps a plain programmatic `buf:insert` (a
  plugin, or the Stage-1 data-API `pmacs.fold.fold`) from unfolding — matching
  Stage 1's named data-API exemption ("programmatic, no invoking point"). The
  dispatcher already stamps `active_frontend` and rotates command boundaries
  around interactive commands (`this_command`/`invoke_interactive`), so the
  "interactive command in flight for frontend F" context is available; Stage 2
  consults it, adding at most a small dispatch-scoped marker if none is directly
  readable.
- **Explicitly EXCLUDED (Stage 3):** the shared `notify_buffer_edit`
  (`src/editor_core.rs:1364`) remote/optimistic-CRDT apply path. A remote peer's
  edit inside *my* fold must not unfold it; a GPU user's own optimistic-CRDT edit
  at *their* point is the parent's Stage 3 obligation, wired when the GPU renders
  folds.
- **Undo/redo (named minor).** `undo`/`redo` reach the invalidation funnel
  directly (`src/editor_core.rs:2062/2102`), not `apply_active_edit`. They are
  local active-frontend operations; Stage 2 **may** unfold at the edit position on
  undo/redo (desirable — an undo that re-touches a folded region should reveal it)
  or defer it. Not a blocker; decided in implementation.

Every path is keyed on the **edit position inside a fold**, and every widened
behavior is **bite-verified** (a test that fails without the widening).

## 9. Deferred (named)

- **Stage 3 (GPU):** GPU collapse, caret/hit-test fold-awareness at TUI parity,
  the `BufferSnapshot` fold-mirror clear (parent R2-4), and CRDT-origin /
  GPU-optimistic interactive unfold (parent R2-3).
- **Recenter** and any frontend scroll control — blocked on viewport facts (the
  GPU not consuming `view_top`); Arc 8 adjacent (§2.5).
- **Search-reveal** — a match (isearch/query-replace) inside a fold auto-unfolds
  to show the hit. Parent §11 marks it "Stage 2+"; deferred here to keep Stage 2
  focused (goto-line likewise does not auto-unfold, §7).
- **A dedicated gutter fold column** — Q#FD20 uses the existing col-0 sign cell;
  a separate column (and its width recompute) is deferred.
- **Fold-aware horizontal/word motion** and screen-line editing semantics beyond
  vertical line-motion.
- **Persisted folds, `hide-level N`, auto-fold-on-open** (parent §11).
- **Fold-store revalidation on revert/reload** (parent §11; v1 still drops it).
- If Q#FD17 is decided "alternative," fold-aware vertical line-motion moves here.

## 10. Bets

- **B4** The per-frame visible-line map (derived from `folds()` + line offsets) is
  cheap: folds are O(top-level blocks), byte→line reuses the render path's line
  table. No incremental/stored map needed. FALSIFIABLE on a pathological
  many-fold buffer (mitigation: the derivation is O(folds), not O(lines)).
- **B5** Keeping `view_top` a source-line index (clamped to visible) is less churn
  and preserves the saveplace/`set_view_top` contracts, versus introducing a
  visible-ordinal coordinate space.
- **B6** No wire schema or protocol change: `FoldState` production (Stage 1) is
  untouched; the TUI collapse is purely daemon-side; a semantic GPU session still
  gets `FoldState` and renders nothing new until Stage 3.

## 11. Acceptance — Stage 2

Tests assert on the **rendered cell grid** through a real-daemon / real-TUI grid
harness (the vterm-Stage-2 real-PTY smoke and the UX-gutter daemon-acceptance are
the precedents), sized for macOS startup + long temp-path width (handoff §5).

1. **Collapse.** Fold a region; the daemon frame omits the hidden source lines,
   the head line shows its text + ellipsis, rows below shift up, and the content
   row count equals the visible-line count.
2. **Head marker.** The head-line row shows the ellipsis (content area) and — when
   no diagnostic clamps there — the gutter fold glyph; hidden rows are absent.
3. **Line numbers.** Absolute mode skips hidden numbers (head's number, then the
   first post-fold number, no gap-fill). Relative/Hybrid distance is measured in
   **visible** lines across a fold (a line two visible rows past the cursor shows
   `2` even if a fold hides ten source lines between them).
4. **Diagnostic clamp.** A diagnostic on a hidden line surfaces as the sign on the
   head-line row; most-severe wins across the head and all its hidden lines; no
   sign on an absent row.
5. **Caret clamp.** With the logical cursor on a hidden line (folded via the
   shared store from a second frontend), the caret renders on the head-line row; a
   click on a grid row never selects a hidden line.
6. **Viewport / paging.** Page-down advances by a screenful of **visible** lines
   across a fold; `view_top` never lands on a hidden line; scrolling with a fold at
   the top clamps to its head; goto-line to a hidden line leaves the target's head
   visible.
7. **Interactive unfold widening (Q#FD19).** A **yank** at a point inside a fold
   unfolds it before the paste is visible; likewise a **query-replace**
   replacement and a **comment-toggle** whose edit point is inside a fold. A
   **programmatic** `pmacs.fold`-adjacent `buf:insert` inside a fold does **not**
   unfold (it translates). Each assertion is **bite-verified**; the test documents
   that CRDT-origin unfold remains the Stage 3 obligation.
8. **(If Q#FD17 = include)** `next-line`/`prev-line` step across a collapsed region
   as one motion; the cursor never rests on a hidden line.
9. **No wire/protocol change.** A semantic session still receives `FoldState`
   exactly as in Stage 1; the protocol version is unchanged; the GPU render path
   is untouched. (Pin: the Stage-1 `FoldState` producer transitions test still
   passes verbatim.)
10. **Shared store, independent viewports.** Two TUI windows on one buffer both
    render the same fold collapsed, each with its own correct `view_top`, gutter,
    and caret — folds are per-buffer/shared, viewports are per-window.

## 12. Gates (Stage 2)

`cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D warnings` (own
step); `cargo test --lib`; `cargo test --lib --features crdt`;
`tests/folding_acceptance.rs` (Stage-1 suite, must stay green) + the new
`tests/folding_stage2_acceptance.rs` (default + CRDT); `cargo test --test
m4_acceptance -- --skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test -p
pmacs-gpu` (must stay green — Stage 2 does not touch the GPU); the workspace
sweep as one invocation; `git diff --check`. New behavioral acceptance is
bite-verified with `scripts/bite`. Flaky-under-load timing tests are rerun
isolated before treating a sweep failure as a regression (handoff §3).

## 13. Numbered decisions (continuing the parent's Q#FD scheme)

- **Q#FD12** Stage 2 introduces the first non-identity source-line↔display-row
  map. It is centralized in one per-frame **visible-line map** (derived from
  `folds()` + line offsets), consulted by the render loop and every
  view_top-arithmetic site; `view_top` stays a source-line index clamped to a
  visible line. (§3)
- **Q#FD13** Collapse lives in `TextView::render` (row `r` → `r`-th visible line),
  with the map carried on `Viewport` so all overlay painters are uniformly
  fold-aware; head line renders content + trailing ellipsis; folding is not an
  overlay. (§4)
- **Q#FD14** Line numbers walk visible lines; Absolute uses the raw `line+1`,
  Relative/Hybrid measure distance in **visible** lines (not raw `abs_diff`);
  gutter width unchanged (sized to absolute count, no jitter). (§5)
- **Q#FD15** A diagnostic on a hidden line **clamps to the fold head row**
  (most-severe merge across head + hidden lines), not dropped. (§6)
- **Q#FD16** Render-time caret clamp to the head for a cursor on a hidden line;
  click inverse maps grid rows to visible lines. (§7)
- **Q#FD17** *(scope fork — needs your ruling)* Recommended: vertical line-motion
  (`next-line`/`prev-line`/arrows) steps over folds by visible lines so the cursor
  never rests hidden. Alternative: raw motion + render-time clamp only. (§7)
- **Q#FD18** Viewport/paging/auto-scroll count **visible** lines; `view_top`
  clamped to visible; goto-line leaves a hidden target's head visible; recenter
  stays deferred. (§7)
- **Q#FD19** Interactive-unfold widening hooks the **local** funnels only:
  `apply_active_edit` (top, subsuming the six primitives; covers yank +
  query-replace) and the interactive Lua-mutator path (comment-toggle, yank-pop)
  gated on interactive-command context + edit position; the remote/optimistic-CRDT
  `notify_buffer_edit` path is **excluded** (Stage 3); undo/redo unfold is a named
  minor. (§8)
- **Q#FD20** Fold gutter glyph reuses the col-0 sign cell on the head row when no
  diagnostic clamps there (diagnostic wins); no new gutter column. (§5)

## 14. Branch and PR plan

Branch **`folding-tui`**, worktree `../pmacs-folding-tui`, off canonical `main` @
`c49a8c7` (folding Stage 1 / #142 merged). This framing is the opening commit;
Stage 2 implements on this same branch and opens as the second folding PR. One
feature, one branch, one PR — Stage 3 (GPU) is a separate branch/PR off the main
resulting from this stage, re-framed after it lands.

Housekeeping owed from #142 (tracked separately, not folded into this PR): the
`docs/active-work.md` folding lane and `docs/agent-handoff.md` §1 still predate
the #142 merge and want a docs PR refresh (the #138–#140 convention).
