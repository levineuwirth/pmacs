# Honoring `full_grid` — QoL Stage 1

**Status: revision 2 — APPROVED, IMPLEMENTED, in review as PR #219.**
Q#FG1 was decided **A**: the sole grid consumer honors the flag by
resetting style, clearing, then applying spans. Reported from
daily-driver use:
zooming a terminal with `Ctrl +/-` leaves the TUI visibly broken —
stale glyphs where content should be blank, and dead regions where
content should be.

This is Stage 1 of three. Stage 2 is GUI zoom; Stage 3 is long-line
wrap/scroll. **They are deliberately separate branches** — this one is
a contained correctness fix with a mechanism already established, and
Stage 3 is a design round that touches `view_top`'s missing sibling
everywhere. Bundling them would hold a daily-driver fix behind a
design.

**Revision 2** folds in five review corrections: the invariant belongs
on the protocol type and updating it is in scope (§1.1a, FG-INV); the
PTY witness must be suffix-scoped or the startup clear satisfies it
against a broken build (§5.2); the unit witnesses must include a
`full_grid` message with **empty spans** (§5.1); the standalone TUI
reaches `present_messages` directly and the earlier draft cited only
the attach route (§3); and the claim that the 19 frontend tests assert
on state rather than output was **wrong** — most assert on output,
which makes the proposed seam a continuation of the house idiom rather
than a new one (§5). §6 now cites the concern it serves, not only the
§20 checklist.

---

## 1. The mechanism, established rather than suspected

`src/instance_render.rs:137` builds the full-grid resync by diffing
against a **blank** grid:

```rust
let spans = if self.needs_full_grid {
    let blank = vec![Cell::default(); self.next.len()];
    diff(&blank, &self.next, self.size.cols, self.size)
} else {
    diff(&self.prev, &self.next, self.size.cols, self.size)
};
```

A cell that *should be blank* is equal to its blank counterpart, so it
**produces no span**. The resync therefore carries only non-default
cells — which is exactly what its comment says it intends.

`src/frontend.rs:326` then receives:

```rust
InstanceMessage::CellDelta { spans, .. } => {
    for span in spans { emit_span(&mut self.out, span)?; }
}
```

**`full_grid` is destructured away and discarded.** Nothing clears.

So after a font zoom: the terminal reflows its own content, pmacs
repaints only its non-blank cells, and every position pmacs considers
blank keeps whatever the terminal left there. That is both reported
symptoms from one cause — leftovers where pmacs is blank, dead regions
where the terminal's own reflow already lost content.

### 1.1 Why this survived a green suite

