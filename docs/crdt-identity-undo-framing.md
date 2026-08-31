# Identity-replace undo — a CRDT-version delta is not a text delta

**Status: revision 5 — APPROVED at revision 4 and IMPLEMENTED**
(PR #246, branch `crdt-identity-undo`). Revision 5 is a correction pass
answering implementation review; it changes the invariant's shape, not
its ruling.

Revision 5 answers three findings against the implementation:

1. **the predicate conflated an empty TEXT delta with a version
   delta.** It called every empty-range/zero-insertion edit
   `version_only` and then accepted `(History, empty, None)` through a
   wildcard arm — which contradicts this framing's own "the op must
   survive". The rule is now a full enumeration over three independent
   axes (§1a), and C5 asserts all four empty-delta quadrants rather
   than two;
2. **the public `Edit` doc was factually false**, saying forward
   `apply_edit` never produces the empty-delta shape while C2b proves
   all three forward empty forms do. The shape is now named an **empty
   text delta**, reachable on both paths, with `crdt_op` as the
   discriminator;
3. **R7's write-up overstated what the paired gate runs exclude** — see
   `docs/ci-red-signatures.md`; the pair excludes the source tree and
   nothing else.

Revision 4 answered review of 3, which found one substantive gap: **C9
guarded the census by file set and count, which a same-file substitution
walks straight through.** C9 now asserts the exact
`(file, impl target)` pairs, and its claim is scoped to in-tree
implementations.

Revision 3 answered review of 2 by completing §4's census: it closes by
construction, and two of its results changed the framing's own claims.

## 1. The decision, ANSWERED — and it is about PROVENANCE, not shape

**A visible TEXT delta and a CRDT-VERSION delta are INDEPENDENT
dimensions of `Edit`.** An `Edit` may legitimately carry
`crdt_op = Some(_)` with `range.is_empty() && inserted_len == 0`.

**Revision 2 stated that without qualification, and review 2 showed why
that is too loose:** an `Edit` carries no provenance marker, so if the
shape alone were legitimate the invariant would have nothing left to
assert. The precise answer:

> **An empty TEXT delta carrying a CRDT op is legitimate when the
> `Edit` came from `undo`/`redo`, and REQUIRED there. On the FORWARD
> path the same shape carrying an op is a bug, and stays asserted.**

That is a real narrowing, not a repeal, and it is what makes C5
testable at all.

### 1a. The three axes, enumerated

Revision 4 wrote this as one predicate with a default, and the
implementation inherited the gap: `(History, empty delta, None)` fell
through a wildcard and was accepted. The axes are independent — that is
the lane's whole claim — so the rule is a full enumeration:

| provenance | text delta | `crdt_op` | verdict |
|---|---|---|---|
| forward | empty | `None` | **valid** — a syntactic no-op |
| forward | empty | `Some` | **invalid** — the original bug |
| forward | real | `Some` | valid |
| forward | real | `None` | invalid |
| history | empty | `Some` | **valid** — a version-only edit |
| history | empty | `None` | **invalid** — the version advance is gone |
| history | real | `Some` | valid |
| history | real | `None` | invalid |

**An empty text delta is a SHAPE, not a verdict.** Both paths reach it.
`crdt_op` is what separates them, and each direction of that separation
is asserted.

**Why this answer:**

- The invariant it contradicts was written for `is_no_op_edit`
  (`src/buffer.rs:1836`), a **pre-check on the `EditOp`** reached only
  from `run_rope_edit_and_broadcast` (`:1256`). **`undo_crdt_mode` and
  `redo_crdt_mode` never reach it** — they diff two ropes via
  `derive_replacement_edit` (`:1440`, `:1525`) and attach the op
  `crdt.undo()` produced (`:1454`), so identical ropes yield an empty
  range describing a real operation.
- **Forward edits reach the empty-delta shape routinely** — each of
  the three syntactically empty `EditOp` forms produces exactly it, as
  C2b asserts. What is unreachable forward is the shape **carrying an
  op**: an empty form short-circuits to `(None, None)`, and a
  real-delta form is not empty. So forward "empty range and zero
  insertion" implies `crdt_op == None`, still — which is what lets the
  invariant keep its full strength there.
- The op must survive. Dropping it would lose a version advance the
  replicas need — which is what C3 now actually tests, and revision 1's
  C3 did not.
- **The codebase already assumes this, in two places written for other
  reasons.** `FoldStore::translate` (`src/fold.rs:211`–`:213`) and
  `BufferStyleSpanTranslator::on_edit` (`src/overlay.rs:261`–`:263`)
  both return early on `old_len == 0 && new_len == 0`, and both say so
  in a comment — *"Buffers broadcast no-op edits; nothing moved."* This
  lane is not introducing a doctrine; it is naming one that consumers
  were already written against.
- The public contract has room for it. `src/rope.rs:301`–`:303`
  enumerates pure insert, pure delete and replace, and **has no fourth
  case**; the `crdt_op` field doc (`src/rope.rs:316`) goes further and
  asserts the conflation outright ("`None` … for no-op edits in CRDT
  mode"). Both are updated by this lane.

**The empty range's LOCATION is settled by §4's census, not deferred.**
It stays at the buffer end. No consumer is harmed there, and for the
one consumer whose cost depends on it, the buffer end is the *cheapest*
choice — see §4.

## 2. What is already known — and precisely how well

`src/buffer.rs:3044` carries a deterministic fixture,
`crdt_undo_of_an_identity_replace_reports_a_no_op_edit_carrying_an_op`,
`#[ignore]`d at `:3042` and documented from `:3005`. It reduces this
exact case: replacing bytes with **identical bytes** is a textual no-op
but a real CRDT delete-plus-insert, so undoing it advances the CRDT
version while leaving text unchanged.

**Its evidence is narrower than revision 1 claimed, and narrower in one
more place than revision 2 admitted:**

| claim | how it is established | strength |
|---|---|---|
| content stays correct | **asserted** in the fixture — rope and CRDT projection agree | direct |
| the op reaches broadcast consumers | **by INSPECTION of the call sites** | reasoning, not execution |
| the cursor does not jump | **by INSPECTION** — `EditorCore::undo` only clamps to length | reasoning, not execution |

**The cursor row was marked "direct" in revision 2. It is not.** The
fixture body (`buffer.rs:3044`–`:3093`) contains **no reference to
`EditorCore` and none to a cursor**; it exercises `Buffer` alone. The
cursor claim is inspection of a different function than the one the
fixture runs.

**Nothing here replays the op on a remote replica or witnesses
convergence.** Revision 1 said "replicas stay converged" as though it
were established. It is not. **That is exactly what C3 must newly
establish**, and it is the main new evidence this lane produces.

The CI red that prompted the lane is a randomly sampled recurrence of
this fixture, not a new defect.

## 3. Terminology, because revision 1's contradicted itself

**An identity replace IS a forward textual no-op**, and it *does*
produce an operation. So "forward textual no-ops produce no operation"
is false, and revision 1 asserted it while §2 said the opposite.

The correct statement names a **syntactic** category:

> The three **syntactically empty `EditOp` forms** — `Insert` with
> empty bytes, `Delete` with an empty range, and `Replace` with both
> empty — produce no CRDT operation.

That is what `is_no_op_edit` tests, and it stays true.

## 4. The consumer census — COMPLETE

Revision 2 listed `broadcast_on_edit` as a row reading *"every attached
view — not enumerated, owes."* **That is a dispatcher, not a consumer,
and review 2 was right that it cannot stand.** Here is the enumeration.

Both `undo_crdt_mode` (`buffer.rs:1456`) and `redo_crdt_mode`
(`:1537`) do broadcast, so this path is real.

### 4a. How the census closes

Three measurements bound it, so it is complete by construction rather
than by search effort — **for this tree**; see §4d on why no in-tree
measurement can reach further:

1. **The `View` trait's `on_edit` default is `Ok(())`**
   (`src/view.rs:450`–`:452`). Every impl that does not override it is
   **structurally inert** — it never reads the range.
2. **Exactly four non-test impls override `on_edit`**: `ParseView`
   (`syntax.rs:1637`), `TextView` (`text_view.rs:521`),
   `FoldStoreTranslator` (`fold.rs:274`), `BufferStyleSpanTranslator`
   (`overlay.rs:248`). The other twelve inherit the default.
3. **Exactly four production `Buffer::attach_view` call sites exist**
   — `fold.rs:341`, `lua_bindings/mod.rs:3963`, `:4008`, `:8137`.
   Measured over the 50 occurrences of `attach_view` outside its own
   definition: **38 sit inside `#[cfg(test)]`**, and of the 12
   remaining, **8 are doc comments or a different API** (the Lua
   `pmacs.diag._attach_view` name, and `SyntaxRegistry::attach_view` at
   `lua/mod.rs:8138`, which registers a handle rather than a buffer
   view).

### 4b. Broadcast consumers, classified

| attached view | site | reads range? | verdict |
|---|---|---|---|
| `FoldStoreTranslator` | `fold.rs:341` | via `FoldStore::translate` | **INERT** — explicit `old_len == 0 && new_len == 0` early return at `fold.rs:211`–`:213` |
| `BufferStyleSpanTranslator` | `lua/mod.rs:4008` | yes | **INERT** — same explicit early return, `overlay.rs:261`–`:263` |
| `ParseView` | `lua/mod.rs:8137` | yes | **PERMITTED, justified below** |
| `LuaInterceptView` | `lua/mod.rs:3963` | — | **INERT** — overrides `intercept_edit` only (`lua/mod.rs:2132`); inherits the `Ok(())` default |

**`ParseView` is the one permitted effect.** At 0→0 its splice
(`syntax.rs:1656`) is `source.splice(n..n, [])` — the source mirror is
**unchanged** — and it pushes one `InputEdit` with
`start_byte == old_end_byte == new_end_byte` and all three `Point`s
equal (`:1661`–`:1668`). **Why that is acceptable:** a degenerate
`InputEdit` describes no change, so the incremental parse it feeds must
produce an identical tree. **C4 asserts that rather than assuming it**,
and also asserts the pending queue drains, since an effect that
accumulates per undo would not be acceptable.

### 4c. Direct (non-broadcast) consumers

**Revision 2 filed `TextView` under broadcast. It is not attached to
any buffer** — it lives on the window (`win.text_view`) and
`EditorCore::undo` calls it directly at `editor_core.rs:2846`.

| consumer | reads range? | verdict |
|---|---|---|
| `Buffer::adjust_marks_for_edit` (def. `buffer.rs:1609`; called `:1442` undo, `:1527` redo) | yes | **INERT, arithmetically** — with `start == end` and `inserted_len == 0`, every branch is identity: `pos < start` → `pos`; `pos > end` → `pos - 0 + 0`; `pos == start` → `start` under both gravities (`:1617`–`:1629`) |
| `EditorCore::undo` → `TextView::on_edit` (`editor_core.rs:2846`, body `text_view.rs:521`) | yes | **PERMITTED** — `rebuild_lines_from(buf, line_at_offset(range.start))`. Text is unchanged, so the rebuild is **output-identical**; the cost is the tail of the buffer from `range.start`. **The buffer-end location makes this the CHEAPEST possible rebuild** — moving the range to the edit site would rebuild strictly more |
| `search_invalidate_for_edit` → `mark_stale` (`editor_core.rs:1974`) | **no** | **PERMITTED** — unconditional and range-independent. Search matches are marked stale on an edit that changed no text. Acceptable (correctness is preserved; a re-search is redundant, not wrong), and **moving the range would not change it** |
| `search_invalidate_for_edit` → `translate_search_origin` (`editor_core.rs:1984`) | yes | **INERT, arithmetically** — with `start == end` and `inserted_len == 0`: `pos < start` → `pos`; `pos > end` → `pos - 0 + 0`; else `start + 0`, reachable only at `pos == start` (`:1994`–`:2000`) |

### 4d. The disposition

**Five inert, three permitted, none harmed. The range does not move,**
and that conclusion now rests on measurement rather than on a deferral.

| | inert | permitted |
|---|---|---|
| broadcast (§4b) | `FoldStoreTranslator`, `BufferStyleSpanTranslator`, `LuaInterceptView` | `ParseView` |
| direct (§4c) | `adjust_marks_for_edit`, `translate_search_origin` | `TextView`, `mark_stale` |

*(Revision 3 said four and three. Miscount, corrected.)*

The two permitted effects with a cost — `TextView`'s rebuild and
`mark_stale` — are both **strictly cheaper or equal at the buffer end**
than at the edit site, so the location the fixture called arbitrary is
not merely harmless but weakly preferable.

**This census is a point-in-time measurement of THIS TREE**, valid at
the commit the lane branches from. Both `View` (`src/view.rs:419`) and
`Buffer::attach_view` (`src/buffer.rs:674`) are **public**, so a
downstream crate may implement `on_edit` and attach it, and no in-tree
measurement can enumerate that. The census, and C9 with it, are scoped
to in-tree implementations; the public contract §5's C7 updates is what
speaks to anyone outside. Revision 3 claimed C4 would guard it against a new
override or attach site; **it cannot — executing three consumers says
nothing about a fourth, and that claim is withdrawn.** C9 is the guard
that actually holds, and it holds the one condition that matters: if
the set of `on_edit` overrides is unchanged, then every attach site,
new or old, attaches a view that is either the inert trait default or
one of the four already classified.

## 5. Acceptance

| # | contract | witness | mutation |
|---|---|---|---|
| C1 | the fixture runs, and is not silently re-ignored | un-ignore it; **plus a structural assertion** that no `#[ignore]` attribute precedes the fixture's `fn` (via `include_str!` on the file), **plus** the run's `1 passed; 0 ignored` line recorded as gate evidence | restore `#[ignore]` → the structural assertion fires **and** the recorded line reads `0 passed; 1 ignored`. Without one of these, re-ignoring is a green suite |
| C2a | `is_no_op_edit` classifies all three **syntactically empty forms** as no-ops | assert `is_no_op_edit` **directly** for `Insert{bytes:[]}`, `Delete{range:empty}`, `Replace{range:empty,bytes:[]}` | flip **any one** arm (`buffer.rs:1838`–`:1840`) → C2a fires. Nothing sits between the assertion and the classifier, so this mutant **cannot be masked** |
| C2b | end-to-end, each empty form still yields `crdt_op == None` | apply each form through `apply_edit` on a CRDT buffer | **compound mutant, and it must be**: flip the arm **and delete that variant's defensive early return** — `buffer.rs:1177`–`:1182` (Insert) or `:1192`–`:1194` (Delete). See below |
| C3 | an empty-text history op **replays convergently on a REMOTE replica**, **for both `undo` and `redo`** | seed replica B with the **forward** ops, apply the history op to B, assert **(a)** identical materialized text **and (b)** identical CRDT version/frontier; then apply a **causally dependent** op and assert both still agree | **drop the history op before replay** → text still matches, so only the version/frontier assertion catches it |
| C4a | the history edit is **broadcast at all**, for both `undo` and `redo` | attach a counting view (the `RecorderView` shape, `buffer.rs:2218`) and assert **exactly one** `on_edit` per history op | **delete `self.broadcast_on_edit(&inverse_edit)?`** at `buffer.rs:1456` (undo) or `:1537` (redo) → the count is 0 → C4a fires |
| C4b | the classified consumers are unchanged by the real history edit | attach `FoldStoreTranslator`, `BufferStyleSpanTranslator` and `ParseView`; run the identity-replace op; assert fold store unchanged, span vector unchanged, **parse tree identical**, and `pending_edit_count()` returns to 0 after the drain (`syntax.rs:712`, `:737`) | see the note below — **C4b claims no guard mutation**, and C4a is what makes it non-vacuous |
| C4c | the style-span guard's own contract, pinned where it can fire | call `BufferStyleSpanTranslator::on_edit` with a **synthetic INTERIOR 0→0 `Edit`** whose position falls strictly inside an existing span, and assert the span vector is **byte-identical** — not merely equal in coverage | delete `overlay.rs:261`–`:263` → the span splits into two adjacent fragments and the vector differs → C4c fires |
| C5 | the invariant is keyed on **provenance**, and covers **all four** empty-text-delta quadrants of §1a | preserve the `GenOp` classification (`buffer.rs:3101`, where `op` is moved before it can be classified) as an operation class; extract the shape check to take `(class, &Edit)`; then **inject** all four: `(Forward, empty, None)` accepted, `(Forward, empty, Some)` rejected, `(History, empty, Some)` accepted, `(History, empty, None)` rejected | widen the forward rule → C5 fires; accept `(History, empty, None)` → C5 fires, and **revision 4's two-assertion C5 did not**. **The proptest alone catches neither**, because no generated input reaches either row — which is why C5 is a directed injection, not a property |
| C6 | the executed history-case set **is** `{Undo, Redo}` | after the parameterized loop, assert the collected set of cases actually run equals the literal `{Undo, Redo}`; a `match` over the case enum keeps a future variant from being added silently | drop `Redo` from the case list → the **set assertion** fires. Without it the suite simply runs one case and stays green, which is why revision 3's C6 was a zero-execution witness |
| C7 | the public contract names the **empty text delta** and says which path produces which `crdt_op` | the `Edit` doc (from `src/rope.rs:292`) gains the empty-delta shape **as reachable on BOTH paths** — `None` forward, `Some` from history — and the `crdt_op` field doc stops asserting that no-op edits have no op | leave the doc → it contradicts the code the lane just blessed. **An earlier version of this row said forward `apply_edit` never produces the shape; C2b proves all three forward empty forms do**, so the doc it produced was false and is corrected in revision 5 |
| C8 | **the fixture's own doc comment is corrected**, not just its attribute | rewrite `buffer.rs:3005`–`:3040`: convergence is **established by C3**, not "verified" (`:3023`–`:3026`); the buffer-end range is **ruled and weakly preferable** per §4, not "genuinely arbitrary" (`:3034`–`:3036`); "**The open question**" (`:3030`) becomes the ruling; and the `#[ignore]` reason string (`:3042`–`:3043`) goes with the attribute | leave the comment → the repository's most-read record of this defect still says the decision is open and that convergence was already checked, contradicting §1, §2 and C3 |
| C9 | §4's census stays closed **for in-tree implementations** | walk `CARGO_MANIFEST_DIR/src` and assert the set of **`(file, impl target)` pairs** carrying a non-`#[cfg(test)]` `fn on_edit` override is exactly `{(syntax.rs, ParseView), (text_view.rs, TextView), (fold.rs, FoldStoreTranslator), (overlay.rs, BufferStyleSpanTranslator)}` — pairs, not file set and count, and by name rather than line number | **replace `ParseView`'s override with an unclassified type in the SAME file** → file set and count are both unchanged, and only the pair set catches it. Adding a fifth override anywhere under `src/` fires it too, naming the file and the type |

**Why C4 was rebuilt.** Revision 3's C4 claimed that deleting the fold
or style guard would make an assertion fire. **Both mutants survive**,
and the arithmetic says why: the history edit sits at the buffer end,
so with `old_start == old_end == len` and `old_len == new_len == 0`,
`BufferStyleSpanTranslator` emits a left fragment `[s, min(e, len))`
for every span within the buffer and no right fragment — the vector is
unchanged with or without the guard (`overlay.rs:269`–`:285`). The fold
store's remaining arithmetic is identity for the same reason. **At this
location the fold guard is an optimization, not a behaviour
discriminator, and no mutation is claimed for it.** What discriminates
is whether the broadcast happens at all (C4a) and whether the style
guard holds where the fragmenting is reachable (C4c).

*(A span of zero width at exactly `len` would be dropped without the
guard and kept with it. That is not used as a witness: whether such a
span is constructible is unestablished, and a witness resting on a
degenerate value is a worse instrument than the interior injection.)*

**C2b's mutant is compound because two of three variants mask it, and
the asymmetry is measured.** `apply_to_crdt_then_normalize_bytes`
returns `(None, None)` early for an empty `Insert` (`:1177`–`:1182`)
and an empty `Delete` (`:1192`–`:1194`), so flipping those arms alone
still yields `crdt_op == None` and the simple mutant survives. The
empty `Replace` has **no** early return — `:1208` skips the delete,
`:1211` skips the insert, and control falls through to the
unconditional `Some(crdt_op)` at `:1231`–`:1236` — so there, and only
there, the simple mutant dies. C2a exists so this asymmetry cannot hide
a classifier regression.

**C3's mutation is the point of C3.** Revision 1's version asserted
text equality alone, and an identity-replace history op leaves text
unchanged — so **dropping the op passed it**. Version/frontier equality
is what discriminates; the causally dependent op is corroboration on
top.

**C5 is the second half of §1's answer.** §1 makes the shape legitimate
*for history ops*; without a provenance-keyed check there is no
remaining assertion for the forward path, and the invariant would have
been repealed rather than narrowed.

**C1, C6 and C9 exist because a green suite is not evidence that a
suite RAN.** Re-ignoring a fixture, dropping a parameter, and adding an
unclassified consumer are all silent under ordinary assertions. Each
gets a witness that fails on absence rather than reporting it.

## 6. Coherence impact (`COHERENCE.md` §20)

Under the resolution the census confirms — invariant narrowed to
provenance, behaviour unchanged:

- **Journey steps touched: NONE.** No product behaviour changes; the
  work is a test contract, a census, and public documentation.
- **Interaction islands: none added.**
- **Config registry: no entry.**
- **Background work: none started.**

Revision 2 made this section conditional on a census that had not run.
**It has now run, and no consumer is harmed, so the section is
unconditional.**

## 7. What this does NOT do

- **It does not commit the proptest regression seed.** That duplicates
  a deterministic fixture and would make a disputed assertion fail
  permanently rather than occasionally.
- **It does not re-verify content correctness**, which §2 records as
  directly asserted. It *does* newly establish remote convergence,
  which §2 records as only inspected.
- **It does not move the empty range**, and after §4 that is a measured
  result rather than a deferral.
- **It does not audit `intercept_edit`**, a different stage with a
  different contract. The census covers `on_edit` and the direct
  consumers of the history `Edit`.
- **It does not reorder the roadmap.** GUI arc 1b remains the next
  product lane.
