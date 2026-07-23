# Folding Stage 2 — grid (daemon-rendered) collapse — framing (Arc 6)

**Revision 2 — 2026-07-23. Status: DRAFT for review (rev 1's five findings
addressed).** Parent architecture (`docs/folding-framing.md`, rev 5) is APPROVED
and Stage 1 (the headless fold engine) is MERGED as **#142** (canonical `main` @
`c49a8c7`). This doc reframes **Stage 2** in detail off that base, per the
parent's §8/§14. Numbering continues the parent's `Q#FD…` scheme from `Q#FD12`.

## 0. Revision history

### Round 1 (rev 1 → rev 2)

- **F1 (major) — nested folds resolved to the wrong head.** Rev 1's `head_of`
  returned the *innermost* enclosing fold's head and assumed it visible; under
  nesting an outer fold hides the inner head, so carets/diagnostics/relative
  numbers would have clamped onto another *hidden* line. Rev 2 defines
  **`visible_head_of(line)` = the head of the *outermost* enclosing fold** (the
  only head guaranteed visible), used by every clamp. A hidden `view_top` clamps
  **backward** to that head (not forward past the fold, which contradicted
  acceptance 6). Relative/Hybrid numbering anchors on the **clamped visible
  cursor**. New nested-fold / shared-cursor acceptance (§11).
- **F2 (major) — the mapping-consumer census was incomplete.** Rev 1 claimed all
  painters route through `Viewport`. They do not: local selection
  (`src/editor.rs:3241`) and the mode-line scroll indicator (`src/editor.rs:3803`)
  paint from `paint_frame` with raw `view_top` arithmetic; peer cursor/selection
  (`src/overlay_paint.rs:159`) runs *after* `paint_frame` and subtracts `view_top`
  independently; the style overlay (`src/overlay.rs:385`), search wash
  (`src/search.rs:492`), diagnostics (`src/diag.rs:563`), and the completion
  anchor each derive `start_line + row_offset`. Rev 2 carries the **complete
  consumer census** (§3.1), adds **TUI peer-presence fold behavior** (clamp a
  hidden peer cursor to the visible head; drop/project hidden selection cells) as
  explicit scope, and expands acceptance to pin local selection, peer presence, an
  ordinary style/search overlay across a fold, completion anchoring, and the
  scroll indicator.
