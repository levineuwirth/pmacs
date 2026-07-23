# Folding — framing (Arc 6)

**Revision 4 — 2026-07-23. Status: framing only, on branch `folding`
(off canonical `main` @ `cac4961`); no implementation.** Rev 1 passed a
ground-truth review; rev 2 fixed round 1's seven findings; rev 3 fixed round
2's five majors and four minors; rev 4 fixes round 3's one major, three
minors, and a nit. See §0 for the per-round changelog.

## 0. Revision history

### Round 1 (rev 1 → rev 2)

- **F1** the grid TUI is daemon-rendered and never receives `FoldState`; its
  collapse is instance-side in the daemon grid renderer. Staging reworked.
- **F2** stored range pinned to a line-aligned interior.
- **F3** the store's translation is the instance-side buffer-attached `View`
  (`BufferStyleSpanTranslator` pattern), not the frontend `translate_byte_range`.
- **F4** stale-tree fold creation refuses.
- **F5** multi-frontend point / edit-vs-fold pinned.
- **F6** explicit-buffer Lua API + validation.
- **F7** authoritative-empty `FoldState`.

### Round 2 (rev 2 → rev 3)

- **R2-1 (major).** The block-kind heuristic picked a **body line** as the
  fold head on indentation grammars — tree-sitter-python's `block` starts on
  the first statement line, so `def foo():` was left above a headless fold
  (verified in review; brace languages escaped only because `{` shares the
  introducer line). Fixed with a **head-selection ascend rule** (Q#FD1, §3),
  a no-op for brace languages. Acceptance 1 now tests both languages.
- **R2-2 (major).** The interactive-vs-programmatic split cannot live in the
  store's `View`: `View::on_edit(&Buffer, &Edit)` (`src/overlay.rs:248`) and
  `Edit` (`src/rope.rs`) carry no source frontend and no "point was inside"
  signal — only optional `crdt_op`. The `View` does **translate + drop only**;
  the **unfold is a pre-edit step at the dispatch/command layer** that knows
  the authenticated frontend and its point (Q#FD5, §5). This is the
  handoff's deferred "origin-pinned `buffer.after-edit` fan-out" gap.
- **R2-3 (major).** "CRDT op = remote = translate" misclassifies GPU typing
  (a GPU user types via CRDT ops but is editing at their own point inside a
  rendered fold). The classifier is the **authenticated source frontend's
  point, not the transport**. Stage 1 implements the unfold for the command
  path; **CRDT-origin unfold is a named Stage 3 obligation** (that is when a
  GPU user can type into a rendered fold). Q#FD5, §5, §8.
- **R2-4 (major).** Q#FD8 recreated the #120 stale-mirror trap: revert drops
  the store, emits `BufferSnapshot`, and resets the producer baseline, so the
  now-empty store is suppressed as "initial empty" and a GPU keeps rendering
  pre-revert folds unless its snapshot arm **clears the fold mirror**. That
  frontend clear is load-bearing; named as a Stage 3 obligation and pinned in
  acceptance 7 (Q#FD8, §5). Same class as [[message-gating-on-active-state]].
- **R2-5 (major).** A line-aligned tail hid non-member text on shared-closer
  lines (`} else {`, `}, [deps])`). Fixed by **keeping a closing-delimiter
  line visible** (Q#FD3, §5); delimiter-less (indentation) nodes still hide
  through their last body line. Decided, not bet.
- **Minors.** (a) unfold is **plural** — every fold containing the point.
  (b) a shared head line (`foo(() => {`) toggles **innermost-first**.
  (c) Q#FD9's rejection reason corrected: `(0,0)` is in-bounds; the reject
  comes from the ≥1-hidden-line rule. (d) Stage 2/3 re-framings must address
  three named interactions (§8): fold-aware `LineNumbers`, visible-line
  viewport/scroll accounting, and hidden-line signs/presence clamp-or-drop.

### Round 3 (rev 3 → rev 4)

- **R3-1 (major).** Rev 3's head-selection ascend was **not** a no-op for
  brace languages: rustfmt wraps long signatures (`fn foo(` / `a: u32,` /
  `) -> bool {`) and puts `{` on its own line under a `where` clause, so
  `block.start_line > parent.start_line`, the ascend fired, and the fold hid
  the wrapped signature — the R2-5 defect class reintroduced one level up.
  Replaced by a **derived head line**: the interior comes from the body node
  alone (closer-aware tail unchanged) and the head is **the line immediately
  above the first hidden line** (§3) — Emacs hideshow / LSP `foldingRange`
  parity; wrapped introducer text now stays visible in both grammar shapes.
  The introducer↔body association survives for **matching and `close-all`
  only**. Acceptance 1 gains wrapped-signature cases.
- **R3-2 (minor).** "Innermost-first" on a shared head line made the outer
  fold unreachable via `fold.toggle` (close inner, reopen inner, forever)
  and allowed zero-visible-change presses. Replaced by **state-aware
  ordering** with an org-TAB-style toggle cycle (§6); acceptance 9 updated.
- **R3-3 (minor).** Stage 1's "command path" is `dispatch_key`
  self-insert/delete only; interactive Lua commands (yank, query-replace,
  comment-toggle) mutate through the Lua mutator path and classify
  programmatic, so their edits land inside a fold without unfolding. Now
  stated as the intended Stage 1 line, and **widening the classifier to
  interactive Lua command contexts is a named Stage 2 obligation** (Q#FD5,
  §5, §8), beside Stage 3's CRDT-origin unfold.
- **R3-4 (minor).** The data API's normalization of an arbitrary range was
  unstated; §6 now defines it (no node, so no introducer or closer
  inference). **Nit:** stored-range containment pinned **start-exclusive,
  end-inclusive** with the matching `View` boundary bias, so typing at the
  end of a head line neither unfolds nor lands hidden (§5; acceptance 6).

## 1. Problem and what ships

Pmacs cannot fold. `FoldState` was declared in the M11.1 semantic-frontend
design but has never been produced — the producer says "pmacs has no
instance-side fold source yet," and a test pins it is never emitted.

Arc 6 gives pmacs a fold engine (instance-side fold store + a fold source +
Lua commands), renders collapsed regions with a gutter fold marker in both
frontends, and produces `FoldState` for semantic (GPU) sessions. It is the
roadmap's "keystone gutter rider."

**`FoldState` needs no protocol bump** — the variant has been in the encoding
since M11.1 and both frontends already decode it (the TUI drops it, the GPU
has a decode arm); Arc 6 only starts *producing* it.

**Git gutter markers are a SIBLING rider, not this arc** (§11).

## 2. Ground truth (scouted 2026-07-22, `main` @ `cac4961`; verified across three review rounds)

- **`FoldState { buffer_id, folds: Vec<ByteRange> }`** —
  `pmacs-protocol/src/message.rs:886`, gated on `semantic_render`,
  DECLARED-BUT-UNPRODUCED. `semantic_render.rs:4002` pins it is never emitted.
- **`BlockAdornments`** (also unproduced) is the declared home for
  folded-region placeholders. Arc 6 does not produce it (Q#FD7).
- **No fold source exists** — the bundled grammars export
  `HIGHLIGHTS`/`INJECTIONS`/`LOCALS`/`TAGS` only, no fold query, no
  `folds.scm`. Fold source is Q#FD1. **tree-sitter-python's `block` node
  starts on the first statement line, not the `def` line** (R2-1), and
  **tree-sitter-rust's `block` starts at `{`, which rustfmt places below the
  `fn` line for wrapped signatures and standalone under `where` clauses**
  (R3-1) — the two facts the derived-head rule answers.
- **Two frontend render paths (F1).** Grid TUI: daemon-rendered
  (`render_states` → `render_state.render_frame`, `src/daemon.rs:1106`),
  advertises `semantic_render: false` (`src/frontend.rs:385`), never receives
  `FoldState`. GPU: semantic session (`semantic_states` → `sem.render_frame`,
  `:1091`), does. Collapse is daemon-side for the TUI, wire-fed for the GPU.
- **Gutter signs are frontend-derived, not a wire channel** — fold markers
  follow suit per path; no new wire type.
- **Instance-side edit translation** is the buffer-attached `View`
  (`BufferStyleSpanTranslator`, `src/overlay.rs:235`; hook
  `on_edit(&Buffer, &Edit)` at `:248`), which sees every edit once,
  provenance-blind (R2-2): `Edit` carries only `crdt_op`, no source frontend.
- **Staleness is detectable** — `ParseViewHandle::current()`
  (`src/syntax.rs:696`) is `None` before first settle; `pending_edit_count()`
  (`:706`) is nonzero while edits await settle.
- **Greenfield** Lua/commands.

## 3. Fold source (Q#FD1) — structural node folding with derived head line and closer-aware tail

The grammars ship no fold queries, so pmacs defines "what is foldable." v1 is
**structural node folding for grammar-backed buffers** (reuses the parse trees
for every bundled grammar and injection layer, zero per-language authoring).
Indentation folding (grammarless fallback) and curated per-language queries
(quality pass) are DEFERRED (§11).

The source, at a point:

1. **Match** the nearest enclosing NAMED node spanning **≥2 source lines**
   (source lines, not display rows — soft wrap is frontend-only and unknowable
   instance-side), biased to block-like kinds (`block`, `body`, `*_list`,
   `declaration_list`, `statement_block`, brace/bracket-delimited nodes).
2. **Resolve introducer↔body (R2-1, R3-1).** If the matched node is an
   introducer — a `function_definition` / `if_statement` / … matched from its
   header lines, whose block-like body child (grammar field `body` /
   `consequence`; feeds B1) starts at or below it — descend to that body
   child. The interior-defining node `B` is the body; otherwise it is the
   matched node itself. `B` is *introduced* when its parent is such an
   introducer. The association exists for **matching and `close-all`
   enumeration only** — `fold.toggle` on `def foo():` or on any
   wrapped-signature line resolves to the body below; it no longer selects
   the head line (rev 3's start-line ascend is removed, R3-1).
3. **Head — the line immediately above the first hidden line (R3-1).** The
   head line is `B.start_line - 1` when `B` is an **introduced,
   delimiter-less body** (a Python `block`: its introducer's header ends on
   the line above — `def foo():`, or the `):` line when the signature
   wraps). Otherwise it is `B.start_line` (a brace body's `{` line —
   normally the introducer's own line; the `) -> bool {` line when rustfmt
   wraps the signature; the standalone `{` under a `where` clause). The
   first hidden line is `head_line + 1`. **Wrapped introducer text always
   stays visible** — rev 3's ascend took the introducer's *start* line as
   the head and so hid wrapped signatures and `where` clauses, the R2-5
   defect class one level up. This is Emacs hideshow / LSP `foldingRange`
   parity: the fold hides the body, nothing else.
4. **Tail — closer-aware (R2-5).** The last hidden line is:
   - if `B`'s last line begins with `B`'s **closing-delimiter token**
     (`}`/`)`/`]`, and `end`-style closers later) — a brace/bracket node —
     then `B.last_line - 1`, **keeping the closer line visible**. This is what
     keeps `} else {` and `}, [deps])` on screen with their trailing siblings.
   - else (a delimiter-less node, e.g. a Python `block`) — `B.last_line`,
     hiding through the last body line.

The stored range is the byte range `[end of head_line, end of last-hidden
line]` (§5). A fold must have **≥1 hidden line**: the ≥2-source-line gate is
a *match* condition on the matched node, foldability is the ≥1-hidden-line
rule on the *normalized* interior. So `fn f() {` / `}` (empty body — the
closer-aware tail leaves nothing between head and closer) is **not
foldable**, while a two-line `def foo():` / `x = 1` **is**: the matched
`function_definition` spans two lines and its one-line body is the interior.

**Stale-tree rule (Q#FD10, F4).** The source reads
`ParseViewHandle::current()`; if it is `None` (no settle yet) or
`pending_edit_count() > 0` (settled coordinates are stale), the fold command
**refuses with a status message and stores nothing** — a fold is durable state
and must not be computed against stale coordinates. Settle is a main-thread
pump, so the window is sub-frame. Translate-through-pending is a §11
refinement.

The block-kind heuristic (step 1) and the body-field bias (step 2) remain a
taste bet (Bet B1); steps 3–4 fixed the *determinable* defects (R2-1, R2-5,
R3-1), which were not taste.

## 4. Where fold state lives (Q#FD2)

Instance-side, per the wire contract. A **per-buffer fold store** (a set of
byte ranges) lives beside the buffer, attached as a `View` (F3). Commands
mutate it; the daemon grid renderer reads it directly (Stage 2); the semantic
producer ships it as `FoldState` to GPU sessions (Stage 3). Nested folds are
allowed; the store is **shared by every attached frontend** (Emacs parity).

## 5. Fold model semantics (Q#FD3, Q#FD5, Q#FD6, Q#FD8)

**Stored range = line-aligned hidden interior (Q#FD3).** A fold is identified
by its **head line** (the line immediately above the hidden interior, §3
step 3), which stays visible with a frontend-drawn ellipsis. The stored byte
range is `[end of head line, end of the last hidden line]`, where the last
hidden line is chosen by §3 step 4 — so a **closing-delimiter line stays
visible** (fixing `} else {`), while a delimiter-less node hides through its
last body line. One normalized form is computed by the source and seen
identically by the store, the grid renderer, the wire, and `folds()`.

**Containment and boundary bias (R3-4 nit).** The stored range is
**start-exclusive, end-inclusive** — `(start, end]`. A point at
`range.start` (the end of the head line) is **outside** the fold: typing
there does not trigger the pre-edit unfold, and the store `View` translates
an insert at exactly `range.start` by shifting the fold right (the
`BufferStyleSpanTranslator` at-or-after bias), so the typed character lands
visible on the head line. A point at `range.end` (the end of the last hidden
line) is **inside**: typing there unfolds. One convention covers both the
containment test and the translation bias.

**Point and folds (Q#FD3, F5).**
- Folding a range containing the **invoking frontend's** point moves that
  point to the head line.
- The store is shared, so another frontend's cursor may sit inside a newly
  folded range. "No cursor inside a fold" is a **per-cursor, render-time**
  invariant (its caret clamps to the head on that frontend's next frame — a
  Stage 2/3 concern). In **Stage 1** there is no motion-awareness, so the
  invariant is **creation-time-only**; acceptance is scoped to that so it
  cannot self-contradict.

**Edits and folds — two separated mechanisms (Q#FD5, Q#FD6, R2-2, R2-3).**
- The store's buffer-attached `View` does **translation and drop only**, and
  is **provenance-blind**: on every edit it translates each fold's range, and
  **drops** any fold whose head/tail the edit destroys or whose interior
  collapses below one line. It cannot unfold-on-typing because `Edit` carries
  no frontend and no point (R2-2).
- **Unfolding on an interactive edit is a pre-edit step at the dispatch layer**
  (which holds the authenticated frontend and its point): before applying an
  edit that a frontend is making at its point, unfold **every** fold
  containing that point (plural — minor a). The classifier is the
  **authenticated source frontend's point, not the transport** (R2-3): a GPU
  user's CRDT-op insert at a point inside a fold is interactive and must
  unfold, even though it arrives as a CRDT op. **Stage 1 implements this for
  the command path** — daemon `dispatch_key` self-insert/delete, the only
  point-anchored edits the daemon applies directly with the frontend + point
  in hand. Two widenings are named, each landing with the rendering that
  makes it user-visible (R3-3):
  - **Interactive Lua command edits — Stage 2 obligation.** Yank,
    query-replace, and comment-toggle mutate through the Lua mutator path,
    which this split classifies as programmatic: in Stage 1 such an edit
    inside a fold translates without unfolding. Invisible while headless,
    but a visible "the yank vanished into the fold" once the TUI collapses —
    Stage 2 widens the classifier to interactive Lua command contexts,
    keyed on the edit position.
  - **CRDT-origin unfold — Stage 3 obligation**, wired when the GPU renders
    folds and a GPU user can type into one.

**Store lifecycle vs producer baseline (Q#FD8, F3, R2-4).** Three coupled
resets, kept distinct:
- The **per-session producer suppression baseline** resets on `BufferSnapshot`
  so the fold set is re-shipped to a (re)joining semantic session.
- The **per-buffer store** is dropped on buffer **content replacement**
  (revert/reload) — its ranges name bytes that no longer exist (revalidation
  is a §11 refinement).
- **The frontend fold mirror must clear on `BufferSnapshot` (R2-4, Stage 3
  obligation).** Revert simultaneously drops the store, emits a snapshot, and
  resets the baseline, so the producer sees an empty store with a fresh
  baseline and correctly suppresses it as "initial empty" — which means the
  GPU keeps rendering pre-revert folds **unless its snapshot arm clears fold
  state**, exactly as it already clears spans/decorations. The
  empty-after-snapshot suppression is correct **only because** the snapshot
  clears the frontend mirror; this pairing is load-bearing and is pinned in
  acceptance 7.

## 6. Lua command surface and validation (Q#FD4, Q#FD11)

**Interactive commands** (resolve to the invoking frontend's active-window
buffer — command context, not ambient resolution):

- `fold.toggle`, `fold.close`, `fold.open`, `fold.close-all`, `fold.open-all`.
- `close-all` folds **top-level** foldable regions only (Emacs `hs-hide-all`
  parity — nested regions are not auto-folded; feeds B2). `open-all` clears.
- **Shared head lines — state-aware ordering (R3-2).** On a head line shared
  by more than one fold (`foo(() => {`), plain "innermost-first" dead-loops:
  toggle closes the inner fold, then acts on it again and *reopens* it,
  forever — the outer fold is unreachable, and opening an inner fold while
  the outer is closed changes nothing on screen. Ordering is therefore
  keyed on fold **state** so every press has a visible effect: `fold.close`
  closes the **innermost open** fold (repeated presses walk outward);
  `fold.open` opens the **outermost closed** fold (repeated presses walk
  inward); `fold.toggle` **cycles org-TAB-style** — it closes the innermost
  open fold until every fold on the head is closed, then one more press
  opens them all.

**Data API (Q#FD4, F6): explicit buffer, no ambient resolution** (matching
#127): `pmacs.fold.fold(buffer, range)`, `unfold(buffer, range)`,
`folds(buffer)`, `toggle(buffer, pos)`.

**Arbitrary-range normalization (R3-4).** A data-API `fold(buffer, range)`
carries no node, so none of §3's introducer or closer inference applies —
the caller names exactly what to hide. The head line is the line containing
`range.start`; the hidden lines are the full lines strictly after it,
through the line containing `range.end` — or through the *previous* line
when `range.end` sits at a line start. The stored form is §5's; validation
then applies.

**Validation (Q#FD11, F6).** `fold(buffer, range)` rejects unless: the buffer
is a normal document buffer; both endpoints are UTF-8 boundaries; and the
range normalizes to **≥1 hidden line**. Q#FD9 (terminals never fold) follows
from the last clause (minor c): `(0,0)` on an empty terminal identity buffer
is technically in-bounds, but it normalizes to zero hidden lines and is
rejected — no special case.

Bindings remain the user's call (Emacs has no single convention).

## 7. Frontend collapse + gutter marker (Q#FD7)

- **Grid TUI — daemon-rendered.** The daemon grid renderer reads the store and
  omits each fold's hidden lines, showing the head line with an ellipsis and a
  gutter fold glyph. No wire (the vterm Stage 2 shape).
- **Semantic GPU — wire-fed.** The GPU receives `FoldState`, excludes the
  hidden bytes from its shaped slice, shows the head + ellipsis + fold glyph,
  and makes caret/hit-test fold-aware. It also clears its fold mirror on
  `BufferSnapshot` (R2-4).

The placeholder is frontend-local (not a `BlockAdornment`); the gutter marker
is derived per path like the diagnostic sign bars — no new wire type.

## 8. Staging and scope

- **Stage 1 — fold engine (instance), headless.** The per-buffer store + its
  translating/dropping `View`; the structural source with derived head line,
  closer-aware tail, and the stale-tree rule; the Lua data API + interactive
  commands + validation; the **command-path pre-edit unfold**; `FoldState`
  production (authoritative-empty, diff-suppressed); headless acceptance. No
  rendering. **Approval-critical.**
- **Stage 2 — grid (daemon-rendered) collapse + gutter marker.** The daemon
  grid renderer collapses folded interiors and draws the TUI gutter glyph +
  head placeholder; caret clamps to the head. **Must also make the
  daemon-computed `LineNumbers` family fold-aware** (skipped lines; relative
  distance measured across a fold), **count visible lines in viewport/scroll
  accounting**, **clamp-to-head-or-drop diagnostic signs on hidden lines**
  (minor d), and **widen the pre-edit interactive unfold to interactive Lua
  command edits** (yank / query-replace / comment-toggle — R3-3).
- **Stage 3 — GPU collapse + gutter marker.** The GPU consumes `FoldState`,
  excludes folded bytes, draws the glyph, makes caret/hit-test fold-aware at
  TUI parity, **clears the fold mirror on `BufferSnapshot`** (R2-4), wires
  **CRDT-origin interactive unfold** (R2-3), and applies the same
  hidden-line rules to **peer-presence rects and line numbers** (minor d).

Stages 2–3 are sketched; each is re-framed in detail after the prior stage
lands. This framing asks approval for the architecture and Stage 1's detail.

## 9. Numbered decisions

- **Q#FD1** Structural node folding: match block-like node ≥2 source lines →
  resolve introducer↔body (matching + `close-all` only) → **head line = the
  line immediately above the first hidden line** (wrapped introducer text
  always visible) → **closer-aware tail** (closing-delimiter line kept
  visible; delimiter-less nodes hide through the last body line);
  stale/absent tree refuses. Indentation and curated queries deferred. (§3)
- **Q#FD2** Fold state is instance-side, per-buffer, a set of ranges, shared
  by all frontends; nested allowed. (§4)
- **Q#FD3** Stored range = line-aligned hidden interior, **start-exclusive,
  end-inclusive** with the matching `View` boundary bias; head line (the
  line above the interior) visible; closer line visible for closer-terminated
  nodes; invoking point moves to the head; no-cursor-inside is per-cursor
  render-time, creation-time-only in Stage 1. (§5)
- **Q#FD4** Interactive commands (invoking frontend's buffer; shared head
  lines use **state-aware ordering** — close innermost-open, open
  outermost-closed, toggle cycles); data API takes an explicit buffer, no
  ambient resolution, with the §6 arbitrary-range normalization; bindings
  decided by the user. (§6)
- **Q#FD5** The store `View` translates + drops only (provenance-blind); the
  **pre-edit interactive unfold** lives at the dispatch layer, keyed on the
  authenticated source frontend's point (not transport), unfolding **every**
  fold containing it; Stage 1 = command path (`dispatch_key`
  self-insert/delete), interactive-Lua-command widening = Stage 2,
  CRDT-origin = Stage 3. (§5)
- **Q#FD6** A fold whose head/tail an edit destroys is dropped, not
  re-anchored. (§5)
- **Q#FD7** Placeholder + gutter marker are frontend-local per path (TUI
  daemon-rendered, GPU wire-fed); `BlockAdornments` stays unproduced; no new
  wire type. (§7)
- **Q#FD8** `FoldState` (semantic sessions only) is whole-buffer,
  authoritative-empty (initial empty suppressed until a fold exists; unchanged
  suppressed; non-empty→empty emits exactly one empty frame); its per-session
  baseline resets on `BufferSnapshot`; the STORE drops on content replacement;
  **the GPU fold mirror must clear on `BufferSnapshot`** or empty-after-revert
  suppression leaves stale folds (#120 class). (§5, §8)
- **Q#FD9** Terminals never fold — from the ≥1-hidden-line validation, not
  from bounds. (§6)
- **Q#FD10** Fold creation against a `None`/stale parse tree refuses. (§3)
- **Q#FD11** `fold(buffer, range)` validates buffer kind, UTF-8 boundaries,
  and ≥1 hidden line after the §6 normalization; rejects otherwise. (§6)

## 10. Bets

- **B1** The block-kind heuristic (§3 step 1) and body-field bias (step 2)
  pick a fold *target* users find natural. FALSIFIABLE on real Rust/Python;
  fallback is curated Tier-1 queries. (The derived head line and the
  closer-aware tail are decided, not bet.)
- **B2** Whole-buffer `FoldState` is cheap: folds are a handful, `close-all`
  is top-level only, so the set is O(top-level blocks). No viewport scoping.
- **B3** Both collapse paths reuse existing machinery (daemon cell painting;
  the GPU's projected↔source map) — no new layout engine.

## 11. Deferred (named)

- Indentation folding for grammarless buffers (Q#FD1 (C)).
- Curated per-language fold queries (Q#FD1 (A)).
- Translate-a-node-range-through-pending-edits so creation need not refuse on a
  stale tree (Q#FD10 refinement).
- Fold-store revalidation against new content on revert/reload (v1 drops it).
- Persisted folds across sessions; `fold.hide-level N`; auto-fold-on-open.
- `BlockAdornments` production (rich placeholders, diff zones, blame bands).
- **Git gutter markers** — the sibling gutter rider; separate diff source.
- Search revealing folds (a match inside a fold auto-unfolds) — Stage 2+.

## 12. Acceptance — Stage 1 (engine)

1. **Head line, both grammar shapes, wrapped headers (R2-1, R3-1).** In Rust
   `fn foo() { … }`, a point in the body folds with head line `fn foo() {`;
   with a rustfmt-wrapped signature (`fn foo(` / `a: u32,` / `) -> bool {`)
   the head is the `) -> bool {` line and **every signature line stays
   visible**. In Python `def foo(): / body`, a point in the body folds with
   head line `def foo():` — **not** a body line; with a wrapped signature
   the head is the `):` line and the signature stays visible. `close-all`
   on each yields the same heads.
2. **Stale/absent tree (Q#FD10).** With `current() == None`, and with
   `pending_edit_count() > 0` after an edit before settle, `fold.toggle`
   refuses and stores nothing; after settle it succeeds.
3. **Commands.** `fold.toggle` folds the enclosing region and unfolds on a
   head; `close-all` folds top-level regions only (a nested inner region is
   not auto-folded); `open-all` clears.
4. **Range semantics (Q#FD3, R2-5).** For a brace node the closing-delimiter
   line is **outside** the stored range (stays visible) and a `} else {` case
   keeps `else {` visible; for a Python node the last body line is **inside**
   the range (hidden). `folds(buffer)` returns exactly the normalized ranges.
5. **Point (Q#FD3).** Folding a range containing the invoking point moves it
   to the head; Stage 1 does not prevent later motion into a fold.
6. **Edits — separated mechanisms (Q#FD5/Q#FD6, R2-2/3, R3-3/4).** The store
   `View` translates a fold across a programmatic edit inside it and drops a
   fold whose head an edit deletes — with no knowledge of source. A
   command-path self-insert at a point inside a fold (or inside **nested**
   folds) unfolds **all** of them before the edit applies. A self-insert at
   the **end of the head line** (`point == range.start`) unfolds **nothing**
   and the fold shifts right — the character lands visible on the head line.
   An interactive Lua-command edit (e.g. a yank) inside a fold translates
   without unfolding in Stage 1; the test documents this as the named
   Stage 2 widening. (CRDT-origin unfold is asserted in Stage 3.)
7. **`FoldState` production (Q#FD8, F7, R2-4).** The flipped pin test asserts
   all three transitions to a semantic session — nothing until a fold exists,
   nothing when unchanged, exactly one empty frame after `open-all` — while
   `BlockAdornments` is still never emitted; the per-session baseline resets on
   `BufferSnapshot`. The test documents that empty-after-snapshot suppression
   is correct only paired with the Stage 3 frontend-mirror clear.
8. **Store lifecycle.** Buffer content replacement (revert) drops the store.
9. **Nested folds (R3-2).** Folding an inner then an outer region yields two
   ranges; `open-all` clears both. On a shared head line: repeated
   `fold.close` closes innermost-then-outer, repeated `fold.open` opens
   outermost-then-inner, and `fold.toggle` cycles close-inner → close-outer
   → open-all — the outer fold is reachable by every command.
10. **Injected layer.** A fold sourced inside an injected layer — a fenced
    code block in a markdown buffer — returns the inner block's range, proving
    the source walks injection layers, not just the root tree.
11. **Lua data API (Q#FD4/Q#FD11, R3-4).** `pmacs.fold.*` with an explicit
    buffer drives the above and round-trips `folds()`; an out-of-bounds,
    non-boundary, or sub-one-line range is rejected — including a range
    whose `end` sits at the start of the line after its head, which
    normalizes to zero hidden lines; a fold on a terminal identity buffer
    is rejected via the ≥1-hidden-line rule (Q#FD9).

## 13. Gates (Stage 1)

`cargo fmt --check`; strict workspace Clippy; `cargo test --lib` and
`--features crdt`; `tests/folding_acceptance.rs` (default + CRDT); `cargo test
--test m4_acceptance -- --skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test
-p pmacs-gpu`; the workspace sweep; `git diff --check`. New behavioral
acceptance is bite-verified with `scripts/bite`.

## 14. Branch and PR plan

Branch `folding`, worktree `../pmacs-folding`, off canonical `main` @
`cac4961`. This framing (rev 1 → rev 4) is its opening commits. Canonical
`main` has since advanced past the base (documentation + tab-width #137);
Stage 1's instance-side scope does not overlap that work — rebase onto
current `main` when implementation starts. After approval, Stage 1
implements on this same branch and opens as the first folding PR. Stages 2
and 3 are separate branches/PRs off the main resulting from the prior
stage, each with its own detailed framing.
