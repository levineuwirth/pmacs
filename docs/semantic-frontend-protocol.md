# Semantic frontend protocol (design note)

**Status: implemented by the M11 arc (M11.1–M11.5), post-v1.0.**
This note was written as a post-v1.0 design draft — recorded so the
v1.0 tag was a conscious decision point — and has since been built
out. It originally concluded the v1.0 wire was *already safe* for
this direction: the capability/version scaffolding in `protocol.rs`
(`SUPPORTED_PROTOCOL_VERSIONS`, `negotiate_capabilities`,
per-session outgoing filters) made the work a non-breaking later
addition, mechanically identical to the M10.5–M10.10 CRDT rollout.
It was, and the rollout matched the plan. Implementation status
against this design:

- **M11.1** — wire + capability scaffolding (`semantic_render`,
  `PROTOCOL_VERSION` 3, the `SemanticFrame` variant family,
  `FrontendEvent::Viewport`).
- **M11.2** — the instance-side projection seam
  (`SemanticRenderState`), selected per session; `StyleSpans`.
- **M11.3** — `Decorations` from diagnostics + selection.
- **M11.4** — `full` + dirty-segment diffing.
- **M11.5** — the headless `SemanticClient` glue + reconstruction-
  equivalence and end-to-end tests.

Post-M11 producer arc (the LSP feature arc landed the missing data
sources, so the "wire in when those features land" promise came due):

- **`StyleSpans`, second authority** — policy A, *per-language*
  styling authority. A grammar-backed language stays tree-sitter
  only; a language with no bundled grammar (C/C++, …) is styled
  from LSP semantic tokens (`lsp_scoped_style_spans`, encoding +
  legend via `LspManager::semantic_style_context`). Never both on
  one buffer. Reuses the M11.4 diff pipeline unchanged.
- **`InlineAdornments`** — produced from the LSP inlay-hint store
  (`scoped_inline_adornments`). The wire variant carries no
  `generation`/`full`/`segments`, so suppression is M11.2-level
  (whole-set re-send on change, nothing when byte-identical, never
  an empty frame).
- **`FileStyleSummary`** — resolves Open Q#2 (minimap / whole-file
  overview). Per-line dominant style for the whole buffer,
  generation-keyed so an idle buffer pays nothing
  (`file_style_summary_msg` / `scoped_file_summary`). Reuses
  `scoped_style_spans`, so policy A's authority pick is inherited
  automatically.

`BlockAdornments` / `FoldState` / `ResourceOffer` remain declared
but deliberately unproduced — pmacs still has no blame / lens /
fold / diff source; their producers wire in when those features
land (the same "declared, not yet wired" discipline). Open
questions #3–#5 remain open by design; #1 is consciously deferred
(no real visual-motion command surface or semantic frontend
consumer exists yet).

The grid path (TUI, SSH, a future GPU terminal-grade frontend)
is unaffected by everything here. This note describes a *second*
projection selected per-session by a negotiated capability, in the
VSCode/Zed "two renderers, one core" shape.

## The content-model decision this assumes

The exploration weighed two models: *semantics-down, layout-local*
(Monaco / VSCode-Remote) versus *layout-down, paint-local* (the
Emacs glyph-matrix end-state). This note draughts the first. The
deciding asymmetry: the optimistic-edit + CRDT-replica substrate
v1.0 already shipped (`optimistic.rs`, the `CrdtOp` flow,
`CursorByte`, the per-session multi-frontend dispatcher) is only
useful if the frontend can lay out a speculative edit locally —
which *is* the semantics-down model. Layout-down would leave that
substrate nearly inert. The one cost consciously accepted: a
slice of rendering correctness (shaping, wrap, hit-testing) moves
into a GPU frontend the instance test harness cannot exercise;
the mitigation is golden-testing the *semantic projection*
instance-side (see "Testability").

## Contract boundary

One sentence, because everything below derives from it:

> The frontend owns the viewport and all visual-motion semantics.
> The instance owns the document and all edit/command semantics.

The instance never learns a pixel. Not viewport pixel size, not
DPI, not font metrics, not glyph advances. This is a deliberate
invariant, not an omission: the moment the instance knows pixels,
it is tempted to lay out, and the model collapses toward
layout-down with per-session reflow caches inside a multi-tenant
daemon — the property pmacs's thesis exists to reject. The only
spatial fact the instance learns is *which buffer byte range is
on screen*, so it can scope its projection rather than ship a
100k-line file's styling.

