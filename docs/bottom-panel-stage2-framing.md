# Bottom panel Stage 2 — the GPU panel band (framing)

**Revision 1 — pre-implementation. Ground truth: canonical `main` @
`5aa9044`, protocol v20, 2026-07-25.**

Stage 1 (#155, merge `e745068`) gave pmacs window placement, window
parameters, TUI side windows, the divider, and the adopter `display`
opt-in. It deliberately set `FrontendView::panel_capable = false` for
every semantic session, so a GPU frontend silently falls back to the
non-side target. **Stage 2 flips that bit and earns the right to.**

This document is the re-framing `docs/bottom-panel-framing.md` (rev 4)
§2 requires before Stage 2 is implemented. It does **not** restate the
parent's decisions. It records what the re-scout against current `main`
found: which anchors moved, which parent claims survived, which did
not, and the four questions the parent framing cannot answer without a
decision from the user.

Read the parent framing's Q#BP8, Q#BP9, Q#BP14b, Q#BP15, Q#BP15a,
Q#BP16, and Q#BP17 alongside this. Those decisions stand except where
§4 below revises them.

## 0. Why the re-scout was required

The parent framing's Stage 2 sections were written against `main` @
`0dd16a5` and last re-scouted at `47581f4`. Since then eleven PRs have
merged: #149/#150 (folding Stage 2), #152–#155 (through bottom-panel
Stage 1), #158–#166 (inline math, minimap, Lean 4 Stages 1–2,
COHERENCE.md, find-file, the dired framing and Stage 1, the GPU
terminal input fix). Every source anchor Stage 2 depends on has moved.

Two things did **not** change, and both are load-bearing:

- **Protocol is still v20.** `PROTOCOL_VERSION` is `20`
  (`pmacs-protocol/src/message.rs:1568`); no intervening PR bumped it.
  Q#BP9's conditional resolves: **Stage 2 is v21**.
- **Both byte pins are still the final variants.**
  `InstanceMessage::InitialTargetResult` is last in its enum
  (`message.rs:577` within the enum at `:569`), and
  `FrontendEvent::TerminalPointer` is last in its own. Q#BP9's
  append-plus-pin instruction applies verbatim, with no re-derivation.

## 1. Anchor re-scout

Every line reference Stage 2 inherits, re-verified. "Claim" is the
parent framing's assertion about that site; "verdict" is what the code
at `5aa9044` actually says.

| Parent anchor | Now at | Claim | Verdict |
| --- | --- | --- | --- |
| `src/editor.rs:2833` `paint_frame` returns cursor separately | `src/editor.rs:3171` | cells alone lose the caret | **Holds.** Signature still returns `Option<CellCoord>` |
| `src/editor.rs:2883-2935` cursor-visible prep | `src/editor.rs:3249+` | extract with the per-window body | **Holds**, but see §3.1 — Stage 1 inserted work *above* it |
| `src/editor.rs:2937-3040` per-window paint body | after `:3260` | origin-agnostic `Viewport<'a>`, extractable | **Holds** |
| `src/editor_core.rs:566` `fold_map_for_window` | `src/editor_core.rs:734` | gates on the **active** frontend | **Holds** — `:738` is `if !self.fold_projection_active()` |
| `src/window.rs:339` stale invariant comment | `src/window.rs:562` | "a semantic session never enters `paint_frame`" | **Holds, still stale.** Updating it remains a Stage 2 obligation |
| `src/statusline.rs:634` indirect `view.active` read | `src/statusline.rs:629`, `:644`, `:675` | one read to close | **Revised: three sites**, not one |
| `src/daemon.rs:3122-3130` grid-only `Mouse` | `src/daemon.rs:3123` | `Mouse` is contractually the grid path | **Holds** |
| `pmacs-gpu/src/attach.rs:420-429`, `:573-577` | `pmacs-gpu/src/attach.rs:577` | permanent `24×80` placeholder | **Holds**, single site now |

Nothing in the parent's mechanical model was falsified by the
re-scout. The decisions in §4 come from what Stage 1 *added*, not from
anything Stage 2 got wrong.

## 2. What Stage 1 already built for Stage 2

More than the parent framing anticipated, which shrinks Stage 2 and
changes one of its wire contracts.

