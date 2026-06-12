# pmacs-gpu diagnostic parity — framing pass

Date: 2026-06-12. The TUI's M4.6 surface (PR #64) gives diagnostics
severity-colored squiggles, column-0 line markers, and mode-line
counts. The GPU frontend renders none of that: diagnostics arrive as
`Decorations` with per-severity kinds and are consumed by
**recoloring the text foreground** (`decoration_kind_to_color`,
main.rs:4041 → `source_color_at` :3933 → chunk `Attrs::color`) — the
exact syntax-color-clobbering approach the TUI just abandoned, and
the reason `underline_color` exists in protocol v6.

## Verified facts (survey 2026-06-12)

- glyphon/cosmic-text 0.18 draw **no underlines**; `line_from_chunks`
  sets only family + color. Squiggles cannot come from text attrs.
- The quad pipeline draws arbitrary pixel rects:
  `push_glyph_extent_rects` (:2696) walks `layout_runs()` and maps a
  byte range to per-visual-line glyph x-extents; `MinimapRect {x, y,
  w, h, color}` → 6 vertices (:3716–:3728). Selection / CurrentLine
  backgrounds already ship through it; all four diagnostic kinds
  return `None` from `decoration_kind_to_bg_color` today by design.
- `FileStyleSummary` (one dominant `Style` per line, minimap-wide) is
  computed in `scoped_file_summary` (semantic_render.rs:1156) from
  **style spans only** — diagnostics never reach the minimap. The
  minimap consumer reads only `Style.fg` (`minimap_style_color`,
  :3063).
- Decorations are viewport-scoped and dirty-merged
  (`current_decorations`, :299), translated through unconfirmed edits
  (M11.4) — the squiggle source data is already maintained correctly.

## Q#D1 — squiggle mechanism

**Stance: quad-pipeline underline bars.** A 2px severity-colored bar
at the bottom of each glyph extent the decoration covers, emitted
next to the existing background quads in
`decoration_background_vertex_bytes`. Straight bars first; wavy needs
a shader or texture and buys nothing until the straight bar is
proven. Geometry reuses `push_glyph_extent_rects` with a height/y
override rather than a parallel walk.

## Q#D2 — retire the fg recolor

**Stance: yes, squiggles replace text recoloring for all four
severities.** Parity with the TUI rationale: the error is *under* the
text; the text keeps its syntax color. `decoration_kind_to_color`
returns `None` for diagnostic kinds; its RGB constants move to the
new severity→bar-color map so the palette is unchanged.

## Q#D3 — minimap diagnostic marks (the GPU's gutter signs)

**Stance: producer-side.** `scoped_file_summary` additionally sets
`underline_color` (severity-max, same indexed palette as the TUI) on
the dominant style of any line a diagnostic touches, honoring the
existing hold-while-stale discipline; the minimap consumer draws its
line stroke in `underline_color` when set, else `fg`. This rides the
v6 field end-to-end and costs no new message. Whole-file diagnostic
positions are available instance-side where the summary is computed —
the viewport-scoping of `Decorations` is irrelevant here.

## Q#D4 — counts surface

**Stance: defer.** The GPU has no status band, and viewport-scoped
decorations cannot produce whole-file counts frontend-side. A status
band is its own session (layout, font sizing, what else lives there);
the minimap marks from Q#D3 carry the at-a-glance signal until then.

## Predicted findings (categorical bets)

1. **Bar placement tuning**: the first y-position for the 2px bar
   will collide with descenders or the next line's ascenders on some
   line-height; expect one round of "squiggle looks off" feedback.
2. **A test pins the fg recolor**: at least one pmacs-gpu test
   asserts diagnostic text color; it fails and gets rewritten to
   assert bar quads instead.
3. **Summary churn**: folding diagnostics into `FileStyleSummary`
   makes it recompute on diagnostic publish, not just generation
   advance — if the producer's "recompute when generation advances"
   gate isn't widened, minimap marks lag one edit behind; if it is
   widened naively, summary traffic grows on every publish.

## Session plan

Single session, three commits: (1) quad squiggles + fg-recolor
retirement, (2) producer `underline_color` in `FileStyleSummary` +
minimap stroke color, (3) tests/polish. Manual validation: rust file
with error+warning+hint, scroll the viewport, edit near a squiggle
(translation), check minimap marks track publishes.