Corollary — there is **no hit-test round trip**. The frontend
resolves pixel→offset locally (it has the layout) and only ever
emits buffer offsets and edits. Click-to-caret latency is local
and therefore zero over SSH. Re-importing a pixel→offset request
into the wire would forfeit the entire latency argument; it is
prohibited by this contract, not merely discouraged.

## Composition with v1.0 primitives

The semantic projection ships **no text**. A `semantic_render`
session is required to also be a text replica — it holds the rope
locally via the existing `crdt_replica` machinery
(`BufferSnapshot` to bootstrap, `CrdtOp` to stay live). The
semantic frame is purely the *interpretation layer* over a buffer
the frontend already has: styling and decoration keyed by byte
range. This mirrors how v1.0 already coupled `multi_frontend`
and `crdt_replica`, and it keeps the new wire tiny — single-digit
KB for a screenful, diffable at span granularity.

Consequently the new surface is small. Cursor reuses the existing
`InstanceMessage::CursorByte` (authoritative cursor as a buffer
offset — added for CRDT optimistic-apply, exactly what a
layout-local frontend consumes). Peer cursors reuse the existing
`PresenceUpdate`. Edits and local cursor travel the existing
`FrontendEvent::CrdtOp` / presence path. The genuinely new wire
is: one capability bit, ~five instance→frontend interpretation
variants, and one frontend→instance `Viewport` variant.

## Capability and version mechanics

Identical pattern to `crdt_replica`:

- New bit `semantic_render` on `FrontendCapabilities` and
  `InstanceCapabilities`, `#[serde(default)]` false — every v1.0
  wire byte still deserializes.
- `negotiate_capabilities` AND-combines it into
  `NegotiatedCapabilities`; mismatch yields the existing
  `Goodbye(CapabilityMismatch { missing: ["semantic_render"] })`.
- `semantic_render` requires `crdt_replica` (text-replica
  dependency above). Negotiation rejects `semantic_render: true`
  with `crdt_replica: false` as a capability mismatch rather than
  silently degrading.
- `PROTOCOL_VERSION` 2 → 3; `SUPPORTED_PROTOCOL_VERSIONS`
  `&[1, 2, 3]`. The slice-membership check already in place means
  v0.1/v1.0 binaries keep connecting unchanged.
- The daemon's per-session outgoing filter gates the entire
  semantic variant family on the negotiated bit. Postcard's
  hard-error on unknown variants is mooted exactly as it is for
  `CursorByte` (M10.10): a non-semantic session never receives a
  variant it cannot decode, because the filter never emits it.

## Instance → frontend: the `SemanticFrame` family

New `InstanceMessage` variants, all gated on negotiated
`semantic_render`, all keyed by `BufferId`, all anchored in
**byte offsets** (consistent with `CursorByte`; line/col is a
rendering concern the frontend derives, CRDT-position is internal
to the replica and not a stable cross-frontend anchor).

