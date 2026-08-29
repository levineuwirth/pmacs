# Bottom panel — framing (window placement + side windows)

**Revision 16 — 2026-08-20, AWAITING APPROVAL.** Revision 16 closes
review of 15: Q#BP-R4 is one pre-effect disposition/gesture-lifecycle
table; projection-raised releases drain after `render_frame` returns
and before its messages are written; horizontal-wheel ground truth is
split by target/domain and the missing document effect is explicitly
owned by GUI Stage 1b; superseded current-state text is removed from
this section and the active-work ledger.

**Revision 4 — pre-implementation, DRAFT after review round 3 plus landed-state
audit. Ground truth: canonical `main` @ `ddaa80d` (documentation landing #152;
runtime @ `0dd16a5`, GPU initial target / #148 after folding Stage 2 / #149),
protocol v20, 2026-07-24. Amended by the pre-implementation dependency
verification in §0.6: the folding dependency is cleared, and one geometry
caller-census error is corrected.**

**Previously, revision 13 — 2026-08-14. Q#BP-R3 was OVERRULED:
the lane BLOCKS on a protocol-bearing cell-mapping generation (new
§5b).** Revision 12 accepted current-state hit semantics on three
bounds, and all three were wrong — a **foreign** edit moves the mapping
with `view_top` untouched, the error is **unbounded** once ticks, folds,
edits or reloads accumulate, and the stale window lasts until the
frontend **presents** the replacement frame rather than one round trip.
With the narrowness gone there is nothing to trade on.

The follow-up is a **cell-mapping generation, not a per-frame token**:
it moves when the inverse mapping moves (viewport, folds, wrap/gutter
geometry, buffer content) and holds across focus, styling, cursor and
selection-only repaints, so ordinary drags survive. §5b frames it —
appended variants rather than widened structs, bilateral version
gating, and a drop-before-mutation check. **GUI arc 1e's `OpenTarget`
moves to the following protocol version.**

**Everything else in revision 12 stands**, including Q#BP-R1, Q#BP-R2
and the R-a…R-d edges.

**Previously, revision 12 — APPROVED.** §5a is the acceptance-48
ground truth and its rulings are settled: **Q#BP-R1** (a single click
selects a listview row only), **Q#BP-R2** (a terminal-chrome wheel is
consumed, daemon-side, before activation), the **R-a…R-d** replay
edges, and the witness matrices A1–A5, B1–B6, D1–D4.

**§5a took eight review rounds (5–12), and the pattern is worth
keeping: every round found a witness or a rule that would have passed
against a broken implementation.** A test fixture cited as a production
handler; activation ordering mistaken for a safety proof; a mode-line
rule that was per-row when the tree's is per-kind, and document-only at
that; a producer rule the frontend had no information to obey; a
consume check that would have left focus stolen; and — twice — an
assertion added without a state in which it could fail. **No round
found a design disagreement.** The rulings from 8 and 9 survived
intact, and what changed each time was *where* a rule lived or *what a
witness could see*.

Revision 12 answered review of 11, whose witness had a **vacuous
precondition**.

"Focus unchanged" proves nothing if the terminal panel is **already
focused**: the below-activation mutation then calls `focus_window` on
the already-active side window, nothing changes, and the row passes.
Leg 2 must **start passive**, and assert it — primary document window
active, terminal side window distinct and passive, controller baseline
captured. The prose is also narrowed: **focus** catches the
below-activation ordering mutation; **controller identity** catches the
shared-path mutation, since `apply_terminal_gesture` claims the
controller at `src/editor.rs:3571` before local handling. Activation
alone claims nothing, so the two assertions are not interchangeable.

**Previously, revision 11 — SUPERSEDED.** Answered review of 10.

**The terminal-chrome wheel must be consumed BEFORE ACTIVATION**, not
merely before `apply_terminal_gesture`. `activates` is
`!matches!(kind, Move)` for a terminal panel
(`src/editor.rs:2695`), so the wheel already activates: focus and
`active_frontend` are written at `:2699` ahead of any replay decision.
A consume check below that block would leave the wheel **changing focus
while scrolling nothing and claiming no controller** — the precise
half-state AC48's activate-then-claim rule exists to prevent. §5a now
states the four-step order, with consumption at step 3, and the
document→terminal witness asserts **focus and controller identity
unchanged** alongside no bytes, no scrollback and no document
movement — assertions that are load-bearing, because the
consume-below-activation mutation moves nothing and is invisible
without them.

**Previously, revision 10 — SUPERSEDED.** Answered review of 9, which
put a correct ruling **on the wrong side of the seam**.

**The GPU cannot know whether a panel holds a terminal.** `PanelFrame`
carries `buffer_id`, both epochs, `size`, `cells`, `cursor` and
`focused` and **no target-kind discriminator**
(`pmacs-protocol/src/panel.rs:73`); `state.terminal`
(`pmacs-gpu/src/main.rs:1964`) is the **primary full-window** terminal,
not the side-window projection. So revision 9's producer rule
*"terminal chrome wheel: do not send"* was unimplementable without a
new wire field, which this lane must not add.

**Q#BP-R2's OUTCOME is unchanged; its ENFORCEMENT POINT moves.** The
producer is now target-blind throughout — it claims the chrome wheel
and sends it for every panel — and the daemon, which resolves the side
window and knows the buffer kind, decides: document → `scroll_window`;
terminal → **consume**, with no child bytes, no local scrollback and no
document fallthrough. The producer/receiver tables are split by seam
side accordingly, and the witness is **one frontend across a
document→terminal replacement**, so only the target differs.

**Previously, revision 9 — SUPERSEDED.** Both items reversed a decision
revision 8 made:

- **Q#BP-R2 is OVERRULED — a terminal-chrome wheel is CONSUMED, not
  clamped.** SGR wheel input is **coordinate-bearing**: `encode_mouse`
  writes `coord.col + 1` / `coord.row + 1` into the sequence
  (`src/terminal/input.rs:102`, `:146`), which A4 pins exactly. Clamping
  **fabricates a hit on the final content row**, so a
  position-routing application acts on a cell the user never pointed
  at. And unlike `Up`, a wheel has **no liveness obligation** —
  dropping one strands nothing. Consumed, never fallen through to the
  document; TUI parity restored.
- **R-c2's field choice is corrected: a SEPARATE
  `gesture_last_content_cell`.** Revision 8 said to retain the `Down`
  cell in `last_pointer_cell`, which would break a **tested
  guarantee** — that field is cleared on press precisely so the first
  same-cell `Drag` reaches the daemon
  (`pmacs-gpu/src/main.rs:19841`). Dedupe baseline and termination
  fallback are different jobs with different lifetimes.
- **The crossing table follows:** a content-originated `Drag` over
  chrome is **normalized and then subject to the ordinary dedupe**, not
  promised as sent. **`Up` is the load-bearing crossing event** and the
  only one promised unconditionally.

**Previously, revision 8 — SUPERSEDED.** Answered review of 7:

- **R-c is target × gesture-ORIGIN, not kind alone.** The TUI's
  per-kind rule is **document-only** — for terminals it rejects every
  kind on the mode line (`src/editor.rs:3273`) and passes a
  content-sized viewport. Raw chrome coordinates are actively unsafe
  there: `apply_terminal_gesture`'s reporting branch is bounds-checked
  (`:3560`), so a chrome row **falls through to the LOCAL branch** — a
  reporting child gets `Down` and no `Up`, and a chrome wheel becomes
  local scrollback. **A content-originated gesture terminates at its
  last valid CONTENT coordinate.** New: **Q#BP-R2**, ruling that a
  chrome wheel over a terminal panel **clamps** rather than dropping —
  a deliberate divergence from the TUI, flagged for overrule.
- **R-c2: the producer never remembers the `Down` cell.** Arming clears
  `last_pointer_cell` (`pmacs-gpu/src/main.rs:7250`) and only motion
  refills it, so once chrome stops being a `PanelCell`, a `Down` +
  immediate release has **no fallback coordinate**. Retain it at arm
  time; for a reporting terminal the row asserts the **exact child
  release bytes**.
- **A1 was a negative row standing alone** — disabling reporting
  entirely satisfies it. **A3–A5** add positive controls on the same
  terminal: exact SGR bytes for `Down`/`Drag`/`Up` and for the wheel,
  plus the non-reporting wheel's local scrollback effect.
- **Four witness seams tightened.** B2 reads the **raw `selection`**,
  because `active_region()` answers `None` on both sides of the `Up`
  (`src/editor_core.rs:4684`); B4 pins the **exact selected word**; B6
  runs its sentinel through **single and double** click; D4 uses a
  **changed** frame with unchanged epochs — the `Down`'s own focus
  repaint — since a byte-identical duplicate is suppressed before the
  reset could run (`pmacs-gpu/src/main.rs:6918`).

**Previously, revision 7 — SUPERSEDED.** Five contract gaps, three of
them corrections to 6's own rules:

- **R-c's "the last row is inert" was wrong.** The TUI's rule is **per
  KIND**: `inner_rows` guards `Down(Left)`/`Drag(Left)`/`Down(Right)`
  and deliberately does not guard `Up(Left)` or the wheel. A blanket
  rule would stop mode-line scrolling and leave a content-started
  gesture unterminated. **And the producer arms before the receiver can
  refuse** — a mode-line `Down` sets `pointer_held` locally
  (`pmacs-gpu/src/main.rs:2878`), so a receiver-only rule cannot
  prevent the orphan. R-c now carries a per-kind producer/receiver
  table, both crossings, and an explicit wheel ruling.
- **R-d covered panel identity but not GEOMETRY identity.** A font or
  scale change advances `geometry_epoch` while `panel_epoch` holds, and
  the transition clears neither pointer field, so a held gesture
  resumes under a new grid with *current, valid* epochs. Four
  mutations now, including the **negative** one: an ordinary
  same-identity refresh must not cancel a live gesture.
- **R-d also needed `last_pointer_cell` constrained separately** —
  clearing only `pointer_held` leaves the successor's first same-cell
  `Move` suppressed as a duplicate.
- **R-a witnessed only the terminal.** The document path consumes Shift
  too, for the selection anchor, so threading modifiers into
  `apply_terminal_gesture` alone would pass the proposed row while
  breaking Shift-click in document and listview panels.
- **R-b's rows were satisfiable by doing nothing.** They now pin A's
  anchor and cursor after the Drag, prove `Up` collapses an empty
  click, and carry Q#BP16's multi-click and Context semantics plus a
  listview visit sentinel for Q#BP-R1.

**Previously, revision 6 — SUPERSEDED.** Answered review of 5.
**Q#BP-R1 is RULED: a single click SELECTS a listview row only**;
RET/SPC remain activation, and this lane adds no click-to-visit.
Revision 5 concluded that the activation ordering made replay safe;
**it is necessary but not sufficient**, and §5a now carries the four
edges it missed — **R-a** modifiers dropped at the daemon boundary,
which breaks Shift's local-selection override; **R-b** `Drag`/`Up` do
not activate and another frontend can interleave, so replay needs a
named side-window cell→byte adapter and a window-targeted path;
**R-c** `panel_grid_size` includes the mode-line row while content is
`rows − 1`, so the terminal viewport must be `rows − 1` and document
replay needs the TUI's "mode-line click: reserved" rule; **R-d**
`Present`→`Present` replacement leaves the pointer latch armed, which
acceptance 49 cannot catch because the orphan gesture carries current
epochs.

**Previously, revision 5 — SUPERSEDED. Added §5a: acceptance 48
re-measured against production at `72da24a`.** The finding stands: AC48
is **half implemented** — focus, activation, the focused-only clamp and
the coalescing rules all landed; **listview row selection, panel
selection, terminal mouse reporting and wheel replay did not**. No
ruling in §3 or §5 changes.

Give pmacs a **bottom panel**: a buffer displayed in a fixed-height window
pinned to the bottom of the frame, resizable by dragging its divider, which
feature code targets **by policy** instead of by stealing the selected window.
This is what makes vterm feel like Emacs's vterm rather than `term` in a stolen
buffer, and it is the presentation substrate flycheck, compile, LSP panels, DAP
(`docs/dap-debugging-framing.md`, **parked awaiting this arc**), remotes, and
MCP surfaces all want.

The feature is a bottom panel. The **missing concept** underneath it is Emacs's
`display-buffer` + window parameters: pmacs has a real window tree but no way to
say *where* a buffer should appear, and no window that is anything other than
proportionally sized.

## 0. Revision history

### 0.1 Round 1 (rev 1 → rev 2) — 8 blocking, 6 revision points, all closed