The flag is **documented** (`pmacs-protocol/src/message.rs:651`: "the
initial sync sent on fresh attach *or after a resize where the previous
grid is no longer applicable*") and **tested** —
`first_frame_is_full_grid_sync`, `resize_reallocates_and_flags_full_grid`,
`instance_message_cell_delta_carries_full_grid_flag`, and four more.

Every one of them asserts the **producer sets the flag**. Not one
asserts a **consumer acts on it**. A workspace-wide search finds no
runtime reader at all: the only other `CellDelta` match outside the
producer and its tests is `pmacs-gpu/src/main.rs:10486`, which uses it
to build a debug label.

### 1.1a The rule exists — in the wrong place

The binding invariant is already written down, in the doc comment of a
**private field** (`src/instance_render.rs:36`):

> *"Remote frontends use this flag to know they must blank their local
> buffer before applying the deltas."*

That is the real contract, and it is stronger than what the protocol
says. `pmacs-protocol/src/message.rs:651` describes `full_grid` only as
*"the initial sync sent on fresh attach … versus an incremental
frame"* — a **label**, from which no consumer obligation follows. A
consumer author reads the protocol type, not a private field on the
producer's struct.

**So updating that protocol documentation is in scope**, and the
invariant is stated here as the thing the code must satisfy:

> **FG-INV.** A `CellDelta` with `full_grid: true` carries **only
> non-default cells**. It is not a picture of the screen; it is a
> picture of the screen's *ink*. A consumer MUST blank its surface
> before applying those spans, or every cell the resync considers blank
> retains whatever was there before.

Writing it on the protocol type is what stops the drift recurring: the
producer's field comment cannot reach whoever writes the next frontend.

This is the handoff §5 lesson landed in #218 — *enforcement and
documentation drift apart silently; only the enforcement is real* —
recurring in a different register. Worth stating plainly in the lane,
because "add a test for the flag" was already done and did not help.

### 1.2 Why it is specifically *zoom*

`src/frontend.rs:166` clears **once**, at startup:

```rust
queue!(me.out, EnterAlternateScreen, Clear(ClearType::All), cursor::Hide, …)
```

So "diff against blank" is correct exactly once — on the fresh-attach
frame, where the screen provably *is* blank — and wrong for every
resize after, where nothing has cleared. The original design is not
careless; it is correct for the case it was written for and was never
extended to the second case its own doc comment names.

A font-size change is the worst version because the terminal reflows
content *in place* rather than dropping it, maximizing the stale glyphs
that survive.

---

## 2. What was eliminated, so the lane does not re-investigate

Three plausible causes were checked and are **not** it:

| hypothesis | verdict | evidence |
|---|---|---|
| Panel/frame geometry goes stale | **no** | `sync_frame_geometry` runs per frame inside `paint_frame` (`src/editor.rs:4412`) against the live `term_size` |
| Render buffers are not reallocated | **no** | `RenderState::resize` reallocates `prev`/`next` and sets `needs_full_grid` |
| `view_top` is not reconciled on shrink | **no** | probed directly: cursor at line 150, painting at 40 rows gives `view_top=113` / cursor row 37; at 8 rows gives `view_top=145` / cursor row 5. Correct both times |

The probe was scratch and is not kept — it proved a negative, and a
test that asserts working behaviour nobody is changing is upkeep
without a customer.

---

## 3. Q#FG1 — consumer honors the flag, or producer emits everything?

**Two designs, and the count of consumers decides it.**

- **A — the consumer clears when `full_grid` is true**, then applies
  spans. Protocol meaning: *the resync assumes a blank surface, and the
  flag is the instruction to produce one.*
- **B — the producer emits every cell** on a resync, blanks included,
  so no consumer needs to change.

The instinct was that A is the trap this project has already hit twice
— the same predicate living in `wait_for_file`, then
`wait_for_published_file`, then a third copy in
`bottom_panel_stage1_acceptance` (R4, R6). Fixing N consumers
independently is how that happens.

**But there is only one consumer.** Both TUI paths land on
`Frontend::present_messages`: the standalone run loop calls it
**directly** (`src/editor.rs:3911`), and the attached TUI reaches the
same method through its own `present_messages` impl
(`src/attach.rs:397`). One implementation, two routes — the earlier
draft cited only the second, which understated how direct the
standalone path is. The GPU frontend is a
**semantic** frontend and never consumes these spans at all — which is
independently consistent with the report that the GUI does not
mis-render on zoom, it simply ignores zoom.

**Recommendation: A.** With one consumer the trap does not apply, and A
is the better answer on the merits:

- The flag **exists solely to be acted on**. Under B it stays unread —
  and a flag nobody reads is the defect, not the fix. B would leave
  the next reader with the same puzzle plus more bandwidth.
- B ships a full grid of mostly-blank spans over the daemon socket on
  every resize, to avoid a one-line clear.
- A makes the protocol's documented sentence true, rather than working
  around it.

**Q#FG1 — DECIDED: A**, on approval. The consumer honors the flag;
`full_grid` keeps its sparse-resync meaning and the producer is
unchanged.

*Implementation note, recorded because it confirmed the reasoning
rather than merely following it:* the fix is `emit_cell_delta` beside
`emit_span` and `emit_status_overlay` — no struct change, no generic
parameter, no new pattern. B would have touched the producer, changed
what every resync costs on the wire, and left the flag unread.

---

## 4. Q#FG2 — what "clear" must include

Not just `Clear(ClearType::All)`. The clear paints with the *current*
background, so it must be preceded by `ResetColor` and
`SetAttribute(Attribute::Reset)`, or a resync taken while a styled span
was last emitted will wash the screen in that style.

`present_messages` already brackets its output in
`BeginSynchronizedUpdate` / `EndSynchronizedUpdate` (`src/frontend.rs:223`,
`483`, `502`), so clear-then-repaint lands atomically and does not
flicker. **No new synchronization is needed** — this is why the fix is
small.

---

## 5. Verification

`out` is a concrete `BufWriter<Stdout>` (`src/frontend.rs:126`), so
`Frontend::apply_message` cannot be observed directly. **The seam is
already the house idiom in this file**, though — `emit_span` is a free
function over a writer, and the tests around
`src/frontend.rs:870`–`1037` capture into `Vec<u8>` and assert on the
exact escape sequences (`src/frontend.rs:587` says so in as many
words: *"Tested below by capturing into a `Vec<u8>`"*).

*An earlier draft said those 19 tests "assert on state rather than
output". That was wrong — most of them assert on output, which makes
the seam a continuation rather than an introduction.*

So: **add `emit_cell_delta` beside the existing helpers** and route
`apply_message` through it. No struct change, no generic parameter, no
new pattern.

### 5.1 Unit witnesses

1. **`full_grid: true` emits reset + clear before any span.** Order
   matters and is asserted as order, not membership: a clear *after* a
   span erases the frame it was meant to precede.
2. **`full_grid: true` WITH EMPTY SPANS still clears.** This is the
   exact stale-blank case and the one a careless implementation
   misses — an early return on `spans.is_empty()` is the obvious
   "optimization", and it reintroduces the entire bug for the frame
   that needs the clear most: a resync to a screen that should be
   blank.
3. **`full_grid: false` emits neither**, whatever the spans are.

### 5.1a What implementation changed about §5.2

**The time-based settle described below does not work**, and the
framing said to build it. A settled pmacs screen emits per-frame bytes
indefinitely, so "output stopped growing" never becomes true — the wait
simply runs to its deadline. The vterm suite already recorded the same
behaviour ("a settled screen emits empty diffs forever"); this framing
did not connect it.

The shipped witness anchors its mark to **content** instead: just past
the first painted byte of the fixture. That excludes both of startup's
clears *by construction* rather than by timing — `Frontend::new` clears
before any frame exists, and the first frame is itself a resync whose
clear precedes its own spans — so no timing assumption survives in the
test at all. It is strictly stronger than what §5.2 asked for.

One further correction found by running it: the repaint-ordering
assertion must be scoped to **after the new clear**. The fixture
repeats its marker, so the suffix opens with the tail of startup's own
frame, and an unscoped comparison measures the clear against paints it
was never meant to precede.

### 5.2 PTY acceptance — as originally framed

`tests/common/pty.rs:42` exposes `resize(rows, cols)`, so the real
scenario is reachable: spawn pmacs, put distinctive content on screen,
resize the PTY (a real `SIGWINCH`), and assert on what follows.

**The assertion must be suffix-scoped, or it proves nothing.**
`src/frontend.rs:166` emits `Clear(ClearType::All)` at startup, so a
test that searches the whole output finds a clear **whether or not
resize handling works** — it would pass against the current broken
build. So:

- capture `output().len()` as a mark **only after startup is known
  complete** (a startup still in flight would put its own clear into
  the suffix and recreate the same false pass, one step later);
- resize;
- slice **strictly after the mark**, and assert that within that
  suffix, reset-color + SGR-reset + clear all appear **before the
  first repaint output**.

**A limit, stated rather than discovered in review:** the vterm suites
assert on **raw output bytes**; there is no screen model and no
`vt100` / `termwiz` / `vte` dependency in the workspace. So this proves
*pmacs emitted a clear at the right moment*, not *the screen ended up
correct*. Closing that gap means introducing a terminal emulator into
the test dependencies — **out of scope here**, and recorded as a
candidate rather than smuggled in.

---

## 6. Coherence impact (§20 requirement)

**Concern served: `COHERENCE.md` §16, "Productize the Semantic
Frontend Architecture."** §16 names *efficient incremental updates* and
*stable remote attachment* among the properties the semantic protocol
must actually deliver, and FG-INV is a rule the incremental-update
mechanism depends on. A resync that silently fails to resync is that
concern's failure mode, not a cosmetic one.

**No scorecard change.** Row 16 reads **Strong** (`COHERENCE.md:112`),
and this lane neither earns nor forfeits that: it repairs one unhonored
invariant inside a mechanism the row already credits. Per §25 an
audited claim moves with the PR that changes it — nothing here changes
what the row asserts, so it stays put and this sentence records that
the question was asked.

- **Journey steps touched:** none. This is rendering correctness, not
  a new capability or step.
- **Interaction islands added:** none. No new keybinding, mode, or
  surface.
- **Config registry:** no new settings. Stage 2's persisted zoom level
  and Stage 3's wrap/scroll toggle enter it; Stage 1 has no option to
  register.
- **Background-work attribution:** none. No async work.

---

## 7. Not in scope

GUI zoom (Stage 2) and long-line wrap/scroll (Stage 3). A terminal
emulator in the test dependencies. Changing what `full_grid` *means* —
FG-INV documents the existing contract on the protocol type, it does
not redefine it, and the sparse-resync design stays as is. The `pmacs.terminal` child-PTY
`SIGWINCH` path, which is a different question about a different
process. Any change to *when* `needs_full_grid` is set — the producer's
triggers are correct and verified in §2.
