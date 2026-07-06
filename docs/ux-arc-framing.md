# UX arc — framing (the gutter, and what it unlocks)

A UX overhaul arc for pmacs. The diagnostic-surface work (colors, status
counts, wavy squiggles, minimap marks) already shipped (PRs #64–69), so
the next frontier isn't polishing diagnostics — it's the **one layout
primitive both frontends still lack: a gutter column.** Line numbers,
diagnostic *gutter signs* (the last deferred Task #23 item), and later
fold/git-diff markers all live in the gutter. This doc frames the umbrella
UX arc and designs its keystone, the gutter, grounded in a coordinate-
system recon of both frontends.

## Why the gutter is the keystone

Two independently-requested features share one missing foundation:

- **Line numbers** — requested; every editor has them; pmacs has none.
- **Diagnostic gutter signs** — explicitly parked in the diagnostics arc:
  *"no gutter column exists; adding one is a layout-level project,
  deliberately not done."* Today the TUI fakes it with a col-0 background
  marker (`diag.rs`) and the GPU proxies it with minimap marks.

Both need a reserved left column and the coordinate-math to go with it.
Build the column once (per frontend) and both features — plus future fold
markers, git-diff bars, breakpoint dots — become content that rides it.

## The load-bearing bet: no protocol change

The daemon is **pixel-pure** — it never learns screen geometry; the
frontend owns viewport and visual layout (the semantic-frontend contract).
The gutter is therefore **entirely frontend-local**, and the recon
confirms each frontend already holds the data it needs:

- **Line numbers** — the frontend owns the text, so it has line indices.
- **Diagnostic severity per line** — already frontend-side (TUI:
  `DiagnosticView` over the diag store; GPU: `current_decorations`, the
  same source the squiggles use). Map each diagnostic's `range.start` to a
  line, take the max severity per line — no new wire data.

So **zero protocol change, zero daemon change.** The blast radius is two
frontend renderers. (This also means it's implemented *twice* — the TUI
grid and the GPU are separate codebases with no shared render code — so
the design fixes a shared *convention*, §Q#UX7, even though the code is
duplicated.)

## The reserved-width model

A gutter is a fixed-width strip carved from the **left**, mirroring how the
GPU already carves the **right** for the minimap. Text shrinks by the
gutter width; the gutter stays fixed while text scrolls (neither frontend
has horizontal scroll, so "fixed while scrolling" is free — only the
*numbers* printed change with the vertical scroll position).

**TUI** (recon: clean seam). The renderer paints everything relative to a
per-window `Viewport { cell_origin, cell_size }`. The whole gutter is
fundamentally *one shift* at the viewport-construction site
(`editor.rs:1610`): `cell_origin.col += gutter_w`, `cell_size.cols -=
gutter_w`, then paint the gutter into the reclaimed `[rect.origin.col,
+gutter_w)` strip. Every painter that consumes `viewport.cell_origin`
(base text, syntax, diagnostic underline, search) becomes gutter-agnostic
**for free**. The handful that read `rect.origin.col` *directly* each need
a manual `+gutter_w`:
- cursor placement (`editor.rs:1683`), local selection (`:1794`),
  remote-presence cursor/selection (`overlay_paint.rs:160,253`);
- mouse hit-test (`dispatch_mouse`, `editor.rs:866`) subtracts `gutter_w`
  from `local_col`, and clicks with `local_col < gutter_w` are gutter
  clicks (§Q#UX6);
- the diagnostic col-0 sign (`diag.rs:499–586`) relocates into the strip —
  `gutter_glyph()` (`diag.rs:75`) is already defined and unused, waiting
  for exactly this.

`pos_to_display`/`display_to_pos` (the byte↔display-column core) operate
in line-local space and need **no change** — the gutter is applied by
callers crossing into grid space, not in the mapping itself.

**GPU** (recon: one knob). All horizontal geometry hangs off `TEXT_LEFT =
16.0`. A gutter of width `G` is:
- **pixel→byte** — one site: `hit_test_source_byte` (`main.rs:2758`),
  `x - TEXT_LEFT` → `x - TEXT_LEFT - G`;
- **byte→pixel** — four sites, each `TEXT_LEFT` → `TEXT_LEFT + G`: glyph
  render origin (`TextArea.left`, `:3894`), text clip `bounds.left` (`:3898`,
  `0` → `G`), caret (`:4350/4355`), washes+squiggles (`:4413`);
- **placement loop** — walk `layout_runs()` (`run.line_top`, `run.line_i`)
  and draw gutter glyphs/quads at `x ∈ [0, G)`, exactly the minimap's
  mirror on the left. Reserve the band for clicks like `in_minimap_band`.

Neither frontend has horizontal scroll or soft-wrap today, so there is no
scroll-offset interaction to reconcile — the single biggest simplifier.

## Forced decisions

**Q#UX1 — frontend-local, no protocol change. ✗ DISPROVEN (see below).**
Framed as: the data is local in both frontends, so no wire change. That was
half-right — *rendering* is local, but the **control** (`M-x
window.toggle-line-numbers`) lives daemon-side, so the mode must reach the
GUI over the wire. Corrected to: the toggle is daemon-owned per-window
state, shipped to the frontend via a new additive `InstanceMessage::
LineNumbers` variant (**protocol v13**, daemon-gated `< 13`). The GUI still
*renders* locally; it just receives the on/off flag. See the as-built
control-plane note.

**Q#UX2 — reserve on the left, mirror the minimap.** Text area shrinks;
the gutter is a fixed strip. TUI: shift the viewport at one site. GPU: add
`G` to the four byte→pixel sites, subtract at the one pixel→byte site, set
a `text_bounds_left`.

**Q#UX3 — dynamic width, digit-count driven.** `gutter_w = digits(line_
count) + padding` (padding = 1 leading + 1 trailing cell/space typical).
Recomputed as the line count crosses a power of ten. Rejected: fixed width
(wastes space on small files, truncates on huge ones). Line count is
already in hand at the width-computation site in both frontends.

**Q#UX4 — modes: `off | absolute | relative | hybrid`.** `absolute` =
line index + 1. `relative` = distance from the cursor line (Vim-style, for
fast `N j`/`N k`). `hybrid` = current line absolute, others relative (the
popular default). Ships incrementally: **absolute first** (proves the
column + width + coordinate math with zero cursor-coupling), relative/
hybrid as a follow-up (they add a cursor-line dependency + repaint-on-
cursor-move, which is why they come second).

**Q#UX5 — the setting hook + where state lives.** Model on the existing
`frame_target_ms` tunable (`async_runtime.rs:630` + Lua `_frame_target_ms`
/`_set_frame_target_ms`). Line-number mode is naturally **per-window**
(relative numbers frequently are), so the field lives on `struct Window`
(TUI) and is read at paint time; a `pmacs.window`/`pmacs.frontend` binding
sets it, wrapped by a friendly `builtin/` Lua chunk. The GPU reads an
equivalent local setting (it has no window tree, so a single frontend-wide
mode is fine there for v1). Consistency of *value* across frontends is a
convention, not shared code.

**Q#UX6 — gutter click behavior.** MVP: a click in the gutter band selects
the whole line (common editor affordance) — or, if that's too much for the
first cut, is consumed as a no-op (never mis-mapped to a text byte). The
recon shows both frontends can classify a gutter-band click cheaply
(`local_col < gutter_w` / `in_minimap_band`-style). Pick line-select if
it's a few lines; else no-op and defer.

**Q#UX7 — shared convention across the two renderers.** The code is
duplicated, so the design pins the *contract* both must honor: same width
formula (`digits + padding`), same number formatting (right-aligned,
1-based), same mode set, same diagnostic-sign glyphs/severity precedence,
gutter never overlaps the mode line / status band / minibuffer. A short
"gutter contract" section in each frontend's code comments points back
here so they don't drift.

## Sub-arc sequence (each its own PR, each green under both flavors)

1. **Gutter + absolute line numbers.** Introduce the reserved column and
   the coordinate shift in *both* frontends; render absolute numbers. The
   riskiest, most valuable step — it lands the foundation and the
   coordinate math. Validation is a human eyeball per frontend (cursor
   lands on the right glyph after a click; caret draws in the right place;
   selection/overlays don't bleed into the gutter).
2. **Diagnostic gutter signs.** Relocate the TUI col-0 marker into the
   gutter (`gutter_glyph()`) and add GPU gutter glyphs; max-severity per
   line from data already present. Closes the last Task #23 item.
3. **Relative / hybrid line-number modes.** Add the cursor-line dependency
   + repaint-on-cursor-move; a mode toggle on the same machinery.
4. **The rest of the UX backlog** (below), sequenced later.

## The umbrella UX backlog (beyond the gutter)

Named now so the arc has a horizon; not committed, sequenced after the
gutter sub-arcs:
- **Minibuffer polish** — `i/total` hint, Telescope-style preview pane,
  candidate annotations (kind/docstring), multibyte-exact band caret.
- **Editing affordances** — current-line highlight refinements, whitespace
  rendering, indent guides.
- **Folding** — needs a fold engine *and* gutter fold markers (rides the
  gutter built here); big, greenfield.
- **Git-diff gutter markers** — needs a diff source; rides the gutter.
- **Context-menu polish** — submenus, kill-ring/clipboard history,
  first-letter mnemonic jump.

## Categorical bets (score at the arc's close)

- **No protocol change holds. → SCORED FALSE.** *Rendering* is local, but
  the toggle *command* lives daemon-side, so the mode has to cross the
  wire. It cost one additive variant + a version bump (v13) — cheap and
  routine here, but the bet was wrong: "renders locally" does not imply
  "no protocol change" when control is daemon-owned. Lesson for the rest of
  the arc: a frontend-rendered feature still needs a wire channel whenever
  its *control* is a daemon command.
- **The coordinate shift is localized, not pervasive.** Recon says TUI = 1
  viewport shift + ~5 manual sites; GPU = 1 pixel→byte + 4 byte→pixel
  sites. If a gutter bug shows up somewhere *not* on those lists, the bet
  was wrong and the mapping was less centralized than the recon found.
- **`pos_to_display` / cosmic-hit stay gutter-agnostic.** The mapping core
  is untouched; only the grid-crossing callers change. If a mapping-core
  edit becomes necessary, the seam was leakier than modeled.
- **Duplication is cheaper than abstraction here.** Two ~50-line gutter
  implementations beat inventing a shared cross-frontend layout layer for
  two consumers. Revisit only if a third frontend appears.

## Validation implications

Per sub-arc: `fmt` + `clippy --all-targets` + full tests under **both** Lua
flavors, as always. But the load-bearing validation is a **human eyeball
in each frontend** — the coordinate shift is precisely the class of change
where a unit test passes and the caret still lands one column off. Minimum
manual checklist for sub-arc 1: click-to-place lands correctly with the
gutter present; caret draws in the right cell/pixel; selection drag stays
out of the gutter; resize/scroll keep the gutter fixed; a >999-line file
grows the width without misaligning anything. GPU and TUI checked
separately.

## As-built

**Sub-arc 1 — gutter + absolute line numbers (TUI + GPU).**

- **TUI** (`window.rs`/`editor.rs`/`overlay_paint.rs`): `LineNumberMode
  {Off, Absolute}` per-window field + `Window::gutter_width()`. One
  viewport shift (`editor.rs`) makes text/syntax/diag/search painters
  gutter-agnostic; ~5 direct-coordinate sites (cursor, selection, mouse
  hit-test → line-start, remote presence) each add `gutter_w`.
  `paint_line_number_gutter` writes right-aligned dim digits alloc-free.
- **GPU** (`pmacs-gpu/main.rs`): everything hangs off `TEXT_LEFT`; a
  `text_left()` (= `TEXT_LEFT + gutter_width_px()`) is applied at the 4
  byte→pixel sites (main text, caret, washes/squiggles) and subtracted at
  the 1 pixel→byte hit-test. A dedicated `gutter_text_renderer`/buffer
  draws right-aligned dim numbers reshaped per scroll, mirroring the
  minimap's reserved column.
- **Control plane (the Q#UX1 correction).** M-x
  `window.toggle-line-numbers` sets the active window's mode (daemon-side).
  The TUI reads its window directly; the GUI receives the mode via the new
  **`InstanceMessage::LineNumbers { buffer_id, enabled }`** (protocol
  **v13**, additive, daemon-gated `< 13`). Producer:
  `SemanticRenderState::line_numbers_msg` (cached-compare suppression,
  seeded to the frontend's `off` default so a plain window adds no
  traffic). Consumer: the GUI drives its local `line_numbers` from it. The
  earlier `--line-numbers` GPU flag was retired. Now one `M-x` toggle works
  in **both** frontends, each affecting its own window — a single source of
  truth.

Validated: `fmt` + `clippy --all-targets` clean under both Lua flavors;
1440 lib tests (incl. the gutter render + `line_numbers_msg` emit/suppress
tests), 12 protocol, 53 pmacs-gpu (incl. the headless gutter render test on
the adapter). Both frontends eyeballed for coordinate correctness.

Deferred to later sub-arcs: relative/hybrid modes; diagnostic gutter signs
(sub-arc 2); the exact gutter padding is a tunable to eyeball.

**Sub-arc 2 — diagnostic gutter signs (TUI + GPU).**

Per-line severity signs riding the sub-arc-1 gutter — closing the last
deferred Task #23 item. No protocol/daemon change: the per-line severity is
already frontend-side (the TUI's diag store, the GPU's `current_decorations`).

- **Coupling decision.** Signs ride the *line-number* gutter — they show
  when line numbers are on, and vanish when off (the legacy col-0
  background marker returns in the TUI's no-gutter mode). A
  signs-without-numbers mode is deferred; when it lands, the gutter's
  presence test widens from "line numbers on" to "line numbers OR signs
  on."
- **TUI** (`diag.rs`/`editor.rs`/`view.rs`): `Viewport` gains `gutter_w` so
  overlays can address the gutter's leading column at `cell_origin.col -
  gutter_w`. The gutter number pass moved *before* the overlay loop so
  `DiagnosticView` can draw its sign into the gutter's blanked leading
  column without the number pass erasing it. The sign is the severity
  glyph (`E`/`W`/`I`/`H`) colored by `underline_color()`; most-severe wins
  (the existing `line_markers` map). Extracted `paint_line_markers`.
- **GPU** (`pmacs-gpu/main.rs`): `collect_gutter_sign_rects` walks
  `layout_runs()`, finds the most-severe diagnostic decoration overlapping
  each line (`diagnostic_severity_rank`, min wins) and pushes a thin
  severity-colored bar at the gutter's left edge, riding the existing
  background quad batch. Rendered as a **bar**, not a glyph — the GPU
  gutter number layer is single-color, so a per-line-colored bar was the
  clean path; same convention as the TUI, per-frontend rendering (Q#UX7).

Validated: `fmt` + `clippy --all-targets` clean both flavors; lib tests
(incl. the TUI gutter-sign placement test) + 54 pmacs-gpu (incl. a headless
sign render test on the adapter). Both frontends eyeballed.

**Cross-cutting bug fixed en route (PR #87, off the arc):** a bare
`--daemon` + a GPU is a *two-frontend* session; `EditorCore::close_active`
/ `close_others` operated on the global `windows` set and so closed *other*
frontends' windows, dangling their `view.active` and crashing the daemon in
`active_window()`. Scoped both to the active frontend's layout. (Surfaced
while eyeballing this sub-arc against the two-frontend setup.)

**Known follow-up (not this arc):** the GPU minibuffer completion-navigation
highlight sticks / doesn't wrap on arrow-up; reproduces on the "normal"
nav path but not the alternate one. Its own thread.

<!-- next sub-arcs appended here -->
