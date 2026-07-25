# Bottom panel Stage 2 — the GPU panel band (framing)

**Revision 3 — pre-implementation. Ground truth: canonical `main` @
`ccf29e3`, protocol v20, 2026-07-25.**

Stage 1 (#155, merge `e745068`) gave pmacs window placement, window
parameters, TUI side windows, the divider, and the adopter `display`
opt-in. It deliberately set `FrontendView::panel_capable = false` for
every semantic session, so a GPU frontend silently falls back to the
non-side target. **Stage 2 flips that bit, under an exact negotiated
rule, and earns the right to.**

This document is the re-framing `docs/bottom-panel-framing.md` (rev 4)
§2 requires before Stage 2 is implemented. It does **not** restate the
parent's decisions or replace its acceptance criteria. It records the
re-scout against current `main`, closes the four scout obligations
review round 1 required, and fixes what round 1 found wrong.

**Inherited reading, all of which remains authoritative:** parent
Q#BP8 (the band), Q#BP9 (protocol), **Q#BP14 (the primary-document
projection contract and its census classification)**, **Q#BP14a (panel
input gating is per-window)**, Q#BP14b (focus chrome and per-window
overlay routing), Q#BP15 (`PanelFrame` lifecycle), Q#BP15a (three
geometries), Q#BP16 (pointer transport), Q#BP17 (fold projection), and
**parent acceptance criteria 37–55**.

## 0. Revision history

### 0.0 Round 2 (rev 2 → rev 3) — 1 blocking, 2 high, 1 medium, all closed

- **R2-1 (blocker).** Rev 2's "one document-bottom seam" conflated two
  boundaries that must **diverge** once a panel exists. Several sites it
  named are not document-bottom consumers at all: the status-band
  background (`main.rs:5908`) must stay at the physical window bottom,
  the status text buffers (`:3175`, `:3185`, `:6601`, `:6607`) consume a
  *height* and never a bottom coordinate, and status text placement
  (`:7134`) sits inside an unchanged band. §5.3 now splits the single
  value into **three** named boundaries, classifies every existing
  `text_area_bottom` call site, and adds the contrast assertion that
  catches a uniformly-wrong implementation moving both together.
- **R2-2 (high).** `accept_frame_geometry -> bool` cannot distinguish
  *advanced* from *accepted duplicate* from *rejected*. It now returns an
  explicit three-valued result. The exhaustion wording also permitted
  retaining stale geometry, which is not fail-closed: §3.1 now clears the
  authoritative declaration and reconciles to hidden, and adds the
  frontend-side terminal latch.
- **R2-3 (high).** Parent acceptance 52 was assigned wholly to 2A, but
  2A has no semantic panel projection — it can only prove the extracted
  painter accepts an explicit `None`. 52 is now also reasserted in 2B,
  where the contract becomes production-reachable.
- **R2-4 (medium).** §9 names the four touched acceptance suites
  explicitly rather than relying on "standing suite".
- Both §8 open items are decided (§5.3): `BASE_DIVIDER_HEIGHT = 4.0` at
  scale 1.0, and `TEXT_TOP` stays unscaled.

### 0.1 Round 1 (rev 1 → rev 2) — 2 blocking, 3 high, 3 revision points, all closed

