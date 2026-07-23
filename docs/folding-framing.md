# Folding — framing (Arc 6)

**Revision 2 — 2026-07-22. Status: framing only, on branch `folding`
(off canonical `main` @ `cac4961`); no implementation.** Revision 1 passed
a ground-truth review (every scouted claim verified against the tree) but the
reviewer found one architectural mis-framing and six spec gaps. Revision 2
fixes all of them; see §0 for the changelog.

## 0. Revision 2 — review round 1 resolutions

- **F1 (architectural).** R1's Stage 2 said "the TUI consumes `FoldState`."
  It cannot: the grid TUI advertises `semantic_render: false`
  (`src/frontend.rs:385`), so the per-session outgoing filter never sends the
  `FoldState` family to it, and the TUI has no layout of its own — the daemon
  renders its cell grid (`render_states` vs `semantic_states`,
  `src/daemon.rs:875`/`881`; the grid path is `render_state.render_frame(...)`
  at `:1106`). **The TUI collapse is instance-side rendering in the daemon
  grid renderer, reading the fold store directly — no wire.** `FoldState` on
  the wire serves ONLY semantic (GPU) sessions. Staging reworked accordingly
  (§8): Stage 2 is grid/daemon rendering; Stage 3 is the wire-fed GPU.
- **F2.** Stored range semantics pinned: the store holds **line-aligned
  interior** ranges (Q#FD3, §5); the head line stays visible, the interior
  including the closing-delimiter line is hidden. §7 rewritten to match.
- **F3.** The fold store's edit-translation is the **instance-side
  buffer-attached `View`** (`BufferStyleSpanTranslator` pattern,
  `src/overlay.rs:235`), which sees every real edit regardless of source — not
  the frontend-side `translate_byte_range`. The two "resets" are split:
  per-session producer baseline vs per-buffer store lifecycle (Q#FD8, §5).
- **F4.** Stale-tree fold creation pinned: read `ParseViewHandle::current()`
  (`src/syntax.rs:696`); if it is `None` or `pending_edit_count() > 0`
  (`:706`), **refuse with a status message** in v1 (Q#FD10, §3).
- **F5.** Multi-frontend point + edit-vs-fold rules pinned (Q#FD3, Q#FD5, §5):
  the invoking frontend's point moves to the head; interactive-point-inside
  edits unfold, programmatic/remote edits translate; the invariant is
  creation-time-only in Stage 1.
- **F6.** Lua surface takes an **explicit buffer** (no ambient resolution,
  per #127), with full range **validation** (Q#FD4, Q#FD11, §6) — which is
  also what makes Q#FD9 (terminals never fold) hold.
- **F7.** `FoldState` follows the authoritative-empty discipline; the
  non-empty→empty transition (open-all) emits exactly one empty frame
  (Q#FD8, acceptance 4).
- **Minors.** "≥2 source lines" not display rows; `close-all` folds
  top-level only (Emacs `hs-hide-all` parity, feeds B2); an injected-layer
  fold acceptance added; and an explicit note that `FoldState` needs **no
  protocol bump**.

## 1. Problem and what ships

Pmacs cannot fold. The `FoldState` wire family was declared in the M11.1
semantic-frontend design but has never been produced — the producer says in
so many words that "pmacs has no instance-side fold source yet," and a test
pins that `FoldState` is never emitted.

Arc 6 gives pmacs a fold engine (instance-side fold model + a fold source +
Lua commands), renders collapsed regions with a gutter fold marker in both
frontends, and produces `FoldState` for semantic (GPU) sessions. It is the
roadmap's "keystone gutter rider": it lights up an already-declared wire
family, adds the fold-marker rider beside the existing diagnostic signs, and
is a visible feature.

**`FoldState` needs no protocol bump.** The variant has been in the wire
encoding since M11.1 and both frontends already decode it (the TUI drops it,
the GPU has a decode arm); Arc 6 only starts *producing* it. No
`PROTOCOL_VERSION` change, no `SUPPORTED` change.

**Git gutter markers are a SIBLING rider, not this arc.** They ride the same
gutter but need a diff source, unrelated to folding. Named as a deferral
(§11), framed separately.

## 2. Ground truth (scouted 2026-07-22, `main` @ `cac4961`; verified in review)

- **`FoldState { buffer_id, folds: Vec<ByteRange> }`** exists in
  `pmacs-protocol/src/message.rs:886`, gated on `semantic_render`,
  DECLARED-BUT-UNPRODUCED. Doc: *"the instance's authoritative fold set as
  document facts. Folding is an instance command-semantics concern (Lua can
  fold); the visual collapse is a frontend layout concern."*
- `semantic_render.rs:4002`
  (`block_adornments_and_fold_state_still_never_emitted`) asserts
  `FoldState`/`BlockAdornments` are never sent. Stage 1 flips this to assert
  `FoldState` IS produced while `BlockAdornments` stays unemitted (F7).
- **`BlockAdornments`** (also unproduced) is the declared home for
  "folded-region placeholders." Arc 6 does **not** produce it — the fold
  placeholder is frontend-local (Q#FD7).
- **No fold source exists.** The bundled grammars export
  `HIGHLIGHTS`/`INJECTIONS`/`LOCALS`/`TAGS` queries only — no fold query and
  no `folds.scm` (`LanguageEntry` has exactly those fields). The roadmap's
  "tree-sitter fold ranges" is not free; the fold source is Q#FD1.
- **Two frontend render paths, not one (F1).** The grid TUI is daemon-rendered
  (`render_states` → `render_state.render_frame`, `src/daemon.rs:1106`) and
  advertises `semantic_render: false` (`src/frontend.rs:385`), so it never
  receives `FoldState`. The GPU is a semantic session (`semantic_states` →
  `sem.render_frame`, `:1091`) and does receive it. Fold collapse is therefore
  daemon-side for the TUI and wire-fed for the GPU.
- **Gutter signs are frontend-derived, not a wire channel.** The GPU's
  `collect_gutter_sign_rects` computes diagnostic sign bars locally; the TUI's
  are painted daemon-side. Fold markers follow the same model per path — no new
  wire type.
- **Instance-side edit translation is a solved pattern (F3).** Compile-mode's
  `BufferStyleSpanTranslator` (`src/overlay.rs:235`) is a buffer-attached
  `View` that sees every real edit — commands, CRDT ops, LSP workspace edits,
  Lua — once per edit, fragment-preserving. The fold store attaches the same
  kind of `View`. The frontend-side `translate_byte_range`
  (`pmacs-gpu/src/main.rs`) is a *different* thing (the GPU translating its own
  received copies across optimistic edits) and is not the store's mechanism.
- **Staleness is detectable (F4).** `ParseViewHandle::current()`
  (`src/syntax.rs:696`) returns `None` before the first settle and the latest
  settled bundle otherwise; `pending_edit_count()` (`:706`) is nonzero while
  edits await settle.
- **Greenfield Lua/commands.** No existing fold surface or commands.

## 3. Fold source (Q#FD1) — the load-bearing decision

The grammars ship no fold queries, so "what is foldable" must be defined by
pmacs. Three options:

- **(A) Curated per-language fold queries** (nvim-treesitter's `@fold`-capture
  model). Highest quality — but per-language authoring plus ongoing
  maintenance, exactly the work the grammars declined to ship.
- **(B) Structural node folding.** At a point, fold the nearest enclosing
  NAMED node that spans **≥2 source lines** (F-minor: source lines, not
  display rows — soft wrap is frontend layout and unknowable instance-side),
  biased to block-like kinds by a small shared heuristic on node-kind names
  (`block`, `body`, `*_list`, `declaration_list`, `statement_block`,
  brace/bracket-delimited nodes). Reuses the parse trees already present for
  every bundled grammar AND every injection layer. Zero per-language authoring.
- **(C) Indentation folding.** Language-agnostic, works with no grammar,
  predictable, but ignores syntax.

**Recommendation: (B) structural node folding for grammar-backed buffers as
the v1 engine.** Reuses tree-sitter, no per-language work, covers all bundled
grammars and injection layers day one. (C) is the grammarless fallback,
DEFERRED so Stage 1 stays scoped to grammar buffers; (A) is a later quality
pass, DEFERRED.

**Stale-tree rule (Q#FD10, F4).** The source reads
`ParseViewHandle::current()`. If it is `None` (no settle yet) or
`pending_edit_count() > 0` (the settled tree's coordinates are stale relative
to the current buffer), a fold command **refuses with a status message and
stores nothing** — a fold is durable state and must not be computed against
stale coordinates. Settle is sub-frame, so the refuse window is tiny.
Translate-the-node-range-through-pending-edits is a named refinement (§11).

The block-kind heuristic (B) is the part most likely to feel wrong ("it folded
the tiny inner block, not the function"); Bet B1 (§10) states it and names the
fallback (curated Tier-1 queries).

## 4. Where fold state lives (Q#FD2)

Instance-side, per the wire contract. A **per-buffer fold store** (a set of
byte ranges) lives beside the buffer, attached as a `View` (F3). Commands
mutate it; the daemon grid renderer reads it directly to collapse the TUI
(Stage 2); the semantic producer ships it as `FoldState` to GPU sessions
(Stage 3). Nested folds are allowed — the store is a set, consumers collapse
the union.

The store is **shared by every attached frontend** (Emacs parity): folds are a
document-level view fact, not per-window. Per-cursor consequences of that
sharing are pinned in §5 (F5).

## 5. Fold model semantics (Q#FD3, Q#FD5, Q#FD6)

**Stored range = line-aligned interior (Q#FD3, F2).** A fold is identified by
its **head line** H. The stored/shipped byte range is the **hidden interior**:
from the newline that terminates H through the end of the last source line the
folded region spans. So:

- H (with its opener, e.g. `fn foo() {`) **stays visible**, with a
  frontend-drawn ellipsis at its end.
- The interior lines **and the closing-delimiter line** (`}`) are **hidden**
  — the range ends at the end of the line containing the region's last byte.
- The structural source yields a raw node span `[node.start, node.end)`; the
  store **normalizes** it to this line-aligned interior before anything else
  (renderer, wire, `folds()`) sees it. One normalized form, one meaning,
  everywhere — resolving the R1 §5/§7 contradiction.

**Point and folds (Q#FD3, F5).**
- Folding a range that contains the **invoking frontend's** point moves that
  point to the head line H (Emacs `hs-minor-mode`).
- The store is shared, so **another** frontend's cursor may already sit inside
  a newly folded range. "A cursor cannot sit inside a fold" is a **per-cursor,
  render-time** invariant: on that frontend's next frame the caret clamps to H
  (a Stage 2/3 rendering concern). In **Stage 1** there is no motion-awareness
  (deferred to Stage 2/3), so the invariant is **creation-time-only**: folding
  moves the invoking point out, but later motion re-entering a fold is not yet
  prevented. Acceptance is written to that scope so it cannot self-contradict.

**Edits and folds (Q#FD5, Q#FD6, F5).** The store's buffer-attached `View`
(F3) sees every edit:
- An **interactive edit at the invoking frontend whose point is inside a
  fold** unfolds that fold first — you cannot type into hidden text you cannot
  see.
- A **programmatic or remote edit** (a peer CRDT op, an LSP workspace edit, a
  Lua buffer edit) **translates** the fold through the `View`, keeping it
  folded — it is not a person typing into the hidden region.
- A fold whose head or tail an edit **destroys** (e.g. the head line deleted,
  or the range collapses below one hidden line) is **dropped**, not
  re-anchored — a fold is view state, never data.

**Store lifecycle vs producer baseline (Q#FD8, F3).** Two distinct resets,
previously conflated:
- The **per-session producer suppression baseline** resets on `BufferSnapshot`
  so the fold set is re-shipped to a (re)joining semantic session — the
  established producer discipline.
- The **per-buffer fold store** is dropped or revalidated on buffer **content
  replacement** (revert/reload): the ranges describe bytes that no longer
  exist, so revert clears the store (revalidation against the new content is a
  §11 refinement).

## 6. Lua command surface and validation (Q#FD4, Q#FD11)

**Interactive commands** (resolve to the invoking frontend's active-window
buffer — command context, not ambient resolution):

- `fold.toggle` — fold the enclosing foldable region at point, or unfold if
  point's line is a fold head.
- `fold.close` / `fold.open` — explicit fold/unfold at point.
- `fold.close-all` / `fold.open-all` — fold every **top-level** foldable
  region (Emacs `hs-hide-all` parity — nested regions are not auto-folded;
  see B2) / clear the fold set.

**Data API (Q#FD4, F6): explicit buffer, no ambient resolution** (matching
#127's deliberate refusal of ambient-buffer lookup):

- `pmacs.fold.fold(buffer, range)`, `unfold(buffer, range)`,
  `folds(buffer) -> {range,...}`, `toggle(buffer, pos)`.

**Validation (Q#FD11, F6).** `fold(buffer, range)` validates and rejects
otherwise: the buffer exists and is a normal document buffer; the range is
in-bounds; both endpoints are UTF-8 char boundaries; the range normalizes
(§5) to **at least one hidden line**. This validation is what makes Q#FD9
hold: a terminal identity buffer is empty, so every range is out-of-bounds and
rejected — no fold can be stored on a terminal even from Lua, with no special
case.

Bindings are left for this review round — Emacs uses `C-x C-z` / `hs-*` /
outline `C-c @`; pmacs has no precedent, so the binding is the user's call.

## 7. Frontend collapse + gutter marker (Q#FD7)

Two paths (F1):

- **Grid TUI — daemon-rendered.** The daemon grid renderer reads the fold
  store directly and omits each fold's hidden interior from the cells it
  paints, showing H with an ellipsis; it draws the gutter fold glyph on H.
  No wire, same shape as vterm Stage 2's daemon-painted terminal cells.
- **Semantic GPU — wire-fed.** The GPU receives `FoldState`, excludes the
  hidden bytes from its shaped code slice, shows H with an ellipsis, and draws
  the fold glyph on H. Caret/hit-test gain a fold-aware step (the largest
  per-frontend cost, and why the GPU is its own stage).

In both paths the placeholder is **frontend-local** (an ellipsis / ` ⋯ N
lines `), **not** a `BlockAdornment` — Q#FD7 keeps `BlockAdornments`
unproduced. The gutter marker is derived from the fold set per path, like the
diagnostic sign bars — no new wire type.

## 8. Staging and scope

Mirrors vterm; reworked for F1 (the TUI path is daemon-side, not a wire
consumer).

- **Stage 1 — fold engine (instance), headless.** The per-buffer fold store +
  its buffer-attached translating `View`; the structural fold source with the
  stale-tree rule; the Lua data API + interactive commands + validation;
  `FoldState` production for semantic sessions (authoritative-empty,
  diff-suppressed); and headless acceptance. No rendering — folds are asserted
  in the store and on the wire, not on screen. **Approval-critical.**
- **Stage 2 — grid (daemon-rendered) collapse + gutter marker.** The daemon
  grid renderer collapses folded interiors and draws the TUI gutter fold glyph
  + head placeholder; caret handling clamps to H. Instance-side rendering
  work; no wire change.
- **Stage 3 — GPU collapse + gutter marker.** The GPU consumes `FoldState`,
  excludes folded bytes from its shaped slice, draws the fold glyph and makes
  caret/hit-test fold-aware, at TUI parity.

Stages 2–3 are sketched here and re-framed in detail after Stage 1 lands.
This framing asks approval for the architecture and Stage 1's full detail.

## 9. Numbered decisions

- **Q#FD1** Fold source: structural tree-sitter node folding (v1);
  indentation fallback and curated queries deferred. (§3)
- **Q#FD2** Fold state is instance-side, per-buffer, a set of ranges, shared
  by all frontends; nested folds allowed. (§4)
- **Q#FD3** Stored range is the line-aligned hidden interior (head line
  visible, closing-delimiter line hidden); the invoking point moves to the
  head; "no cursor inside a fold" is a per-cursor render-time invariant,
  creation-time-only in Stage 1. (§5)
- **Q#FD4** Interactive commands `fold.toggle/close/open/close-all/open-all`
  (invoking frontend's active buffer); data API `pmacs.fold.*` takes an
  explicit buffer, no ambient resolution; bindings decided in review. (§6)
- **Q#FD5** Interactive edit with point inside a fold unfolds it first;
  programmatic/remote edits translate the fold. (§5)
- **Q#FD6** A fold whose head/tail an edit destroys is dropped, not
  re-anchored. (§5)
- **Q#FD7** Placeholder + gutter marker are frontend-local per path; the TUI
  path is daemon-rendered, the GPU path wire-fed; `BlockAdornments` stays
  unproduced; no new wire type. (§7)
- **Q#FD8** `FoldState` (to semantic sessions only) is whole-buffer,
  authoritative-empty (initial empty suppressed until a fold exists; unchanged
  suppressed; non-empty→empty emits exactly one empty frame), and its
  per-session baseline resets on `BufferSnapshot`. The per-buffer STORE is a
  separate lifecycle, dropped on buffer content replacement. (§5, §8)
- **Q#FD9** Terminal identity buffers never fold — guaranteed by validation
  (empty buffer ⇒ out-of-bounds ⇒ rejected), not a special case. (§6)
- **Q#FD10** Fold creation against a `None` or stale
  (`pending_edit_count() > 0`) parse tree refuses with a message; no fold is
  stored. (§3)
- **Q#FD11** Explicit-`fold(buffer, range)` validates buffer kind, bounds,
  UTF-8 boundaries, and ≥1 hidden line; rejects otherwise. (§6)

## 10. Bets

- **B1** "Nearest enclosing block-like node spanning ≥2 source lines" is
  predictable enough for a v1 fold without curated queries. FALSIFIABLE: if the
  review finds the fold target surprising on real Rust/Python, fall back to
  curated queries for Tier-1 languages.
- **B2** Whole-buffer `FoldState` is cheap: folds are a handful, and
  `close-all` folds **top-level only** (Q#FD4), so the set is O(top-level
  blocks), far below the style-span volume the producer already ships. No
  viewport scoping.
- **B3** The two collapse paths reuse existing machinery: the daemon grid
  renderer already paints cells from instance state (vterm Stage 2), and the
  GPU already has a projected↔source map for adornments — neither needs a new
  layout engine.

## 11. Deferred (named)

- Indentation folding for grammarless buffers (Q#FD1 (C)).
- Curated per-language fold queries (Q#FD1 (A)).
- Translate-a-node-range-through-pending-edits so fold creation need not refuse
  on a stale tree (Q#FD10 refinement).
- Fold-store revalidation against new content on revert/reload (Q#FD8: v1
  drops the store).
- Persisted folds across sessions (saveplace-style).
- `fold.hide-level N` / outline-style folding by depth; auto-fold-on-open.
- `BlockAdornments` production (rich placeholders, diff zones, blame bands).
- **Git gutter markers** — the sibling gutter rider; separate diff source,
  separate framing.
- Search revealing folds (a match inside a fold auto-unfolds) — Stage 2+.

## 12. Acceptance — Stage 1 (engine)

1. **Structural source.** At a point inside a multi-line block, the source
   returns the enclosing block-like node normalized to its line-aligned
   interior; at top level it returns the enclosing item; in a grammarless
   buffer it returns nothing (fallback deferred).
2. **Stale/absent tree (Q#FD10).** With `current() == None`, and with
   `pending_edit_count() > 0` after an edit before settle, `fold.toggle`
   refuses and stores nothing; after settle it succeeds.
3. **Commands.** `fold.toggle` folds the enclosing region and unfolds on a
   fold head; `close-all` folds every top-level block-like region (nested not
   auto-folded); `open-all` clears.
4. **Range semantics (Q#FD3).** The stored/shipped range is the line-aligned
   interior: the head line's bytes are outside it, the closing-delimiter line
   is inside it; `folds(buffer)` returns exactly the normalized ranges.
5. **Point (Q#FD3).** Folding a range containing the invoking point moves it
   to the head; the creation-time-only scope holds (Stage 1 does not prevent
   later motion into a fold).
6. **Edits (Q#FD5/Q#FD6).** An interactive edit at a point inside a fold
   unfolds it; a programmatic edit inside a fold translates it; an edit
   deleting the head drops it; each leaves a consistent set.
7. **`FoldState` production (Q#FD8, F7).** The flipped pin test asserts all
   three transitions to a semantic session — nothing until a fold exists,
   nothing when unchanged, exactly one empty frame after `open-all` — while
   `BlockAdornments` is still never emitted; the per-session baseline resets on
   `BufferSnapshot`.
8. **Store lifecycle.** Buffer content replacement (revert) drops the store.
9. **Nested folds.** Folding an inner then an outer region yields two ranges;
   `open-all` clears both.
10. **Injected layer (§3 injection-coverage claim).** A fold sourced inside an
    injected layer — a fenced code block in a markdown buffer — returns the
    inner block's range, proving the source walks injection layers, not just
    the root tree.
11. **Lua data API (Q#FD4/Q#FD11).** `pmacs.fold.fold/unfold/folds/toggle`
    with an explicit buffer drive all of the above and round-trip `folds()`;
    an out-of-bounds, non-boundary, or sub-one-line range is rejected; a fold
    on a terminal identity buffer is rejected (Q#FD9).

## 13. Gates (Stage 1)

The standing suite: `cargo fmt --check`; strict workspace Clippy; `cargo test
--lib` and `--features crdt`; the new `tests/folding_acceptance.rs` (default +
CRDT); `cargo test --test m4_acceptance -- --skip basedpyright`;
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`; the workspace sweep; `git diff
--check`. New behavioral acceptance is bite-verified with `scripts/bite`.

## 14. Branch and PR plan

Branch `folding`, worktree `../pmacs-folding`, cut from canonical `main` @
`cac4961`. This framing (rev 1 → rev 2) is its opening commits. After approval,
Stage 1 implements on this same branch and opens as the first folding PR.
Stages 2 and 3 are separate branches/PRs off the main resulting from the prior
stage, each with its own detailed framing.
