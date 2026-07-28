# Bottom panel Stage 2 — the GPU panel band (framing)

**Revision 6 — PR #184 review correction; the underlying Stage 2
framing remains APPROVED 2026-07-27. 2A (#177) and 2B-1 (#184) are both
merged, and 2B-2 is the next slice. Ground truth: canonical `main` @
`6bee09d`, where protocol schema support is `v6..=v21` while the
production server-first handshake still deliberately advertises v20.**
Revisions 1–4 were pre-implementation; rev 5 recorded the three-way
slice of Stage 2B after its first slice was already built; rev 6
corrects that slice's mixed-version and gate contracts. **No revision
bump accompanies 2B-1's merge** — the status sentence and ground-truth
anchor above are landed-state facts, and no design decision in this
document changed when #184 landed.

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

### 0.0 Rev 5 → rev 6 — PR #184 review round 2, four findings closed

- **R6-1 (P1) — v21 is reserved, not advertised, in 2B-1.** The
  protocol's handshake is server-first. An existing v20 TUI or GPU
  frontend rejects a `Hello { protocol_version: 21 }` before it can
  send an `AttachRequest`, so rev 5's claim that a v21 daemon and v20
  peer "still negotiate 20" was impossible. 2B-1 therefore extends the
  schema and accepted-version ladder to v21 while the production daemon
  continues advertising v20. A real-daemon acceptance emulates the
  shipped v20 rejection point and then requires the attachment to reach
  its initial grid. **2B-3 owns both a compatibility-preserving
  activation mechanism and the production move to v21; it may not
  simply change the unsolicited `Hello` to 21.**
- **R6-2 (P2) — durable protocol claims move with the wire.**
  `COHERENCE.md` and `docs/agent-handoff.md` now distinguish v21 schema
  support from the still-v20 production handshake.
- **R6-3 (P2) — the gate contract names the actual decomposition.**
  §9 now names 2B-1's
  `bottom_panel_stage2b_protocol_acceptance` suite and the exact planned
  daemon/GPU suite names for 2B-2 and 2B-3 instead of the nonexistent
  `bottom_panel_stage2b_acceptance`.
- **R6-4 (P2) — "one byte over" means exactly one.** The panel and
  copied terminal boundary fixtures replace a one-byte cluster with a
  two-byte cluster, and each independently asserts a total of
  `limit + 1`.

### 0.1 Rev 4 → rev 5 — the three-way slice of 2B (not a review round)

This revision changes no decision. It splits one approved
implementation slice into three and reallocates the acceptance
criteria across them.

- **R5-1 — why.** Rev 4 §9 scoped 2B as a single PR: v21 protocol,
  daemon panel projection, GPU band, and the negotiated
  `panel_capable` flip. Implementation showed that to be roughly four
  thousand lines spanning `pmacs-protocol`, `src/daemon.rs`, and
  `pmacs-gpu` — three review surfaces with different failure modes, in
  one diff. The same argument that produced 2A/2B applies again one
  level down, and it is the argument this arc has already accepted
  twice (Lean 4 stages 3a/3b and 4a/4b).
- **R5-2 — the boundary rule.** A slice ends where the next thing to
  build has a different *authority*: the wire format, the daemon that
  produces frames, and the frontend that paints them. Each slice is
  independently reviewable against a subset of the parent criteria,
  and each is additive — no slice makes a previously-passing assertion
  fail.
  **Criteria that span a boundary are named in every slice they touch,
  with their half stated**, rather than assigned wholesale to one. The
  clearest case is parent 39: its shared-validation and
  transport-budget halves are wire properties provable in 2B-1, while
  "the previous valid frame is retained" and "a duplicate does no
  work" are receiver-state properties that need the epoch machine and
  land in 2B-2.
- **R5-3 — this revision is retroactive for slice 1, and that is a
  process defect worth recording.** `bottom-panel-stage2b` already
  carries the v21 protocol layer (three commits, one review round
  closed) written before this revision existed. The workflow is
  framing → approval → branch → implement; slice 1 inverted it. The
  slicing decision was sound, but it was taken in code and discovered
  in the branch rather than proposed in the document, which is exactly
  how a stage's scope drifts without anyone deciding that it should.
  Rev 5 exists to put the decision back where it belongs before slices
  2 and 3 are written.
- **R5-4 — two of the three slices ship dark, deliberately.** Nothing
  in 2B-1 or 2B-2 is reachable by a user: `panel_capable` stays
  `false` for every negotiated semantic session until 2B-3. Rev 5
  incorrectly described that posture as a production v21 negotiation
  that remained compatible with v20 clients; rev 6 R6-1 supersedes
  that claim. The actual dark posture keeps the server-first
  production handshake on v20 while the v21 schema is reserved. This
  is the same posture 2A took ("seam adoption that becomes
  load-bearing in 2B"), and it means the arc must not stall between
  2B-1 and 2B-3. Recorded here so a stall is visible as a decision
  rather than inherited as a default.

### 0.2 Round 3 (rev 3 → rev 4) — 1 blocking, 1 high, 1 medium, all closed

- **R3-1 (blocker).** Rev 3's three-boundary model was right but its
  call-site table was wrong in five places, and each error was a real
  defect: `:6140` is **document completion placement** (classified
  status-owned, which would let completion overlap the panel);
  `:7195`/`:7212` are the two **status text bounds** (classified
  document-owned); `:7351` clips **global minibuffer candidate glyphs**
  to the dropdown's band anchor (classified document-owned, which would
  clip them against the document boundary); `:8561` (**document edge
  scrolling**) was missing entirely, leaving it tied to the old bottom;
  and `:8077` was described as completion placement when it is **caret
  clipping** (its class was right, its label wrong). §5.3's table is
  rebuilt from the full census and every row is verified against the
  source.
  **Root cause worth recording:** rev 3's table was built from a
  `grep | head -20` over 29 matches. The truncation is exactly why
  `:8561` vanished. The census is now stated as 20 production sites +
  1 definition + 8 test sites = 29, so a future reader can check the
  arithmetic instead of trusting the list.
- **R3-2 (high).** The three equations permitted negative coordinates
  on a surface shorter than its chrome, where today's
  `text_area_bottom` clamps with `.max(0.0)`. All three are now
  explicitly clamped, preserving the current helper's behavior.
- **R3-3 (medium).** §5.1's "exact split" omitted `validate_cells`'s
  `cell.attachment.is_some()` rejection. It is now classified — and
  **shared**, with the reasoning pinned.

### 0.3 Round 2 (rev 2 → rev 3) — 1 blocking, 2 high, 1 medium, all closed

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

### 0.4 Round 1 (rev 1 → rev 2) — 2 blocking, 3 high, 3 revision points, all closed

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

**Protocol was still v20 at this re-scout**; no intervening PR had
bumped it. Q#BP9's conditional resolved: **Stage 2 reserves v21**.
Rev 6 R6-1 adds the server-first compatibility constraint discovered
during 2B-1 review.

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
session. **The production daemon does not advertise v21 in 2B-1 or
2B-2.** Because `Hello` is server-first, 2B-3 must add or prove a
compatibility-preserving way to activate v21 before applying this rule;
merely advertising 21 would strand already-shipped v20 clients before
they can identify themselves.

## 4. Revisions to the parent framing

Only these; everything else stands.

- **Q#BP9 resolves to the v21 schema, with production advertisement
  held at v20 until 2B-3 supplies compatible activation.**
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

**Attachment rejection is shared, not terminal-only.** `validate_cells`
also rejects `cell.attachment.is_some()`
(`pmacs-protocol/src/terminal.rs:305`), and its error text reads "A
cell carries a frontend attachment, which terminals never use"
(`:190-191`) — phrased as a terminal-specific fact, which is why rev 3
missed it. **Stage 2 classifies it shared**: panels implement no
attachment rendering, so a `PanelFrame` carrying one describes a
surface the GPU would silently not draw. Shared rejection fails closed
on the producer side rather than shipping an invisible cell. The error
message is reworded away from "which terminals never use" to a
grid-neutral phrasing when it moves. If a later stage gives panels
attachment rendering, this rejection moves back to terminal-only as a
deliberate, reviewed change — not by default.

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

All three clamp at zero, preserving today's `text_area_bottom`
`.max(0.0)` behavior — without the clamps a surface shorter than its
own chrome yields negative coordinates, and the "exact formula" stops
being exact precisely where it matters most:

```
status_band_top          = max(0, surface_height - status_band_height)

geometry_capacity_bottom = max(0, status_band_top - divider_height)
                           // divider reserved even while absent

document_text_bottom     = max(0, status_band_top
                                  - installed_panel_height
                                  - installed_divider_height)
```

`geometry_capacity_bottom` is what Q#BP15a's asymmetry already
requires: the divider is subtracted **for sizing purposes even while
the panel is absent**, which is what breaks the first-open cycle, while
the document renderer does not actually lose those pixels until a
`Present` panel is painted.

**Today `text_area_bottom` (`pmacs-gpu/src/main.rs:8490`) is all three
at once**, and its doc comment calls it "the single source for every
bottom-of-text computation" (Q#S3).

The census is **29 matches: 20 production call sites, 1 definition
(`:8490`), and 8 test sites** (`:12887`, `:12937`, `:12997`, `:13109`,
`:13793`, `:14013`, `:15306`, `:15386`). Every production site,
classified individually against the source:

**Status-owned — must stay pixel-identical at the physical window
bottom, using `status_band_top`** (8 sites):

| Site | What it is |
| --- | --- |
| `:5908` | Status-band background rect `y` |
| `:6003` | `mb_visible_window` — rows that fit **above the band** |
| `:6027` | `mb_dropdown_window` origin — dropdown grows up from the band |
| `:7134` | `status_top` for the right status group |
| `:7195` | `status_buffer` `TextBounds.top` — status text bound |
| `:7212` | `status_left_buffer` `TextBounds.top` — status text bound |
| `:7351` | Minibuffer **candidate glyph** clip, anchored to the dropdown's band origin |
| `:7922` | `status_top`, second site |

The minibuffer is **global, bufferless chrome anchored to the status
band** (Q#BP14b keeps `MinibufferPrompt` global), so all four of its
sites — `:6003`, `:6027`, `:7351`, and its `status_left_buffer` bound
`:7212` — stay status-owned. Clipping candidate glyphs at
`document_text_bottom` would clip the dropdown against a boundary it
does not sit above.

**Document-owned — must move when a band is installed, using
`document_text_bottom`** (12 sites):

| Site | What it is |
| --- | --- |
| `:4566` | `terminal_cell_viewport` — drawable height for the cell grid |
| `:6118` | `completion_anchor_px` — anchor visibility bottom |
| `:6140` | `completion_dropdown_layout` — **document completion placement**; `band_top - (line_top + line_h)` is the space below the anchor line |
| `:6581` | `code_height` |
| `:7174` | Code text clip bottom |
| `:7242` | Math text clip bottom |
| `:7273` | Gutter clip bottom |
| `:7421` | Terminal clip bottom |
| `:8077` | `code_caret_rect_in_clip` — **caret clipping** |
| `:8497` | Minimap drawable height |
| `:8501` | Visible-line estimate |
| `:8561` | `edge_scroll_direction` — **document edge scrolling** |

**Geometry declaration** uses `geometry_capacity_bottom`, and is the
Q#BP15a conversion only.

**Sites that consume no bottom coordinate at all** and must not be
touched: `:3175`, `:3185`, `:6601`, `:6607` size the status text
buffers to `status_band_height` directly. Rev 2 listed them as seam
consumers; they are not.

Three of these classifications are the ones a plausible implementation
gets wrong, and each has a visible symptom: document completion
(`:6140`) anchored to `status_band_top` **overlaps the panel**;
minibuffer candidates (`:7351`) clipped at `document_text_bottom` are
**cut off**; and edge scrolling (`:8561`) left on the old bottom
**auto-scrolls from inside the panel**.

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
- **Which slice pays the coherence debt (rev 5).** The journey claim
  above is Stage 2B-3's alone. 2A, 2B-1, and 2B-2 close **no** journey
  divergence: with `panel_capable = false`, a GPU user still gets the
  Stage 1 non-side fallback on steps 7–10 after all three land. Stated
  explicitly so no slice's PR can claim the arc's coherence benefit
  before the flip earns it — three quarters of this stage is
  preparation, and only the last quarter is the improvement.

## 7. Acceptance

**Parent criteria 37–55 remain authoritative and are not replaced.**
This section maps them to the four slices — 2A, then 2B-1/2B-2/2B-3 —
and adds only refinements. A criterion that spans a slice boundary is
named in each slice it touches, with its half stated.

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

### 7.2 Stage 2B — v21 protocol, daemon projection, GPU band

Stage 2B as a whole owns parent criteria **37, 38, 39, 40, 41, 45, 46,
47, 48, 49, 50, 51, 53, 54, 55**, plus re-assertion of **42, 43, 44,
and 52** **through the actual negotiated capability flip** rather than
through a test-only panel-capable semantic view. 52's 2B form is the
production one: a real semantic frontend with `fold_projection = false`
displaying a folded buffer in a panel shows every source line, and the
panel path never reaches `fold_map_for_window`.

Per §0.0 R5-1 those land across three slices. Each slice's own gate run
is the standing suite plus §9's named acceptance suites; **only 2B-3
changes what a user sees.**

#### 7.2.1 Slice 2B-1 — the v21 wire layer

**Authority: `pmacs-protocol`.** The four wire shapes Q#BP9 names, the
version bump, and the shared cell-grid validator. No producer, no
consumer, no capability change.

- **37, in full.** `PanelFrame` round-trips including `panel_epoch` and
  `geometry_epoch`, with independent byte pins on the previous final
  `InstanceMessage::InitialTargetResult` and
  `FrontendEvent::TerminalPointer` variants. **Both pins must be
  falsified by revert**, not merely observed passing: a byte pin that
  never saw the shift it exists to catch pins nothing.
- **39, the wire half only.** Shared cell/topology/glyph/area
  validation; an area-bounded panel wider than 512 columns is accepted
  while a terminal frame retains its 512-column PTY cap; the maximum
  legal panel encoding stays below the transport limit. **The ratchet's
  fixture must be shown to spend the whole aggregate glyph budget** —
  otherwise it measures something smaller than the worst case and the
  bound it proves is not the bound that matters. The worst case is
  `1 × MAX_PANEL_VISIBLE_CELLS`, a legal panel geometry no terminal can
  express, so the terminal's own ratchet has never covered it.
  **39's receiver half — atomic rejection with retention of the
  previous valid frame, and a duplicate doing no work — is 2B-2.**
- **The version ladder moves with the bump.** `PROTOCOL_VERSION`
  becomes 21, `SUPPORTED_PROTOCOL_VERSIONS` accepts `6..=21` and
  rejects 22, and any test whose *name* encodes the old number is
  renamed. A ladder pin that passes across a bump was not pinning the
  version. **`ADVERTISED_PROTOCOL_VERSION` remains 20 in 2B-1 and
  2B-2** because the unsolicited `Hello` precedes any client version
  signal. A real daemon must remain attachable by a client whose
  supported range ends at 20.
- **Shared bounds are aliased, not duplicated.** Every constant the
  terminal screen and the panel validator both enforce is one
  definition with the other as an alias, so truncation and validation
  cannot drift apart.
- **Not in this slice:** the daemon arm that drops panel events from a
  grid session is exhaustiveness bookkeeping the bump forces, not
  projection. It asserts only that a grid session's panel declaration
  is dropped rather than trusted.

#### 7.2.2 Slice 2B-2 — the daemon panel projection and epoch machine

**Authority: `src/daemon.rs`.** Produces `PanelFrame`; derives the
grid; owns stale-event rejection. Exercised through a **test-only**
panel-capable semantic view — `panel_capable` stays `false` in
production negotiation until 2B-3.

- **38** (open → replace buffer → hidden by a tiny frame → reappear →
  close, with authoritative `Absent` and a new epoch on
  replacement/reappearance), **40** (first open at a non-80×24 frame
  stays absent until real `FrontendCellGeometry` arrives, never
  consulting the 24×80 attach placeholder), **49**, **50**, **51**,
  **53**.
- **39's receiver half**, per §7.2.1.
- **41, the daemon half:** the daemon alone derives the grid; an older
  retained frame neither paints nor accepts input after a new
  `geometry_epoch` until a matching `Present` arrives; row-clamping
  preserves the stored request; zero, non-finite, and non-positive
  metric inputs fail closed to zero usable geometry. *The pixel→cell
  formula and its call sites are 2B-3.*
- **42, 43, 44, 45, 52** in their projection form, through the
  test-only panel-capable view. Their production re-assertion through
  the real flip is 2B-3.
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
  fresh session clears the latch. **A2B-1's grid-exhaustion half is
  2B-2; its frontend-latch half needs a real frontend and is 2B-3.**
  Both halves are named here so neither is lost at the seam.

#### 7.2.3 Slice 2B-3 — the GPU band and the capability flip

**Authority: `pmacs-gpu`, plus the compatibility-preserving negotiation
activation.** This is the only slice a user can observe, and the only
one that closes the journey divergence in §6. It must not advertise
v21 in the server-first `Hello` until an existing v20 client can still
attach.

- **46** (band + divider shrink the document text area by exactly their
  pixel height; carets, hits, and scroll geometry respect the reduced
  area), **47** (divider drag, `window.min-height`, `RowResize` hover,
  and the stalled-writer tail-coalescing), **48** (`PanelPointer`
  driving selection, terminal mouse reporting, and click-to-focus
  without disturbing the document mirror), **54** (the
  `--headless-probe` run: one real daemon, real PTY, real wgpu, through
  a panel-hosted terminal), **55**.
- **41, the GPU half:** the pixel→cell conversion pinned at fractional
  widths and heights, and geometry refresh on window resize, font
  change, and scale change.
- **42, 43, 44, 45, 52 re-asserted through the production flip**, not
  the test-only view. This is the point of the re-assertion: a
  test-only panel-capable view can be constructed wrongly and agree
  with itself, so the production negotiation path must carry the same
  assertions.
- **A2B-1's frontend-latch half**, per §7.2.2.
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
- **A2B-4 (contrast assertion).** Installing a panel moves **all twelve
  document-owned consumers** of §5.3 by exactly
  `installed_panel_height + divider_height`, **while all eight
  status-owned sites stay pixel-identical** at the physical window
  bottom. Both halves are asserted in one scenario: a uniformly wrong
  implementation that moves the status band too passes the "everything
  moved" half alone. Three rows carry their own named symptom because
  they are the ones a plausible implementation misclassifies —
  **document completion (`:6140`) must not overlap the band**,
  **minibuffer candidates (`:7351`) must not be clipped by it**, and
  **edge scrolling (`:8561`) must not trigger from inside it**. The
  geometry declaration separately reserves the divider while the panel
  is `Absent`, and the document loses no pixels until a `Present` is
  painted. All three boundaries clamp at zero on a surface shorter than
  its chrome.
- **A2B-5.** `panel_capable` is true only for a v21+ negotiated
  authenticated semantic session; a v20 semantic session is never
  **placed** in a side window, not merely denied the events. The same
  acceptance must attach an actual v20 client to the production daemon
  after v21 activation, so the new path cannot pass by breaking the old
  handshake before placement is evaluated.

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

Per review round 1 and §0.0 R5-1: **four serial implementation PRs**,
each a named slice under this framing so one-feature/one-branch/one-PR
holds. **Each slice lands before the next branches** — none are
stacked, and each is cut from `main`.

- **Stage 2A — MERGED as #177** (`main` @ `0a3fcd1`). Classified census
  routing + per-window painter extraction. Branch
  `bottom-panel-stage2a`. No protocol change. The three-boundary GPU
  split is **2B-3**, not 2A: it is only observable once a band can be
  installed.
- **Stage 2B-1 — the v21 wire layer.** Branch `bottom-panel-stage2b`.
  The four wire shapes, the version bump, the shared cell-grid
  validator, and the version-ladder move. The v21 schema is reserved
  while the production daemon continues advertising v20. **No
  producer, no consumer, no capability change** — `panel_capable`
  stays `false`.
- **Stage 2B-2 — the daemon panel projection and epoch machine.** Cut
  from `main` after 2B-1 merges. Produces `PanelFrame` and owns
  stale-event rejection, exercised through a **test-only**
  panel-capable semantic view. Still no production flip.
- **Stage 2B-3 — the GPU band and the negotiated flip.** Cut from
  `main` after 2B-2 merges. The three-boundary text-area split, the
  divider, pointer routing, the compatibility-preserving v21
  activation, and `panel_capable = true` for a v21+ negotiated
  authenticated semantic session. **This is the slice that changes
  what a user sees**, and it repeats 2A's and 2B-2's relevant
  assertions through the real capability flip.

**Each slice runs the full gate set below, not a subset of it.** A
slice that touches only `pmacs-protocol` still runs the GPU and vterm
suites: the shared validator and the wire enums are exactly the kind of
change whose breakage surfaces in a consumer rather than at its own
definition.

Gates for each slice: the standing suite from `CLAUDE.md`, plus the **touched
acceptance suites named explicitly** — the standing rule is to run the
suites a change touches, and "standing suite" does not name them:

- `bottom_panel_stage1_acceptance` — the substrate all four Stage 2
  slices build on.
- `bottom_panel_stage2a_acceptance` — Stage 2A's classified census and
  painter extraction.
- `bottom_panel_stage2b_protocol_acceptance` — Stage 2B-1's v21 schema,
  server-first v20 compatibility, byte pins, and shared validation.
- `bottom_panel_stage2b_daemon_acceptance` — the exact suite name
  reserved for Stage 2B-2's projection and epoch machine.
- `bottom_panel_stage2b_gpu_acceptance` — the exact suite name reserved
  for Stage 2B-3's band, compatible activation, and capability flip.
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