Verdict: **panel-as-window (Q#BP1) confirmed; everything downstream of it in the
GPU and display-policy contracts rejected.** R1-1 → Q#BP14; R1-2 → Q#BP15;
R1-3 → Q#BP16; R1-4 → Q#BP2a; R1-5 → Q#BP11a; R1-6 → Q#BP13; R1-7 → Q#BP5a;
R1-8 → Q#BP7; rp-1 → Q#BP6; rp-2 → Q#BP10a; rp-3 → Q#BP4; rp-4 → Q#BP5b;
rp-5 → Q#BP2; rp-6 → acceptance.

### 0.2 Round 2 (rev 2 → rev 3) — 7 blocking, 5 revision points

Verdict: substantially stronger, still not approvable. **Bets B1, B6, and B7
falsified as written.** Every anchor below was re-verified against `6ed4fe9`
before this revision; all seven findings reproduce in the code.

| # | Finding | Closed in |
| --- | --- | --- |
| R2-1 | Q#BP14 doesn't fully separate projection from focus; census incomplete | Q#BP14 (rewritten) |
| R2-2 | Auto round-trip marking is buffer-global, not panel-local | Q#BP14a (new) |
| R2-3 | `PanelResize { size }` conflates three geometries; first-open cycle | Q#BP15a (new) |
| R2-4 | Q#BP4 erases `select = true` | Q#BP4 (rewritten) |
| R2-5 | Real visit paths lack a target-aware load; no compile/terminal entry points | Q#BP11b (new) |
| R2-6 | The jump ring becomes wrong once a panel is a separate window | Q#BP11c (new) |
| R2-7 | `PanelPointer` has no stale-frame identity | Q#BP16 |
| rp-1 | Minimum-height must be recursive; "never violate" too strong | Q#BP2 |
| rp-2 | Hidden-panel focus must be a durable transition | Q#BP2b (new) |
| rp-3 | B6 / acceptance 2 mathematically impossible as written | Bet B6 |
| rp-4 | Q#BP5b ancestor rule and `resize(win, …)` resolution | Q#BP5b |
| rp-5 | `WindowParams` gaps: remembered id, `no_other_window`, `dedicated` vs raw switch, fallback hygiene | Q#BP2c (new) |

**Bet corrections:**

- **B1 falsified as written** — Q#BP14a needs one panel-aware condition in
  `dispatch_idle_for`. Narrowed to terminal controller / escape routing only.
- **B6 falsified as written** — opening an N-row panel *necessarily* changes
  document rectangles. Restated over the document subtree's **structure**.
- **B7 falsified** — the active-window census is larger than
  `primary_document_window`; four more producers (R2-1) plus the input-side
  validator. Replaced by B7' over an explicit classified census.

### 0.3 Round 3 + integration review (rev 3 → rev 4) — 22 blocking,
7 revision points, all closed

Verdict: the three corrected bets were honestly restated, but **B7' was
falsified immediately** by four indirect active-buffer consumers. The remaining
findings were contract holes in geometry ownership, presentation identity,
placement precedence, real adopter lifecycle, jump-history ownership, and
interactive minima. The integration review extended that audit through
terminal/statusline helpers, render preparation, wire bounds, and the Stage 3
default lifecycle rather than stopping at the originally reported eight.

| # | Finding | Closed in |
| --- | --- | --- |
| R3-B1 | §1.3 missed four `active_buffer_id()` semantic producers; focus chrome could disappear or remain stale | §1.3, Q#BP14, Q#BP14b |
| R3-B2 | Hidden-panel reconciliation had no authoritative geometry or mutation seam | Q#BP2b, Q#BP4, Q#BP13 |
| R3-B3 | `buffer_id` cannot identify a close/hide/reopen presentation of the same buffer | Q#BP15, Q#BP16 |
| R3-B4 | Global reuse-first defeated requested side/exact-window placement; dedication was not universal | Q#BP3, Q#BP11b |
| R3-B5 | `listview` had no Stage 1 opt-in or panel-aware quit path; compile lifecycle was incomplete | Q#BP11b, Q#BP12 |
| R3-B6 | A global jump ring plus frontend-tagged entries lets one frontend consume another's history; origin buffer was not revalidated | Q#BP11c |
| R3-B7 | `FrontendCellGeometry` lacked an exact pixel→cell contract, an unknown initial state, and a session-kind gate | Q#BP15a, Q#BP9 |
| R3-B8 | `window.min-height` had no interactive recursion and sub-floor requested heights hid a satisfiable panel | Q#BP2, Q#BP5, Q#BP5b |
| R3-B9 | The viewport terminal-context guard and semantic terminal helpers still resolved the focused panel instead of the full-window document surface | §1.3, Q#BP14 |
| R3-B10 | `StatuslineEvaluationTarget::Semantic` captures `view.active` transitively; focusing a panel could clear the document statusline and the panel painter had no callback result | §1.3, Q#BP8, Q#BP14 |
| R3-B11 | `QuitAction` had no neutral non-side state and restored only a buffer/action, leaking replacement height or dedication into the prior presentation | Q#BP2, Q#BP2c, Q#BP11b |
| R3-B12 | Ordinary reuse admitted a side window, and side→ordinary fallback could carry panel-only parameters into a document window | Q#BP3 |
| R3-B13 | The non-side invariant fallback attempted to fabricate a document leaf instead of failing closed | Q#BP11a |
| R3-B14 | A horizontal boundary may sit below a subtree, so one divider can span several exposed leaf mode-line segments | Q#BP5, Q#BP5a, Q#BP5b |
| R3-B15 | Falling back from an invalid side-origin jump could duplicate a hidden panel buffer into the document window | Q#BP11c |
| R3-B16 | A frame for old font/scale geometry could arrive after a new declaration; presentation epoch alone cannot detect that race | Q#BP15, Q#BP15a, Q#BP16 |
| R3-B17 | `display_file` resolved a default target before dedup/eligibility, so a dedicated origin could force load-before-failure | Q#BP11b |
| R3-B18 | A creation-only `origin_document` becomes stale after the user enters the panel from another document split | Q#BP2c, Q#BP11a |
| R3-B19 | A semantic panel terminal cannot use full-window `TerminalResize`, and attach `term_sizes` is the wrong 24×80 source | Q#BP7, Q#BP15a |
| R3-B20 | Active-window cursor auto-scroll lives before the per-window paint loop; extracting only the loop leaves a focused panel caret off-screen | Q#BP8, Bet B2' |
| R3-B21 | Terminal's 512-column PTY bound is not a generic panel-grid bound; wide GPU frames can exceed it | Q#BP15, Q#BP15a, Bet B5' |
| R3-B22 | Stage 3 flipped defaults without an explicit current-window opt-out or its own acceptance contract | Q#BP11b, Q#BP12, acceptance |
| R3-rp1 | Do not publish an intentionally inert `no_other_window` API | Q#BP2c, §6 |
| R3-rp2 | Keep raw `switch_buffer` as the dedication escape hatch, but make display policy honor dedication everywhere | Q#BP2c, Q#BP3 |
| R3-rp3 | `origin_document` is implementation-owned, not caller-settable | Q#BP2c, Q#BP11 |
| R3-rp4 | Terminal bell state is per session, but choosing which session to drain is focus-facing | §1.3, Q#BP14b |
| R3-rp5 | The exact base advanced to `b168dcad`; only handoff/ledger documentation changed after `6ed4fe9` | title, §1, §7 |
| R3-rp6 | An unspecified `window.toggle-panel` and focus traversal into a hidden panel are not shippable contracts | Q#BP6, Q#BP11, §6 |
| R3-rp7 | Open PR #148 overlaps the attach/protocol seams this arc must edit | §7 |

**Bet corrections:** B7' was too syntactic. Searching only literal
`active_window_for` / `active_window` calls missed helpers such as
`active_buffer_id()` that resolve through the same focused window. It is
replaced by a transitive contract. The integration pass then falsified B7'' as
well: `semantic_terminal_key` reaches `view.active` from `src/editor.rs`, and
`StatuslineEvaluationTarget::Semantic` captures it directly in
`src/statusline.rs`. B7''' is now over **all transitive active-context reads
reached by the daemon/semantic projection**, not merely reads spelled in those
two files, plus the explicit surface-routing matrix in Q#BP14b. B2 is narrowed
to B2': the concrete painters are origin-agnostic, but the per-window active
auto-scroll preparation at `src/editor.rs:2883-2935` must be extracted with
them. B5 is narrowed to B5': cell/topology/aggregate wire validation is shared,
but terminal-specific per-axis PTY limits do not apply to a generic panel grid.

### 0.4 What the post-folding re-scout established (2026-07-24)

Folding Stage 2 merged as **#149**. Carried forward from rev 2, still true:

1. **The capability seam exists.** `FrontendView.fold_projection`
   (`src/window.rs:348`) is a non-`Default` bool passed explicitly into
   `build_fresh_frontend_view` (`src/daemon.rs:2935`) from the attach
   transaction (`src/daemon.rs:1769`), where both
   `negotiated_capabilities.semantic_render` and `negotiated_protocol_version`
   are in hand (`src/presence.rs:74-84`). Q#BP13 copies it.
2. **`DispatchIdle` is the existing optimistic-apply gate** — per frontend,
   keyed on `active_window_for(fid)` (`src/editor.rs:753-769` →
   `src/daemon.rs:1223-1248` → `pmacs-gpu/src/main.rs:4135`). Rev 2 concluded this
   needed no code; round 2 corrected that (Q#BP14a).
3. **The panel band breaks a folding invariant** — `paint_frame`'s per-window
   map is built ungated (`src/editor.rs:2991`) on the premise that a semantic
   session never enters it. Q#BP17.
4. **No protocol version is reserved.** #148 has now landed as protocol v20;
   its final `InstanceMessage` variant is `InitialTargetResult`, while
   `TerminalPointer` remains the final `FrontendEvent` variant. Q#BP9.

`Layout::compute` / `split_node` / `remove_leaf` / `collapse_single_child_splits`
took no folding edits, so Q#BP2/Q#BP2a stand. `Viewport<'a>` carries
`folds: Option<&'a VisibleLineMap>` (`src/view.rs:131-155`) and stays `Copy`.

### 0.5 Landed #148 + final integration audit — 9 blocking findings,
1 revision point, all closed

The runtime moved to `0dd16a5` during this review, then canonical `main`
advanced to `ddaa80d` through #152's handoff/ledger documentation only. The new
initial-target transaction directly overlaps attach, target loading, semantic
snapshot publication, and the protocol append point, so the document was
re-scouted against the landed runtime rather than retaining an open-PR
sequencing note.
**B7''' is falsified**: #148 added #21, while the final transitive scan found
the older attach inheritance (#22) and input-side source-window use (#23).
B7'''' is the corrected census bet after integrating them.

| # | Finding | Closed in |
| --- | --- | --- |
| R4-B1 | A hidden side leaf had durable focus state but no defined effective placement; the requested fixed extent could still steal rows or become flexible | Q#BP2, Q#BP2b, acceptance 7 |
| R4-B2 | #148's semantic replica-publication filter asks whether a peer displays a buffer through its focused window; panel focus can cause both a missed document snapshot and a panel-driven mirror swap | §1.3, Q#BP14, acceptance 43 |
| R4-B3 | Fresh no-target attaches inherit `LOCAL`'s focused buffer; attaching while the TUI focuses a panel would make that panel the new frontend's document | §1.3, Q#BP13, Q#BP14, acceptance 51 |
| R4-B4 | #148's private target loader reselects `view.active` after hooks; once hooks can create/select a panel, bootstrap could overwrite it instead of reasserting the requested document | Q#BP11b, acceptance 55 |
| R4-B5 | Omitted `height`/`dedicated` semantics were undefined across same-presentation redisplay, replacement, and creation | Q#BP3, acceptance 13 |
| R4-B6 | Recursive `QuitAction::Restore` history grew without bound under repeated panel replacement | Q#BP2c, acceptance 20 |
| R4-B7 | Wire/client documentation still defines semantic `CursorByte`/mirror state as the focused “active buffer”; after panel focus that name means the primary document surface, not input focus | Q#BP14, acceptance 42 |
| R4-B8 | A panel-owned `SearchPrompt` names the panel buffer, but the GPU currently displays prompts only when their buffer matches the document mirror; frame/chrome ordering and validation were undefined | Q#BP14b, acceptance 45 |
| R4-B9 | New geometry/drag/move events omitted the GPU outbox's bounded tail-coalescing contract, so a stalled daemon could turn normal resize/pointer traffic into lossless-queue overflow | Q#BP15a, Q#BP16, acceptance 47–48 |
| R4-rp1 | The focus census also omitted remote-CRDT source-window cursor/provenance application even though its classification remains Focus | §1.3, Q#BP14a |

### 0.6 Pre-implementation dependency verification (2026-07-24) — folding
cleared, 1 correction

Run against canonical `main` @ `ddaa80d` before branching Stage 1.

**The folding dependency is cleared.** #149 (`6ed4fe9`) and its landed-doc
refresh #150 (`b168dca`) are both ancestors of `ddaa80d`; no PR is open; the
retained `folding` and `folding-tui` branches carry zero commits beyond
`githubsucks/main`; and Stage 3 has neither a branch nor a framing
(`docs/active-work.md`). `cargo test --test folding_stage2_acceptance` is
48/48 green on this base. Every anchor this document borrows from the arc
reproduces: `fold_projection` (`src/window.rs:348`, non-`Default`), its attach
install (`src/daemon.rs:1769`), `build_fresh_frontend_view`
(`src/daemon.rs:2935`) and its `LOCAL`-active inheritance (`:2949-2958`), the
ungated per-window map in `paint_frame` (`src/editor.rs:2991`), the
active-frontend gate behind `fold_map_for_window`
(`src/editor_core.rs:566` → `fold_projection_active` `:549-551`), and the
`Copy` `Viewport<'a>` with `folds` (`src/view.rs:130`, `:155`). Q#BP17's stale
comment is at `src/window.rs:339-340`. Folding's entire `src/window.rs` diff was
one 22-line hunk at `:324`, and nothing has touched that file since `6ed4fe9`,
so `compute` / `compute_node` / `split_node` / `remove_leaf` /
`collapse_single_child_splits` remain pre-folding code and Q#BP2/Q#BP2a stand
unchanged. The only surviving coupling is forward and non-blocking: Stage 1 has
no folding surface at all, and if folding Stage 3 flips `fold_projection` true
for semantic sessions before this arc's Stage 2 lands, Q#BP17's "pass `None`"
becomes "pass that window's map".

| # | Finding | Closed in |
| --- | --- | --- |
| R5-B1 | `Layout::compute` has **two** production callers, not one; the second (`src/overlay_paint.rs:112`) derives its own text-area `Rect` and never consults `window_placements`, so the Q#BP2 signature change would leave peer-cursor overlays on unfixed geometry | §1.1, Q#BP2, acceptance 1 |

## 1. Ground truth (re-scouted 2026-07-24 against canonical `main` @
`ddaa80d`; runtime @ `0dd16a5`)

### 1.1 What already exists

- **A real window tree, per frontend.** `LayoutNode::{Leaf, Split{orientation,
  weights, children}}` (`src/window.rs:283`); `FrontendView { layout, active,
  fold_projection }` (`:320`); all windows in one flat `core.windows`.
- **Geometry is purely proportional.** `compute_node` (`src/window.rs:435`)
  divides by weight, last child takes the remainder; **zero extents are an
  explicitly permitted outcome on a tiny frame** (`src/window.rs:362-365`).
  `compute` (`src/window.rs:367`) has **two** production callers (R5-B1):
  `window_placements` (`src/editor.rs:2359`, calling at `:2372`), and the
  peer-presence overlay pass (`src/overlay_paint.rs:112`), which builds its own
  text-area `Rect` from `core.active_layout()` and never routes through
  `window_placements`. The remaining `compute` calls (`src/editor.rs:6363`,
  `:6783`, `:6788`) are inside the `cfg(test)` module opening at `:4098`.
  `compute` is the only producer of window rectangles; every other layout
  consumer (`statusline.rs`, `desktop.rs`, `editor_core.rs`, `lua_bindings`)
  reaches the tree through `iter_ids` / focus traversal and needs no fixed-extent
  argument.
- **`WindowPlacement { outer, content }`**, `content = outer` minus the mode
  line (`src/editor.rs:2352`); frame area is `rows - 1`.
- **The mode-line row is reserved in the mouse path** — `window_at_cell`
  (`src/editor.rs:2391`), early return at `local_row >= inner_rows`
  ("Mode-line click: reserved.", `:1865`); `MouseClickState` (`:204`).
- **`paint_frame` is a per-window loop** (`src/editor.rs:2937`) through an
  origin-agnostic `Viewport<'a>` (`:3008`); one fold map per rendered window
  (`:2991`).
- **The terminal controller is keyed on the window** (`src/terminal/view.rs:19`,
  `:86`, `:105`); `active_terminal_key` reads `view.active`
  (`src/editor.rs:989`).
- **Terminal scroll is anchor-based**, `selection_froze_top`
  (`src/terminal/view.rs:360`, `:422`), `view_geometry` (`:625`),
  `record_view_size` (`:273`), controller-only PTY resize
  (`src/editor.rs:1234-1249`).
- **Attach-time capability plumbing is proven** (`src/presence.rs:74-84`,
  `peer_declared_terminal_support` `src/daemon.rs:888`, folding's install at
  `src/daemon.rs:1769`).
- **The GPU has a bottom band and renders a foreign cell grid**
  (`pmacs-gpu/src/main.rs:395-402`, `:1574-1609`, `:6670-6683`;
  `TerminalFrame`
  `pmacs-protocol/src/terminal.rs:103`, planner `pmacs-gpu/src/terminal.rs`).

### 1.2 What does not exist

- **No placement policy** — `pmacs.window` (`src/lua_bindings/mod.rs:12167`)
  acts only on the active window; no `display`, `pop_to_buffer`, `quit_window`.
- **No window parameters** on `Window` (`src/window.rs:158`).
- **No divider drag, no keyboard resize**, in either frontend.
- **No `CursorIcon` / `set_cursor` in `pmacs-gpu/`.**
- **No `WindowId` in `pmacs-protocol/`.**
- **No `MIN_WINDOW_*` constant, no `window.min-height`.**
- **No general target-aware load.** #148 added the private attach-only
  `open_initial_target` transaction (`src/daemon.rs:1625-1677`) and the
  side-effect-free `EditorCore::get_or_load_buffer` seam
  (`src/editor_core.rs:660-684`), but the former still switches
  `view.active` before and after hooks. The public
  `pmacs.buffer.find_or_open` likewise switches the **active** window in both
  branches before firing `buffer.after-switch` / `after-load`
  (`src/lua_bindings/mod.rs:3089`, `:3108`, `:3113`). Neither accepts an exact
  destination window.
- **No way to open a terminal off-active.** `pmacs.terminal.open` hardwires
  `switch_active_buffer_for(frontend_id, …)` into that frontend's active window
  (`src/lua_bindings/mod.rs:8500`), and rolls the session back if it fails.
  Compile creates its buffer then `switch_buffer`s (`compile.lua:263`, `:808`).

### 1.3 The active-context census (R2-1, R2-2, R3-B1) — every consumer,
transitively classified

Round 2 found the literal active-window consumers. Round 3 found the remaining
trap: `active_buffer_id()` is itself an active-window read, so a census of only
the spelling `active_window*` is not exhaustive. The contract is therefore over
**every transitive read of the focused window/buffer** in the daemon and
semantic producer, including helper calls.

The bounded production scope is: reads that choose an attached frontend's
semantic document messages/alignment, focus chrome, snapshot routing, attach
inheritance, presence/bell surface, or optimistic-input acceptance/application
in `src/daemon.rs`, `src/semantic_render.rs`, and `src/statusline.rs`, plus
their named `src/editor.rs` helpers. Ordinary grid per-window painting and
normal key/mouse command semantics are excluded—they already operate on the
real window that owns them—as are test-only reads.

| # | Consumer | Site | Class |
| --- | --- | --- | --- |
| 1 | Semantic buffer-follow + `BufferSnapshot` re-send | `src/daemon.rs:1133-1149` | **Projection** |
| 2 | Lazy CRDT upgrade + replica broadcast | `src/daemon.rs:1096`, `:2319-2343` | **Projection** |
| 3 | `CursorByte` | `src/daemon.rs:1418-1428` | **Projection** |
| 4 | `LineNumbers` mode | `src/semantic_render.rs:1350` | **Projection** |
| 5 | Selection decorations | `src/semantic_render.rs:1706` | **Projection** |
| 6 | Terminal-frame suppression of the document pass | `src/semantic_render.rs:610` | **Projection** |
| 7 | `Viewport` alignment | `src/daemon.rs:2020` → `align_semantic_window_to_buffer` `:2900` | **Projection** (must not move focus) |
| 8 | Document `Pointer` | `src/daemon.rs:2085` → same helper | **Projection + focus** |
| 9 | `Viewport` terminal-context gate | `src/daemon.rs:2002-2009` | **Projection** — a terminal panel must not suppress a document viewport |
| 10 | Full-window semantic terminal declaration/snapshot/sync | `src/daemon.rs:2026-2046` → `semantic_terminal_key`, `src/editor.rs:1157` | **Projection** — these describe the primary document surface, never the panel band |
| 11 | Full-window `TerminalPointer` | `src/daemon.rs:2048-2067` → `dispatch_semantic_terminal_pointer`, `src/editor.rs:1276` | **Projection + focus** — a non-hover gesture on the document terminal takes document focus |
| 12 | Semantic statusline target capture | `src/semantic_render.rs:628` → `capture_target_contexts`, `src/statusline.rs:634` | **Projection + panel projection** — document segments use the primary document; panel segments paint in its mode line |
| 13 | Remote CRDT-op validation | `src/daemon.rs:2531-2537` | **Focus** |
| 14 | `dispatch_idle_for` | `src/editor.rs:753-769` | **Focus** |
| 15 | Presence snapshot (peer cursor broadcast) | `build_presence_snapshot`, `src/daemon.rs:2988-3000` | **Focus** — it answers "where is this user working", which *is* the panel when the panel is focused |
| 16 | `SearchPrompt` active-buffer gate | `src/semantic_render.rs:1106` | **Focus chrome** — the prompt follows the modal session; match washes paint on its owning window |
| 17 | `MenuPrompt` active-buffer gate | `src/semantic_render.rs:1164` | **Surface-routed** — a document menu is semantic chrome; a panel menu is painted in `PanelFrame`; the other surface receives a clear |
| 18 | `MinibufferPrompt` active-buffer gate | `src/semantic_render.rs:1220` | **Focus chrome, global** — bufferless and emitted independently of the document viewport |
| 19 | `CompletionPopup` active-buffer/window gate | `src/semantic_render.rs:1027`, `:1030` | **Surface-routed** — a document popup is semantic chrome; a panel popup is a window overlay in `PanelFrame`; the other surface receives a clear |
| 20 | Terminal bell drain | `take_pending_terminal_bell`, `src/daemon.rs:1565-1600` | **Focus/session** — the counter is per session, but the active window chooses which session may drain |
| 21 | Semantic recipient filter for lazy/initial-target `BufferSnapshot` publication | `publish_buffer_snapshot_to_replicas`, `src/daemon.rs:2441-2475` | **Projection** — “displays this buffer” means the peer's primary document surface |
| 22 | Buffer inherited by a fresh no-target frontend view | `build_fresh_frontend_view`, `src/daemon.rs:2949-2958` | **Projection** — inherit `LOCAL`'s primary document, never its focused panel |
| 23 | Remote CRDT-op source-window cursor/provenance application | `handle_remote_crdt_op`, `src/daemon.rs:2735-2797` | **Focus/input** — the validated op applies to the source's actually focused window |

**Why #2 is the sharpest.** Focusing a *fresh generated* panel buffer triggers
the lazy CRDT upgrade, which **broadcasts a `BufferSnapshot` to every replica**
(`src/daemon.rs:1096-1107`) and records it in `last_active_buffer_sent`. That
swaps the GPU's mirror to the panel buffer — directly contradicting rev 2's
acceptance 34. It is not reachable from rev 2's three-coupling model at all.

**Why #9–#12 are separate from ordinary document projection.** The viewport
gate, terminal declaration/key, terminal pointer, and statusline target all
reach focused-window state outside the main `render_frame` buffer producers.
If left unchanged, a focused terminal panel rejects the still-visible document
viewport, a full-window document terminal cannot repaint or receive a click,
and `DeclaredBufferMismatch` clears the document's statusline. The document
terminal events and document statusline must resolve the primary document
window, while the panel gets its own `PanelPointer` and painted mode line.

**Why #13 and #23 forbid an opt-out.** Remote-op validation requires the op's
`buffer_id` to equal the **source's active window buffer**
(`src/daemon.rs:2531-2537`), and the accepted-op path applies cursor/provenance
to that same source window (`:2735-2797`). If a panel is focused while the GPU
still optimistically edits its document mirror, every resulting op is rejected
— silently diverging the mirror. Optimistic apply and daemon input must agree
on *one* window.

**Why #16–#19 cannot inherit the document viewport.** `render_frame` invokes
all four producers with `vp.buffer_id` (`src/semantic_render.rs:800-807`), but
their current guards compare it with `core.active_buffer_id()`. With a panel
focused, a panel-opened `M-x` emits no `MinibufferPrompt`; search/menu chrome
can disappear; and a document completion popup can remain stuck because the
producer returns before emitting its authoritative close. Q#BP14b splits
per-window overlays from global/native semantic chrome and makes both open and
clear paths explicit.

**Why #21 and #22 are projection even though they sit outside
`render_frame`.** #148 publishes a target/upgraded snapshot to an existing
semantic peer only when that peer “displays” the buffer. Testing the focused
panel would miss a buffer visible in the document or replace the GPU mirror
because only the panel showed it. A fresh no-target attach has the same
surface question when it clones `LOCAL`: panel focus must not turn panel
content into the new frontend's full document. Both therefore use
`primary_document_window`, not focus.

## 2. What ships (staged)

- **Stage 1 — window placement + TUI side windows. No wire change** (inherits
  the protocol version on its eventual base).
- **Stage 2 — the GPU panel band. Next available protocol version.** Own
  re-framing before implementation.
- **Stage 3 — default placement flip**, after Stage 2 (Q#BP12).

## 3. Decisions

### Q#BP1 — A panel is a WINDOW, not a new kind of slot *(confirmed round 1)*

Side windows are ordinary leaves in `Layout`, carrying parameters.
`TerminalController` is keyed `(frontend_id, window_id)` and
`active_terminal_key` reads `view.active` (`src/editor.rs:989`), so child-input
routing, the fixed `C-c` escape, atomic controller replacement, and
release-on-blur need no new machinery — **this is the whole of B1 now** (round 2
correctly removed the input-gating half; see Q#BP14a). A non-window slot would
need a second copy of the controller model plus per-window overlays, gutter,
statusline, mouse routing, selection, and desktop handling.

### Q#BP2 — Window parameters; fixed extents; the recursive minimum (rp-1, rp-5)

```rust
pub struct WindowParams {
    pub side: Option<Side>,               // immutable after placement (Q#BP2a)
    pub fixed_rows: Option<u32>,          // outer rows, incl. the mode line
    pub dedicated: bool,
    quit_action: Option<QuitAction>,      // implementation-owned; None off-side
    origin_document: Option<WindowId>,    // implementation-owned; read-only to Lua
}

pub enum QuitAction {
    Delete,
    Restore {
        buffer_id: BufferId,
        fixed_rows: u32,
        dedicated: bool,
        cursor: Position,
        view_top: usize,
        goal_col: Option<u32>,
        selection: Option<Selection>,
        then: Box<QuitAction>,
    },
}
```

`Layout::compute(area)` → `Layout::compute(area, fixed: &HashMap<WindowId,
u32>)`. **Both** production callers supply the map (R5-B1): `window_placements`
(`src/editor.rs:2372`) and the peer-presence overlay pass
(`src/overlay_paint.rs:112`). The second is easy to miss because it derives its
own text-area `Rect` from `core.active_layout()` instead of reusing
`window_placements`; leaving it on unfixed geometry would paint every peer
cursor at the row it would occupy with no panel open. Since both callers need
the same `HashMap<WindowId, u32>`, the fixed map is derived by one shared
helper over the frontend's side windows rather than assembled at each call
site. Two-pass inside a split: subtract fixed children, then divide the
remainder by weight among flexible children (preserving
last-flexible-takes-the-remainder).

**The minimum is recursive (rp-1).** Rev 2's "leave the document subtree two
rows" is wrong: two rows at the root does not give each nested leaf two rows.
Define, over row extent:

```
subtree_min_rows(Leaf)                     = MIN_WINDOW_OUTER_ROWS       // 2
subtree_min_rows(Split{Horizontal, kids})  = Σ subtree_min_rows(kid)
subtree_min_rows(Split{Vertical,   kids})  = max subtree_min_rows(kid)
```

(Horizontal splits stack rows, so minima add; vertical splits share rows, so the
tallest child governs.)

**And the promise is bounded (rp-1).** The layout **already permits zero
extents** on an intrinsically tiny frame (`src/window.rs:360`) and renderers
already skip empty rects — rev 2's "the layout can never violate the floor" was
too strong. The honest contract: **the panel allocator never makes an otherwise
satisfiable document tree unsatisfiable.** Formally, the panel takes
`min(fixed_rows, area.rows.saturating_sub(subtree_min_rows(document_root)))`,
and if that is below `MIN_WINDOW_OUTER_ROWS` the panel is **hidden** (Q#BP2b).
What the frame does to a document tree that could not fit anyway is unchanged
behavior.

`MIN_WINDOW_OUTER_ROWS = 2` (one text row + one mode line, since `content =
outer - 1`) is a structural floor. Every programmatic source of `fixed_rows`
(`height`, `window.panel-height`, and `set_params`) clamps a nonzero request to
that floor; a request of `0` is rejected rather than being an invisible
"open". A frame shrinking under the floor hides the panel, but a caller asking
for one row on a large frame gets a two-row panel. Side creation resolves an
omitted height through `window.panel-height`, so a live side leaf always has
`fixed_rows = Some(requested_rows)`; `None` remains the ordinary-window value.

**Hidden has an exact effective geometry.** The requested `fixed_rows` remains
stored, but `window_placements`/`Layout::compute` receives the reconciled
effective state: while `panel_hidden`, the side leaf receives an empty rect and
the prior document root receives the full frame area (minus the one global
status row), as if the wrapper's side child consumed zero rows. The side leaf,
wrapper, weights, `WindowId`, and requested extent remain intact. It must not
fall through as a flexible child, and the requested fixed extent must not
continue stealing rows while hidden.

`window.min-height` is a **user preference clamped into
`[MIN_WINDOW_OUTER_ROWS, …]`** that applies only to *interactive* resize (drag,
keyboard, and GPU `PanelResizeRows`). Define a second recursion with the same
sum/max shape:

```
interactive_min_rows(Leaf)                    = window.min-height
interactive_min_rows(Split{Horizontal, kids}) = Σ interactive_min_rows(kid)
interactive_min_rows(Split{Vertical,   kids}) = max interactive_min_rows(kid)
```

Each leaf resolves the setting against that window's current `buffer_id`
(buffer-local override → global → default), and one gesture snapshots the
result before changing geometry. Side creation similarly resolves
`window.panel-height` against the buffer being displayed.

Interactive boundary motion preserves the preferred minimum on **both** sides
when the current frame can satisfy it; if the frame is already smaller, the
motion may not make either side worse. The ordinary layout pass and
frame-resize reconciliation consult only `subtree_min_rows`, so changing a
preference never invalidates an existing layout.

### Q#BP2a — Side-window topology (R1-4)

- **At most one bottom-side leaf per `FrontendView`.**
- Installed as the **final child of a root-level horizontal split wrapping the
  entire prior root**: `root := Split { Horizontal, weights: [1, 1],
  children: [<prior root>, Leaf(panel)] }`. `fixed_rows` makes the panel's
  weight inert; the prior root takes the flexible remainder.
- **Closing collapses the wrapper** via `collapse_single_child_splits`
  (`src/window.rs:537`) once `remove_leaf` drops the panel — no new tree code.
- **`fixed_rows` is interpreted only on that root-level side child**; elsewhere
  it is inert (Q#BP2's `fixed` map is built from side windows only).
- **`side` is immutable after placement.** `set_params` rejects adding,
  changing, or clearing it. Transactional rehoming is deferred by name.

### Q#BP2b — Hiding is a durable state transition, not a per-frame effect (rp-2)

Rev 2 said a hidden panel "hands focus to a document window for that frame".
That is a render-time dodge: keys would still route to an invisible window, and
the terminal resize path merely returns on zero content **without releasing the
controller** (`src/editor.rs:1118`). Geometry currently lives outside
`EditorCore` — the local loop reads `frontend.size()`, while the daemon owns
`term_sizes` — so "wherever layout is recomputed" is not a mutation seam.

Each `FrontendView` therefore gains
`frame_geometry: Option<DeclaredFrameGeometry>` and `panel_hidden: bool`, where
`DeclaredFrameGeometry = { geometry_epoch: u64, total: CellSize }`. `None`
means **unknown**, not 24×80. Grid/LOCAL views cache their real attach/resize
size with an internal epoch; a semantic view stays `None` until its first
authenticated `FrontendCellGeometry` in Stage 2 (Q#BP15a).

`EditorState::reconcile_panel_layout(frontend_id)` is the single idempotent
mutable transaction. It runs after attach/resize, display/split/close,
`fixed_rows`/setting changes, and any Lua hook or callback transaction that can
mutate the layout (including statusline evaluation); it also runs defensively
before final-focus resolution, input dispatch, terminal sync, and paint. Thus
two events drained in one burst cannot route the second event to a panel the
first event made invisible, and a render callback cannot leave stale panel
geometry for the painter. The transaction:

1. If there is no live side window, set `panel_hidden = false`; Stage 2 records
   and emits `Absent` as needed, clears presentation input authority, and
   returns. `panel_hidden` never describes a panel that no longer exists.
2. Compute the panel's allocation per Q#BP2. Unknown geometry or
   `frame_geometry.total.cols == 0` is not presentable and follows the hidden
   arm; no zero-width `Present` frame is legal. Install Q#BP2's effective
   empty-side/full-document placement for the hidden state rather than feeding
   the stored request to ordinary fixed/flexible allocation.
3. If it is below the floor and the panel is currently visible → mark
   `panel_hidden = true`; if `view.active` is the panel, **set `view.active` to
   the non-side target** (Q#BP11a); **release the terminal controller** for
   that view key (`release_controller`, the existing call at
   `src/editor.rs:1113`).
4. If it becomes satisfiable again → `panel_hidden = false`, restore its
   allocation from the still-stored requested `fixed_rows`. **Focus is not
   restored** — the user moved on; `C-x o` returns.
5. Stage 2: a hide or unhide emits `PanelFrame::Absent` / a fresh `Present`
   authoritatively (Q#BP15).

`panel_hidden` is cached derived layout state on the `FrontendView`, not a
`WindowParams` field. It is recomputed from authoritative geometry and must
never be persisted or set by Lua.

### Q#BP2c — Parameter semantics the API must pin (rp-5)

- **`origin_document: Option<WindowId>`** is the remembered document window
  Q#BP11a needs; rev 2 described it but omitted it from the struct. Recorded at
  panel creation, then refreshed on every focus transition from a non-side
  window into the panel (keyboard, pointer, or selecting display).
  Panel→panel redisplay and passive display do not overwrite it.
  It is **revalidated on every use** (live, in this frontend's layout,
  non-side) and cleared when it fails. It is implementation-owned:
  `params(win)` may report it for diagnostics, but `set_params` rejects it.
- **`no_other_window` does not ship in v1.** A public parameter that is stored
  and deliberately ignored is a false contract. The whole parameter and its
  traversal semantics are deferred. If added later, traversal filters it only
  as a **destination**; a currently focused no-other window can always leave,
  so the caller cannot strand focus.
- **`dedicated` binds `display_buffer` only.** Raw `pmacs.window.switch_buffer`
  and `switch_active_buffer_for` **ignore it**: they are the deliberate
  low-level escape hatch, and every existing caller predates this arc.
  `display_buffer` is the policy layer and checks dedication on **every**
  candidate, not only the side slot (Q#BP3); making the primitive enforce
  policy would change existing behavior silently.
- **Quit restoration preserves replacement history.** `QuitAction` is
  present only on a side window. Creating the side installs `Some(Delete)`;
  ordinary windows and every fallback carry `None`. Replacing a side
  presentation captures its buffer, requested height, dedication, cursor,
  viewport/goal/selection state, and prior action in `Restore`. Restoring
  rebuilds `TextView`, clamps the saved positions against the buffer's current
  contents, reinstalls `then`, and fires the normal switch hook so overlays
  reattach. Derived `last_visible_rows` and trait-object overlays are never
  snapshotted. Thus C→B→A→delete restores the actual presentation rather than
  forgetting A or leaking C's height/dedication into it. A killed restore
  buffer still fails closed to `Delete` (Q#BP10a), dropping the unusable chain.
  `MAX_PANEL_QUIT_DEPTH = 64` bounds the recursive history: before wrapping an
  existing action, count iteratively; if the new depth would exceed the cap,
  truncate the oldest retained `Restore` by replacing its `then` with
  `Delete`. The newest 64 presentations therefore remain LIFO-restorable and
  the following quit closes the slot. Construction, traversal, and truncation
  never recurse past the same bound.
- **The two bookkeeping fields are read-only.** `params(win)` may expose
  `origin_document` and a diagnostic description of `quit_action`;
  `set_params` rejects both. Lua cannot forge a window id, buffer restore
  chain, or stale cursor state. `window.quit` on a window with
  `quit_action = None` returns a pointed error without closing or switching
  anything; non-side adopter fallbacks call their existing restore path
  instead.
- **Capability fallback discards every side-specific parameter.** When
  `!panel_capable` (Q#BP13), `display_buffer` drops `side`, `fixed_rows`,
  `dedicated`, `quit_action`, and `origin_document`, and displays into the
  non-side target as an ordinary buffer switch. A fallback must never dedicate,
  pin, or otherwise poison the primary document window.

### Q#BP3 — `display_buffer`: the placement policy

Placement affinity precedes generic reuse; otherwise a persistent compilation
buffer already visible in a document window makes `{side = "bottom"}` silently
ignore its requested placement. `action.window` and `action.side` are mutually
exclusive; supplying both is an error. `height` requires a side request or an
exact target that is already the side window. Stage 1 accepts only
`Side::Bottom`; every other side value is a pointed unsupported error, not an
ordinary fallback.

1. **Exact target (`action.window`).** Validate that it is live and belongs to
   this frontend. Use that exact window or error; generic reuse may not
   substitute another. A target dedicated to a different buffer errors.
2. **Side target (`action.side`).**
   1. Reuse a window on the requested side already showing `buffer_id`.
   2. Otherwise use that side slot if absent or not dedicated to another
      buffer, creating it per Q#BP2a when absent.
   3. If the one side slot is dedicated to another buffer, never create a
      second one: fall back to the ordinary non-side policy below **after
      discarding `side`, `height`, `dedicated`, and quit bookkeeping**. Only an
      explicitly supplied `select` survives. A failed placement request may
      not pin or dedicate a document window.
   A non-side window already showing the buffer **does not preempt** a usable
   requested side slot; displaying the same buffer in two windows is legal and
   avoids the deferred rehoming problem.
3. **Ordinary target (no usable exact/side target).**
   1. Reuse a visible **non-side** window on this frontend already showing
      `buffer_id`. An ordinary display never selects the panel by coincidence.
   2. Otherwise use the first candidate from Q#BP11a that is not dedicated to a
      different buffer. Continue in `iter_ids()` order when the preferred
      document target is dedicated.
   3. If no eligible non-side window exists, return a pointed error; do not
      overwrite a dedicated window or create an unrequested split.

After target resolution, an omitted `action.select` defaults to **false** for
an actual side target and **true** for an ordinary target. An explicit
`select` survives fallback unchanged. Placement and reuse are strictly per
frontend.

`height` and `dedicated` are option-valued at the policy boundary; omission is
not silently equivalent to an explicit zero/false:

- creating the side slot uses `window.panel-height` when `height` is omitted
  and `dedicated = false` when dedication is omitted;
- redisplaying the **same continuously presented buffer** preserves its
  current requested height, dedication, and quit action unless an explicit
  value changes the first two;
- replacing the buffer in an existing usable side slot preserves its current
  requested height when `height` is omitted, but the new presentation defaults
  to `dedicated = false`; an explicit dedication applies only after the old
  presentation passed eligibility and cannot be used to clear-and-bypass an
  existing dedication in the same call;
- an ordinary/exact non-side replacement defaults to undedicated, while a
  same-buffer redisplay preserves existing dedication unless explicitly
  changed.

These rules let a user-resized panel keep its height as compile/listview
replace one another, prevent a harmless same-buffer redisplay from unpinning a
window, and still make every adopter's newly installed presentation
undedicated by default.

### Q#BP4 — The display transaction and the final-focus matrix (R2-4, rp-3)

Rev 2's Phase 2 *always* restored `saved_active`, which erases `select = true`
outright, and restored only when the saved window was non-side, so
`select = false` from a live focused panel blurred it. Both are wrong.

**Phase 1 (core, no Lua).** `EditorCore::display_buffer` chooses the target
(Q#BP3/Q#BP11a), installs the buffer, records `saved_active: WindowId`, returns
`DisplayOutcome { target, saved_active, select, fire: HookKind }` where
`HookKind ∈ { AfterSwitch, AfterLoad, None }`.

**Phase 2 (the Lua-owning layer).** Activate `target`, fire the hook, run
`reconcile_panel_layout(frontend_id)` (hooks may resize, close, or replace the
target), then **revalidate both ids** against `core.windows`, the frontend's
`layout.iter_ids()`, and `panel_hidden`, and apply:

| `select` | `target` after hooks | `saved_active` after hooks | Final focus |
| --- | --- | --- | --- |
| `true` | live + visible | — | **`target`** |
| `true` | dead or `panel_hidden` | live + visible | `saved_active` |
| `true` | dead or `panel_hidden` | dead or `panel_hidden` | non-side target rule |
| `false` | — | live + visible (**side or not**) | **`saved_active`** |
| `false` | live + visible | dead or `panel_hidden` | `target` |
| `false` | dead or `panel_hidden` | dead or `panel_hidden` | non-side target rule |

Two corrections encoded here: `select = true` **keeps the target selected**, and
`select = false` restores a saved window **even when it is the panel** — a
passive display invoked from a focused panel must not blur it. "Visible" means
not `panel_hidden` per Q#BP2b.

The hook-failure arms are tested in **both** `select` modes (acceptance).

### Q#BP5 — The divider is a general split-boundary drag (sub-problem 1)

`window_at_cell` maps a mode-line row to the window above it and `dispatch_mouse`
already reserves that row (`src/editor.rs:1865`). We spend that reservation.

- A leaf's outer bottom row is a **drag handle** when it is an exposed segment
  of a horizontal ancestor boundary. If that ancestor's upper child is a
  vertical/nested subtree, every leaf segment touching the same bottom edge
  paints and resolves to the **same boundary**; dragging any segment has the
  same result.
- `Down` arms `WindowDragState { frontend_id, boundary, start_row,
  start_extents }` beside `MouseClickState`; `Drag` recomputes — `fixed_rows`
  when one side is fixed, weights when both are flexible — clamped by Q#BP2's
  **interactive** recursive minimum when satisfiable, and never worsening an
  already-unsatisfied side; `Up` disarms. Selection is untouched.
- A flexible pair writes weights (ratio survives a terminal resize); a side
  window writes `fixed_rows` (absolute height survives). That difference is the
  point.
- **No pointer-shape change in the TUI** (`OSC 22` is xterm-only) — deferred by
  name. The affordance is `ui.divider` (Q#BP5a) plus keyboard parity (Q#BP5b).

### Q#BP5a — Where the divider actually is (R1-7)

- **TUI.** The divider is the upper subtree's exposed existing mode-line
  segment(s). No row is added or consumed; every adjacent leaf segment along
  that boundary renders with the reserved theme face **`ui.divider`**
  (`src/highlight.rs:234-248`) plus a handle glyph. The root panel divider is
  therefore full width even when the document subtree ends in several columns.
  **`fixed_rows` excludes it.**
- **GPU.** The projected panel grid holds the panel window's rows and **its own
  bottom mode line** — not the document's mode line, which is not part of the
  panel window. So the GPU **paints its own divider chrome**: a `ui.divider`
  rule of `BASE_DIVIDER_HEIGHT`, **frontend-local, outside the projected grid
  and outside `fixed_rows`**, exactly as the status band is chrome outside the
  document. The drag hit strip is that rule.

**The daemon is authoritative for rows**: the GPU converts pixels to rows and
sends rows (Q#BP15a), never the reverse.

### Q#BP5b — Keyboard resize, boundary resolution, and `resize(win, …)` (rp-4)

`window.enlarge` (`C-x ^`) / `window.shrink` (`C-x C-^`) act on the **active**
window. `pmacs.window.resize(win, delta_rows)` resolves from the **supplied
`win`** — the Lua entry point is explicit, the commands are implicitly active.
Both resolve the boundary identically:

1. Active/supplied window is a **side window** → its own fixed boundary.
2. Otherwise → walk up from the leaf to the **nearest horizontal-split ancestor
   at which the path child has a following sibling**, and move that boundary.
   (Rev 2 said "nearest horizontal ancestor", which is wrong when the subtree is
   that ancestor's *final* child — there is no boundary below it there.)
3. No such ancestor → report "no adjustable horizontal boundary", no-op.

Rule 2 moves the same boundary a drag on that window's bottom mode line moves —
that identity is an acceptance case, tested in a nested layout where the naïve
"nearest horizontal ancestor" reading picks the wrong one. All three
interactive entry points share the Q#BP2 preference clamp; programmatic
`display(..., {height = ...})` uses only the structural floor.

### Q#BP6 — Focus, child input, and the window guards

Input needs no new code (Q#BP1). The **guards** do:

- `close_active` (`src/editor_core.rs:2349`) must refuse **only when the target
  is the last non-side window**. Closing the side window itself is always legal,
  including as the only other window.
- `close_others` from a document window also deletes the panel; from a **side
  window it errors**. `split_active` from a side window errors.
- `focus_next/prev` include a side window only while it is visible. A hidden
  panel is never a focus destination; after it reappears, traversal reaches it
  normally (Q#BP2b/Q#BP2c).

### Q#BP7 — Panel height vs scrollback (sub-problem 3)

**Invariant: a height change is a viewport change, never a scroll change.**
`top` is preserved verbatim. Not "preserve the bottom row" — that fights
tail-follow.

1. **Growth reaching the live tail re-arms follow** (`top` → `None`) — **only
   when no selection is active** (R1-8). `selection_froze_top`
   (`src/terminal/view.rs:360`, `:422`) and `view_geometry` (`:625`) already
   encode the freeze; the re-arm goes in the **shared viewport-size path**
   (`record_view_size` / `snapshot_for_view`) so grid and semantic declarations
   agree.
2. **Shrink to zero never happens**: Q#BP2's clamp plus Q#BP2b's hide, and
   `record_view_size` already fails closed (`src/terminal/view.rs:273-296`).
3. **Only the controller resizes the PTY** (`src/editor.rs:1234-1249`); two
   frontends may hold different panel heights over one child. Pinned, not
   "fixed".
4. **A semantic panel terminal sizes from the panel content rect.** At the
   existing pre-child-drain terminal-sync point, the daemon resolves the
   visible side window against `frame_geometry`, derives
   `(fixed_rows - mode_line) × total.cols`, records that exact
   `TerminalViewKey` size, and resizes the PTY only if it is the controller.
   It never consumes the GPU attach `term_sizes` placeholder or the
   full-document `TerminalResize` declaration.

### Q#BP8 — The GPU panel band (Stage 2)

The daemon projects the frontend's bottom side window into a standalone cell
grid; the GPU paints it as a band above the status band and shrinks its text
area by band + divider height. The panel projection is a sibling pass: it runs
independently of whether the primary document has declared a byte viewport or
is in full-window terminal mode, so neither existing early return can suppress
the band.

- **Extraction, not new rendering**: `paint_frame`'s per-window body already
  paints one window into a `CellGrid` through an origin-agnostic `Viewport<'a>`
  (`src/editor.rs:2937-3040`); painting into a panel-sized grid at `(0,0)` is
  that body lifted out. The extraction also takes the active-window
  cursor-visible preparation currently just before the loop
  (`src/editor.rs:2883-2935`): it runs for the panel only when that window owns
  focus, uses the same supplied fold map, and leaves passive `view_top`
  untouched. No concrete text/gutter/overlay/modeline painter forks (Bet B2').
  `pmacs-gpu/src/terminal.rs` is already a pure cell-space planner for this
  payload shape.
- **The extraction boundary is per-window, not per-frame.** Text, gutter,
  selection, mode line, and window-attached overlays (including a panel's
  `SearchView` / `MenuView` / `CompletionView`) enter `PanelFrame`. The
  frame-global status row, search prompt, and minibuffer do not; semantic focus
  chrome carries those surfaces per Q#BP14b.
- **Statusline callbacks still run once.** Generalize the existing
  `StatuslineEvaluationTarget::Grid` fan-out into a frontend-layout target:
  the grid target keeps today's layout-leaf fan-out but omits a derived-hidden
  side, while a semantic panel target captures exactly the primary document
  window plus its visible side window (unprojected document splits do not run
  callbacks). Evaluate before paint, then transactionally revalidate as today.
  Route the primary document result to semantic `StatuslineSegments` and the
  side result to the panel mode line. A callback mutation runs Q#BP2b
  reconciliation before either result is consumed, and an invalidated
  evaluation paints no stale text. This closes the indirect `view.active` read
  at `src/statusline.rs:634` without evaluating a provider twice.
- **One transport for every panel kind** — a terminal panel is painted
  daemon-side by the same `paint_terminal_snapshot` the TUI uses.
- **Accepted consequence**: panels are **monospace cell grids** in the GPU.
  Documents keep the rich renderer.
- **Document declarations follow the installed band.** Applying
  `Present`/`Absent` recomputes the GPU document clip and emits the ordinary
  document `Viewport` or full-window `TerminalResize` if its effective size
  changed. `FrontendCellGeometry` does not change in response—the whole-frame
  declaration deliberately excludes panel presence—so this cannot feed back
  into panel sizing.
- **Discipline inherited from `TerminalFrame`**: whole-grid replacement,
  `validate` both sides, atomic rejection retaining the previous valid frame,
  duplicate suppression on the complete ordered payload, byte-bounded payload.

### Q#BP14 — One authoritative primary-document context (R2-1, R3-B1)

Rev 2 proposed `primary_document_window(fid)` and named three couplings. The
transitive §1.3 census now finds twenty-three. The projection contract:

**Definition.** `EditorCore::primary_document_window(fid) -> Option<WindowId>` —
the frontend's active window when it is non-side, else its non-side target
(Q#BP11a). **Every consumer classified Projection in §1.3 (#1–#12 and
#21–#22) routes through it.** Focus and surface-routed consumers follow
Q#BP14b.

The census rule is transitive: a new call to `active_window_for`,
`active_window`, `active_buffer_id`, or any helper that reaches one of them in
the daemon/semantic projection — including helpers implemented in
`src/editor.rs` or `src/statusline.rs` — must add or reaffirm its
classification. This is a review checklist item, not a lint; acceptance pins
each class.

**The alignment helper splits in two.** `align_semantic_window_to_buffer`
(`src/daemon.rs:2900`) unconditionally rewrites `view.active`'s buffer, which is
exactly why rejecting panel-named events does not fix the *document* event —
with the panel focused, an ordinary document `Viewport` overwrites the panel's
buffer with the document buffer.

- **`align_primary_document_window(fid, buffer_id)`** — rewrites the **primary
  document window's** buffer/`TextView`/cursor. **Never touches `view.active`.**
  Used by `Viewport` (#7).
- **Document `Pointer` (#8)** calls the same aligner **and then activates the
  primary document window** before dispatching the gesture — a click in the
  document area means "work here", so it moves focus out of the panel. This is
  the one place projection and focus legitimately move together.
- **The `Viewport` terminal-context guard (#9)** tests the primary document
  window plus the declared buffer, never the focused panel. A terminal panel
  therefore cannot reject the still-visible document's viewport.
- **Existing full-window terminal transport (#10–#11)** remains the document
  surface. `TerminalResize`, terminal snapshot/sync, and terminal-frame
  suppression resolve a terminal key from the primary document window.
  `TerminalPointer` validates against that declaration; any accepted
  non-`Move` gesture activates the primary document window before replaying the
  existing terminal gesture, while hover neither focuses nor claims control.
  Panel terminals use `PanelFrame`/`PanelPointer`, never these declarations.
- **Statusline evaluation (#12)** uses Q#BP8's one frontend-layout fan-out:
  primary-document segments remain on the semantic document status band while
  the side context paints only in the panel mode line.
- **Semantic snapshot publication (#21)** tests whether a recipient displays
  the published buffer through that recipient's primary document window. This
  predicate is shared by lazy-upgrade and #148 initial-target publication:
  panel-only visibility never swaps the document mirror, while panel focus
  never hides a matching document surface from the publication.
- **Fresh no-target view construction (#22)** clones the buffer in
  `primary_document_window(FrontendId::LOCAL)`, not `local_view.active`. A TUI
  panel may own focus at attach without becoming the new frontend's
  full-window document. The new view still starts as one ordinary leaf focused
  on that inherited document buffer.
- **`PanelPointer` (Q#BP16)** activates the **panel**.

So: `Viewport` never steals focus; document clicks take it; panel clicks give
it back. And because #1–#12 plus #21–#22 use the
primary-document/surface split, focusing the panel re-sends no snapshot,
suppresses no document, swaps no mirror, clears no document terminal or
statusline declaration, and cannot leak into a newly attached document view.

**“Active buffer” in the semantic replica is now a document-surface term, not
an input-focus term.** Stage 2 audits and updates the contracts/comments/tests
for `InstanceMessage::CursorByte`, `BufferMirror::active_buffer`,
`SemanticRenderState`, `StatusFacts`, `LineNumbers`, `StatuslineSegments`, and
`TerminalFrame`: for a panel-capable semantic session these identify the
primary document declaration/mirror while a panel may separately own focus.
No wire field is renamed and legacy/grid behavior is unchanged; grid clients
already discard the semantic families. `DispatchIdle`, authenticated input,
presence, and Q#BP14b remain the authorities for actual focus. This vocabulary
split is load-bearing—leaving “active means focused” in the replica contract
invites a later producer to reintroduce the mirror swap.

**The lazy CRDT upgrade (#2) is the sharpest case** and gets its own rule: the
upgrade + broadcast (`src/daemon.rs:1096`) keys on the **primary document
window**, so focusing a fresh generated panel buffer never broadcasts a
`BufferSnapshot` for it. A panel buffer that genuinely needs CRDT backing gets
it when it is displayed as a document, not as a side effect of focus.

### Q#BP14a — Panel input gating is per-window, not per-buffer (R2-2)

Rev 2 proposed auto-marking every side window's buffer round-trip, with an
opt-out. Both are wrong. `round_trip_buffers` is a **global set keyed by
`BufferId`** across every frontend and window (`src/editor_core.rs:349`), so
marking buffer A because *one* frontend panels it disables optimistic input for
another frontend editing A as its document; replacement and close would need
reference counting plus preservation of any pre-existing mark. And an opt-out is
unsafe: with the panel focused, the GPU would optimistically edit its document
mirror while daemon input targets the panel, and every resulting op fails
remote-op validation (#13, `src/daemon.rs:2531-2537`) — a silent mirror
divergence. The accepted-op cursor/provenance path (#23) intentionally retains
the focused source window; it is not redirected to the primary document.

**The rule: `dispatch_idle_for` returns `false` whenever the acting frontend's
active window is a side window**, independently of the buffer-global set
(`src/editor.rs:753-769`). No auto-marking, no reference counting, no opt-out.
Existing `listview` / compile / terminal marks stay exactly as they are and keep
governing their full-window behavior.

This is **one panel-aware producer condition** — which is why B1 is narrowed to
terminal controller/escape routing rather than claiming all input gating is free.

### Q#BP14b — Focus chrome and per-window overlay routing (R3-B1, R3-rp4)

The semantic producer gains a **focus-chrome pass** that runs once per semantic
frontend independently of whether a document viewport exists and independently
of the document/terminal projection pass. It reads modal state through the
acting frontend's focused context, never through `vp.buffer_id`.

| Surface | Document focused | Panel focused |
| --- | --- | --- |
| Search | `SearchPrompt` on the semantic status band; document `SearchView` supplies washes | `SearchPrompt` still uses the semantic status band; panel `SearchView` washes are in `PanelFrame` |
| Minibuffer | `MinibufferPrompt` | `MinibufferPrompt` — it is global and bufferless |
| Menu | `MenuPrompt` native popup; no document cell-grid menu | `MenuView` is painted in `PanelFrame`; semantic `MenuPrompt` emits/retains authoritative empty |
| Completion | `CompletionPopup` native popup | `CompletionView` is painted in `PanelFrame`; semantic `CompletionPopup` emits/retains authoritative close |

The menu/completion baselines track the **currently owned surface**, not merely
a per-buffer payload. A document→panel focus change therefore emits the clear
for a formerly open native popup even if the focused panel carries a different
buffer; a panel→document change cannot leave a pre-painted panel popup in
native GPU state. `BufferSnapshot` baseline resets audit both the open and clear
mirrors, following the #120 rule.

No new focus-owner wire field is needed: a current
`PanelFrame::Present { buffer_id, focused: true, ... }` is the authenticated
panel-surface declaration. A surface transition is ordered:
**authoritative closes for the old owner → new `PanelFrame`
focus/presence → opens/updates for the new owner**. Thus a panel-owned search
clear is accepted while the old focused declaration still exists; only then
may `Absent` or `focused = false` remove that authority. Conversely, a newly
panel-owned prompt follows the `focused = true` frame it relies on. The GPU
accepts `SearchPrompt { buffer_id, ... }` when `buffer_id` matches either its
primary document mirror or its current focused `Present` panel; the latter
still renders prompt text in the semantic status band while match washes come
only from the panel grid. A prompt naming neither surface is stale and is
dropped without changing the current prompt.
Document-native completion validates against the document as today; panel
completion/menu opens only inside `PanelFrame`, while the semantic native
variants carry authoritative close. `MinibufferPrompt` remains bufferless.

Focus consumers #13–#15 and #23 keep the focused window. #16–#19 use the
routing table above. Bell drain #20 keeps its per-session counter but uses the
focused window to choose the eligible terminal; passive/historical bells
remain baseline-suppressed exactly as today.

### Q#BP15 — `PanelFrame` lifecycle (R1-2)

- **Explicit presence.** `InstanceMessage::PanelFrame(PanelFramePayload)` where
  the payload is `Present(PanelFrame)` | `Absent`. **`Absent` is authoritative
  and must be sent** on close *and* on hide (Q#BP2b) — silence would leave the
  last valid frame on screen forever under the retain-on-invalid rule. `Absent`
  is duplicate-suppressed like any payload.
- **Cursor and focus travel with the frame.** `paint_frame` returns the cursor
  separately (`src/editor.rs:2833`), so cells alone lose the caret.
  `PanelFrame` carries `cursor: Option<CellCoord>` and `focused: bool` — the GPU
  paints the band caret only when the panel owns focus. `focused` is
  presentation/focus-chrome routing only (Q#BP14b); the *keys* decision is
  `DispatchIdle` (Q#BP14a).
- **Presentation identity.** `PanelFrame` carries `buffer_id` **and**
  `panel_epoch: u64`, plus the frontend-owned `geometry_epoch` it is answering.
  The panel epoch is opaque and monotonic per frontend. It stays stable across
  ordinary frames of one continuously present window/buffer, and changes on
  buffer replacement, new side-window creation, and every
  `Absent`→`Present` transition. Thus closing/hiding and reopening the same
  persistent buffer cannot reuse the identity of an old frame (Q#BP16).
  Allocation is checked; exhaustion fails closed to `Absent` rather than
  wrapping into a stale identity.
  `geometry_epoch` is different: it changes whenever the frontend declares new
  effective cell geometry, even if the panel presentation is otherwise the
  same (Q#BP15a).
- **Absent clears input authority.** Emitting or applying `Absent` clears the
  last declared panel size and **panel** epoch on both sides before any later
  event can validate. Whole-frame geometry remains valid until superseded by a
  newer authenticated declaration.
- **Cell-grid validation is shared, terminal dimensions are not.** Factor the
  cell count, cursor, glyph width/continuation topology, aggregate glyph-byte,
  visible-cell, and transport-safety checks out of
  `pmacs-protocol/src/terminal.rs` into one parameterized wire-cell-grid
  validator. `TerminalFrame` still adds its PTY-specific
  `MAX_TERMINAL_ROWS/COLS = 512`; `PanelFrame` does **not** inherit that
  per-axis cap. A common 4K/small-font panel wider than 512 columns remains
  legal as long as its checked area and aggregate glyph bytes fit the shared
  wire budget (Bet B5').

### Q#BP15a — Three geometries, two messages, one exact conversion
(R2-3, R3-B2, R3-B7)

Rev 2's `PanelResize { size: CellSize }` conflated the frontend's total frame,
the requested panel rows, and the resulting grid — and created a **first-open
cycle**: the declaration was gated on a side window existing, but the daemon
needs columns before it can paint the first frame. The GPU's attach `CellSize`
cannot fill the gap: it is permanently the placeholder `24×80`
(`pmacs-gpu/src/attach.rs:420-429`, `:573-577`) and no resize updates it.

Two messages with different lifetimes:

- **`FrontendEvent::FrontendCellGeometry { frontend_id, geometry_epoch,
  total: CellSize }`** — the frontend's authoritative cell-equivalent layout
  capacity. It is valid **without a side window**, sent immediately after
  attach acceptance and refreshed on **window resize, font change, and scale
  change**. `geometry_epoch` is a checked, monotonically increasing
  frontend-owned declaration id; exhaustion fails closed rather than wrapping,
  and a lower/repeated epoch with different data is stale/invalid. The event is
  accepted only from the authenticated, negotiated
  panel-capable semantic session; the word "without" refers to side-window
  presence, not protocol/session gates.
- **`FrontendEvent::PanelResizeRows { frontend_id, geometry_epoch,
  panel_epoch, rows }`** — the requested fixed panel rows from a divider drag.
  Its only size component is rows; the epochs are identities, not geometry. It
  is accepted only for the currently visible `Present` panel matching both the
  latest geometry declaration and presentation epoch, then clamped by Q#BP2's
  interactive preference.

Both events join `pmacs-gpu/src/attach.rs`'s bounded outbox policy as distinct
same-kind **tail-coalescible** classes. Geometry is latest-wins (epochs need
only increase, not be consecutive); resize drag is latest-wins over the
complete event, including its epochs, so a new presentation may supersede a
queued stale drag. Tail-only replacement preserves ordering across a click,
key, `PanelPointer`, or geometry transition, and daemon-side epoch validation
still rejects anything stale. Neither human-rate stream consumes the 8192
lossless-event budget while the writer is stalled.

The GPU declares **whole-cell capacity**, not pixels or a guessed grid. For
current GPU geometry:

```
available_height_px =
    max(0, surface_height_px
           - status_band_height_px
           - TEXT_TOP_px
           - divider_height_px)

layout_rows = floor(available_height_px / code_line_height_px)
total.rows  = layout_rows + 1  // virtual daemon status row
total.cols  = floor(surface_width_px / resolved_monospace_advance_px)
```

All quantities use the frontend's current scale. `divider_height_px` is the
scaled frontend-local divider reserved **for sizing purposes even while the
panel is absent**; this keeps the declaration independent of panel presence
and breaks the first-open cycle. The document renderer does not actually lose
those divider pixels until a `Present` panel is painted. `total.cols` describes
the full-width panel grid beginning at x=0; document `TEXT_LEFT`/gutter padding
is unrelated. Only full cells count. While the band is present, any fractional
right-edge remainder is painted as panel background but maps to no cell and
emits no `PanelPointer`; above the band, the document keeps its normal full
pixel width.

The conversion accepts only finite, positive line-height/advance metrics and
uses checked/saturating conversion to `u32`. A zero surface, non-finite metric,
or non-positive advance/line height declares zero usable geometry under a new
epoch and therefore hides the panel; it never divides, wraps, or emits a giant
grid. Aggregate area validation still applies after conversion.

The added row is virtual because the shared grid placement helper subtracts one
global status row before laying out windows. The GPU's real status band remains
pixel chrome; it is not painted into `PanelFrame`.

**The daemon derives the third geometry.** Panel grid cols = `total.cols`.
Rows are `fixed_rows` clamped per Q#BP2 against `total` and, for a semantic
panel, by `shared_visible_cell_budget / total.cols`; if that wire-area cap is
below the structural two-row floor, the panel follows Q#BP2b's hidden arm.
The requested `fixed_rows` remains stored, so a later narrower geometry can
restore it. The daemon paints and ships the resulting grid in `PanelFrame`; the
GPU never asserts its size. The rendered band is exactly
`grid.rows * code_line_height_px`; divider and status-band pixels remain
frontend chrome, so document shrink is exact and contains no row-rounding
feedback loop. For a terminal panel, the grid's content rows exclude its one
mode line and feed Q#BP7's pre-drain terminal view/controller sync before the
snapshot is painted.

**Unknown is first-class.** A semantic `FrontendView` starts with
`frame_geometry = None`; the daemon must not consult the attach request's 24×80
placeholder for panel layout. A panel requested before the first real
declaration remains non-presentable under Q#BP2b. The GPU sends geometry before
enabling user input; receipt stores it, reconciles visibility, and permits the
first `Present`. Grid/LOCAL frontends continue to populate the same cached field
from their existing real attach/resize sizes and never send this new event.

**Geometry changes fail closed.** As soon as the GPU sends a new
`geometry_epoch`, it retains but does not paint or hit-test an older
`PanelFrame`; only a matching `Present` can make the band visible and
interactive again. An `Absent` is always safe to apply because it only removes
paint/input authority. Every `Present` echoes the daemon's latest accepted
geometry epoch. This is the font/scale/resize analogue of terminal-frame size
validation and prevents an old grid from being interpreted under new metrics.

### Q#BP16 — GPU panel pointer transport and presentation identity
(R1-3, R2-7, R3-B3)

Existing events cannot carry panel gestures: semantic `Pointer` carries a
**document byte**, `TerminalPointer` is keyed to a terminal buffer, and `Mouse`
is contractually the **grid** path (`src/daemon.rs:3122-3130` drops terminal
declarations from grid sessions for exactly this reason).

`FrontendEvent::PanelPointer { frontend_id, geometry_epoch, panel_epoch,
buffer_id, coord: CellCoord, kind: MouseKind, mods: Modifiers }`.

`buffer_id` catches A→B replacement, but it cannot catch close/hide/reopen of
the **same** persistent buffer. `panel_epoch` closes that hole without putting
`WindowId` on the wire. `PanelPointer` is validated in this order:

1. The authenticated source negotiated the panel event and matches/owns the
   claimed `frontend_id`.
2. Its `FrontendView` has a live side window that is **not
   `panel_hidden`**, and its latest daemon→frontend declaration is `Present`.
3. The payload's `geometry_epoch` equals both the latest accepted frontend
   geometry and the echoed epoch in that `Present`.
4. The payload's `panel_epoch` equals that declaration's presentation epoch.
5. The side window's current `buffer_id` equals the payload's.
6. `coord` is inside that declaration's panel size.

`Absent` clears steps 4–6's presentation state. Any failure drops the event
before any view, controller, selection, menu, or PTY mutation. A
`PanelResizeRows` follows the same
source/visible/Present/geometry-epoch/panel-epoch validation before changing
`fixed_rows`.

`PanelPointer` events whose `kind` is `Move` or `Drag` receive their own
same-kind tail-coalescing tags beside document/terminal motion and drag.
Every `Down`/`Up` and wheel step remains lossless and ordered: repeated left
`Down`s are what the existing daemon click state interprets as a multi-click,
and `Down(Right)` is the context-menu gesture, so neither may collapse. The
event's geometry/presentation identities remain part of daemon validation;
coalescing never crosses an intervening event or combines different kinds.

Once accepted, the daemon re-derives the panel window and replays existing
semantics: a terminal panel takes the Stage 2 vterm pointer path (child SGR
reporting when eligible, else per-view scroll/selection/menu); otherwise the
ordinary document gesture path in cell space. Click-to-focus is a `Down` on the
band; it activates the panel and, per Q#BP14, does **not** disturb the document
mirror. One terminal-specific consequence is explicit: every accepted
non-`Move` terminal gesture activates the panel before the shared terminal
adapter runs, because that adapter deliberately claims the controller for
wheel/press/drag/release as well as clicks. Bare hover neither focuses nor
claims. Non-terminal wheel motion keeps today's scroll-without-focus behavior.

### Q#BP17 — Fold projection for the panel grid

Folding asserts *"a semantic session never enters `paint_frame`"* and builds the
per-window map **ungated** on that basis (`src/editor.rs:2991`). The panel band
breaks the premise.

**Rule: the panel projection honors the owning frontend's `fold_projection`.**
The extracted per-window painter takes the map as a **parameter** rather than
building it; the panel path passes `None` when the owning frontend's
`fold_projection` is false. The panel path must **not** call
`EditorCore::fold_map_for_window`, which gates on the **active** frontend
(`src/editor_core.rs:566`) — right for command-time reckoning, wrong for
painting another frontend's panel. **Updating the now-stale invariant comment at
`src/window.rs:339` is part of Stage 2.**

### Q#BP9 — Protocol: Stage 1 none; Stage 2 takes the next available version

- Stage 1 changes no wire shape. The reviewed base is v20 after #148 and Stage
  1 inherits it without adding or reserving another version.
- Stage 2 appends `InstanceMessage::PanelFrame` after whatever that enum's final
  variant is at the time, and appends
  `FrontendEvent::{FrontendCellGeometry, PanelResizeRows, PanelPointer}` after
  that enum's final variant. **Each extended enum gets a byte pin on its own
  previous final variant's discriminant.** On `0dd16a5`, those pins are
  `InstanceMessage::InitialTargetResult` and
  `FrontendEvent::TerminalPointer`. Gated in both directions.
- **No future version is reserved.** Stage 2 takes the next available version
  at implementation time—v21 if no intervening protocol PR lands—per
  `docs/dap-debugging-framing.md` Q#DAP8.
- **Every gate keys on the daemon's own state.** All three events require an
  authenticated semantic session whose claimed `frontend_id` equals the
  transport source and that negotiated the panel version/capability.
  `PanelResizeRows` / `PanelPointer` additionally require the current visible
  `Present` declaration and matching geometry/presentation epochs;
  `FrontendCellGeometry` deliberately does **not** require a side window
  (Q#BP15a). A grid session or pre-panel semantic peer sending any new event is
  rejected before payload state is trusted.

### Q#BP10 — Persistence: side windows are not saved

`src/desktop.rs`'s save walk skips side leaves; restore never creates one.
The v1 `SavedLeaf` shape remains unchanged: every restored ordinary window gets
default `WindowParams` (`side/fixed_rows/quit_action/origin_document` empty,
`dedicated = false`). Thus this arc does not bump `DESKTOP_VERSION` merely to
persist transient display policy.
Deferred: persisting panel geometry as a setting (blocked on settings
persistence).

### Q#BP10a — Killing a panel buffer (rp-2 of round 1)

`kill_buffer` redirects **every** window showing the victim to `*scratch*`
(`src/editor_core.rs:3046`). For a side window that is wrong twice.

- Killing the buffer in a **side window closes the side window** (Q#BP2a
  collapse) rather than redirecting it.
- If that would leave no non-side window — impossible under Q#BP6, asserted
  anyway — the wrapper collapse restores the prior root, which by construction
  holds a leaf.
- `QuitAction::Restore { buffer_id, .. }` **revalidates** at quit time; a
  killed target degrades to `Delete`. This lifts `listview`'s existing fallback
  (`builtin/runtime/listview.lua:164-166`) into the core.

### Q#BP11 — Lua surface

```lua
pmacs.window.display(buf, { side = "bottom", height = 12,
                            dedicated = true, select = false })
pmacs.window.display_file(path, { window = win, select = true })  -- Q#BP11b
pmacs.window.quit()
pmacs.window.panel()
pmacs.window.params(win) / set_params(win, {...})   -- side/origin/quit action are read-only
pmacs.window.resize(win, delta_rows)                -- boundary per Q#BP5b
pmacs.window.display_target()                       -- the non-side target
```

Commands: `window.quit`, `window.enlarge`, `window.shrink`. Settings:
`window.panel-height` (default 12 outer rows), `window.min-height` (Q#BP2).
Every Lua operation taking a `WindowId` validates that it is live and belongs
to the acting frontend's layout; a cross-frontend id is a pointed error before
read or mutation.

### Q#BP11a — The non-side target rule (R1-5)

1. Selected window is **not** side → it is the target (byte-identical to today).
2. Else the **remembered document window** (`origin_document`, Q#BP2c) if it
   revalidates.
3. Else the **first non-side window in `iter_ids()` order**.
4. Else (no non-side window — forbidden as a resting state by Q#BP6) →
   `debug_assert!` the broken invariant and return a pointed error without
   mutation. There is no document leaf from which a valid fallback can be
   fabricated.

### Q#BP11b — A target-aware load transaction (R2-5)

`display_target()` returns a *window*, but Lua has **no operation that loads or
switches into an arbitrary window**. `pmacs.buffer.find_or_open` switches the
**active** window in both branches before firing hooks
(`src/lua_bindings/mod.rs:3089`, `:3108`, `:3113`), and LSP
(`builtin/runtime/lsp.lua:1597`) and compile (`builtin/runtime/compile.lua:869`)
call it directly. #148's private `open_initial_target`
(`src/daemon.rs:1625-1677`) proves the useful off-ambient load seam
(`EditorCore::get_or_load_buffer`), but it too installs and reasserts through
`switch_active_buffer_for`, so it is not an arbitrary-window API. A visit to a
**previously unopened file** would still replace a focused panel before
`display_buffer` could help. Rev 2's Q#BP4 also covered only `after-switch`,
while a fresh load must fire `after-load` with the **document target** active.

**`pmacs.window.display_file(path, { window, select })`** — one transaction:

1. Construct the same path key `find_or_open` uses and perform its
   side-effect-free registry dedup; do **not** read the file yet.
2. Resolve the destination before I/O. An explicit `window` is an **exact
   target** under Q#BP3, not a hint; it must be live, owned by this frontend,
   and not dedicated to a different already-open buffer (or, on a miss, to any
   buffer). With no explicit window, an existing buffer uses Q#BP3's ordinary
   non-side reuse/candidate policy; a miss chooses the first non-dedicated
   Q#BP11a candidate. No eligible target is an error **before loading**.
3. On a registry miss, load/create the buffer; on a hit, preserve its unsaved
   contents exactly as `find_or_open` does.
4. Enter Q#BP4's transaction with `fire = AfterLoad` on a fresh load,
   `AfterSwitch` on a reuse (including a same-buffer no-op), and `None` for a
   newly created `NotFound` path, matching #148/local-startup behavior — so any
   hook observes the **document target** as active, which saveplace / recentf /
   syntax / LSP all require.
5. Apply Q#BP4's final-focus matrix.

The implementation factors one Rust/editor-core **resolve/load-without-switch**
primitive and one exact-window install primitive for both `display_file` and
#148's `open_initial_target`; the daemon bootstrap does not call back through
the public Lua binding. This prevents two path-normalization, dedup, and hook
transactions from drifting.

Initial-target bootstrap retains its stronger Q#GT5/Q#GT8 postcondition. It
captures the fresh view's original document window before I/O and runs the
shared exact-window transaction with `select = true`. After its one hook, it
revalidates the target `BufferId`: removal is still bootstrap failure. If the
original document window remains live, reassert the target there and activate
it; if a hook closed that window, resolve an eligible non-side window in the
same new frontend, install the target there without firing a second hook, and
activate it. A hook-created/selected side window is never overwritten merely
because it became `view.active`. Snapshot publication and
`InitialTargetResult::Opened` retain #148's existing order and name the
reasserted document buffer.

Adopters route through this: `listview` visit
(`builtin/runtime/listview.lua:126`), LSP `visit_location`, compile
`visit_error`. Raw `find_or_open` stays for programmatic use.

**Stage 1 also needs real opt-in entry points**, because compile, terminal, and
listview currently create/switch their buffers through active-window-only
paths. Calling a generic display afterward was the rev-3 vacuous path; Q#BP3's
placement-aware rule and these entry points make the requested side placement
the first real display:

All three parse the same strict placement value:
`display = "current" | "panel"`. Unknown values error before creating a buffer,
session, process, or wrapper. In Stages 1–2, omission means `"current"`; in
Stage 3, omission means `"panel"`. Explicit `"current"` always preserves the
adopter's pre-arc selected-window behavior and is the user-facing opt-out from
the default flip.

- `pmacs.terminal.open{ display = "panel" }` — `pmacs.terminal.open` hardwires
  `switch_active_buffer_for(frontend_id, …)` into the active window
  (`src/lua_bindings/mod.rs:8500`) and rolls the session back on failure. The
  binding takes an optional exact target window (mutually exclusive with
  `display = "panel"`), defaulting to today's behavior; the panel opt-in uses
  `select = true`. Placement failure removes any side wrapper created by the
  transaction before the existing session/buffer rollback completes.
- `compile.run{ display = "panel" }` — compile creates its buffer
  (`compile.lua:263`) then `switch_buffer`s (`:808`); the first display becomes
  a side-affine `display` call even when an older document window already shows
  `*compilation*`, explicitly with `select = false`. Recompile reuses the
  current panel. `compile.quit` routes through `pmacs.window.quit` when the
  compilation buffer is in a side window, so it deletes/restores the
  presentation instead of leaving a source buffer stranded in the side slot.
  In capability fallback it keeps today's previous-buffer restore in the
  selected document window.
- `pmacs.listview.open{ ..., display = "panel" }` — `listview.open` currently
  hardwires `switch_buffer` (`listview.lua:126`). The opt-in calls
  `display(..., {side = "bottom", select = true})`, because `seat_cursor` and
  refresh are active-window-only. `listview.quit` keeps the same `q` command and
  user-visible behavior, delegating to `pmacs.window.quit` only when the
  listview is in a side window; capability fallback retains the current
  previous-buffer switch.

For all three adopters the default panel is **undedicated**, so the one side
slot can be replaced. Creating a new side slot records
`Some(QuitAction::Delete)`. Replacing it snapshots the prior buffer, height,
dedication, cursor/view/selection state, and quit action into
`Some(QuitAction::Restore { … })`; merely redisplaying the same buffer
preserves its action. `origin_document` belongs to the slot lifetime: a
replacement retains the existing valid origin rather than remembering the
currently focused panel.

`window.quit` executes through Q#BP4's activate–switch-hook–reconcile
transaction. Restoring C→B→A reinstalls each saved presentation and its
`then`; executing `Delete` collapses the wrapper and focuses the revalidated
origin/non-side target. Capability fallback creates no window-level quit
action and leaves no side parameters behind; each adopter uses its existing
ordinary document-window restore path.

Acceptance pre-seeds the persistent listview/compilation buffer in a document
window before asking for panel placement. That is the bite against accidentally
restoring global reuse-first.

### Q#BP11c — Jump-ring origins (R2-6)

The jump ring stores only `(BufferId, Position)` (`src/editor_core.rs:279`), and
`jump_back` switches the **currently active** window to that buffer
(`src/editor_core.rs:811`). After `RET` from an outline or compilation panel,
`M-,` would put the **panel buffer into the document window** while the panel
stays open — a duplicate-buffer/window corruption, and a regression of today's
"M-, returns to the panel row" behavior.

**History becomes per frontend**, matching `command_history`:
`HashMap<FrontendId, Vec<JumpEntry>>`, where `JumpEntry` is
`{ window_id, buffer_id, position, side_origin }`. `push_jump` and `jump_back`
address only the acting frontend's vector; detach purges it. One frontend can
therefore neither pop nor destroy another frontend's navigation trail.
`JUMP_RING_CAP` applies independently to each vector with today's oldest-entry
eviction.

`jump_back` restores into the **origin window** only when all of these
revalidate: the window is live, belongs to the acting frontend's layout, is not
hidden when side, **and still shows the recorded `BufferId`**. A live panel
that has since been replaced does not resurrect its old buffer. When validation
fails for a **non-side** origin, the entry degrades to today's active-window
switch behavior within the same acting frontend. When it fails for a recorded
**side** origin (closed, hidden, replaced, or moved out of the layout), the
entry is skipped: switching its buffer into the document window would recreate
the duplicate-panel corruption this design is meant to remove. Entries whose
buffer is gone are likewise skipped.

Acceptance runs the real paths: **panel → `RET` source → `M-,`** for both
outline and compilation, asserting focus returns to the **existing** panel with
its row restored and the document window unchanged. A second acceptance
interleaves two frontends' jump histories and replaces one origin window's
buffer before `M-,`.

### Q#BP12 — Default placement flips in Stage 3

Stage 1 ships the mechanism **opt-in**; existing acceptance suites keep their
meaning. Between Stage 1 and Stage 2 a semantic frontend could hold a side
window it cannot render, so the flip waits.

**Stage 3 is not "one line per consumer"**: each adopter also moves its visit
path onto `display_file`/`display_target` and takes its own `select` decision:

| Adopter | Panel placement | Dedicated | Quit action | Visit | `select` on visit |
| --- | --- | --- | --- | --- | --- |
| `listview` (references/outline) | panel, `select = true` | `false` | delete if created; restore replaced panel | `display_file` | `true` |
| compile output | panel, `select = false` | `false` | delete if created; restore replaced panel | `display_file` | `true` |
| terminal | panel, `select = true` | `false` | delete if created; restore replaced panel | n/a | n/a |
| DAP stack/variables | panel, `select = true` | `false` | delete if created; restore replaced panel | `display_file` | `true` |

An interactive `listview` **must** take `select = true`: `seat_cursor`
(`builtin/runtime/listview.lua:64`) and `listview.refresh` are active-window-only
and would silently seat the wrong window otherwise.

The Stage 3 default is resolved as a panel request and therefore still passes
through Q#BP13 capability fallback. It is not a hidden global setting.
Explicit `display = "current"` bypasses side placement deliberately and keeps
the old adopter-specific quit/previous-buffer path; like today's entry points,
it uses the raw switch escape and does not consult display-policy dedication.

### Q#BP13 — Panel capability: a per-`FrontendView` bit set at attach (R1-6)

```rust
pub struct FrontendView {
    pub layout: Layout,
    pub active: WindowId,
    pub fold_projection: bool,              // Arc 6 Stage 2
    pub panel_capable: bool,                // this arc; no Default
    pub frame_geometry: Option<DeclaredFrameGeometry>, // epoch + total; None != 24x80
    pub panel_hidden: bool,                 // cached derived state, never persisted
}
```

Set in the attach transaction that already computes `fold_projection`
(`src/daemon.rs:1769`) from `SessionState` (`src/presence.rs:74-84`):

| Session | `panel_capable` |
| --- | --- |
| `FrontendId::LOCAL` / grid | `true` |
| semantic, `negotiated_protocol_version < PANEL_MIN_VERSION` | `false` |
| semantic, `>= PANEL_MIN_VERSION` | `true` |

`peer_declared_terminal_support` (`src/daemon.rs:888`) is the helper shape.
`peer_declared_panel_support` is explicitly
`semantic_render && negotiated_protocol_version >= PANEL_MIN_VERSION`; no
client-asserted standalone boolean is trusted. Stage 1 sets `true` for
grid/LOCAL, `false` for every semantic session; Stage 2 flips the version arm
on. `display_buffer` with a `side` falls back to the non-side target **and
discards every side-specific parameter** (Q#BP2c).

Grid/LOCAL construction supplies real geometry before first input/render.
Semantic construction supplies `None`; Stage 2's authenticated declaration
fills it. Desktop restore spells all fields explicitly, preserving folding's
non-`Default` discipline. Stage 2 additionally holds the current presentation
epoch/declaration beside the semantic render baseline; it is runtime-only and
never desktop state. The same constructor inherits its initial buffer through
Q#BP14's `primary_document_window(LOCAL)`, so adding the capability fields
cannot preserve the old panel-focused attach leak.

## 4. Bets (explicit, falsifiable)

- **B1 (narrowed after R2-2) — panel-as-window means the terminal controller,
  the `C-c` escape, and release-on-blur need zero new code.** Falsified if any
  `TerminalViewKey` / `TerminalController` / escape-dispatch code needs a panel
  case. *Input gating is explicitly excluded: Q#BP14a is one new condition.*
- **B2' (narrowed after R3-B20) — the active-window preparation plus
  `paint_frame`'s per-window body extract to a standalone panel grid without
  modifying a concrete painter.** Falsified if a text/gutter/overlay/modeline
  painter reads absolute frame coordinates or `term_size` rather than its
  `Viewport<'a>`/placement, or if the shared preparation cannot keep a focused
  panel cursor visible.
- **B3 — the terminal's anchor model absorbs height changes with no new state.**
  Falsified if Q#BP7 needs a new `TerminalViewState` field.
- **B4 — no document painter breaks when a window's rect becomes fixed rather
  than proportional.** Falsified if any painter assumes the flexible-remainder
  rule.
- **B5' (narrowed after R3-B21) — `PanelFrame` reuses one factored wire-cell
  validator and aggregate area/glyph/transport budgets, but not terminal PTY
  per-axis limits.** Falsified if panel cells need a second glyph/topology
  implementation or if a legal >512-column, area-bounded panel cannot
  round-trip.
- **B6 (restated after rp-3) — opening a panel leaves the prior document
  subtree's STRUCTURE byte-identical**: same nodes, same weights, same order,
  same `WindowId`s. Its **rectangles necessarily change**, being recomputed
  inside the smaller flexible remainder. Falsified if opening a panel reorders,
  reweights, or re-ids any document node.
- **B7'''' (replacing the falsified B7'/B7''/B7''') — the transitive §1.3
  census and Q#BP14b surface matrix are complete.** Falsified if any direct or helper-
  mediated read of active window/buffer state reached by the daemon/semantic
  projection is missing, if a focus surface inherits the document viewport
  again, or if an open/clear baseline survives on the wrong surface under
  acceptance.

## 5. Acceptance

**Stage 1 — core + TUI (no wire change from its eventual base).**

1. `Layout::compute` honors a fixed extent: a bottom child of N rows gets
   exactly N; siblings divide the remainder by weight. **Both production
   callers are pinned through their real paths** (R5-B1): a document window's
   rows come from `window_placements`, and a peer cursor in that same window is
   painted by the overlay pass (`src/overlay_paint.rs:112`) at an identical row
   whether or not a panel is open — the assertion that fails if the second
   caller keeps computing unfixed geometry.
2. Opening a panel leaves the prior document subtree's **structure** identical
   (nodes, weights, order, ids); its rects are recomputed (B6).
3. `subtree_min_rows` is recursive: a **nested** document tree (horizontal
   inside vertical inside horizontal) keeps every leaf at the floor, and the
   panel is clamped — not the document — when they compete.
4. Programmatic `height`, `window.panel-height`, and side `fixed_rows` requests
   of one row clamp to `MIN_WINDOW_OUTER_ROWS`; zero rejects. An intrinsically
   too-small or zero-column frame uses saturating arithmetic and hides rather
   than underflows or emitting a zero-width panel.
5. A terminal resize preserves a side window's **absolute** height and a
   flexible pair's **ratio**, in one layout.
6. Geometry is cached before first input. A command/hook opens and selects a
   panel in a too-small frame, then a second key in the **same drained burst**:
   reconciliation marks the panel hidden, moves focus to a document, and
   releases the observed terminal controller before that key dispatches.
7. Growing the frame enough to make the request satisfiable restores the panel
   at its exact requested `fixed_rows`; focus is **not** auto-restored, and
   `focus_next/prev` skip it while hidden but reach it after reappearance.
   While hidden, its rect is empty and the unchanged document subtree receives
   every reclaimed row; the stored request, wrapper, ids, weights, and order
   remain intact.
8. Keys typed while the panel is hidden reach the document window, never the
   invisible panel.
9. `window.min-height` below the structural floor clamps; a value materially
   above it constrains drag/keyboard resize recursively across a nested tree,
   while frame-resize layout ignores the preference.
10. Closing the panel collapses the wrapper and restores the prior root exactly.
11. `set_params` rejects adding/changing/clearing `side` and rejects
    `origin_document`; `params` may report the origin; a stray `fixed_rows` on
    a non-side window is inert. Every `WindowId`-taking Lua operation rejects a
    live id owned by another frontend.
12. Raw `switch_buffer` **ignores** `dedicated`; `display_buffer` honors it on
    side, reused, exact, and non-side candidates, falling through or erroring
    without overwriting one. An ordinary display never reuses a matching side
    window.
13. Side placement is affinity-aware: a buffer already visible in a document
    window does not preempt a requested usable side slot. An explicit
    `window` is exact. A dedicated side fallback never creates a second side
    window and discards height/dedication/quit state before touching the
    document target; `window` + `side` and a freestanding `height` reject.
    Same-buffer redisplay preserves omitted height/dedication/action;
    replacement preserves an omitted user-resized height but defaults the new
    presentation undedicated; creation uses the setting/default. Explicit
    `dedicated = false` cannot bypass an existing dedication in the same call.
14. Capability fallback discards all side-only parameters and leaves the
    document target undedicated/unpinned.
15. **Final-focus matrix (Q#BP4), all six rows**, including `select = true`
    leaving the target selected and `select = false` restoring a **side**
    `saved_active`.
16. The three hook-failure arms (hook closes target / closes saved / switches
    buffers) are covered in **both** `select` modes, with reconciliation between
    the hook and final-focus decision.
17. A panel displayed into a passive window has its overlays re-attached.
18. **`display_file` to a previously unopened file from a focused panel** opens
    it in the exact document target, leaves the panel intact, and fires
    `buffer.after-load` with the **document target** active — asserted through
    the real LSP and compile visit paths. A dedicated exact target fails
    without loading/switching it; an omitted target skips a dedicated
    remembered origin and chooses the next eligible non-side window before I/O.
    A `NotFound` path creates a path-backed buffer and fires no load/switch
    hook, matching initial-target/local-startup behavior.
19. `pmacs.terminal.open{display="panel"}`,
    `compile.run{display="panel"}`, and
    `pmacs.listview.open{display="panel"}` place through their real entry
    points. The fixture first shows persistent `*compilation*` / `*outline*` in
    a document window, proving side-affine placement is not vacuous. Unknown
    `display` values fail before buffer/process/session/wrapper creation.
20. `listview`/compile `q` route through `window.quit`: the first panel deletes
    its wrapper; C→B→A restores each saved height, dedication,
    cursor/view/goal/selection, hook-attached overlays, and prior quit action;
    a killed restore target collapses safely. Capability fallback restores the
    prior document through the adopter's old path and leaves no quit action.
    Terminal placement failure removes a newly created wrapper before its
    existing session/buffer rollback completes. Replacing more than
    `MAX_PANEL_QUIT_DEPTH` times retains exactly the newest 64 presentations,
    then terminates in `Delete`; depth never grows beyond the cap.
21. **`panel → RET source → M-,`** for outline and compilation returns focus to
    the same still-showing-origin panel row; the document window remains
    unchanged and no duplicate presentation is created.
22. Jump histories are per frontend: interleaved pushes/pops cannot consume a
    peer's entries. A live origin window now showing a different buffer
    is skipped when it was a side origin rather than resurrecting or
    duplicating the old panel; an invalid non-side origin retains today's
    acting-frontend fallback.
23. `window.quit` revalidates `QuitAction::Restore`; a killed restore target
    degrades to delete.
24. Killing a panel buffer **closes the side window** rather than redirecting to
    `*scratch*`.
25. `close_active` refuses only when the target is the last **non-side** window;
    closing the side window itself is legal even as the only other window.
26. `close_others` from a document window deletes the panel; from a side window
    it errors. `split_active` from a side window errors.
27. `C-x o` reaches the panel and returns; the terminal controller is claimed on
    entry and released on exit. With two document windows, entering the panel
    from B refreshes `origin_document`, so `display_target`, a panel visit, and
    a Delete-form `window.quit` target B rather than the window from panel
    creation.
28. With the panel focused, unescaped bound keys reach the child; `C-c` escapes
    for exactly one key; `C-c C-c` sends one literal interrupt. **B1 pin.**
29. A focused side window makes `dispatch_idle_for` return `false` **without**
    marking its buffer round-trip, and another frontend editing that same
    buffer as a document keeps optimistic apply. A forged/stale optimistic op
    for the document is rejected before source-window cursor/provenance
    mutation; a valid round-trip edit still updates the focused panel window.
30. Divider drag changes side `fixed_rows` and document-pair weights under the
    interactive recursive preference; a click on the reserved row creates no
    selection, and `ui.divider` resolves through the `ui.*` face walk. A
    boundary whose upper child is a vertical split paints all adjacent exposed
    mode-line segments, and dragging either segment resolves the same boundary.
31. `window.enlarge`/`shrink` equal the equivalent drag in a **nested** layout
    where the active subtree is its nearest horizontal ancestor's final child;
    `resize(win, …)` resolves from `win`; no horizontal ancestor reports/no-ops.
32. A terminal panel scrolled back keeps its `top` across a height change;
    growth reaching the tail re-arms follow; later output scrolls in.
33. Growth reaching the tail with a historical selection leaves the selection
    and anchor frozen, via the shared viewport-size path.
34. Only the controller's height change resizes the PTY. A semantic panel
    terminal uses the daemon-derived panel content rect at the pre-drain sync
    point, never the 24×80 attach placeholder or full-window terminal
    declaration.
35. The desktop round-trips a layout containing a file-backed side window
    **without** the side leaf or its root wrapper; restored document leaves
    have default parameters and the desktop format version does not change.
36. Full gate suite per `AGENTS.md`; because Stage 1 factors #148's target-load
    seam, this includes `gpu_initial_target_acceptance` in default and CRDT
    configurations in addition to the new/touched panel suites.

**Stage 2 — GPU band (own re-framing; next available protocol version).**

37. `PanelFrame` round-trips, including `panel_epoch` and `geometry_epoch`;
    independent **byte pins on the previous final
    `InstanceMessage::InitialTargetResult` and
    `FrontendEvent::TerminalPointer` variants** catch a shift in either
    extended enum.
38. Full lifecycle: **open → replace buffer → hidden by a tiny frame →
    reappear → close**, with authoritative `Absent` at hide/close and a new
    epoch on replacement/reappearance.
39. An invalid `PanelFrame` is rejected atomically; the previous valid frame is
    retained. A duplicate valid frame (including duplicate `Absent`) does no
    work. Shared cell/topology/glyph/area validation accepts an area-bounded
    panel wider than 512 columns, while terminal frames retain their 512-column
    PTY cap; maximum legal panel encoding stays below the transport limit.
40. **First open at a non-80×24 frame before any valid panel baseline** remains
    absent until real `FrontendCellGeometry` arrives, then produces the correct
    grid without consulting the 24×80 attach placeholder.
41. Pixel→cell conversion is pinned at fractional widths/heights: status band,
    `TEXT_TOP`, potential divider, virtual status row, full-width monospace
    columns, and floor rounding agree. Geometry refreshes on window resize,
    font change, and scale change; the daemon alone derives the grid. After a
    new `geometry_epoch` is sent, an older retained frame neither paints nor
    accepts input until a matching `Present` arrives; `Absent` remains an
    always-safe removal, and stale/conflicting epochs reject. A requested panel
    whose rows×cols would exceed the shared wire-area budget is row-clamped
    without losing its stored request, or hidden when even two rows cannot fit.
    Zero/non-finite/non-positive metric inputs fail closed to zero usable
    geometry without overflow or an oversized allocation.
42. **Focus into and out of a terminal panel while the document stays visible
    and unchanged**: no `BufferSnapshot` re-send, no document suppression, no
    mirror swap, no `CursorByte` for the panel buffer, no line-number,
    selection-decoration, document-terminal declaration, or document
    statusline replacement/clear with the panel buffer. Document statusline
    callbacks may truthfully observe `active = false`; presence reports the
    focused panel context. The GPU replica's `active_buffer` and authoritative
    cursor remain the primary document buffer/cursor while `DispatchIdle` is
    false, and the revised protocol/client contract tests name that distinction.
43. **Focusing a fresh generated panel buffer triggers no lazy-CRDT-upgrade
    broadcast** (§1.3 #2 — the case rev 2 could not see). With semantic peer A
    focused in panel P over document D, a target launch/upgraded-buffer
    publication for D still reaches A, while one visible only as P does not
    replace A's document mirror (§1.3 #21).
44. A document `Viewport` naming the document buffer while the panel is focused
    aligns the **primary document window** and **does not move focus**; a
    document `Pointer` aligns **and** activates the document window. With a
    full-window document terminal under a focused panel, its viewport and
    `TerminalResize` remain accepted, bare `TerminalPointer::Move` does not
    focus or claim, and every accepted non-hover terminal gesture activates
    the document before replay.
45. From a focused panel, `M-x` opens/types/closes a visible
    `MinibufferPrompt`; isearch keeps its semantic prompt while panel washes
    paint in the grid. A new focused `PanelFrame` arrives before the
    panel-buffer `SearchPrompt`, which the GPU accepts without changing its
    document mirror; on hide/close/focus-out, the old panel prompt clears
    before its focused declaration is removed. A prompt naming neither current
    surface is ignored.
    Document→panel focus authoritatively clears a native document
    menu/completion popup, while panel menu/completion overlays paint only in
    `PanelFrame`; returning to the document reverses ownership cleanly.
    The global prompt/clear pass also works before a document viewport exists
    and while the primary document is a full-window terminal.
    One statusline provider invocation supplies the primary-document wire
    segments and panel mode line; a provider that mutates the layout
    invalidates stale results and reconciliation runs before paint.
46. The band + divider shrink the document text area by exactly their pixel
    height; document carets, hits, and scroll geometry respect the reduced
    area. `Present`/`Absent` refresh the ordinary document `Viewport` or
    full-window `TerminalResize` without sending a new whole-frame geometry
    declaration.
47. Dragging the divider sends
    `PanelResizeRows {geometry_epoch, panel_epoch, rows}` and honors
    `window.min-height`; hover shows `CursorIcon::RowResize`. A stalled-writer
    outbox tail-coalesces repeated resize rows and whole-frame geometry
    declarations without crossing an intervening event or exhausting the
    lossless queue.
48. `PanelPointer` drives listview row selection, panel selection, terminal
    mouse reporting, and click-to-focus without disturbing the document mirror.
    A terminal panel's non-`Move` wheel/press/drag/release first activates it
    so controller ownership remains consistent; hover does neither. Keyboard
    motion beyond a focused panel's viewport runs the extracted active-window
    auto-scroll clamp, while a passive panel preserves `view_top`. Panel
    move/drag tails coalesce; press/release/context/wheel remain lossless and
    ordered.
49. Stale panel events are dropped before mutation for all four cases:
    A→B replacement (`buffer_id`), close/reopen of the same A, and
    hide/reappear of A (`panel_epoch` / latest-`Present` check), plus a
    font/scale/resize declaration race (`geometry_epoch`). `Absent` clears
    declared panel size/presentation epoch on both sides without discarding the
    whole-frame geometry declaration.
50. `PanelResizeRows` / `PanelPointer` from a source with no visible current
    `Present` panel are dropped. `FrontendCellGeometry` from the correctly
    negotiated semantic source is accepted without a side window; grid,
    pre-panel, forged-source, and wrong-version variants are rejected.
51. **Mixed session**: a pre-panel semantic frontend falls back to a document
    window — with every side-specific parameter discarded, leaving the document
    window undedicated (Q#BP2c) — while a grid frontend on the same daemon gets
    its side window. With `LOCAL` focused in that panel, a fresh no-target
    semantic attach inherits `LOCAL`'s primary document buffer, never the
    panel buffer (§1.3 #22).
52. A panel projected for a `fold_projection = false` frontend does **not**
    collapse folds; the stale comment at `src/window.rs:339` is updated in the
    same PR.
53. Bell drain remains focus/session-scoped: a focused panel terminal rings
    once per frontend, while passive and historical bells remain suppressed.
54. A `--headless-probe` run drives one real daemon + real PTY + real wgpu
    through a panel-hosted terminal, followed by the full gate suite for the
    Stage 2 PR.
55. A v20 initial-target attach whose `after-load`/`after-switch` hook
    creates and selects a side window still reasserts the requested buffer in
    and activates a non-side document window without overwriting the panel.
    Closing the original document window in the hook rehomes the target to a
    remaining eligible non-side window without a second hook; killing the
    target buffer still fails bootstrap. The target snapshot precedes matching
    `InitialTargetResult::Opened` exactly as in #148.

**Stage 3 — adopter default flip.**

56. Omitting `display` from real listview, compile, and terminal entry points
    resolves to the Q#BP12 panel/select policy on a panel-capable grid and
    semantic frontend; explicit `display = "current"` preserves each
    adopter's pre-arc selected-window behavior.
57. On a pre-panel semantic frontend, the omitted Stage 3 default takes
    capability fallback with no side parameters or quit action left on the
    document window; its visit and `q` paths remain the existing non-side ones.
58. Updated default-placement suites exercise
    open→visit→return→quit through each adopter rather than a generic helper,
    preserving the Stage 1 unknown-value rollback assertions; the Stage 3 PR
    then runs the full gate suite.

## 5a. Acceptance 48 — ground truth, RE-MEASURED at `2c0d3ff` (2026-08-20)

**Status: revision 16 — AWAITING APPROVAL.** Revision 13 ruled Q#BP-R3
and **blocked** this lane on a protocol-bearing mapping generation.
That slice was framed as §5b, approved at revision 16, and **merged as
#242 (`47b5463`)**, so **revision 13's block is DISCHARGED** and its
ruling stands as history rather than as a gate. Revision 14 answered
what the lane inherits from a substrate that changed underneath it.

**Revision 16 closes review of 15.** It replaces the two competing
Q#BP-R4 matrices with one pre-effect disposition/lifecycle table;
names the projection drain at the only enforceable seam — after
`render_frame` returns and before its messages are written; splits
horizontal-wheel ground truth by target and gesture domain; records
GUI Stage 1b as the owner of document-panel horizontal scrolling; and
removes revision 14's now-false remeasurement obligation from the live
section and ledger.

**Revision 15 answers review of 14, and four of its five changes are
corrections rather than additions:**

- **Q#BP-R4's release half was bookkeeping wearing an effect's
  clothes.** It said a chrome `Up` should "consume" the latch and
  claimed that prevents a child left button-down. It prevents nothing:
  the daemon drops the record, and terminal chrome returns before
  replay. Now ruled as **TERMINATE**, with the row asserting the
  child's release or the local selection's completion — and a
  no-duplicate leg.
- **The pending-release slot had an invariant but no ordering.** The
  seam replays before it arms, so the invariant could only fire after
  the damage. **Drain order is now part of the ruling** — before any
  subsequent panel-pointer effect, before detach teardown, before the
  next frame — with an old-release-before-new-press row.
- **The ground truth was knowingly false and is now RE-MEASURED**, at
  `2c0d3ff`, rather than deferred to implementation. Two clauses the
  old table called MISSING are DONE, and a third — the horizontal
  wheel — exposed an ownership gap that revision 16 now splits and
  assigns explicitly.
- **"This lane MOVES the grade" was wrong.** Step 5/GPU is floored
  `Partial` by 5(a) IME and step 8 is already `Works`; no journey cell
  moves. The draft had read this section's own MISSING column as if it
  were the scorecard.
- Q#BP-R4 also **drops a false TUI claim**: only semantic legacy and
  mapped peers reach this dispatcher.

**Why this section exists.** GUI arc Stage 1b's ground-truth pass found
that `PanelPointer` replayed nothing past focus. **That is no longer
the state of this branch**, and the paragraph that said so — with the
wheel "dead on both axes" — is superseded below. This lane has since
landed the document and terminal replay, and §5b merged underneath it.
1b still rebases onto this lane's merge.

**RE-MEASURED at `2c0d3ff`, the tip this revision was written against.**
The 2026-08-14 measurement was taken at `72da24a`, **before this
branch's own implementation commits and before §5b**, so it described
neither the branch nor main. Every anchor in it had also moved. It is
kept below as history; **this table is the current one**:

| clause | status at `2c0d3ff` | production anchor |
|---|---|---|
| click-to-focus | **DONE** | `activates` → `focus_window` (`src/editor.rs:2888`) |
| terminal panel: non-`Move` activates, hover does neither | **DONE** | the same `activates`, split by `is_terminal` |
| focused-only auto-scroll clamp; passive preserves `view_top` | **DONE** | `src/editor.rs:2698`, which cites "A2A-3 / parent 48" |
| move/drag tails coalesce; press/release/context/wheel lossless and ordered | **DONE** | `coalesce_kind` (`pmacs-gpu/src/attach.rs:338`) |
| **panel selection** | **DONE — was MISSING** | `replay_panel_document_gesture` (`src/editor.rs:2914`), called at `:2903`: `Down`/`Drag`/`Up(Left)`, shift-extend, double-click word select, right-press |
| **terminal mouse reporting** | **DONE — was MISSING** | `apply_terminal_gesture` called at `src/editor.rs:2899`, viewport `content_rows` |
| **vertical wheel moves the panel's viewport** | **DONE — was MISSING** | document `ScrollUp`/`ScrollDown` → `scroll_window` (`src/editor.rs:2925`); reporting and local terminal paths share `apply_terminal_gesture` |
| **horizontal wheel, terminal precedence selects child reporting** | **RECEIVER DONE; PRODUCER AWAITS GUI 1b** | `apply_terminal_gesture` reaches `encode_mouse`, whose `ScrollLeft`/`ScrollRight` codes are 66/67 (`src/terminal/input.rs:130`–`:131`) |
| **horizontal wheel, terminal precedence selects local handling** | **RULED INERT** | Shift, reporting-off or a scrolled-back view can select the local branch; terminal scrollback is vertical and there is no local horizontal viewport, so the shared path deliberately has no `ScrollLeft`/`ScrollRight` arm |
| **horizontal wheel, document panel** | **NAMED DEFERRAL: GUI Stage 1b B1–B3** | `ScrollLeft`/`ScrollRight` are claimed and dropped at `src/editor.rs:3004`; the comment at `:2999`–`:3003` already assigns horizontal panel scrolling to 1b, whose framing now explicitly includes the panel-document surface |
| **listview row selection** | **MISSING** | — no listview handling on the replay path; Q#BP-R1 ruled the semantics (single click selects only), and nothing implements them |
| without disturbing the document mirror | **the constraint on all of the above** | — |

**Two findings the re-measurement produced**, neither of which a
line-number patch would have surfaced:

- **"Horizontal wheel" was not one verdict.** A tick for which terminal
  precedence selects child reporting already sends the horizontal SGR
  code; the local terminal branch has no horizontal viewport to move;
  only a document panel lacks an effect. That last leg is a **named
  deferral to GUI Stage 1b B1–B3**,
  not an unexplained catch-all: 1b owns horizontal tick production,
  sign, per-surface residuals and the window-targeted `view_left`
  effect. The panel-replay lane remains 1b's prerequisite.
- **Two of the four "MISSING" effects are DONE on this branch.** The
  section had been telling every reader that replay does nothing while
  the branch it describes had implemented most of it.

### Superseded — the 2026-08-14 measurement at `72da24a`

Kept because it is what the lane was scoped against, and because two of
its verdicts are now wrong rather than merely stale. Measured clause by
clause:

| clause | status | production anchor |
|---|---|---|
| click-to-focus | **DONE** | `dispatch_semantic_panel_pointer`'s `activates` → `focus_window` (`src/editor.rs:2701`) |
| terminal panel: non-`Move` activates, hover does neither | **DONE** | the same `activates` computation, split by `is_terminal` |
| focused-only auto-scroll clamp; passive preserves `view_top` | **DONE** | `src/editor.rs:2568`–`:2571`, which already cites "A2A-3 / parent 48" |
| move/drag tails coalesce; press/release/context/wheel lossless and ordered | **DONE** | `pmacs-gpu/src/attach.rs:374`–`:381` — `Move` key 6, `Drag` key 7, everything else unkeyed |
| **listview row selection** | **MISSING** | — |
| **panel selection** | **MISSING** | — |
| **terminal mouse reporting** | **MISSING for panels** | the path exists: `apply_terminal_gesture` |
| **wheel moves the panel's viewport** | **MISSING** | the mechanism exists: `scroll_window` |
| without disturbing the document mirror | **the constraint on all of the above** | — |

### The replay is mostly WIRING, and both mechanisms already exist

**Terminals.** `apply_terminal_gesture` (`src/editor.rs:3525`) is
documented as *"The one terminal pointer path, shared by both frontend
kinds"* — TUI via crossterm, semantic frontend via
`FrontendEvent::TerminalPointer` — and it already drives child mouse
reporting, selection and scrollback. A panel terminal needs the **same
call**, not a second implementation: `side_window_for`
(`src/editor_core.rs:3146`) gives the window, `TerminalViewKey::new`
the key, and `panel_grid_size` — which the dispatcher **already
fetches** — the viewport size. **A wheel-only bridge here would be the
wrong shape**: the shared path handles every kind at once.

**Documents.** `scroll_window` (`src/editor.rs:3845`) is window-scoped
throughout, cursor carry included, and says so: *"a wheel event names
the pane under the pointer and does NOT activate it, so the map must
come from `win_id`"*. Its cursor carry moves **that window's** point —
the panel's own, not the document mirror.

### The scoping hazard, and why the activation rule already answers it

**The selection and cursor API is ACTIVE-WINDOW scoped.**
`set_cursor_byte` (`src/editor_core.rs:1216`), `begin_selection`
(`:4691`) and `clear_selection` all write `active_window_mut()`. Used
naively from the panel path they would move the **document's** point —
precisely what "without disturbing the document mirror" forbids.

**Activation ordering is NECESSARY BUT NOT SUFFICIENT — revision 5
stopped here and was wrong to.** It holds only for the gestures that
activate, in a session that nothing interleaves with, on a
presentation that does not change mid-gesture, with modifiers intact
and the mode line excluded. **Each of those four provisos is an edge
revision 6 had to add** (R-a…R-d below). The ordering itself is real:

- **Document panel** — `activates` on `Down(_)`. A drag-select is
  Down → Drag → Up, and the Down focused the panel, so the later
  gestures act on an active window legitimately.
- **Terminal panel** — `activates` on every non-`Move`, so reporting,
  selection and wheel all run focused. This is AC48's own
  controller-ownership clause.
- **The one gesture that does NOT activate** is a document panel's
  wheel — and it needs only `scroll_window`, which is window-scoped.

So the ordering rule is: **replay after activation, and anything
reachable without activation must use a window-scoped mechanism.**
That is the floor, not the contract — R-a through R-d are the rest of
it, and three of the four are places where the tree already contains
the right precedent and the panel path simply does not use it.

### Q#BP-R1 — does a listview row visit on click? **RULED: no — single click SELECTS only**

RET/SPC remain the activation path (`builtin/runtime/listview.lua:610`,
which reads `pmacs.editor.cursor_line()` and calls `on_visit`). **This
lane adds no click-to-visit and no double-click-to-visit.** That
follows acceptance 48's wording — it names row *selection* — and keeps
document navigation from becoming an incidental consequence of wiring
replay.

#### Q#BP-R3 — a panel cell has no frame-content provenance **RULED: BLOCK on a protocol-bearing mapping generation**

**Revision 12's answer — accept current-state semantics — is
OVERRULED, and all three of its bounds were wrong.** They are recorded
because each was the reason the trade looked cheap:

| the bound I claimed | why it does not hold |
|---|---|
| "self-inflicted — the same frontend must move the view and then click" | **another frontend, or a background process, can edit the same buffer.** The cell→byte mapping changes with `view_top` untouched, and the clicking frontend did nothing |
| "bounded by `SCROLL_LINES`" | **unbounded.** Multiple wheel ticks, paging, folds, edits or a reload can all land before the new frame is seen |
| "one round trip" | **until the frontend actually PRESENTS the replacement frame** — a slow or backed-up frontend widens the window arbitrarily |

With those gone there is no narrowness left to trade on, so **the lane
blocks on the wire fix** rather than shipping a mis-hit whose size and
cause are both unbounded.

**And the follow-up is NOT a per-frame token.** A token that changed on
every frame would invalidate a gesture on every repaint, which is the
same mistake `panel_epoch` deliberately avoids — it would break drags
outright. What is needed is a **CELL-MAPPING GENERATION**: an identity
of the *inverse mapping*, not of the frame.

- **It changes when the inverse mapping changes**: viewport (`view_top`
  or grid size), folds, wrapping or gutter geometry, or buffer content.
- **It is stable across repaints that cannot move a cell's byte**:
  focus, styling and theme, cursor movement, and selection-only
  changes. That stability is what lets an ordinary drag survive the
  repaints it provokes.

See **§5b** for the slice.

#### Superseded — revision 12's reasoning, kept for the record

**The hole.** `PanelPointer` carries `geometry_epoch`, `panel_epoch`,
`buffer_id` and a `coord` — **and nothing identifying the frame CONTENT
the user was looking at** (`pmacs-protocol/src/message.rs:500`).
`panel_epoch` is deliberately *"stable across ordinary frames of one
continuously present window/buffer"* (`panel.rs:61`), so an ordinary
repaint changes no epoch at all.

**The race that follows is real.** A document-panel wheel moves
`view_top` **daemon-side**. The daemon repaints and emits a new frame.
Before that frame reaches the GPU, the user clicks. Every validation
passes — same buffer, same epochs — and the daemon inverts the cell
through its **current** `view_top`, selecting a row the user never saw.
Off by up to `SCROLL_LINES` (3). **No existing gate can reject it**,
because nothing about the event is stale by any test the ladder
applies.

**Why the epochs cannot be stretched to cover it.** Moving
`panel_epoch` on content change would invalidate a gesture on every
repaint, which breaks drags outright — the field is stable *by design*,
and that design is what makes selection possible.

**Why a fix is not free.** Closing it properly needs a **per-frame
token** on `PanelFrame`, echoed by `PanelPointer` — **a wire change**,
which makes it a protocol-bearing slice. This lane is explicitly
non-protocol-bearing, and GUI arc 1b is blocked behind it.

**A daemon-only mitigation was considered and does not work.** Having
the daemon invert against the `view_top` it used for its **last emitted
frame** sounds like it removes the wire dependency, but it does not:
the daemon still cannot know **which** emitted frame the user saw, and
the failing window — frames emitted after the one on screen — is
exactly the same one. Without a token echoed back, the information does
not exist on the receiving side.

**Ruling: this lane accepts CURRENT-STATE hit semantics**, and says so
rather than leaving it undiscovered:

- A panel cell is resolved against the daemon's state **at the moment
  the event is processed**, not against the frame the frontend painted.
- **The window is narrow and self-inflicted**: it requires the same
  frontend to change the panel's view and then click inside one
  round-trip. It cannot arise from another frontend's activity, because
  a foreign edit that moves `view_top` is not a thing panels do.
- **The magnitude is bounded** by whatever moved the view — one wheel
  step, `SCROLL_LINES` rows.
- **The TUI is unaffected.** It has no round trip; the hazard is
  structural to a remote frontend inverting cells against mutable
  daemon state.

**Named follow-up, not a shrug: `PanelFrame` gains a content token and
`PanelPointer` echoes it, in the next protocol-bearing slice.** The
daemon then drops a gesture whose token no longer matches — the same
shape as the epoch ladder, one level finer. It is recorded here so the
next wire slice inherits it rather than rediscovering the race.

**Overrule this if you would rather block the lane on a protocol
slice.** The trade is explicit: a narrow, bounded, same-frontend
mis-hit now, against serializing this lane and 1b behind a v25 wire
change.

### The four replay edges — none of which "activation ordering" covers

§5a's first draft concluded that the activation rule made the replay
safe. **It does not, and each gap below is a place where a plausible
implementation is silently wrong.**

#### R-a. Modifiers are dropped before they reach the shared path

`FrontendEvent::PanelPointer` **carries `mods`**, and the daemon
**destructures them into `..`** (`src/daemon.rs:2425`);
`dispatch_semantic_panel_pointer` has no modifier parameter at all.

**This breaks terminal precedence, not merely fidelity.**
`apply_terminal_gesture` opens with
`let shift = modifiers.contains(TerminalModifiers::SHIFT)` (`:3534`)
and gates child reporting on `!shift && … && modes.mouse_sgr`. **Shift
is the user's override for "select locally instead of talking to the
child."** Arriving with modifiers zeroed, a Shift-drag over a
reporting terminal panel sends SGR to the child instead of selecting.

**And the terminal is not the only consumer.** The ordinary document
path reads Shift too: `dispatch_pointer`'s `Down` arm computes
`extending = mods.contains(SHIFT)` and `keep_anchor`, then either
**extends from the previous cursor** or collapses the selection to the
clicked byte (`src/editor.rs:3673`–`:3687`). So Shift-click in a
**document or listview panel** is selection extension, and threading
modifiers *only* into `apply_terminal_gesture` would **pass the
terminal row while leaving document panels broken** — the exact shape
of a witness that certifies half a fix.

**Thread `mods` through** the destructure, the dispatcher signature,
and into **both** consumers. Two rows, and both mutations:

| # | row | mutation |
|---|---|---|
| A1 | terminal panel, reporting enabled: **Shift**-drag selects locally, child receives **no bytes** | drop `mods` before `apply_terminal_gesture` → the child receives bytes |
| A2 | document/listview panel: **Shift**-click **extends** the selection from the prior cursor; unmodified click collapses it | drop `mods` before the document path → Shift-click collapses, A2 fails while A1 still passes |

A2's mutation is deliberately separate from A1's: a single "drop
`mods`" at the boundary bites both, which proves the boundary matters
but not that **each consumer** is wired.

**A1 is a NEGATIVE row and cannot stand alone.** It asserts bytes that
must *not* appear, and **disabling child reporting entirely satisfies
it** — as does never reaching `apply_terminal_gesture` at all. The
whole edge matrix could stay green against a replay that never reports.
Positive controls, on the **same terminal** as A1 so the pair is a
genuine discriminator:

| # | row | mutation |
|---|---|---|
| A3 | unmodified content `Down`/`Drag`/`Up`, reporting enabled → the child receives the **exact SGR byte sequences** for each | bypass `apply_terminal_gesture` → no bytes, A3 fails while A1 still passes |
| A4 | unmodified content **wheel**, reporting enabled → the child receives the **exact SGR wheel bytes** | as A3 |
| A5 | same wheel, reporting **disabled** → **no bytes**, and the view's **local scrollback moves** | route the wheel around the shared path → the scrollback does not move |

A3–A5 pin the *positive* half: that replay reaches the shared path and
the child hears what it should. A1 then pins the one case where it must
not.

#### R-b. Activation does not make the document path target-safe

`activates` is true for a document panel's `Down(_)` only, so **`Drag`
and `Up` do not activate**, and **another frontend's input can
interleave between them** — sessions are independent and nothing
freezes the active window for the duration of a gesture. A replay that
reads ambient active-window state for `Drag`/`Up` therefore acts on
whatever is active *then*, which may be a different window entirely.

**The framing names the mechanism rather than leaving it to the
implementation: an explicit side-window cell→byte adapter, and
selection routed through a window-TARGETED path.** The precedent and
the trap are the same function — `activate_and_position`
(`src/editor.rs:3795`) is already window-scoped in its conversion (it
takes `win_id` and uses *that* window's `view_top` and fold map, per
the round-3 F1 correction) **but it also calls
`set_active_window_id`**, and the selection APIs it feeds
(`set_cursor_byte`, `begin_selection`) write `active_window_mut()`.
Panel replay needs the conversion **without** the ambient write.

**The witnesses must pin the RESULT, not just the absence of collateral
damage.** "Only A's panel changes" is satisfied by an implementation
that drops the tail entirely and changes nothing anywhere.

| # | row | what it pins |
|---|---|---|
| B1 | panel A `Down` → **frontend B input** → panel A `Drag` | A's **anchor is the Down cell's byte and A's cursor is the Drag cell's byte**, exactly; B's window and the document mirror are unchanged |
| B2 | panel A `Down` → `Up` at the **same** cell | the window's **raw `selection` is `None`** afterwards — see below |
| B3 | orphan `Drag`/`Up` on a **passive** panel, no preceding `Down` | the document mirror is **byte-identical**: cursor, selection, `view_top` |
| B4 | **repeated left `Down`s** at one cell | the **exact word** at that cell is selected after the second `Down` — the multi-click actually resolved, per Q#BP16 |
| B5 | `Down(Right)` on a panel cell | the **context menu** opens, per Q#BP16 — a right press is not a selection gesture |
| B6 | **single** click, then **repeated/double** click, on a listview row | the row is **selected** and the `on_visit` sentinel **never fires**, in *both* — Q#BP-R1 forbids double-click-to-visit too |

**B2 must read the raw selection, not `active_region()`.** That helper
returns `None` *"if no region is set **or it is empty**"*
(`src/editor_core.rs:4684`–`:4688`), so it answers `None` **both before
and after** the `Up` and cannot see the residue at all. The residue
being guarded against is an **active-but-empty selection** — a
`Some(Selection { anchor })` whose anchor equals the cursor — which
would make the next shift-motion extend from a stale anchor. Inspect
the **owning window's** `selection` field directly.

**B6 runs both click shapes** because the ruling forbids
double-click-to-visit as well as single. A sentinel that only tries a
single click leaves the more tempting implementation — visit on the
second click, like a file manager — completely uncovered.

B4 and B5 are not new contracts: Q#BP16 already states that repeated
left `Down`s are what the click state reads as a multi-click and that
`Down(Right)` is the context gesture, *"so neither may collapse"*
(§3, Q#BP16). They are in this matrix because **replay is where those
statements first become executable** — until now nothing replayed, so
nothing could contradict them.

B6 is the sentinel for the ruling above: selection without activation
is only meaningful if something fails when a click visits.

#### R-c. `panel_grid_size` is the FRAME, not the terminal viewport

The panel's last row is its **mode line**. Projection says so
explicitly: `content = Rect::new(0, 0, size.rows.saturating_sub(1),
size.cols)` against `outer` at full height (`src/editor.rs:2499`–
`:2500`). But **`panel_hit_test` reports cells across the whole
frame** — it passes `frame.size` (`pmacs-gpu/src/main.rs:7184`) — so a
`PanelPointer` **can name the mode-line row**.

Passing `panel_grid_size` straight to `apply_terminal_gesture` as the
viewport therefore **makes the mode line a child terminal cell**: the
child is told about a row it does not own, and every coordinate below
it is off by the same row when the size is used for clamping.

- **Terminal panels:** the viewport is **`rows − 1`**.
- **Document panels:** the mode line needs an explicit rule.

**"The last row is inert" is WRONG, and revision 6 said it.** The TUI's
rule — which is the precedent — is **per kind**, not per row.
`inner_rows` guards `Down(Left)` (`src/editor.rs:3303`), `Drag(Left)`
(`:3331`) and `Down(Right)` (`:3348`), and **`Up(Left)` (`:3339`),
`ScrollUp` (`:3358`) and `ScrollDown` (`:3362`) are deliberately NOT
guarded.** A blanket
"inert" rule would break two things at once: a wheel over the mode line
would stop scrolling, and a gesture that begins in content and releases
over the mode line would **never terminate**.

**And the producer arms before the receiver can refuse.** `PanelCell`
comes from `panel_hit_test`, which spans the whole frame, so a
mode-line `Down` runs `set_panel_pointer_held(true)` **locally**
(`pmacs-gpu/src/main.rs:2878`–`:2880`) before any daemon decision.
Dragging from there into content then emits a `Drag` with no accepted
`Down` — an orphan the daemon cannot distinguish from a real one. **A
receiver-only rule cannot fix this**; the producer must not arm on a
mode-line press.

**Kind alone is still not enough. The rule is target × gesture
ORIGIN**, and revision 7's table had neither axis complete.

**The TUI's per-kind rule is DOCUMENT-ONLY.** For a terminal window it
rejects **every** kind on the mode line before any per-kind match:
`if local_row >= inner_rows || … { self.mouse_click = None; return; }`
(`src/editor.rs:3273`), and it passes the content-sized viewport
`CellSize::new(inner_rows, …)` (`:3272`) to `dispatch_terminal_mouse`.
Terminals never see a chrome coordinate at all.

**And raw chrome coordinates are actively unsafe for terminals**,
because `apply_terminal_gesture`'s reporting branch is bounds-checked:
`coord.row < screen_size.rows && coord.col < screen_size.cols`
(`src/editor.rs:3560`–`:3561`). A chrome row equals `screen_size.rows`,
so the check **fails and the gesture falls through to the LOCAL
branch**. Two concrete wrongnesses follow: a reporting child gets a
`Down` and then **no `Up`** (its release became a local
`finish_selection`), and a chrome wheel becomes **local scrollback**
instead of behaving as a content wheel does.

##### The producer is TARGET-BLIND, and the table must respect that

**Revision 9 split the producer column by target. The GPU cannot make
that split.** `PanelFrame` carries `buffer_id`, both epochs, `size`,
`cells`, `cursor` and `focused` — **and no target-kind discriminator**
(`pmacs-protocol/src/panel.rs:73` onward). The panel is an **opaque
cell projection** to the frontend. `state.terminal`
(`pmacs-gpu/src/main.rs:1964`) describes the **primary full-window**
terminal, not the side-window, so it cannot answer the question either.

A producer rule reading "terminal chrome wheel: do not send" is
therefore **unimplementable without a new wire field**, and this lane
is explicitly non-protocol-bearing. **Every producer rule must be
target-blind; every target-dependent decision belongs to the daemon**,
which resolves the side window and knows the buffer kind
(`is_terminal(buffer_id)`).

**Producer — by kind and origin only:**

| kind | origin | producer |
|---|---|---|
| `Down(Left)`, `Down(Right)` | chrome | **do not arm, do not send** |
| `Drag(Left)` | chrome | not sent — never armed |
| `Drag(Left)` | content, now over chrome | normalize to the last valid CONTENT cell, then **ordinary dedupe** |
| `Up(Left)` | content, now over chrome | **always send**, normalized |
| wheel | chrome | **claim it and send**, carrying its valid frame coordinate |

These collapse safely because the two targets want the **same producer
behaviour** everywhere except the wheel: a chrome press is reserved by
documents and dropped by terminals — both drop; a crossing `Up` must
arrive for both, to terminate a selection or to deliver a release. The
wheel is the sole divergence, and it is now decided receiver-side.

**Receiver — after validation and resolving the side window:**

| kind | document panel | terminal panel |
|---|---|---|
| chrome `Down`/`Drag` | reserved — drop | drop, matching the TUI |
| crossing `Drag` (normalized) | process when it arrives | process at the content coordinate |
| crossing `Up` (normalized) | terminate the gesture | **replay at the last valid CONTENT coordinate** — never the chrome row |
| chrome wheel | **process through `scroll_window`** | **CONSUME, before activation** — no focus change, no controller claim, no child bytes, no local scrollback, no document fallthrough (Q#BP-R2) |

**The chrome wheel carries a chrome coordinate over the wire, and that
is fine**: it is a valid frame cell, the daemon's coord validation
accepts it, `scroll_window` is a window-level move that does not read
it, and the terminal branch never forwards it to a child. **The
coordinate is never fabricated and never reaches an application.**

**`Up` is the load-bearing crossing event, and the only one promised
unconditionally.** A content-originated gesture terminates at its last
valid content coordinate — that is what keeps a reporting child from
being told about a row it does not own while still receiving its
release.

**A crossing `Drag` is NOT promised as sent.** Once normalized, its
coordinate is frequently the one already reported, and the ordinary
motion dedupe suppresses it — correctly. Promising delivery would
oblige the producer to defeat its own dedupe for no gain: the daemon's
state after a suppressed same-cell `Drag` is identical.

#### Q#BP-R2 — a chrome wheel over a TERMINAL panel **RULED: CONSUME, do not clamp**

Revision 8 ruled *clamp*, on a consistency argument. **That was wrong,
and the reason is that SGR wheel input is COORDINATE-BEARING.**
`encode_mouse` takes a `coord` and writes `coord.col + 1` and
`coord.row + 1` into the sequence (`src/terminal/input.rs:102`,
`:146`–`:147`) — which A4 now pins exactly. **Clamping would fabricate
a hit on the terminal's final content row**, so an application that
routes wheel input by position could act on a cell the user never
pointed at. The consistency gain is cosmetic; the cost is a synthetic
coordinate delivered to a program as though it were real.

**And the wheel carries no liveness obligation.** `Up` must be
normalized because a gesture left un-terminated hangs — the daemon
holds a button down forever. A wheel tick is self-contained: dropping
one ends nothing and strands nothing.

**Ruling: a wheel over a terminal panel's chrome is CONSUMED** — not
reported, not scrolled locally, and **not fallen through to the
document**. The band owns the pixel either way. This also restores TUI
parity, which revision 8 traded away for the weaker argument.

Document chrome is unchanged: a wheel there still scrolls the panel.

**Enforced in the DAEMON, not the frontend.** Revision 9 wrote this as
a producer rule, which the GPU cannot implement — it has no way to know
the panel holds a terminal (see the seam note above). The producer
claims the chrome wheel and sends it for **every** panel; the daemon
resolves the side window and consumes it when the buffer is a terminal.

##### And it is consumed BEFORE ACTIVATION, not merely before replay

**"Consume before `apply_terminal_gesture`" is not early enough.**
Every non-`Move` terminal panel gesture activates the side window
first, and `activates` is computed as `!matches!(kind, Move)`
(`src/editor.rs:2695`–`:2698`), which **includes the wheel**. Focus and
`active_frontend` are written at `:2699`–`:2701`, *before* any replay
decision.

A consume check placed after that block leaves the exact half-state
AC48's activate-then-claim rule exists to prevent: **the wheel changes
FOCUS while scrolling nothing and claiming no controller.** The panel
steals focus and does not move.

**A terminal-chrome wheel is not a terminal gesture at all**, and the
dispatcher must treat it that way. The order is:

1. Authenticate and validate the panel event (the existing ladder).
2. Resolve the side window and determine its **buffer kind**.
3. **If terminal + chrome wheel: consume IMMEDIATELY** — before
   `focus_window`, before `active_frontend`, before any controller
   claim, before any command-chain mutation, and before the shared
   terminal path.
4. Otherwise, the existing activation and replay rules.

Step 3 is implementable where it needs to be: `is_terminal` is resolved
from `buffer_id` **before** the activation block, so the kind is
already known at that point.

**Witness — one frontend across a document→terminal replacement**, so
the two outcomes are separated by nothing but the buffer kind:

1. panel shows a **document**; wheel over chrome → **the panel
   scrolls**.
2. the panel is replaced by a **terminal**; wheel over chrome →
   **nothing changes at all** — no child bytes, no scrollback movement,
   no document scroll, **and the focused window and terminal controller
   identity are unchanged**.

##### Leg 2 must START PASSIVE, or "focus unchanged" is vacuous

**A focus assertion proves nothing if the panel already has focus.**
With the terminal side window active, moving the consume check below
activation calls `focus_window` on **the window that is already
active** — nothing changes, and the row stays green against the very
mutation it exists to catch.

**Preconditions, asserted before the wheel, not merely arranged:**

- the **primary document window is active**;
- the **terminal side window is distinct from it and PASSIVE**;
- the **terminal controller identity is captured as a baseline**.

Asserted rather than assumed, because each is exactly the kind of setup
detail a later edit silently changes — and each failure mode is a
witness that passes while proving nothing.

**With that setup the two mutations separate cleanly, and each is
caught by a DIFFERENT assertion:**

| mutation | what it does | caught by |
|---|---|---|
| the consume check sits **below** the activation block | `focus_window` runs and the panel takes focus; nothing scrolls, so every movement assertion still passes | **focus** — and focus alone |
| the terminal branch calls `apply_terminal_gesture` | `claims_control` is `!matches!(kind, Move)` (`src/editor.rs:3555`), so a wheel **claims the panel's controller at `:3571`, before any local handling** at `:3575`; the chrome coordinate then fails the reporting bounds check and drops into the local branch, which may also move scrollback | **controller identity** — plus scrollback when it moves |

**The two assertions are not interchangeable**, which revision 11's
prose blurred by naming them together. **Focus catches the ordering
mutation; controller identity catches the shared-path mutation.**
Activation alone does not claim a controller — `focus_window` and
`claim_terminal_controller` are separate acts — so a row asserting only
controller identity would miss the below-activation ordering bug
entirely, and one asserting only focus would miss a replay that quietly
claims the terminal.

Doing it in one frontend across a replacement is what makes it a
control rather than two unrelated observations: the geometry, the
pointer position and the producer path are identical, and only the
target differs.

**Both crossings need witnesses, and they fail in opposite
directions:**

- **Chrome → content:** press on the mode line, drag into content.
  **No `Drag` reaches the daemon**, because nothing armed. *Mutation:
  arm on a chrome press — the orphan appears.*
- **Content → chrome:** press in content, release over the mode line.
  **The gesture terminates at the last content coordinate.** For a
  **document** panel the selection commits and the latch clears; for a
  **reporting terminal** the child receives the release **at the
  content row**. *Mutation: reserve `Up` for documents — the gesture
  hangs latched; pass the raw chrome coordinate for terminals — the
  child receives no `Up` at all, because the bounds check drops it into
  the local branch.*

#### R-c2. The producer does not remember where the gesture began

`set_panel_pointer_held` **clears `last_pointer_cell`**
(`pmacs-gpu/src/main.rs:7250`), and only `panel_motion_is_new`
(`:7238`) ever fills it. `panel_release_cell` falls back to that field
when the release is not over a cell (`:7225`–`:7230`).

Today that fallback is rarely reached, because chrome *is* a
`PanelCell`. **Once R-c stops chrome being one, it becomes the normal
path** — and it is empty. A `Down` in content followed **immediately**
by a release over chrome or outside the band, with **no intervening
motion**, has no coordinate to fall back to, so the `Up` is either
dropped or sent with nothing.

**Revision 8 said "retain the `Down` cell in that field". That would
break a tested guarantee.** `last_pointer_cell` is cleared on press
*deliberately*, and a live test says why:

> *"A press or release re-arms it: the first drag after a press must
> reach the daemon even at the cell the press landed on."*
> (`pmacs-gpu/src/main.rs:19841`–`:19844`)

Storing the `Down` cell there would make the press's own cell the
dedupe baseline and **suppress that first `Drag`**.

**Ruled: a SEPARATE field, `gesture_last_content_cell`.** The two have
different jobs and conflating them was the error —
`last_pointer_cell` is a **wire dedupe baseline**, answering *"is this
motion worth sending?"*; the new field is a **termination fallback**,
answering *"where did this gesture last legitimately point?"*. They
have different lifetimes and different reset rules, so one field cannot
serve both without one job corrupting the other.

- **Written** on arm (the `Down` cell) and on every accepted content
  motion.
- **Reset** alongside the rest of the gesture state: on release, and on
  **both** identity changes (R-d's D1 and D2).
- **Never consulted** by `panel_motion_is_new`, which keeps its own
  baseline and its existing behaviour unchanged.

*The alternative was considered and rejected:* ruling that `Down`
becomes the dedupe baseline, retiring the guarantee and its test. It
would need a justified mutation showing the first same-cell `Drag` is
redundant, and it is not obviously so — the daemon's drag state is
established by that event. **Preserving a tested contract beats
retiring one to save a field.**

Row: `Down` in content → release over chrome, **no intervening
motion** → the `Up` carries the **`Down` cell's** coordinate.
*Mutation: reset `gesture_last_content_cell` on arm instead of writing
the `Down` cell — the release has no coordinate.* **For a reporting
terminal the row asserts the exact child release BYTES**, not merely
that the frontend latch cleared: a latch that clears while the child
never hears the release is the failure this row exists for.

**A second mutation guards the separation itself:** make
`panel_motion_is_new` consult `gesture_last_content_cell`. The existing
first-`Drag`-after-press assertion (`:19841`) must fail — which is what
proves the new field did not quietly become the dedupe baseline after
all.

#### R-d. Panel replacement leaves the frontend's gesture latch armed

`PanelFramePayload::Absent` clears `pointer_held` and
`last_pointer_cell` (`pmacs-gpu/src/main.rs:6909`–`:6910`). **The
`Present` arm does not** (`:6913` onward) — it validates, swaps the
frame, rebuilds buffers, and leaves the latch exactly as it was.

So: press on panel **A**, A is replaced by **B**, and the held latch
emits a `Drag` or release **for B with no B press**. **Acceptance 49
cannot reject it** — the event carries B's *current* epochs, so it is
not stale by any test 49 applies. 49 is a staleness gate, and this is
not a stale event; it is a **well-formed event from a gesture that
belongs to a presentation that no longer exists.**

**And panel identity is only half of it.** The two epochs move
independently, by design (`pmacs-protocol/src/panel.rs:61` onward):
`panel_epoch` is *"stable across ordinary frames of one continuously
present window/buffer"*, while `geometry_epoch` *"moves whenever the
frontend declares new effective cell geometry — including a font or
scale change that leaves `CellSize` identical"*. A font or scale change
therefore advances `geometry_epoch` with `panel_epoch` untouched, and
`next_geometry_declaration` (`pmacs-gpu/src/main.rs:6847`) advances it
**without clearing either pointer field**.

So a held gesture **resumes under a new grid** — new cell advance, new
row heights, the same cells meaning different text — carrying epochs
that are *current and valid*. Acceptance 49's geometry-race check
cannot help: it rejects events bearing a **stale** `geometry_epoch`,
and this one bears the new one.

**Reset the gesture latch on a change of EITHER identity.** The
precedent is in the same file: the **divider** drag latch already
carries both epochs and self-invalidates when the presented frame
differs in either (`:7288`). The pointer latch never got it.

**Both fields, and both must be constrained separately.** Clearing only
`pointer_held` kills the orphan drag but leaves `last_pointer_cell`
set, and `panel_motion_is_new` (`:7238`–`:7242`) then **suppresses B's
first same-cell `Move` as a duplicate** — the successor's opening
motion silently vanishes. The fields fail differently and need
independently discriminating legs.

| # | mutation | must bite |
|---|---|---|
| D1 | no reset on `Present`→`Present` (panel identity) | the replaced-panel orphan-drag row |
| D2 | no reset on `geometry_epoch` change | the font/scale held-gesture row |
| D3 | reset clears `pointer_held` only | the successor's first same-cell `Move` row |
| D4 | reset on **every** frame | the negative row below |

**The negative leg is required**, or D1/D2 are satisfiable by resetting
unconditionally: an **ordinary same-identity frame refresh must NOT
cancel a live gesture**. A panel repaints constantly during a drag;
resetting on each frame would make selection impossible.

**D4's frame must be a CHANGED frame with unchanged epochs**, and the
natural one is **the focus repaint the `Down` itself causes** — the
panel activates, its mode line and cursor render differently, and the
frame arrives mid-gesture with both epochs identical. A byte-identical
duplicate would not exercise the rule at all: production returns early
on one, *"A duplicate does no work — not even a reshape"*
(`pmacs-gpu/src/main.rs:6918`–`:6919`), so the reset code is never
reached and D4 passes against a broken implementation.

R-d's orphan is distinct from R-b's: **R-b's arrives from a passive
panel, R-d's from a replaced or re-declared one**, and an
implementation can fix either alone.

### Revisions 14–16 — what §5b changed underneath this lane

#### Q#BP-R4 — what does `dispatch_semantic_panel_pointer`'s answer MEAN? **RULED: a three-state outcome, not a bool**

**This question did not exist before the merge, and it was created by
two branches agreeing on a type while disagreeing on its meaning.**

| branch | what `true` meant |
|---|---|
| §5b | the gesture was **ACCEPTED** — and §5b's review round 4 made the accepted-gesture latch follow exactly this answer |
| this lane | the event was **CONSUMED HERE** — including the chrome swallows Q#BP-R2 and R-c introduced |

Both were right in their own tree. Merged, they are one `bool` that
means two things, and the daemon reads it as §5b's. **The consequence
is live in `b758c2e`:** a press on the band's **mode line** returns
`true`, so the latch **arms** for a gesture that never began in
content. That is the precise defect class §5b's round 4 found and
fixed, re-entering by merge rather than by edit.

**Three candidates were considered.**

| candidate | why not |
|---|---|
| keep `bool`, make chrome swallows return `false` | behaviourally correct **today**, and that is the whole problem — it silently merges "refused as malformed" with "handled and deliberately stopped", so the next author restores the collision without touching a test |
| keep `bool`, re-derive "was this content?" in the daemon | **violates the seam §5b established.** One authoritative derivation, read through one accessor, is why the mapping generation works at all; a second derivation beside the dispatcher is the same hole in a new place |
| **a three-state outcome** | **RULED** |

```rust
/// How an authenticated panel event relates to the authoritative
/// panel surface, decided BEFORE any target effect.
pub enum PanelPointerOutcome {
    /// Not addressable as this panel: no grid, out of grid, no side
    /// window, or a buffer that is not the one shown there.
    Refused,
    /// This panel owns the cell/event, but it is deliberately not a
    /// content gesture --- the chrome claims of Q#BP-R2 and R-c.
    Consumed,
    /// A content gesture for the resolved target.
    Accepted,
}
```

**The outcome is a DISPOSITION, not proof that an effect already ran.**
It is decided before replay. The current function combines validation,
classification and mutation, but that ordering cannot implement §5b's
stale-tail rule: an `Up` or `Drag` with no accepted `Down` has already
reached the child/selection before the daemon consults the latch. The
implementation may split the function into classify/apply helpers or
use an equivalent single wrapper, but these two facts are fixed:

1. the editor remains the **only** authority that derives
   `Refused`/`Consumed`/`Accepted`; the daemon never re-derives chrome,
   target kind or content bounds;
2. disposition and the live-gesture check both complete **before** a
   left `Drag`/`Up` target effect.

**One transition table is authoritative:**

| event | `Refused` | `Consumed` | `Accepted` |
|---|---|---|---|
| `Down(Left)` | no effect; do not arm | no target effect; do not arm | apply the content press, then arm **from the effect result** so target/domain/encoding and `reached_child` describe what actually happened |
| `Drag(Left)` | no effect; retain any live record | no target effect; retain any live record | no live record → inert; live record → continue in the **recorded domain**, update its last valid content cell |
| `Up(Left)` | no effect; retain any live record for a later authoritative cancellation | no live record → inert; live record → deliver the **recorded completion** at its last valid content cell, then take it | no live record → inert; live record → apply the ordinary in-content completion **once**, then take it; do **not** also run recorded completion |
| every other kind | no effect; no latch change | no target effect; no latch change | apply its one-shot content effect; never arm or consume the left-gesture latch |

This is the required asymmetry. A chrome press begins nothing. A
chrome release can end a live gesture, but a refused release cannot be
known to concern it. An accepted release already performs the ordinary
target effect; a consumed release has not, so it terminates from the
record. A stale `Drag`/`Up` with no accepted `Down` is inert rather than
sending a child tail or changing selection.

**"TERMINATE", not "consume", and the distinction is the whole of
review finding 1.** An earlier draft of this ruling said consume, and
justified it by claiming it prevents a child left button-down. **It
prevents no such thing.** Emptying the latch is bookkeeping;
`update_accepted_gesture` currently does `let _ =
state.consume_accepted_gesture()` (`src/daemon.rs:1067`) — it **takes
the record and drops it**. And on the path that matters, terminal
chrome returns at `src/editor.rs:2862`–`:2870`, **before**
`apply_terminal_gesture` at `:2899`, so the child is never told
anything. A latch that empties while the child stays pressed is
*exactly* the failure the draft claimed to prevent, now invisible
because the bookkeeping looks right.

**RULED: a live gesture plus non-`Refused` `Up(Left)` must produce the
target's completion effect exactly once, and the row asserts the
EFFECT, not the latch.**

| target | required completion effect |
|---|---|
| terminal panel, child reporting on | the **release is reported to the child in the recorded encoding** — an accepted content `Up` uses its accepted cell; a consumed/chrome `Up` uses the gesture's last content cell per R-c2 |
| terminal panel, child reporting off | the **local terminal selection completes** — the selection the drag built is finalised, not abandoned |
| document panel | the document gesture completes at the last content cell; an empty selection is cleared **without moving point** (the effect §5b's split table already assigns here) |

**And exactly once.** An accepted `Up` must not replay normally and
then also run the record-driven completion. Any completion must take
the record, so a later cancellation of the same gesture — reachable
through the four stranding transitions below — cannot repeat it.

| # | mutation | must bite |
|---|---|---|
| P1 | chrome press returns `Accepted` | a chrome `Down(Left)` does not arm |
| P2 | `Refused` treated as `Accepted` | §5b's four `g5_substrate_a_refused_*` rows plus their no-target-effect assertions |
| P3 | chrome `Up` empties the latch but delivers **no** completion | the child-release / selection-completion row — **the latch-only assertion must NOT satisfy this** |
| P4 | `Refused` release delivers a completion | a refused release leaves both the latch and the child untouched |
| P5 | an accepted content `Up` replays normally **and** invokes record-driven completion | the exact child stream contains one release, not two |
| P6 | completion delivered, then a later cancellation delivers another | the no-duplicate-release row |
| P7 | an accepted `Up` with no live record reaches the target | the stale-release row: no child bytes and no local/document selection mutation |
| P8 | an accepted `Drag` with no live record reaches the target | the orphan-drag row: no child bytes and no cursor/selection mutation |

**P3 is written against the earlier draft's own weakness.** A row that
asserts only `has_accepted_gesture() == false` passes while the child
receives nothing, so the row must read the child's reported bytes or
the terminal's selection state.

**That asymmetry is what earns the third state.** `Refused` preserves a
live gesture, `Consumed` owns the surface but runs only a recorded
completion, and `Accepted` reaches content. Each is behaviourally
distinct at the effect/lifecycle seam, and the single matrix above is
the witness contract.

**R-c2 does not discharge P3.** The producer normalizes a release that
lands on chrome back to the last content cell, so a conforming
frontend should not send one — but the daemon's contract cannot rest
on the producer's good behaviour, and **a legacy `PanelPointer` peer
predates that normalization entirely**. Producer-side normalization and
daemon-side termination are **both** required, and the existing GPU row
(`a_press_on_the_bands_mode_line_neither_arms_nor_reports_content`)
covers only the producer half.

**No TUI claim is made here.** An earlier draft said a TUI reaches the
same path; it does not. `dispatch_semantic_panel_pointer` has exactly
two callers, both `FrontendEvent` arms in `src/daemon.rs` (`:2609`,
`:2680`), so **only semantic legacy and mapped peers reach it** — the
TUI goes through `dispatch_mouse`.

#### The rows §5b's split table assigned here

§5b states its own split rather than leaving it to whoever runs the
tests, and this lane is the other column. Inherited, verbatim in
substance:

- the **document cancellation effect** — an empty selection cleared
  without moving point;
- **real stable-generation drag continuation**, G5a's **effects**, and
  the v24/v25 replay-effect controls **G4b, G6c, G7c**;
- the **complete producer reset lifecycle** — geometry, `Absent`,
  identity and detach — whose fields this lane introduces (**G5f**);
- **every** common legacy/mapped cancellation transition and the real
  gesture/click lifecycle (**G5b–e, G5g, G5i–p**), exhaustion
  cancellation (**G11b**), and both two-tick wheel effects (**G12**).

**The IDs stay as §5b wrote them.** §5b deliberately pinned its own
three decisions under `g5_substrate_*` names precisely so these IDs
would be unclaimed here; taking them is the point, not a collision.

#### The cancellation record has nowhere to wait

§5b landed cancellation as a **saturating count**, and said why: a
queue drained by nobody grows one entry per cancelled drag. It also
left the record itself reachable — `cancel_accepted_gesture` **returns**
the `AcceptedPanelGesture` it ends.

**The gap is not the count, it is the caller.** Two of the three
cancellation sites are inside `semantic_render.rs`, reached during
**frame production**, where a release cannot be delivered. So the
record is returned into a context that cannot act on it and is
dropped.

**RULED: one pending-release SLOT per frontend, not a queue.** The
latch holds at most one gesture per frontend, so at most one release
can be owed at a time, and the slot is bounded by construction rather
than by a cap someone has to choose.

**AND THE DRAIN ORDER IS PART OF THE RULING, because an invariant alone
is a detector, not a guard.** An earlier draft asserted only that
arming over a pending release is impossible. That check would sit
inside `arm_accepted_gesture` — and the seam replays **before** it:
`dispatch_semantic_panel_pointer` runs its effects and only then does
`update_accepted_gesture` arm (`src/daemon.rs:2608`–`:2616`). So the
invariant can fail **only after the new press has already reached the
child or moved the selection**. It would report the collision one
effect too late.

**A pending termination DRAINS FIRST:**

1. **before any subsequent panel-pointer effect** for that frontend —
   the drain happens ahead of `dispatch_semantic_panel_pointer`, not
   after it, so the old gesture's release reaches the child before the
   new gesture's press does;
2. **before detach teardown**, so a frontend that goes away does not
   take an owed release with it — detach is one of the four stranding
   transitions below, and it is the one with no later opportunity;
3. **after semantic projection returns and before any message it
   returned is filtered or written.** Mapping cancellation is
   discovered *inside* `SemanticRenderState::render_frame`, while the
   successor `PresentMapped` is being constructed, so "before that
   frame is produced" is impossible. The enforceable daemon seam is
   immediately after `sem.render_frame(editor)` returns
   (`src/daemon.rs:1428`–`:1430`) and before `for msg in &messages`
   (`:1591`). The pending termination drains there, so the successor
   frame cannot overtake the release its new mapping required.

| # | mutation | must bite |
|---|---|---|
| Q1 | drop the record instead of parking it | the cancelled-gesture release row |
| Q2 | park it but never drain | the same row, from the delivery side |
| Q3 | drain **after** `dispatch_semantic_panel_pointer` instead of before | the **old-release-before-new-press ordering row** — assert the child's byte stream carries the release ahead of the press, not merely that both arrive |
| Q4 | skip the drain on detach | the detach row: an owed release is delivered before teardown |
| Q5 | enter the returned-message write loop before draining a projection-raised release | the daemon-order witness records the termination effect before the first successor-frame write |
| Q6 | allow arming over a pending release | the invariant witness — kept, now as a **backstop** behind the ordering rows rather than as the guarantee |

#### Four transitions strand a live gesture, and they become defects HERE

§5b recorded this as inert and named the branch that owns it: a
**panel-epoch change**, a **buffer replacement**, a **same-size
geometry change** and a **detach** all leave the latch armed with a
release that can never be accepted. Only `Absent` was wired, because
`publish_absent_panel` already cleared input authority two lines
later and omitting it would have been an inconsistency inside one
function.

**Inert became a defect the moment this lane attaches effects.** Each
of the four needs its own leg, and they fail differently — R-d already
established that for the producer side, where clearing `pointer_held`
without `last_pointer_cell` silently eats the successor's first
motion. The daemon side has the same shape and needs the same
independently discriminating legs rather than one shared "resets
something" assertion.

#### Re-measurement closed in revision 16

The authoritative clause table is now the `2c0d3ff` table at the start
of §5a. The `72da24a` table is retained only under its explicit
**Superseded** heading. Revision 14's line-movement inventory did its
job by forcing a clause-by-clause scout; it is no longer a live
implementation obligation.

#### Coherence impact (`COHERENCE.md` §20)

- **Journey steps touched: 5** — Edit, clause (c) selection, kill and
  yank — **and 8** — Open terminal, clause (b) input and output
  round-trip. **NO JOURNEY-CELL GRADE MOVES**, and an earlier draft
  claimed one. Checked against the authoritative scorecard rather than
  inferred from this section's own MISSING column:
  - **Step 5 / GPU is `Partial`, and stays `Partial`.** It is floored
    by **5(a)** — no IME, no `set_ime_allowed`, so composed and CJK
    input is impossible (`COHERENCE.md:468`). Completing 5(c) inside a
    panel cannot lift a cell held down by a different clause. That is
    §2a's aggregation rule working as designed.
  - **Step 8 is already `Works` on all three columns**
    (`COHERENCE.md:471`). There is no grade left to move; 8(b) is
    hardened, not opened.
  - The draft's error was reading §5a's MISSING column as if it were
    the scorecard. **A section-local gap list is not a journey grade**,
    and the one place that decides grades is `COHERENCE.md`.
- **Interaction islands: none added.** It **completes** the existing
  panel island rather than opening a new one — the same gestures the
  band already advertises, finally reaching their target.
- **Config registry: no entry.** Nothing here is tunable.
- **Background work: none started**, and no existing attribution
  changes.

#### What revisions 14–16 do NOT reopen

It does not re-open Q#BP-R1 (single click selects only), Q#BP-R2 (a
terminal panel's chrome wheel is consumed, not clamped), or R-a/R-b/
R-c/R-c2/R-d. Those are approved at revision 12 and unaffected by §5b
— **except** where R-c's chrome swallows now feed Q#BP-R4's outcome,
which changes their return value and not their behaviour.

## 5b. The cell-mapping generation — a protocol slice (Q#BP-R3)

**Status: revision 16 — APPROVED 2026-08-15. IMPLEMENTED AND MERGED as
#242 (`47b5463`), 2026-08-20.** The "Nothing implemented" this line
carried until then was left behind by the merge; the slice's own
"What the slice actually landed, row by row" subsection below had been
contradicting it.
Revision 16 answers review of 15; three of its items reverse a rule 15
introduced:

- **The mapped-generation producer rule contradicted proactive
  cancellation.** Since the daemon cancels *before* emitting that
  successor frame, the frontend simply **clears its latch on accepting
  the frame and sends nothing**. 15's "emit a tail or retain the
  latch" was redundant in the first case and **actively harmful** in
  the second — retaining it manufactures a `Drag` under the new
  generation with no accepted `Down`, the exact orphan the section
  exists to prevent. Other authority losses have their own reset
  signals below.
- **The latch had one trigger and needed five.** Cancellation runs on
  every loss of gesture authority: generation advance, `Absent`, panel
  epoch or buffer identity change, geometry-epoch change **even at an
  unchanged cell total**, and detach. The last four apply to the legacy
  and mapped families alike; generation advance exists only in the
  mapped family. **An ordinary accepted `Up(Left)` must clear the
  latch**, or a later invalidation synthesises a duplicate release.
- **"The replacement frame" was not one producer signal.** A mapped
  generation or panel/buffer replacement clears on atomic acceptance of
  its valid frame; `Absent` clears when accepted; a local geometry
  declaration and detach clear at their own transition, before any
  replacement frame can be required. Invalid frame payloads and
  same-identity repaints retain the latch.
- **Gesture liveness and click counting are different state.** The
  accepted-gesture latch covers an accepted `Down(Left)` only and is
  consumed by `Up(Left)` or loss of authority. `MouseClickState`
  deliberately survives an ordinary `Up` for multi-click detection,
  but its mapping identity must be cleared or re-keyed on every mapping
  invalidation even when no gesture is held.
- **Routing controls had been written as effects.** This base can prove
  legacy and mapped events reach the already-landed focus path; it
  cannot prove selection, terminal reporting, drag continuation,
  two-tick scrolling, or exhaustion cleanup before replay exists.
  Those controls are split, and the effect halves are obligations of
  the rebased replay lane rather than claims made by this slice.
- **Bilateral gating needed all four wrong-family legs.** The v25
  refusal rows did not constrain a v24 session sending or accepting the
  mapped variants. Same-crate peers can encode them regardless of what
  they negotiated, so both inbound and outbound are now witnessed at
  v24 and v25, with exact-family positive controls.
- **G9b's mutation was a valid implementation.** Keying dedupe by
  `(mapping_generation, coord)` is a correct design, so requiring it to
  fail would have forbidden a good one. Replaced with two real defects.
- **"Projected cell identity" contradicted the styling control** — wire
  `Cell` equality includes `style`. Terminal identity is now glyph and
  row topology plus the view anchor, **excluding face, style and
  cursor**, with a same-glyph/different-style control.
- Zero-generation rows added **in both directions**; G7 split into
  outbound and inbound legs; G2's grid and fold composites split;
  journey steps **5** and **8** named in §20.
- **The folded closure pass made the remaining implicit contracts
  executable in one matrix:** authenticated-session family authority;
  a frontend generation high-water mark surviving `Absent`; exact pins
  for the new variants as well as their previous finals; left-only and
  fixed-domain terminal gestures; exact-once coincident invalidations;
  independent click identity; the semantic/TUI boundary; and explicit
  ownership of every route versus effect row (G0–G15).

**Previously, revision 15 — SUPERSEDED.**
Revision 15 answers review of 14, and four of its items reverse
something 14 asserted:

- **"Beside `Present`/`PanelPointer`" was positionally dangerous.**
  Appended means **last**: `PresentMapped` after **`Absent`**,
  `PanelPointerMapped` after **`TextInput`**. Exact field order, `u64`,
  **zero invalid**, and `PANEL_MAPPING_MIN_VERSION` are now stated.
- **Blanket refusal stopped the wheel after one tick.** The first
  effective wheel advances the key and the next queued tick carries the
  old one. **Coordinate-free** gestures — document wheel, local
  scrollback — are **exempt**; **child-reported** wheels keep the
  check, because SGR carries row and column. Two-tick witnesses added.
- **Cancellation is now PROACTIVE and has a latch.** Reactive
  cancellation loses a race: a replacement frame arriving before the
  physical `Up` makes the producer suppress the very event that would
  have cancelled. It triggers on the key advancing, and the daemon
  keeps an accepted-gesture latch — was the `Down` accepted, did it
  reach the child, with which coordinate, button and encoding. A stale
  `Up` with no accepted `Down` is inert, and cancellation never
  reclaims a controller another frontend took.
- **The existing screen generation cannot be the terminal key** — it
  advances from 39 sites including style, bell, tab-stops and
  cursor-only motion. A **dedicated terminal mapping revision** is
  defined, with those events as **stable controls**.
- **G5's effects moved to the rebased replay lane**, which is the only
  branch where they exist; **G5a stays here**, so the proactive rule
  keeps a witness in the slice that introduces it.
- **The `PanelPointer` pin exists** (`src/protocol.rs:1975`) — revision
  14's claim that it could not be found was wrong.

**Previously, revision 14 — SUPERSEDED.** This
slice **blocks** panel-pointer replay (`panel-pointer-replay`), which
blocks GUI arc 1b. It is protocol-bearing and runs alone.

### What it fixes

A `PanelPointer` names a **cell**; the daemon must invert that to a
**byte**. Nothing on the wire says which inverse mapping the frontend
was looking at, so the daemon inverts against whatever is current — and
the mapping can move for reasons the clicking frontend neither caused
nor can observe.

This is the one hole in the ladder: `buffer_id` catches replacement,
`panel_epoch` catches close/reopen, `geometry_epoch` catches a
declaration race, and **nothing catches "the text under that cell
changed"**.

The hole exists only across the **semantic frontend's asynchronous
projection/inbound seam**. The TUI hit-tests and dispatches against its
current daemon state in one process; it neither receives `PanelFrame`
nor echoes a mapping generation. This slice adds no TUI token, gate or
gesture state, and the existing direct TUI panel effects remain a
regression control. Applying the mapped-family check inside the shared
replay body instead of at the semantic-session boundary would break
that control.

**Revision 12 accepted this as narrow. It is not.** A **foreign** edit
moves the mapping with `view_top` untouched; ticks, paging, folds,
edits and reloads accumulate without bound; and the window lasts until
the frontend **presents** the replacement frame, which a backed-up
frontend widens arbitrarily.

### The generation, and why not a token

**A per-frame token is the wrong object.** Panels repaint constantly,
and a token that moved with the frame would invalidate a live drag on
the next repaint — the mistake `panel_epoch` is stable to avoid.

**`mapping_generation` identifies the INVERSE MAPPING.**

| changes it | leaves it alone |
|---|---|
| `view_top` | focus gained or lost |
| **`view_left`** — 1b makes horizontal scrolling real | styling, theme, face changes |
| panel grid size | selection-only changes |
| fold state | a re-emitted identical frame |
| wrap mode, gutter geometry | **cursor motion that moves nothing else** |
| **buffer content — any edit, from any source** | |
| **terminal output or scrollback movement** (terminal panels) | **terminal buffer revision** (see below) |

**"Cursor movement is stable" is CONDITIONAL, and revision 13 stated it
flatly.** A cursor move that triggers vertical or horizontal follow
changes `view_top` or `view_left`, and therefore **does** change the
generation. The stable case is a cursor move the follow rules absorb.

**Terminal panels need their OWN mapping revision, and the existing
screen generation cannot supply it.** What a coordinate denotes there
is decided by the terminal's screen — so output and scrollback movement
change the mapping, while the buffer's revision counter tracks
something else entirely.

But `Screen::changed()` is **not** that signal.
(`src/terminal/screen.rs:1467`) bumps a single generation from **39
call sites**, including `SetStyle` (`:270`), `Bell` (`:287`), the
tab-stop operations (`:309`–`:319`), cursor-only motion (`:324`) and
`SetTitle` (`:449`). **None of those changes what a coordinate
denotes**, and keying on it would cancel a drag every time the child
recoloured a character or rang the bell.

**Define a dedicated terminal mapping revision** over the things that
actually decide the inverse:

- **glyph and row TOPOLOGY** — which glyphs occupy which cells, and
  which rows the projection holds;
- **retained-row identity** — scrollback rows entering or leaving the
  projection;
- **the per-view scroll anchor**.

**Explicitly EXCLUDING face, style and cursor**, and revision 15's
"projected cell identity" got this wrong. The wire `Cell` derives
`PartialEq` over `glyph`, **`style`** and `attachment`
(`pmacs-protocol/src/cell.rs:153`), so an identity keyed on cell
equality **moves on a pure recolour** — which contradicts the very
stable control that rules style out, in the same section. The two
statements could not both hold.

**Control row: same glyphs, different style** — the child repaints the
projection in a new colour and the revision **does not move**, so a
drag survives it. That is the row that catches an implementation
reaching for `Cell` equality because it is right there.

**Stable controls are required, not optional**: style, title, bell,
tab-stop and cursor-only operations each get a row asserting the
revision **does not move**. Those are precisely the events the
convenient existing counter would have caught, so a reader who later
reaches for it fails a test instead of shipping a cancelled drag.

**The stability half is load-bearing, not an optimisation.** A drag
provokes selection repaints on every motion; a generation that moved
with them would cancel the drag after one step.

### Ownership and refresh timing

**One authoritative per-frontend mapping key**, owned by the daemon,
**used by both projection and inbound validation**. Not two derivations
that agree by inspection.

- It **advances after any mapping mutation and BEFORE the next inbound
  pointer is handled**, whether or not a frame has been rendered or
  emitted since.
- **Comparing against the last EMITTED frame recreates the hole.** A
  mutation that has not yet been painted still changes the inverse
  mapping, and a gesture arriving in that gap must be refused.
- Projection stamps the frame with the same key it validates against,
  so "what the frontend was shown" and "what the daemon checks" cannot
  drift.

### Wire shape — appended at the END, with the field order stated

Postcard encodes enums **positionally**, so a shipped variant's field
list is frozen **and so is every discriminant**. Revision 14 said
"beside `Present`" and "beside `PanelPointer`", which is **positionally
dangerous** — "beside" reads as *adjacent*, and inserting adjacent to
an existing variant shifts every discriminant below it. **Appended
means LAST.**

```text
PanelFramePayload            FrontendEvent
  0  Present(PanelFrame)       …
  1  Absent                    n-1  PanelPointer   (v21)
  2  PresentMapped  <- NEW     n    TextInput      (v24)
                               n+1  PanelPointerMapped  <- NEW
```

`PresentMapped` goes **after `Absent`**, not after `Present`;
`PanelPointerMapped` goes **after `TextInput`**, not after
`PanelPointer`.

**Exact shapes**, field order frozen on landing:

```rust
PanelFramePayload::PresentMapped {
    frame: PanelFrame,           // unchanged, reused whole
    mapping_generation: u64,
}

FrontendEvent::PanelPointerMapped {
    frontend_id: FrontendId,     // untrusted, as every inbound variant
    geometry_epoch: u64,
    panel_epoch: u64,
    buffer_id: BufferId,
    coord: CellCoord,
    kind: MouseKind,
    mods: Modifiers,
    mapping_generation: u64,     // appended last within the variant
}
```

`PanelPointerMapped` deliberately mirrors `PanelPointer`'s field order
with the generation **appended**, so the two are diffable by eye and a
future reader can see that nothing was reordered.

- **`mapping_generation` is `u64`.**
- **Zero is INVALID** and is refused like a mismatch: it is the value a
  default-constructed or partially-initialised sender produces, and
  accepting it would let a peer opt out of the check by sending
  nothing. A live key starts at 1.
- It is **monotonic for the authenticated frontend session**. A
  frontend accepts a valid generation above its retained high-water
  mark (gaps are fine), and accepts an equal-generation repaint because
  styling, focus and selection may legitimately change while the
  mapping holds. It rejects a lower generation atomically. `Absent`
  clears the presentation and pointer state but **retains the
  high-water mark**; otherwise a delayed pre-`Absent` frame can roll
  authority backward. Detach ends the session and its high-water mark.
- **`PANEL_MAPPING_MIN_VERSION = 25`**, beside the existing family
  constants, and the gate reads that constant rather than a literal.

`Absent` is unchanged and **common to both families** — hiding a band
carries no mapping.

### Bilateral gating — REFUSAL, not fallback

Revision 13 said an unmapped event from a new peer is "handled under
the old semantics". **That is a bypass**: it leaves the exact hole the
slice exists to close, reachable by omitting a field.

| negotiated | daemon sends | daemon accepts | frontend sends | frontend accepts |
|---|---|---|---|---|
| **≤ v24** | `Present` | `PanelPointer` | `PanelPointer` | `Present` |
| **≥ v25** | `PresentMapped` | `PanelPointerMapped` **only** | `PanelPointerMapped` | `PresentMapped` **only** |

Both choices use the **authenticated session's negotiated version**.
Inbound code never derives the family gate from the payload's claimed
`frontend_id`; like every inbound variant, that field is untrusted and
is validated separately. Outbound projection uses the destination
session's negotiated version, not the crate's compile-time ability to
encode either discriminant. The family gate runs before focus,
controller, click-chain, gesture-latch or document mutation.

- A **≥ v25 session sending bare `PanelPointer` is REFUSED**, dropped
  before any mutation, exactly as an out-of-epoch event is.
- A **≥ v25 frontend receiving legacy `Present` REJECTS it** rather
  than painting a band it cannot safely hit-test.
- Only a negotiated **≤ v24** session retains legacy semantics.
- `Absent` is accepted from either family.

### Enforcement, and the liveness it must not break

On `PanelPointerMapped`, the daemon compares the echoed generation with
the authoritative key and refuses the gesture before any mutation when
they differ.

#### The wheel exception — or scrolling stops after one tick

**A blanket refusal breaks the wheel, and revision 14's was blanket.**
The first effective document wheel changes `view_top`, which *is* a
mapping change, so the key advances. A second wheel event already
queued behind it carries the **old** generation and would be refused —
**the panel scrolls exactly one tick and then goes dead** until the
frontend observes the new frame. Local terminal scrollback has the same
shape, moving its view anchor.

The discriminator is **whether the gesture uses its coordinate**:

| gesture | uses `coord`? | generation check |
|---|---|---|
| document wheel | **no** — `scroll_window` is window-level | **EXEMPT** |
| terminal wheel, **local scrollback** (not reporting) | **no** — moves the view anchor | **EXEMPT** |
| terminal wheel, **child-reported** | **yes** — SGR carries row and column | **REQUIRED** |
| every press, drag, release, context gesture | yes | **REQUIRED** |

**Coordinate-free gestures cannot be mis-aimed by a stale mapping**,
because they never invert a cell. Refusing them buys nothing and costs
the feature. A **child-reported** wheel is the opposite case: it puts a
row and column into the child's input stream, so a stale one aims an
application action at a cell the user never pointed at — the same
hazard as a stale click, and it keeps the check.

The exemption skips **only generation equality**. It does not bypass
the authenticated family gate, the nonzero-generation requirement,
frontend/panel/geometry/buffer identity, coordinate bounds, or target
resolution. In particular, a zero-generation coordinate-free wheel is
still refused; zero means "no mapping identity was supplied", not "one
was supplied and has since gone stale".

**Witnesses: two ticks under ONE generation.** A document wheel and a
non-reporting terminal wheel must each scroll **twice** when the second
event carries the generation the first invalidated. Without a
two-tick row, a blanket-refusal implementation passes every
single-event row in the matrix.

**But a blanket drop breaks liveness, and revision 13's did.** A
refused **tail** is not the same as a refused **beginning**:

- **Stale BEGINNINGS may simply drop.** A `Down` that never took effect
  leaves nothing behind.
- **Stale TAILS must TERMINATE the gesture, not vanish.** A dropped
  `Up` leaves an empty document selection armed with a stale anchor,
  and leaves a **reporting terminal child holding a button forever**.

**Cancellation is therefore ruled as a first-class outcome — and it is
PROACTIVE, driven by the key advancing, not reactive to a refused
event.**

Revision 14 made it reactive, and that has a race it cannot win: if the
replacement mapped frame reaches the frontend **before** the physical
button comes up, the producer resets `pointer_held`, **suppresses its
own `Up`**, and the daemon is never told anything — so the selection
stays armed and the child keeps holding its button. **The cancelling
event never arrives.**

So the daemon cancels at the transition that revokes authority, not at
the next pointer event:

| authority loss | daemon cancellation point | protocol families |
|---|---|---|
| mapping generation advances | after the mapping mutation and **before** publishing the next `PresentMapped` | mapped, ≥ v25 only |
| `Absent` | **before** publishing `Absent` | legacy and mapped |
| panel epoch or buffer identity changes | before publishing the successor `Present`/`PresentMapped` | legacy and mapped |
| geometry epoch changes | when the new frontend geometry declaration is accepted, even when its rows and columns equal the old declaration | legacy and mapped |
| detach | in that frontend's teardown, before its pointer state is discarded | legacy and mapped |

If an accepted left-button gesture is live, cancellation is taken
**exactly once**:

- **document:** clear an empty selection, preserve an already-nonempty
  region, and apply no cursor move;
- **terminal:** deliver the child's **release**, at the last coordinate
  known valid, with the left button and encoding the accepted press
  used.

The click chain is invalidated at the same authority transitions but is
**not conditional on a live gesture**; its lifetime is specified
separately below.

Authority losses may coincide. A changed-size geometry declaration, for
example, advances both `geometry_epoch` and the mapping generation;
buffer/panel replacement can also change the projected mapping. All
causes take the **same** accepted-gesture latch, so the first
cancellation settles it and later causes observe no live gesture.
Click-chain invalidation is likewise idempotent. Trigger ordering may
not create two releases or two selection cleanups.

**Revision 15 asked the producer to emit a cancellation tail or retain
the latch. Both are wrong, and the second is actively harmful.** A tail
is redundant — the daemon cancelled first and would receive a release
for a gesture it has already settled, which is the duplicate release
the latch exists to prevent. **Retaining the latch is worse: it
manufactures a `Drag` under the NEW generation with no accepted
`Down`** — precisely the orphan this whole section is about, created by
the rule meant to avoid it.

**There is no single producer-side "replacement frame" transition.**
The producer clears its local held-gesture state at the first
authoritative signal available for the cause:

| cause | producer reset point |
|---|---|
| mapped generation changes | atomically accepting the valid successor `PresentMapped` |
| panel epoch or buffer identity changes | atomically accepting the valid successor `Present`/`PresentMapped` |
| `Absent` | accepting `Absent` |
| geometry epoch changes | locally, when `next_geometry_declaration` advances the epoch (`pmacs-gpu/src/main.rs:6847`), **before** sending the declaration |
| detach | frontend teardown |

An invalid frame and a repaint with the same generation and identities
**do not clear** the producer latch. A local geometry declaration is
different: advancing the local epoch has already revoked the old grid,
so the producer clears before the daemon can accept or refuse the
declaration. For the daemon-authored frame cases, ordering is cancel
then emit; the frame's arrival is the producer's signal, so no second
channel is needed. For geometry and detach, the local transition is
itself the signal and no replacement frame is required.

After any reset, subsequent motion and the physical `Up` produce **no
new drag and no duplicate release**. The producer never emits a
cancellation tail and never carries the held state into the successor
identity.

##### The latch's full lifecycle

**Cancellation runs on EVERY loss of gesture authority, not only a
mapping-generation advance.** Revision 15 named one trigger and left
the rest to be inferred:

| trigger | why it ends the gesture |
|---|---|
| **mapping generation advances** | the cells mean different bytes |
| **`Absent`** | the band the gesture belongs to is gone |
| **panel epoch or buffer identity change** | the successor never saw the press |
| **geometry-epoch change** | a new declaration, **even when the cell total is unchanged** — the frontend re-declared, so the grid it hit-tested against is not the one in force |
| **detach** | there is no frontend left to finish the gesture |

**And an ordinary accepted `Up` MUST clear the latch.** Otherwise a
later invalidation finds a gesture it believes is still live and
**synthesises a duplicate release** for a button already up. That is
the same orphan race the replay lane's D1/D2 producer resets expose,
arriving from the daemon's side instead of the frontend's.

**The daemon needs an ACCEPTED-GESTURE LATCH** to do any of this, and
revision 14 assumed state that does not exist. The latch is common to
legacy `PanelPointer` and mapped `PanelPointerMapped`, because four of
the five authority losses are common. Per frontend it records:

- whether a **`Down(Left)` was ACCEPTED**, and for a terminal whether
  it actually reached the child;
- the last valid coordinate and the encoding the child was told — a
  release must match the press it terminates;
- enough identity to know the gesture is still the one that began.

It arms only **after** `Down(Left)` is accepted, updates its last valid
coordinate on an accepted drag, and is taken by either an ordinary
accepted `Up(Left)` or one authority-loss transition. `Move`, wheel,
context, and right/middle press events never arm it. `MouseKind` has
right and middle `Down` variants (`pmacs-protocol/src/message.rs:220`),
and the GPU panel path currently emits `Down(Right)` from the
press-only `apply_right_press` route
(`pmacs-gpu/src/main.rs:2997`, `:3029`). Treating every `Down` as held
would therefore manufacture a delayed release at the next
invalidation. If right/middle release semantics are added, they need
their own complete producer/replay contract rather than silently
borrowing this left-drag latch.

For a terminal, the accepted `Down(Left)` also fixes the gesture's
**domain** until it ends: child-reported, or local terminal selection.
Shift precedence and the child's reporting mode decide the domain at
the beginning; later modifier or mode changes do not switch a live
gesture into a path that never saw its Down. Thus a child-reported
gesture sends Drag/Up using the recorded reporting contract and exact
release encoding even if reporting turns off or Shift is pressed, and
a locally-started gesture stays local even if Shift is released or
reporting turns on. This does not freeze wheel precedence: a wheel is
self-contained and never arms the gesture latch.

Two consequences fall out of the latch and are ruled here:

- **A stale `Up` with no accepted `Down` is INERT.** It terminates
  nothing, because nothing began; it must not synthesise a release or
  clear another gesture's state.
- **Cancellation never reclaims a controller another frontend has since
  taken.** The release settles *this* gesture; if ownership moved on,
  the claim is not taken back. Reclaiming would make a stale gesture
  steal a live one's terminal.

Detach has an additional ordering constraint: if the departing
frontend still owns the controller, its child's matching release is
delivered **before** normal detach teardown releases that claim and
discards the gesture record. If another frontend owns it, the release
still settles the recorded gesture without changing that newer claim.
Clearing controller/gesture state first would make the teardown path
look clean while leaving the child button down.

**A cancelled gesture is not a replayed one**: the release is delivered
for liveness, and no selection or scroll effect is applied from the
stale event.

##### Click-chain identity is independent

`MouseClickState` (`src/editor.rs:352`) is **not** the
accepted-gesture latch. An ordinary
`Up(Left)` consumes the gesture latch, while click state intentionally
survives long enough to recognize a second click. Consequently, only
clearing click state from the cancellation path is insufficient: after
a completed click there may be no live gesture to cancel.

The click chain therefore carries, or is cleared against, the complete
mapping identity: frontend, target window and buffer, panel epoch,
geometry epoch, and — for mapped sessions — mapping generation. Every
authority transition in the table above invalidates it independently
of the gesture latch. A completed click followed by a foreign edit and
the successor mapped frame, then a `Down` at the same cell inside the
multi-click interval, is a **single** click under the new mapping, not
a double-click on unrelated text. The same rule holds for the common
legacy identity transitions even though v24 has no mapping generation.

This extends rather than replaces Q#M5 and the existing TUI event
lifetime: only an **unmodified `Down(Left)`** writes click state;
Shift-Down never enters the double-click chain. Drag, Move, wheel and
right/context gestures clear it, while ordinary `Up(Left)` and a
same-identity repaint preserve it. Mapping invalidation is an
additional clear, not permission to couple click state to every frame
or to the accepted-gesture latch.

Semantic click chains are **keyed per frontend**, then carry the target
identity above. Merely storing `frontend_id` inside one global slot
prevents a false cross-frontend double-click but lets B's click erase
A's legitimate chain. The adjacent `window_drag` comment already names
that exact defect and uses a `HashMap<FrontendId, ..>` because
concurrent frontend gestures are legal (`src/editor.rs:298`). Detach
removes only the departing frontend's entry.

### Motion must stay coalesced

`PanelPointerMapped` carrying `Move` or `Drag` **takes the same
tail-coalescing tags as `PanelPointer`** (`pmacs-gpu/src/attach.rs:374`
onward). A new variant that fell through to the lossless default would
put pixel-rate motion on a bounded queue — the failure the coalescing
tags exist to prevent. `Down`/`Up`/wheel/context stay lossless and
ordered, as today.

### Pins ACCUMULATE

**They are not moved.** Revision 13 said the pin "moves to the previous
final variant", which would delete coverage of the shape it was
protecting.

| pin | covers |
|---|---|
| existing `FrontendEvent::PanelPointer` | retained, unchanged |
| **new**: exact `FrontendEvent::TextInput` bytes | the previous-final `FrontendEvent`, appended by 1a and never pinned |
| **new**: complete nested bytes of `InstanceMessage::PanelFrame(PanelFramePayload::Absent)` | the previous-final `PanelFramePayload` variant |
| **new**: exact `FrontendEvent::PanelPointerMapped` bytes, encoded **and decoded**, with distinct values in every same-typed field | the new discriminant and its frozen field order; a same-type field swap still round-trips against itself, so round-trip alone is insufficient |
| **new**: complete nested bytes of `InstanceMessage::PanelFrame(PanelFramePayload::PresentMapped { .. })`, encoded **and decoded** | the new payload discriminant, nesting and `frame`-then-generation order |

**Correction: the `PanelPointer` pin DOES exist**, at
`src/protocol.rs:1975`
(`panel_pointer_encoding_is_unchanged_by_the_v24_build`), and revision
14 said it could not be found. It is in the **root crate's** test
module rather than under `pmacs-protocol/` or `tests/`, which is
exactly where the search did not look. `message.rs:524` was right and
the doubt was mine. The pin is **retained**, and the four new pins are
added beside it. The previous-final pins catch an insertion on the
wrong side of the append boundary; the new-variant pins catch
reordering inside the variants themselves. Neither substitutes for the
other.

### Acceptance and mutation matrix

| # | row | mutation |
|---|---|---|
| G0a | the existing `PanelPointer` pin and the new exact previous-final `TextInput` and nested `PanelFrame(Absent)` pins all remain byte-identical | insert a wedge before each protected previous-final variant → only the pin whose discriminant moved fails |
| G0b | exact encode **and decode** bytes for `PanelPointerMapped` and nested `PresentMapped`, using unequal values for every adjacent same-typed field | reorder two `u64` fields or reverse `frame`/generation → a self-consistent round-trip stays green but the exact pin fails |
| G1 | a **foreign** edit before the next render → the old generation is refused | never advance on content change → G1 passes a stale hit |
| G2 | every **changing** entry moves the key, one row each — and the composites are **split**: `view_top`; `view_left`; grid **rows** and **columns** separately; **fold-map content** and the owning frontend's **`fold_projection` policy** separately; **wrap** and **gutter geometry** separately; buffer content; terminal **projected content** and **scrollback movement** separately | omit that one entry from the key |
| G3 | every **stable** entry leaves it unchanged, one row each — focus; styling; selection-only; absorbed cursor motion; and the terminal controls **style, title, bell, tab-stop, cursor-only** | include that one entry → drags cancel on a repaint |
| G4a | a **selection repaint** preserves the generation | as G3 |
| G4b | after that repaint, an in-flight drag reaches real replay and **continues** | route only or clear the latch despite the stable generation → G4a stays green while the gesture dies |
| G5a | **mapped generation**: the key advancing with an accepted left gesture live raises cancellation before the successor frame, without waiting for another pointer event | make cancellation reactive to a refused event → nothing happens when the frame lands before the physical `Up` |
| G5b | **table-driven common authority-loss matrix**: `Absent`, panel-epoch change, buffer replacement, same-size geometry-epoch change, and detach each cancel a live left gesture; every entry has legacy-v24 and mapped-v25 legs, and the rebased effect matrix applies each to a document and a reporting terminal | omit that transition, arm the latch only for the mapped family, or cancel only one target kind → its named leg leaves the gesture live |
| G5c | an ordinary accepted `Down(Left)` → `Up(Left)` consumes the latch; a later authority loss emits **no second** document cleanup or child release, with v24 same-size geometry and v25 generation legs | leave the latch armed after `Up` → the later transition duplicates the release |
| G5d | a stale `Up` with **no accepted `Down`** is **inert** | synthesise a release anyway → an unpressed button is released |
| G5e | cancellation **does not reclaim** a controller another frontend has since taken | reclaim it → a stale gesture steals a live one's terminal |
| G5f | **producer reset matrix**: mapped generation and panel/buffer replacement clear only on atomic acceptance of the valid successor frame; `Absent` clears on acceptance; same-size geometry clears at local declaration; detach clears in teardown; structurally invalid, stale, zero-generation and wrong-family frames plus same-identity repaints retain | funnel every reset through frame arrival → geometry/detach stay armed; clear before validation or on every repaint → the negative controls lose a valid drag |
| G5g | only an accepted **`Down(Left)`** arms cancellation; table-driven Move, wheel, context and right/middle controls followed by an authority loss emit no synthetic release | arm on every `Down` or every accepted pointer event → the press-only right path or another non-gesture event manufactures a delayed release |
| G5h | **click-chain identity is independent**: complete a click, then independently drive `Absent`→Present, panel-epoch change, buffer replacement, same-size geometry change, and detach→reattach with no live gesture; a same-cell click inside the interval is single. A foreign content edit plus successor generation is the additional mapped-v25 leg | clear/re-key `MouseClickState` only while cancelling a held gesture, or omit one identity → unrelated text is treated as the second/third click |
| G5i | reporting-terminal detach delivers the recorded child release before dropping the old gesture/controller state, then leaves no controller claim owned by the detached frontend; a newer owner's claim remains untouched | tear down controller or gesture state before cancellation → no release bytes; reclaim/release unconditionally → the newer owner is disturbed |
| G5j | document cancellation has two legs: Down with no accepted Drag clears the empty selection; Down plus accepted Drag preserves the nonempty region's exact anchor/cursor while ending the gesture | clear every selection → the user's completed range disappears; preserve every selection → the empty armed anchor captures later motion |
| G5k | **terminal gesture-domain matrix**: child-reported Down then (a) reporting off or (b) Shift held before Drag/Up still yields child Drag/release in the recorded encoding and no local selection; local Down via (c) Shift or (d) reporting-off then the condition reverses still finishes locally with no child tail | re-evaluate Shift/reporting independently on every event → the child keeps a button down or receives an Up for a Down it never saw |
| G5l | **click-chain stability control**: complete a click, accept a same-generation style/selection repaint with every identity unchanged, then click the same cell inside the interval → it remains the second click | clear click state on every frame or couple it to the ordinary-Up gesture-latch clear → multi-click never survives a harmless repaint/release |
| G5m | **coincident invalidations cancel once**: changed-size geometry (geometry epoch + mapping generation) and panel/buffer replacement that also changes the mapping each produce exactly one document cleanup/child release | keep per-cause live flags or inspect without taking the shared latch → one transition emits duplicate tails |
| G5n | **click event-lifetime table**: unmodified left Down writes; Shift-Down does not; Drag, Move, wheel and right/context clear; ordinary left Up preserves | write on Shift-Down or omit one clear arm → Q#M5 or the successor click is misclassified; clear on Up → every double-click becomes two singles |
| G5o | **per-frontend click ownership**: A click, B click, then A same-cell click inside A's interval still produces A's double-click, while detaching B removes only B's chain | keep one global `Option<MouseClickState>` or clear the whole map on detach → B erases A's valid sequence |
| G5p | **per-frontend gesture ownership**: A and B hold gestures on distinct panels; B's panel-epoch change or detach cancels B exactly once while A's next valid Drag still applies | keep one global accepted-gesture latch or clear the whole map on one frontend transition → B's lifecycle cancels or erases A |
| G6a | **v24 positive control, OUTBOUND** — a v24 peer receives exactly legacy `Present`, never `PresentMapped` | send the mapped family or both → an old negotiated session receives a discriminant it did not declare |
| G6b | **v24 routing control, INBOUND** — a current legacy `Down(Left)` reaches the semantic dispatcher and performs the already-landed focus activation | gate v24 off or drop after decoding → the old peer's event is mapped but inert |
| G6c | **v24 effect control after replay rebase** — the same event performs its document/terminal replay effect | keep only the pre-existing focus path → G6b stays green while replay is dead |
| G7a | **v25 positive control, OUTBOUND** — a v25 peer receives exactly `PresentMapped`, carrying a live generation, never legacy `Present` | withhold `PresentMapped`, send legacy, or send both → the mapped frame contract is absent or ambiguous |
| G7b | **v25 routing control, INBOUND** — a current `PanelPointerMapped` carrying `Down(Left)` reaches the semantic dispatcher and performs the already-landed focus activation | ignore the mapped variant inbound → every refusal row still passes while nothing routes |
| G7c | **v25 effect control after replay rebase** — that current mapped event performs its document/terminal replay effect | stop after focus → G7b stays green while replay is dead |
| G8a | a **bare `PanelPointer` from a ≥ v25 session is REFUSED before mutation** | accept it → the bypass returns inbound |
| G8b | a **legacy `Present` at a ≥ v25 frontend is REJECTED**, atomically retaining its mapped frame, generation and pointer state | paint it or clear state first → the frontend hit-tests a band it cannot map or loses a valid gesture |
| G8c | a **`PanelPointerMapped` from a ≤ v24 session is REFUSED before mutation**, even though a peer compiled from this crate can encode it | accept it → inbound negotiation is treated as a sender convention rather than a gate |
| G8d | a **`PresentMapped` at a ≤ v24 frontend is REJECTED**, retaining the previous legacy frame atomically | paint it → outbound negotiation is treated as a sender convention rather than a gate |
| G8e | **authenticated-session authority**: in both inbound wrong-family cases, claiming another frontend whose session negotiated the desired family still refuses | gate by the payload's `frontend_id` lookup → the forged cross-session claim bypasses negotiation |
| G9a | **daemon emission**: identical cells across a generation change are still **emitted** | dedupe daemon-side across the change → the second gesture is suppressed |
| G9b | **frontend motion dedupe** suppresses a repeat within one generation and **re-arms across one** | (a) compare **only the cell**, never keying or resetting by generation → the first post-change motion is **eaten**; (b) reset on **every same-generation repaint** → repeated same-cell motion **returns** as pixel-rate traffic |
| G9c | **frontend acceptance**: a byte-identical `PanelFrame` with a higher generation still updates the high-water/current generation and resets pointer state atomically; it may report no visual redraw | return early on frame equality before applying generation → G9a passes because the message arrived, but the producer keeps echoing stale authority |
| G10 | an **invalid** mapped frame retains the previous frame **and** its generation, atomically | update one without the other → a valid generation names a frame never shown |
| G10a | a **structurally valid `PresentMapped` carrying generation ZERO is REJECTED**, atomically — previous frame and generation both retained | accept zero → a sender that never initialised the field disables the check for the whole session |
| G10b | a **`PanelPointerMapped` carrying generation ZERO is REFUSED** before mutation, including a coordinate-free document and local-terminal wheel | apply the wheel exemption before the nonzero check → the inbound opt-out returns through the exempt path |
| G10c | mapped-frame generation is nondecreasing: a higher generation may skip values, an equal-generation style/selection repaint is accepted, and a lower generation is rejected atomically; after `PresentMapped(9)` → `Absent`, a valid `PresentMapped(8)` is still rejected | require only nonzero, or clear the high-water mark on `Absent` → a delayed frame rolls the producer's authority backward |
| G11a | **generation exhaustion fails CLOSED, and does not leave a zombie band**: the daemon publishes `Absent`, clears input authority, and latches exhaustion for the session | fail open → the hole returns at the boundary; refuse input only → a stale panel stays painted and permanently inert; omit the latch → the next frame resurrects it |
| G11b | exhaustion with an accepted left gesture also performs the ordinary `Absent` cancellation effects **before** publishing it | publish `Absent` and discard authority without replay cancellation → G11a stays green while the gesture is orphaned |
| G12a | one mapped generation carrying **two document wheel ticks** applies both window-level scrolls even though the first changes the mapping | check every mapped coordinate indiscriminately → only the first tick applies |
| G12b | one mapped generation carrying **two non-reporting terminal wheel ticks** applies both local-scrollback moves even though the first changes the view anchor | remove the terminal coordinate-free exemption → only the first tick applies |
| G13a | mapped `Move` and `Drag` retain their respective tail-coalescing behaviour, including replacement by the latest coordinates/generation without crossing an intervening lossless event | let the new variant fall through to the default → pixel-rate traffic fills the bounded queue |
| G13b | mapped press, release and all wheel kinds remain lossless and ordered | coalesce every mapped pointer → multi-click, release or wheel ticks silently disappear |
| G14a | version tripwire: `PROTOCOL_VERSION == 25`, `SUPPORTED_PROTOCOL_VERSIONS` reaches it, `PANEL_MAPPING_MIN_VERSION == 25`, and `ADVERTISED_PROTOCOL_VERSION == 20` remains pinned | bump only the wire constant or move the advertised baseline → self-handshake fails or the permanent baseline drifts |
| G14b | the supported set accepts every version through `PROTOCOL_VERSION`, rejects `PROTOCOL_VERSION + 1`, and the literal boundary routes v24 legacy versus v25 mapped | end the set at 24 or express the boundary arithmetically against a moving ceiling → the current wire is unsupported or a later bump silently moves this feature |
| G15 | **TUI structural control** — the existing direct panel click/drag/wheel paths keep their effects with no generation token | move the generation requirement into shared replay instead of the authenticated semantic boundary → local TUI input is refused |

**G9b's mutation in revision 15 was a valid implementation, not a
defect.** Keying the dedupe by `(mapping_generation, coord)` preserves
same-generation suppression *and* naturally admits the first motion
under a new generation — it is one correct way to satisfy the row, so
requiring it to fail would have forbidden a good design. The two
mutations above are actual defects, one in each direction.

G10a and G10b are **independent rows in opposite directions**, not one
row seen twice: zero can arrive from either peer, and G10's atomicity
row exercises neither.

G2 and G3 are enumerated per entry deliberately: one row asserting "the
generation changed" cannot show **which** input moved it, and a key
that ignores `view_left` passes every row that only scrolls
vertically.

The mutation column states each row's **minimum required bite**, not a
prediction that no legitimate dependent can fail. Alternatives `(a)`,
`(b)`, or distinct omitted table arms are separate mutations; one
mega-mutation cannot establish their individual coverage. Record the
actual bite graph only after each compiling mutant runs. A proposed
mutation that is itself a valid implementation is replaced, as G9b
was, rather than used to reject the implementation.

### What this slice can witness, and what the replay lane owes

**Gesture EFFECTS cannot be proven on this branch, and earlier
revisions claimed some anyway.** Cancellation's observable outcomes —
the complete producer latch, the panel's selection, the click chain and
the child's release — plus drag continuation, replay positive controls
and two-tick scrolling are all reached through **panel replay, which
does not exist at this base**.
`gesture_last_content_cell` and the document/terminal replay live on
`panel-pointer-replay` (`pmacs-gpu/src/main.rs:2143` there); the same
struct here (`:2124`) has no such field, and
`dispatch_semantic_panel_pointer` still validates and returns.

So the obligations split, and the split is stated rather than left to
whoever runs the tests:

| owned by **this slice** | owed by the **rebased replay lane** |
|---|---|
| wire shapes, positions, pins and field order (G0), plus the bidirectional family gate (G6a–b, G7a–b, G8) | document cancellation effect — empty selection cleared without moving point |
| authoritative-key changing/stable inputs (G1–G4a) and the **mapping-generation** authority-loss signal before frame emission (G5a) | real stable-generation drag continuation, G5a's effects, and the v24/v25 replay-effect controls (G4b, G6c, G7c) |
| mapped-frame atomicity and generation-aware producer state (the mapped-frame legs of G5f) | complete producer reset lifecycle, including the geometry, `Absent`, identity and detach paths whose fields are introduced by replay (G5f) |
| motion coalescing, dedupe, invalid-frame atomicity, zero rejection, exhaustion state, version fallout and the TUI structural control (G9–G11a, G13–G15) | every common legacy/mapped cancellation transition and real gesture/click lifecycle (G5b–e, G5g, G5i–p), exhaustion cancellation (G11b), and both two-tick wheel effects (G12) |

Only the **new mapping-generation** transition stays here on purpose,
and its classification does **not** discharge the effect row. The
common invalidations do not get a dead classification helper on this
base; replay owns them where accepted gesture state exists. The rebased
lane must run the whole G5 matrix through real document and terminal
call sites. This is the same producer-emits/receiver-discards blind
spot that forced the replay prerequisite in the first place.

#### The latch substrate that lands here, and what it does not claim

G5a needs something to cancel, so the accepted-gesture latch itself —
`AcceptedPanelGesture` and the per-frontend slot on
`SemanticRenderState` — lands on this slice, armed from the daemon's
accepted inbound arms. Writing that code decides three things the
deferred rows later assert about, so those decisions are pinned here
under **substrate** names (`g5_substrate_*`) rather than under G5c,
G5d, G5g or G5p:

- which events arm and which consume (`update_accepted_gesture`);
- that an ordinary `Up` consumes **without** counting as a
  cancellation;
- that the latch is per frontend, so one frontend's loss cannot empty
  another's.

The framing rows keep their IDs on `panel-pointer-replay`, because each
of them asserts something about a synthetic release or a real drag
continuation that does not exist on this base. Two IDs claimed in two
branches is the merge hazard; substrate names avoid it.

Two consequences are recorded rather than fixed:

- **Cancellations are counted, not queued.** A queue of records is what
  replay drains to deliver each release; landing it here would grow one
  entry per cancelled drag with nothing ever draining it. A saturating
  count is bounded and still separates a consume from a cancellation.
- **The other G5b transitions leave the latch armed on this base.** A
  panel-epoch change, a buffer replacement, a same-size geometry change
  and a detach all strand a live gesture — its release can never be
  accepted. That is inert here, because nothing consumes the latch, and
  becomes a defect exactly when replay gives it effects, in the branch
  that owns the row. `Absent` is wired anyway, because
  `publish_absent_panel` already clears input authority two lines
  later and leaving that one out would be an inconsistency inside a
  single function rather than a clean deferral.

#### What the slice actually landed, row by row

Recorded because the split table above states the *intent*, and a
reader auditing this later needs the *outcome* — including the two
places where writing the witnesses changed the design.

**Landed here:** G0 (wire shapes, positions, field order, accumulated
pins), G1–G4a (the authoritative key, document and terminal halves),
G5a plus the latch substrate, G6a–b/G7a–b/G8a–e (both directions, all
four wrong-family quadrants, forged identity), G9a/G9b/G9c, G10 and
G10a–c, G11a, G13a–b, G14a–b, G15.

**Still the replay lane's**, unchanged: G4b, G5b–e, G5g, G5i–p, G6c,
G7c, G11b, G12a–b.

Two corrections the mutation pass forced, both of which had passed
their first review as written:

- **G10a proved nothing about zero.** Asserted with a generation
  already held, zero is also *lower* than it, so the nondecreasing
  clause did the refusing and deleting the zero check left the row
  green. Zero is only isolable before any authority exists — which is
  also the case the framing names, a sender that never initialised the
  field. Split into its own row with that setup.
- **G11a's exhaustion latch had no proven job.** The overflow path
  already returns before storing the ceiling snapshot, so the next read
  re-takes the changed arm and the band stays down without any latch.
  Measured, the two are alternatives: only removing both resurrects the
  band. The latch is kept, and now earns its place through `peek`,
  which honours it so the peek and the authoritative read agree that an
  exhausted session has no key rather than reporting the ceiling.

One witness-shape note, since it recurs: the two G10b rows call
`panel_mapping_is_current` directly. A wheel has **no
dispatcher-visible effect on this base** — a document panel focuses on
`Down` only, and no panel pointer coordinate is consumed anywhere — so
a row asserting focus for a wheel would be green whatever the gate did.
Each row carries a press leg alongside, which does have an effect, to
show the predicate is wired into the production arm rather than merely
correct in isolation.

### Coherence impact (`COHERENCE.md` §20)

- **Journey steps touched: 5** (document-panel editing and selection)
  and **8** (terminal-panel interaction). **Neither changes grade** —
  this hardens steps that already work rather than opening one.
  Naming them matters even so: a reader auditing §20 by grade movement
  alone would conclude this slice touches no journey at all.
- **Interaction islands: none added.** It **hardens an existing panel
  island** — the same gestures, refused when their mapping is stale.
- **Config registry: no entry.** Nothing here is tunable; a generation
  is an identity, not a threshold.
- **Background work: none started**, and no existing work changes
  attribution. The key advances synchronously with the mutations that
  move it.

Recorded because §20 asks for it per slice, and because "hardens an
existing island" is the kind of impact that gets omitted precisely
*because* the census does not move.

### Consequence for the GUI arc

This slice takes **v25**, so GUI arc **1e's `OpenTarget` moves to
v26** — corrected in `docs/gui-stage1-input-framing.md` **by this
slice**, because a canonical document that says v25 is false the moment
this lands. `ADVERTISED_PROTOCOL_VERSION` stays pinned at **20**.

## 6. Deferred (named)

Left / right / top side windows; multiple slots per side; **rehoming a leaf
across the tree**; the entire **`no_other_window` parameter and destination-only
traversal semantics**; manual panel hide/show and a future
`window.toggle-panel`; user-facing `display-buffer-alist`-style rules; **GPU
document splits (Arc 8)**; panel
persistence (blocked on settings persistence); `OSC 22` pointer shape in the
TUI; per-panel statusline segments on the wire; proportional-font panels in the
GPU; `window-configuration` registers; atomic windows; panel-local keymaps
beyond buffer and mode scopes; horizontal (`C-x {`/`}`) resize.

## 7. Interaction with other work

- **Folding Stage 2 has landed** (#149, runtime base `6ed4fe9`) — the blocking
  dependency, now cleared and re-verified against `ddaa80d` in §0.6 (nothing in
  flight, suite green, every borrowed anchor reproducing). Canonical `main` is
  now `ddaa80d`; any eventual branch starts from current canonical main.
  Folding's
  `FrontendView` policy-bit pattern is Q#BP13's model, its `Viewport<'a>` is what
  Q#BP8 inherits, and Q#BP17 owns the one invariant this arc invalidates.
  **Folding Stage 3 (GPU)** and this arc's Stage 2 both touch the semantic
  projection; whichever is framed second re-scouts the other's landed state.
- **GPU initial target #148 has landed** at runtime commit `0dd16a5` and owns
  protocol v20; #152 then refreshed only the durable handoff/active-work
  documentation at canonical `main` `ddaa80d`.
  Its attach transaction, `build_fresh_frontend_view`, private target loader,
  semantic snapshot publication filter, and previous-final wire variant were
  all re-scouted in §0.5. Q#BP9 now starts from v20; Q#BP11b shares the landed
  load seam without routing bootstrap through Lua; Q#BP14 covers both the
  publication predicate and no-target buffer inheritance. There is no
  remaining branch-order dependency on #148.
- **DAP** stays parked until this arc's **Stage 1** lands, then re-baselines its
  §0 touch census. Its Stage 2 panels become `display` + `display_file` calls.

## 8. Prior art in pmacs

Folding Stage 2 (`docs/folding-stage2-framing.md`, `src/fold_view.rs`) for the
per-`FrontendView` policy bit, the non-`Default` discipline, and per-window map
derivation; Vterm Stage 2 for the controller model, the `C-c` escape, and
per-view projection; Vterm Stage 3 for the whole-grid frame message, `validate`,
payload-complete suppression, stale-declaration rejection (extended here from
terminal-unique `buffer_id` to a panel presentation epoch), and the
`--headless-probe` seam; `listview.lua` for what a panel needs and currently
fakes; `src/desktop.rs:444-452` for activate-then-fire-per-leaf; M11.6's
`DispatchIdle` for the input gate.
