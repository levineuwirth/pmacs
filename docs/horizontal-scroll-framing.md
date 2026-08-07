# Horizontal scroll — QoL Stage 4

**Status: revision 1 — NOT APPROVED. Six questions open (Q#HS1–HS6).
No implementation may begin.**

This closes the QoL arc opened by one daily-driver report. Stage 1
(#219) made the TUI survive terminal zoom; Stage 2 (#220) gave the GUI
native zoom; Stage 3 (#221) added `ui.line-wrap` and made `wrap` the
default. Stage 4 is the other half of the user's own sentence:

> long lines need to either **wrap somehow or be scrollable**. […] This
> should also be something that the user can configure, whether to wrap
> or scrollable.

Stage 3 shipped the *mode*. It did not ship the *navigation*, and said
so: under `truncate`, text past the right edge is not merely off-screen
but **unreachable**. That is recorded in the setting's description and
in `ui.toggle-line-wrap`'s status message. Stage 4 removes that caveat
or the caveat stands permanently.

---

## 1. What is actually there today

**Verified in the tree at `02f3ec3`, not recalled.** Stage 3's revision
1 inverted its whole cost model by assuming both frontends consumed the
same `CellGrid`; every claim below carries a citation for that reason.

### 1.1 There is no horizontal scroll anywhere

No `view_left`, `scroll_left`, or `hscroll` in `src/` or `builtin/`.
This is greenfield. The window carries `view_top`, `cursor`, and
`goal_col` (`src/window.rs:374-376`) and nothing horizontal.

That matters for estimating: this is not "extend the vertical
mechanism sideways". There is no shared abstraction to extend.

### 1.2 The grid walk starts every line at column 0

`paint_line` (`src/text_view.rs:266`) walks from the line's first
character with no offset parameter, exactly as it did before Stage 3 —
wrapping changed *where rows break*, not *where the walk starts*.
`place_of_byte` and `byte_at_place` have the same shape.

So `view_left` enters the same functions Stage 3 just rewrote. **The
wrap rule must stay written exactly once** (`advance_wrapped`,
`src/text_view.rs:396`); a second copy differing by an offset is the
defect Stage 3 spent its review budget avoiding.

### 1.3 The GPU cannot use cosmic-text's horizontal scroll

**This is the finding most likely to invert the cost estimate, and it
is the Stage 4 analog of revision 1's error — so it leads.**

`Scroll::horizontal` is discarded throughout the GPU, and not by
oversight: **glyphon 0.11 never applies it when placing glyphs.**
Documented at `pmacs-gpu/src/main.rs:1611`, `:6316`, `:8020`, and
*asserted* by tests at `:16266` (`"horizontal is discarded"`), `:16337`,
`:16737`.

So the GPU's half of Stage 4 cannot be "set the scroll and reshape". It
needs a different mechanism — a shifted text origin at paint time,
adjusted clip bounds, or something else — and that mechanism has to
interact correctly with the gutter, the caret (`code_byte_px`),
decoration geometry (`push_glyph_extent_rects`), and hit testing
(`gutter_aware_rel_x`), each of which currently assumes x starts at
`text_left()`.

**Q#HS1 asks whether the GPU is in scope for Stage 4 at all.**

### 1.4 `view_top` is persisted; a `view_left` would want to be

`SavedLeaf` carries `path`, `cursor`, and `view_top` at
`DESKTOP_VERSION = 1` (`src/desktop.rs:33`, `:276-280`). The restore
path clamps `view_top` against the line count (`:512`).

A horizontal offset that does not survive restart is defensible; one
that does needs a defaulted field or a version bump. **Q#HS5.**

### 1.5 The cursor-follow hazard is already documented

`scroll_window` (`src/editor.rs:3628`) carries the cursor with a
vertical scroll, and its comment says exactly why:

> The cursor must follow the scroll: the renderer has an "auto-scroll
> to keep cursor visible" pass that would otherwise snap `view_top`
> straight back to wherever the cursor sits, so the user's mouse-wheel
> scroll would feel stuck after one notch.

A horizontal analog hits the identical problem. Stage 3's Q#LL3
deferred the choice here deliberately: **does explicit horizontal
scroll drag the cursor, or does the next motion snap back?** That is
**Q#HS4**.

### 1.6 `goal_col` exists and its relationship to `view_left` is unexamined

`goal_col` (`src/window.rs:376`) remembers a target column across
vertical motion and is cleared at seven sites in `src/editor.rs`. It is
a *column within the line*, not a viewport offset — but both are
"horizontal position" state on the same window, and a design that
ignores the interaction will produce a cursor that jumps on the first
vertical motion after a horizontal scroll. Called out so it is designed
rather than discovered.

---

## 2. The scope question, stated before the answers

Stage 3 ended with `wrap` as the default. **Under `wrap`, horizontal
scroll is meaningless** — there is nothing past the right edge. So
Stage 4's entire surface is conditional on a buffer-local mode.

That is a coherence fact, not only an implementation one: one
user-facing concept ("how do I see the rest of this line?") now has two
disjoint answers depending on a setting, and the commands, key
bindings, and status affordances for the `truncate` half do not exist
under `wrap`. `COHERENCE.md` §20 requires this be stated. **Q#HS6.**

---

## 3. Open questions

### Q#HS1 — is the GPU in scope for Stage 4?

The strongest argument for **yes**: Stage 3's entire thesis was that
the two frontends should stop disagreeing by accident. Shipping
horizontal scroll in the TUI only would recreate exactly the divergence
`ui.line-wrap` was built to close — a `truncate` buffer would be
navigable in one frontend and not the other.

The strongest argument for **no, name it Stage 5**: §1.3. The GPU needs
a mechanism that does not exist yet, touching caret placement,
decoration geometry, and hit testing. That is plausibly larger than the
TUI half, and bundling them makes one reviewable change into two
unreviewable ones.

**My vote: split it, and say so in the setting's description.** Ship
the TUI half as Stage 4 and the GPU half as Stage 5, with the
divergence *documented and time-boxed* rather than accidental — which
is the distinction Stage 3 actually drew. Stage 3's defect was never
"the frontends differ"; it was "the frontends differ and nobody chose
that". But this is a product call about shipping a known asymmetry, and
it is not mine to make.

### Q#HS2 — what moves the viewport?

Options, not mutually exclusive:

- **Automatic only** — the cursor-visibility pass gains a horizontal
  component, so moving the cursor past the edge scrolls the view. No
  new commands, no new bindings. Smallest surface; makes a long line
  readable by arrowing along it.
- **Explicit commands** — `ui.scroll-left` / `ui.scroll-right`, bound
  or not, plus the `goal_col` and cursor-follow questions.
- **Both**, which is what every editor with this feature ships.

**My vote: automatic first, as its own stage-within-a-stage.** It is
the smallest change that makes the reported text *reachable*, and it
needs no binding decisions. Explicit commands can follow with the
evidence of use.

### Q#HS3 — per window or per buffer?

`view_left` is **per window**, unambiguously: two panes on one buffer
must scroll independently, exactly as they already hold independent
`view_top`s (`src/desktop.rs:92`). Stage 3's Q#LL2 already recorded
this and accepted the consequence — the *mode* is buffer-local while
the *offset* is per-window, so one user-facing concept spans two
scopes.

**This is not really open**; it is listed so the accepted split is
re-confirmed at the moment it takes effect rather than inherited
silently.

### Q#HS4 — does the cursor follow an explicit scroll?

Only live if Q#HS2 includes explicit commands. §1.5 has the precedent
and the hazard. **My vote: follow, matching `scroll_window`** — the
existing snap-back pass makes the alternative feel broken, and the
vertical behavior is already the answer users have been trained on
*in this editor*.

### Q#HS5 — does `view_left` survive a restart?

`view_top` does (§1.4). Consistency argues yes; a defaulted field
avoids a `DESKTOP_VERSION` bump.

**My vote: yes, defaulted, no version bump** — but confirm that
`SavedLeaf`'s deserializer tolerates a missing field before relying on
it, because §1.4 is a citation of the *shape*, not of serde's
behavior on it.

### Q#HS6 — what does the coherence statement say?

Per `COHERENCE.md` §20 this framing must state its coherence impact.
The honest version is uncomfortable: Stage 4 adds capability that
exists **only under a non-default mode**, which is a new conditional
surface rather than a uniform improvement. Journey step 4 ("Understand
interface") is the row it serves.

**Q#HS6 is whether that is acceptable, or whether the arc should
instead reconsider the default.** Naming the alternative honestly: if
horizontal scroll makes `truncate` genuinely good, `wrap` being the
default is a choice worth re-examining rather than treating as settled
— and Stage 3 chose it partly *because* scroll did not exist.

---

## 4. Verification sketch (not final — depends on Q#HS1/HS2)

- Cell-level tests at several window widths with a non-zero offset —
  which is the Stage 4 case Stage 3's sketch explicitly refused to
  write, because "at non-zero offset" was a `view_left` requirement
  smuggled into a wrap lane.
- **A `wrap` control for every claim**, asserting the wrap path is
  byte-identical with a horizontal offset present, since under `wrap`
  the offset must be inert.
- Round-trip identity for `place_of_byte` / `byte_at_place` at non-zero
  offset, including the wide-character and tab cases Stage 3 settled at
  offset 0.
- **A PTY acceptance test for reachability**, following
  `tests/long_line_readable_acceptance.rs`: in `truncate`, the tail of
  a long line must reach the terminal *after* whatever Q#HS2 chooses
  moves the view. That file's `truncate` control currently asserts the
  tail is **absent** — Stage 4 must update it, and that update is
  itself the proof the caveat is gone.
- If the GPU is in scope: a headless witness that the caret, a
  decoration, and a hit test all agree with the shifted origin — the
  three consumers §1.3 names.

---

## 5. Coherence impact (§20 requirement)

- **Journey step 4, "Understand interface — Partial."** Stage 3's
  framing said the scorecard should name "a line that cannot be read in
  full" against this step. Stage 4 completes that only for `truncate`.
- **§16 Semantic Frontend Architecture.** Q#HS1 decides whether this
  lane *narrows* or *widens* frontend divergence. If the GPU is
  deferred, the divergence is deliberate and time-boxed — which is
  materially different from Stage 3's inherited accident, and the
  release notes must say which kind it is.
- **No new interaction island** if Q#HS2 lands automatic-only.
  Explicit commands would go in the ordinary command registry, not a
  new surface.
- **Config registry adoption**: none new expected. Stage 4 navigates
  the mode Stage 3 declared; if it needs a setting, that is a signal
  the design has drifted.
