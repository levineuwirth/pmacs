# Framing — the tree primitive

**Revision 2.** Status: framing only, **not yet approved**. No
implementation. Scouted against `githubsucks/main` @ `12f2970`.

**Revision 1 → 2**, all from review, all verified against source before
applying:

- **Refresh was unscoped for the anchor consumer.** Two criteria rested
  on `g` refresh; the outline has no `g`, no `on_refresh`, and
  `listview.refresh` no-ops without one. Refresh is now **out of scope**,
  with the LSP re-request question it actually raises stated (§1.5a).
- **Acceptance 1 decided Q#TR4 while calling it open** — "no
  `string.rep` in `lsp.lua`" commits to primitive-owned indentation. It
  is representation-neutral now.
- **Q#TR1 misread §14.** A tree is not the "second primitive" §14 warns
  against; §14 **explicitly lists one**. The warning is against bespoke
  per-consumer plumbing. Retradeoffed without prejudging.
- **The regression criterion named the wrong consumers.**
  `*buffer-list*` and project search do **not** use listview — §14 says
  so and calls the older claim an error. The real siblings are
  `*references*` and `*lsp-help*`.
- **Consumer accounting tightened**: five named future consumers, not
  six; **one** existing anchor plus **one** future constraint source,
  not "two that exist"; and "every input a tree needs" qualified, since
  stable identity is exactly what is missing.

`COHERENCE.md` §14 grades **Tree ✗ — none**, and it is the last missing
workbench primitive. §20's Priority 5 names it as what remains after the
bottom panel, and its argument is specific: *"building it once before
dired's directory view and the workers tree harden their own conventions
is exactly this section's point."*

**This document deliberately narrows that argument.** §14 names five
future consumers — project files, symbol hierarchy, package dependency
graph, worker trees, git status — and designing a shared primitive
against five hypothetical ones is how you get a model that fits none.

The scout found a narrower and firmer basis, and the distinction between
its two halves matters:

- **One EXISTING anchor consumer.** The LSP outline already ships a
  tree and fakes it (§1.1). It is the only consumer that exists today.
- **One FUTURE constraint source.** dired's `i` insert-subdirectory is
  scoped and deliberately deferred (§1.2). It constrains the design; it
  does not validate it, because nothing has been built against it.

Calling these "two consumers that exist" would overstate the evidence by
exactly one.

---

## 0. Coherence impact (COHERENCE §20)

- **Concern: §14 Coherent Workbench Primitives.** Tree is the one
  remaining ✗ in its inventory. This closes it for the **one consumer
  that exists** (the LSP outline) and gives the rest an adoption path.
- **Journey steps touched:** none directly. Step 6 (LSP) gains a real
  hierarchy view where it currently has indented text.
- **Interaction islands (§6): NONE, unless evidence forces one.** The
  default is the ordinary **buffer-local keymap** idiom that listview,
  compile, dired and terminal already use — a tree's expand/collapse
  keys are buffer-local bindings on a generated buffer, not a new
  dispatch shadow. §6 grades islands "weak, and growing"; this stage
  must not add to that count. If some behaviour genuinely cannot be
  expressed as a buffer-local binding, that is a finding to report, not
  a licence to add an island.
- **Config registry adoption:** none proposed. If a preference emerges
  (initial expansion depth, say), it enters the registry rather than
  becoming a hardcoded constant — but nothing yet requires one.
- **Background-work attribution:** none.
- **Enables:** DAP's variables view, which is inherently a tree
  (scopes → objects → fields) and would otherwise become the **third**
  bespoke implementation.

---

## 1. Ground truth (measured at `12f2970`)

### 1.1 The LSP outline is a shipped tree consumer, faking it

**The hierarchy already exists and is already discarded.**

`Symbol::push_hier` (`src/symbol.rs:110`) walks a genuine LSP
`DocumentSymbol` tree — it recurses on `item.get("children")` — and
**flattens it** into `Vec<Symbol>`, preserving:

- `depth: u32` — "Nesting depth in a hierarchical `DocumentSymbol` tree";
- `parent` — `containerName` for flat shapes, or the parent chain joined
  with `::` for hierarchical ones;
- document order, **parents before children** (`SymbolResponse.symbols`).

`lsp.lua` then re-renders that depth as *leading spaces inside the row
text*:

