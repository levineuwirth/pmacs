# GPU terminal input — the double terminal-layout sync

**Revision 2 — approved 2026-07-25. Scouted against canonical `main` @
`8c86d34`; implemented on branch `gpu-terminal-input` off `main` @ `46a1b8f`,
whose only delta (#161) touches no file on this fix surface. Protocol stays
v20.**

Revision 2 answers Q#GT4 from the code instead of deferring it, which changes
the proposed fix from a one-line guard to a **split of `sync_terminal_layout`
into a frontend-kind-neutral liveness half and a grid-only geometry half**;
rescores B1 as half-false; gives acceptance criteria 2 and 3 a landable
observation seam; and corrects four line citations plus the criterion-4
rationale. Revision 1's diagnosis is unchanged — the defect, its measurements,
and the three falsified hypotheses all stand.

Reported symptom: *"Text input within the terminal doesn't work on GUI, this
is fine in TUI."*

This is a bug-fix framing, not a feature. It repairs a defect in Vterm Stage 3
(#135) that ships on `main` today, and it closes the acceptance hole that let
the defect ship: the Stage 3 real-path acceptance drives a terminal session
end to end, and *still could not see this*.

## Summary of the defect

Every dispatcher tick, the daemon applies **both** terminal-layout syncs to
**every** attached frontend:

```rust
// src/daemon.rs:1536-1554 (current main)
for frontend_id in &attached_fids {
    if let Some(size) = term_sizes.get(frontend_id).copied() {
        editor.sync_terminal_layout(*frontend_id, size);          // GRID path
    }
    if let Some((buffer_id, size)) = semantic_states
        .get(frontend_id)
        .and_then(SemanticRenderState::terminal_viewport)
    {
        editor.sync_semantic_terminal_layout(*frontend_id, buffer_id, size);  // SEMANTIC path
    }
}
```

They are written as twins — the comment on the semantic arm even says *"right
beside the grid sync"* — but they are applied as **siblings, not
alternatives**. A GPU session has an entry in `term_sizes` (its `AttachRequest`
carries an initial cell size, and `Resize` events maintain it) *and* a
semantic terminal declaration. So both run, every tick.

The two disagree by construction, and the semantic arm's own doc comment says
why:

> the frontend declared a CONTENT rectangle, so this consumes the size
> directly instead of running the TUI placement helper, **which would subtract
> a modeline the GPU never drew**.

That is exactly what the grid arm then does. Measured, on a real daemon with a
real PTY and the real GPU attach client:

```
PROBE sync_semantic old=Some(24x80) declared=25x92
PROBE manager.resize BufferId(3) 25x92
PROBE manager.resize BufferId(3) 22x80
PROBE sync_semantic old=Some(22x80) declared=25x92
PROBE manager.resize BufferId(3) 25x92
PROBE manager.resize BufferId(3) 22x80
...
```

The PTY is resized **twice per dispatcher tick, forever**. Each resize is a
`TIOCSWINSZ` + `SIGWINCH` to the child and a screen reflow in
`TerminalScreen`, so the child gets a SIGWINCH storm at tick cadence and the
screen alternates between two geometries. An interactive line editor
(readline, zle, fish's reader) redraws on every SIGWINCH, so what the user
types is continuously destroyed before it can settle — while ordinary child
*output* keeps flowing, which is why the terminal looks alive.

Measured user-visible effect, real bash `-i` in the real GPU path, typing one
character:

| | frames for a static screen | typed `Z` ever visible at the prompt |
|---|---|---|
| `main` today | **730** in a 20 s window | **no** |
| with the guard | **2** | (see Q#GT5 — a separate question) |

The TUI is unaffected: a grid session has no semantic terminal declaration, so
only one arm ever runs for it. This is a **frontend-kind** defect, which is
why it presents as "GUI broken, TUI fine".

## Ground truth (measured this session, not inferred)

Everything below was established against `main` @ `8c86d34` with a real
daemon, a real PTY child, and the real `pmacs-gpu` attach client. The probe
harness is preserved (see "Verification plan").

### What is *not* wrong — three hypotheses falsified

Recording these because each is a plausible-looking cause that a future
reader (or a review round) will re-propose.

1. **The GPU's optimistic-CRDT path is not implicated.** The first hypothesis
   was that a typed character becomes a `CrdtOp` against the read-only
   terminal identity buffer and is dropped. It does not. Terminal buffers are
   already marked round-trip — `core.set_round_trip_input(buffer_id, true)`
   at `src/terminal/session.rs:338`, beside `set_read_only(true)` — so
   `dispatch_idle_for` returns **false** while a terminal window is focused,
   the daemon publishes `DispatchIdle { idle: false }`, and the GPU's
   `daemon_intercepts_keys()` is true. Measured on the wire:
   `dispatch_idle_in_terminal=false`, `intercept_in_terminal=true`,
   `input_route=send_key(intercept)`. The optimistic gate is shut.
2. **Key transport is not implicated.** The keystroke reaches the daemon,
   resolves a terminal view key, encodes, and is written to the PTY without
   error: `PROBE dispatch_key ... terminal_key=Some(TerminalViewKey { .. })`,
   `PROBE terminal transport encode=Some([90])`, `PROBE after send status=""`.
   With a `cat` child the byte comes back on screen through the whole real GPU
   path (`echoed_typed_char=true`).
3. **The `pmacs --attach` TUI replica does *not* share the defect.** It gates
   its optimistic path on `dispatch_idle` alone (`src/attach.rs:843`), and
   that signal is already correct for terminals per (1).

### The mechanism

- `EditorInstance::sync_terminal_layout` (`src/editor.rs:1195`) is the grid
  path: it runs the TUI placement helper over the frontend's *frame* size.
- `EditorInstance::sync_semantic_terminal_layout` (`src/editor.rs:1331`) is
  the semantic path: it consumes a declared *content* rectangle directly.
- Both resolve the same controller and call `TerminalManager::resize` on the
  same session. Each has a correct `old_size == size` idempotence guard
  (`src/editor.rs:1239` grid, `src/editor.rs:1360` semantic) — the guards are
  individually sound and jointly useless, because each arm sees the size the
  *other* just installed.
- `TerminalViewStore::record_view_size` (`src/terminal/view.rs:276-292`)
  returns `true` for any valid declaration with no unchanged-size dedupe,
  which is why the semantic arm re-fires every tick against the grid arm's
  flip rather than settling.
- Loop order is grid first, semantic second, so the screen *ends* each tick at
  the declared size. That is why rendering looks alive while the child is
  whipsawed — and why a frame-based assertion is the wrong instrument
  (acceptance criteria 2 and 3).
- `TerminalScreen::changed` bumps `generation` per mutation
  (`src/terminal/screen.rs:1467`), which is why generation advances by
  **exactly 2** per tick — one bump per resize.
- Frame suppression is full-struct equality
  (`self.last_terminal_frame.as_ref() == Some(&frame)`,
  `src/semantic_render.rs:882`). It is behaving correctly: the frames really
  do differ. The churn is upstream, and fixing the churn fixes the frame
  storm. **No suppression change is proposed.**

### Why the Stage 3 acceptance could not catch it

`a37_real_daemon_real_pty_and_headless_gpu_render_one_terminal_session`
(`tests/vterm_stage3_acceptance.rs:637`) is a genuine real-daemon +
real-PTY + real-wgpu path, and it still passes on the broken tree. Three
reasons, each worth keeping:

1. Its child is `sh` printing 400 rows on a timer. **A frame storm is
   invisible against a child that legitimately produces ~400 frames**, and its
   only frame-count assertion is `frames >= 2`.
2. Its input step is `client.send_key(...)` called **directly**
   (`pmacs-gpu/src/main.rs:784-785`), so it pins transport, not routing — and
   it asserts nothing about the result of that input reaching the child.
3. It resizes **once, deliberately**, and asserts the new width comes back.
   A geometry that oscillates *through* the asserted width satisfies that
   assertion. This is the project's own "a geometric readout is not a state
   predicate" lesson (`docs/active-work.md`, bottom-panel round 2) in a new
   place: `observed_resized_frame` says "a frame at this width arrived", not
   "the geometry settled at this width".

## Decisions

**Q#GT1 — Where does the fix go?** `sync_terminal_layout` is **split**, and
only its geometry half is gated by frontend kind. A bare "skip the grid arm
for semantic frontends" guard is wrong — see Q#GT4, which establishes that the
grid arm is also the only per-tick controller-liveness release. Not by
removing `term_sizes` for semantic sessions: semantic key and mouse dispatch
hard-depend on it (`src/daemon.rs:2191-2213`).

The function has three separable concerns
(`src/editor.rs:1195-1260`), and they do not split where the name suggests:

| lines | concern | frontend kind |
|---|---|---|
| 1199 | `reconcile_panel_layout` (Q#BP2b per-tick defensive) | **neutral** |
| 1200-1221 | controller liveness: released when the frontend has no view, or its active window no longer shows that terminal | **neutral** — reads only `core.views` / `core.windows` / the controller, never `term_size` |
| 1222-1259 | TUI placement (`window_placements`) + `resize` | **grid only** |

So the daemon loop becomes: run the neutral half for every attached frontend
every tick, then exactly one geometry arm per frontend kind.
`sync_terminal_layout` survives as the composition of both halves, so
`editor::run`'s in-process loop and `LOCAL` keep byte-identical behavior. The
liveness half must run **once** per frontend per tick — reconciliation is
idempotent, so a double call is safe rather than wrong, but the loop should
not pay for it.

**The trap inside the split:** the third release, at `src/editor.rs:1226`
(no placement found for the window), looks like liveness and is **not** — it
is grid geometry. A semantic frontend has no `window_placements` entry at all,
so moving that arm into the neutral half would release a GPU session's
controller on every single tick. That would be a new defect of exactly the
family this framing fixes, so it stays in the grid half.

**Q#GT2 — Which arm wins for a semantic frontend?** The semantic one,
unconditionally. It is the only arm that consumes a *content* rectangle; the
grid arm's modeline subtraction is meaningless for a frontend that draws no
modeline into the terminal band. A GPU frontend that has not yet declared a
terminal viewport gets **neither** arm, which is correct: the terminal keeps
the geometry it was opened with until the frontend declares one.

**Q#GT3 — Is the guard "no semantic state" or "not a semantic session"?**
`semantic_states` keyed by frontend id is the same map the semantic arm reads
one line later, so the two arms become provably exclusive by construction
rather than by two independent predicates that could drift apart. Rejected
alternative: keying on the negotiated `semantic_render` capability bit — it is
the *same* fact one indirection away, and the pair could then disagree.

**Q#GT4 — Does anything else in `sync_terminal_layout` need to keep running
for a semantic frontend? Yes: the controller-liveness release, and the
semantic arm neither performs it nor can be made to.** Revision 1 left this
open; the code answers it.

`release_controller` is called from exactly five sites, all in
`src/editor.rs`: the three grid-arm early returns (1210, 1219, 1226),
`dispatch_focus(gained = false)` (1189), and `reconcile_panel_layout`'s
unsatisfiable-panel path (848). **`sync_semantic_terminal_layout` releases
nothing.** When the window has switched away, `semantic_terminal_key` returns
`None` (`src/editor.rs:1278` — `window.buffer_id != buffer_id`) and the arm
returns `false` without touching the controller.

Growing a release inside the semantic arm — revision 1's stated fallback for
B1 — **cannot work**, and the reason is worth keeping: when a GPU window
switches from the terminal to a document, the buffer-follow snapshot clears
the viewport declaration (`on_buffer_snapshot_sent` sets
`terminal_viewport = None`, `src/semantic_render.rs:574`), so
`terminal_viewport()` returns `None` and the semantic arm **stops running
entirely** for that frontend. A release placed inside it would never execute
in precisely the scenario that needs it.

Nor do the other two sites cover it: `dispatch_focus(false)` fires on
whole-frontend focus loss, not on a window or buffer switch, and semantic
sessions are not panel-capable yet (`panel_capable_for` is false for them —
`src/daemon.rs:1893-1898`), so 848 never fires either.

Consequence of shipping revision 1's guard as written: a GPU frontend that
switches away from its terminal **holds the controller indefinitely**. Because
another frontend's grid sync early-returns on a
`controller_view_for_frontend` mismatch, that peer then cannot resize the PTY
until it explicitly re-claims. This is why Q#GT1 splits the function instead
of gating it.

**Q#GT7 — The per-tick defensive panel reconcile stays for semantic
frontends.** It is the only per-tick pre-paint reconcile the Q#BP2b contract
names (`src/editor.rs:822-830`), and today the grid arm supplies it for GPU
sessions too. Putting it in the neutral half of the split preserves that
exactly. It is harmless-either-way today — semantic sessions have unknown
frame geometry until the bottom-panel GPU band lands — but "harmless today"
is not a reason to remove a contract's only per-tick enforcement point in a
PR about something else.

**Q#GT5 — Typed characters not echoing by an interactive shell is a
*separate* question and is deliberately out of scope.** Measured: with `bash
--norc -i` on a `TerminalMode::Raw` PTY, typed characters are not echoed to
the screen — **and this reproduces identically in-process**, i.e. on the TUI's
own path, where the user reports the terminal works. Because it is not
frontend-specific it cannot be the GUI/TUI asymmetry, and folding it in would
make this PR two features. It gets its own scout: whether `TerminalMode::Raw`
is the right mode for a `pmacs.terminal.open` child, and what pmacs owes a
child that expects to own its termios. Named, not silently dropped.

**Q#GT6 — Protocol impact: none.** No wire shape, no negotiation, no version
change. Stays v20.

## Bets

- **B1 — SCORED HALF-FALSE before implementation (revision 2).** "Removing the
  grid arm for semantic frontends removes the storm without removing any
  behavior a GPU session relies on." The first clause holds (measured). The
  second is **false**: it also removes the only controller-liveness release
  and the only per-tick Q#BP2b reconcile a GPU session gets (Q#GT4, Q#GT7).
  Its stated contingency — "the semantic arm grows the release" — is false
  too, for a structural reason (`terminal_viewport` is cleared by the very
  snapshot that signals the switch-away). Hence the split in Q#GT1. Recorded
  rather than deleted: the failure mode is one a reviewer or a future
  simplification will re-propose.
- **B2.** The user's reported symptom is this defect. *Partially scored: the
  storm is proven and GUI-only, and its shape (line editor unusable, output
  still flowing) matches the report. Not fully scored until the user, or an
  acceptance running the **user's own shell**, confirms typing works after the
  fix. Q#GT5 is the reason this bet is stated rather than assumed.*
- **B3.** No other pair of per-frontend-kind daemon operations is applied as
  siblings rather than alternatives. *Scored by an explicit audit of the
  dispatcher's per-frontend loop during implementation — this defect's shape
  is "twins applied as siblings", and it would be negligent to fix one
  instance without looking for others.*

## Deferred (named)

- Interactive-shell echo on a raw-mode PTY (Q#GT5) — its own scout.
- **A geometry change appears to clear the visible screen.** Observed while
  building acceptance 4: after the probe's deliberate 25×92 → 20×71 resize,
  the next frame's visible grid is entirely blank even though the content
  (two short lines near the top) should survive a shrink of that size. It
  reproduces on the pre-fix tree, so it is neither caused nor fixed here, and
  it is why acceptance 4 latches its observation across frames instead of
  reading the final one. Not investigated: it could be correct reflow
  behaviour given where the child leaves its cursor (frames show the cursor
  on the bottom row), or a real reflow defect. Named because the next person
  to write a resize assertion will hit it.
- `TerminalFrame` suppression including `screen_generation` in its equality:
  correct today and load-bearing for correctness, but it means any future
  content-neutral generation bump re-emits a frame. Recorded, not changed.
- The `a37` probe's structural weaknesses beyond what the acceptance below
  fixes (it still cannot exercise `App::window_event`'s routing, because that
  logic is inline in the winit handler with no extractable seam). Making GPU
  key routing testable is a real refactor and belongs to its own lane.

## Acceptance criteria

**The observation seam (revision 2).** Criteria 2, 3 and 6 assert daemon-side
state, and `TestDaemon` runs the daemon as a **subprocess**
(`tests/common/daemon.rs:90`), so nothing in-process can see it and the
scouting instrumentation does not land. The seam that does land: **extract the
dispatcher loop's per-frontend terminal-layout step into a named function**
that takes `(&mut EditorState, &[FrontendId], &term_sizes, &semantic_states)`.
That is required by Q#GT1's split anyway, it makes the grid/semantic
exclusivity structural rather than two adjacent `if`s, and it lets an
in-process test in the style of the existing `src/daemon.rs` unit tests
(3375ff) drive **the real loop body** rather than a re-implementation — which
is the a37 lesson applied to this PR's own tests.

The observable is `TerminalScreen::generation`, reachable through
`TerminalManager::snapshot(..).generation`. It advances once per screen
mutation (`src/terminal/screen.rs:1467`), so with a quiet child it is a
**state predicate**, not a readout: "the geometry settled" is exactly
"generation stopped advancing".

1. On a real daemon + real PTY + real GPU attach, a terminal session that
   receives no child output produces a **bounded** number of terminal frames
   (settling to zero new frames once the screen is static) — not one per tick.
   Fails on `main` with ~730 frames in 20 s; passes with ≤ a small constant.
2. Driving the extracted loop body N times against a semantic frontend with a
   fixed declaration and a quiet child: `TerminalManager::resize` takes effect
   **exactly once** (generation advances once, then is constant for the
   remaining N-1 iterations). Fails on `main`, where generation advances by
   two per iteration.
3. After a declaration, `screen_size(buffer)` **equals the declared content
   rectangle and stays equal** across subsequent iterations — the state
   predicate, not the "a frame at this width arrived" readout that
   `observed_resized_frame` provides today.
4. A character sent through the real GPU attach client reaches the child and
   its echo appears in a rendered frame. **This is a keep-working pin, not a
   fix discriminator: it already passes on today's broken `main`** (falsified
   hypothesis 2 measured `echoed_typed_char=true`). Pinned with a `cat` child
   — not because `cat` echoes (termios `ECHO` is off in raw mode; nothing
   echoes) but because `cat` *copies stdin to stdout*, so the byte comes back
   exactly once, with no line discipline and no double echo to disambiguate.
5. A grid (TUI) session's terminal resize behavior is **unchanged** — pinned
   against the existing Stage 2 real-TUI PTY smoke, which must stay green
   without modification.
6. A semantic frontend whose window stops showing the terminal **releases its
   controller** (Q#GT4), pinned through the extracted loop body — driven by an
   actual buffer switch, not by calling the release directly. This one bites
   against **revision 1's naive guard**, and deliberately **passes on `main`**:
   today's sibling arms do supply the release, by the accident of the grid arm
   running for a frontend it should never have run for. It is the pin that
   stops the fix from trading one defect for another.
7. End-to-end SIGWINCH count through the real PTY: a child trapping `WINCH`
   and printing a **fresh distinct breadcrumb per signal** (`WINCH 1`,
   `WINCH 2`, …) shows a bounded count. The distinctness is load-bearing —
   the established PTY-paint trap is that cell diffing skips both spaces and
   already-matching cells, so a repeated identical marker can assert nothing.
8. Bite-verified against **two** pre-images, because one is not enough here —
   the naive guard fixes the storm and introduces a different defect, so a
   single revert would score the fix complete when it is not. Measured
   (`cargo test --lib`, manual revert since these tests share `src/daemon.rs`
   with the production code):

   | pin | `main` (sibling arms) | rev-1 naive guard | the split |
   |---|---|---|---|
   | acc 2+3 settle | **FAIL** | pass | pass |
   | acc 6 controller release | pass | **FAIL** | pass |
   | acc 5 grid still resizes | pass | pass | pass |

   The middle column is B1's half-false score made executable: the naive
   guard's first clause holds (the storm stops) and its second does not.

Criteria 1, 2 and 7 are deliberately expressed as **quiet-child** assertions,
because the existing acceptance's chatty child is exactly what hid this.

## Coherence impact (`COHERENCE.md` §20)

- **§2 golden journey, step 8 ("Open a terminal")** — currently graded *"Works
  but undiscoverable"*. On the GPU frontend it does not work; this restores
  the step for the frontend the document calls the more capable one. Priority
  1 explicitly treats journey regressions as release blockers.
- **§16 Productize the Semantic Frontend Architecture** — graded *strong*,
  with "graceful per-frontend degradation is practiced, not aspirational" as
  its evidence, citing per-frontend fold projection. This defect is the
  counter-example: a per-frontend-kind operation applied to both kinds at
  once. The section's claim survives, but the audit should record that the
  practice is enforced by convention, not by structure — two arms that must be
  alternatives are currently just two adjacent `if`s. Q#GT1's extracted loop
  body makes this one structural; the audit note should say the *pattern* is
  still convention-enforced everywhere else (B3).
- **§6 Eliminate Hardcoded Interaction Islands** — the audit's row 6 note that
  the GPU optimistic classifier "is kept honest by `dispatch_idle_for`" is
  **confirmed correct** by this investigation (falsified hypothesis 1), and the
  §6 citation `crate::optimistic::classify_key` should be corrected: that
  symbol is `src/optimistic.rs`, the **`pmacs --attach` TUI replica's**
  classifier. The GPU's separate, unrelated classifier is
  `optimistic_insert_text` / `optimistic_crdt_insert` in
  `pmacs-gpu/src/main.rs`. Two replica frontends, two classifiers; the audit
  conflates them.
- **§19 Product Coherence Acceptance Tests** — this is a concrete instance of
  the section's thesis. Every subsystem test passed; the defect lives in how
  two correct subsystems compose per frontend kind. Criterion 1's quiet-child
  shape is the transferable technique.
- No interaction island added, no config registry surface, no background-work
  attribution change.

## Verification plan

Full gate suite per `CLAUDE.md`, plus:

- `cargo test --features crdt --test vterm_stage3_acceptance` (the suite this
  repairs) and `--test vterm_stage2_acceptance` (the TUI no-regression pin).
- `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`.
- The scouting harness is preserved and should be re-run against the branch:
  a quiet-child variant of the `a37` probe plus daemon-side resize tracing,
  saved as `scratch_gui_terminal_input.rs`, `scratch_inproc_input.rs`, and
  `gui-terminal-probe-instrumentation.patch`. The instrumentation is scratch;
  the acceptance criteria above are what lands.
- Manual confirmation with the user's own shell (fish) in a real GPU window,
  since B2 is not fully scored by any automated test (Q#GT5).

**Ops.** This doc is currently untracked in a detached-HEAD worktree
(`../pmacs-gui-term-input`), so it does not travel. On approval it becomes the
branch's first commit before any implementation, per the standing workflow —
no cross-machine expectation should attach to it until then.
