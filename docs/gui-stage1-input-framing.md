# GUI arc, Stage 1 — input foundation (framing)

**Status: revision 20 — §2a RE-MEASURED at `0ec13b3`, and three base
facts corrected.** Revision 18's §2a was measured at `72da24a`. That
base is now **167 commits back**, and #242 and #243 landed on exactly
the pointer paths 1b builds on, so every 1b anchor is stale again —
which §2a's own history predicted would happen.

**All four load-bearing claims were re-verified and all four SURVIVE:**

1. **Quantization still happens BEFORE routing.** `:3254` quantizes,
   `:3262` returns on zero, `:3270` routes to the panel. CORRECTION 5's
   central point is intact — this was checked first, because an
   inversion would have changed B1's design;
2. **`Elsewhere` still collapses four wheel targets** — its doc at
   `:2108` still reads *"Not the band: the document, the terminal, the
   minimap, or the chrome."*;
3. **the wheel's panel branch still tests `PanelCell` only** (`:3270`),
   so minimap pixels still fall through to `scroll_by_lines`. **B6's
   premise survives**;
4. **`code_scroll_residual` is still not a wheel accumulator** — it is
   the caret-follow pixel residual (`:1664`), cleared at `:6218` and
   `:8433`. Reusing it would still be a defect.

**Three base facts were wrong, and are corrected in place:**

- `pmacs-gpu/src/main.rs` is **22,502 lines**, not 21,435;
- **`PROTOCOL_VERSION = 25`**, not 24 — v25 was taken by #242's
  cell-mapping generation;
- **the panel-replay premise is stale.** §2a said a panel wheel "does
  nothing today, on either axis". **#243 landed the replay and it is in
  this base**: the dispatcher is split into `classify_panel_pointer`
  (`src/editor.rs:2985`) and `apply_panel_pointer` (`:3088`), and the
  vertical axis replays for real at `:3277`–`:3278`. **The prerequisite
  is DISCHARGED** — completely, within its scope, and it supplies the
  vertical receiver path. **It discharges none of B1's witness**: 1b
  still owes an end-to-end panel-wheel **effect** witness on **both**
  axes, because B1's fractional accumulator is NEW and no #243 test
  exercises it. Implementation already present is not evidence already
  owed.

**Citation policy, applied throughout:** a citation that makes a claim
about the tree **as it is** was re-measured and renumbered — 45 in the
first pass, plus **six the first pass got wrong**. A citation that records **what an earlier revision cited** is
left exactly as written and its block is labelled *"citations are
HISTORICAL (`72da24a`)"*. Renumbering those would have falsified
sentences whose whole point is that an earlier citation was wrong.

**The first pass verified RANGE, not IDENTITY, and that was the wrong
check.** Every citation landed inside its file, and six still pointed at
the wrong construct: B4's `UnusedButton`/`route_pointer` (`:3803`,
`:3811`, misfiled against `editor.rs` when they are `main.rs`), the
panel paint call (`src/editor.rs:2863`), `OwnCursor`'s **type doc**
(`:2448`) as against its field (`:1774`), the GPU **wrap branch**
(`:8120`) as against the `horizontal_follow` call (`:8089`), and
`scroll_top`'s zeroing (`:6217`), which sits one line before the
residual's. **Every live citation is now identity-checked against the
construct its prose names**, not merely bounds-checked.

**No ruling changes in revision 20.** It is a re-measurement, three
corrections, a discharge, and one conceptual separation — implementation
already present is not evidence already owed.

**Previously, revision 19 — RECONCILES TWO LINEAGES THAT BOTH NUMBERED
THEMSELVES 13.** This document was advanced independently by two
branches, and both called their change "revision 13" — the second
block at `revision 13` below is the other lineage's, left exactly as it
was written. **Nothing is renumbered retroactively**, because rewriting
those numbers would falsify sentences like revision 15's "three defects
in 14".