```lua
-- Arc 1b phase 2: a browsable *outline* panel. Symbols arrive
-- FLAT with a `depth` field --- indent, don't recurse.
text = string.format("%s%s  [%s]", string.rep("  ", sym.depth or 0), sym.name, tag),
```

So the outline has **no collapse, no expand, no parent/child
navigation** — indentation is a string.

**Every input a tree needs to RENDER is already computed** — depth,
parent, and order — and only the view throws it away. That is not the
same as every input a tree needs: **stable node identity is missing**
(§1.5, Q#TR3), and it is the one input no existing field supplies.

**This is the anchor consumer.** It needs no new data plumbing, and its
limitation is observable today rather than hypothetical.

### 1.2 dired declined to invent one, and its case is already scoped

Dired Stage 1 landed a **flat** listing for Emacs parity and deferred
the recursive case explicitly: `docs/dired-framing.md` §13 names
**`i` insert-subdirectory (in-buffer recursive listing)** as deferred.

§14 credits this directly: dired "landed **without** inventing one". That
restraint is what keeps the door open — and it means this constraint
source carries real requirements rather than a wishlist: a fixed-width
column contract (`pmacs.dired._layout`), a frozen test fixture, and
path-keyed entries.

### 1.3 The workers view is NOT a consumer yet

`editor.list-workers` is
`pmacs.window.switch_buffer(pmacs.workers.show())` — a **Rust-generated
text buffer, raw-switched** into the active window. It is not a listview
and has no rows. §14's "worker trees" is a future consumer, not a
current one.

*(Incidental, and worth a separate look: that raw switch is the same
`switch_buffer` pattern that broke the outline under bottom-panel Stage
3. It is harmless while `*workers*` is not a panel, and inherits the
hazard the moment it becomes one.)*

### 1.4 What listview's model would have to gain

listview's contract today:

```lua
pmacs.listview.open {
  name = "*references*",
  header = "12 references   RET visit  n/p move  g refresh  q quit",
  rows = { { text = "src/foo.rs:12:4", item = <any> }, ... },
  on_visit = function(item) ... end,
  on_refresh = function() return rows end,
}
```

and `render` is:

```lua
local lines = { p.header }
p.line_to_item = {}
for _, row in ipairs(rows) do
  lines[#lines + 1] = row.text
  p.line_to_item[#lines - 1] = row.item
end
pmacs.buffer.set_generated_contents(p.buffer, table.concat(lines, "\n"))
```

**A flat array plus a line→item map.** Depth appears nowhere; it is
baked into `row.text` before listview ever sees it. `item` is `<any>` —
**opaque to listview by design**, which matters for §2's identity
question.

### 1.5 Refresh restores a LINE, not a node — and that is the crux

```lua
local saved = pmacs.editor.cursor_line()
local rows = p.on_refresh() or {}
render(p, rows)
...
seat_cursor(p, saved)
```

`refresh` saves a **line number**, rebuilds the rows wholesale from a
freshly-produced array, and re-seats by walking `move_down` that many
times.

Today this is a mild wrong-restore: if the new list has a different
shape, the cursor lands on whatever row now occupies that line. For a
flat list of roughly stable shape, tolerable.

**Collapse breaks it outright.** Expanding a node inserts rows *above*
the cursor, so a line-keyed restore lands somewhere unrelated. And
collapse state itself must survive refresh, which requires recognising
"the same node" across two independently-produced arrays — something
neither `line_to_item` nor an opaque `item` can do.

**This is why Q#TR3 exists and why it is not a detail.** It decides
whether selection and expansion can survive a model update at all.

### 1.5a …but the ANCHOR CONSUMER HAS NO REFRESH AT ALL

Revision 1 built two acceptance criteria on `g` refresh without checking
that the outline supports it. **It does not:**

- its header offers `RET visit  n/p move  q quit` — **no `g`**
  (`lsp.lua:2490`);
- it supplies **no `on_refresh`**;
- and `listview.refresh` opens `if not (p and p.on_refresh) then return
  end` — **a no-op** for this panel (`listview.lua:262`).

So "collapse state survives `g`" was unreachable for the only consumer
that exists. **Refresh is therefore out of scope for this stage** unless
someone first answers a question this framing does not: an outline
refresh means **re-requesting `textDocument/documentSymbol`**, which is
an async LSP round-trip with its own await, failure and staleness
handling — and it raises who owns the resulting state when the response
arrives against a buffer the user may have edited or left.

That is a real feature (`*references*` has `g` and an `on_refresh`; the
outline never gained one), and it is **not** a tree concern. Bundling it
here would make the tree lane responsible for LSP request lifecycle.

**Consequence for acceptance:** the criteria are re-scoped to what the
anchor consumer can actually exercise — collapse and selection surviving
**re-render**, which the primitive controls — and refresh-survival is
recorded as a follow-on for whoever gives the outline a refresh.

### 1.6 What is NOT established

- **Nothing is implemented or measured.** Unlike the last two lanes
  there is no fallout to census: §14 grades Tree ✗, so there is no
  existing behaviour to preserve and no baseline to diff. **This framing
  argues a model rather than measuring one**, which is the shape that
  has historically needed the most review rounds here (Lean Stage 3b
  took six; the signal lane had three tolerance rules rejected in a
  row). Treat its claims as proposals.
- **No consumer has asked for collapse.** The outline's limitation is
  inferred from its structure and its own "indent, don't recurse"
  comment, not from a user report.
- **dired's `i` has not been re-scouted** against current `main`; §13's
  deferral is the only evidence that its requirements are as described.

---

## 2. Questions

All four are genuinely open. The first two the review already flagged as
open; the third is the one review added; the fourth follows from §1.4.

- **Q#TR1 — extend `listview`, or add a separate `treeview`?**

  **Revision 1 framed this wrongly and the correction changes the
  tradeoff.** It claimed a separate treeview would be "exactly the
  second primitive §14 warns about". §14 does not warn against a tree —
  **it explicitly lists one**, alongside virtual list, in the reusable
  set it wants: *"editable text view, virtual list, **tree**, structured
  table, inspector…"* (`COHERENCE.md:1264`). A tree surface is a named
  goal, not a violation.

  What §14 actually warns against is **bespoke per-consumer plumbing** —
  each subsystem inventing its own UI vocabulary. A `treeview` that
  shares the existing buffer/panel disciplines (generated-buffer writes,
  Q#GB18 handle identity, panel placement, `q` quit-action) is not that;
  a tree hand-rolled inside `lsp.lua` would be.

  So the real tradeoff is narrower:
  - **Extending listview** touches three shipped call sites and the
    `line_to_item` contract, and risks making a working flat primitive
    worse for the consumers that do not need depth.
  - **A separate treeview** keeps the flat primitive untouched, but must
    *share* rather than *duplicate* the panel disciplines — and "shares
    them" is an implementation claim that has to be verified, not
    asserted.

  **No leaning recorded**; the scout found nothing that decides it.
- **Q#TR2 — who owns collapse state?** Candidates: the primitive (keyed
  by node id), the consumer (passed in with the rows each render), or
  the buffer (as generated-buffer state). Consumer-owned keeps the
  primitive stateless and makes refresh the consumer's problem;
  primitive-owned centralises it and forces Q#TR3 to be answered first.
- **Q#TR3 — what is a stable node identity across refresh?** *(Added at
  review.)* This determines whether **selection and expansion survive a
  model update** (§1.5). Constraints the scout established:
  - listview cannot derive one: `item` is `<any>` and opaque.
  - The outline's data is nearly sufficient — the `::`-joined parent
    chain plus name — but **not unique**: overloads and same-named
    siblings collide.
  - dired's would be genuinely stable: the path.
  - So identity is almost certainly **consumer-supplied**, which makes
    it part of the primitive's public contract rather than an internal
    detail. That is a real API commitment and should be decided
    deliberately, not defaulted into.
