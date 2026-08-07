# GPU horizontal scroll — QoL Stage 5

**Status: revision 1 — NOT APPROVED. Five questions open (Q#G1–G5). No
implementation may begin.**

**This closes the QoL arc.** Stage 1 (#219) made the TUI survive
terminal zoom; Stage 2 (#220) gave the GUI native zoom; Stage 3 (#221)
added `ui.line-wrap`; Stage 4 (#222) made `truncate` navigable **in the
TUI only**. Stage 5 is the GPU half, and it is the lane Q#HS1's time box
exists to guarantee: it is the immediately-next QoL work, and `wrap`
stays the default until it lands.

---

## 1. Stage 4's framing was wrong about the hard part

**§1.3 of `docs/horizontal-scroll-framing.md` said the GPU "needs a
mechanism that does not exist" and called it the fact most likely to
invert the cost estimate. It was half right and the half it got wrong
is the expensive half.**

What it got right: `Scroll::horizontal` **is** discarded throughout,
because glyphon 0.11 never applies it when placing glyphs
(`pmacs-gpu/src/main.rs:1611`, `:6316`, `:8020`, asserted at `:16266`,
`:16337`, `:16737`). Scrolling via cosmic-text's own scroll is not
available.

What it got wrong: that is not the only mechanism. **The document
`TextArea` already carries an explicit origin and a clip rectangle**
(`pmacs-gpu/src/main.rs:8712`):

```rust
TextArea {
    buffer: &self.buffer,
    left: text_left,                       // paint origin
    top: TEXT_TOP,
    bounds: TextBounds { left: gutter_clip_left, right: text_bounds_right, .. },
    ..
}
```

Horizontal scroll is `left: text_left - offset_px` with `bounds.left`
unchanged. glyphon clips to `bounds`, so glyphs pushed left of the
gutter simply are not painted — which is the same "paint from column 0,
clip at the edge" shape the grid renderer uses, expressed in pixels.

**This is a mechanism the file already relies on**, not a new one:
`gutter_clip_left` exists precisely so the gutter does not get painted
over.

**Why the correction matters beyond the estimate.** Stage 4's framing
used §1.3 to justify splitting the GPU out, and I endorsed the split on
that basis. The split is still right — the *consumers* below are real
work, and shipping them inside Stage 4 would have made one reviewable
change into two unreviewable ones — but it was justified partly by a
claim that overstated the difficulty. Recorded here rather than quietly
dropped.

### 1.1 The real work is the three consumers, exactly as named

Each produces an x relative to `text_left()`, so each needs the same
offset applied — and the offset must be applied **once**, in one place,
or they will disagree:

- **Caret**: `code_byte_px` → `caret_rect` → `code_caret_rect_in_clip`
  (`:9586`, `:9623`, `:9649`).
- **Decoration geometry**: `push_glyph_extent_rects` (`:9672`) —
  washes, squiggles, selection extents.
- **Hit testing**: `gutter_aware_rel_x` (`:6469`) and the byte-at-pixel
  path, which must invert the offset.

`code_byte_painted` (Stage 3) already intersects with the drawable clip,
so the scroll indicator inherits the fix rather than needing its own.

### 1.2 No wire, and that is not an accident

The GPU **owns its viewport locally** — `scroll_top` and
`code_scroll_residual` are local state, never sent. A horizontal offset
is the same kind of state, so Stage 5 adds **no protocol message and no
version bump**, exactly as Stage 4 added none.

This is worth stating because the parallel with `ui.line-wrap` is
misleading: the *mode* is buffer state and needed `LineWrapFacts` at
v22, but the *offset* is viewport state and needs nothing.

---

## 2. Open questions

### Q#G1 — is the stored offset a column or a pixel count?

The TUI stores a display **column** (`view_left`). The GPU paints in
**pixels** and its font need not be monospace.

There is already a measured per-cell advance (the `ADVANCE_PROBE`
helper at `:336`), so columns convert to pixels.

*My vote: store **pixels**, and convert only where a column is the
honest unit (parity assertions against the TUI).* The GPU's other
viewport state is already pixel-flavoured (`code_scroll_residual` is a
float), a pixel offset composes with the clip without rounding, and
Q#G3 makes "column" ill-defined anyway.

### Q#G2 — how is the offset moved?

Stage 4 chose **automatic only**: the cursor-visibility pass gains a
horizontal component. The GPU has the analogous pass
(`ensure_caret_painted` / `follow_cursor`).

*My vote: mirror it exactly*, so the two frontends agree on when the
view moves. A GUI is also the place a horizontal **wheel/trackpad**
gesture exists — but that is an explicit-scroll surface, which Stage 4
deliberately deferred, and adding it here would make the frontends
disagree again in the lane that exists to stop that.

### Q#G3 — what happens under a proportional font?

The GPU can resolve a non-monospace family (there is a fallback path
and `unresolvable_and_proportional_families_fall_back` covers it). With
a proportional font, "column" has no fixed pixel width, so column↔pixel
parity with the TUI is not achievable.

*My vote: the offset is pixels and the behavior is defined in pixels*,
so a proportional font scrolls correctly and simply does not match the
TUI column-for-column. **Q#G3 asks whether that divergence is
acceptable, or whether horizontal scroll should be gated to monospace
families.** I lean to the former; gating a navigation feature on a font
choice is a worse surprise than an imprecise correspondence.

### Q#G4 — does the minimap move?

The minimap draws line-shape bands. Horizontal scroll does not change
which lines exist.

*My vote: no.* The minimap is a whole-document overview; scrolling the
text sideways should not scroll it.

### Q#G5 — what does the verification look like?

The GPU suite is headless-render based (`headless_or_skip`), and
`PMACS_REQUIRE_GPU=1` makes a missing adapter a failure rather than a
skip.

Sketch, pending Q#G1–G3:

- **The three consumers agree with the shifted origin** — a caret, a
  decoration rect, and a hit test at a non-zero offset, which is the
  triple §1.1 names and the one a partial fix would break.
- **Round-trip hit testing**: pixel → byte → pixel at a non-zero
  offset.
- **Inert under `wrap`**, mirroring Stage 4's control.
- **A TUI-parity witness** at a monospace font: the same buffer, the
  same offset in columns, the same first visible character. This is the
  test that makes "the frontends agree" checkable rather than asserted
  — and Q#G3 decides whether it is conditional on the font.

---

## 3. Coherence impact (§20)

- **§16 Semantic Frontend Architecture.** This lane *closes* the
  divergence Stage 4 opened deliberately. The release notes should say
  the gap is closed, since Stage 4's said it existed.
- **Journey step 4, "Understand interface."** Completed for `truncate`
  in both frontends, which is what the scorecard row should then say.
- **After this merges, Q#HS6 becomes live**: whether `wrap` remains the
  default is revisitable on use evidence once parity exists. Not part
  of this stage.
- **Rule 4 applies at this merge**: the arc is done, so the long-lines
  lane in `docs/active-work.md` is removed — after its durable facts
  reach `docs/agent-handoff.md`, which is the precondition, not a
  formality.