| lineage | revisions | contribution | approval |
|---|---|---|---|
| `gui-stage1b-pointer-scroll` | 13 → 18 | **§2a, 503 lines** — six corrections, B1's wheel-target enumeration, Q#S1-11 ruled (B, viewport only), the five-clause lifetime contract with L1–L8 | **APPROVED at 18** |
| the panel-replay lane (`1fc3af6`, #243) | its own 13 | the enriched **B1/B2/B3 rows** and the 12-line "Panel-replay consequence" paragraph in §1b | **approved by merge** — #243 landed it on `main`; its status line was never updated and still reads "AWAITING APPROVAL" |

**Both are kept in full, and that was verified rather than assumed:**
the enriched rows, the panel-replay paragraph, `wrap pins horizontal
origin to zero`, §2a, the six-target enumeration and L1–L8 are all
present after the rebase.

**They are complementary, and the one place they could have collided is
worth naming.** The panel-replay lane's B1 row names **three** residual
owners — primary document, panel document, minimap. §2a's model is a
**superset**: R2 panel, R3 terminal, R4 document *(sharing chrome's
residual)*, R5 minimap. So §2a adds the **terminal** as an owner and
**rules that chrome shares the document's**; it does not overturn the
three the table names.

**Where the table and §2a disagree, §2a wins**, exactly as §1b's own
blockquote already says: it replaces B1's undefined "surface" with the
enumeration and B3's "content bounds" with B7's saturated upper bound.
The enriched rows stand for everything else they add — notably B2's
window-targeted `view_left` path and B3's wrap-pins-to-zero.

**Nothing in revision 19 changes a ruling.** It is a reconciliation and
a record.

**Previously, revision 18 — APPROVED (2026-08-14); its citations are HISTORICAL (`72da24a`).** §2a is the 1b ground
truth and its rulings are settled: **Q#S1-11 (B)**, the five-clause
lifetime contract with witnesses L1–L8, B1's six-target enumeration
with discriminators **R1–R5**, and B3's exact saturated bound. Revision
18 answers review of 17:

- **Identity has a second half that R2/R3 cannot reach.** A panel or
  terminal residual is keyed to a surface that *goes away*; the
  **document's and the minimap's live in the long-lived GPU `State`
  and outlive the buffer**. Their reset is a **manual list** —
  `pmacs-gpu/src/main.rs:5922` zeroes `scroll_top`,
  `code_scroll_residual` and `code_scroll_left` one line at a time — so
  a new wheel residual will not reset itself. **R4** (document plus the
  chrome residual it shares) and **R5** (the minimap's independent
  residual) add buffer A→B legs, each with its own mutation omitting
  its own reset. `code_scroll_left` had this exact defect once already.
- **U9/U10 said "the same executable"; the two artifacts differ.**
  `pmacs-9d4963…` and `pmacs-7624…`, different sizes, differing bytes.
  The supported claim is the same *target*, 1,940-test population and
  default intra-binary threading. **"Cargo runs test binaries
  sequentially" is narrowed to the observed cargo 1.95.0 sweep** rather
  than asserted universally. The concurrency conclusion is unaffected.

**§2a took six review rounds (13–18), and each one found a witness that
would have passed against a broken implementation.** Emission-only
panel rows against a receiver that discards; a GPU preservation row
driven by a paint that never calls the follow; lifetime rows whose
setup left held and released authority indistinguishable; mutation
dependents predicted rather than derived; and a residual-identity story
that stopped at the surfaces which get destroyed, missing the two that
outlive the buffer. **None was a design disagreement** — the rulings
survived from 14 and 15 untouched. That is the failure mode §6 names,
arriving once per round, and it is why the R- and L-rows specify their
*drivers* and *setups* rather than only their assertions.

**Previously, revision 17 — SUPERSEDED; its citations are HISTORICAL (`72da24a`).** Two additions and one
correction:

- **B1 gains the two separations its crossing witness could not
  reach.** "Per axis and surface" has two nouns; 16 witnessed one
  category crossing and left both of the others open. **R1** pins the
  **axes** apart (a sub-tick horizontal then a sub-tick vertical over
  one surface must reach no tick), and **R2/R3** pin **identity**
  apart — panel A→panel B and terminal A→terminal B, so "per surface"
  cannot degrade to "per surface *kind*". Each has a mutation that
  shares the store in question. The end-to-end panel row now runs
  **both axes**, the horizontal leg being the one with no handler at
  all.
- **Two mutation dependencies were assumptions and are removed.**
  "Never releases" need not bite L8 — clause 5's wrap/replacement clear
  is a separate path — and "writes point" need not bite L4, which
  concerns the vertical path. Scoped to L3 and L5. Dependents are
  recorded **only after a mutation run shows them**.
- **U10's concurrency claim was false and is withdrawn**, along with
  U9's candidate that prompted it. Detail in
  `docs/ci-red-signatures.md`; the short form is that cargo runs test
  binaries **sequentially**, so the two steps never differed in
  concurrency and no experiment reversed anything.

**Previously, revision 16 — SUPERSEDED; its citations are HISTORICAL (`72da24a`).** Four changes, three of them
corrections to 15's own reasoning:

- **The panel-replay lane becomes a HARD PREREQUISITE in §3's
  topology**, not a note. The defect is "frontend emits, receiver
  discards", so 15's emission-only witnesses would have reproduced the
  blind spot that hid it. The lane merges, **1b rebases onto it**, and
  1b carries an **end-to-end panel-wheel effect** row. 15's rationale
  *"it is not input work"* was wrong — replay includes terminal mouse
  reporting and click-to-focus. The real reason is **breadth and
  ownership**: acceptance 48 (`docs/bottom-panel-framing.md:1719`).
- **Divider and background must BANK NOTHING.** 15 gave them the panel
  cell's residual, which manufactures the surface-switch jump B1
  forbids — motion banked over an inert target, spent on entering a
  cell. They discard both axes, and a **crossing witness** pins it.
- **The discriminating setup applies to every lifetime row**, not just
  L3. With the cursor inside the manual viewport, held and released
  authority produce the **same origin**, so the rows pass either way.
  **L2 becomes a height-only GPU resize**, and **L7 gains a
  content-shrink leg** for the half viewport-widening cannot witness.
- **The mutation table follows §6's dependency-aware rule.** 15's
  heading promised "own rows and no others", which this document
  already says is false and unachievable; mutations now name the rows
  they must **bite** and the **legitimate dependents**.

**Previously, revision 15 — SUPERSEDED. Every citation and every
present-tense claim in this block is HISTORICAL, measured at
`72da24a`**; see revision 20 for the live sites, and note in particular
that "replays nothing" was true then and is **not** true on `0ec13b3`.
Answered review of 14 and
**changed two B-row contracts**, which 14 wrongly denied.

- **B1 is now fully ruled**, not half-ruled. Revision 14 left two
  "must be ruled" cells in the wheel-target table, which is a question
  wearing a table's clothes. Every cell is normative now, and the two
  gaps are closed from measurement: the **terminal** emits both axes
  (the SGR encoder already carries codes 66/67, and a non-reporting
  terminal is inert, matching the TUI), and the **panel** emits both
  axes while its missing **replay** is repaired in a **prerequisite
  lane** rather than absorbed here. Divider, background and chrome are
  ruled too.
- **Revision 14 cited a TEST FIXTURE as the panel's production
  handler** (`src/daemon.rs:6683`, inside `#[cfg(test)]`). The real
  path validates and focuses and **replays nothing** — a pre-existing
  violation of an already-ruled contract, which the wrong citation hid.
- **The GPU preservation witness 14 specified was vacuous.** "The next
  paint" is TUI-only; the GPU's `render()` never calls
  `horizontal_follow`. The lifetime rows are respecified per frontend.
- **Five clauses now have eight witnesses**, not three. 14 left point
  and selection, clamp-absorbed motion, re-clamping, and the authority
  latch under wrap unconstrained.
- **Clause 3's direction was backwards** in 14: a *wider* viewport
  lowers the maximum origin.

**What is normative here, stated plainly:** §2a **changes B1 and B3**.
B1 gains a per-target enumeration it did not have, and B3 gains B7's
exact upper bound in place of "content bounds". B2, B4, B5, B6 and B7
are unchanged.

**Previously, revision 14 — SUPERSEDED; its citations are HISTORICAL (`72da24a`).** Ruled Q#S1-11 (B) with the
five-clause lifetime contract, and gave B3 B7's exact saturated bound.
Both stand.

**Previously, revision 13 — SUPERSEDED; its citations are HISTORICAL (`72da24a`).** §2a's ground-truth
re-measurement for Stage 1b at the post-#240 tip. Every 1b anchor was
stale, as expected; what was not was that **three B-rows describe a
field as empty when it is occupied**, and that **B7 re-opens a question
another framing deliberately deferred**. Its claim that the snap-back
lands on the next *caret event* was corrected by 14 to the next
**paint**, and by 15 to *per-frontend* drivers.

**Previously, revision 13 (the PANEL-REPLAY lineage) — approved by
merge in #243; the status line it shipped with is preserved verbatim
below and was never updated.**

**Status: revision 13 — AWAITING APPROVAL with bottom-panel §5a
revision 16.** Revision 13 changes no previously ruled outcome. It
makes B1–B3's “per surface” scope explicit for the panel-document
surface, closing the ownership gap exposed by §5a's remeasurement:
panel replay is the prerequisite, while GUI 1b owns horizontal tick
production, residuals and the document-panel `view_left` effect.

**Previously, revision 12 — APPROVED.** Revision 12 is §2's ground-truth
re-measurement for Stage 1a and changes no ruling; it carries two
corrections to claims that were wrong at the original anchor too.

**Previously, revision 11 — APPROVED.** Revisions 1–8 rejected; revision 9
is the approved design. Revision 10 recorded a scope correction found
against the 1-pre implementation and **also made a claim about P2 that
review overturned; revision 11 retracts it and P2 is implemented as
written** (§6). **Q#S1-8, Q#S1-9 and Q#S1-10 are RULED.** **1-pre is
IMPLEMENTED**; 1a onward may begin from this document.

**v26, not v25 — corrected by the panel mapping-generation slice.**
That slice (`docs/bottom-panel-framing.md` §5b) takes **v25** for
`PanelFramePayload::PresentMapped` / `FrontendEvent::PanelPointerMapped`,
and it lands ahead of 1e because panel-pointer replay blocks 1b.
Protocol slices stay serialized; one was inserted in front.
`ADVERTISED_PROTOCOL_VERSION` remains pinned at **20**.

**Verification base: `0ec13b3`** (revision 20). §2 was re-measured at
`4f77491` (2026-08-12) and §2a **originally** at `72da24a`
(2026-08-13); both are historical anchors now, kept so the
re-measurements can be checked against what they replaced.
§2 is **re-measured at `4f77491`** (2026-08-12),
the tip after 1-pre; it was originally taken at `a994f37`. **§2a is
measured at `0ec13b3`** (revision 20) — originally at `72da24a`
(2026-08-13), the tip after 1a and #240 — and it is the base for **1b
only**; it carries Q#S1-11's ruling. Sections
other than §2/§2a were written against `a994f37` and their *rulings*
are unaffected by 1-pre, which changed no behaviour — but **any line
number outside §2 and §2a
predates 1-pre and should be re-checked before it is relied on.** The
1b table's own anchors are superseded by §2a wholesale; read §2a first.

## 1. What this stage closes

Journey **step 5**. **Not step 12** (Stage 4b, P2-gated). Five of nine
§3.1 blockers die here.

## 2. Ground truth — RE-MEASURED at `4f77491` (2026-08-12)

**Originally taken at `a994f37`, before 1-pre.** 1-pre (#237) moved
almost every GPU-side coordinate below, so the section is re-measured
rather than left to rot — **a framing whose ground truth points at the
wrong lines is how an implementation ends up arguing with the tree**.
Two claims were *wrong at both anchors* and are corrected, not merely
renumbered; they are marked **CORRECTION**.

### Still true, re-checked

- **`FrontendEvent`: sixteen variants**, none carrying an open path or
  command invocation. **`PROTOCOL_VERSION = 23`.**
- **`WindowEvent::Ime` is ignored entirely** —
  `set_ime_allowed`/`WindowEvent::Ime` still **zero occurrences**, so
  `Ime::Commit(String)` produces nothing. (1d's D1.)
- **TUI wheel arms unmoved**: `EditorState::dispatch_mouse`
  (`src/editor.rs:3052`), `ScrollUp`/`ScrollDown` at **`:3203`** — 1-pre
  touched only `pmacs-gpu`.
- **The handshake precedes any window**: the client is constructed and
  the handshake done before `run_app` (connect `main.rs:702`, `run_app`
  `main.rs:733`; the old citation `:696` was the enclosing block).
- **1c is producer-side only for Focus/Detach** — those variants exist
  on the wire. **Title, Bell and `Goodbye` are GPU consumer work.**
- **`Outbox::enqueue` returns `false` once closed** and **coalesces by
  kind** (`attach.rs:415`; the old `:414` was off by one at both
  anchors).

### Moved by 1-pre

- **`App::window_event` is `main.rs:4450` and is FOUR lines**, not 655
  at `:2734`. It calls `dispatch_window_event` and performs the exit;
  **routing lives in `route_event`, and the bodies in seven `apply_*`
  methods.**
- **"Eight arms handled, the rest fall to `_`" is now three family
  decision functions** — `route_lifecycle`, `route_keyboard` (plus
  `route_key_action`), `route_pointer` — over **nine** named
  `WindowEvent` variants, with `Route::Unrouted` as the wildcard. **1a
  edits `apply_keyboard` and `translate_key`, not `window_event`.**
- **`translate_key(logical: &Key, …)` is `main.rs:12053`**, not
  `:10975`. It still reads the **logical key** and still truncates via
  `chars().next()`, and `_ => return None` is still there — so **A1's
  witness holds**.

### CORRECTION 1 — `KeyEvent.text` IS read, and always was

The original section said *"`KeyEvent.text` is never read."* **That is
false, and was false at `a994f37` too** (`:2800` there, `main.rs:3251`
now): the AltGr rule reads it, as `is_layout_text(key.text.as_deref(),
pmods)`.

The claim the section meant, and which is true: **`KeyEvent.text` is
never read as the text a keypress INSERTS.** It is consulted only as a
*discriminator* — Ctrl+Alt plus printable text means AltGr rather than a
command chord — and the inserted character always comes from
`translate_key`'s logical key, truncated to one scalar.

**This matters to A5, not just to accuracy.** §5's rule 2 exempts
"printable Ctrl+Alt recognized by the existing AltGr rule", so the
precedence table already depends on the code the section claimed did not
exist. **1a widens `text` from a discriminator to a payload**, and that
is the actual change of kind — stating it as "text is never read" hides
the one place the new payload must not disturb.

### CORRECTION 2 — A4's exit site

A4's witness cited *"exits (`main.rs:2771`)"*. 1-pre moved the mechanism
without changing the behaviour: an idle Escape still exits, but
`apply_keyboard` (`main.rs:3219`) now returns `EventOutcome::Exit` and
**`window_event` (`main.rs:4452`) performs the only executable
`event_loop.exit()` in the crate.**

**A4 therefore deletes a branch in `apply_keyboard` and changes its
return type — it does not touch `window_event`.** And **`EventOutcome`
survives A4**: a native close still returns `Exit`, and
`dispatch_window_event` must still distinguish it from `Continue`.

**Today's two failures are unchanged by any of this:** multi-scalar
keyboard input is truncated to its first scalar, and an IME commit
produces nothing.

## 2a. Ground truth for 1b — RE-MEASURED at `0ec13b3` (revision 20); originally at `72da24a` (2026-08-13)

§2 above was measured for **1a**, at `4f77491`. 1a (#239) has since
merged and #240 landed on top, so **every coordinate the 1b table cites
is stale** — `pmacs-gpu/src/main.rs` is now **22,502 lines** and 1-pre's
router extraction moved the wheel path wholesale. Renumbering alone
would have been routine. It is not what the measurement found.

**`PROTOCOL_VERSION = 25`** now (`pmacs-protocol/src/message.rs:2002`)
— **not 24**, which is what this line said until revision 20: v25 was
taken by the panel cell-mapping generation (#242). 1b remains
non-protocol-bearing either way.

### The anchors, re-measured

**Left column: HISTORICAL — what §1b's table cited, as measured at
`72da24a`. Right column: LIVE, re-measured at `0ec13b3`.**

| the table cited (historical, `72da24a`) | held what then | the real site NOW (`0ec13b3`) |
|---|---|---|
| `main.rs:2061` "minimap is `Elsewhere`" | `PanelCell(CellCoord)` | `enum PointerSurface` `:2099`; **`Elsewhere` `:2109`** |
| `main.rs:3337` wheel reads `pointer_pos` | an `attach_client` line | `apply_wheel` `:3270` and `:3293` |
| `main.rs:3373` falls to `scroll_by_lines` | a bare `}` | `apply_wheel` `:3306`; `scroll_by_lines` `:8417` |
| `editor.rs:3052` `dispatch_mouse` | — | **`:3920`** |
| `editor.rs:3203` scroll arms | a doc-comment line | `ScrollUp` `:4296`, `ScrollDown` `:4299` |

**B6's premise survives the move intact**, and is now stated by the code
itself: `Elsewhere`'s doc comment reads *"Not the band: the document,
the terminal, the minimap, or the chrome."* The wheel's panel branch
(`:3270`) tests `PanelCell` only, so minimap pixels still fall through
to `scroll_by_lines`.

**B1's defect is visible in four lines.** `apply_wheel` rounds to whole
lines and returns on zero:

```rust
let lines = match delta { LineDelta(_, y) => (-y * WHEEL_LINES_PER_TICK).round() as i64, … };
if lines == 0 { return; }
```

`:3254`–`:3262`. The `_` is the **x** delta, discarded at the same site
— so **B1 and B2's witnesses are the same four lines**, and a residual
accumulator is what both need. **There is no wheel accumulator today.**
`code_scroll_residual` (`:1664`) is *not* it: that is the caret-follow
pixel residual, buffer-scoped, cleared at `:6218`. **Reusing it would
be a defect**, not a shortcut.

### CORRECTION 5 — B1's "surface" is not enumerated, and the classifier cannot enumerate it

B1 says "residual per **axis and surface**" and the table leaves
"surface" undefined. Two facts make that a hole rather than a detail.

**Quantization happens BEFORE routing.** The rounding and the
`lines == 0` return are at `:3254`–`:3262`; the panel branch is `:3270`
and the terminal branch `:3292`. So a sub-tick delta bound for the
panel or the terminal is **discarded before anything knows where it was
going**. An accumulator added after the routing decision would fix the
document and leave the wire targets exactly as broken as they are now.

**And `PointerSurface` cannot name the surfaces B1 needs.**
`classify_pointer_surface` (`:7577`) resolves panel geometry only;
`Elsewhere` (`:2109`) is *"the document, the terminal, the minimap, or
the chrome"* — four wheel targets under one name, three of which B1 and
B6 must distinguish. **B1 needs a wheel-target enumeration; it does not
get one for free from the existing classifier.**

Note also a live consequence of `:3270` matching `PanelCell(_)` alone:
**a wheel over the panel DIVIDER or the band's BACKGROUND scrolls the
document today**, though the enum's own doc says the band "still owns
the pixel."

The enumeration B1 carries. **Every cell is normative** — "vertical
today" is the measurement, "vertical RULED" is the contract, and where
they differ 1b closes the gap:

| wheel target | classified | vertical today | vertical RULED | residual owner | horizontal RULED |
|---|---|---|---|---|---|
| Panel cell | `PanelCell` | emits (`:3282`); **vertical replay LANDED in #243** (`src/editor.rs:3277`) | emit **both axes**; the horizontal leg is B1–B3's | per panel | emit `ScrollLeft`/`Right` |
| Panel divider / background | `PanelDivider` / `PanelBackground` | **scrolls the document** | **consume both axes** — the band owns the pixel | **none — discards** | consumed, no emit |
| Terminal | `Elsewhere` + `terminal.is_some()` (`:3292`) | emits (`:3302`) | emit **both axes** | per terminal | emit `ScrollLeft`/`Right`; **inert when not reporting** |
| Minimap | `Elsewhere` | document `scroll_by_lines` | document viewport, **own residual** (B6) | **its own** | inert |
| Document | `Elsewhere` | `scroll_by_lines` (`:3306`) | unchanged | document | `code_scroll_left` (B3) |
| Chrome | `Elsewhere` | document | **shares the document's**, deliberately | **the document's** | shares the document's |

**Divider and background consume both axes, and BANK NOTHING.** Falling
through to the document contradicts `PanelBackground`'s own doc —
*"the band still owns the pixel"* — and is a measurement, never a
decision.

**Revision 15 gave them the panel cell's residual. That was a defect,
and it is the very jump B1 forbids.** Motion over an inert target would
be banked and then spent the instant the pointer entered a cell, so a
gesture that scrolled nothing could complete a tick on arrival — a
surface-switch jump manufactured by the accumulator itself. Consumption
therefore **discards and resets both axes**; if an implementation keeps
state here at all, it must be state that **can never combine with cell
input**.

**Crossing witness (required):** a partial motion over the background,
then a partial motion over a cell, **must not reach a tick** — neither
sub-tick alone is enough, and the first must not be available to
complete the second. The mutation is exactly revision 15's error:
share the cell residual with the background, and this row jumps.

##### "Per axis and surface" needs three separations, not one

The crossing witness above proves **one** of them — a category change,
background to cell. **Two more are unwitnessed by it, and an
implementation can satisfy every row written so far while failing
both.** B1's phrase has two nouns in it, and each needs its own
discriminator:

**Axis.** A single accumulator per surface, fed by both axes, passes
every row above: each one moves in one axis at a time. **R1 —
cross-axis:** a sub-tick **horizontal** motion followed by a sub-tick
**vertical** motion over the *same* surface must reach **no tick on
either axis**. *Mutation: one residual per surface instead of one per
(surface, axis)* — the two half-ticks combine and R1 sees a tick.

**Identity, not just kind.** "Per surface" that means "per surface
*kind*" leaks between two panels or two terminals, which is the same
defect as leaking between kinds but invisible to a kind-level row.

- **R2 — panel A→panel B:** bank a sub-tick over panel A, replace it
  with panel B (the `buffer_id`/`panel_epoch` replacement acceptance 49
  already models), motion over B must **start from zero**. *Mutation:
  key the residual by "the panel" rather than by panel identity.*
- **R3 — terminal A→terminal B:** the same, across two terminal
  buffers. *Mutation: key the residual by "the terminal".*

**R2 and R3 are not the whole of identity, because the document and the
minimap do not get replaced — their state OUTLIVES the buffer.** A
panel or terminal residual can be keyed to a surface that goes away; the
document's and the minimap's live in the long-lived GPU `State`, which
survives a buffer switch. **Their reset is manual, and there is a list
to be forgotten from.** Buffer replacement (`pmacs-gpu/src/main.rs:6217`
onward) zeroes `scroll_top`, `code_scroll_residual` and
`code_scroll_left` **one explicit line at a time**, each with its own
comment — including `code_scroll_left`, added later precisely because a
switch *"inherits the PREVIOUS document's leftward viewport and renders
the new buffer scrolled sideways until a cursor motion repairs it — a
symptom nothing about the new buffer explains."* **A new wheel residual
will not reset itself.** Two more legs:

- **R4 — document A→document B**, covering the document residual **and
  the chrome residual it shares** (chrome's owner is the document's, so
  one reset must serve both): bank a sub-tick over the document, replace
  the buffer, motion over B **starts from zero**. *Mutation: omit the
  document residual from the replacement reset.*
- **R5 — minimap across A→B:** the minimap's residual is **independent**
  (B6), so it needs its own reset and its own leg. *Mutation: omit the
  minimap residual from the replacement reset.*

Two mutations, deliberately separate: **one reset omitted at a time**,
because a single "forgot to reset" mutation would bite both legs and
prove neither field is individually covered. This is the same defect
`code_scroll_left` already had once, and the reason it now has a
comment explaining itself.

R2 and R3 also pin the **discard on disappearance**: a residual banked
against a surface that no longer exists must not be inherited by its
successor. That is the identity question and the staleness question at
once, and acceptance 49's epoch machinery is the existing precedent for
answering it.

**The end-to-end panel row exercises BOTH axes.** "A wheel moves the
panel viewport" leaves the horizontal half untested precisely where
the emit-and-discard defect lived — the axis with no handler at all.
Vertical and horizontal are separate legs.

**Chrome shares the document's scrolling and the document's residual.**
That is today's behaviour, and making it normative is what keeps 1b
from having to change both frontends' hit testing for no user-visible
gain. It is written down so it is a choice rather than a leak.

**The horizontal gap is not a wire gap.** `MouseKind::ScrollLeft` and
`ScrollRight` already exist (`pmacs-protocol/src/message.rs:245`,
`:247`) and round-trip (`src/protocol.rs:720`), so **emitting them
needs no protocol bump** — 1b stays non-protocol-bearing.

**The terminal answer follows from the encoder.** `sgr_mouse_report`
(`src/terminal/input.rs`) already encodes `ScrollLeft` as **66** and
`ScrollRight` as **67** (`:130`–`:131`), so the terminal handles both
axes the moment they are emitted. Its guard (`:120`) returns `None`
unless `mouse_sgr` is on and tracking is not `Off`, so a
**non-reporting terminal is horizontally inert** — which matches the
TUI, where nothing consumes a horizontal tick either. No new arm is
needed.

#### The panel replay gap — a pre-existing defect, repaired in a prerequisite lane

**Revision 14 cited `src/daemon.rs:6683` as the panel's `ScrollUp`
handler. That was wrong: it is a test fixture**, inside `#[cfg(test)]
mod tests` (opened at `:6813`). Citing a fixture as production is
precisely the error B1's enumeration exists to prevent, and it hid a
real defect.

**As measured at `72da24a`**, the production path was
`dispatch_semantic_panel_pointer` (`src/editor.rs:2674` *at that base*).
It validated the coord against the panel grid, resolved the side
window, focused when the gesture activated — and **returned `true`
without replaying anything.** Its own doc said so: *"**Replay is out of
scope in Stage 2B-2.** Driving selection, listview rows, or child SGR
reporting is **parent acceptance 48**, which needs the GPU band and
lands in **Stage 2B-3**."*

**THAT IS NO LONGER THE BASE. #243 landed the replay, and revision 20
re-measures it.** On `0ec13b3` the dispatcher is split into
`classify_panel_pointer` (`src/editor.rs:2985`) and
`apply_panel_pointer` (`:3088`), and the effect path replays a **panel
wheel's vertical axis** for real: `PKind::ScrollUp =>
self.scroll_window(side, -SCROLL_LINES)` and its `ScrollDown` twin
(`:3277`–`:3278`).

**So "a panel wheel does nothing today, on either axis" is FALSE on this
base, and the sentence is corrected rather than deleted** — it was true
when written and is the reason the prerequisite lane existed. What
remains is narrower and was already assigned here: **the horizontal
document-panel leg is B1–B3's**, per the "Panel-replay consequence"
paragraph in §1b.

**Ruling as made (historical): 1b does not absorb it, and 1b does not
ship before it.** The replay was to be repaired in a **prerequisite
lane** carrying parent acceptance 48, with **1b depending on that
lane**. Two reasons were given, and the one revision 15 gave was wrong:

1. **Replay is broader than 1b, and already owned.** Acceptance 48
   (`docs/bottom-panel-framing.md:1719`) has `PanelPointer` driving
   *"listview row selection, panel selection, terminal mouse reporting,
   and click-to-focus without disturbing the document mirror"*, plus
   activation ordering and coalescing rules. A wheel is one gesture in
   that set. Implementing it alone from an input slice would deliver a
   fragment of an acceptance criterion owned elsewhere.
2. **The defect predates 1b** and was not horizontal-specific:
   vertical panel scrolling was **equally dead at `72da24a`**. A fix
   belongs where the contract lives, not bolted to the slice that
   happened to find it.

**STATUS AT `0ec13b3`: the ruling was carried out and the dependency is
DISCHARGED.** #243 was that prerequisite lane, it merged, and this base
contains it. The two reasons above are why the work went there rather
than here; they are not live obligations.

**Revision 15 said "it is not input work". That was wrong** — replay
includes terminal mouse reporting and click-to-focus, which is exactly
input work. The reason is ownership and breadth, not category.

##### Ordering — and why emission-only witnesses are not enough

**The defect is precisely "the frontend emits and the receiver
discards".** An emission-only witness asserts the half that already
works and cannot see the half that does not — *it reproduces the blind
spot that let this sit undetected.* Passing panel rows would again mean
nothing about whether a panel wheel scrolls.

**Steps 1 and 2 are DONE. Step 3 is 1b's, undiminished.**

1. ~~The replay lane merges first.~~ **#243 merged.**
2. ~~1b rebases onto it.~~ **Done: 1b's base is `0ec13b3`**, which
   contains #243. §2a's other measurements were unaffected by that
   lane, as predicted — it touched the daemon side.
3. **1b carries an END-TO-END panel-wheel EFFECT witness, on BOTH
   axes**: a wheel over a panel cell moves that panel's viewport
   vertically, and a horizontal wheel moves it horizontally. Not "a
   `PanelPointer` was emitted" — the observable effect.

**Why BOTH axes are still owed, when #243 already supplies the vertical
receiver.** These are different things and revision 20's first draft
conflated them:

- **The receiver path** for a vertical panel wheel exists now
  (`src/editor.rs:3277`–`:3278`). That is implementation, and #243
  discharges it completely within its scope;
- **B1's evidence** is not discharged by any of it, on either axis,
  **because B1's producer is NEW**. 1b introduces a per-surface
  fractional residual accumulator that did not exist when #243 was
  written and that no #243 test exercises. A vertical panel wheel
  routed through a new accumulator can fail in ways the old whole-tick
  path could not — dropped sub-ticks, residue carried across a surface
  switch, a residual keyed to a panel that is gone.

So: **#243 discharges the PREREQUISITE. It discharges none of the
WITNESS.** The horizontal leg additionally has no handler at all, which
is where the emit-and-discard defect lived — but the vertical leg is
owed for its own reason, not as a formality.

**Panel inertness is therefore NOT an option and is not claimed.** The
panel's contract is "emit both axes with its own residual, and the
panel scrolls."

**Without this table the rows are satisfiable by an implementation that
is wrong**: one global accumulator passes every per-surface row that
only ever tests one surface, and a document-and-minimap-only
implementation passes B1 and B6 while panel and terminal traffic still
carries residue across surfaces or drops sub-tick deltas silently.

### CORRECTION 3 — three rows say "nothing exists" where something does

Each of these reads as an empty field in the table and is not one. The
contracts are unaffected; the **implementation shape** is.

- **B3 — "no horizontal scroll to clamp".** The GPU has a horizontal
  origin: **`code_scroll_left`** (`:1681`), a pixel offset *snapped to
  the column grid*, moved by its own `horizontal_follow` (`:8117`).
  What is missing is a **wheel-driven** horizontal scroll. B3 adds a
  second writer to an existing field, which is a different job from
  introducing one.
- **B5 — "no I-beam".** True as stated, but the cursor already has an
  owner: **`apply_panel_cursor_icon`** (`:7743`) sets `RowResize` over
  the divider and **`CursorIcon::Default` everywhere else**. An I-beam
  written as a separate site would be **clobbered by that else branch**.
  B5 must extend this function, not join it.
- **B4 — "no middle-click path".** 1-pre already built the landing
  site and named this row in it: `PointerRoute::UnusedButton` (`:3803`)
  is documented *"Stage 1b's B4 gives the middle button a meaning
  (PRIMARY-selection paste on Linux) and lands here."* `route_pointer`
  (`:3811`) sends every non-left, non-right-press button there. B4
  splits a variant that already exists.

### CORRECTION 4 — B7 re-opens a deferred question, and the table does not say so

This is the finding that needs a ruling rather than a renumber.

The TUI horizontal origin is **`window.view_left`** (`src/window.rs:386`),
and `horizontal_follow` (`src/editor.rs:5233`) already **pins it to 0
under wrap** — so *B7's wrap clause is implemented today*, for the
caret-follow path. Both frontends share the arithmetic
(`pmacs_protocol::scroll::follow_left`, `scroll.rs:134`), deliberately.

But that function's doc states the premise B7 removes:

> there are no explicit scroll commands, so **every viewport move
> originates here**, and Q#HS4's snap-back hazard cannot arise.

**B7 is an explicit horizontal viewport move.** `docs/horizontal-scroll-framing.md`
is explicit that such commands *"are what re-opens Q#HS4"* (`:189`), and
Q#HS4 is recorded as **DEFERRED, not answered** (`:202`). The hazard is
concrete and already cost this project once — §1.5 there quotes
`scroll_window`, which carries the cursor with a **vertical** wheel
scroll for exactly this reason:

> the renderer has an "auto-scroll to keep cursor visible" pass that
> would otherwise snap `view_top` straight back … so the user's
> mouse-wheel scroll would feel stuck after one notch.

(That citation is itself stale: `scroll_window` is **`src/editor.rs:4583`**,
not `:3628`.)

So a wheel-driven `view_left` that does **not** carry the cursor is
snapped back, and horizontal wheel scrolling "feels stuck after one
notch" — the identical bug, one axis over, on **both** frontends, since
the GPU's `horizontal_follow` (`:8117`) has the same shape.

**And it happens on the next PAINT, not the next caret event.**
`prepare_window_cursor_visible` (`src/editor.rs:5256`) calls
`horizontal_follow` unconditionally as its **first** act (`:5277`), and
`paint_frame` (`:5485`) calls it every frame (`:5590`, and `:2863` for
the panel). Revision 13 said "the next caret event"; that was wrong and
understated the exposure — the origin is overwritten by a redraw with
no input at all.

**B7's stated contract does not mention the cursor**, so its mutations
cannot detect this: a clamp row and a wrap row both pass against a
viewport that is overwritten on the next frame.

#### Q#S1-11 — does a horizontal wheel scroll carry the cursor? **RULED: (B), viewport only**

**(A) is not viable in 1b, and the vertical precedent does not reach
it.** `scroll_window` carries point because it is **TUI-side**, where
the editor owns the cursor directly. The GPU has no such power:
`OwnCursor` (type doc `pmacs-gpu/src/main.rs:2448`; the field is
`:1774`) is *"pmacs-gpu's own cursor
position, **mirrored** from `CursorByte`"* — a read-only reflection of
daemon state. The only wire operation that positions it is `Pointer`,
and `dispatch_pointer` (`src/editor.rs:4376`) sets `active_frontend`,
calls `break_command_chain`, and — by its own comment — *"Every
`PointerKind` moves point or changes the selection."* Carrying point
from a wheel would mean **a new wire operation**, which contradicts
1b's non-protocol scope outright.

Note also that **GPU vertical scrolling already does not carry point**:
`apply_wheel` ends at `send_viewport` (`:3309`). (A) would therefore not
even be internally consistent — it would make the horizontal axis carry
point on a frontend where the vertical axis does not.

##### The lifetime contract (B)

A ruling that only says "do not carry" leaves the origin's lifetime
undefined, which is the part that decides whether the feature works.
**All five clauses are the ruling**, not commentary on it:

1. **Horizontal wheel changes the VIEWPORT only** — never point, never
   selection, on either frontend.
2. **An effective wheel move makes that origin authoritative**, and it
   stays authoritative while the cursor position is unchanged. "An
   effective move" means one that actually changed the origin; a move
   fully absorbed by the clamp arms nothing.
3. **Preserved by** repaint, a same-cursor follow, resize, and vertical
   wheel. **Clamped by** geometry and content changes — clamped, not
   released. **A WIDER viewport lowers the maximum origin**
   (`widest − viewport`), so widening re-clamps downward; the gesture is
   preserved at the new bound rather than discarded, and authority
   survives the clamp. (Revision 14 said "a narrower window", which had
   the direction backwards — narrowing *raises* the ceiling and needs no
   clamp at all.)
4. **A genuine cursor-position change releases it**, and normal follow
   resumes on that same event. Release is driven by the cursor
   *changing*, never by elapsed time or by the follow running.
5. **Wrap and buffer replacement clear it and pin the origin to zero.**
   This is the existing rule (`horizontal_follow`'s wrap branch, and
   the GPU's wrap branch at `:8120`; `:8089` is only the
`horizontal_follow` call); authority must not survive either.

##### What B7 and B3 must witness

Rows against the **real call sites**. A unit test of a helper cannot see
a follow that runs inside a frame.

**The two frontends need DIFFERENT preservation drivers, and this is
where revision 14 was wrong.** It said "the next paint" without
qualification. **That is TUI-only.** The GPU's `render()` (`:10296`) goes
straight to `render_to_view` and **never calls `horizontal_follow`**;
the follow reaches it only through `ensure_caret_painted` (`:8071`),
whose callers are cursor paths (`:5897`, `:5949`, `:6066`, `:6610`) and
geometry paths — `resize` (`:10266`), `apply_font_facts` (`:10144`),
`reflow_dynamic_code_geometry` (`:10087`). **A GPU wheel-then-paint row
would stay green with the overwrite mutation restored**, which is a
vacuous witness of exactly the kind this framing keeps producing.

##### The discriminating setup, required by every row

**Revision 15 stated this for L3 alone. It is required by all of
them.** Every row below asserts "the origin is X"; if the cursor sits
*inside* the manually scrolled viewport, `follow_left` returns that
same origin, so **held authority and released authority produce
identical state** and the row passes either way. Visible state
coincides; the assertion proves nothing.

**Setup for L1, L2, L4, L6, L7 and L8, not only L3: the cursor is at a
column OUTSIDE the manual viewport**, so the two outcomes are
distinguishable — authority held keeps the wheel origin, authority
released snaps to the caret's.

| # | witness | driver (cursor outside the manual viewport throughout) |
|---|---|---|
| L1 | preservation, TUI | wheel sideways → **a real paint** (`paint_frame` → `prepare_window_cursor_visible`) |
| L2 | preservation, GPU | wheel sideways → **a HEIGHT-ONLY `resize`** — real follow, horizontal geometry unchanged |
| L3 | release | wheel sideways → a genuine cursor-position change, landing outside the manual viewport |
| L4 | cross-axis, **TUI only** | wheel sideways → wheel **vertically**; vertical wheel carries point in the TUI (`scroll_window`), so a naive authority-on-any-cursor-write releases here. Clause 3 says the origin survives |
| L5 | point and selection unmoved | wheel sideways on both frontends → point and selection byte-identical (clause 1) |
| L6 | clamp-absorbed motion does not arm | at the bound already, wheel further → origin unchanged **and authority NOT armed**, so the next follow moves normally (clause 2's "effective") |
| L7a | re-clamp on **viewport widening** | wheel sideways → **widen** the viewport → origin re-clamped to the new maximum, authority still held (clause 3) |
| L7b | re-clamp on **content shrink** | wheel sideways → **shorten the widest line** so the maximum falls → origin re-clamped, authority still held |
| L8 | wrap and buffer replacement clear the LATCH | wheel sideways → toggle to `Wrap` (and separately, replace the buffer) → origin zero **and authority cleared**, verified by a following `truncate` toggle where the caret rule governs again (clause 5) |

**L2 is height-only for the same reason L3 leaves the viewport.** A
resize that changes width also changes the clamp, so the origin could
move for a reason unrelated to authority and the row would not
discriminate. Height-only invokes the real follow path
(`resize` → `ensure_caret_painted` → `horizontal_follow`) while leaving
horizontal geometry fixed.

**L7 needs both legs.** Clause 3 promises re-clamping on *geometry and
content* changes; widening the viewport witnesses only the geometry
half. **L7b shrinks the content** — the maximum is
`widest − viewport`, so a shortened widest line lowers it with the
viewport untouched.

**L8 is not covered by the existing wrap-origin rows.** Those assert the
origin is zeroed; they cannot see a **stale latch** surviving the wrap,
which surfaces only on the return to `truncate` when the caret rule
should have resumed and does not.

Mutations. Per §6's dependency-aware rule, each must **bite its named
rows**; where one *necessarily* breaks dependents, the dependency is
named rather than treated as a failure of the mutation.

**A dependency belongs in the third column only when it is
UNAVOIDABLE**, and revision 16 put two there that are not. Predicting
collateral damage is not the same as deriving it, and a predicted bite
that does not occur reads afterwards as a witness that failed to fire.

| mutation | must bite | necessary dependents |
|---|---|---|
| follow ignores manual authority (always overwrites) | L1, L2 | L4, L7a, L7b — each asserts a preserved origin, which cannot survive an unconditional overwrite |
| manual authority never releases | **L3 only** | none |
| authority armed by *any* wheel event, effective or not | L6 | none |
| re-clamp releases authority instead of preserving it | L7a, L7b | none |
| wrap/replacement zeroes the origin but leaves the latch set | L8 | none |
| the wheel path writes point or selection | **L5 only** | none |

**The two removed dependencies were assumptions, not derivations.**

- *"Never releases" → L8.* Clause 5 gives wrap and buffer replacement
  their **own explicit clear**, which does not run through the
  cursor-change release path. An implementation with release suppressed
  and clause 5 intact clears the latch on wrap and L8 passes. **Scoped
  to L3.**
- *"Writes point" → L4.* L4 asserts the horizontal origin survives a
  vertical wheel. A horizontal wheel that spuriously writes point does
  not change what the *vertical* path does, so L4 can pass while L5
  fails. **Scoped to L5.**

**Additional bites are recorded only after a mutation run shows
them** — observed, with the run named, never predicted in advance.

Clause 5's *origin* half is already implemented for the caret path; the
existing wrap-guard removal named in the B7 row remains its mutation.
**L8 covers the half that is new — the latch.**

### CORRECTION 6 — B3's right bound is vaguer than B7's, for the same bound

B7 states its upper bound exactly — *"(widest display-line width − text
viewport width), SATURATING AT ZERO"* — and pins that the final display
column stays visible. **B3 says only "content bounds"** and witnesses
just the negative-origin end. That asymmetry is not defensible: it is
the same bound on the same rule, and B7's exactness exists because the
loose version *blanks the viewport*.

**B3 takes B7's rule verbatim, in the GPU's column-grid units.** The
GPU already reckons in that grid — `horizontal_follow` (`:8117`)
derives `cols = (width / advance).floor()` and `left_col =
(code_scroll_left / advance).round()`, then re-multiplies to snap the
offset back onto the grid, deliberately, so both frontends put the same
first character on screen. The clamp is therefore stated in **columns**
and applied to `code_scroll_left` through the same conversion — not in
pixels, which would break the snap the shared rule depends on.

B3's rows, matching B7's:

- **Lower bound:** the origin never goes negative (already named).
- **Upper bound:** clamped at *widest display-line width − text
  viewport width*, **saturating at zero** — a buffer narrower than the
  viewport clamps to 0, not to a negative.
- **Narrow-buffer row:** every line shorter than the viewport → the
  origin stays 0 however far the wheel is pushed.
- **Final-column-visible row:** at the upper bound, the widest line's
  **last display column is still on screen**. This is the row that
  distinguishes the correct bound from the plausible one.

**Mutation: clamp at the full content width.** It must fail the
final-column-visible row — that clamp lets the origin advance past
every glyph and leaves the viewport blank, which is exactly the defect
B7's revision found and B3 currently has no row to catch.

## 3. PR topology

`1-pre` → `1a`\* → **panel-replay lane** → `1b` → `1c` → `1d` → `1e`\*
 (\* `--protocol`)

**1c is NOT protocol-bearing** under Q#S1-8's ruling. Protocol slices
are serialized.

**The panel-replay lane WAS a hard prerequisite of 1b. It is
DISCHARGED.** §2a found, at `72da24a`, that
`dispatch_semantic_panel_pointer` validated and focused but **replayed
nothing**, so a panel wheel was dead on both axes — a pre-existing gap
in **parent acceptance 48** (`docs/bottom-panel-framing.md:1719`), which
owned it.

**#243 repaired it, and it is in this base.** `0ec13b3` contains the
split dispatcher (`src/editor.rs:2985`, `:3088`) and the vertical
panel-wheel replay (`:3277`–`:3278`). **1b has no outstanding
prerequisite**, and the rebase this revision was written on is the one
the order below called for.

**What does NOT lapse is the witness, on EITHER axis.** The reason for
demanding it stands unchanged: 1b's B1 makes the panel a first-class
wheel target **with its own residual**, and **the original defect was
exactly "frontend emits, receiver discards"** — so a 1b that witnessed
emission alone would repeat the blind spot that hid it.

**#243 supplies the vertical RECEIVER; it does not supply B1's
EVIDENCE.** B1's producer — a per-surface fractional accumulator — did
not exist when #243 was written, and no #243 test exercises it. So
**1b owes the end-to-end effect witness on BOTH axes**: the horizontal
leg because it has no handler at all, and the vertical leg because it
now runs through a new accumulator that can drop sub-ticks or carry
residue across a surface switch.

**Order, as it now stands: the prerequisite is met; 1b implements
B1–B7 on this base and lands the both-axis effect witness.** 1b's base
is **`0ec13b3`**, off `72da24a` as the earlier order required; the rest
of §2a is measured on the GPU and TUI sides and is unaffected.

## 4. Q#S1-8 — RULED: (A), preserve pre-window readiness

`AttachRequest.initial_size` for a semantic session is a **named,
provisional `SEMANTIC_BOOTSTRAP_GRID` of 24×80**. It is **not measured
geometry** and **must never become semantic frame or panel authority**.
**`FrontendCellGeometry`, sent after window creation, is the sole real
frame declaration.**

This **codifies current daemon behaviour**, so **1c stays
non-protocol-bearing** — unless implementation changes that behaviour,
which would be a wire-contract change even with no bytes moved.

## 5. Q#S1-9 — RULED: `TextInput` precedence

**A `KeyboardInput` stays `Key` unless a rule below moves it.**

1. **Named keys and control text remain `Key`**, regardless of
   `KeyEvent.text` — so `Enter`'s `"\r"` never becomes text.
2. **Ctrl/Alt chords remain `Key`**, except **printable Ctrl+Alt
   recognized by the existing AltGr rule**.
3. **Meta/Super-only text stays reserved to the OS.**
4. **Plain printable SINGLE-scalar remains `Key`** — preserving mode
   keymaps and today's typed provenance.
5. **Printable MULTI-scalar becomes one `TextInput`.**
6. **Every non-empty `Ime::Commit` becomes one `TextInput`**, even
   single-scalar.
7. **`Key::Dead` is 1d-owned; 1a buffers nothing.**
8. **`Shift` is already reflected in resolved text** and is **not**
   carried on `TextInput`.

**Provenance and chain.** A **single-scalar** `TextInput` rotates to
`buffer.self-insert` and creates **today's one-codepoint typed
provenance**. A **multi-scalar** `TextInput` **breaks the command chain
and creates no typed provenance**. Both are **one edit, one undo unit,
one hook, one eligible CRDT op**.

**Modal precedence is preserved:** terminals take **raw UTF-8**;
search and minibuffer **consume** text; menu and query-replace **retain
their shadow behaviour**; **only the ordinary document path performs the
atomic edit**.

**Payload cap: 64 KiB UTF-8, oversize REJECTED, never truncated.**

## 6. Evidence

**The promise, corrected.** Revision 4 claimed every mutation fails only
its own clause. That is **false and cannot be made true**: clauses have
real dependencies — D1 gates every 1d row, D3's failure surfaces at A6,
and **E0 and E2–E6 all presuppose E1** (no transport, no receiver). The promise is therefore
**dependency-aware, and split by clause kind** — revision 5 still said
"every clause fails today", which C6 falsifies by design:

- **CHANGE clauses** have a witness that **fails today** and a mutation
  that fails **at least** its own clause.
- **PRESERVATION clauses** (marked **[P]**) **pass today**; their
  witness pins behaviour that must not regress, and their mutation is
  the change that would break it.
- Where a mutation necessarily breaks dependents, **the dependency is
  named**.

P3 remains an accepted structural exception: not headlessly testable.

### 1-pre

| # | Contract | Witness (fails today because) | Mutation |
|---|---|---|---|
| P1 | Every handled family routes through an extracted function | no extracted functions exist | misroute one family → that family's row |
| P2 | Harness records outbound events **and local effects** (exit, redraw, resize, state mutation) | no harness | record outbound only → exit/redraw rows |
| P3 | `window_event` is a thin call-through | — | **structural/code-review invariant; not testable headlessly** (no `ActiveEventLoop`) |

**Revision 10 — one finding against the implementation, not the
design.** The 1-pre seam is built and the table above holds, with one
scope correction that could not be seen from the design. *(Revision 10
also argued P2 was satisfied by classification alone. It is not — see
revision 11 at the end of this section.)*

**P1 has a SECOND structural exception, and it is winit's rather than
this seam's.** `KeyEvent` carries a `pub(crate) platform_specific`
field (`winit-0.30.13/src/event.rs:655`), so **no
`WindowEvent::KeyboardInput` can be constructed outside winit** and no
headless test can feed one to the router. P1's mutation — misroute a
family, fail that family's row — is therefore unavailable for the
**keyboard** family alone.

Three things bound it, so it is a measured exception rather than a
blanket one:

- **The exception does not extend to the pointer families.** Winit
  provides `DeviceId::dummy()` for exactly this purpose, and
  `CursorMoved` / `MouseInput` / `MouseWheel` are constructible. Checked
  before the exception was written down; all three are witnessed.
- **What stays unwitnessed is one pattern arm with no logic in it.** The
  family's only decision — a press is acted on, a release is claimed and
  discarded — is factored into `route_key_action(ElementState) ->
  KeyAction`, which takes a constructible argument and is witnessed
  directly, with both misroute mutations failing that row alone.
- **P3 is now measured, not assumed.** Replacing `window_event`'s entire
  body with `let _ = (event_loop, event);` — a GUI that responds to no
  input at all — leaves **every `pmacs-gpu` test green**. That is the
  exception's true extent: no headless test anywhere in the crate
  observes the delegation. **Re-measured at the current shape after
  revision 11: 265/265 under `PMACS_REQUIRE_GPU=1`** (it was 256 before
  the effect rows existed, and the number is re-run rather than carried
  forward). Revision 11 also shrinks what the exception *covers*:
  `window_event` is four lines, so the unwitnessed residue is one `if`
  rather than a 33-line match.

**Revision 11 — P2 IS IMPLEMENTED AS WRITTEN. Revision 10's argument
here was wrong and is retracted.**

Revision 10 claimed a route-classification transcript covered both
halves of P2 because a route "names its local effect". **The wheel
falsifies that.** A wheel route carries a delta; whether that delta
becomes a viewport update, a panel event, a terminal event or nothing at
all depends on `State`. The route names the *family*, and only running
the body names the *effect* — so classification could not have
satisfied P2, and arguing that it did was a narrowing wearing the
costume of a mechanism.

P2 now has a second harness beside the routing one:

- **`EffectHarness` drives production end to end** — a real
  `AttachClient` over a `socketpair` (real handshake, outbox, writer
  thread, encoder, so the transcript is the wire), a real windowless
  `State`, and `App::dispatch_window_event`.
- **`App::dispatch_window_event` is what made this reachable.** Left
  inside `window_event`, the dispatch would force a harness to
  re-implement it, and a harness that re-implements what it tests
  witnesses its own copy. **P3 therefore narrows from a 33-line match to
  a single `if`**: `window_event` is now `call dispatch, exit if it
  asks`.
- **Local effects are read where they land**: exit from the returned
  `EventOutcome`, redraw from a test-only `render_calls`, resize from
  the surface config, modifiers from `App`, scroll from `scroll_top`.
- **Steps are delimited by a non-coalesceable sentinel key, not a
  sleep** — "this step sent nothing" is otherwise undecidable without
  waiting, and a fixed-duration wait against a writer thread is the
  core-count assumption of PR #235's CI red.
- **The rows never skip.** A missing wgpu adapter is an assertion
  failure; mutation M21 confirms all nine effect rows fail loudly while
  the thirteen GPU-free routing rows stay green.

`M22` (blind to outbound) and `M23` (blind to local) fail rows in both
directions, which is P2's contract executable rather than asserted.

The routing rows stay, and the division of labour is deliberate: the
routing harness answers *where did this event go*, the effect harness
answers *what did it do*. **P2 is owned by the effect rows.** The
routing transcript row is only the sole owner of the *routing*
harness's own recording, which is what keeps that one mutation
surgical — revision 10 claimed it owned P2, and it never did.

**One further correction revision 11 carries, and it is the design
consequence 1a inherits.** Revision 10 stated that Stage 1a's A4 would
leave `EventOutcome` with a single variant and that the type should go
with the Escape branch. **Both are wrong.**

`EventOutcome` has **two producers today**: `LifecycleRoute::Exit`, a
native window close that must always exit, and `apply_keyboard`'s idle
Escape, a local quit. **A4 removes the keyboard one**, leaving **exactly
one `Exit` producer** — the native close.

**One producer is not one variant.** The type survives because
`dispatch_window_event` still has to distinguish `Continue` from
`Exit` on every event it handles: the overwhelming majority of
dispatches must *not* exit, and the native close must. What A4 actually
removes is `apply_keyboard`'s need to return an outcome at all, which is
a change to that one signature rather than to this type.

The crate has **exactly one** executable `event_loop.exit()`, in
`window_event`.

### 1a — `TextInput` (v24)

| # | Contract | Witness (fails today because) | Mutation |
|---|---|---|---|
| A1 | `F1`–`F35` → `F(1..=35)` | `_ => return None` | map `F13+` → `None` → F13–F35 rows |
| A2 | Shift+Tab → `BackTab` with `Shift` set | produces `Tab` | drop `Shift` → A2 only |
| A3 | `ContextMenu` → `Menu` | produces nothing | map to `Char('\0')` → A3 only |
| A4 | Idle Escape reaches the daemon, never exits | exits — `apply_keyboard` (`main.rs:3219`) returns `EventOutcome::Exit`, performed at `main.rs:4452` | restore the quit branch → A4 only |
| A5 | Precedence per §5 (1–8) | multi-scalar truncated; IME ignored | move rule 1 (control text → text) → the `Enter`-in-dired row |
| A6 | One commit = one edit, undo unit, hook, eligible CRDT op | commit truncated to one scalar | one edit per scalar → undo-unit row (**and D3 surfaces here**) |
| A7 | Prompts consume scalars **in order** | multi-scalar never arrives | reverse order → A7's prompt transcript |
| A8 | Terminals get **raw UTF-8, never bracketed paste** | multi-scalar never arrives | route via `Paste` → terminal row shows bracket markers |
| A9 | **64 KiB cap; oversize rejected, not truncated** | no cap exists | truncate instead → the oversize row observes silent loss |

### 1b — pointer and scroll

> **Read §2a first, and note that it CHANGES two of these rows.** §2a
> is measured at **`0ec13b3`**; the line numbers it replaced were
> `72da24a`-era. Every
> line number below was measured before 1-pre and is stale. §2a
> re-measures them — at `72da24a` when this note was written, and at
> `0ec13b3` since revision 20; records three rows whose "nothing
> exists yet" is wrong (B3, B4, B5); **replaces B1's undefined
> "surface" with a normative six-target enumeration**; **replaces B3's
> "content bounds" with B7's exact saturated upper bound**; and rules
> **Q#S1-11 (B, viewport only)** with a five-clause lifetime contract
> and eight witnesses. B2, B4, B5, B6 and B7 are unchanged. **None of
> B1, B3 or B7 is implementable from this table alone.**

| # | Contract | Witness | Mutation |
|---|---|---|---|
| B1 | Residual per **axis and surface** — primary document, panel document and minimap are distinct owners | deltas discarded | share one accumulator → primary↔panel or document↔minimap surface-switch jump |
| B2 | Wheel-right raises the leftmost column of the **document surface under the pointer**; wheel-down raises its top line. Primary-document horizontal state stays frontend-local; a panel-document tick reaches the daemon's window-targeted `view_left` path | `x` discarded; panel replay currently claims and drops document `ScrollLeft`/`ScrollRight` | invert a sign → that axis's row; omit the panel effect → primary moves while the panel remains inert |
| B3 | Each document surface clamps at its own content bounds; never a negative origin, and wrap pins horizontal origin to zero | no horizontal scroll to clamp | remove a clamp → **at-bounds row: that surface's origin goes negative and the view blanks**; reuse primary bounds for a panel → unequal-width surface row fails |
| B4 | Middle-click paste uses **PRIMARY on Linux** | no middle-click path | use `CLIPBOARD` → B4 only |
| B5 | I-beam over text content only | no I-beam | extend over the gutter → B5 only |
| B6 | Wheel over the minimap scrolls the **document viewport** with **its own residual accumulator**; click/drag remains scrub | **a FULL tick already scrolls today** — minimap pixels are `Elsewhere` (`main.rs:2061`) and the wheel falls through to `scroll_by_lines` (`main.rs:3373`). What fails is **fractional accumulation**, and **residual ownership distinct from the document's**: sub-tick minimap deltas are discarded, and a **surface-switch fractional witness** (part-tick over the minimap, then over the document) must not carry residue across | share the document's accumulator → the surface-switch fractional row jumps |
| B7 | **TUI horizontal**: **three columns per wheel tick**, sign per B2; **left origin clamps at 0 and at (widest display-line width − text viewport width), SATURATING AT ZERO**; **wrap pins the origin to 0** | events arrive at `:3203` and are dropped | step of one → the three-column row; clamp at the widest line's **full width** → **the right-bound row blanks the viewport**; drop the wrap guard → the wrap row scrolls a wrapped buffer sideways |

**B7's right bound corrected.** Clamping at the widest line's full width
lets the origin pass every glyph and leave the viewport **entirely
blank**. The bound is *width − viewport*, saturating at zero for buffers
narrower than the viewport, and **the right-bound witness asserts the
final display column is still visible**.

**Panel-replay consequence, ruled after §5a's 2026-08-20
re-measurement.** “Horizontal panel wheel” has three receiver outcomes,
not one. When terminal precedence selects child reporting it already
forwards `ScrollLeft` and `ScrollRight` as SGR codes 66/67. The local
terminal branch — selected by Shift, reporting-off or a scrolled-back
view — has no horizontal viewport and remains inert. **B1–B3 own the missing
document-panel leg**: fractional production/residuals in the GPU, the
lossless `PanelPointer` tick, and the daemon's window-targeted
`view_left` effect and clamp. This is a named deferral from panel replay,
whose source comment already assigned it here; it does not make the
prerequisite lane implement half of 1b early.

**Why B6 changed — and revision 5's reason was wrong.** Scrubbing on
wheel is not *impossible*: the wheel handler already reads the cached
`state.pointer_pos` for surface routing (`main.rs:3337`), so an absolute
target is available. It is the **wrong semantics**: a wheel is a
**relative** gesture, and mapping relative ticks onto an absolute
position would make one notch jump to wherever the pointer happens to
rest. Click and drag remain scrub because those *are* absolute.

### 1c — session and window signals

| # | Contract | Witness | Mutation |
|---|---|---|---|
| C1 | Title `"<buffer> — pmacs"`, `"pmacs"` when unnamed | title is static | drop the name → C1 only |
| C2 | **Visible bell: 120 ms, WHOLE CLIENT AREA visibly changes**; repeats **neither queue nor extend** the first deadline | `Bell` unconsumed | let repeats extend → the repeat row's flash outlasts 120 ms; flash a sub-region → the headless render witness cannot see it |
| C3 | `Goodbye` names the daemon's reason, else an explicitly **locally-classified** transport/EOF reason; never blank | live-loop reason discarded | blank the fallback → EOF row |
| C4 | `FocusLost` precedes `Detach`; `FocusGained` only after attach completes | `Focused` unhandled | swap → C4 only |
| C5a | **Local DPI correctness**: at scale 1→2 with **unchanged logical size**, logical wrapping, row count and hit testing are **stable** while **physical pixels double** — glyphs, clips, caret, hit tests and overlays all rescale | `scale: 1.0` hardcoded (`main.rs:8950`) | rescale glyphs only → **caret, clip and hit-test rows fail while text still looks right**, which is the bug this splits out |
| C5b | **Geometry declaration**: the epoch advances and `FrontendCellGeometry` is emitted | no `ScaleFactorChanged` arm | suppress the emit → C5b only, **C5a still passing** |
| C6 **[P]** | `initial_size` = `SEMANTIC_BOOTSTRAP_GRID` (24×80), **never** frame or panel authority | **passes today** — this codifies current daemon behaviour; the witness pins that a semantic frame/panel is sized from `FrontendCellGeometry` alone | derive a frame or panel extent from `initial_size` → the panel-authority row |
| C7 | Close contract — §7 | see §7 | see §7 |

**C5 was one row and hid half the defect** — suppressing the wire emit
says nothing about glyphs or hit tests staying at scale 1.

### 1d — IME

| # | Contract | Witness | Mutation |
|---|---|---|---|
| D1 | `set_ime_allowed(true)` | zero occurrences — **no composition arrives at all** | omit → **every 1d row (stated dependency, not a matrix defect)** |
| D2 | Preedit overlay with caret and selection; **indices are BYTE OFFSETS** into the preedit string | no overlay | treat as char indices → multibyte row |
| D3 | `Ime::Commit` emits A5's `TextInput` | commit ignored | emit per-scalar `Key`s → **A6's undo row (named dependency)** |
| D4 | **`set_ime_cursor_area` updated** after caret motion, scroll, resize, font/DPI change, and every preedit change | never called → candidates misplaced | update on caret only → scroll and resize rows |
| D5 | Overlay clears on **empty `Preedit`, `Ime::Disabled`, and focus loss** — all three | no overlay | clear on focus loss only → `Disabled` row leaves stale text |
| D6 | Dead-key state owned here; 1a buffers nothing | dead keys dropped | buffer in 1a → D6 by construction |

### 1e — `OpenTarget` (v26)

| # | Contract | Witness (fails today because) | Mutation |
|---|---|---|---|
| E0 | **SUCCESS**: a **successfully handled valid target** is **resolved, installed, hooked and dispatched through the existing file/directory pipeline**, and the originating frontend receives **exactly one terminal result** — `Opened { request_id, buffer_id }` when a commit lands, **or `Handled` when an extension legitimately claims it** | no receiver — a drop does nothing | dispatch without firing the open hooks → the hook row; settle before the deferred commit lands → the async-directory row (Q#S1-10) |
| E1 | Versioned `OpenTarget { request_id: u64, cwd, path }` carrying `InitialTarget`'s raw shape. **`request_id` is unique among a frontend's OUTSTANDING requests; a duplicate is REJECTED at the protocol boundary** and must never replace or settle the original completion | no variant exists | **omit `request_id`** → the two-concurrent-drops row misattributes; **accept a duplicate id** → the reuse row settles the first drop's completion with the second drop's outcome |
| E2 | Source is **authenticated** | no receiver | accept an unauthenticated sender → E2's forged-source row |
| E3 | Primary document `ViewDestination` captured **immediately on receipt**, before any await | no receiver | capture after the open resolves → **frontend-switch row: the file lands in whichever frontend is ambient** (the #231 defect) |
| E4 | **No window identity trusted from the wire** | no receiver | accept a window id → E4's forged-window row |
| E5 | Failures **before terminal disposition** — including completion-aware `Deferred` work — are **visible to the ORIGINATING frontend** as bounded `Failed { request_id, message }`. After `Handled`, responsibility and any later failure belong to the claiming extension, and no second result is sent — see §8 | no receiver | swallow the error → the permission and embedded-NUL rows |
| E6 | `InitialTarget` limits enforced: **32 KiB per raw path**, **non-empty path**, **absolute non-empty cwd**, **embedded NUL rejected** | no receiver | drop the NUL check → the embedded-NUL row |

**The failure taxonomy was wrong in revision 5.** **A missing path is
NOT a failure**: the resolver deliberately creates an **empty
path-backed buffer** on `NotFound` (`src/editor_core.rs:1177`), which is
how "open a file that does not exist yet" works. **Directories are valid
targets too.** Genuine failures are permission denial, validation
rejection (E6), and open errors that are not `NotFound`. Revision 5
listed "missing-path" and "not-a-directory" rows that would have pinned
the opposite of the intended behaviour.

### Q#S1-10 — RULED: the result is TERMINAL

Directory opening completes **asynchronously**, so `OpenTargetResult`
either waits for the captured-destination commit or merely acknowledges
dispatch. **Ruling: terminal.** The result is sent **when the request
reaches a terminal disposition** — for `Opened`, after the commit
resolves; for `Handled`, at the moment responsibility transfers; for
`Failed`, at the failure, **which for several paths has no commit at
all**. Asynchronous failure is reported through the same result — an acknowledgement-plus-later-channel design would need a
second source-scoped mechanism to carry exactly the failures that matter
most.

```
OpenTargetResult::Opened  { request_id: u64, buffer_id: BufferId }
OpenTargetResult::Handled { request_id: u64 }                  // claimed; no buffer attributable
OpenTargetResult::Failed  { request_id: u64, message: String }  // 4 KiB cap
```

**Terminal completion must be TOTAL over the existing pipeline, and
"after the commit resolves" is not** — three legitimate paths reach
neither a commit nor a result:

- **Claimed**: a `path.open-directory` listener returns `proceed =
  false` and the dispatch **returns without committing**
  (`src/editor.rs:1375`).
- **Disabled**: the `directory_handler` slot is clear, which is a
  supported configuration — the dispatch emits a **status message and no
  commit** (`src/editor.rs:1389`).
- **Asynchronous fallback**: the default handler calls `open_async` and
  **returns immediately** (`builtin/runtime/dired.lua:750`), so the
  commit lands long after the dispatch unwinds.

**Mechanism: a request-scoped ONE-SHOT COMPLETION with an EXPLICIT
STATE MACHINE**, carried through the directory pipeline, settled
**exactly once**, and **discarded on source detach** — a frontend that
left cannot be told anything.

```
Pending ──(defer before scheduling)──▶ Deferred ──▶ Settled
   │                                    │
   └──────────(commit / failure)────────┴──▶ Settled
   │
   └──(source detach, from Pending or Deferred)──▶ Cancelled
```

**`open_async` transitions to `Deferred` BEFORE handing the completion
to scheduled work.** The ownership transfer is explicit and does not
depend on whether today's scheduler starts a coroutine synchronously or
a future scheduler defers its first step. Without the transition the
dispatch unwinds while the completion is still `Pending`, the
end-of-turn fallback fires, and the request is settled *before* the
commit it was waiting for.

**The end-of-turn fallback acts ONLY on `Pending`.** A `Deferred`
completion is owned by the scheduled work. **Any later settlement is
exactly-once**: a second attempt against `Settled` or `Cancelled` is a
no-op, never a second message.

*Mutation:* omit the `Deferred` transition, or delay it until after the
end-of-turn fallback → the async-directory row receives a premature
`Handled`/`Failed`; exactly-once settlement then suppresses the later
`Opened` attempt when the commit lands.

`open_async` carrying the completion to the commit is what makes the
asynchronous path terminal rather than silent.

**Total disposition, so no path can fall through:**

| path | settles as |
|---|---|
| commit lands | `Opened { buffer_id }` |
| listener **claimed** and did not settle | `Handled` |
| **replacement handler** ran and did not settle | `Handled` |
| handler slot **disabled** | `Failed` naming that directory opening is disabled |
| **synchronous error** (listener raised, validation, permission) | `Failed` with the reason |
| pipeline unwinds **still `Pending`** at end of dispatch turn | `Handled` if a listener claimed or a replacement handler ran; **otherwise `Failed`** |
| **source detached** before settlement | discarded — nothing is sent, and the completion is dropped rather than leaked |

**`Handled` exists because `Opened` cannot be honest there.** A claim
means a user listener took responsibility and **no buffer is
attributable**; reporting `Opened` would require inventing a
`buffer_id`, and reporting `Failed` would mislabel a supported
extension point as an error. `Handled` is the terminal responsibility
transfer: any later extension-owned failure uses the extension's own
reporting surface and cannot emit a second `OpenTargetResult`.

*Witness:* one case per **live-source** row, each asserting **exactly
one** result. **The detach row asserts the opposite and must not be
read as "one result":** zero messages sent, **no completion retained**,
and a later settlement attempt **ignored** rather than delivered.
*Mutation:* drop the end-of-turn fallback → the claimed and disabled
rows hang with no result at all, which is the defect this ruling
closes.

`message` uses the **existing 4 KiB error cap**. **Both affected enums
get an independent frozen-byte pin on their own preceding final
variant** — `FrontendEvent` for `OpenTarget`, `InstanceMessage` for
`OpenTargetResult` — because an appended variant's own round-trip cannot
detect a discriminant shift in either.

## 7. The close contract

`send_event` only enqueues (`attach.rs:1145`); the writer takes a batch
and releases the lock before blocking writes (`:671`); **`enqueue`
returns `false` once closed** (`:414`).

**So revision 4's order was literally unexecutable** — closing first
would have rejected the `Detach` it then tried to enqueue.

**Contract — state inspection plus any append/transition is ONE atomic
critical section under a single lock hold, dispatched on the outbox
STATE. The lock is released before notifying the writer, waiting on an
acknowledgement, returning, or shutting down.** Holding it while waiting
would prevent the writer from taking the suffix or transitioning to
`Drained`. The four states are tabulated below; this paragraph is their
normative statement, and revision 6's "append `Detach` /
already-closed → fallback" wording is superseded:

- **Open** → under the lock, append the **complete suffix** (`FocusLost`
  when currently focused, then `Detach`), transition to `Sealed`, and
  capture its acknowledgement handle; release the lock, wake the writer,
  then wait.
- **Sealed** → under the lock, capture the **existing acknowledgement**;
  release the lock, then join it. Do not re-append, do not seal again,
  do not shut down.
- **Drained** → release the lock, then return immediately.
- **Failed** → release the lock, then take the socket-shutdown fallback.

**An ordinary post-seal `send_event` REJECTS and does nothing else** —
it must not invoke shutdown, because a healthy drain is in flight.
**`Detach` is exempt from coalescing.** **Wake the writer after
append-and-seal**, or
a sealed outbox with a pending `Detach` waits on a condvar nobody
signals and the 250 ms bound expires on a daemon that was reading fine.

**The exactly-full case, which revision 5 left undefined.**
`OUTBOX_MAX` is **8192**, and a queue at exactly that length is a
**valid OPEN state**: the *next* ordinary `enqueue` both **sets
`closed`** and **rejects the event** (`attach.rs:427`). So terminal
close must not go through the ordinary path. **Ruling: the terminal operation appends the whole required SUFFIX
atomically** — **`FocusLost` when currently focused, then `Detach`** —
reserving **at most `OUTBOX_MAX + 2`**.

**One slot was not enough, and C4 is why.** C4 requires `FocusLost`
before `Detach`; at exactly `OUTBOX_MAX` an *ordinary* `FocusLost`
enqueue is **rejected and sets `closed`**, so the terminal append would
then find a closed outbox and fall back — losing both events. Revision 6
reserved one slot and so contradicted a contract two sections above it.

**Exact-cap witness:** fill to exactly `OUTBOX_MAX` **while focused**,
then close. The transcript ends **`… FocusLost, Detach`**, the writer is
woken, and acknowledgement precedes exit. *Mutation:* reserve one slot →
`FocusLost` is dropped and C4's ordering row fails at exact cap only.

**Outbox states — clean SEALED is not failed CLOSED.** Revision 6 had
one `closed` flag doing both jobs, so a duplicate close or a post-seal
send would see "closed" and **invoke the socket-shutdown fallback,
aborting the drain it should have joined**. Four states, with distinct
behaviour:

| state | ordinary enqueue | close called again |
|---|---|---|
| **Open** | ordinary policy: accepted below the cap; a cap-crossing lossless append is rejected and transitions to `Failed` | performs the terminal append-and-seal |
| **Sealed** (suffix appended, drain in flight) | rejected | **waits on the existing acknowledgement** — never re-seals, never falls back |
| **Drained** (acknowledged) | rejected | returns immediately |
| **Failed** (overflow, or transport error) | rejected | **takes the socket-shutdown fallback** |

*Mutation:* collapse `Sealed` into `Failed` → the duplicate-close witness
aborts a healthy drain and exits before `Detach` is written.

**Acknowledgement point:** the batch containing `Detach` **fully written
and flushed to the socket**. **Bound: 250 ms.**

**Two witnesses, mutually exclusive:**

- **Responsive reader** — all preceding lossless events **and** `Detach`
  are written, acknowledgement occurs, **then** exit. *Mutation:* drop
  the drain → exit-before-`Detach` is observable.
- **Stalled reader** — the deadline fires, socket shutdown yields EOF
  cleanup, the frontend **exits anyway**. *Mutation:* remove the bound
  → this test hangs.

*Seal mutation:* permit a late enqueue → an event appears after
`Detach` in the responsive transcript.

## 8. Wire contracts

| | **1a — `TextInput`** | **1e — `OpenTarget` + `OpenTargetResult`** |
|---|---|---|
| **floor** | **v24** | **v26**, after v25's mapping generation, serialized |
| **encoding** | **appended variant**; never widen a field in place — postcard is positional | appended variants |
| **byte pin** | frozen-byte fixture on the **previous final variant** | same |
| **gate** | daemon accepts from `>= 24`; producer withholds below | **`>= 26`**; producer withholds below |
| **old peer** | a `< 24` frontend **retains its existing `Key` behaviour and its existing limitations** — it truncates multi-scalar input today and ignores IME, and continues to. **The guarantee is NO REGRESSION, not retroactive correctness** | a **`< 26`** frontend cannot drop-open; nothing it already had degrades |
| **bounds** | **64 KiB** UTF-8; oversize **rejected** | **32 KiB** per raw path; non-empty path; absolute non-empty cwd; **embedded NUL rejected**; `Failed.message` capped at the **existing 4 KiB** error cap |
| **pins** | frozen bytes on `FrontendEvent`'s previous final variant | **two independent pins** — `FrontendEvent` for `OpenTarget`, `InstanceMessage` for `OpenTargetResult` |

**E5's delivery mechanism.** `StatusFacts.message` is **global and can
be cleared before the originating frontend observes it**, so it cannot
carry this. **1e adds a source-scoped `OpenTargetResult`, correlated by
a frontend request ID**, so a failure reaches the frontend that dropped
the file and no other.

## 9. Coherence impact (§20)

- **Journey steps**: **5**; **3** (1e); **6(e)** on the GPU column.
- **Islands**: Escape ceasing to be a local quit **removes** one;
  Q#S1-7 adds none → the census **falls by one**.
- **Config registry**: none. The bell's 120 ms is a constant.
- **Background work**: **1e adds no new worker**, but it **attributes
  the existing asynchronous directory operation to an originating
  frontend and request** until terminal settlement — the one-shot
  completion is that attribution — **and drops it on source detach**.
  That is §9's ownership question appearing in miniature, and it is
  answered here for this one operation rather than in general.

## 10. Rulings

**Q#S1-1** native close detaches, `editor.quit` shuts down the daemon
and its attachments, Escape only cancels/round-trips · **Q#S1-5** A/`1e`
· **Q#S1-6** B/`TextInput` · **Q#S1-7** Meta/Super → Stage 2, arc §2.5
and the backlog amended by 1-pre's first PR · **Q#S1-8** (A),
`SEMANTIC_BOOTSTRAP_GRID` · **Q#S1-9** precedence per §5 · **Q#S1-10** terminal `OpenTargetResult`.

**Q#S1-11 — RULED: (B), viewport only** (§2a). A horizontal wheel
scroll never moves point or selection; the origin it sets is
authoritative under a **five-clause lifetime contract**, and B7 and B3
carry preservation, release and cross-axis witnesses at the real call
sites. (A) — carrying point, as `scroll_window` does vertically — is
**not viable in 1b**: the GPU cursor is a mirror of daemon state, the
only wire operation that positions it also breaks the command chain and
changes selection, and a new one would break 1b's non-protocol scope.
This settles `docs/horizontal-scroll-framing.md`'s **Q#HS4** for the
wheel case, which was deferred against exactly this arrival.

## 11. Gates

`./scripts/gate --acceptance gpu_invocation_acceptance` plus touched
input suites, and `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`.
**`--protocol` for 1a and 1e only.**
