# GPU horizontal scroll — QoL Stage 5

**Status: revision 4 — APPROVED 2026-08-07. All five questions
resolved. Implementation may begin within this scope.**

- **Q#G1** — the GPU-local offset is stored in **pixels**; parity
  conversion is exact via the supported monospace advance.
- **Q#G2** — **automatic cursor-follow only**; reset on **both** the
  wrap transition and `BufferSnapshot`, with the specified pre-motion
  witnesses.
- **Q#G3** — monospace-only, by the font contract that already exists.
- **Q#G4** — the minimap does not move.
- **Q#G5** — the complete verification set is accepted, including the
  snapshot-reset and minimap-stability witnesses.

**Scope boundary, restated because it is what makes this lane small:**
local GPU viewport state only. **No wire message, no protocol bump, no
command surface, no minimap movement.**

Revision 3 fixes four things review found in revision 2: Q#G1 still
carried two claims Q#G3 had already falsified; Q#G2 was missing the
**buffer-snapshot** reset; Q#G5's "nothing paints into the gutter" is
**impossible** as written, because the gutter legitimately holds line
numbers and diagnostic signs; and an off-left completion anchor
**hides** the popup rather than closing it, which is a protocol
distinction this lane must not blur.

Revision 2 had answered two functional findings: §1.1's "three
consumers" was incomplete because the manual quad/squiggle renderers
have **no code-area scissor at all**, and Q#G2's "inert under wrap" was
too weak — the offset must be **reset to zero**, as the TUI already
does.

