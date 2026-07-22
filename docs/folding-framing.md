# Folding — framing (Arc 6)

**Draft Revision 1 — 2026-07-22. Status: framing only, committed to the
`folding` branch for review; no implementation yet.** Branch `folding` is cut
from canonical `main` @ `cac4961` (Vterm Stage 3 #135 merged, atop tab-width
parity #137 and locals-query #134), so Arc 5's terminal stage is complete and
this scout reflects that tree. Expect one to three findings rounds on this
document before implementation; revise the doc, then implement Stage 1 on this
same branch.

## 1. Problem and what ships

Pmacs cannot fold. The `FoldState` wire family was declared in the M11.1
semantic-frontend design but has never been produced — the producer says in
so many words that "pmacs has no instance-side fold source yet," and a test
pins that `FoldState` is never emitted.

Arc 6 gives pmacs a fold engine (instance-side fold model + a fold source +
Lua commands), produces `FoldState`, and renders collapsed regions with a
gutter fold marker in both frontends. It is the roadmap's "keystone gutter
rider": it lights up an already-declared wire family, adds the fold-marker
rider beside the existing diagnostic signs, and is a visible feature.

**Git gutter markers are a SIBLING rider, not this arc.** They ride the same
gutter but need a diff source, which is unrelated to folding. Named as a
deferral (§11), framed separately.

## 2. Ground truth (scouted 2026-07-22, `main` @ `cac4961`)

- **`FoldState { buffer_id, folds: Vec<ByteRange> }`** exists in
  `pmacs-protocol/src/message.rs`, gated on `semantic_render`,
  DECLARED-BUT-UNPRODUCED. Its doc: *"the instance's authoritative fold set as
  document facts. Folding is an instance command-semantics concern (Lua can
  fold); the visual collapse is a frontend layout concern — the frontend
  renders the placeholder and adjusts its own layout."* So the **fold set is
  instance-side state** and the **collapse is frontend layout**.
- `semantic_render.rs::block_adornments_and_fold_state_still_never_emitted`
  asserts `FoldState`/`BlockAdornments` are never sent (not even empty). Stage
  1 flips this test.
- **`BlockAdornments`** (also unproduced) is the declared home for
  "folded-region placeholders." Arc 6 does **not** produce it — the fold
  placeholder is frontend-local (Q#FD7).
- **No fold source exists.** The bundled grammars export
  `HIGHLIGHTS`/`INJECTIONS`/`LOCALS`/`TAGS` queries only — **no fold query and
  no `folds.scm`** in any grammar crate. The roadmap's "tree-sitter fold
  ranges" is therefore not free; the fold source is the load-bearing decision
  (Q#FD1).
- **No frontend consumes `FoldState`.** The TUI drops it in `frontend.rs`'s
  ignored set; the GPU has only a debug label. Both must add consumption.
- **Gutter signs are frontend-derived, not a wire channel.** The GPU's
  `collect_gutter_sign_rects` computes diagnostic sign bars locally from the
  decorations it already holds. Fold markers follow the same model: derived
  frontend-locally from `FoldState`; **no new wire type**.
- **Edit-translation is a solved discipline.** Style spans and decorations are
  already translated through `translate_byte_range` on every optimistic edit
  and reset on `BufferSnapshot`. Fold ranges reuse it, plus an invalidation
  rule when an edit destroys a fold's structure (Q#FD6).
- **Greenfield Lua/commands.** No existing fold surface or commands.

## 3. Fold source (Q#FD1) — the load-bearing decision

The grammars ship no fold queries, so "what is foldable" must be defined by
pmacs. Three options:

- **(A) Curated per-language fold queries** (nvim-treesitter's `@fold`-capture
  model). Highest quality, matches the tree-sitter investment — but per
  language authoring plus ongoing maintenance, and it is exactly the work the
  grammars declined to ship.
- **(B) Structural node folding.** At a point, fold the nearest enclosing
  NAMED node that spans ≥2 display rows, biased to block-like kinds by a small
  shared heuristic on node-kind names (`block`, `body`, `*_list`,
  `declaration_list`, `statement_block`, brace/bracket-delimited nodes).
  Reuses the parse trees already present for every bundled grammar AND every
  injection layer. Zero per-language authoring.
- **(C) Indentation folding.** Fold the maximal run of lines more-indented
  than a header line. Language-agnostic, predictable, works with **no grammar
  at all** (plain text, unknown languages), but ignores syntax.

**Recommendation: (B) structural node folding for grammar-backed buffers as
the v1 engine.** It reuses tree-sitter, needs no per-language work, and folds
the whole set of bundled grammars and injections on day one. (C) indentation
folding is the right **grammarless fallback** but is DEFERRED so Stage 1 stays
scoped to grammar buffers; (A) curated queries are a later **quality pass**,
DEFERRED. This is the honest adjustment to the roadmap's premise: tree-sitter
still drives folding, but via node structure rather than queries that do not
exist.

Open sub-question for review: the block-kind heuristic in (B) is the part most
likely to feel wrong ("it folded the tiny inner block, not the function").
The bet (§10) is that "nearest enclosing block-like node spanning ≥2 rows" is
predictable enough for v1; the fallback if it isn't is (A) for the handful of
Tier-1 languages.

## 4. Where fold state lives (Q#FD2)

Instance-side, per the wire contract. `EditorCore` (or a sibling store) owns a
**per-buffer set of folded byte ranges**. Commands mutate it; the semantic
producer ships it as `FoldState`; each frontend collapses the union of folded
ranges in its own layout. Nested folds are allowed — the store is a set, and
the frontend collapses the union, so a fold inside a fold is just two ranges.

## 5. Fold model semantics (Q#FD3)

- **Head-anchored, byte-range folds.** A fold is `[start, end)` where `start`
  is the byte at the fold-head line's content and `end` is one past the last
  folded byte. The head line stays visible; the interior collapses.
- **Cursor cannot sit inside a fold.** Folding a range that contains point
  moves point to the fold head (Emacs `hs-minor-mode` behavior). Editing
  commands that would enter a fold either skip it or unfold it — Q#FD5.
- **Edits translate folds** through the existing `translate_byte_range`, and a
  fold whose head or tail is destroyed by an edit (e.g. the head line deleted)
  is dropped rather than re-anchored (Q#FD6). Dropping is safe: a fold is a
  view convenience, never data.

## 6. Lua command surface (Q#FD4)

Greenfield, mirroring the comment/kill-ring command style:

- `fold.toggle` — fold the enclosing foldable region at point, or unfold if
  point's line is a fold head.
- `fold.close` / `fold.open` — explicit fold/unfold at point.
- `fold.close-all` / `fold.open-all` — fold every foldable region in the
  buffer / clear the fold set.
- `pmacs.fold` Lua surface: `fold(range?)`, `unfold(range?)`, `folds()`,
  `toggle()` — so Lua can drive folding (the wire contract's "Lua can fold").

Bindings are deliberately left for the review round — Emacs uses `C-x C-z` /
`hs-*` / outline `C-c @`; pmacs has no precedent, so the binding is a decision
for the user, not a default I pick.

## 7. Frontend collapse + gutter marker (Q#FD7)

- **The collapse is frontend-local layout.** The frontend receives the fold
  set and removes folded byte ranges from what it lays out: the TUI skips the
  folded display rows; the GPU excludes the folded bytes from its shaped code
  slice. The head line shows a **placeholder** (e.g. `⋯` or ` ⋯ N lines `) —
  frontend-drawn, **not** a `BlockAdornment` (Q#FD7 keeps `BlockAdornments`
  unproduced).
- **The gutter marker is frontend-derived from `FoldState`**, exactly like the
  diagnostic sign bars: a fold-head line draws an open/closed fold glyph in the
  gutter. No new wire type.
- **Caret/hit-test cross folds.** Clicking or arrowing across a fold-head skips
  the folded bytes; the existing GPU projected↔source maps gain a fold-aware
  step. This is the largest per-frontend cost and is why the frontends are
  separate stages.

## 8. Staging and scope

Mirrors vterm: one useful, independently testable stage per PR.

- **Stage 1 — fold engine (instance).** The fold model, the structural fold
  source, the Lua/command surface, `FoldState` production (whole-buffer,
  diff-suppressed, edit-translated, snapshot-reset), and headless acceptance.
  No frontend rendering — `FoldState` is asserted on the wire, not on screen.
  This is the approval-critical stage.
- **Stage 2 — TUI collapse + gutter marker.** The TUI consumes `FoldState`,
  collapses folded rows, draws the gutter fold glyph and the head placeholder,
  and makes cursor motion fold-aware.
- **Stage 3 — GPU collapse + gutter marker.** The GPU consumes `FoldState`,
  excludes folded bytes from its shaped slice, draws the fold glyph + caret/hit
  fold-awareness, at TUI parity.

Stages 2–3 are sketched here and **re-framed in detail after Stage 1 lands**,
exactly as vterm did. This framing asks approval for the overall architecture
and Stage 1's full detail.

## 9. Numbered decisions

- **Q#FD1** Fold source: structural tree-sitter node folding (v1); indentation
  fallback and curated queries deferred. (§3)
- **Q#FD2** Fold state is instance-side, per-buffer, a set of byte ranges;
  nested folds allowed. (§4)
- **Q#FD3** Head-anchored byte-range folds; point cannot sit inside a fold;
  edits translate, structure-destroying edits drop. (§5)
- **Q#FD4** Command surface `fold.toggle/close/open/close-all/open-all` +
  `pmacs.fold`; bindings decided in review. (§6)
- **Q#FD5** Entering a fold: motion skips it; an edit that targets inside an
  existing fold unfolds it first. (Detail deferred to Stage 2 framing.)
- **Q#FD6** A fold whose head/tail an edit destroys is dropped, not
  re-anchored — folds are view state, never data. (§5)
- **Q#FD7** Placeholder + gutter marker are frontend-local, derived from
  `FoldState`; `BlockAdornments` stays unproduced; no new wire type. (§7)
- **Q#FD8** `FoldState` is whole-buffer (folds are sparse and shift line
  numbers above the viewport), diff-suppressed (cached-compare), and reset on
  `BufferSnapshot` — the established producer discipline. (§8, Stage 1)
- **Q#FD9** Terminal identity buffers (read-only, #135) never fold; the
  producer already suppresses the document family in terminal mode, so no
  special case is needed.

## 10. Bets

- **B1** "Nearest enclosing block-like node spanning ≥2 rows" is predictable
  enough for a v1 fold without curated queries. FALSIFIABLE: if the review
  finds the fold target surprising on real Rust/Python, fall back to curated
  queries for Tier-1 languages.
- **B2** Whole-buffer `FoldState` is cheap enough (folds are a handful, not
  O(lines)); no viewport scoping needed. FALSIFIABLE by a fold-all on a huge
  file — but fold-all produces one range per block, still far below the style
  span volume the producer already ships.
- **B3** The frontend collapse can reuse each renderer's existing
  projected↔source machinery (the GPU already has `translate_byte_range` and a
  projected-run map for adornments) rather than a new layout engine.

## 11. Deferred (named)

- Indentation folding for grammarless buffers (the Q#FD1 (C) fallback).
- Curated per-language fold queries (the Q#FD1 (A) quality pass).
- Persisted folds across sessions (saveplace-style).
- `fold.hide-level N` / outline-style folding by depth.
- Fold-on-open (auto-fold imports/license headers) — needs a policy.
- `BlockAdornments` production (rich placeholders, diff zones, blame bands).
- **Git gutter markers** — the sibling gutter rider; separate diff source,
  separate framing.
- Search revealing folds (a match inside a fold auto-unfolds) — Stage 2+.

## 12. Acceptance — Stage 1 (engine)

1. Structural fold source: at a point inside a multi-line block, the fold
   source returns the enclosing block-like node's byte range; at top level it
   returns the enclosing item; in a grammarless buffer it returns nothing
   (fallback deferred).
2. `fold.toggle` folds the enclosing region, and toggling on a fold head
   unfolds it; `close-all` folds every block-like region, `open-all` clears.
3. Folding a range containing point moves point to the fold head.
4. `FoldState` is produced (the pinned "never emitted" test is replaced by a
   "emitted with the current fold set" test), whole-buffer, diff-suppressed
   (an unchanged fold set sends nothing), and reset on `BufferSnapshot`.
5. An edit inside a folded range translates the fold; an edit deleting the
   fold head drops the fold; both leave a consistent set.
6. Nested folds: folding an inner then an outer region yields two ranges;
   `open-all` clears both.
7. The `pmacs.fold` Lua surface drives all of the above and round-trips
   `folds()`.
8. A terminal identity buffer never produces `FoldState` (Q#FD9).

## 13. Gates (Stage 1)

The standing suite: `cargo fmt --check`; strict workspace Clippy; `cargo test
--lib` and `--features crdt`; the new `tests/folding_acceptance.rs` (default +
CRDT); `cargo test --test m4_acceptance -- --skip basedpyright`;
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`; the workspace sweep; `git diff
--check`. New behavioral acceptance is bite-verified with `scripts/bite`.

## 14. Branch and PR plan

Branch `folding`, worktree `../pmacs-folding`, cut from canonical `main` @
`cac4961`. This framing is its first commit. After the framing is approved,
Stage 1 is implemented on this same branch and opened as the first folding PR.
Stages 2 and 3 are separate branches/PRs off the main resulting from the prior
stage, each with its own detailed framing.