- **R1-1 (blocker).** Rev 1 said all 23 census reads route through
  `primary_document_window`. That contradicts Q#BP14, which routes only
  the **Projection** class (#1–#12, #21–#22) that way and leaves focus,
  input, chrome, and bell consumers on their own authorities. Rev 1's
  rule would have broken remote-op validation, `DispatchIdle`,
  presence, focused search/menu/completion routing, and bell ownership.
  §3.2 now restores all four classes; §7's criterion pins them
  separately. The inherited-reading list above gains Q#BP14 and Q#BP14a.
- **R1-2 (blocker).** Rev 1 treated the three `src/statusline.rs`
  active reads as one disposition. Only `:644` selects the wrong
  window; `:629` and `:675` must keep tracking **actual focus**. §3.3
  is rewritten and the criterion states the required behavior instead
  of routing focus away.
- **R1-3 (high).** The `panel_capable` flip needed an exact attach
  rule, not "for semantic sessions". §3.5 states it: **v21-or-later
  negotiated authenticated semantic session only**.
- **R1-4 (high).** Option 1 accepted, but the epoch needed a state
  machine, split APIs, and a fail-closed allocator. §3.1 now carries
  the transition table and the API split. Rev 1's phrasing "rejects a
  lower-or-equal epoch carrying different data" was itself wrong — a
  lower epoch carrying *identical* data is still stale.
- **R1-5 (high).** Rev 1's eleven draft criteria silently omitted
  parent 37–55. §7 now declares the parent list authoritative, maps it
  to 2A/2B, and adds only refinements. The painter-extraction criterion
  pins cursor, `view_top`, and passive-window state, not just cells.
- **R1-6.** All four scout obligations are closed in §5.
- **R1-7.** The coherence statement understated journey impact and
  overclaimed on background work. §6 names journey steps 7–10 and
  narrows the §9 claim.
- **R1-8.** Factual corrections in §1 and §3.2.

## 1. Anchor re-scout

| Parent anchor | Now at | Verdict |
| --- | --- | --- |
| `paint_frame` returns cursor separately (`editor.rs:2833`) | `src/editor.rs:3171` | Holds |
| Cursor-visible prep (`editor.rs:2883-2935`) | `src/editor.rs:3249+` | Holds; Stage 1 inserted work above it (§2) |
| Per-window paint body (`editor.rs:2937-3040`) | after `src/editor.rs:3260` | Holds |
| `fold_map_for_window` gates on the **active** frontend (`editor_core.rs:566`) | `src/editor_core.rs:734`, gate at `:738` | Holds |
| Stale "semantic session never enters `paint_frame`" (`window.rs:339`) | `src/window.rs:562` | Holds, still stale; now embedded in a longer `fold_projection` doc block, so the edit is a paragraph rewrite |
| `Mouse` is contractually the grid path (`daemon.rs:3122-3130`) | `src/daemon.rs:3123` | Holds |
| Permanent `24×80` placeholder (`attach.rs:420-429`, `:573-577`) | `pmacs-gpu/src/attach.rs:577`, single site | Holds |
| Byte pin `InstanceMessage::InitialTargetResult` | `pmacs-protocol/src/message.rs:1145` | Holds — still the enum's final variant |
| Byte pin `FrontendEvent::TerminalPointer` | final variant of its enum | Holds |

**Protocol is still v20** (`pmacs-protocol/src/message.rs:1568`); no
intervening PR bumped it. Q#BP9's conditional resolves: **Stage 2 is
v21**, no reservation was taken and none was needed.

Fifteen PRs merged between the parent's last re-scout (`47581f4`) and
this one: #149, #150, #152–#155, #158–#166. Nothing in the parent's
mechanical model was falsified by any of them.

## 2. What Stage 1 already built for Stage 2

- `DeclaredFrameGeometry { geometry_epoch: u64, total: CellSize }`
  (`src/window.rs:522-528`), held as
  `FrontendView::frame_geometry: Option<_>` (`:589`) where `None` means
  **unknown** — Q#BP15a's "unknown is first-class", already landed.
- `EditorState::sync_frame_geometry` (`src/editor.rs:877-882`) →
  `declare_frame_geometry` + `reconcile_panel_layout`, driven from two
  daemon sites gated on `panel_capable_for` (`src/daemon.rs:1882-1883`
  attach, `:1972-1973` resize).
- `paint_frame` declares geometry itself (`src/editor.rs:3187`), before
  the statusline fan-out and before the long mutable core borrow.
- `StatuslineEvaluationTarget` (`src/statusline.rs:212-226`) is already
  a two-variant enum, so Q#BP8's fan-out generalization is an added
  variant, not a refactor.
- `primary_document_window` (`src/editor_core.rs:2830`) and
  `primary_document_buffer` (`:2845`).

## 3. Findings and decisions

### 3.1 Q#BP2S1 — epoch ownership, resolved: frontend-owned, with an exact state machine

**Decision: option 1.** The epoch is owned by the frontend for
negotiated semantic-panel sessions. The deciding argument is one rev 1
missed: **a font or scale transaction can require invalidating an old
`PanelFrame` even when the derived `CellSize` is identical.** Daemon
value dedup cannot detect that case, because the cell totals it
compares are unchanged while the pixels behind them are not.

The landed allocator conflicts in three ways
(`src/editor_core.rs:3155-3172`): it allocates the id itself, it
early-returns when `total` is unchanged (value dedup), and it uses
`saturating_add`, which is neither wrapping nor fail-closed — it pins
at `u64::MAX`, after which two different geometries share one id.

**Acceptance rules for a semantic declaration:**

| Incoming declaration | Result |
| --- | --- |
| epoch **greater** than stored | Accept, store **verbatim**, even if `total` is unchanged |
| same epoch, same `total` | Idempotent no-op |
| same epoch, **different** `total` | Reject |
| **lower** epoch, any `total` | Reject |

The last row is deliberate and corrects rev 1: a lower epoch carrying
identical data is still stale and must not be accepted.

**API split.** Two methods, not one method with an optional epoch:

- `declare_frame_geometry(fid, total)` — the **grid/LOCAL** allocator.
  Keeps value dedup (correct there: cells are the unit, and an
  unchanged grid means an old frame is still valid under unchanged
  metrics). Changes from `saturating_add` to **checked** allocation
  with an explicit fail-closed exhaustion arm.
- `accept_frame_geometry(fid, geometry_epoch, total) -> GeometryUpdate`
  — the **semantic** path. No value dedup; applies the table above
  verbatim.

An ambiguous single method with an `Option<u64>` epoch is rejected
explicitly: it would let a future caller silently take the wrong regime.

**The result is three-valued, not a boolean.** A boolean cannot
distinguish the three outcomes the caller must act on differently:

```rust
enum GeometryUpdate {
    /// Epoch advanced: stored verbatim. Run panel reconciliation.
    Advanced,
    /// Same epoch, same total: already current. Do no work.
    Duplicate,
    /// Same epoch with different total, or a lower epoch: stale or
    /// conflicting. Drop the event before any reconciliation.
    Rejected,
}
```

`Advanced` reconciles, `Duplicate` returns without touching panel
state, and `Rejected` drops the event. Collapsing `Duplicate` into
either neighbour is a defect in one direction or the other: folded into
`Advanced` it reconciles on every repeated declaration, folded into
`Rejected` it would log or surface a stale-event condition that never
happened. (If a boolean is kept for a narrower internal caller, it must
be named `advanced`, never `accepted` — `Duplicate` *is* accepted.)

**Initial epoch.** The frontend's first declaration after attach
acceptance carries epoch `1`. `0` is reserved as "never declared" and
is rejected on the wire.

**Exhaustion fails closed on both sides, and rev 2's wording did not.**
Saying the panel "stays at its last valid geometry" is not fail-closed:
if the real frame resizes after the allocator is exhausted, the daemon
would keep painting a panel sized to geometry that no longer describes
the frontend.

- **Grid/LOCAL path.** On checked-allocation exhaustion, **clear** the
  authoritative `frame_geometry` (back to `None` = unknown) and
  reconcile. Unknown is already non-presentable under Q#BP2b, so the
  panel hides. Stale geometry is never retained.
- **Frontend path.** On exhaustion the frontend sets a **terminal
  latch** for the life of the session: it sends no further geometry,
  and — critically — an old matching `Present` **cannot** make the band
  reappear, because the latch suppresses paint and hit-testing
  independently of frame validity. Only a fresh session (reconnect)
  clears it. Without the latch, a retained `Present` whose epoch still
  matches the last declaration would resurrect a band under geometry
  the frontend has disowned.

### 3.2 The census is classified, and it is mostly unrouted

**Correction to rev 1.** Q#BP14 routes only the **Projection** class
through `primary_document_window`. Rev 1's "all 23 reads" was wrong and
would have broken five subsystems. The four classes, restored:

| Class | Census items | Authority |
| --- | --- | --- |
| **Projection** | #1–#7, #9, #10, #12, #21, #22 | `primary_document_window` / `primary_document_buffer` |
| **Projection + focus** | #8 (document `Pointer`), #11 (full-window `TerminalPointer`) | Align the primary document window **and then activate it** — the one place the two legitimately move together |
| **Focus / input** | #13 (remote-op validation), #14 (`dispatch_idle_for`), #15 (presence), #23 (remote-op application) | The frontend's **actually focused** window. Q#BP14a: gating is per-window, never per-buffer |
| **Focus chrome / surface-routed** | #16–#19 (search, menu, minibuffer, completion) | Q#BP14b's routing table — the currently owned surface, with authoritative clears for the other |
| **Focus / session** | #20 (terminal bell drain) | Per-session counter; the **focused** window chooses which session may drain |

Rerouting any of the last three classes to the document is a defect,
not a simplification: it would break remote-op validation and
application, `DispatchIdle`, presence, focused search/menu/completion
routing, and bell ownership.

**How much is already routed.** `primary_document_window` has **four**
references in `src/` and **two production paths**: directly at
`src/daemon.rs:1639` (#148's initial-target bootstrap, Q#BP11b), and
through `primary_document_buffer` at `src/daemon.rs:2998`, which is
census **#22** and carries a comment naming it. So one census item is
routed and the Projection class is otherwise open. For scale, `src/*.rs`
still holds ~80 non-test direct `.active` reads on top of the
`active_window*` / `active_buffer*` helper family
(`src/editor_core.rs:663-967`).

This is not a Stage 1 defect — with `panel_capable = false` no semantic
frontend can hold a side window, so the unrouted Projection reads are
unreachable from the GPU. It does mean **classified census routing is
the bulk of Stage 2**, which is why it is Stage 2A.

### 3.3 The three statusline reads have two dispositions, not one

All three sites are real, but only one is wrong:

- `src/statusline.rs:644` — `.get(&view.active)` **selects the wrong
  window** when a panel is focused. This is the Projection read (#12).
- `src/statusline.rs:629` and `:675` — `active: window_id ==
  view.active` **must continue tracking actual focus**. Three reasons:
  grid contexts need a truthful `active`; post-callback revalidation
  must notice a focus change; and parent acceptance 42 explicitly
  requires that a document provider may observe `active = false` while
  the panel is focused.

**The new semantic-layout target** therefore captures the **primary
document window plus the visible side window**, marks each context
`active` iff its `window_id == view.active`, invokes each provider
**exactly once**, and **invalidates the entire evaluation** if a
callback mutates layout or focus. Unprojected document splits run no
callbacks (Q#BP8). Route the primary-document result to semantic
`StatuslineSegments` and the side result to the panel mode line.

### 3.4 Fold projection

Unchanged from Q#BP17, with the anchor corrected: the extracted painter
takes the map as a **parameter**; the panel path passes `None` when the
owning frontend's `fold_projection` is false and must never call
`fold_map_for_window`, which gates on the **active** frontend
(`src/editor_core.rs:734`, gate at `:738`) — right for command-time
reckoning, wrong for painting another frontend's panel. The stale
comment is at `src/window.rs:562`.

### 3.5 The `panel_capable` flip needs a negotiated rule

Not "true for semantic sessions". Exactly:

> `panel_capable = true` **only** for an authenticated semantic session
> that negotiated **v21 or later**.

A v6–v20 semantic frontend stays non-panel-capable and takes the
existing Stage 1 fallback: the non-side target with **every
side-specific parameter discarded**, leaving the document window
undedicated (Q#BP2c). "It receives no new events" is insufficient — if
the daemon nevertheless places that frontend's window in a side panel
it cannot render, the window becomes invisible. The gate is on
placement, not only on transport. Parent acceptance 51 pins the mixed
session.

## 4. Revisions to the parent framing

Only these; everything else stands.

- **Q#BP9 resolves to v21.**
- **Q#BP15a's epoch ownership is specified** by §3.1's table and API
  split, replacing the parent's one-line "frontend-owned" statement.
- **Q#BP8's statusline criterion splits** per §3.3: one read reroutes,
  two keep tracking focus.
- **Q#BP17's stale comment is at `src/window.rs:562`**, and parent
  acceptance 52's reference to `:339` should be read against that.

## 5. The four scout obligations, closed

### 5.1 The shared cell-grid validator boundary

`TerminalFrame::validate` (`pmacs-protocol/src/terminal.rs:226`)
currently interleaves both concerns. The exact split:

- **Factored into the shared parameterized wire-cell-grid validator:**
  checked area (the `checked_mul` + `usize::try_from` guard), the
  `MAX_TERMINAL_VISIBLE_CELLS = 262,144` aggregate cap, cell-count
  equality against declared area, cursor-in-bounds, and
  `validate_cells`'s glyph width / continuation topology and aggregate
  glyph-byte checks.
- **Stays terminal-only:** the `MAX_TERMINAL_ROWS/COLS = 512` per-axis
  caps in `checked_area`, `validate_metadata` for title/signal/crash
  text, `validate_selection`, and the `at_bottom == (scroll_offset ==
  0)` coupling.

`PanelFrame` takes the shared half plus its own presence/epoch rules
and does **not** inherit the 512 per-axis cap (Bet B5'), so a 4K
small-font panel wider than 512 columns is legal while the shared area
budget still binds. Parent acceptance 39 pins exactly this.

### 5.2 The GPU outbox needs four more tags

`coalesce_kind` (`pmacs-gpu/src/attach.rs:331`) today returns four
tail-only tags: `Viewport` → 0, `Pointer{Drag}` → 1,
`TerminalPointer{Move}` → 2, `TerminalPointer{Drag}` → 3. Everything
else is `None` = lossless, counting against `OUTBOX_MAX = 8192`.

Stage 2 adds **four distinct tags**: `FrontendCellGeometry` → 4,
`PanelResizeRows` → 5, `PanelPointer{Move}` → 6, `PanelPointer{Drag}`
→ 7. Geometry is latest-wins (epochs need only increase, not be
consecutive); resize drag is latest-wins over the complete event
including its epochs. `PanelPointer` `Down`/`Up`/wheel/context stay
lossless and ordered — repeated left `Down`s are what the daemon click
state reads as a multi-click, and `Down(Right)` is the context-menu
gesture. Tail-only replacement preserves ordering across an
intervening event of any other class.

### 5.3 The pixel formula's inputs — and one trap

The formula in Q#BP15a is contract-level, not an implementation
detail, because its inputs are not all safe to adopt:

| Input | Source | Note |
| --- | --- | --- |
| `status_band_height_px` | `FontMetrics::status_band_height` (`pmacs-gpu/src/main.rs:137`) = `BASE_STATUS_BAND_HEIGHT * scale` | Safe |
| `TEXT_TOP_px` | `const TEXT_TOP: f32 = 16.0` (`main.rs:352`) | Safe; unscaled today |
| `code_line_height_px` | `FontMetrics::code_line_height` (`main.rs:131`) = `BASE_CODE_LINE_HEIGHT * scale` | Safe |
| `resolved_monospace_advance_px` | `State::mono_advance` (`main.rs:4899`) | **Unsafe to adopt blindly** |
| `divider_height_px` | `BASE_DIVIDER_HEIGHT` | **Does not exist yet** |

**The `mono_advance` trap.** `State::mono_advance` returns
`measured_mono_advance` when a `FontFacts` probe has been applied, but
otherwise falls back to **the first shaped glyph of the document
buffer** (`main.rs:4903+`). Panel column count would therefore become
**document-dependent**: two GPU frontends showing different files could
derive different `total.cols` from identical metrics, and the same
frontend's panel width could change when the document's first glyph
changes.

**Decision.** The panel geometry declaration uses a **stable normal-face
probe**, never the document sample. `probe_mono_advance(font_system,
family, metrics)` (`main.rs:323`) already exists and is exactly this: it
shapes `ADVANCE_PROBE` in a scratch buffer, independent of document
contents, dividing total run width by logical cells so ligature
substitution survives. The declaration resolves its advance from that
probe for the current family/metrics. If the probe returns `None` (the
family shapes no width), the frontend declares **zero usable geometry**
under a new epoch — the panel hides — rather than falling back to a
document sample.

**`BASE_DIVIDER_HEIGHT = 4.0`** at scale 1.0, scaled by
`FontMetrics::scale` like `status_band_height`. A 1–2 px rule is
adequate decoration but too fragile as the drag hit strip; 4 px still
reads as a rule while giving the pointer a usable target. **The entire
strip is painted with `ui.divider`, and that exact rectangle is the
hover/drag hit region** — paint geometry and hit geometry are the same
rect, so they cannot drift apart.

**`TEXT_TOP` stays `16.0`, unscaled.** It is a fixed surface inset
today, like `TEXT_LEFT` and the other paddings, while
`FontMetrics::scale` governs font-derived metrics and row chrome.
Scaling it only inside the declaration formula would disagree with the
actual renderer; scaling every renderer and hit-test occurrence is a
wholesale inset/DPI change and is **named here as separate work**, not
smuggled into Stage 2. The formula is pinned to the real unscaled inset.

Accordingly, **Q#BP15a's "all quantities use the frontend's current
scale" is narrowed**: font-derived metrics and the divider scale; fixed
surface insets keep their current units.

#### The seam is three boundaries, not one

Rev 2 asked for a single document-bottom accessor. That was wrong:
once a panel is installed, today's single value must **diverge into
three**, because some of its consumers must not move at all.

```
status_band_top          = surface_height - status_band_height

geometry_capacity_bottom = status_band_top - reserved_divider_height
                           // divider reserved even while absent

document_text_bottom     = status_band_top
                           - (installed_panel_height + divider_height
                              if Present, else 0)
```

`geometry_capacity_bottom` is what Q#BP15a's asymmetry already
requires: the divider is subtracted **for sizing purposes even while
the panel is absent**, which is what breaks the first-open cycle, while
the document renderer does not actually lose those pixels until a
`Present` panel is painted.

**Today `text_area_bottom` (`pmacs-gpu/src/main.rs:8490`) is all three
at once**, and its doc comment calls it "the single source for every
bottom-of-text computation" (Q#S3). Its ~19 call sites split into three
classes:

| Class | Boundary | Sites |
| --- | --- | --- |
| **Status-owned** — must stay pixel-identical at the window bottom | `status_band_top` | Status-band background rect `:5908`; band tops `:6003`, `:6027`, `:6140`; status text placement `:7134`, `:7922`; global minibuffer chrome |
| **Document-owned** — must move when a band is installed | `document_text_bottom` | Code/terminal clips `:7174`, `:7195`, `:7212`, `:7242`, `:7273`, `:7351`, `:7421`; caret visibility and code height `:4566`, `:6118`, `:6581`; document completion placement `:8077`; minimap `:8497`; visible-line estimate `:8501` |
| **Geometry declaration** | `geometry_capacity_bottom` | The Q#BP15a conversion only |

**Sites that consume no bottom coordinate at all** and must not be
touched: `:3175`, `:3185`, `:6601`, `:6607` size the status text
buffers to `status_band_height` directly. Rev 2 listed them as seam
consumers; they are not.

Each call site is classified individually. A blanket rewrite of
`text_area_bottom` to subtract the band would move the status chrome
with the document and is the defect this section exists to prevent.

**The contrast assertion (A2B-4).** "Every document consumer moved" is
only half a test — a uniformly wrong implementation that moves
everything passes it. The criterion must assert **both directions in
one scenario**: installing a panel moves every document-owned consumer
**while the status band stays pixel-identical** at the physical window
bottom. That is the assertion a blanket rewrite fails.

The one-accessor-per-boundary rule still holds within each class: a
second, unrouted derivation of any of the three is the exact shape of
Stage 1's `Layout::compute` two-caller defect, where
`src/overlay_paint.rs` derived its own rect and painted peer cursors at
unfixed rows.

### 5.4 Ordering against folding Stage 3

Settled by review round 1: **bottom-panel Stage 2 first, through the
landed GPU band.** Folding Stage 3 then re-scouts the extracted
painter, the panel projection, clipping, and `fold_projection` behavior
exactly once.

## 6. Coherence impact (per `COHERENCE.md` §20)

- **Journey steps touched: four, on the GPU frontend — steps 7–10**
  (find symbol / find file, terminal, build and test, error
  inspection). Rev 1 said "none directly", which contradicted its own
  next sentence. Today a GPU user who triggers references, project
  search, a terminal, compile, or error inspection gets the Stage 1
  non-side fallback: the output surface steals a document window
  instead of opening a panel. Every one of those steps therefore
  behaves differently on GPU than on TUI, and Stage 2 is what closes
  the divergence.
- **Interaction islands added: none, and this is a reduction.** §6
  grades islands "weak, and growing by one island per modal feature".
  Stage 2 extends one already-adopted policy (`display = "panel"`,
  used by listview, compile, and terminal) to a second frontend rather
  than minting a GPU-only surface. Q#BP14b deliberately reuses the
  existing `SearchPrompt` / `MenuPrompt` / `CompletionPopup` messages
  instead of panel-specific twins.
- **Config registry adoption: inherited, not extended.** Stage 1's
  `window.panel-height` and `window.min-height` already live in the
  registry. Stage 2 adds no new user-facing option; if the band needs
  one, it enters the registry.
- **Background-work attribution: unchanged, and this stage does not
  advance it.** Rev 1 implied Stage 2 helps §9's activity-view gap. It
  does not. A panel gives output a coherent *placement*; it does not
  make terminal PTYs, LSP servers, or workers appear in the
  activity/ownership view §9 describes, and it adds no join key across
  the four disjoint activity planes. The §9 gap is untouched.
- **Section this serves:** `COHERENCE.md` §14, which records the panel
  primitive as landed for Stage 1 and names "Stage 2 (GPU band)
  pending its own framing" as the open item.

## 7. Acceptance

**Parent criteria 37–55 remain authoritative and are not replaced.**
This section maps them to the two slices and adds only refinements.

### 7.1 Stage 2A — classified census routing + painter extraction

No protocol change. Parent criteria that apply in full: **42, 43, 44,
51 (the `LOCAL`-panel inheritance half)**, plus the extraction half of
**52**.

**52 splits across the slices.** 2A has no semantic panel projection
and no `PanelFrame`, so all it can prove is that the extracted painter
honors an explicitly supplied `None` fold map and that the stale
`src/window.rs:562` comment is corrected. The actual contract — *a
semantic panel with `fold_projection = false` never collapses folds and
never calls `fold_map_for_window`* — is production-reachable only once
2B lands the projection and the capability flip. It is therefore
reasserted in 2B (§7.2).

Refinements 2A adds:

- **A2A-1 (replaces rev 1's criterion 1).** Every **Projection** census
  item (#1–#7, #9, #10, #12, #21, #22) resolves through
  `primary_document_window` / `primary_document_buffer`; #8 and #11
  align **and then activate**; **#13, #14, #15, #23 continue to resolve
  the actually focused window**; #16–#19 follow Q#BP14b's routing
  table; #20 keeps its per-session counter with focus choosing the
  eligible terminal. Each class is asserted separately, at the
  outermost user-reachable seam, and falsified by revert. A test that
  only proves "the document is used" would pass with the focus classes
  wrongly rerouted, so the focus-class assertions are the load-bearing
  half.
- **A2A-2 (replaces rev 1's criterion 2).** `src/statusline.rs:644`
  resolves the primary document window, while `:629` and `:675`
  continue to report **actual focus** — pinned by a document provider
  truthfully observing `active = false` while the panel is focused
  (parent 42). The semantic-layout target captures primary document +
  visible side window, invokes each provider exactly once, and
  invalidates the whole evaluation when a callback mutates layout or
  focus.
- **A2A-3 (replaces rev 1's criterion 3).** The painter extraction
  preserves, for grid frontends: the painted **cells**, the **returned
  cursor**, the **focused window's `view_top` mutation** from the
  auto-scroll clamp, and **passive windows' untouched `view_top` and
  scroll state**. Byte-identical cells alone would not catch a clamp
  that silently moved to the wrong window.

### 7.2 Stage 2B — v21 protocol + daemon projection + GPU band

Parent criteria that apply in full: **37, 38, 39, 40, 41, 45, 46, 47,
48, 49, 50, 51, 53, 54, 55**, plus re-assertion of **42, 43, 44, and
52** **through the actual negotiated capability flip** rather than
through a test-only panel-capable semantic view. 52's 2B form is the
production one: a real semantic frontend with `fold_projection = false`
displaying a folded buffer in a panel shows every source line, and the
panel path never reaches `fold_map_for_window`.

Refinements 2B adds:

- **A2B-1.** The epoch state machine of §3.1 is pinned row by row,
  including the lower-epoch-identical-data rejection and the
  same-epoch-different-total rejection, and each row's
  `Advanced`/`Duplicate`/`Rejected` result is asserted — a `Duplicate`
  performs no reconciliation and a `Rejected` mutates nothing. Epoch
  `0` is rejected on the wire. **Exhaustion is pinned on both sides**:
  grid exhaustion clears `frame_geometry` to unknown and the panel
  hides (a subsequent real resize must not paint a stale-geometry
  panel), and a frontend that exhausts latches — a retained `Present`
  whose epoch still matches cannot make the band reappear, and only a
  fresh session clears the latch.
- **A2B-2.** A font or scale change that leaves `CellSize` **identical**
  still produces a new `geometry_epoch`, and the older `PanelFrame`
  neither paints nor hit-tests until a matching `Present` arrives. This
  is the case daemon value dedup cannot see and is why option 1 was
  chosen.
- **A2B-3.** Panel columns are derived from the **stable normal-face
  probe**, not `State::mono_advance`'s document-glyph fallback: two GPU
  frontends with identical metrics and different documents derive
  identical `total.cols`, and a probe returning `None` declares zero
  usable geometry rather than falling back to a document sample.
- **A2B-4 (contrast assertion).** Installing a panel moves **every**
  document-owned consumer — code and terminal clips, caret visibility,
  document completion placement, gutter/math clipping, minimap,
  visible-line estimate, hit testing, edge scrolling — by exactly
  `installed_panel_height + divider_height`, **while the status band
  stays pixel-identical** at the physical window bottom (background
  rect, band top, and status text placement all unchanged). Both halves
  are asserted in one scenario: a uniformly wrong implementation that
  moves the status band too would pass the "everything moved" half
  alone. The geometry declaration separately reserves the divider while
  the panel is `Absent`, and the document loses no pixels until a
  `Present` is painted.
- **A2B-5.** `panel_capable` is true only for a v21+ negotiated
  authenticated semantic session; a v20 semantic session is never
  **placed** in a side window, not merely denied the events.

## 8. Open items

**None.** Both round-1 open items are decided in §5.3:
`BASE_DIVIDER_HEIGHT = 4.0` at scale 1.0 (scaled, whole strip painted
`ui.divider` and used as the hit rect), and `TEXT_TOP` stays unscaled
with wholesale inset/DPI scaling named as separate work.

One deferral is recorded rather than resolved: **wholesale surface-inset
scaling** (`TEXT_TOP`, `TEXT_LEFT`, and the sibling paddings under
`FontMetrics::scale`) is pre-existing behavior Stage 2 pins rather than
fixes. It belongs to a spacing-system change of its own.

## 9. Slices, branches, and gates

Per review round 1: **two serial implementation PRs**, each a named
slice under this framing so one-feature/one-branch/one-PR holds. **2A
lands before 2B branches** — not stacked.

- **Stage 2A** — classified census routing + per-window painter
  extraction. Branch `bottom-panel-stage2a`. No protocol change.
- **Stage 2B** — v21 protocol, daemon panel projection, GPU band, and
  the negotiated `panel_capable` flip. Branch `bottom-panel-stage2b`,
  cut from `main` after 2A merges. Repeats 2A's relevant census
  assertions through the real capability flip.

Gates for both: the standing suite from `CLAUDE.md`, plus the **touched
acceptance suites named explicitly** — the standing rule is to run the
suites a change touches, and "standing suite" does not name them:

- `bottom_panel_stage1_acceptance` — the substrate both slices build on.
- `bottom_panel_stage2a_acceptance` / `bottom_panel_stage2b_acceptance`
  — new, one per slice.
- `statusline_segments_acceptance` — the fan-out target change (§3.3).
- `m11_5_semantic_acceptance` — the semantic census (§3.2).
- `gpu_initial_target_acceptance` — parent criterion 55.
- `gpu_font_acceptance` — font/scale geometry refresh (§5.3), including
  the normal-face probe and the unscaled-`TEXT_TOP` decision.
- The three vterm suites — the panel hosts terminals.
- Folding Stage 2's 48 — shared projection.
- `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`.

Protocol round-trip and byte-pin tests ride 2B. Parent criterion 54's
`--headless-probe` run — one real daemon, real PTY, real wgpu, through a
panel-hosted terminal — is a 2B gate.