- **Q#TR4 — does the row still carry pre-rendered `text`?** Today the
  consumer formats indentation into the string. If the primitive owns
  depth it should probably own indentation too — but the outline also
  appends `[kind]` tags and dired has a fixed-width column contract, so
  "the primitive renders the row" may not survive contact with either.

---

## 3. Bets

- **Bet 1 — the outline is a sufficient first consumer on its own.** Its
  data already carries depth and parent; adopting it requires no LSP-side
  change. *Falsified if adoption needs `Symbol` to change shape.*
- **Bet 2 — identity must be consumer-supplied** (§1.5, Q#TR3).
  *Falsified if some derivable key proves both stable and unique across
  the two consumers.*
- **Bet 3 — no interaction island is required.** Expand/collapse are
  buffer-local bindings on a generated buffer, exactly as `RET`/`n`/`p`/
  `g`/`q` already are. *Falsified if some behaviour cannot be expressed
  that way — which would be a finding worth reporting, not a licence.*

---

## 4. Acceptance

**Not final** — this framing argues a model, and the criteria cannot be
fixed until Q#TR1–TR3 are decided. The shapes they will take:

1. The **LSP outline renders its hierarchy through the primitive**
   rather than by pre-formatting it into row text, and `Symbol` is
   unchanged. **Representation-neutral on purpose:** whether the
   primitive emits the indentation, or the consumer still supplies a
   rendered string alongside structural depth, is **Q#TR4** and is not
   decided here. Revision 1's wording ("no `string.rep` in `lsp.lua`")
   committed to primitive-owned indentation while calling that question
   open.
2. **Collapse and expand work**, and **collapse state survives a
   re-render** — the primitive re-emitting the buffer from the same
   model. *(Not "survives `g` refresh": the anchor consumer has no
   refresh at all — §1.5a.)*
3. **Selection survives a re-render by node, not by line** (§1.5).
4. **No new interaction island**: every tree key is a buffer-local
   binding, and the dispatch-shadow count is unchanged. Asserted, not
   assumed.
5. **listview's OTHER consumers are unaffected** — `*references*` and
   `*lsp-help*`, which with `*outline*` are the **only three
   `pmacs.listview.open` call sites, all in `lsp.lua`**. They render
   byte-identically, pinned by their existing suites.

   **Revision 1 named `*buffer-list*` and project search here and was
   wrong** — §14 measured that they do **not** use listview and calls
   the earlier claim a documentation error (`COHERENCE.md:1286`).
   Repeating it would have re-introduced a mistake that document exists
   to correct. If the broader surfaces are ever in scope, their
   independent render paths and tests must be named explicitly.
6. **Q#GB18 identity holds**: a foreign buffer with the panel's name is
   never adopted, and the primitive is keyed by handle, not name.
7. The **generated-buffer write invariant** is preserved
   (`set_generated_contents`), including the rope lock and history rules
   the listview suite already pins.

---

## 5. Parked

- **The other four §14 consumers** — project files, package dependency
  graph, worker trees, git status. They adopt later; designing for them
  now is the failure this framing avoids.
- **DAP's variables view.** The reason the primitive is worth building
  before the debugger, and not part of it.
- **dired's `i` insert-subdirectory.** The second consumer, and the
  right forcing function for the design — but it is its own stage with
  its own framing, and dired Stage 2b/3 are ahead of it.
- **Making `*workers*` a listview/tree consumer** (§1.3), and the
  raw-switch hazard noted there.
- **Tree rendering in the GPU frontend** beyond whatever the shared
  generated-buffer path already gives.
- **Giving the outline a refresh** (§1.5a). `*references*` has `g` and
  an `on_refresh`; the outline never gained one. Adding it means
  re-requesting `textDocument/documentSymbol` with its own await,
  failure and staleness handling, and deciding who owns the result when
  it arrives against a buffer the user may have edited. **That is LSP
  request-lifecycle work, not tree work** — and only once it exists can
  "collapse survives refresh" be a criterion rather than an aspiration.

---

## 6. Gates

The standing `CLAUDE.md` suite. The suites most likely to move are
`listview_acceptance` and `m4_acceptance` (the LSP outline and hover
panels are listview consumers — bottom-panel Stage 3 established that
transitive relationship the hard way).

**Sweep both feature configurations.** Stage 3 shipped a broken
crdt-gated suite because every local sweep ran `--features luajit`
without `crdt`; and `--no-fail-fast` is required, or a multi-suite break
reports as one suite.

---

## 7. Branch plan

Not settled, because it depends on Q#TR1. Two shapes:

- **If listview is extended:** one branch, with the flat-consumer
  no-change proof (acceptance 5) landing *before* the outline adopts, so
  a regression in references or buffer-list is attributable.
- **If a separate `treeview`:** the primitive and its first consumer are
  separable, and the outline's adoption can be its own PR.

Either way the outline adopts **before** dired's `i` is attempted: it is
the consumer whose data already fits, and it is the one that proves the
model without also needing a new listing mode.