**This closes the QoL arc.** Stage 1 (#219) made the TUI survive
terminal zoom; Stage 2 (#220) gave the GUI native zoom; Stage 3 (#221)
added `ui.line-wrap`; Stage 4 (#222) made `truncate` navigable **in the
TUI only**. Stage 5 is the GPU half, and it is the lane Q#HS1's time box
exists to guarantee: it is the immediately-next QoL work, and `wrap`
stays the default until it lands.

---

## 1. Stage 4's framing was wrong about the hard part

**§1.3 of `docs/archive/framings/horizontal-scroll-framing.md` said the GPU "needs a
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

### 1.1 The real work is a shared transform AND a shared clip

**Revision 1 said "three consumers" and that was wrong in a way that
would have shipped a defect.** Shifting the `TextArea` clips *glyphon's*
text, because glyphon honors `TextBounds`. **The manual quad and
squiggle renderers have no code-area scissor at all** — nothing stops
them painting into the gutter, and today nothing needs to, because no
code-relative x can be negative. Scrolling makes that false.

So Stage 5 needs **two** shared things, not one offset:

1. **One screen↔code transform.** `code_x → screen_x` is
   `text_left() - offset_px + code_x`, and hit testing is its exact
   inverse. Applied once, in one place, or the consumers disagree.
2. **One code clip rectangle.** `[gutter_clip_left, text_bounds_right)`
   — the same bounds the `TextArea` gets, expressed for the paths
   glyphon does not clip. **Every code-relative painter must intersect
   with it**, and that is a new obligation, not a threading exercise.

The paths that need both:

| path | site | what breaks without the clip |
|---|---|---|
| caret rect in clip | `:9698` | **asserts the caret cannot precede `text_left`** — its comment says so explicitly. False after scrolling; the caret paints over the gutter |
| caret-painted predicate | `:9734` | same missing left-edge test. **So revision 1's claim that the scroll indicator "inherits the fix" is FALSE** — `code_byte_painted` reuses this and would call an off-left byte painted |
| glyph extent rects | `:9766` | washes, squiggles and selection extents must be **cropped** at the gutter edge, not merely offset |
| inline math origins | `:9434` | math boxes derive from the code origin and would render into the gutter |
| completion anchor | `:7606` | an off-left anchor must **hide** the popup — it already returns `None` when scrolled out, and closure is the daemon's `CompletionPopup { anchor: None }`, not this lane's |

The two caret sites are the sharpest: `:9698` does not merely lack a
check, it **documents the absence as safe** (*"right of the gutter isn't
needed: the caret x can't precede `text_left`"*). A comment asserting an
invariant this lane deletes is worse than silence, so it must be
rewritten rather than merely joined by a new test.

### 1.2 No wire, and that is not an accident

The GPU **owns its viewport locally** — `scroll_top` and
`code_scroll_residual` are local state, never sent. A horizontal offset
is the same kind of state, so Stage 5 adds **no protocol message and no
version bump**, exactly as Stage 4 added none.

This is worth stating because the parallel with `ui.line-wrap` is
misleading: the *mode* is buffer state and needed `LineWrapFacts` at
v22, but the *offset* is viewport state and needs nothing.

### 1.2a The one approved exception to "local GPU viewport state"

**Approved by the user 2026-08-08**, after implementation raised it.
The scope line for this stage is *local GPU viewport state — no wire
message, protocol bump, command surface, or minimap movement*. One
change lands outside it, and only one:

**`pmacs_protocol::scroll::follow_left`.** The follow rule — *scroll the
minimum distance that puts the cursor back inside* — now lives in the
protocol crate beside `classify`, and **both** frontends call it:
`src/editor.rs::horizontal_follow` delegates, and the GPU converts
px ↔ columns around it (exact, per Q#G1/Q#G3).

*Why it is not scope creep.* Q#G5 requires a TUI-parity witness that is
"checkable rather than asserted". Two tests in two crates asserting the
same literal is not that — it is the structural duplication
`pmacs-protocol::scroll`'s own module docs condemn, and **that module
exists because this arc already shipped exactly that defect**: the
scroll indicator, fixed in one copy and left wrong in the other. Without
a shared rule there is no way to make the witness real.

*What it does not do*, which is what keeps it narrow and is the basis of
the approval: it moves **no viewport state** (the TUI still owns
`view_left`, the GPU still owns `code_scroll_left`), adds **no wire
message**, and needs **no protocol-version bump**. `follow_left` is a
pure function over values each side already holds — the identical
argument `classify`'s module docs already make for living there.

---

## 2. Open questions

### Q#G1 — is the stored offset a column or a pixel count?

The TUI stores a display **column** (`view_left`). The GPU paints in
**pixels**.

**Revision 2 left two claims here that its own Q#G3 answer had already
falsified**: that the GPU's font "need not be monospace", and that Q#G3
makes "column" ill-defined. Neither is true — the code font is
monospace by contract, so a column has one well-defined width, measured
by the existing `ADVANCE_PROBE` helper.

*My vote is unchanged — store **pixels** — but the reasons narrow to
the ones that survive:* the GPU's other viewport state is already
pixel-flavoured (`code_scroll_residual` is a float), and a pixel offset
composes with the clip rectangle without rounding at every frame.

**Conversion is therefore exact, not approximate.** `columns × the
supported monospace advance` is the definition, and it is what makes
the unconditional TUI-parity witness in Q#G5 checkable at all. A
proportional font would have made this a lossy conversion; the font
contract means it never is.

### Q#G2 — how is the offset moved?

Stage 4 chose **automatic only**: the cursor-visibility pass gains a
horizontal component. The GPU's analogous pass is
**`ensure_caret_painted`** — named rather than cited by line, since
revision 2 also invented a `follow_cursor` that does not exist.

*My vote: mirror it exactly*, so the two frontends agree on when the
view moves. A GUI is also the place a horizontal **wheel/trackpad**
gesture exists — but that is an explicit-scroll surface, which Stage 4
deliberately deferred, and adding it here would make the frontends
disagree again in the lane that exists to stop that.

**And the wrap transition must RESET the offset to zero, not merely
ignore it.** Revision 1 said "inert under wrap", which is too weak:
inertness hides a stale value that reappears the moment the buffer
toggles back to `truncate`, before any cursor motion. The TUI does not
rely on inertness — `horizontal_follow` (`src/editor.rs`) assigns
`view_left = 0` on the wrap branch and returns.

The GPU must specify the **identical lifecycle**, and `apply_line_wrap`
is where it belongs, beside the reflow it already performs.

**And a second reset the TUI has no analogue for: the buffer
snapshot.** The GPU zeroes `scroll_top` and `code_scroll_residual`
whenever a snapshot installs a new buffer — the offset is viewport
state tied to the document being shown, and it must reset there for the
same reason they do. Without it a buffer switch **inherits the previous
document's leftward viewport**, showing the new buffer scrolled
sideways until a cursor motion repairs it.

That is a worse symptom than the wrap case, because nothing about the
new buffer explains it. Both resets are lifecycle requirements with
their own witnesses (Q#G5), not properties that fall out of the
transform.

### Q#G3 — proportional fonts **ANSWERED: they do not occur**

> **Revision 1 asked the wrong question, from a false premise.** It said
> "the GPU can resolve a non-monospace family" and proposed accepting a
> new TUI/GPU divergence to accommodate it.
>
> **The GPU does not resolve proportional code fonts.**
> `family_is_monospace_everywhere` (`pmacs-gpu/src/main.rs:8199`) gates
> the family across all four weight/style combinations, `apply_font_facts`
> (`:8235`) falls back when it fails, and
> `unresolvable_and_proportional_families_fall_back` **requires** that
> fallback. A proportional family cannot become the code font.
>
> So the answer is **monospace-only, by the font contract that already
> exists** — not a new constraint this lane imposes, and not a
> divergence to negotiate. Revision 1 would have introduced a
> font-dependent behavior difference to solve a problem the codebase had
> already solved, in the lane whose entire purpose is removing
> unchosen divergence.
>
> **The consequence for Q#G5**: the monospace TUI-parity witness is
> **unconditional** for every font the GPU supports, rather than gated
> on a font check. That is a stronger test, and it exists only because
> the premise was corrected.

Pixel storage (Q#G1) is unaffected and still preferred — but its
column↔pixel conversion is now defined against **the supported
monospace advance**, which is a well-defined quantity rather than an
approximation.

### Q#G4 — does the minimap move?

The minimap draws line-shape bands. Horizontal scroll does not change
which lines exist.

*My vote: no.* The minimap is a whole-document overview; scrolling the
text sideways should not scroll it.

### Q#G5 — what does the verification look like?

The GPU suite is headless-render based (`headless_or_skip`), and
`PMACS_REQUIRE_GPU=1` makes a missing adapter a failure rather than a
skip.

Sketch, pending Q#G1/G2/G4:

- **The gutter is unchanged by scrolling.** Revision 2 proposed
  asserting that *nothing* paints left of `gutter_clip_left`, which is
  **impossible**: with line numbers on, the gutter deliberately holds
  digit glyphs and diagnostic-sign quads. The assertion would fail on a
  correct implementation.

  The checkable form of the same intent: **the gutter rectangle is
  byte-identical before and after a horizontal scroll**, and separately,
  only *code-relative* output geometry is inspected for the left-edge
  rule. That still catches the failure a per-painter test would miss —
  a code painter bleeding into the gutter changes those pixels — while
  remaining true of a working build.
- **A left-clipped caret predicate.** `code_caret_rect_in_clip` and
  `caret_painted_in_code_clip` must both report *not painted* for a
  caret scrolled off-left. This is what makes the scroll indicator
  correct; revision 1 wrongly assumed it came for free.
- **Math and rule clipping**: an inline math box and a decoration rule
  whose origins are left of the edge are cropped or culled, not drawn.
- **An off-left completion anchor HIDES the popup; it does not close
  it** — and the boundary is a **point**, tested at the edge.

  *Added after review found the first implementation wrong here.* It
  reused `survives_code_clip_left` and passed `line_height` as the
  horizontal extent: a vertical dimension standing in for a horizontal
  one. An anchor up to a line-height left of the gutter therefore
  survived, and `completion_dropdown_rect` bounds `ax` against the right
  margin only — so the popup painted over the line numbers. An anchor is
  a position between glyphs with no width of its own, so the predicate
  is `screen_x < code_clip_left()`.

  **The far-off-left witness cannot catch this**, which is why it stayed
  green: 200px off-left fails a width-based predicate too. The witness
  must **straddle** the edge — the same anchor a fraction of a pixel
  either side — and assert the popup's own left edge stays out of the
  gutter on the visible side.

  The distinction is a protocol one and revision 2 got it wrong.
  `completion_anchor_px` returns `None`, so nothing draws — but the
  daemon-owned completion state and its key handling are retained.
  Actual closure is `CompletionPopup { anchor: None }`, which is the
  daemon's to send. So the witness is: **no completion paint while the
  anchor is off-left, and the popup reappears when it scrolls back into
  view** — session semantics unchanged. A lane about viewport geometry
  must not quietly redefine when a completion ends.
- **The three consumers agree with the shifted origin** — caret,
  decoration rect, and hit test at one non-zero offset, since a partial
  fix shows up as disagreement between them.
- **Round-trip hit testing**: pixel → byte → pixel at a non-zero
  offset.
- **The wrap transition resets to zero** (Q#G2): scroll right under
  `truncate`, toggle to `wrap`, toggle back, and assert the offset is 0
  **before any cursor motion**. An inertness-only implementation passes
  a "wrap looks right" test and fails this one.
- **The buffer snapshot resets to zero** (Q#G2, second lifecycle rule).
  Revision 3 added the requirement and left it untested, which is how a
  lifecycle rule quietly becomes a comment. Scroll to a non-zero offset
  in buffer A, install a **buffer B snapshot**, and assert the offset is
  0 **and B renders at its code origin — before any `CursorByte`
  arrives**. The pre-cursor scoping is the whole test: a later cursor
  motion repairs the offset anyway, so a witness that waits for one
  cannot distinguish "reset on snapshot" from "repaired on first
  motion", which is precisely the bug.
- **The minimap does not move** (Q#G4). The implementation already
  supports the vote — the minimap derives from the summary, the surface
  dimensions and `scroll_top`, with no horizontal input — so this pins
  an existing property rather than asking for new work, and that is the
  reason to write it: an offset threaded one seam too far would break
  it silently. Assert **equal minimap vertices (or pixels) before and
  after a non-zero horizontal offset**, with vertical state unchanged.
- **A TUI-parity witness**, now **unconditional** (Q#G3): the same
  buffer and the same column offset yield the same first visible
  character in both frontends. This is what makes "the frontends agree"
  checkable rather than asserted.

---

## 3. Coherence impact (§20)

- **§16 Semantic Frontend Architecture — the direct target.** This lane
  *closes* the divergence Stage 4 opened deliberately. The release notes
  should say the gap is closed, since Stage 4's said it existed.
- **Journey step 4 is NOT completed by this lane, and revision 1
  claimed it was.** Step 4 ("Understand interface") is scored on
  **welcome / help / tutorial discoverability**, and `COHERENCE.md:395`
  holds it **Partial** for reasons this lane does not touch: `C-h`
  deletes a word (deliberately, §18) and there is no tutorial.
  Long-line reachability is interface *comprehension*, which the step
  benefits from without being scored on.

  So the honest claim is **preserving and improving interface
  comprehension**, with no scorecard movement — and a scorecard edit
  here would need new journey evidence, which this lane does not
  produce. Recorded because writing an unearned ✔ into a scorecard is
  how a coherence document stops being ground truth.
- **After this merges, Q#HS6 becomes live**: whether `wrap` remains the
  default is revisitable on use evidence once parity exists. Not part
  of this stage.
- **Rule 4 applies at this merge**: the arc is done, so the long-lines
  lane in `docs/active-work.md` is removed — after its durable facts
  reach `docs/agent-handoff.md`, which is the precondition, not a
  formality.
