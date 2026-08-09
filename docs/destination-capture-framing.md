# A destination capture any async continuation can use

**Status: framing pass, revision 2. Pre-implementation. Awaiting
approval.**

**Revision 2 takes three review findings.** Q#DC-2's parameterization
was **incomplete** — a panel result does not depend on the captured
document window being live or non-dedicated either, not just on its
buffer, so the question now carries a full **preflight matrix** with
every omission testable. `tests/journey_acceptance.rs` joins dired as a
named **preservation suite and stop signal**; it carries the
`commit_to` scope, forged-userdata, preflight and restoration pins this
lane generalizes, and Journey Stage 1a's framing treats it as a
required gate. And **§5 (coherence impact) was missing entirely**,
which `CLAUDE.md` and `COHERENCE.md` §25 both require of
coherence-affecting work — this lane adds Lua API surface and
generalizes a Journey substrate, so it qualifies twice over.

**A prerequisite lane. PR #227 (git Stage 1) blocks on it**, and its
P1a review finding is the reason this exists.

---

## 1. Why, and why as its own lane

PR #227's review found that git's async completions mutate and display
UI without capturing the initiating frontend
(`builtin/runtime/git.lua:609`, `:854`), so a result can surface in
whichever frontend happens to be active when git exits. Run
`git.status` in frontend A, let frontend B become active, and A's panel
opens in B.

**The finding named the right mechanism.** `pmacs.window.commit_to`
exists for exactly this continuation boundary: Journey Stage 1a's
Q#JR14 built it because "the listing settles a tick or more later, and
by then the ambient frontend, selected window, and active buffer may
all name something else" (`src/editor.rs:1238-1240`).

**But it is not reachable from Lua outside one path**, which is why
this is a lane and not a line in #227:

- `commit_to` takes a `DirectoryDestinationLua`, **nonconstructible
  from Lua** by deliberate design (`src/lua_bindings/mod.rs:4256`) —
  userdata with no constructor and no setters, so a caller cannot
  fabricate a plausible triple.
- The only site that mints one is inside the `path.open-directory`
  listener dispatch (`src/editor.rs:1311`), from
  `capture_directory_destination`, which is `pub(crate)`
  (`src/editor.rs:1241`).

So any async Lua continuation that is **not** a directory open has no
way to say where its result belongs. Git is the first to need it; it
will not be the last.

Landing this inside #227 would put new Lua API surface, over another
lane's merged mechanism, inside a feature branch — the same folding
that was declined for the `scripts/gate` repair, for the same reason.

## 2. Ground truth

- **The captured data is already generic.**
  `DirectoryDestination { frontend, window, buffer }`
  (`src/editor_core.rs:159-166`) contains nothing directory-specific.
  Only its **name** and its **capture site** are.
- **The blast radius of a rename is small**: 8 references across 4
  files (`editor_core.rs`, `editor.rs`, `lua_bindings/mod.rs`,
  `lua_bindings/window_panel.rs`). Checked, not estimated.
