# pmacs-gpu status band — framing pass

Date: 2026-06-12. The GPU window has no status surface: no buffer
name, no modified flag, no cursor position, no diagnostic counts
(the M4.6 parity session deferred counts here for exactly this
reason). Survey facts: `InstanceMessage::ModeLine(Vec<Cell>)` has
sat reserved-but-unused since day one; `BufferSnapshot` carries no
name/modified; glyphon's `prepare()` takes `&[TextArea]` so a second
band area is structurally cheap; five geometry sites assume text
runs to the surface bottom (`estimated_visible_lines`,
`TextBounds.bottom`, the minimap height pair, the edge-scroll
bottom band).

## Q#S1 — where status facts come from

**Stance: split by authority and freshness.**

- **Locally derived, per frame**: cursor L:C (from `own_cursor` ×
  `current_line_starts` — the *optimistic* caret, so the readout
  tracks typing bursts instead of lagging a round trip behind
  them), and the scroll indicator (`scroll_top` × visible ×
  total, the TUI's All/Top/Bot/NN% formula).
- **Instance-authoritative, on the wire**: buffer name, modified
  flag, exact whole-file diagnostic counts. A new additive variant
  (protocol v8, ladder continues on the v6 floor):
  `StatusFacts { buffer_id, name, modified, diag_errors,
  diag_warnings }`, emitted by the semantic producer when any fact
  changes (cached-compare, like `FileStyleSummary`; counts piggyback
  the diag-store epoch from the parity session).

Rejected: populating the reserved `ModeLine(Vec<Cell>)`. It is
grid-shaped — pre-formatted styled cells — which bakes the TUI's
layout into a frontend that does its own, and a daemon-formatted
L:C would visibly lag the optimistic caret. The variant stays
reserved for grid use. Also rejected: counting marked lines in
`FileStyleSummary` as the counts — that counts *lines*, the TUI
counts *diagnostics*; two frontends showing different numbers for
the same buffer reads as a bug.

## Q#S2 — band rendering

**Stance: quad + second `TextArea`.** A `STATUS_BAND_HEIGHT` (26px)
strip at the surface bottom: a background quad through the existing
quad pipeline, and a one-line cosmic-text `Buffer` shaped only when
the composed status string changes, passed as a second `TextArea`
in the same `prepare()` call. Left: `name *` (modified star).
Right: `E:n W:n  L:C  NN%` — diag counts colored by the severity
palette, omitted when zero.

## Q#S3 — geometry

**Stance: one helper, no scattered arithmetic.** `text_area_bottom
(surface_height) = height - STATUS_BAND_HEIGHT` feeds every site
that assumed the surface bottom: `estimated_visible_lines`,
`TextBounds.bottom`, the minimap height pair
(`minimap_band_contains` / `minimap_y_to_line` / painters), and the
edge-scroll bottom band. The minimap keeps its own `MINIMAP_BOTTOM`
inset *above* the band.

## Predicted findings (categorical bets)

1. **A missed bottom-assuming site**: something still measures to
   the raw surface bottom — surfaces as content drawn under the
   band, or a hit-test/scroll off-by-a-line at the window bottom.
2. **Visible-lines ripple**: shrinking `estimated_visible_lines`
   perturbs viewport/scroll math tuned to the old height — surfaces
   as a flickering scroll indicator or an unreachable last line.
3. **Optimistic L:C flicker**: the locally derived readout jumps
   when a daemon `CursorByte` confirms through the floor-release
   path mid-burst — cosmetic, but noticeable enough to get reported.

## Session plan

Order: geometry + band rendering with the local facts (L:C, scroll)
→ v8 `StatusFacts` wire + daemon emission + name/modified/counts →
tests/polish. Manual validation: resize the window, type a burst
mid-file (L:C tracks), scroll to both extremes (Top/Bot), introduce
and fix an error (counts appear/vanish), edit (modified star), and
re-run the minimap + edge auto-scroll gestures against the new
bottom edge.
