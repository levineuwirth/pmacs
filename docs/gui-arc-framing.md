# The GUI arc — framing

**Status: revision 3 — APPROVED 2026-08-10.** Approved on its design;
the seven accuracy and process edits requested with the approval are
folded in, and no further review round is required before the Stage 0
branch.

**This document is itself the framing for Stage 0**, which is docs-only.
Revision 3's opening previously said that *every* stage gets its own
framing while §10 proceeded straight from this document into Stage 0 —
the two could not both be true. The rule, stated exactly:

- **Stage 0 is framed by this document.** No separate framing round; it
  ships documentation only, and its scope is enumerated in §5.
- **Stages 1 through 10 each require their own framing**, approved
  before that stage's branch, as the arc-level contract-ownership rule
  demands. This document owns ordering, gates and the arc-level bar —
  never a stage's acceptance criteria.

**Revision 3 answers the second review round (two blocking findings,
seven required corrections, and nine rulings that close Q#GA4–Q#GA12).**

- **The closure comparison is falsifiable now** (blocking №1). Revision
  2's ordinal ordered grades but never said how to *assign* one, and
  "normalize to the head grade" silently mis-graded compound rows: a
  step reading "Works but undiscoverable" took `Works` from its first
  word while the failing half became a non-blocking annotation — even
  though **discoverability is the substance of steps 4, 7 and 11**, not
  a qualifier on them. §3.3 now defines observable criteria for each
  grade, grades a compound step by its **weakest required subclaim**,
  and separates **local TUI / attached TUI / GPU** into three columns.
  The comparison is GPU against the **local TUI** — the canonical
  `pmacs .` journey — with the attached TUI retained as evidence,
  because it is what distinguishes a semantic-wire gap from a
  frontend-local one.
- **Half B's dependency graph was inverted, and is re-ordered**
  (blocking №2). Viewport facts cannot be designed before the
  multi-window model decides whether semantic windows are daemon
  projections or frontend-local objects, because that decision
  determines **the identity a viewport fact is *about***. And a
  framing-only stage cannot hand the sidebar geometry it consumes. Half
  B is now 6 model framing → 7 viewport/window-identity substrate
  (with smooth scroll and the scrollbar) → 8 splits **plus implemented**
  non-bottom side geometry → 9 sidebar riding Stage 8 → 10 tabs.
- **"The GPU consumes daemon `view_top`" was backwards.** The backlog
  says the opposite in as many words — "the GPU **never** consumes
  daemon `view_top`" (`docs/side-quest-backlog.md:147`). Corrected.
- **Stage 4b owns save *and* restore as a pair.** Q#DS9 scopes both to
  local mode and makes **both** no-ops under a daemon, enforced in Rust
  (`desktop-save-framing.md:222`). Revision 2's Q#GA10 claimed only the
  restore *trigger* remained open; snapshot ownership, save timing and
  multi-frontend arbitration remain open too.
- **The Bell audit was wrong.** The daemon already emits
  `InstanceSignal::Bell` (`src/daemon.rs:1373`), at the audit anchor
  `4bc55e8` as well; `src/frontend.rs:349` consumes it and `pmacs-gpu`
  does not. Bell is **consumer-only** work for Stage 1c, with no
  producer question to answer.
- **Stage 0 no longer contradicts the portability rule.** Revision 2
  put the absorption PR *before* committing this framing, which leaves
  the approved framing living only in one worktree. The approved
  framing and the Stage 0 lane are now the branch's **first commit**;
  if synchronization stays a separate PR, that PR carries the framing
  first.
- **No orphan scorecard row.** A GUI-product grade needs criteria and
  ground truth under `COHERENCE.md` §16 — a distinct **product**
  subgrade beside the architectural one — before the scorecard can
  point at it.
- **§2.5 stops overclaiming.** It said the arc sequences the whole GPU
  backlog while items had neither stage nor deferral. Every item is now
  mapped or explicitly left standing, in a table.
- **Q#GA8's temporary island is withdrawn** — ruled against, so this
  arc adds **no** off-path hardcode at all (§7).
- **Reconnect attribution covers the silent cases** (§7). EOF and crash
  may deliver no `Goodbye` at all; "the daemon's stated reason" alone
  would have been unsatisfiable exactly when it matters.

**One correction this round found on its own**, not raised in review:
§3's condition 1 listed the Q#GA3 goals as "Stages 5, 6 and 10" while
§5 marked **four** stages as Q#GA3 goals — the sidebar was missing from
the closure sentence. Fixed, and re-checked against the renumbering.

**Origin.** A daily-driver report, 2026-08-09: *the TUI is a suitable
daily driver; the GUI feels behind similar editors, enough that the
reporter would default to a different editor before using pmacs' GUI.*
This is the same shape that opened the QoL arc (one daily-driver
report → an arc that closed when the report's complaints were answered
on both frontends), at a larger scale — so it gets a standard and an
arc-level frame before any lane, not a framing per gap.

**Ground truth below was established 2026-08-09** by a three-lane audit
(GPU frontend inventory, TUI/grid inventory, documentation sweep) at
`main` @ `4bc55e8`, with the §2.2 producer matrix re-verified against
`src/semantic_render.rs` at the same anchor. Per `COHERENCE.md`'s
citation convention: symbols first, `file:line` second; line numbers
drift, symbols are authoritative.

Three arc-shaping questions were put to the user and ruled on
2026-08-09; they are recorded as resolved, not open:

- **Q#GA1 — RESOLVED: one arc, two halves.** Half A is maturity
  (input, parity, chrome, robustness, hover); Half B is structure
  (viewport facts, splits/multi-window, side surfaces, and the
  presentation stages that depend on them). The arc closes only when
  **both** halves land (§3).
- **Q#GA2 — RESOLVED: the closure bar is journey parity plus an empty
  blocker list**, now stated as a conjunction over the whole stage map
  (§3).
- **Q#GA3 — RESOLVED: all four GUI-native affordances are goals, none
  is a non-goal.** Hover/signature popups, smooth scroll + scrollbar,
  and a project/files sidebar are in-scope goals. **Tabs/tabline is a
  declared goal at deliberately low priority, sequenced last** — the
  user's ruling verbatim: it is not a non-goal and "would be nice down
  the line," with care required. What the care means is Q#GA12 plus
  the anti-patterns pinned at Stage 10.

Coherence sections this framing serves: §2 (the journey, which Stage 0
makes frontend-graded), §3 (the recommended default surface for the
graphical frontend), §6 (interaction islands — see §7's accounting),
**§7 (first-class workspaces — two stages are gated on it, §5.1)**,
§14 (workbench primitives — the sidebar is tree-primitive adoption),
§16 (the semantic-frontend grade this arc completes the product half
of), §20 (priority order — placement is Q#GA5).

---

## 1. Why an arc: the diagnosis

**The GPU frontend is a rendering showcase over a single buffer, not a
workbench.** It is genuinely ahead of the TUI on rendering richness —
a real minibuffer candidate dropdown where the grid has only an inline
`[candidate]` suffix, a minimap, inline math, pixel-precise squiggles,
correct grapheme shaping where the grid drops combining marks in body
text — and behind on the three things that make an editor a daily
driver. Those three are the arc's spine:

1. **The one-window ceiling.** `State` holds exactly one `buffer` and
   one `current_buffer_id`; the daemon's entire per-frontend split
   layout (`Layout::compute`, `core.views`) is invisible to a semantic
   session. The bottom panel band (`PanelBand`) is a hand-built special
   case of "a second region." Everything spatial queues behind the
   general version: splits, side windows beyond `Side::Bottom`, the
   project/files surface `COHERENCE.md` §3 names, per-window status
   bands. The July roadmap called this "the largest unscoped design
   problem" and it still is.
2. **The GUI cannot be driven like a GUI.** `keymap_stack::Scope` has
   no frontend identity and `FrontendEvent` has no command-invocation
   variant, so a GPU-only binding cannot exist (the reason #220 shipped
   zoom as unbound commands — handoff §6's capability-aware keymap
   item). Beneath that, `translate_key` and the winit handler consume
   a narrow slice of desktop input; the rest lands in `_ => {}` (§2.3).
3. **The monolith has no test seam for input.** `pmacs-gpu/src/main.rs`
   is ~11.7k production lines in one file; the
   `gpu-terminal-input` framing already recorded that GPU key routing
   is untestable because `App::window_event`'s logic is inline "with no
   extractable seam," and called the refactor "a real refactor
   [belonging] to its own lane." An input arc that skips the seam ships
   blind.

**Why the standard never caught this drifting.** `COHERENCE.md` §16
grades the *semantic protocol* — degradation practiced, capability
negotiation, versioning — and that grade (Strong) is earned. But **no
scorecard row measures the GUI as a product**, and **the golden journey
has only ever been graded on the TUI**. "Semantic frontend: Strong" and
"I'd use a different editor before the GUI" stayed simultaneously true
because the standard only measured the first. This is §1.1's
substrate-without-surface at frontend scale: the substrate is the
protocol and the daemon's facts; the missing surface is the GPU
consumers of them (§2.2 below shows which halves exist). Stage 0
closes the measurement gap so it cannot reopen.

---

## 2. Ground truth (audited 2026-08-09)

### 2.1 What the GPU has

Code area with syntax/LSP styling, gutter (Off/Absolute/Relative/
Hybrid) with diagnostic signs, minimap with click/scrub, one bottom
status band (statusline segments validated and themed), bottom panel
band with divider drag, minibuffer with a 10-row candidate dropdown,
in-buffer completion popup with kind glyphs, right-click context menu,
isearch band UI, diagnostic squiggles (dedicated pipeline), selection
and search washes, peer presence (cursors + selections), inline math,
terminal mode, optimistic CRDT editing with unconfirmed-edit
journaling. Mouse: click, drag, double/triple-click, wheel (line-
quantized), edge auto-scroll, panel and divider gestures, minimap
scrub. Clipboard both directions via `arboard`.

### 2.2 Wire-capability matrix: produced vs consumed

The GPU's live-loop catch-all is one `_ => None` (`main.rs:5211`);
`FoldState` and `BlockAdornments` appear in `pmacs-gpu` **only** in a
debug-name helper. But "the GPU ignores it" means different work
depending on whether a producer exists — revision 1 conflated these,
and three items hid producer scope. Producer column verified against
`src/semantic_render.rs` at `4bc55e8`:

| Capability | Produced? | GPU consumes? | Work required |
|---|---|---|---|
| `FoldState` | **Yes** (`semantic_render.rs:1881`) | No | **Consumer-only** — Stage 3a |
| `InlineAdornments` | Only `(AtOffset, Text)`, from the inlay-hint store (`semantic_render.rs:1849`) | Exactly that same subset | **No live gap today.** Other placements/content are producer *and* consumer work; not claimed by this arc |
| `BlockAdornments` | **No** — a producer test asserts none is emitted (`semantic_render.rs:4543`) | No | Producer + consumer; Stage 3a's framing decides whether GPU folding renders from `FoldState` alone or needs placeholders |
| `ResourceOffer` / `AdornmentContent::Resource` | **No** | No | Producer + consumer; stays deferred (§6) |
| `InstanceSignal::Title` | **No producer found** in `src/` | No | **Not needed for a dynamic window title** — `StatusFacts` already carries the buffer name, so Stage 1c titles the window frontend-locally; a `Title` producer (e.g. terminal-set titles) is separate, unclaimed work |
| `InstanceSignal::Bell` | **Yes** (`daemon.rs:1373`, present at `4bc55e8`) | No — `frontend.rs:349` is the grid consumer; `pmacs-gpu` has no arm | **Consumer-only** — Stage 1c. *Revision 2 recorded "no producer found" and gave Stage 1c a producer question to answer; the producer was there the whole time, and the audit had searched the semantic-render path rather than the daemon's signal path.* |
| `Goodbye(reason)` post-handshake | Yes | Bootstrap only; live-loop reason discarded | **Consumer-only** — Stage 1c |
| `CompletionPopup.prefix_len`/`total` | Yes (on the wire) | Stored under `#[allow(dead_code)]`, unrendered | **Consumer-only** — minibuffer/completion refinement |

### 2.3 Input gaps (verified in-session, not carried from docs)

- **Escape quits the entire application** when no intercept/popup is
  active (`main.rs:2769`; the comment says "otherwise it stays the
  local quit"). No data is lost — the daemon holds state — but it
  reads as a crash to anyone with Escape reflexes.
- `translate_key` produces **no `ProtocolKey::F(u8)`** — F1–F12 are
  unbindable in the GUI though the protocol carries them. `BackTab`,
  `Menu` also unmapped; `Key::Dead(_) → None` (dead keys silently
  dropped); multi-codepoint `Key::Character` truncated to its first
  char.
- **No `WindowEvent::Ime`, no `set_ime_allowed`** — CJK/compose input
  is unusable. Undocumented anywhere before this audit.
- **No `ScaleFactorChanged` arm; `scale: 1.0` hardcoded** — HiDPI is
  wrong (also recorded as a pre-existing gap in
  `gpu-set-font-framing.md`).
- Sub-line wheel deltas are rounded then discarded with **no residual
  accumulator** — precise-pixel trackpad scroll under ~½ line height
  does nothing. Horizontal wheel x is discarded although
  `code_scroll_left` exists; `MouseKind::ScrollLeft/Right` are never
  emitted. Ctrl+wheel is ignored.
- No middle-click paste, no `DroppedFile`, no I-beam cursor over text,
  minibuffer dropdown not clickable (audit F-007).
- **`FrontendEvent::FocusGained`/`FocusLost`/`Detach` are never
  sent** — no `Focused` arm; `CloseRequested` exits without `Detach`,
  so the daemon learns of departure by socket EOF.

### 2.4 Structure, robustness, chrome

- One document window forever (§1 cause 1). Daemon splits invisible.
- No auto-reconnect; the reconnect banner is an attach-TUI-only seam
  (`Frontend::draw_status_overlay`). F-008 in
  `gpu-attach-robustness-framing.md`.
- **Session restore is structurally never**: desktop save/restore
  early-returns in Rust under a daemon (Q#DS9), and the GPU is always
  semantic — so a GPU session can never restore. Journey step 12's
  thin end, at its thinnest on this frontend.
- Chrome theming is half-applied: `MENU_BG`, completion popup
  background, `MINIMAP_BG`, `CARET_COLOR`, `WINDOW_BG_RGBA`, and the
  peer-presence palette are hardcoded constants; custom themes
  fracture in the GUI. (The TUI's completion popup and menu are also
  unthemed `Indexed` constants — the pair should be fixed together,
  per no-privileged-frontend.)
- Cursor: fixed 2px bar, fixed color, no blink, no styles.
- Word wrap regressed at #221: the GPU had cosmic-text
  `WordOrGlyph` since it existed and now gets `Wrap::Glyph`; the
  long-lines framing already names `ui.line-wrap = "word"` as the
  clean additive third value.
- LSP styling diverges by model: the grid **merges** LSP tokens over
  tree-sitter (`LspStyleView`/`merge_styles`); the semantic wire is
  single-authority — GUI highlighting is strictly poorer in
  mixed-authority languages.
- `HoverView` and `SignatureView` exist in the core, **built and never
  attached anywhere** (§1.1 dark matter) — relevant to Stage 5.

### 2.5 Already-recorded backlog: mapped or explicitly left standing

Revision 2 said this arc "sequences" `docs/side-quest-backlog.md`
§"GPU frontend mechanics (non-theme)" without restating it, which
claimed coverage it did not have — several items had no stage *and* no
deferral, and a reader checking whether the arc covered their complaint
had nothing to check. **Every item in that section is below. An item is
either mapped to a stage or explicitly left in the standing backlog;
there is no third state.** Handoff §6's capability-aware keymap item
and the folding framings' Stage 3 obligations are absorbed **by
reference** — each keeps its own framing.

| Backlog item | Disposition |
|---|---|
| Command/minibuffer chord forwarding; Meta/Super chords | Stage 1a |
| Rebindable local `Ctrl-V`/`Escape` | Escape half → Stage 1a; `Ctrl-V` half → Stage 2 (it is a keymap-vocabulary question, not an input-plumbing one) |
| Middle-click paste | Stage 1b |
| Right-click context menu | **Already shipped** (§2.1) — the backlog item is stale and Stage 0 retires the line |
| Frontend-local provisional selection | **Standing backlog** — a selection-ownership question, not a GUI-maturity gap |
| Minibuffer `i/total` hint (= `CompletionPopup.prefix_len`/`total`) | Stage 3d |
| Clickable minibuffer dropdown rows (audit F-007) | Stage 3d |
| Multibyte-exact band caret; nav highlight-wrap bug | Stage 3d |
| Telescope-style preview pane; candidate kind/doc annotations; unify TUI inline vs GPU dropdown | **Standing backlog** — the unification is a cross-frontend convergence design, and the other two ride it |
| Scrollbar scroll; pixel-smooth sub-line scroll | Stage 7 (the *discard* bug is Stage 1b; the smooth **model** needs Stage 7's facts) |
| Horizontal scroll / soft-wrap | wheel → Stage 1b; wrap → Stage 3b |
| Auto-reconnect + "reconnecting…" banner | Stage 4a |
| `AttachRequest.initial_size` cell-grid assumption | Stage 1c, with DPI — the assumption is only visible once scale is real |
| Capability renegotiation (relaunch daemon `--features crdt`) | **Standing backlog** — daemon lifecycle, not frontend maturity |
| Peer caret glyph + name label; own-vs-peer cursor merge; `SelectionSnapshot` vs `Decorations::Selection`; background-kind decorations painted | **Standing backlog** — collaboration/decoration rendering; no journey step and no §3.1 blocker depends on them |
| Inline adornment placements beyond `AtOffset` | **Standing backlog** — producer *and* consumer scope (§2.2) |
| Glyphon full-buffer `prepare` ceiling; `Renderer` sub-struct extraction | **Standing backlog** — perf and refactor; the `main.rs` split's first slice is Stage 1-pre and claims no more |
| Golden-PNG comparison harness | **Deferred by §8**, with its condition stated there |

Three gaps from §2.3/§2.4 are not in that backlog section and are
mapped here so they cannot fall through: **`DroppedFile`** → Stage 1b;
**cursor blink and styles** → Stage 3c (§7 registers the knob, and
Stage 3c is what ships it — revision 2 named the configuration with no
stage behind it); **the LSP merge-vs-single-authority divergence**
(§2.4) → **standing backlog**, explicitly, because it is a semantic-wire
authority question whose fix belongs to whoever owns multi-server token
policy, not to a GUI maturity stage.

---

## 3. Closure criterion (Q#GA2)

**The arc closes when all three of the following hold; none alone is
sufficient:**

1. **Every stage in §5 has landed** — or has been explicitly re-ruled
   by the user at the time, with the ruling and its reason recorded in
   §3.2. There is no stage outside the closure contract: revision 1's
   "Tail" is dissolved, and the Q#GA3 goals are Stages **5, 7, 9 and
   10** (hover/signature; smooth scroll + scrollbar; the project/files
   sidebar; tabs). *Revision 2's sentence said "Stages 5, 6 and 10"
   while §5 marked four stages as Q#GA3 goals — the sidebar was absent
   from the closure sentence that is supposed to enumerate them.*
2. **The per-frontend journey table shows GPU ≥ local TUI at every
   step**, under the grading rules in §3.3.
3. **The daily-driver blocker list (§3.1) is empty.**

Divergences that survive must be declared in §3.2, not accidental.

### 3.1 Blocker list (seed — membership is Q#GA11)

1. Escape quits the application (§2.3).
2. IME absent — CJK/compose input unusable.
3. `translate_key` holes: F-keys, BackTab, dead keys, multi-codepoint
   text.
4. Sub-line scroll discard (trackpad feels broken); no horizontal
   wheel.
5. No DPI/scale handling.
6. Folding silently dead on the GPU.
7. No session restore on the GPU, ever (Q#DS9).
8. No reconnect after daemon restart.
9. One-window ceiling (graded via the journey table's affected steps
   rather than as a single line — listed here so the list cannot be
   emptied while the ceiling stands).

### 3.2 Declared-divergence and re-ruling register

Divergences that survive the arc, and any stage the user re-rules out
of the closure contract, are recorded here with a reason (the model is
#221's honest-divergence ruling on word wrap). Seed: none — entries
are added by stage framings or user rulings as they happen.

### 3.3 How a (step, frontend) cell is graded

`COHERENCE.md` §2's existing verdicts are compound strings ("Works but
undiscoverable", "Partial (good once reached)") and do not order.
Revision 2 replaced them with an ordered set but never said how a cell
*acquires* a grade, and its normalization rule — take the head grade,
demote the rest to annotation — is unsound in the exact case it was
written for: **"Works but undiscoverable" would grade `Works`**, and the
undiscoverability would become prose that cannot block closure. That
inverts the standard, because discoverability is not a qualifier on
journey steps 4, 7 and 11 — it *is* their substance.

**Three columns, not two.** Stage 0 grades each step for **local TUI**
(`pmacs .`), **attached TUI** (`pmacs --attach`) and **GPU**
separately.

**The comparison is GPU against the local TUI.** That is the canonical
`pmacs .` journey and the frontend the daily-driver report calls
suitable, so it is the bar the GUI must meet.

**The attached TUI column is retained as evidence, not as the bar — and
what it is evidence *of* is narrower than revision 3 first claimed.**
The attached TUI is a **grid** frontend: it handshakes
`semantic_render: false`, and the field's own comment says it "never
consumes the SemanticFrame family" (`src/attach.rs`, the
`FrontendCapabilities` constructor). So a shared GPU/attached-TUI gap
cannot mean "the semantic wire is at fault" — the attached TUI is not
on that wire. What the three columns actually separate is:

- **local vs daemon-attached** behaviour (local TUI against attached
  TUI), which isolates everything the daemon boundary introduces; and
- **attached-grid vs semantic/GPU** behaviour (attached TUI against
  GPU), which isolates what is specific to semantic rendering.

**Neither comparison alone establishes producer-versus-consumer
ownership.** Reading the columns narrows where to look; **source
tracing is what assigns the gap**, exactly as §2.2's matrix had to be
verified against `src/semantic_render.rs` rather than inferred from
behaviour. A single TUI column would still have merged two distinct
diagnoses — that argument survives — but it was never going to hand out
owners for free.

**The grades, by observable criteria.** Each is a test someone else can
run and get the same answer:

> **Broken < Missing < Partial < Works**

- **Works** — every required subclaim holds with no qualifier, by a
  route the step's own discoverability subclaim admits.
- **Partial** — every required subclaim is *satisfiable*, but at least
  one is degraded: reachable only by a route the step does not admit
  (e.g. only by typing an unlisted command), or holding only under a
  stated precondition.
- **Missing** — a required subclaim has **no surface at all**: the
  action is unavailable and attempting it produces neither effect nor
  error.
- **Broken** — a surface exists and using it produces a **wrong
  result**, data loss, or an application-level failure. Ranked *below*
  `Missing` deliberately: an absent feature is honest, while a present
  one that misleads costs the user work and trust.

**A compound step is graded by its weakest required subclaim.** Each
step in the table declares its required subclaims explicitly; the cell's
grade is the **minimum** over them, never the first word of a prose
verdict.

**The worked examples below are HISTORICAL, and `COHERENCE.md` §2b is
authoritative for every current grade.** They are kept because they are
what motivated this rule, and they are marked because a framing and the
standard it serves must not hand a reader two different grades for one
step. They quote the **pre-Stage-0** verdict strings, not today's table:

- **Step 7** — the row then read "Symbol: **works but
  undiscoverable**". Subclaims *reachable* / *discoverable*: `Works` and
  `Missing` ⇒ **`Missing`**. **Stage 0 graded it `Works`** on evidence
  this example did not have: advertisement is transitive through the
  help graph, and the advertised `M-x help` route reaches
  `help.list-keybindings`, which names every registered binding.
- **Step 11** — the row then read "**Works but undiscoverable**". Same
  shape, and here the outcome **stands**: Stage 0 also graded it
  `Missing`, because **no binding opens `editor.list-workers`** for a
  listing to name. The view does carry a buffer-local `C-c C-k`
  (`workers.cancel-at-point`), which is reachable only once you are
  already inside it.

The rule these examples exist to establish is untouched by either
outcome: **the head-grade rule would have graded both `Works`**, and
that is what makes it unsound. Step 7 moving on better evidence is the
system working; step 11 not moving is the defect surviving contact with
it.

Under revision 2's head-grade rule **both would have graded `Works`**,
and the undiscoverability that is the entire finding would have become
annotation text with no effect on closure. Two of the journey's twelve
steps is not an edge case.

**Annotations carry only what is not a required subclaim.** They cannot
absorb a failing subclaim; if something is load-bearing enough to
mention as a defect, it is load-bearing enough to be a subclaim and be
graded. Where the two frontends differ only in an annotation, the
difference is recorded and does not block closure — that remains true,
and is now narrow rather than a loophole.

**Stage 0 must publish the subclaim list per step**, not just the
grades. A grade whose subclaims are unstated is not falsifiable, which
is the whole objection this section answers.

---

## 4. Arc structure (Q#GA1)

**One arc, two halves; the name is "the GUI arc," deliberately a name
and not a number.** The roadmap's "Arc 8 — GPU structural parity"
label already collides (the Lean 4 framing also claims Arc 8; the
collision is recorded in `docs/dired-framing.md`). This arc subsumes
roadmap-Arc-8's scope as its Half B; the numeric label retires.

- **Half A — maturity** (Stages 0–5): the GPU behaves like a competent
  desktop application over its existing one-window model. No
  structural redesign; heavy protocol work only where §2.2 shows a
  producer already exists, or the stage's framing names the producer
  scope it adds.
- **Half B — structure** (Stages 6–10): the multi-window model, then
  the viewport/window-identity substrate it defines, then splits and
  side geometry, then the presentation stages that depend on
  multi-window state (sidebar, tabs). The order is load-bearing — see
  the note opening Half B in §5.

Half A ships visible value while Half B's model framing matures; the
arc does not close at the end of Half A (§3's condition 1 spans both
halves), so the early wins cannot quietly become the whole arc.

---

## 5. Stage map

**Stages 1–10 each get their own framing before their branch; Stage 0
is framed by this document** (see the status block). This document owns
the ordering rationale and the arc-level bar, never stage-level
acceptance criteria (the contract-ownership rule). **Every PR in this
arc opens with its `docs/active-work.md` lane written at the branch's
first commit** — the standing correction from #171/#215, missed again
at #224 and #225, and adopted here as an arc rule rather than re-hoped.

### 5.1 The P2 gate (blocking №2's resolution)

Two stages are **workspace-owned** and carry a hard gate: they may not
start before the P2 workspace arc has landed at least the workspace
object they consume.

- **Stage 4b (session save *and* restore)**: "what a session *is*" is
  the workspace question — Q#DS9 failed precisely because a daemon
  layout had "nothing principled to attach to" (`COHERENCE.md` §7). A
  frontend-keyed convention invented here would be a new ownership
  story P2 then has to unwind; revision 1 called that v1 "plausible",
  revision 2 withdrew the recommendation, and Q#GA10 is now **ruled**
  (both surfaces preserved, the save path owned here too). The gate is
  what makes the ruling implementable: snapshot ownership and
  multi-frontend arbitration have no answer without P2's object.
- **Stage 9 (project/files sidebar)**: a sidebar must show *something
  rooted*, and §7 warns P2 must start "before a fifth subsystem grows
  its own root convention — four have already diverged." The sidebar
  is the fifth if it picks its own root.

**Reaching Stage 4b is a P2 START GATE, not merely a pause** (Q#GA5
ruling, revision 3). Revision 2 let the gated stage stall while
everything else proceeded. **It could not have let the arc formally
close around P2** — the gated stages are inside the closure contract
(§3, condition 1), so closure still blocked on them. What it *would*
have allowed is every **non-gated** stage finishing before P2 began,
leaving P2 as a **terminal closure blocker**: an arc sitting at 100%
of the work it could do, waiting on an arc nobody had started. The rule
is stronger:

1. When the arc reaches Stage 4b, **P2 starts**. That is the trigger.
2. **No later GUI stage starts** — gated or not — until P2 has **an
   approved framing and an opened lane**. Those two are the observable
   condition; P2 need not have *landed* anything.
3. Once P2 has both, **non-gated GUI work may interleave** freely while
   the workspace object lands. Only the two gated stages (4b, 9) wait
   on the object itself.

The gate is on *starting P2*, not on P2's completion, so the arc is
never blocked on work nobody has begun — and it cannot outrun the
model it depends on. A gated stage never proceeds on a local
convention; that was already true and stays true. The arc's
`docs/active-work.md` lane records the gate state whenever it is in
force.

### Half A — maturity

**Stage 0 — the standard sees the GUI (docs only).**

*The framing goes first, and that reverses revision 2's ordering.*
Revision 2 put the absorption PR ahead of committing this document,
which contradicts the portability rule it cites elsewhere: an approved
framing that lives only in one worktree is one `git clean` from gone
and does not travel to another machine. **The approved framing and this
arc's `docs/active-work.md` lane are the Stage 0 branch's first
commit.** If synchronization stays a separate PR, **that PR carries the
framing first** — absorption may precede the rest of Stage 0, never the
framing.

*The absorption pass*, whose scope is now enumerated rather than
described (it grew on 2026-08-10 when six lanes merged in one session):

- **Five stale lanes in `docs/active-work.md`** — #224 and #225 carried
  as OPEN, #228 as OPEN and MERGE-BLOCKED, LSP LaTeX as "no PR yet"
  (merged as #230), destination capture as "PR #231 OPEN" (merged as
  `0e4c58d`). Durable facts into `docs/agent-handoff.md` first, then
  remove the **PR-specific** block.

  **#228 is the exception, and it must not be retired "per Rule 4" as
  though Discovery were finished.** Rule 4 removes a lane when its
  **arc** is done; Discovery's is not. Two entries exist — the
  PR-specific block and the standing **Discovery lane (P4)**, which
  already says "Rewritten, not removed" for exactly this reason. Stage
  0 removes the first after re-homing its facts and **rewrites and
  coalesces the second** to *"Stage 2 merged; later discovery work
  remains"*. Still open there: **predicate evaluation**, **command
  metadata** (title/category/aliases/flags), **help unification**, and
  **the prefix decision**. Deleting that lane would drop four named
  pieces of open work on the strength of one merged stage.
- **The authority/recovery anchor**, which points at `9a26ac8` while
  `main` has moved well past the audit anchor `4bc55e8`.
- **`COHERENCE.md` §0 row 16 / §16's `v6..=v21` → `v6..=v23`.** The
  ceiling moved **twice**: #221 took it to v22 for `LineWrapFacts`, and
  **#228 took it to v23** for `MinibufferPromptRows`
  (`PROTOCOL_VERSION = 23`, `SUPPORTED_PROTOCOL_VERSIONS = 6..=23`,
  `pmacs-protocol/src/message.rs:1843`). *Revisions 2 and 3 both said
  v22, having read the range at the audit anchor and not re-read it
  after Discovery landed.* The same row's "production attach remains
  v20" is **still correct** — `ADVERTISED_PROTOCOL_VERSION` is 20 — and
  must not be swept along with the range.
- **The U4 correction and the U9 residue** in
  `docs/ci-red-signatures.md`. U4's flavour field is not a matching key
  (the same selector and fragments red on both macOS flavours) and one
  of its four "occurrences" was a deliberate bite. **U9's text must be
  fixed, not merely carried**: it says a same-tree green shows the
  failure "is not the tree," which contradicts this file's own rerun
  rule — a tree can raise an intermittent failure *rate* without making
  it deterministic. The replacement claim is **"not deterministic on
  this tree; causation and rate effect unresolved."**
- **The stale backlog line** for the right-click context menu, which
  ships (§2.5).
- **Journey step 11's verdict**, which #232 falsified on 2026-08-10.
  `COHERENCE.md` §2 still reads "**Works but undiscoverable** … no
  keybinding, no statusline spinner/progress indicator anywhere (§9)".
  #232 shipped exactly that indicator — a statusline provider showing
  an in-flight count and the oldest job's purpose, absent when idle.
  The row needs regrading under §3.3, and §9's own grade needs
  re-reading: the mechanism-without-identity finding is partly
  answered. **Found while grading the journey for this revision, not
  in review** — which is the argument for §3.3's three columns, since
  a stale row survives precisely as long as nobody has to assign it a
  falsifiable grade.

*Then Stage 0 proper:* add the per-frontend journey verdicts to
`COHERENCE.md` §2 under §3.3's grading rules, **including the subclaim
list per step**; place the arc in §20 (Q#GA5); cross-reference from
`docs/agent-handoff.md` §6; retire the "Arc 8" numbering (Q#GA4). No
runtime code.

**No orphan scorecard row.** Revision 2 proposed adding a GUI-product
row to the scorecard with a grade attached ("Weak — renderer ahead,
workbench and input behind"). A scorecard row is a pointer to a graded
concern, and there is no graded concern for the GUI *as a product*:
`COHERENCE.md` §16's grade is architectural. So Stage 0 first
establishes **a distinct product subgrade beside the architectural one
in §16**, with its own criteria and audited ground truth, and *then*
the scorecard points at it. A row whose grade rests on nothing is the
thing §16 exists to prevent.

**Stage 1 — input foundation.**

- **1-pre: the input seam.** Extract `App::window_event`'s routing
  into testable functions — the refactor `gpu-terminal-input` already
  named as its own lane, plus the first slice of the recorded
  `main.rs` split. This is the stage's first PR because everything
  after it needs witnesses.
- **1a keyboard correctness**: Escape stops being the local quit
  (round-trips like any key; quitting becomes a command/window
  affordance — subsumes the backlog's "rebindable local Ctrl-V/Escape"
  item on the Escape half); `translate_key` completion (`F(u8)`,
  `BackTab`, `Menu`; dead keys held for 1d rather than dropped).
- **1b pointer/scroll correctness**: sub-line residual accumulator;
  horizontal wheel → `code_scroll_left`; middle-click paste; I-beam
  cursor over text; `DroppedFile`.

  **Q#GA6 — RULED: land the TUI answer in this same stage; no declared
  divergence.** The TUI half is smaller than revision 2 implied, and
  the corrected trace is this: crossterm already delivers
  `MouseEventKind::ScrollLeft`/`ScrollRight`; `src/protocol.rs` and
  `pmacs-protocol` already carry them as `MouseKind::ScrollLeft`/
  `ScrollRight`; and **attached** mouse events already round-trip
  through `mouse_from_crossterm` / `mouse_to_crossterm`
  (`src/protocol.rs:712`, `:777`). Local and attached document events
  **converge on the one document handler, whose only wheel arms are
  `ScrollUp`/`ScrollDown` at `src/editor.rs:3189`** — that single site
  is where horizontal wheel is dropped.

  *Revision 3 cited `src/editor.rs:5863-5864` and `:3407` here. Both
  are **terminal-content** paths — `:3407` matches `TerminalMouseKind`
  and drives `terminal_manager.scroll_view` — not the document window,
  so they were the wrong sites for this ruling.*

  So the TUI answer is one handler arm on an event that already
  arrives, not new plumbing, and QoL Stage 5's reason for excluding
  explicit-scroll surfaces — keeping the frontends agreeing — is
  *served* by doing both here rather than traded against.
- **1c session/window signals**: `Focused` → `FocusGained`/`FocusLost`;
  `CloseRequested` sends `Detach` before exit; post-handshake
  `Goodbye` reason surfaced (consumer-only, §2.2); **window title
  composed frontend-locally from `StatusFacts`** — no `Title` producer
  required (§2.2); **`Bell` as a plain consumer** — the producer exists
  and always did (§2.2), so there is no producer question and no option
  to drop it; `ScaleFactorChanged`/DPI, and with it the
  `AttachRequest.initial_size` cell-grid assumption, which is only
  observable once scale is real.
- **1d IME — Q#GA7 RULED: the full preedit overlay, not a commit-string
  minimum.** The scope is explicitly: commit string; **caret and
  selection range within the preedit**; **cancellation**; and
  **focus-loss cleanup** so a dropped composition cannot survive as
  stale overlay text. Preedit needs a rendering surface, which is why
  this is not folded into 1a — and why the ruling has real cost, stated
  rather than discovered later.

**Stage 2 — capability-aware keymap resolution.**

**By reference, not absorption**: handoff §6 requires this to be its
own framing round and forbids starting it as a half-lane. This arc
sequences it here because Stage 1's seam makes its GPU consumers
testable, and consumes it for default zoom bindings and every future
GPU-native chord. **This arc feeds Stage 2's framing one explicit
input question: does the capability-aware vocabulary cover pointer
gestures (wheel-with-modifier), or keys only?** Revision 1 assumed
the former; nothing yet guarantees it (major №4).

**Q#GA8 — RULED: wait for Stage 2. The temporary Ctrl+wheel zoom
island is not created.** Zoom arrives through this stage's mechanism or
not at all. Consequently this arc adds **no** off-path hardcode, and
§7's island accounting is now unconditional rather than
"at most one" — there is no removal criterion to track because there is
nothing to remove.

**Stage 3 — parity consumers.**

- **3a folding Stage 3** — consumer-only per §2.2's matrix (the
  producer exists). The obligations are already enumerated in the
  folding framings (a `FoldState` consumer, the fold-mirror clear on
  `BufferSnapshot` (R2-4), `fold_projection` flip, optimistic-edit
  unfold (R2-3), caret/hit-test fold-awareness). Whether GPU folding
  needs `BlockAdornments` placeholders — which would add producer
  scope — is that framing's question. Its ordering precondition
  (bottom-panel Stage 2's landed band) is satisfied.
- **3b word wrap** — `ui.line-wrap = "word"` as the declared third
  value. Nearly free on the GPU (cosmic-text `WordOrGlyph`).
  **Q#GA9 — RULED: implement the grid answer with UAX #14; no declared
  divergence.** The dependency is accepted rather than traded for a
  §3.2 entry, so both frontends wrap by the same rules.
- **3c chrome theming** — `ThemeFacts` adoption for menu, completion
  popup, minimap, caret, window background, peer palette; the TUI's
  unthemed popup/menu pair is fixed in the same stage or declared.
  **Cursor blink and cursor styles ship here** — §7 registers the knob
  through the config registry and this is the stage behind it (§2.5).
- **3d minibuffer/completion refinements** — `prefix_len`/`total`
  rendered (consumer-only, already on the wire per §2.2); clickable
  dropdown rows (audit F-007); multibyte-exact band caret; the nav
  highlight-wrap bug. Grouped as its own sub-stage rather than
  scattered, because §2.5 showed four backlog items landing in one
  surface.

**Stage 4 — robustness.**

- **4a auto-reconnect** + a reconnecting banner (parity with the
  attach TUI's seam; F-008). This adds a background reconnect loop and
  therefore owes §20's background-work attribution — see §7 for the
  contract its framing must satisfy (owner, lifetime, cancellation,
  failure attribution).
- **4b session save *and* restore for semantic frontends** (Q#DS9) —
  **P2-gated, §5.1.** The stage owns **both halves as a pair**, which
  revision 2 got wrong by naming only restore. Q#DS9 scopes v1 to the
  local `editor::run` path and makes `desktop_mode(true)` auto-save
  **and** auto-restore no-ops in daemon mode, enforced in Rust by a
  `DaemonMode` marker that `save_session`/`restore_session` early-return
  on. So there is no snapshot being written under a daemon today: a
  restore path alone would have nothing to read, and shipping restore
  without save would be a stage that cannot work by construction.

  **Q#GA10 — RULED: preserve both surfaces.** Automatic restore on the
  **first eligible attach** when armed and no explicit target was
  supplied, **plus** the existing explicit command — not one or the
  other. What remains open is more than revision 2 admitted when it
  said only the trigger shape was: **snapshot ownership** (who writes
  it, keyed how, once the frontend is not the owner), **save timing**
  (before-quit is a local-mode assumption; a daemon frontend can detach
  without quitting anything), and **multi-frontend arbitration** (two
  attached frontends with divergent layouts and one workspace key).
  All three are decided in this stage's framing, on P2's object.

**Stage 5 — hover/signature popups** (Q#GA3 goal). The core's
never-attached `HoverView`/`SignatureView` are the data-model
precedent; the GPU needs a popup surface and a wire decision (ride an
existing family vs a new message — its framing decides; hover data
currently flows Lua → echo/`*lsp-help*`, so there is **producer scope
here by construction**, stated rather than hidden). Independent of
Half B; sequenced after Stage 3 so the popup is themed from birth.

### Half B — structure

**Revision 3 reorders this half.** Revision 2 ran viewport facts (6)
before the multi-window model (7), which is backwards twice over. A
viewport fact is *about* something — a window — and whether a semantic
window is a **daemon projection** or a **frontend-local object** is
exactly what the model stage decides; designing the facts first would
fix an identity the model then has to honour or break. Second, revision
2's model stage was **framing-only** yet the sidebar was told to "ride
Stage 7's geometry": a framing produces no geometry, so Stage 9
consumed something no stage shipped. Non-bottom side geometry is now
**implemented** in Stage 8.

**Stage 6 — the multi-window model framing.** The arc's center of
gravity and the reason Half B exists: how a daemon layout projects to
a semantic frontend (project the per-frontend layout tree vs
frontend-local layout over multiple buffer subscriptions — the wire
today assumes one document window per semantic session, with the panel
band as the only exception). **Its output that everything downstream
needs is the window-identity decision**, because that is what a
viewport fact, a split, a side slot and a tab all refer to. Framing
only; it ships no runtime code, and nothing downstream is told to
consume geometry from it.

**Stage 7 — the viewport/window-identity substrate, and the scroll
feel that reads it** (Q#GA3 goal). Viewport facts on the wire, carrying
the identity Stage 6 settled — **the GPU never consumes daemon
`view_top` today**, which the backlog names as the blocker for recenter
and every scroll command (`docs/side-quest-backlog.md:147`; revision 2
stated this exactly backwards). **Smooth scroll and the scrollbar live
here**, not in a tail: a scrollbar needs authoritative extent and
position, and pixel-smooth scrolling changes the scroll model those
facts feed.

**Stage 8 — splits shipped, and side geometry with them**: rendering,
input routing, per-window status bands, window-command parity
(`C-x 2/3/o/0/1`), **plus implemented side-window geometry beyond
`Side::Bottom`**. The geometry is here rather than in Stage 6 because
it is code, and because Stage 9 consumes it.

**Stage 9 — the project/files sidebar** (Q#GA3 goal) —
**P2-gated, §5.1.** Tree-primitive adoption (`COHERENCE.md` §14 names
project files as a future tree consumer; §3 names the surface). Rides
**Stage 8's implemented geometry** and P2's root object.

**Stage 10 — tabs/tabline** (Q#GA3 goal — last, low priority by
ruling). **Q#GA12 — RULED: a deliberate deferral to this stage**,
decided after P2 and the multi-window model exist, because both are
what make the readings meaningful — the lineage precedents disagree
(Emacs `tab-bar-mode` tabs are **window configurations**; tab lines and
Doom's centaur-tabs are **buffers**), and a workspace-keyed third
reading only becomes available once P2 has landed. Deferring is the
ruling, not an absence of one. The constraints hold regardless and are
pinned now: tabs present **existing objects** (whichever kind), **never
a parallel registry** with unvalidated references — the menu-label
mistake is the named anti-pattern — and the surface is **optional and
off by default**.

---

## 6. Non-goals and named deferrals

- **GUI as the default frontend.** Deliberately **not** the closure
  bar (Q#GA2 chose journey parity). It remains
  `gpu-initial-target-framing.md`'s deferral, to be *decided* — not
  assumed — when the arc closes.
- **Git integration** (`COHERENCE.md` §15): editor-wide, not
  GUI-specific; not this arc.
- **Settings/preferences GUI**, **native menu bar**, **multiple OS
  windows**: out of scope; nothing below depends on them.
- **`ResourceOffer`/image rendering**: unproduced and unconsumed
  (§2.2); stays deferred unless a stage (sidebar icons, hover docs)
  pulls it in with a framing that owns both halves.
- **Proportional code fonts, ligature/feature toggles, font wire
  transfer**: `gpu-set-font-framing.md`'s deferrals stand.
- **Remote GPU paths, daemon service management**: unchanged.
- **Multi-cursor**: pre-existing v0.1 non-goal, unchanged.

---

## 7. Coherence impact (per `CLAUDE.md` / `COHERENCE.md` §20)

- **Journey steps touched**: **3, 4, 5, 6, 7, 8, 10, 12** — *on the GPU
  frontend*; Stage 0 makes the journey frontend-graded so the impact is
  measured per step rather than asserted. Revision 3 listed five; three
  were missing because the list was carried from revision 1's smaller
  stage map and never re-derived against the stages this arc actually
  ships. **Step 3** — `DroppedFile` (Stage 1b) is an open-a-file route.
  **Step 5** — keyboard correctness, IME, folding, wrapping and
  scrolling are all editing-surface work (Stages 1a, 1d, 3a, 3b, 7).
  **Step 6** — completion refinements and hover/signature popups
  (Stages 3d, 5).
- **Interaction islands**: this arc adds **no dispatch shadows** — the
  count stays at six — and, after Q#GA8's ruling, **no off-path
  hardcode either**. Revision 2 reserved one temporary island for the
  Ctrl+wheel zoom intercept under a mandatory removal criterion; the
  ruling declined it, so the census is untouched by this arc and there
  is no removal criterion to track. Zoom arrives through Stage 2's
  mechanism or not at all. A stage that believes it needs a new
  *shadow*, or a new island, returns to this document first.
- **Config registry adoption**: every user-visible knob this arc adds
  (smooth scroll, scrollbar, cursor blink, tabline toggle, IME
  behavior if any) registers through the config registry — no new raw
  Lua-table settings. The minimap's divergent tab width (4 vs the
  shared 8) stays owned by config-registry Q#CR13, referenced not
  absorbed.
- **Background-work attribution** (moderate №8): Stage 4a's reconnect
  loop is background work and owes the §20 attribution regardless of
  §9's unsolved general model. The contract its framing must satisfy:
  **owner** — the GPU frontend process, scoped to its session, never
  the daemon; **lifetime/cancellation** — bounded backoff, canceled
  on user quit and on successful re-attach, never outliving the
  window; **failure attribution** — every terminal failure surfaces
  in-window with a reason, and the contract covers the case where the
  daemon supplies none.

  **The reason requirement is two-sided, because the silent cases are
  the common ones.** Revision 2 required "the daemon's stated reason",
  which is unsatisfiable exactly when it matters: a daemon that
  **crashes or drops the socket delivers no `Goodbye` at all**, and the
  frontend learns of departure by EOF (§2.3 records that the GPU
  already loses its peer this way today). So: **use the daemon's reason
  when one arrives** — which is why Stage 1c's post-handshake
  `Goodbye`-reason consumer precedes this stage — **and otherwise
  surface an explicitly locally-classified transport/EOF reason**,
  labelled as locally inferred rather than reported. A banner that says
  nothing because the daemon said nothing is the failure this clause
  exists to prevent. §9's activity-indicator gap is *not* claimed by
  this arc.

---

## 8. Verification shape

- **What already exists is used, not rebuilt** (major №5): the real
  offscreen `render_to_view` composition harness, the readback path,
  the smoke tests, and the required-GPU CI job are **landed**. Stages
  3c, 8 and any pixel-visible change add pixel assertions against
  that harness immediately. What is deferred from
  `gpu-golden-harness-framing.md` is only **golden-PNG comparison and
  the case gallery**; a stage adopts those if image diffing beats
  direct assertions for its witnesses, with that framing.
- **The a37 problem is confronted, not inherited.** Real-GPU
  end-to-end tests compile only when `pmacs-gpu` is built, return
  `ok` without running otherwise, and are load-sensitive — the
  recorded footing hazard. Stage 1-pre's seam exists so input stages
  are witnessed *without* a display; stages that genuinely need a
  real frontend say so and name their witness (`PMACS_REQUIRE_GPU`
  discipline).
- **The arc ratchet**: extend `tests/journey_acceptance.rs` with
  GPU-frontend rows where headlessly drivable; stages add rows, none
  removes them — same rule as the existing ratchet.

---

## 9. Rulings — Q#GA4 through Q#GA12, all closed

**Every arc-level question is ruled as of revision 3.** They are kept
here with their answers rather than deleted, because a stage framing
that wants to revisit one needs to see what was decided and why it is
not open.

- **Q#GA4 — RULED.** The name is "the GUI arc"; the numeric **Arc 8
  label retires** at Stage 0, resolving the collision with the Lean 4
  framing's claim on the same number.
- **Q#GA5 — RULED, with a hardening.** Half A slots after P1. Reaching
  Stage 4b is a **P2 start gate**: no later GUI stage starts until P2
  has an approved framing and an opened lane, after which non-gated
  work interleaves freely while the object lands (§5.1). Stronger than
  the recommendation carried in revision 2, which would have let every
  **non-gated** stage finish before P2 began — leaving P2 a terminal
  closure blocker rather than letting the arc close around it.
- **Q#GA6 — RULED.** Land the TUI answer in Stage 1b; no declared
  divergence. The events already arrive and are dropped by the
  document-window handler (Stage 1b records the sites).
- **Q#GA7 — RULED.** Full preedit overlay: commit string, caret and
  selection range, cancellation, focus-loss cleanup. Not the
  commit-string minimum.
- **Q#GA8 — RULED.** Wait for Stage 2. **No temporary Ctrl+wheel
  island**, so this arc adds no off-path hardcode (§7).
- **Q#GA9 — RULED.** Implement the grid answer with UAX #14. No
  declared divergence.
- **Q#GA10 — RULED.** Preserve **both** surfaces: automatic restore on
  the first eligible attach when armed and no explicit target was
  supplied, plus the existing explicit command. Stage 4b owns the
  paired **save** path too, and three questions remain live inside that
  stage — snapshot ownership, save timing, multi-frontend arbitration.
- **Q#GA11 — RULED.** The §3.1 blocker seed stands **unchanged at nine
  items**.
- **Q#GA12 — RULED as a deliberate deferral** to Stage 10, taken after
  P2 and the multi-window model exist. The existing-object,
  no-parallel-registry and optional/off-by-default constraints hold
  from now, not from Stage 10.

---

## 10. Sequencing against #227 (git Stage 1)

Settled with the user on 2026-08-10, and recorded here because it
constrains when Stage 0 may start:

1. **Revision 3 → approval.**
2. **The approved framing and the Stage 0 lane are committed and
   pushed** on the Stage 0 branch, as its first commit. Until that
   happens this document is not portable and nothing downstream is
   safe to rely on.
3. **#227 is finished and merged** before Stage 0 implementation.
4. **Stage 0 rebases and performs the absorption**, which by then
   includes **#227's own newly merged lane** alongside the five already
   enumerated.

The reason #227 goes first rather than riding alongside: its ref is
**72 `main` commits behind**, and it touches `COHERENCE.md`,
`docs/active-work.md` and `builtin/runtime/listview.lua` — the three
files Stage 0's absorption rewrites. Carrying it across the arc would
compound exactly the conflicts Stage 0 exists to retire.