- **`DeclaredFrameGeometry { geometry_epoch: u64, total: CellSize }`
  exists** (`src/window.rs:522-528`), stored as
  `FrontendView::frame_geometry: Option<_>` (`:589`) with `None`
  meaning **unknown** — exactly Q#BP15a's "unknown is first-class".
- **The declaration path exists.**
  `EditorState::sync_frame_geometry` (`src/editor.rs:877-882`) calls
  `EditorCore::declare_frame_geometry` then `reconcile_panel_layout`.
  Two daemon sites already drive it, both gated on
  `panel_capable_for` (`src/daemon.rs:1882-1883` at attach,
  `:1972-1973` on resize).
- **`paint_frame` itself declares geometry** (`src/editor.rs:3187`),
  before the statusline fan-out and before the long mutable core
  borrow, with a comment naming Q#BP2b/Q#BP15a.
- **`StatuslineEvaluationTarget`** (`src/statusline.rs:212-226`) is
  already a two-variant enum — `Grid { frontend_id }` and `Semantic {
  frontend_id, declared_buffer }`. Q#BP8's "generalize the fan-out"
  is an added variant, not a refactor of a concrete type.
- **`primary_document_window`** exists (`src/editor_core.rs:2830`).

## 3. What the re-scout found

### 3.1 The geometry epoch is allocated daemon-side; the wire contract says frontend-side

This is the one genuine conflict between landed Stage 1 and framed
Stage 2, and it needs a decision before implementation.

`declare_frame_geometry` (`src/editor_core.rs:3155-3172`) **allocates
the epoch itself**:

```rust
let next = view
    .frame_geometry
    .map_or(1, |geometry| geometry.geometry_epoch.saturating_add(1));
```

