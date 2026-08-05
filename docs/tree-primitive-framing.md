# Framing — the tree primitive

**Revision 5.** Status: **IMPLEMENTED and GATED; PR HELD for review.**
Q#TR1–TR4 decided at approval. Both sweeps green and both behavioural
claims bite-verified — see §6a. Scouted against
`githubsucks/main` @ `12f2970`. Carries a correction to `COHERENCE.md`
§14 (revision 4, below).

**Revision 4 → 5** records the four decisions and makes acceptance final.

**The decision that made the others cheap:** collapse only ever *hides*
rows — it never changes a surviving row's depth. Combined with §1.1's
document order (**parents before children**), a node's descendants are a
**contiguous run of following rows with greater depth**. So collapse is
**filtering an existing array**, not re-deriving one. The primitive
therefore never calls the consumer to re-render a collapse, and
pre-rendered indentation stays correct — which is why Q#TR4 resolves
toward the consumer keeping `text`.

| question | decision |
|---|---|
| **Q#TR1** | **Extend `listview`.** A separate `treeview` would either duplicate ~200 lines of panel discipline (Q#GB18 handle identity, Q#GB13 `<2>` disambiguation, the read-only intercept, `prev` capture, quit chain, generated-buffer writes) or require *extracting* them from a shipped primitive first — the riskier change. Extending is backward-compatible by construction: absent `depth`/`id` give today's behaviour exactly, which the three flat consumers already produce. |
| **Q#TR2** | **Primitive-owned collapse state**, keyed by row id, held in the panel record. Consumer-owned would make every consumer reimplement refresh survival. |
| **Q#TR3** | **Consumer-supplied `row.id`**, a **string or number**, compared by value; the primitive never derives one. *(Review narrowed this from "opaque": collapse state keys a table, and Lua table indexing ignores `__eq`, so an opaque id could restore selection while silently losing its fold. `check_ids` enforces it where rows enter.)* The outline uses **`line:col`** — unique per document and stable across re-render, where the `::` parent chain collides on overloads. |
| **Q#TR4** | **Consumer keeps pre-rendered `text`**; `depth` is structural only. Also sidesteps the conflict with dired's fixed-width `_layout` column contract when it adopts. |

**Acceptance 5 is decided too: byte-identity coverage is written**,
including the fake-LSP harness work for `*references*`.

**Revision 3 → 4**:

- **"No `g`" was literally false**, in revisions 2 and 3 both.
  `bind_local_keymap` binds `g → listview.refresh` on **every** panel
  unconditionally (`listview.lua:147`). What three of the four lack is
  an `on_refresh`. The §1.3a table now separates **`g` bound**,
  **refresh advertised** and **refresh functional**, because those are
  three different facts and I had been collapsing them into one.
  Consequence worth noting on its own: the outline has a **dead refresh
  binding** — `g` is dispatched and silently does nothing, with no
  status message.
- **`COHERENCE.md` §14 is corrected in this branch**, not deferred.
  §25 requires an audited claim to be updated by the PR that changes
  it; **#204 changed it and missed it**, so the correction rides the
  framing that found it. The `ad41cf1` audit fact is retained as
  history, with the current count of four and `*lsp*` named as the
  post-audit addition. The §0 scorecard row moves with the body, since
  it carried the same "3 call sites".

**Revision 2 → 3**, three further review findings, all verified in
source:

- **`*references*` has no `on_refresh`** — revision 2 claimed it had
  refresh, twice. The consumer with refresh is **`*lsp*`** (§1.5a). The error
  came from reading `listview.lua`'s **module-docstring example**, which
  illustrates the API using `*references*` with `g refresh` in the
  header. An example is not a consumer. *The refresh-scoping conclusion
  is unaffected — it depended on the OUTLINE lacking refresh, which
  holds.*
- **The branch plan still said "references or buffer-list"** — the same
  `*buffer-list*` error acceptance 5 had already been corrected for,
  surviving one section further down.
- **"Byte-identically, pinned by their existing suites" was
  unsupported.** `listview_acceptance` states in its own header that
  `*references*` needs a live LSP and is not exercised there, and the m4
  hover test asserts content *presence*, not byte-exact output.
  Acceptance 5 now poses that as a decision — write the byte-identity
  test, or weaken the claim — with a leaning and its cost.

