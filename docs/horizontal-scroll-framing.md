# Horizontal scroll — QoL Stage 4

**Status: revision 4 — every question answered; APPROVAL NOT YET
RECORDED. No implementation may begin until it is.**

| question | state |
|---|---|
| Q#HS1 — GPU in scope? | **answered**: no, Stage 5, time-boxed (§3) |
| Q#HS2 — what moves the viewport? | **answered**: automatic only |
| Q#HS3 — window or buffer? | re-confirmed: per window |
| Q#HS4 — cursor follows explicit scroll? | **deferred** — not live under HS2 |
| Q#HS5 — persist `view_left`? | **approved**: yes, no version bump, on two conditions |
| Q#HS6 — the default | **answered**: `wrap` stays |
| Q#HS7 — what IS `view_left`? | **accepted**: (a), (b), (c′), (c″), (d) |

Revision 4 adds only Q#HS7(c″) — the tab-straddle mapping — and fixes
the handoff's "Stage 4 is the remainder" to name Stages 4–5.

**Stage 4 does NOT close the QoL arc.** Revision 1 said it did, and
that was written before Q#HS1 moved GPU horizontal scroll to Stage 5.
The claim is not merely stale — it is load-bearing in the wrong
direction: `docs/active-work.md`'s **Rule 4 removes a lane when its
ARC is done**, so a framing asserting Stage 4 closes the arc would
license retiring this lane at the TUI merge, **orphaning the very
Stage 5 that Q#HS1's time box exists to guarantee**. The arc closes at
**Stage 5**.