- **F3 (major) — the fold glyph's sign cell does not exist by default.** Line
  numbers default to `Off` ⇒ `gutter_width()==0` (`src/window.rs:216`); diagnostics
  then fall back to a col-0 *background* on the first content cell
  (`src/diag.rs:635`), so rev 1's "zero-width-change" gutter glyph had nowhere to
  render. Rev 2 makes the fold glyph **conditional on a gutter existing**
  (Q#FD20): gutter off ⇒ **ellipsis only**; gutter on ⇒ reuse the sign cell with
  diagnostic priority. Acceptance covers both. The parent's unconditional
  gutter-marker promise, if required, needs a **dedicated sign column** and an
  acknowledged layout change (named alternative, §5/§9).
- **F4 (major) — a per-frame map instance cannot serve command-time motion, and
  `Viewport` is `Copy`.** `move_up/down`, paging, wheel scroll, and clicks execute
  **outside** the render frame, so they cannot reuse a map installed on that
  frame's `Viewport`; and `Viewport` is compile-time-pinned `Copy`
  (`src/view.rs:130`). Rev 2 reframes the map as **one shared derivation/query
  primitive** with **separate short-lived instances** for rendering vs
  command/event handling (Q#FD12); the render instance is threaded as
  `Option<&'a VisibleLineMap>` on a **lifetime-bearing `Viewport<'a>>`** (a shared
  ref is `Copy`, so `Viewport` stays `Copy`); the primitive's home is usable from
  `EditorCore`, not render-only.
- **F5 (moderate) — tighter unfold seam + settled undo decision.** Rev 2 keys the
  Lua-path widening on the existing **`InteractiveCommandOrigin`**
  (`src/editor.rs:53`), not command-history inference; hooks the **common
  `run_buffer_edit`** (`src/lua_bindings/mod.rs:1305`) so interactive
  `bypass_intercept` mutations do not escape; **requires the target to be the
  invoking frontend's active-window buffer** (an explicit inactive-buffer Lua
  mutation stays programmatic); keeps the Rust hook at `apply_active_edit` and the
  `notify_buffer_edit` exclusion; and **explicitly defers undo/redo unfold**
  (Q#FD19).

### Rulings absorbed

- **Q#FD17 = INCLUDE.** Vertical motion steps through *visible* lines; if motion
  begins from a hidden logical cursor (shared fold or goto-line), it **first
  normalizes to the visible head**, then steps to the preceding/following visible
  line (§7).
- **#142 housekeeping** (retire the active-work.md folding lane + refresh handoff
  §1) is a **separate docs PR**, kept out of `folding-tui`.

## 1. What Stage 2 ships

The grid TUI is **daemon-rendered**: the daemon walks a buffer's source lines and
paints a character grid it ships to the terminal client. Stage 1 built the
instance-side fold store and produces `FoldState` for GPU sessions, but **no
frontend renders a collapse yet** — the daemon grid path never consults the fold
store.

Stage 2 makes the **daemon grid renderer fold-aware**:

1. **Collapse** — omit each fold's hidden source lines; head line shows a trailing
   ellipsis; rows below shift up.
2. **Gutter fold marker** — a fold glyph on the head-line row **when a gutter
   exists** (Q#FD20); ellipsis-only otherwise.
3. **Fold-aware line numbers** — the `LineNumbers` family skips hidden lines;
   relative/hybrid distance is measured across the collapse, anchored on the
   clamped visible cursor.
4. **Fold-aware diagnostic signs** — a sign on a hidden line clamps to the fold's
   **visible head** row (most-severe merge).
5. **Fold-aware caret, local selection, and peer presence** — no caret, no
   selection cell, and no peer cursor renders on a hidden line; each clamps to the
   visible head or is projected/dropped.
6. **Fold-aware viewport/scroll/motion** — `view_top`, paging, wheel scroll,
   clicks, auto-scroll, vertical line-motion, and the mode-line scroll indicator
   all reckon in **visible** lines.
7. **Interactive-Lua-command unfold widening** — yank, query-replace, and
   comment-toggle unfold a fold at their edit point before the edit is visible.

**No wire schema changes and no protocol bump.** `FoldState` production (Stage 1)
is untouched; the TUI collapse is entirely daemon-side (the vterm-Stage-2 shape);
the GPU render path is Stage 3.

## 2. Ground truth (scouted + review-verified 2026-07-23, `main` @ `c49a8c7`)

### 2.1 The grid render path is layered — collapse belongs deep in it

`RenderState::render_frame` (`src/instance_render.rs:98`) is a **diff shell** (it
double-buffers cells, emits `CellDelta`); it does not walk source lines. The
source-line→grid-row loop is two calls down:

```
RenderState::render_frame  (src/instance_render.rs:98)   — diff shell
  → paint_frame            (src/editor.rs:2796)          — per-window composition; HAS state.fold_registry
      → window.text_view.render(buf, viewport, grid)     (src/editor.rs:2948)
          → TextView::render (src/text_view.rs:207)      — the line→row loop
```

`TextView::render` (`src/text_view.rs:213`) maps `line = start_line + row_offset`
— **strictly identity**, no skip, no wrap (long lines truncate). `Window.view_top`
(`src/window.rs:179`) is a **source-line index**. `DisplayCoord`'s doc
(`src/view.rs:108`) anticipates a non-identity map "once virtual lines, wrapping,
and inline expansions appear," but nothing implements one today — **folding is the
first**.

### 2.2 The complete set of identity-assuming consumers (F2)

Every site below assumes `display_row = source_line − view_top` (or the inverse)
and must consult the shared visible-line map. Rendering-frame sites, after-frame
sites, and command-time sites are distinguished because they need **different
instances** of the map (§3, F4):

| # | Site | file:line | phase |
|---|---|---|---|
| 1 | text render loop | `src/text_view.rs:214` | render (Viewport) |
| 2 | line-number gutter | `src/editor.rs:3215` | render (paint_frame) |
| 3 | style/syntax overlay | `src/overlay.rs:385` | render (Viewport) |
| 4 | diagnostics overlay | `src/diag.rs:563` + `paint_line_markers` `:635` | render (Viewport) |
| 5 | search wash overlay | `src/search.rs:492` | render (Viewport) |
| 6 | completion popup anchor | completion overlay (byte→row) | render (Viewport) |
| 7 | caret grid row | `src/editor.rs:3044` | render (paint_frame) |
| 8 | local selection | `paint_local_selection`, `src/editor.rs:3241` | render (paint_frame) |
| 9 | mode-line scroll indicator | `format_scroll_indicator`, `src/editor.rs:3803` | render (paint_frame) |
| 10 | peer cursor + selection | `src/overlay_paint.rs:159` | **after** paint_frame |
| 11 | click inverse | `activate_and_position`, `src/editor.rs:2233` | command/event |
| 12 | auto-scroll clamp | `src/editor.rs:2866` | command/event |
| 13 | paging / wheel / vertical motion | `src/editor_core.rs:1745/1779`, `src/editor.rs:2264`, `move_up/down` | command/event |

Overlays (1,3,4,5,6) already thread through `Viewport`; sites 2,7,8,9 run in
`paint_frame` directly; site 10 runs after `paint_frame` and re-derives
`gutter_w`/`view_top` itself (`src/overlay_paint.rs:143-160`); sites 11–13 run
during input dispatch, entirely outside any render frame.

### 2.3 The renderer can reach the fold store

`fold_registry: SharedFoldRegistry` is a field on `EditorCore`
(`src/editor_core.rs:223`) and `EditorState` (`src/editor.rs:110`). Both the
render path (`paint_frame`) and command-time code (`EditorCore` methods) reach it.
The read surface (`src/fold.rs`): `FoldRegistry::folds(buf) -> Vec<ByteRange>`
(`:327`, whole-buffer, sorted); `FoldStore::containing(p)` (`:171`, byte-space,
`(start, end]`). **No line-space query exists** — Stage 2 adds it (§3). A fold's
`start` = end-of-head-line content byte, `end` = end-of-last-hidden-line content
byte, so `head_line = line_at_offset(start)`, hidden = `head_line+1 ..=
line_at_offset(end)`. Use the fold's `(start,end]` convention, never the
`ByteRange` struct doc's `[start,end)`.

### 2.4 The gutter

- `LineNumberMode` (`pmacs-protocol/src/message.rs:1173`; default **`Off`**) is
  per-window (`Window.line_numbers`, `src/window.rs:188`). Number rule
  `number_for(line, cursor_line)` (`:1197`) uses **raw-line `abs_diff`** for
  relative distance.
- `paint_line_number_gutter` (`src/editor.rs:3177`): `buffer_line = view_top + r`
  (`:3215`) → `number_for(...)`; `cursor_line = line_at_offset(window.cursor)`
  (`:3189`).
- **Gutter width is 0 when line numbers are Off** (`Window::gutter_width`,
  `src/window.rs:216`). When on, width = `decimal_digits(line_count)+2`, fixed to
  the absolute count (no jitter; folding does not change `line_count`).
- Diagnostic signs: `DiagnosticView::render` (`src/diag.rs:496`) →
  `paint_line_markers` (`:635`): `gutter_w>0` draws the sign glyph in the gutter's
  leading cell; **`gutter_w==0` falls back to a col-0 background** on the first
  content cell (the "fake gutter"). So a dedicated sign cell only exists with a
  gutter on (F3).

### 2.5 The interactive-edit unfold seam (F5, correcting rev 1)

Stage 1's `unfold_before_point_edit` (`src/editor_core.rs:1847`, reads
`active_window().cursor`) is called at the top of the six `EditorCore` primitives;
the shared `apply_active_edit` (`:1266`) they call does not itself unfold.

- **Yank / query-replace** are **`apply_active_edit` callers** (local; never the
  remote path): yank → `clipboard_paste`→`insert_bytes_over_region`→`apply_active_edit`
  (`:2544/2570`); query-replace → `query_replace_apply_current`→`apply_active_edit`
  (`:1129/1137`). They skip the six primitives, so they do not unfold today.
- **Comment-toggle / yank-pop** take the Lua mutator path. The common entry is
  **`run_buffer_edit`** (`src/lua_bindings/mod.rs:1305`), which dispatches to
  `run_managed_edit` (`:1347`) *or* `run_bypass_edit` (`:1318`) on the
  `bypass_intercept` flag; both call `apply_edit_skip_intercepts` then
  `notify_buffer_edit_to_windows`. Hooking only `run_managed_edit` would let an
  interactive `bypass_intercept` edit escape — hook `run_buffer_edit`.
- **`InteractiveCommandOrigin`** (`src/editor.rs:53`) is an ephemeral, Lua-app-data
  authenticated origin: `.current() -> Option<FrontendId>` is the frontend while an
  interactive command runs; `.enter(fid)` returns a guard that restores on drop and
  **clears even when a Lua command errors**. This is the scoped authority for the
  Lua-path widening — no command-history inference needed.
- **Undo/redo** reach the buffer through `notify_buffer_edit_to_windows` directly
  (`add_history_methods`, `src/lua_bindings/mod.rs`), not `apply_active_edit` nor
  `run_buffer_edit`.

### 2.6 Scroll/viewport commands count raw lines; no recenter

`move_page_down/up` (`src/editor_core.rs:1745/1779`), `scroll_window`
(`src/editor.rs:2264`), `move_to_line` (`:708`), and the mode-line indicator all
work in **raw source-line** space. There is no `beginning/end-of-buffer` command
and **no recenter** (deferred, blocked on viewport facts; handoff §6). Stage 2
inherits the recenter deferral.

## 3. The shared visible-line map primitive (Q#FD12)

Stage 2's spine is **one derivation/query primitive** — a `VisibleLineMap` type
plus a builder — computed from `state.fold_registry.folds(buffer_id)` and the
buffer's line offsets. It is **not one instance pinned to a frame**: it is built as
**short-lived instances** wherever a source↔display mapping is needed (F4), and its
home is a module usable from **both** the render path and `EditorCore` (not
render-only). Candidate home: `src/fold_view.rs`, or a `FoldStore`
convenience that returns hidden-line intervals given the line-offset table (the
byte→line conversion then lives beside the `(start,end]` convention it must match).
The byte-range store in `src/fold.rs` stays the single source of truth; the map is
derived, never stored.

### 3.1 Queries

Folds may nest; the derivation unions their hidden intervals into a sorted,
non-overlapping hidden-line set `H` (a line is hidden regardless of which fold owns
it). The map answers, in **line space**:

- `is_hidden(line) -> bool` — `line ∈ H`.
- **`visible_head_of(line) -> line`** — for a hidden `line`, the head of the
  **outermost** enclosing fold (resolve to the enclosing fold's head; if that head
  is itself hidden, recurse; heads strictly decrease, so it terminates on the one
  visible head). For a visible `line`, itself. **Every clamp uses this** — caret,
  diagnostics, peer cursor, relative-number cursor anchor, and the `view_top`
  backward clamp. (Rev 1's innermost `head_of` is removed — F1.)
- `next_visible(line)` / `prev_visible(line)` — the next/previous visible line,
  skipping whole folds (for the render walk and vertical motion).
- `visible_between(a, b) -> isize` — signed count of visible lines from `a` to `b`
  (relative/hybrid distance; paging).
- **`clamp_view_top(line) -> line`** — if `line ∈ H`, `visible_head_of(line)`
  (**backward**, so a fold at the top shows its head — acceptance 6); else `line`.
  This replaces rev 1's forward `first_visible_at_or_after`, which skipped past the
  fold and hid the head.

Build cost is O(folds) (folds are O(top-level blocks), parent B2), reusing the
render path's existing line-offset table — **Bet B4**.

### 3.2 Instances per phase (F4)

- **Render frame.** `paint_frame` (`src/editor.rs:2796`) builds one instance and
  (a) threads it to the overlay painters (1,3,4,5,6) as `Option<&'a
  VisibleLineMap>` on a **lifetime-bearing `Viewport<'a>>`** — a shared ref is
  `Copy`, so `Viewport` stays `Copy` (F4); (b) passes the **same** instance
  directly to the in-`paint_frame` sites (2,7,8,9). The `View::render` signature
  becomes `Viewport<'_>`; every construction site gains the field (non-fold callers
  pass `None`). This is a settled compile-time change across the `View` impls and
  `Viewport` constructions.
- **After the frame.** The peer-presence pass (`src/overlay_paint.rs`, site 10)
  runs after `paint_frame` and takes its own arguments; it receives (or builds) a
  fresh instance for the same buffer and clamps peer cursors / projects peer
  selection through it.
- **Command / event.** Click inverse, auto-scroll, paging, wheel, and vertical
  motion (sites 11–13, in `EditorCore` / `EditorState`) each build a short-lived
  instance from `state.fold_registry` + the buffer at call time. "The map is
  already built" (rev 1) was false for these — they run outside any frame.

`view_top` **stays a source-line index** (Q#FD12, **Bet B5**), only ever set via
`clamp_view_top` so it never rests on a hidden line — preserving the saveplace
(`saveplace.lua:60`) and `_view_top`/`set_view_top` contracts. Visible-line
*ordinals* are derived where needed, never stored.

## 4. Collapse rendering (Q#FD13)

`TextView::render` (`src/text_view.rs:207`) advances over **visible** lines via
`next_visible`: row `r` shows the `r`-th visible source line at/after
(already-visible) `view_top`; hidden lines are skipped; trailing rows clear as
today. A visible head line renders its real content then a trailing ellipsis
marker (` …`) in the **content area** (clipped like any long line) — the
authoritative, layout-neutral fold indicator. Overlays (1,3,4,5,6) read the same
map through `Viewport`, so a span/wash/sign/anchor on a hidden line is simply not
painted (its row does not exist) and one on a visible line lands on the right row.
Folding is **not** an overlay — overlays cannot delete rows.

## 5. Gutter: line numbers + fold glyph (Q#FD14, Q#FD20)

**Line numbers (Q#FD14).** `paint_line_number_gutter` (`src/editor.rs:3177`) walks
visible lines (row `r` → the `r`-th visible line). **Absolute** shows that line's
raw `line+1` (hidden numbers just do not appear — the column jumps from the head's
number to the first post-fold number). **Relative/Hybrid** measure distance in
**visible** lines: `visible_between(anchor, row_line)`, where the cursor **anchor**
is `visible_head_of(cursor_line)` (F1 — the cursor may be on a hidden shared-fold
line). Absolute still needs the raw line, so the walk carries both the raw source
line and the visible ordinal. Gutter width is unchanged (§2.4).

**Fold glyph (Q#FD20, F3).** Conditioned on a gutter existing:
- **Line numbers off (`gutter_w==0`, default):** no sign cell exists, so the fold
  marker is the **content-area ellipsis only**; the diagnostic col-0 background
  fallback is unchanged.
- **Line numbers on (`gutter_w>0`):** on a head-line row, draw a fold glyph in the
  gutter's col-0 sign cell **unless a diagnostic clamps there** (Q#FD15) —
  diagnostic wins (an error inside the fold is higher-signal).

This keeps the gutter width rule intact and adds no column. Making the gutter
marker **unconditional** would require a **dedicated fold sign column** and an
acknowledged layout change; deferred (§9). The parent §7's promise is thus honored
*when a gutter is present*, with the ellipsis as the universal indicator.

## 6. Diagnostic signs on hidden lines (Q#FD15)

`DiagnosticView::render` (`src/diag.rs:496`): before recording a marker for source
`line` (`:563/:570`), if `is_hidden(line)` **remap to `visible_head_of(line)`**
(clamp to the outermost visible head, F1), not drop. The existing
most-severe-per-row merge (`:570`) then makes the head row show the most severe
sign among the head and every hidden line under its fold. Clamp (not drop)
preserves the "there is a problem inside this collapsed region" signal.
`DiagnosticView` reaches the map through `Viewport` (§3.2), needing no separate
handle.

## 7. Caret, selection, peer presence, viewport, and motion (Q#FD16, Q#FD17, Q#FD18)

**Caret clamp (Q#FD16).** Caret grid-row (`src/editor.rs:3033-3044`): if the
logical cursor's line `is_hidden`, render at `visible_head_of(line)`'s row —
satisfying the parent's per-cursor render-time invariant (Q#FD3), including the
shared-store case.

**Local selection (Q#FD16, F2).** `paint_local_selection` (`src/editor.rs:3241`):
selection cells on hidden lines are dropped; the visible portion projects through
the map (a selection spanning a fold paints on the visible head row and the visible
tail rows, contiguous on screen).

**Peer presence (Q#FD16, F2).** `src/overlay_paint.rs:159`: a peer cursor on a
hidden line clamps to `visible_head_of`; peer selection cells on hidden lines are
dropped/projected exactly like local selection. Folds are per-buffer/shared, so the
peer's map is the same buffer's map.

**Click inverse (Q#FD16).** `activate_and_position` (`src/editor.rs:2233`): grid
row `k` maps to the `k`-th visible line, so a click never lands on a hidden line.

**Viewport / scroll (Q#FD18).** `view_top` is set only via `clamp_view_top`
(backward to the head for a hidden candidate, F1). Paging
(`move_page_down/up`), wheel (`scroll_window`), and the auto-scroll-to-cursor clamp
(`src/editor.rs:2866`) advance and bound by **visible** lines
(`next_visible`/`prev_visible`, `visible_between`). `move_to_line`/goto-line targets
a raw line; if hidden, `view_top` clamps so the target's **head** is visible (no
auto-unfold — that is the deferred search-reveal, §9). The **mode-line scroll
indicator** (`format_scroll_indicator`, `src/editor.rs:3803`) computes All/Top/Bot/%
in **visible-line** space (visible total, `view_top`'s visible ordinal, cursor's
visible ordinal). Recenter stays deferred (§2.6).

**Vertical line-motion (Q#FD17 — RULED: include).** `next-line`/`prev-line` (and
arrows) step to the adjacent **visible** line (`next_visible`/`prev_visible`), so a
collapsed region is one motion step and the cursor never rests hidden. **If motion
begins from a hidden logical cursor** (after a shared fold or a goto-line into a
fold), it **first normalizes to `visible_head_of(cursor)`**, then steps to the
preceding/following visible line. The render-time caret clamp (Q#FD16) remains the
backstop for the pre-motion shared-fold frame. These reuse the command-time map
instance (§3.2).

## 8. Interactive-Lua-command unfold widening (Q#FD19)

Widen the pre-edit unfold beyond the six `dispatch_key` primitives to the
interactive Lua commands, **local-interactive only** (never the remote/optimistic
CRDT path — parent Stage 3).

- **`apply_active_edit` funnel (yank, query-replace, and the six).** Move the
  pre-edit unfold to the **top of `apply_active_edit`** (`src/editor_core.rs:1266`),
  keyed on the active frontend's point. This subsumes the six primitives'
  individual calls (retired — one funnel) and covers yank + query-replace for free.
  `apply_active_edit` is never the remote-apply path, so this funnel is inherently
  local.
- **Interactive Lua-mutator funnel (comment-toggle, yank-pop).** Hook the **common
  `run_buffer_edit`** (`src/lua_bindings/mod.rs:1305`) — above both
  `run_managed_edit` and `run_bypass_edit`, so an interactive `bypass_intercept`
  edit does not escape (F5). Unfold **only when all** hold: (i)
  `InteractiveCommandOrigin.current()` is `Some(f)` (`src/editor.rs:53`); (ii) the
  edited buffer **is `f`'s active-window buffer** (an explicit inactive-buffer Lua
  mutation stays programmatic — no unfold); (iii) `edit.range.start` is inside a
  fold. Condition (i)+(ii) is what distinguishes an interactive command's edit at
  the point from a plugin's/data-API's programmatic edit (matching Stage 1's
  data-API exemption).
- **Excluded (Stage 3):** the remote/optimistic-CRDT apply path
  (`notify_buffer_edit`, `src/editor_core.rs:1364`). A remote peer's edit inside my
  fold must not unfold it; a GPU user's own optimistic edit is the parent's Stage 3
  obligation.
- **Undo/redo — DEFERRED (F5 ruling).** `undo`/`redo` reach
  `notify_buffer_edit_to_windows` directly; their unfold behavior is **explicitly
  deferred** (§9), not decided at implementation time.

Each widened behavior is **bite-verified** (a test failing without the widening).

## 9. Deferred (named)

- **Stage 3 (GPU):** GPU collapse, caret/hit-test fold-awareness at TUI parity, the
  `BufferSnapshot` fold-mirror clear (parent R2-4), and CRDT-origin / GPU-optimistic
  interactive unfold (parent R2-3).
- **Undo/redo unfold** (F5 ruling) — deferred.
- **Recenter** and any frontend scroll control — blocked on viewport facts; Arc 8
  adjacent (§2.6).
- **Search-reveal** — a match inside a fold auto-unfolds; parent §11 "Stage 2+";
  goto-line likewise does not auto-unfold (§7).
- **A dedicated gutter fold column** (unconditional marker) — its width recompute /
  layout change is deferred; Q#FD20 uses the conditional sign cell.
- **Fold-aware horizontal/word motion** and screen-line editing beyond vertical
  line-motion.
- **Persisted folds, `hide-level N`, auto-fold-on-open**; fold-store revalidation on
  revert/reload (parent §11; v1 still drops it).

## 10. Bets

- **B4** The per-instance visible-line map (from `folds()` + line offsets) is cheap:
  O(folds) build, reusing the render path's line table. Multiple short-lived
  instances per interaction are still O(folds) each. FALSIFIABLE on a pathological
  many-fold buffer (mitigation: derivation is O(folds), not O(lines)).
- **B5** `view_top` stays a source-line index (clamped via `clamp_view_top`) —
  less churn than a visible-ordinal coordinate space, and preserves saveplace /
  `set_view_top`.
- **B6** No wire/protocol change: `FoldState` (Stage 1) untouched; the TUI collapse
  is daemon-side; a semantic GPU session still gets `FoldState` and renders nothing
  new until Stage 3.
- **B7** Threading `Option<&'a VisibleLineMap>` on a lifetime-bearing `Viewport<'a>>`
  keeps `Viewport: Copy`. FALSIFIABLE if a `View` impl or construction site cannot
  satisfy the lifetime; fallback is to settle `Viewport` as non-`Copy` explicitly.

## 11. Acceptance — Stage 2

Tests assert on the **rendered cell grid** through a real-daemon / real-TUI grid
harness (the vterm-Stage-2 real-PTY smoke and the UX-gutter daemon-acceptance are
the precedents), sized for macOS startup + long temp-path width (handoff §5).

1. **Collapse.** Fold a region; the frame omits the hidden lines, the head shows
   text + ellipsis, rows below shift up, content row count = visible-line count.
2. **Head marker, both gutter states (F3).** With line numbers **off**: the head
   shows the ellipsis and **no gutter glyph** (width unchanged, 0). With a
   line-number mode **on**: the head shows the ellipsis **and** the gutter fold
   glyph (unless a diagnostic clamps there — then the diagnostic sign).
3. **Line numbers.** Absolute skips hidden numbers (head's number, then first
   post-fold number, no gap-fill). Relative/Hybrid distance is measured in
   **visible** lines across a fold, anchored on the visible cursor head.
4. **Diagnostic clamp (F1).** A diagnostic on a hidden line surfaces on the
   **outermost visible head** row; most-severe wins across the head and all its
   hidden lines (including a diagnostic on a *nested* inner-fold line clamping to
   the outer head).
5. **Nested-fold / shared cursor (F1).** With a fold nested inside another and the
   logical cursor on a deeply-hidden line (folded via the shared store from a
   second frontend), the caret renders on the **outermost** visible head; relative
   numbers anchor there; a `view_top` set into the nest clamps **backward** to that
   head.
6. **Caret, local selection, peer presence (F2).** Caret on a hidden line → head
   row. A local selection spanning a fold paints on the visible head + visible tail
   rows, nothing on hidden rows. A peer cursor on a hidden line clamps to the head;
   peer selection cells on hidden lines drop/project. A click never selects a hidden
   line.
7. **Ordinary overlays across a fold (F2).** A style/syntax span and a search wash
   on lines straddling a fold paint only on the visible rows, correctly aligned;
   the completion popup anchored below a fold lands on the right visible row.
8. **Viewport / paging / indicator (F2).** Page-down advances by a screenful of
   **visible** lines across a fold; `view_top` never rests hidden; a fold at the top
   clamps to its head; goto-line to a hidden line leaves its head visible; the
   mode-line indicator reports Top/Bot/% in visible-line space.
9. **Vertical motion (Q#FD17).** `next-line`/`prev-line` step across a collapsed
   region as one motion; starting from a hidden logical cursor, motion first
   normalizes to the visible head; the cursor never rests hidden.
10. **Interactive unfold widening (Q#FD19).** A **yank**, a **query-replace**
    replacement, and a **comment-toggle** whose edit point is inside a fold each
    unfold it before the edit is visible; a **bypass-intercept** interactive edit at
    the point also unfolds (proving the `run_buffer_edit` seam). A **programmatic**
    `buf:insert` — and an interactive command mutating an **inactive** buffer — do
    **not** unfold. **Undo/redo do not unfold** (deferred). Each assertion is
    **bite-verified**; the test documents CRDT-origin unfold as the Stage 3
    obligation.
11. **No wire/protocol change.** A semantic session still receives `FoldState`
    exactly as in Stage 1; the protocol version is unchanged; the GPU render path is
    untouched (the Stage-1 `FoldState` producer transitions test passes verbatim).
12. **Shared store, independent viewports.** Two TUI windows on one buffer both
    render the same fold collapsed, each with its own correct `view_top`, gutter, and
    caret.

## 12. Gates (Stage 2)

`cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D warnings` (own
step); `cargo test --lib`; `cargo test --lib --features crdt`;
`tests/folding_acceptance.rs` (Stage-1 suite, stays green) + new
`tests/folding_stage2_acceptance.rs` (default + CRDT); `cargo test --test
m4_acceptance -- --skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`
(stays green — Stage 2 does not touch the GPU); the workspace sweep as one
invocation; `git diff --check`. New behavioral acceptance is bite-verified with
`scripts/bite`; timing-flaky tests are rerun isolated before treating a sweep
failure as a regression (handoff §3).

## 13. Numbered decisions (continuing the parent's Q#FD scheme)

- **Q#FD12** One shared visible-line map **primitive** (derivation + queries from
  `folds()` + line offsets), instantiated as short-lived instances per phase
  (render via `Option<&'a VisibleLineMap>` on `Viewport<'a>>` preserving `Copy`;
  after-frame direct; command-time fresh), home usable from `EditorCore`. `view_top`
  stays a source-line index, set only via `clamp_view_top`. Every consumer in the
  §2.2 census routes through it. (§3)
- **Q#FD13** Collapse in `TextView::render` (row `r` → `r`-th visible line); head
  renders content + trailing ellipsis; folding is not an overlay. (§4)
- **Q#FD14** Line numbers walk visible lines; Absolute uses raw `line+1`,
  Relative/Hybrid measure **visible**-line distance anchored on `visible_head_of`
  of the cursor; gutter width unchanged. (§5)
- **Q#FD15** A diagnostic on a hidden line clamps to `visible_head_of` (outermost
  visible head), most-severe merge; not dropped. (§6)
- **Q#FD16** Render-time clamp to `visible_head_of` for caret, local selection, and
  peer cursor on hidden lines; hidden selection cells drop/project; click inverse
  maps grid rows to visible lines. (§7)
- **Q#FD17** *(ruled: include)* Vertical line-motion steps by visible lines; motion
  from a hidden cursor first normalizes to the visible head. (§7)
- **Q#FD18** Viewport/paging/auto-scroll/indicator reckon in visible lines;
  `view_top` set via `clamp_view_top` (backward to the head); goto-line leaves a
  hidden target's head visible; recenter deferred. (§7)
- **Q#FD19** Interactive-unfold widening hooks **local** funnels only:
  `apply_active_edit` (top; subsumes the six; covers yank + query-replace) and the
  common `run_buffer_edit` gated on `InteractiveCommandOrigin.current()` **and**
  the edit targeting the invoking frontend's active-window buffer **and**
  `edit.range.start` inside a fold; the remote/optimistic-CRDT `notify_buffer_edit`
  path is excluded (Stage 3); **undo/redo unfold is deferred**. (§8)
- **Q#FD20** Fold gutter glyph is **conditional on a gutter existing**: off ⇒
  ellipsis only; on ⇒ col-0 sign cell on the head row with diagnostic priority. A
  dedicated fold column (unconditional marker + layout change) is deferred. (§5)

## 14. Branch and PR plan

Branch **`folding-tui`**, worktree `../pmacs-folding-tui`, off canonical `main` @
`c49a8c7` (folding Stage 1 / #142 merged). This framing is the opening commits (rev
1 → rev 2); Stage 2 implements on this same branch and opens as the second folding
PR. One feature, one branch, one PR — Stage 3 (GPU) is a separate branch/PR off the
resulting main.

Housekeeping from #142 (retire the `docs/active-work.md` folding lane + refresh
`docs/agent-handoff.md` §1) is a **separate docs PR**, kept out of `folding-tui`
(ruled).