**Found while verifying those: §14's "exactly three call sites" is
stale — there are FOUR** (§1.3a), and the fourth (`*lsp*`, added by
#204) is the only one with refresh.

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

### 1.3a There are FOUR listview consumers now, not three

§14 measured "exactly **three** `pmacs.listview.open` call sites, all
three in `builtin/runtime/lsp.lua`" at `ad41cf1`. **There are four**, and
the fourth matters here:

| call site | panel | `on_visit` | `on_refresh` | `g` bound? | refresh advertised? | refresh FUNCTIONAL? |
|---|---|---|---|---|---|---|
| `lsp.lua:2442` | `*references*` | yes | no | **yes** | no | **no** |
| `lsp.lua:2488` | `*outline*` | yes | no | **yes** | no | **no** |
| `lsp.lua:2924` | `*lsp-help*` | no | no | **yes** | no | **no** |
| `lsp.lua:3004` | `*lsp*` (`lsp.status`) | no | **yes** | **yes** | **yes** | **yes** |

**`g` is bound on ALL FOUR.** `bind_local_keymap` binds
`g → listview.refresh` for every panel unconditionally
(`listview.lua:147`), so "no `g`" — which revisions 2 and 3 both said —
is **literally false**. What three of them lack is an `on_refresh`, and
`listview.refresh` returns immediately without one.

**So `g` on the outline is a DEAD BINDING**: bound, dispatched,
silently does nothing. That is a small UX wart in its own right — a key
that responds to nothing, with no status message — and it is a separate
observation from anything this framing proposes. Recorded, not fixed
here.

`*lsp*` arrived with Journey Stage 1b-2 (#204), after §14's audit. It is
**the only listview consumer with refresh at all**, which is why §1.5a's
scoping conclusion holds: refresh is a feature exactly one panel has, and
it is not the anchor.

§14's line numbers have also drifted (`:2056`/`:2102`/`:2513` against
today's `:2442`/`:2488`/`:2924`). The count is the part that matters;
this is recorded so the next reader does not inherit "three".

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

- its header offers `RET visit  n/p move  q quit` — refresh is **not
  advertised** (`lsp.lua:2490`);
- it supplies **no `on_refresh`**;
- `listview.refresh` opens `if not (p and p.on_refresh) then return
  end` — **a no-op** for this panel (`listview.lua:262`);
- and `g` **is** bound regardless (§1.3a), so the outline has a
  **dead refresh binding**, not an absent one.

So "collapse state survives `g`" was unreachable for the only consumer
that exists. **Refresh is therefore out of scope for this stage** unless
someone first answers a question this framing does not: an outline
refresh means **re-requesting `textDocument/documentSymbol`**, which is
an async LSP round-trip with its own await, failure and staleness
handling — and it raises who owns the resulting state when the response
arrives against a buffer the user may have edited or left.

That is a real feature — **`*lsp*` (`lsp.status`) has `g refresh` and an
`on_refresh`** (`lsp.lua:3004`); the outline never gained one — and it is
**not** a tree concern. Bundling it here would make the tree lane
responsible for LSP request lifecycle.

*(Revision 2 attributed refresh to `*references*` twice. It has neither:
its header is `RET visit  n/p move  q quit` and it supplies only
`on_visit` (`lsp.lua:2442`). The error came from reading
`listview.lua`'s **module-docstring example**, which illustrates the API
using `name = "*references*"` and a header containing `g refresh` — an
example, not a consumer.)*

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

1. The **LSP outline supplies structural `depth` and `id`** so the
   primitive can collapse and expand, and `Symbol` is unchanged.
   **Per Q#TR4 its `text` stays consumer-rendered** — the `string.rep`
   indentation remains in `lsp.lua`, because collapse hides rows without
   changing any surviving row's depth, so pre-rendered indentation is
   still correct. `id` is `line:col`.
2. **Collapse and expand work**, and **collapse state survives a
   re-render** — the primitive re-emitting the buffer from the same
   model. *(Not "survives `g` refresh": the anchor consumer has no
   refresh at all — §1.5a.)*
3. **Selection survives a re-render by node, not by line** (§1.5).
4. **No new interaction island**: every tree key is a buffer-local
   binding, and the dispatch-shadow count is unchanged. Asserted, not
   assumed.
5. **listview's OTHER consumers are unaffected** — `*references*`,
   `*lsp-help*` and `*lsp*` (§1.3a: four call sites, all in `lsp.lua`).

   **What existing suites actually pin, stated honestly.**
   `listview_acceptance` drives the substrate hermetically and says so
   in its own header: *"The references panel itself needs a live LSP and
   is validated manually / via the m4 harness"* — so **it does not
   exercise `*references*` at all**. The m4 hover test asserts content
   *presence*, not byte-exact output. **"Byte-identical, pinned by
   existing suites" was therefore unsupported** for both panels named.

   So this criterion needs a decision, not a wording tweak:
   - **either** byte-identity becomes a **new test requirement** this
     stage writes — capturing each panel's rendered buffer before and
     after and diffing it, which needs the m4 fake-LSP harness for
     `*references*`;
   - **or** the claim weakens to what is genuinely pinned today:
     the substrate behaviours `listview_acceptance` covers (open,
     navigate, visit, `q` restore, the read-only intercept, the
     round-trip gate, refresh) plus content-presence for hover.

   **DECIDED: write the byte-identity test**, including the fake-LSP
   harness work for `*references*`. A flat consumer silently gaining an
   indent column is exactly the regression this criterion exists to
   catch, and content-presence would not see it.

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
- **Giving the outline a refresh** (§1.5a). **`*lsp*` has `g` and an
  `on_refresh`**; the outline never gained one. Adding it means
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

## 6a. Verification record, including one unclassified occurrence

**The luajit sweep is 3453 / 0** and the count reconciles exactly:
`main` is 3450 (Stage 3's 3449 sweep predated its own capability-fallback
pin) plus this lane's three listview tests and one m4 test.

**The crdt sweep is 3722 / 0**, likewise +4 on `main`'s 3718.

### An UNCLASSIFIED, UNCAPTURED local occurrence

The **first** local crdt sweep of this branch reported **7 failures**.
**It is recorded here as unclassified and it is deliberately NOT a row
in `docs/ci-red-signatures.md`** — that registry keys on a normalized
signature, and this occurrence has none to match, so a row would confer
recognisability it cannot support.

**The signatures were destroyed before they were read.** The sweep was
piped through an aggregation that emitted only totals. That is the exact
failure the registry exists to prevent, committed one lane after writing
it — and it is why the cause cannot now be established rather than
merely being unknown.

**Re-runs, with what each does and does not support:**

| run | isolated? | result |
|---|---|---|
| first | no — concurrent with another lane's build | **7 failed, signatures lost** |
| second | no | 3722 / 0 |
| A | **no** — the isolation guard printed "aborting" and did not abort | 3722 / 0 |
| B | **yes** — verified idle | 3722 / 0 |
| C | **yes** — verified idle | 3722 / 0 |

Two genuinely isolated runs, both clean. **That supports repeatability
under isolation. It does not establish what caused the original.**

### Two NON-CAUSAL hypotheses, neither testable now

Both are mechanisms known to have been present. Neither is offered as an
explanation, because the occurrence's signatures no longer exist to test
either against:

1. **Shared `CARGO_TARGET_DIR`.** Another lane's worktree shared
   `/home/jeans/build/cargo-target`, so its `cargo test --workspace`
   overwrote `target/debug/pmacs` mid-sweep. That lane observed the
   reciprocal case independently, caught the concurrent build with
   `pgrep`, and its failing text named its own cause ("start the daemon
   built with the `crdt` feature").
2. **Resident leaked daemons.** ~40 orphaned `pmacs --daemon` processes
   were present, some four days old (see the lane in
   `docs/active-work.md`). Isolated sweeps leak 3–4 each, so the
   population was growing throughout.

**Having two plausible mechanisms and no way to discriminate is the
result.** Reporting either as *the* cause would be the reasoning this
project has rejected repeatedly: concluding something about an
occurrence from something that was not about that occurrence.

## 7. Branch plan

Q#TR1 is decided, so the listview-extension shape applies:

1. **Extend `listview`** — optional `depth` / `id` on rows,
   primitive-owned collapse state, ancestor-collapsed filtering in
   `render`, selection re-seated **by id**, and a toggle binding. Absent
   `depth`/`id` must behave exactly as today.
2. **Byte-identity proof for the flat consumers** (acceptance 5),
   landing *before* the outline adopts, so any regression in
   `*references*`, `*lsp-help*` or `*lsp*` is attributable to the
   primitive change rather than to adoption.
3. **The outline adopts** — supplies `depth` and `id = line:col`, keeps
   its rendered `text`.
4. **Lane, handoff and `COHERENCE.md` §14** updated; the §14 correction
   from revision 4 rides this PR per §25.

dired's `i` is **not** attempted here (§5).