- **`commit_to`'s preflight is four checks**
  (`src/lua_bindings/window_panel.rs:488-525`), in order: the
  requesting frontend still has a layout; the destination window is
  still live in it; **the window still shows the captured buffer**
  (Q#JR14c stale intent); and the window is not dedicated (Q#JR14f).
- **`Handle:await` refuses inside a commit scope**
  (`builtin/runtime/async.lua:87-90`) — yielding would restore the
  scope while the coroutine is still parked. Any adopter awaits
  *before* committing, as dired does.
- **Git's two continuations do not have the same shape**, and this is
  the finding that shapes the design:
  - `*git-status*` goes through `listview.open`, which resolves
    `display` with a **`"panel"`** default
    (`builtin/runtime/listview.lua:550`). It lands in the bottom
    panel, **not** in a document window.
  - `*git-diff*` calls `pmacs.window.display(buf, { select = true })`
    — the **document** target, deliberately, "so the status panel it
    was invoked from stays visible beside it"
    (`builtin/runtime/git.lua:852-854`).

## 3. The tension this lane has to resolve

`DirectoryDestination.buffer` exists for one purpose, stated at its
definition: *"what that window held at capture time, so **stale intent
loses to the user**"* — a user who replaced the buffer while work was
in flight is newer information than the request.

**That predicate is right for a document replacement and wrong for a
panel.** The git status panel does not replace the captured window's
buffer; it opens in the bottom panel beside it. Refusing to show it
because the user switched files in the document window would be a
refusal with no relationship to what the continuation actually does —
the panel case would inherit a check about a window it never touches.

Meanwhile the diff case *is* a document replacement, and wants exactly
the dired semantics.

So a single one-size destination either **over-refuses** the panel case
or **under-checks** the document case. Q#DC-2 is where that gets
decided, and it is the substance of this lane.

## 4. The change, in outline

- **A Lua-reachable capture**, returning the same nonconstructible
  userdata for the *current* frontend and its document window.
- **Generic naming.** `DirectoryDestination` becomes something that
  does not lie about a git panel; `capture_directory_destination` and
  the userdata type follow. 8 references (§2).
- **The directory path keeps behaving exactly as it does today** — this
  lane generalizes the capture, it does not change Journey Stage 1a's
  semantics.
- **No adopter in this lane.** Git's adoption is #227's, after this
  lands. A prerequisite that also converts its first consumer makes the
  two impossible to review separately.

## 5. Coherence impact (§20)

**Revision 1 omitted this section entirely, and it is required.**
`CLAUDE.md` and `COHERENCE.md` §25 both say a framing for
coherence-affecting work must cite the section it serves and state its
impact — and this lane adds **new Lua API surface** and generalizes a
Journey-substrate mechanism, which is coherence-affecting on both
counts. Recording the impacts as neutral where they are neutral is part
of the requirement, not a way around it.

- **§16 semantic frontend — the section this serves.** The defect it
  removes is a continuation resolving its target from *ambient* state a
  tick after the request, which is precisely the multi-frontend
  correctness §16 exists to protect. A capture makes "which frontend
  asked" a value rather than a guess.
- **§14 workbench primitives — indirect, and the honest framing is
  *enabling*.** This does not add a primitive. It removes the reason an
  async adopter would hand-roll frontend tracking, which is the
  mechanism by which primitives acquire per-consumer idiosyncrasies.
- **Journey steps touched: none directly, one PROTECTED.** The golden
  journey does not gain a step. But Journey Stage 1a's Q#JR14 substrate
  is what this generalizes, and §7 makes `tests/journey_acceptance.rs`
  a preservation suite precisely so a generalization cannot erode the
  step it came from.
- **Interaction islands (§6): none added.** No key interception, no
  dispatch precedence rung. `dispatch_key` is untouched.
- **Config registry: no setting.** Where a continuation lands is a
  correctness property, not a preference, and a toggle would offer to
  turn correctness off.
- **Background-work attribution (§9): NEUTRAL, and worth stating
  precisely rather than skipping.** This lane adds no background work
  and no new unattributable surface. It also does **not** improve §9 —
  knowing which frontend a result belongs to is not knowing who asked
  for it or why. That is the worker-identity lane's arc, and the two
  should not be confused because both concern async continuations.
- **§10 extension trust — a small positive.** The capture keeps the
  Q#JR14d property that a destination is **nonconstructible from Lua**,
  so generalizing the mechanism does not widen what extension code can
  fabricate. §7 re-asserts the forged-destination refusal after the
  rename for exactly this reason.

## 6. Open questions

### Q#DC-1 — what does the capture take as arguments?

*My vote: **no arguments** — capture the acting frontend and its
document window from the ambient state at call time.* That is what the
existing `capture_directory_destination(frontend, window)` is handed by
its one caller, and a Lua-supplied frontend id would reintroduce the
fabrication hole the userdata design closes.

### Q#DC-2 — one destination shape, or a panel/document distinction? **(the substantive one)**

§3 is the problem. Three candidates:

1. **One shape, all four checks.** Simplest; over-refuses the panel
   case, and the refusal reason would be about a window the panel does
   not touch.
2. **One shape, preflight parameterized by the continuation** — the
   caller declares whether it is replacing the captured window's
   buffer, and the stale-intent check applies only then.
3. **Two capture kinds**, document and panel, with different preflights.

*My vote: **(2)***, with the profiles spelled out below rather than
left to implementation.

**Revision 1 said only "skip the stale-buffer check for a non-replacing
continuation", and that was incomplete.** Review is right: a panel
result does not depend on the captured **document window** at all. It
does not replace that window's buffer, so check 3 is irrelevant; it
does not occupy that window, so check 4 (dedicated) is irrelevant; and
it does not need that specific window to exist, so check 2 is
irrelevant. Retaining any of the three can reject `git.status` for a
document-window change that has nothing to do with where the panel
goes. But dropping them **without an explicit profile** is how document
replacement quietly loses its guarantees.

**The matrix, stated so every omission is deliberate and testable:**

| # | Precondition (`window_panel.rs:488-525`) | Document replacement | Frontend/panel scope |
|---|---|---|---|
| 1 | Requesting frontend still has a layout | **required** | **required** |
| 2 | Destination window still live in it | **required** | not applicable |
| 3 | Window still shows the captured buffer (Q#JR14c stale intent) | **required** | not applicable |
| 4 | Window is not dedicated (Q#JR14f) | **required** | not applicable |

**Check 1 is the entire panel profile**, and that is the honest reading
of what a panel continuation actually depends on: the frontend it was
launched from still exists. Everything else in the capture is document
state the panel never touches.

**Consequence for the capture, which follows and should not be
discovered later:** if the panel profile needs only the frontend, then
a frontend with **no document window** can still host a panel — so
Q#DC-4's "return `nil`" is right for the document profile and possibly
wrong for the panel one. That interaction is settled as part of
answering this, not after it.

**I hold the *choice* loosely, not the matrix.** (1) has a real
argument — a uniform rule is easier to reason about, and over-refusal
is safe — but it would refuse the git panel for reasons unrelated to
it, and "safe" refusals that users cannot explain are how a mechanism
gets worked around. If review prefers (1) or (3), the matrix above is
what changes, and **every cell marked "not applicable" must still be
tested as deliberately omitted** (§7) so a future reader cannot mistake
an omission for an oversight.

### Q#DC-3 — what is the type called?

*My vote: **`ViewDestination`***, with `pmacs.window.capture_destination()`
as the Lua entry point. It names what it is — a place in a view where a
continuation's result belongs — without claiming a directory or a
buffer kind.

The Q#JR14 doc comments should keep their references intact; a rename
that orphans the rationale is worse than a slightly stale name.

### Q#DC-4 — is the capture refused when there is no document window?

`capture_directory_destination` already returns `None` when the
frontend has no document window (`src/editor.rs:1236`). *My vote:
**return `nil`, and require every adopter to handle it***, rather than
inventing a fallback destination. A continuation with nowhere to land
should say so, and #227's adopter should degrade to today's ambient
behaviour with a status message rather than silently guessing.

## 7. Verification

- **A captured destination survives a frontend switch**: capture in A,
  make B active, commit, and assert the result lands in **A**. This is
  P1a's actual failure and the reason the lane exists — asserting only
  that the API returns userdata would pass on a capture that does
  nothing.
- **A fabricated destination is still refused** — the existing Q#JR14d
  guarantee, re-asserted after the rename so the generalization cannot
  quietly open the hole it was built to close.
- **Every preflight refusal is witnessed by its own case, in BOTH
  profiles** (Q#DC-2's matrix): frontend gone, window gone, stale
  buffer, dedicated window — each asserted to **refuse** under the
  document profile, and each of the three marked "not applicable"
  asserted to **NOT refuse** under the panel profile. A deliberately
  omitted check that has no test is indistinguishable from a check
  someone forgot, and the next reader will restore it.
- **`nil` when the frontend has no document window** (Q#DC-4) — for
  the **document** profile. Whether the panel profile can capture
  without one follows from Q#DC-2 and is asserted whichever way it is
  answered.
- **The directory path is unchanged** — dired's existing acceptance
  coverage passes untouched.
- **`tests/journey_acceptance.rs` passes UNCHANGED**, as a named
  preservation suite. It carries the established contract this lane
  generalizes — 27 `commit_to` references across nine named pins
  including `commit_to_refuses_a_forged_destination`,
  `commit_to_scopes_and_restores_on_a_normal_return`,
  `commit_to_restores_when_the_callback_raises`,
  `commit_to_refuses_an_await_and_restores`,
  `commit_to_delivers_to_the_requesting_frontend_not_the_ambient_one`,
  `a_declining_listener_cannot_redirect_the_destination`, and two
  rows already named `preservation_*`. Journey Stage 1a's own framing
  treats this suite as a required gate; a lane that generalizes its
  substrate does not get to relax that.
- **STOP SIGNAL, for both suites.** If any existing `dired` or
  `journey_acceptance` test needs editing, the generalization changed
  Journey Stage 1a's semantics. That is cause to stop and report, not
  to adjust the test — a suite edited to accommodate the change under
  test has stopped being evidence.
- **`Handle:await` still refuses inside the scope**, including through
  `pmacs.async.yield_to_next_tick` if the worker-identity lane's Q#W-7
  has landed by then; if it has not, this lane does **not** add that
  guard — it belongs to that lane and duplicating it would produce a
  conflict for no benefit.

**What this will NOT prove:** that git surfaces in the right frontend —
that is #227's adoption, after this lands. This lane ships the
mechanism and one set of tests for the mechanism.

## 8. Not in scope

**Adopting the capture anywhere**, including git (#227 does that) and
including migrating other async continuations that have the same latent
bug — worth an audit, not this lane's work. Changing Journey Stage 1a's
directory semantics. The `commit_to` scope guard for
`yield_to_next_tick` (worker identity Q#W-7). Any protocol change —
this is entirely core + Lua bindings. Panel geometry or placement
policy, which is the bottom-panel arc's.
