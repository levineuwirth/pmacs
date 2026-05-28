# pmacs-gpu Phase A audit

Date: 2026-05-28

Phase A's framing criterion was not "the GUI is done." It was:
exercise the producer-arc wire shapes against a real consumer, absorb
small shape findings, and score the predicted finding categories.

This audit covers the producer/consumer arc through Sessions 5-8:
`StyleSpans`, `Decorations`, `InlineAdornments`,
`FileStyleSummary`, `CursorByte`, and the pmacs-gpu consumer paths
that render or cache them.

## Evidence

| Session | Surface | Evidence |
|---|---|---|
| 5 | `Decorations` consumption | pmacs-gpu paints diagnostic severities as foreground overrides. Session validation surfaced stale diagnostic styling and byte-position replacement mistakes; the producer now resolves URI by `vp.buffer_id`, suppresses stale diagnostics, and full-resyncs style/deco families on CRDT generation transitions. |
| 6 | `InlineAdornments` consumption | pmacs-gpu inserts LSP inlay hints as zero-width virtual text. `m4_29_real_rust_analyzer_inlay_hints_via_auto_attach` covers rust-analyzer's strict range behavior; the GUI was manually validated with a Rust inlay-hints test file. |
| 7 | `FileStyleSummary` consumption | pmacs-gpu renders the right-side minimap from whole-file dominant line style and local rope structure. Manual validation pushed it from one-color-per-line bars to code-shaped strokes. |
| 8 | Temporal probe | Added a 1000-edit synthetic semantic-render probe that exercises CRDT generation transitions, stale inlay suppression, and fresh inlay rehydration. Also extended `didChange` stale marking to inlay hints. |

The Session 8 automated probe is producer-level and deterministic: it
does not spawn rust-analyzer for all 1000 edits. Real-server coverage
for inlay production remains the PATH-gated rust-analyzer acceptance
test; the stress loop validates the temporal contract that LSP-derived
zero-width adornments must not render while their store is stale.

## Findings

| ID | Finding | Category | Classification | Resolution |
|---|---|---|---|---|
| A1 | Incremental `StyleSpans` / `Decorations` could leave cached byte ranges positioned against pre-edit text after CRDT edits. | Headless-test-blind-spot | Small | Track buffer generation and force `full=true` style/deco resync on generation transitions. |
| A2 | Diagnostics could render against stale post-edit text before the next `publishDiagnostics`. | Headless-test-blind-spot | Small | `DiagnosticStore` stale flag; `did_change_full` marks stale; `publishDiagnostics` clears via `set`; semantic producer suppresses stale diagnostics. |
| A3 | Multi-frontend semantic projection could resolve LSP stores through the editor's active buffer rather than the projected buffer. | Headless-test-blind-spot | Small | URI lookup now routes through `vp.buffer_id` for diagnostics, inlay hints, and LSP semantic tokens. |
| A4 | rust-analyzer rejected over-wide inlay-hint ranges, leaving `InlineAdornments` empty in normal Rust files. | Real-server-strictness | Small | Auto-pull inlay hints over the exact document end; guarded by `m4_29_real_rust_analyzer_inlay_hints_via_auto_attach`. |
| A5 | Inlay hints were not marked stale on `didChange`, so zero-width virtual text could stay anchored to pre-edit positions during sustained typing. | Temporal-interaction | Small | `InlayHintStore` now has stale flags; `did_change_full` marks stale; semantic render emits one empty `InlineAdornments` replacement to clear cached virtual text until fresh hints arrive. |
| A6 | `FileStyleSummary` trailing empty line needed an explicit convention. | Convention-vs-contract | Small | Producer test codifies "final newline creates a trailing empty summary line"; minimap consumes that shape directly. |
| A7 | Initial minimap rendering was visually valid but too coarse to be useful. | Consumer-projection-granularity | Small | pmacs-gpu derives indentation/length strokes from its local rope while using `FileStyleSummary` for color. |
| A8 | Selection/search/current-line backgrounds accumulate in consumer state but cannot render via glyph foreground attributes. | Quad-pipeline-needed | Structural | Deferred to the post-Phase-A wgpu quad pipeline. |

## Predicted vs actual

| Predicted bet | Result | Count | Notes |
|---|---:|---:|---|
| `StyleSpans` / `Decorations` dirty-segment edges at viewport boundaries | Surfaced | 3 | A1, A2, A3. The category was right and under-counted: stale state came from generation, freshness, and buffer identity. |
| `InlineAdornments` suppression flicker / edit-then-revert behavior | Surfaced | 1 | A5. Whole-set suppression is acceptable only when the backing store has freshness state. |
| `FileStyleSummary` trailing-empty-line behavior may need wire decision | Surfaced | 1 | A6. This is now a producer convention, not an open contract question. |
| `CursorByte` per-tick cadence may be wrong for a 60fps consumer | Did not surface as a Phase A fix | 0 | Current shape remains state-notification cadence. Cursor-derived backgrounds are deferred to the quad pipeline, so no new wire frequency was justified in Phase A. |
| `PresenceUpdate` peer color stability may need renderer-side identity discipline | Not exercised | 0 | pmacs-gpu does not yet consume peer cursors. This remains a future multi-frontend GUI surface, not a Phase A blocker. |

Unpredicted categories that surfaced:

| Category | Count | Findings |
|---|---:|---|
| Real-server-strictness | 1 | A4: rust-analyzer exact inlay range requirement. |
| Consumer-projection-granularity | 1 | A7: minimap needed local rope shape, not just per-line color. |
| Quad-pipeline-needed | 1 | A8: foreground-only glyph attributes cannot render background decoration kinds. |

Score summary:

- Predicted categories surfaced: 3 of 5.
- Predicted categories not fixed in Phase A: 2 of 5 (`CursorByte`
  cadence, `PresenceUpdate` identity).
- Unpredicted categories surfaced: 3.
- Small findings absorbed in Phase A: 7.
- Structural findings deferred: 1.

## Phase A close

Phase A can close for the shipped producer-arc families:

- `StyleSpans`: consumed by pmacs-gpu; generation/full-resync and stale
  LSP token suppression are tested.
- `Decorations`: diagnostic foreground rendering works; stale
  diagnostics are suppressed. Background kinds are stored but deferred
  to the quad pipeline.
- `InlineAdornments`: inlay hints render as virtual text; real
  rust-analyzer range behavior and edit-time stale clearing are tested.
- `FileStyleSummary`: minimap consumes the whole-file summary and uses
  local rope shape for useful granularity.
- `CursorByte`: still delivered to semantic/CRDT sessions; no Phase A
  contract change.

Not closed by Phase A:

- Background rectangles for `Selection`, `SearchMatch`,
  `SearchMatchActive`, and `CurrentLine`.
- pmacs-gpu `PresenceUpdate` consumption.
- Producers for `BlockAdornments`, `FoldState`, and `ResourceOffer`.
- Soft-wrap intent beyond the current local frontend implementation.

The next natural step is the wgpu quad pipeline, because it unlocks the
background-bearing decoration kinds already present in the wire and in
pmacs-gpu's cached decoration state.
