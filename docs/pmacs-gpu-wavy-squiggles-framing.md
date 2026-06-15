# pmacs-gpu wavy squiggles — framing pass

Date: 2026-06-15. The diagnostic-parity session (PR #65) deferred the
wavy look: diagnostics underline with a straight 2px quad bar
(`DIAG_UNDERLINE_PX`), with the framing note "wavy needs a shader or
texture and buys nothing until the straight bar is proven." The bar is
proven; this is the shader.

## Survey facts

- The single `QuadRenderer` (one solid-color shader) draws every quad:
  selection/current-line washes, caret, minimap, status band, and the
  diagnostic bars. Its vertex is `pos(2) + color(4)` = 24 bytes.
- Diagnostic bars are emitted in `collect_own_decoration_rects` via
  `push_glyph_extent_rects(.., Some(DIAG_UNDERLINE_PX))` and ride the
  `bg_vertices` batch (drawn *under* the text).
- `fwidth`/`smoothstep` are core WGSL — anti-aliasing needs no feature
  flag or MSAA change.

## Q#W1 — mechanism

**Stance: a dedicated squiggle pipeline, not geometry.** A zigzag of
many tiny quads would need no shader but reads as a sawtooth and
multiplies vertex count per underline. Instead a second pipeline
(`SquiggleRenderer`) whose fragment shader computes
`A·sin(x·2π/λ)` and alphas pixels by distance to the curve
(`1 - smoothstep(thickness±fwidth, dist)`). Six vertices per glyph-run
extent, same as the bar — the geometry path (`push_glyph_extent_rects`)
is reused unchanged; only the vertex *format* differs (adds a `uv`:
absolute screen-x for continuous phase, signed px from the band
centerline for the cross-axis). The band grows from 2px to ~6px to
contain the wave.

## Q#W2 — routing & z-order

**Stance: split the diagnostic underline out of the wash batch; keep
it under the text.** Washes stay solid quads in `bg_vertices`;
squiggles get their own `squiggle_vertex_bytes` → own reusable buffer
→ own draw, slotted between the bg quads and the text (same z-slot the
bar had — descenders paint over the wave, the conventional
spell-check look). `decoration_kind_to_underline_color` is unchanged,
so the severity palette and the minimap marks are untouched.

## Predicted findings (categorical bets)

1. **Placement/amplitude tuning**: wavelength, amplitude, or the
   band's vertical offset reads wrong at the default font size — one
   round of "too tight / too tall / too faint" feedback.
2. **Phase seams**: a diagnostic spanning two glyph *runs* (or two
   adjacent diagnostics) emits separate rects; if phase isn't keyed to
   absolute screen-x the waves don't line up at the seam. (Mitigated
   by using screen-x as the phase input — but worth eyeballing.)
3. **DPI/hidpi**: amplitude and thickness are in logical px; on a
   fractional-scale surface the AA may thin or the wave may alias.

## Session plan

One commit: squiggle shader + pipeline + vertex format, route the
underline out of the wash batch, rename `DIAG_UNDERLINE_PX` →
`DIAG_SQUIGGLE_PX`, pure-fn test on the UV emission. Manual
validation: a file with an error + warning + hint on adjacent lines,
a diagnostic spanning a multi-glyph token, and a resize.