A `ByteRange` is `{ start: u64, end: u64 }` (half-open, like the
rope's own ranges).

```rust
/// Syntax + face styling over the frontend's current viewport
/// range. `generation` ties the spans to a CRDT version so the
/// frontend can discard styling that predates an edit it has
/// already applied optimistically.
StyleSpans {
    buffer_id: BufferId,
    generation: u64,
    spans: Vec<StyleSpan>, // { range: ByteRange, style: Style }
},

/// Diagnostics, selection, search hits, current-line, and any
/// other "this region means something" overlay, as offset
/// ranges plus a kind. Peer selection is NOT here — it stays on
/// the existing PresenceUpdate path.
Decorations {
    buffer_id: BufferId,
    decorations: Vec<Decoration>, // { range: ByteRange, kind: DecorationKind }
},

/// Inlay hints, blame, lens, virtual text. Anchored at a single
/// offset with a placement; content is text+style or a resource
/// handle (images, see ResourceOffer). Occupies no document
/// bytes — the frontend interleaves it at layout time.
InlineAdornments {
    buffer_id: BufferId,
    items: Vec<InlineAdornment>,
    // { at: u64, placement: BeforeLine|EndOfLine|AtOffset, content: AdornmentContent }
},

/// Diff zones, folded-region placeholders, anything occupying
/// its own vertical band. Anchored to an offset (the line it
/// precedes/replaces); the frontend allocates the vertical space.
BlockAdornments {
    buffer_id: BufferId,
    items: Vec<BlockAdornment>,
},

/// The instance's authoritative fold set, as document facts.
/// The frontend renders the placeholder and adjusts its own
/// layout. Folding is an instance command-semantics concern
/// (Lua can fold); visual collapse is a frontend layout concern.
FoldState {
    buffer_id: BufferId,
    folds: Vec<ByteRange>,
},

/// Out-of-band content an adornment refers to (images, etc.).
/// Sent once, referenced by handle, so a blame avatar or an
/// inline image is not re-shipped per frame.
ResourceOffer {
    handle: u64,
    mime: String,
    body: ResourceBody, // Inline(Vec<u8>) | Uri(String)
},
```

Each family member diffs against the previous frame the same way
`CellDelta` does today — the instance ships changed spans, not
full re-sends, scoped to the viewport range the frontend last
declared.

## Frontend → instance: `Viewport`

One new `FrontendEvent` variant, gated identically:

```rust
/// The buffer byte range currently on screen, in buffer
/// coordinates. Replaces the instance-derived grid viewport for
/// semantic sessions. `generation` lets the instance ignore a
/// viewport that races a not-yet-applied edit. NO pixels: see
/// the contract boundary invariant.
Viewport {
    frontend_id: FrontendId,
    buffer_id: BufferId,
    visible: ByteRange,
    generation: u64,
},
```

That is the *entire* new frontend→instance surface. Cursor,
selection, edits, focus, paste, detach all reuse existing
variants. There is deliberately no `SemanticResize` and no
hit-test request — both would leak pixels across the contract
boundary.

## Instance-side projection seam

`SemanticRenderState`, a sibling of
`instance_render::RenderState`, reading the same `EditorState`.
`RenderState` rasterizes to cells and exits late; `SemanticRenderState`
exits earlier — it emits the structured ranges the cell painter
would have consumed (tree-sitter spans from `syntax.rs`/
`highlight.rs`, overlays from `overlay*.rs`, diagnostics from
`diag.rs`, LSP adornments from `lsp.rs`/`hover.rs`) without the
grid-packing step. The dispatcher selects the projection
**per session, not per buffer**, so a grid frontend and a
semantic frontend can attach to the same buffer simultaneously —
the M10.8 multi-frontend dispatcher already supports per-session
fan-out; this is a constraint on `SemanticRenderState`, not new
dispatcher work.

## Open questions (deliberately unresolved here)

These are the residue of the responsibility migration; they are
design work, not blockers, and none affect the v1.0 tag.

1. **Soft-wrap-dependent commands.** `move-by-visual-line`,
   `recenter`, `scroll-by-page` assume the instance knows visual
   layout. For semantic sessions it does not. Likely resolution:
   the instance emits *intent* ("recenter the cursor") and the
   frontend interprets against its layout; commands that are
   irreducibly visual become frontend capabilities. Needs a Lua
   API story so package authors see one model, not two.

2. **Minimap / whole-file overview.** *Resolved.* Implemented as
   `InstanceMessage::FileStyleSummary { buffer_id, generation, lines:
   Vec<Style> }` — one dominant style per source line, computed by
   `SemanticRenderState::file_style_summary_msg` over whole-buffer
   spans (policy A inherited from `scoped_style_spans`). Keyed on
   CRDT generation so an idle buffer pays nothing; recomputed only
   on edits. Per-line is the v1 choice because minimap rows
   naturally correspond to code lines. Future refinements if a real
   frontend prefers them: fixed-N bands (smaller wire, coarser) or
   whole-file RLE style runs (highest detail); both straightforward
   given the existing wire shape.

3. **Testability strategy.** Recommended: golden-test
   `SemanticFrame` sequences instance-side (cleaner than golden
   cell grids — they are semantic, not pixel). Accept GPUI's
   layout engine as externally battle-tested. This bounds the
   untested surface to the frontend↔instance glue, not all
   rendering. The existing `audit/` + proptest discipline
   extends naturally to semantic-frame goldens.

4. **Per-byte tree-sitter / LSP style blend.** The producer arc
   chose policy A (per-language authority) deliberately: C/C++ has
   *no* grammar so there is no overlap to resolve, and it reuses
   the M11.4 diff pipeline untouched. A grammar-backed language
   refined by LSP semantic precision (mutable-vs-const, etc.) would
   need tree-sitter-base + LSP-overlay with a per-byte span merge —
   more code and more async-LSP diff churn. Unresolved; deferred
   until a concrete language motivates it.

5. **Multiple servers, one URI.** `SemanticTokenStore::for_uri` and
   `semantic_style_context` pick the lowest-id server when several
   attach to the same document. Blending styling/adornments across
   servers (e.g. a typo linter + a language server) is unresolved;
   the single-representative-server rule is the v1 simplification,
   not a final answer.