Q#BP15a specifies the opposite: `FrontendEvent::FrontendCellGeometry {
frontend_id, geometry_epoch, total }` carries a **frontend-owned**
declaration id, and `PanelResizeRows` / `PanelPointer` / every
`Present` echo it. Under the landed code the daemon would have to
either ignore the wire epoch (breaking the echo contract the GPU
validates against) or overwrite its own allocator for panel-capable
semantic frontends only (two allocation regimes for one field).

Two further details of the landed allocator matter:

- **It dedups on value.** The function returns early when `total` is
  unchanged, so the epoch advances only on an actual size change. For
  a grid frontend that is correct — cells are the unit, and an
  unchanged grid means an old `PanelFrame` is still valid under the new
  metrics. For a frontend that *owns* its epoch, the daemon cannot
  dedup by value without discarding a declaration the frontend already
  considers current.
- **`saturating_add` is neither wrapping nor fail-closed.** Q#BP15a
  requires exhaustion to "fail closed rather than wrap". Saturation
  pins the epoch at `u64::MAX`, after which two different geometries
  share one id — the exact staleness confusion the epoch exists to
  prevent. Unreachable in practice; wrong as a contract, and free to
  fix.

**Q#BP2S1 (new, needs a decision).** Three candidate resolutions:

1. **Frontend-owned, as framed.** `FrontendCellGeometry` carries the
   epoch; the daemon stores it verbatim for semantic panel-capable
   frontends and rejects a lower-or-equal epoch carrying different
   data. Grid/LOCAL keep the local allocator, which never collides
   because those frontends never send the event. Cost: one field, two
   provenances, documented.
2. **Daemon-owned, GPU echoes.** `FrontendCellGeometry` carries only
   `total`; the daemon allocates and the GPU learns its current epoch
   from the next `PanelFrame`. Simpler invariant, but it reintroduces a
   first-open ordering problem — the GPU must send `PanelResizeRows`
   and `PanelPointer` carrying an epoch it has not been told yet, so
   the first gesture after a resize is unvalidatable and must be
   dropped.
3. **Frontend-owned everywhere.** Grid/LOCAL synthesize an epoch at
   their existing declaration sites and the allocator moves out of
   `EditorCore` entirely. Most uniform; largest Stage 1 churn, and it
   touches code #155 just stabilized.

**Recommendation: option 1.** It preserves the parent framing's
validation chain intact, and the "two provenances" cost is one doc
comment on a field that already carries three.

### 3.2 The §1.3 census is essentially unrouted

The parent framing's §1.3 lists 23 transitive active-context reads that
must route through `primary_document_window` before a panel can hold
focus without corrupting the document mirror. Stage 1 created the seam
but routed almost nothing through it: `primary_document_window` has
**three** references in `src/`, one of which is its own definition and
one a doc-comment link. The single production caller is
`src/daemon.rs:1639`.

For scale, `src/*.rs` still contains ~80 non-test direct `.active`
reads (excluding `active_frontend`, setters, and predicates), on top of
the `active_window*` / `active_buffer*` helper family at
`src/editor_core.rs:663-967`.

This is not a defect in Stage 1 — with `panel_capable = false` for
every semantic session, no semantic frontend can hold a side window, so
the unrouted reads are unreachable from the GPU. It does mean **the
census is the bulk of Stage 2's work**, not a tidy-up at the end, and
the stage plan in §5 sequences it first.

### 3.3 The statusline read is three sites, not one

Q#BP8 says closing the indirect `view.active` read at
`src/statusline.rs:634` falls out of the target generalization. There
are three: `:629` and `:675` compute `active: window_id ==
view.active`, and `:644` does `.get(&view.active)`. They are the same
concern, but a fix that closes one and leaves two is a live risk, and
the acceptance criterion should name all three.

### 3.4 Scout obligations still open

Stated plainly rather than papered over. These were not re-verified in
this pass and must be before the doc leaves revision 1:

- `pmacs-protocol/src/terminal.rs`'s validator internals, which Q#BP15
  asks to factor into a shared parameterized wire-cell-grid validator
  (the `MAX_TERMINAL_ROWS/COLS = 512` split).
- `pmacs-gpu/src/attach.rs`'s bounded outbox policy and its existing
  tail-coalescing classes, which Q#BP15a asks to extend with two new
  classes and Q#BP16 with two more.
- The GPU-side band renderer and where it clips against the status
  band — Q#BP15a's pixel formula is stated but its inputs
  (`status_band_height_px`, `TEXT_TOP_px`, `code_line_height_px`,
  `resolved_monospace_advance_px`) were not located in this pass.
- Whether folding Stage 3 lands first. Both stages touch the semantic
  projection, and the ledger's standing rule is that whichever is
  framed second re-scouts the other's landed state.

## 4. Revisions to the parent framing

Only these. Everything else in Q#BP8/9/14b/15/15a/16/17 stands.

- **Q#BP9 resolves to v21.** No reservation was taken; none was needed.
- **Q#BP15a's epoch ownership is reopened as Q#BP2S1** (§3.1).
- **Q#BP8's statusline criterion names three sites** (§3.3).
- **Q#BP17's stale comment is at `src/window.rs:562`**, and its text is
  now embedded in a longer `fold_projection` doc block that also
  explains the Stage 2/Stage 3 split — the edit is a paragraph rewrite,
  not a one-line correction.

## 5. What ships, in order

Sequenced so each step is independently gateable and the census — the
riskiest part — lands before anything depends on it.

1. **Route the census.** Every §1.3 read through
   `primary_document_window`, with `panel_capable` still `false`. No
   wire change, no behavior change for any existing frontend; pure
   seam adoption, falsifiable by revert.
2. **Extract the per-window painter.** Lift `paint_frame`'s per-window
   body plus the active-window cursor-visible preparation into a
   function taking the fold map as a **parameter** (Q#BP17), leaving
   `sync_frame_geometry` and the statusline fan-out where Stage 1 put
   them. Grid rendering must be byte-identical.
3. **Protocol v21.** Append `InstanceMessage::PanelFrame` and
   `FrontendEvent::{FrontendCellGeometry, PanelResizeRows,
   PanelPointer}`, each with a byte pin on the current final variant.
   Factor the shared cell-grid validator. Gated both directions.
4. **Daemon-side panel projection.** `PanelFrame` production,
   presentation epochs, `Absent` authority, the third statusline
   target, the focus-chrome pass (Q#BP14b).
5. **GPU band.** Geometry declaration, band paint, divider chrome,
   document clip, pointer transport, and the `panel_capable = true`
   flip for semantic sessions — the flip is last, and it is what makes
   the whole stage observable.

Steps 1 and 2 are reviewable without any protocol change and could ship
as a separate PR if the user prefers a smaller first review. **That is
one of the questions in §8.**

## 6. Coherence impact (per `COHERENCE.md` §20)

- **Journey steps touched:** none directly. Stage 2 does not add a
  journey step; it removes a frontend-dependent *hole* in one. Today a
  user who runs `pmacs --gpu` and triggers compile, grep, references,
  or a terminal gets the non-side fallback — the panel silently becomes
  a stolen window. Every journey step that ends in an output surface
  behaves differently on GPU than on TUI, and Stage 2 is what closes
  that.
- **Interaction islands added: none, and this is a reduction.** §6
  grades islands "weak, and growing by one island per modal feature".
  The panel is the opposite move: `display = "panel"` is one adopted
  policy across listview, compile, and terminal, and Stage 2 extends
  the existing policy to a second frontend rather than adding a
  parallel GPU-only surface. The focus-chrome routing table (Q#BP14b)
  deliberately reuses the existing `SearchPrompt` / `MenuPrompt` /
  `CompletionPopup` messages instead of minting panel-specific ones.
- **Config registry adoption:** inherited, not extended. Stage 1's
  `window.panel-height` and `window.min-height` already live in the
  registry; Stage 2 adds no new user-facing option. If the GPU needs a
  band-specific preference, it enters the registry — no new
  configuration mechanism.
- **Background-work attribution:** unchanged. Stage 2 introduces no
  worker, task, or process. It does, however, make §9's "✓ mechanics /
  ✗ visibility" gap materially cheaper to close on the GPU: a terminal
  PTY appearing in no user-visible activity view (§9) is partly a
  *placement* problem, and after Stage 2 both frontends have a place to
  put one.
- **Section this serves:** `COHERENCE.md` §14, which already records
  the panel primitive as landed for Stage 1 and names "Stage 2 (GPU
  band) pending its own framing" as the open item. This is that doc.

## 7. Acceptance criteria (draft)

Numbered for review; each must be falsifiable by revert, per the
standing lesson that a guard with no production caller passes every
direct-call test.

1. Every §1.3 census read resolves through `primary_document_window`;
   asserted at the outermost user-reachable seam, not by direct call.
2. All three `src/statusline.rs` active reads route through the new
   frontend-layout target.
3. Grid `paint_frame` output is byte-identical across the extraction.
4. A semantic frontend with `fold_projection = false` painting a panel
   over a folded buffer shows **every** source line (Q#BP17), and the
   panel path never calls `fold_map_for_window`.
5. v21 round-trips; v20 peers negotiate without the panel events; each
   extended enum's previous final variant is byte-pinned.
6. `Absent` is emitted on both close and hide, and clears input
   authority before any later event validates.
7. A `PanelPointer` failing any of Q#BP16's six checks mutates no view,
   controller, selection, menu, or PTY.
8. A geometry epoch change makes an older `PanelFrame` non-painted and
   non-hit-testable until a matching `Present` arrives.
9. A panel wider than 512 columns is legal; a panel exceeding the
   shared wire budget fails closed to `Absent`, not to a partial frame.
10. Panel focus does not disturb the document mirror
    (`BufferSnapshot`, `CursorByte`, `Viewport`).
11. Native popups clear on document→panel focus change and panel
    popups clear on panel→document, per Q#BP14b's ordering.

## 8. Questions for the user

1. **Q#BP2S1 epoch ownership** (§3.1) — recommendation is option 1,
   frontend-owned with the daemon storing verbatim. Confirm or pick
   another.
2. **One PR or two?** Steps 1–2 (census + extraction) are protocol-free
   and independently valuable; steps 3–5 are the wire and the band.
   Splitting gives two small reviews instead of one very large one, at
   the cost of a second round-trip.
3. **Ordering against folding Stage 3.** Both touch the semantic
   projection. Framing bottom-panel Stage 2 first means folding Stage 3
   re-scouts against a landed band. Confirm that order.
4. **Scout obligations in §3.4** — should revision 2 close all four
   before implementation, or is the GPU-side pixel formula (the largest
   of them) allowed to be settled during implementation?

## 9. Gates

The standing suite, plus `bottom_panel_stage1_acceptance` and a new
`bottom_panel_stage2_acceptance`, the three vterm suites (the panel
hosts terminals), folding Stage 2's 48 (shared projection), and
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`. Protocol round-trip and
byte-pin tests ride step 3.