Stage 1 (#219) made the TUI survive terminal zoom; Stage 2 (#220) gave
the GUI native zoom; Stage 3 (#221) added `ui.line-wrap` and made
`wrap` the default. Stage 4 is the TUI half of the other half of the
user's own sentence:

> long lines need to either **wrap somehow or be scrollable**. […] This
> should also be something that the user can configure, whether to wrap
> or scrollable.

Stage 3 shipped the *mode*. It did not ship the *navigation*: under
`truncate`, text past the right edge is not merely off-screen but
**unreachable**.

**Revision 1 claimed that caveat "is recorded in the setting's
description". It is not.** `builtin/runtime/linewrap.lua:23` says only
*"How a line wider than the window is shown: wrap onto following rows,
or truncate at the edge."* The word "unreachable" appears in
`ui.toggle-line-wrap`'s status message and in a source comment — neither
of which a user sees if they set `ui.line-wrap = "truncate"` in
`init.lua` and never invoke the toggle. **That is a real, if small,
user-facing gap shipped in #221**, and §6 makes amending the
description a Stage 4 deliverable rather than leaving the false claim
standing.

---

## 1. What is actually there today

**Verified in the tree at `02f3ec3`, not recalled.** Stage 3's revision
1 inverted its whole cost model by assuming both frontends consumed the
same `CellGrid`; every claim below carries a citation for that reason.

### 1.1 There is no horizontal scroll anywhere

No `view_left`, `scroll_left`, or `hscroll` in `src/` or `builtin/`.
The window carries `view_top`, `cursor`, and `goal_col`
(`src/window.rs:374-376`) and nothing horizontal. This is greenfield —
not "extend the vertical mechanism sideways", because there is no
shared abstraction to extend.

### 1.2 The grid walk starts every line at column 0

`paint_line` (`src/text_view.rs:266`) walks from the line's first
character with no offset parameter, exactly as before Stage 3 —
wrapping changed *where rows break*, not *where the walk starts*.
`place_of_byte` and `byte_at_place` have the same shape.

So `view_left` enters the same functions Stage 3 just rewrote, and
**the wrap rule must stay written exactly once** (`advance_wrapped`,
`src/text_view.rs:396`). A second copy differing by an offset is the
defect Stage 3 spent its review budget avoiding.

### 1.3 The GPU cannot use cosmic-text's horizontal scroll

**The finding most likely to invert the cost estimate, so it leads.**

`Scroll::horizontal` is discarded throughout the GPU, and not by
oversight: **glyphon 0.11 never applies it when placing glyphs.**
Documented at `pmacs-gpu/src/main.rs:1611`, `:6316`, `:8020`, and
*asserted* by tests at `:16266` (`"horizontal is discarded"`), `:16337`,
`:16737`.

The GPU's half therefore cannot be "set the scroll and reshape". It
needs a mechanism that does not exist — a shifted text origin at paint
time, adjusted clip bounds, or something else — interacting correctly
with the gutter, the caret (`code_byte_px`), decoration geometry
(`push_glyph_extent_rects`), and hit testing (`gutter_aware_rel_x`),
each of which assumes x starts at `text_left()`.

**Answered in Q#HS1: the GPU is Stage 5.**

### 1.4 `view_top` is persisted; a `view_left` would want to be

`SavedLeaf` carries `path`, `cursor`, and `view_top` at
`DESKTOP_VERSION = 1` (`src/desktop.rs:33`, `:276-280`); the restore
path clamps `view_top` against the line count (`:512`). **Q#HS5.**

### 1.5 The cursor-follow hazard is already documented

`scroll_window` (`src/editor.rs:3628`) carries the cursor with a
vertical scroll, and its comment says why:

> The cursor must follow the scroll: the renderer has an "auto-scroll
> to keep cursor visible" pass that would otherwise snap `view_top`
> straight back to wherever the cursor sits, so the user's mouse-wheel
> scroll would feel stuck after one notch.

Under Q#HS2's answer (automatic-only) this pass is not a hazard but
**the entire mechanism** — Stage 4 adds its horizontal component. The
hazard returns with explicit commands, which is why Q#HS4 is deferred
rather than closed.

### 1.6 `goal_col` exists and its relationship to `view_left` is unexamined

`goal_col` (`src/window.rs:376`) remembers a target column across
vertical motion, cleared at seven sites in `src/editor.rs`. It is a
*column within the line*; `view_left` is a *viewport offset*. Both are
horizontal state on the same window, and a design ignoring the
interaction produces a cursor that jumps on the first vertical motion
after a horizontal scroll. Feeds **Q#HS7**.

---

## 2. The scope fact, stated before the answers

**Under `wrap`, horizontal scroll is meaningless** — nothing sits past
the right edge. So Stage 4's entire surface is conditional on a
buffer-local mode: one user-facing question ("how do I see the rest of
this line?") gets two disjoint answers depending on a setting. Stated
here because `COHERENCE.md` §20 requires it, and answered in Q#HS6.

---

## 3. Questions

### Q#HS1 — is the GPU in scope? **ANSWERED: no — Stage 5**

> **Answered 2026-08-07 (user):** split the GPU work into Stage 5.
> *"This is a conscious, bounded divergence, not a repeat of Stage 3's
> accidental one. Make the time box concrete and keep `wrap` default
> until parity lands."*

The distinction is the load-bearing part. Stage 3's defect was never
"the frontends differ" — it was "the frontends differ and **nobody
chose that**". A divergence that is decided, recorded, and bounded is a
different object from one inherited from a library default.

**The time box, concrete** (the user's requirement, and the part that
makes this a decision rather than a deferral):

1. **Stage 5 is the immediately-next QoL lane after Stage 4 merges** —
   not backlogged behind another arc. If something displaces it, that
   displacement is itself a decision to record here.
2. **`wrap` stays the default until Stage 5 lands** (independently
   reaffirmed in Q#HS6). This is what keeps the divergence invisible to
   anyone who has not opted in: a default-configuration user is never
   exposed to it.
3. **Stage 4's release notes must state the asymmetry** — horizontal
   scroll works in the TUI and not yet the GUI — in the same way #221's
   had to state the word-wrap loss.
4. **While the gap exists, the `truncate` affordances must name it.**
   §6's description amendment is where that lands, so a GUI user
   choosing `truncate` learns the limitation from the setting rather
   than from the behavior.

### Q#HS2 — what moves the viewport? **ANSWERED: automatic only**

> **Answered 2026-08-07 (user):** *"Automatic-only first. It makes the
> report's text reachable with no new command surface."*

The cursor-visibility pass gains a horizontal component, so moving the
cursor past the edge scrolls the view. No new commands, no binding
decisions, no new interaction island.

Explicit `ui.scroll-left` / `ui.scroll-right` are **not** in Stage 4.
They can follow with evidence of use, and they are what re-opens Q#HS4.

### Q#HS3 — per window or per buffer? **NOT ACTUALLY OPEN**

`view_left` is **per window**, unambiguously: two panes on one buffer
must scroll independently, exactly as they already hold independent
`view_top`s (`src/desktop.rs:92`). Stage 3's Q#LL2 recorded this and
accepted the consequence — the *mode* is buffer-local while the
*offset* is per-window, so one user-facing concept spans two scopes.

Listed so the accepted split is re-confirmed where it takes effect
rather than inherited silently.

### Q#HS4 — does the cursor follow an explicit scroll? **DEFERRED, not answered**

Not live under Q#HS2's answer: automatic-only means every viewport move
already originates from a cursor move. §1.5 holds the precedent and the
hazard for whenever explicit commands arrive.

Deferring rather than deleting, because the hazard is real and
rediscovering it costs more than carrying the paragraph.

### Q#HS5 — does `view_left` survive a restart? **APPROVED: yes**

> **Approved 2026-08-07 (user):** persist `view_left` **without** a
> desktop-version bump, **provided implementation adds
> `#[serde(default)]` and a literal v1 JSON fixture omitting the field,
> asserting restoration at zero.** Both conditions are part of the
> approval, not advice attached to it.

`view_top` does (§1.4). Consistency argues yes.

**Verified, not assumed:** `SavedLeaf` is a plain
`#[derive(Serialize, Deserialize)]` (`src/desktop.rs:85`) with **no
`#[serde(default)]` on any field and none anywhere in the file**. So
serde will **reject** a version-1 desktop JSON that omits a newly added
`view_left` — a missing field is a deserialization error, not a zero.
Revision 2 cited the struct's shape as though it settled serde's
behavior on it; it did not.

**"Yes, persisted, no `DESKTOP_VERSION` bump" is sound only with both
of:**

1. **`#[serde(default)]` on the new field.** This is the whole of the
   new-binary-reads-old-file direction.
2. **A regression fixture**: a literal version-1 desktop JSON with no
   `view_left`, deserialized in a test, asserting it restores at offset
   0 rather than erroring. Without this, (1) is an untested claim about
   a crate's behavior — which is precisely the failure this question
   was reopened for.

**The other direction already works, and that is why no bump is
needed.** An old binary reading a new file passes the version check
(`version` is still 1, `src/desktop.rs:366`) and then meets an unknown
`view_left` field — which serde **ignores** by default, and
`src/desktop.rs` sets no `deny_unknown_fields` anywhere (verified). So
both directions are safe at `DESKTOP_VERSION = 1` **given (1)**, and
neither is safe without it.

### Q#HS6 — the coherence statement **ANSWERED: keep `wrap` default**

> **Answered 2026-08-07 (user):** *"Keep `wrap` as default for now.
> Revisit only after GPU parity and use evidence; changing to
> `truncate` before Stage 5 would make the default unreachable in the
> GUI."*

That last clause is the argument revision 1 missed. I had framed this
as "if scroll makes `truncate` good, the default deserves
re-examination" — but with the GPU deferred to Stage 5, a `truncate`
default would ship a mode that is **navigable in the TUI and a dead end
in the GUI**, for every user who never opened the setting. Q#HS1 and
Q#HS6 are therefore coupled: the split is only safe *because* the
default does not move.

Revisit after Stage 5, on use evidence, not before.

### Q#HS7 — what IS `view_left`? **ACCEPTED (revision 4)**

> **Accepted 2026-08-07 (user):** *"keep `view_left` as an unsnapped
> window display column; derive the effective edge per line; render a
> bisected wide glyph's trailing cell as styled blank and designate it
> to the glyph start. The multi-line discriminating witness is exactly
> right."* Plus (c″) below, on the user's recommendation.

**Revision 1 decided what moves the viewport without ever saying what
the viewport offset is.** That is the same omission Stage 3 would have
made had it shipped `WrapMode` without `DisplayCoord`: the mode is
useless until the coordinate contract is written down, and the contract
is where every sharp edge lives.

Revision 1's verification sketch named tabs and wide characters. **It
had no oracle for either**, because nothing defined what a left edge
is. Four things must be settled together:

**(a) The unit.** Candidates: a source byte offset within the line; a
display column (cells from the line start, after tab expansion); or a
cell boundary with an explicit validity rule.

*My vote: display column.* Tab expansion depends on the absolute column
from the line start, so the walk must begin at column 0 and compute
forward **regardless** of the offset — which makes a byte offset buy
nothing and lose tab correctness. Starting at 0 and suppressing paint
until `col >= view_left` preserves tab stops **for free**, and costs no
more than `paint_line` already pays under wrapping.

**(b) What happens at a left edge that bisects a glyph.** A tab
straddling the edge is unambiguous: its expansion is width-1 spaces, so
the remaining ones paint. **A wide (width-2) glyph is not** — a grid
cannot paint half of one.

**(c) ~~The snap rule when an invalid edge is requested.~~ WITHDRAWN
(revision 3).**

> Revision 2 voted *"a left edge may not fall inside a wide glyph"* plus
> *"snap toward the line start, at the moment `view_left` is set"*.
> **That cannot hold, and the reason is structural rather than a detail
> to tune.**
>
> `view_left` is **one** per-window display column. "Does column N
> bisect a wide glyph?" is a **per-line** question: column 11 can be a
> wide glyph's trailing cell on line 3 and an ordinary ASCII cell on
> line 4. **No single setter-time value is canonical for every visible
> line**, so a snap performed once is simply wrong for most of them —
> and the invariant in (d), which the whole question exists to serve,
> would stay undefined exactly where it matters.
>
> Snapping *per line* is the other way to read it, and it is worse: the
> same source column would then appear at different screen columns on
> different rows, destroying the vertical alignment that a
> column-oriented view exists to provide.

**(c′) The per-line effective edge, which replaces it.**

`view_left` is stored **unsnapped** — the requested display column,
constrained only to `>= 0` and whatever maximum the design picks. Each
line derives its own **effective edge** during the walk it already
performs from column 0.

When the requested edge bisects a wide glyph *on this line*, that
glyph's trailing cell is the leftmost visible cell. It cannot be
painted as half a glyph, so:

- **It paints as a space**, carrying the glyph's own cell style.
- **The mapping designates that cell to the wide glyph's START byte.**

Both halves are load-bearing, and the second is the part the finding
correctly says was missing:

- The cell visually belongs to that character, so a click there
  selecting it is what a user expects.
- It keeps `byte_at_place` **total** over visible cells — every painted
  cell maps to some byte, with no hole at column 0.
- It preserves the round trip: `place_of_byte(glyph_start)` reports the
  straddle and designates cell 0, so `byte_at_place(0) == glyph_start`.

**And the direction rule `place_of_byte` needs at the left edge:** a
byte whose cells lie *entirely* left of the effective edge is **not
visible**, and `place_of_byte` must report that rather than clamping to
column 0. Clamping would make arbitrarily many bytes share cell 0 and
destroy (d). Only the straddling glyph designates cell 0.

**This is deliberately NOT the mirror of Stage 3's right-edge rule**,
and the asymmetry should be stated so nobody "fixes" one to match the
other. Under `wrap`, a wide glyph that will not fit at the right edge is
pushed to the next row **entirely** (`advance_wrapped`, with its
`max_cols >= 2` guard). At the left edge under `truncate` there is no
next row to push to, so the blank-plus-designation rule is what the
same intent requires here.

**(c″) A tab whose expansion straddles the edge — PRESERVED, not
chosen.**

Each visible tab-expansion cell maps to **the byte immediately after
the tab**. This is not a new rule: it is what `byte_at_place` already
does, and its doc comment says so — *"Rounds forward to the next
character boundary… matching the unwrapped `display_to_pos`"*
(`src/text_view.rs:224`). The walk accumulates `walked` past the tab
byte and returns on the *next* character's `start_col`, so every column
inside the expansion already yields the post-tab offset
(`src/text_view.rs:243-254`).

So the requirement on Stage 4 is **that horizontal scroll not perturb
it**: with the expansion's leading cells scrolled off, the surviving
cells must still map post-tab, exactly as they do at offset 0.

**Why this direction differs from (c′)'s, which is the obvious
objection.** A wide glyph's two cells belong to **one character**;
forward-rounding its trailing cell would designate it to the *next*
character and leave the straddling glyph with **no visible cell mapping
to it at all** — unreachable by click precisely when it is the thing
the user scrolled toward. A tab's expansion cells are whitespace
*between* the tab byte and the next character, and forward-rounding
them is already how clicking in indentation lands at the start of the
text. Different directions, one principle: **every visible cell is
designated to the byte a user would mean by clicking it.**

With (c′) and (c″) together, the (d) contract is total over visible
cells: ordinary character → its own start byte; bisected wide glyph →
the glyph's start byte; tab expansion → the byte after the tab.

**(d) The invariant rendering and coordinate mapping share.** For a
given `view_left` **and line**, `byte_at_place` must invert
`place_of_byte` on every canonical input, and `paint_line` must place
exactly the bytes `place_of_byte` claims. If the painter clips where the
mapper does not, **clicks land on the wrong character** — silently, and
only for lines wide enough to scroll.

**"And line" is what (c′) forced**, and it is the whole of that
finding: the invariant is not a property of `view_left` alone. It is a
property of `(view_left, line)`, because the effective edge is derived
per line. A test that fixes one line and sweeps offsets will not see
the failure; the oracle has to sweep **lines whose glyph widths differ
at the same column**.

*This is the invariant the verification sketch needs as its oracle*,
and it is why Q#HS7 blocks: §4 cannot be written until it exists.

---

## 4. Verification sketch (depends on Q#HS7)

- Cell-level tests at several window widths **at non-zero offset** —
  the case Stage 3's sketch explicitly refused, because "at non-zero
  offset" was a `view_left` requirement smuggled into a wrap lane.
- **A `wrap` control for every claim**, asserting the wrap path is
  byte-identical with a horizontal offset present. Under `wrap` the
  offset must be **inert**, not merely harmless.
- Round-trip identity for `place_of_byte` / `byte_at_place` at non-zero
  offset — the Q#HS7(d) invariant, walked exhaustively over a short
  line rather than sampled, as Stage 3 established.
- **The Q#HS7(c′) case**: a wide glyph straddling the left edge — the
  trailing cell blank, and `byte_at_place` on it returning the glyph's
  **start** byte.
- **The Q#HS7(c″) case**: a tab whose expansion straddles the edge,
  with every surviving cell still mapping to the byte **after** the
  tab. This one is a **regression** witness rather than a new claim —
  `byte_at_place` already behaves this way at offset 0
  (`src/text_view.rs:224`), so the test asserts scroll did not perturb
  it, and it should fail if the walk is "optimized" to start at the
  effective edge instead of column 0.
- **A multi-line fixture whose glyph widths DIFFER at the same column**
  — the case (c′) exists for, and the one a single-line sweep cannot
  reach. At one `view_left`, one line must take the straddle path and
  another the ordinary path, with the (d) invariant holding on both.
  A test that fixes one line and sweeps offsets passes against the
  withdrawn setter-time snap, which is what makes this the
  discriminating fixture rather than an extra one.
- **A PTY acceptance test for reachability**, following
  `tests/long_line_readable_acceptance.rs`. That file's `truncate`
  control currently asserts the tail is **absent** — Stage 4 must
  update it, and **that update is itself the proof the caveat is
  gone**.
- **No GPU witness in Stage 4** (Q#HS1). Stage 5 owes one that the
  caret, a decoration, and a hit test all agree with the shifted
  origin — the three consumers §1.3 names.

---

## 5. Coherence impact (§20 requirement)

- **Journey step 4, "Understand interface — Partial."** Stage 4
  completes "a line that cannot be read in full" for `truncate` **in
  the TUI only**. The scorecard row should say so rather than reading
  as closed.
- **§16 Semantic Frontend Architecture.** Q#HS1 *widens* frontend
  divergence for the duration of the Stage 4→5 gap. Per the answer,
  this is deliberate and time-boxed, and materially different from
  Stage 3's inherited accident — but the release notes must say which
  kind it is, or a reader cannot tell them apart.
- **No new interaction island** — automatic-only adds no command
  surface (Q#HS2).
- **Config registry: no new settings expected.** Stage 4 navigates the
  mode Stage 3 declared. If it needs a setting, that is a signal the
  design has drifted, not a feature.

---

## 6. A Stage 4 deliverable that is not scroll

**Amend `ui.line-wrap`'s description** (`builtin/runtime/linewrap.lua`).
Today it says only *"…or truncate at the edge"*, which does not tell a
user that the edge is a wall. The honest text depends on where Stage 4
lands:

- **Before Stage 4**, `truncate` means unreachable in both frontends.
- **After Stage 4**, it means reachable in the TUI and unreachable in
  the GUI until Stage 5 (Q#HS1's time box, item 4).
- **After Stage 5**, the caveat is gone and the sentence should shrink
  back.

Carried here rather than filed elsewhere because it is the one place
the arc's user-visible honesty is currently wrong, and revision 1
asserted it was already right.
