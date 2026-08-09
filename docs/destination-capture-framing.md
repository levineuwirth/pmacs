# A destination capture any async continuation can use

**Status: revision 9. The mechanism is implemented at `0efc8c0`; the
correctness blocker revisions 6–9 carry is IMPLEMENTED, in revision 8's
shape with revision 9's scope correction, and §3's enumeration is
performed and recorded below.** Revisions 6 and 7 proposed fixes that
review rejected; **neither is in the tree**, and the two paragraphs
describing them are kept as the record of why this shape and not those.

*(Revisions 2–5 said "Pre-implementation. Awaiting approval" while the
ledger recorded the lane as approved and implemented. Same
contradiction class this document keeps correcting elsewhere, left
standing in its own header.)*

**Revision 9 fixes a hole in revision 8's guard — one that is about the
guard's SCOPE, not about which mutations it names.** Revision 8 refuses,
inside a `"panel"` commit, the mutations that would make its relaxed
preflight wrong. But a **nested `commit_to` REPLACED** the enclosing
contract with its own and restored it afterwards (`src/editor.rs:129`,
`src/lua_bindings/window_panel.rs`), so the outer restriction went out of
force for the whole of the inner body. Review reproduced the sequence:
an outer `"panel"` commit passes the relaxed preflight; a nested
`"document"` commit masks its contract; the nested callback dedicates the
side slot and **is not refused**; the outer commit resumes, its side
request falls back, and it overwrites a newer document — the original
P1a failure, reached through one extra call.

**What this invalidated, precisely.** *Not* §3's enumeration of
dedication write sites. That enumeration was performed against the tree,
it is still complete, and every site in it is still guarded. What was
wrong was the surrounding claim — that the guard was **in force for the
whole outer body**. §3's "PREFLIGHT STAYS WHERE IT IS" paragraph and the
enumeration that follows it are therefore kept and **qualified**, not
withdrawn.

**The fix: contracts COMPOSE across nested scopes; the strictest active
restriction wins.** The core holds a *stack* of contracts rather than one
slot: `commit_to` pushes and pops rather than swapping, and the
dedication guard consults **every** contract in force rather than the
innermost. Matching stays per frontend, so a nested commit for a
different frontend may still dedicate *its* side slot — that cannot
change where this frontend's side request lands. The alternative shape,
**prohibiting nested `commit_to` outright**, was rejected: it closes the
hole by forbidding a construction no rule objects to. `commit_to` is
public Lua API for saying where a continuation's result belongs, and a
body that commits to a second destination (a diff beside a status panel)
is where #227's adoption is heading. Only the *restriction* needed
preserving. **Detecting the dedication when the outer commit resumed was
not available**: by then the mutation has happened, which is a late
refusal, which is what revision 7 was rejected for.

**Revision 8 rejects BOTH of the previous two fixes and takes a third
shape.** Revision 6 predicted the fallback at preflight (the body can
change it). Revision 7 moved enforcement to the placement boundary —
which **breaks the invariant `commit_to` exists for**:
`docs/agent-handoff.md:748` says it preflights *before* the callback
because "validating at display time is four mutations too late", so a
placement-time refusal arrives after arbitrary Lua has created buffers,
handles and paint. Revision 8 keeps the preflight and **refuses the
mutations that would invalidate it**, the same shape as the existing
await refusal. Refusal stays mutation-free on the `(false, reason)`
path.

**Revision 6 fixes an UNSOUND matrix, not a preference.** Q#DC-2 gave
the panel profile only check 1, on the claim that a panel result never
touches a document window. **Panel placement falls back to an ordinary
document window** when the frontend is not panel-capable or its side
slot is dedicated — so a `"panel"` commit could replace a *newer*
document while skipping every stale-intent guard. Reproduced in review.
The relaxation is now conditional on the placement really being a
panel. Revision 6 also closes an invalid-UTF-8 hole in the profile
diagnostic — the same reachability class as revision 5's, one layer
down.

**Revision 5 fixes a binding-level contradiction in revision 4's own
API spec.** It required `profile: Option<String>` *and* a pointed error
naming the accepted values for a non-string — but mlua rejects a
number or table during argument conversion, before the closure runs, so
that message was unreachable. This is the exact trap the existing
binding documents for `dest`, in a comment revision 4 quoted while
repeating the mistake one argument to the right. The profile is now
`mlua::Value`, validated in the body, with `nil` and absence both
meaning `"document"`.

**Revision 4 specifies the call shape the last two revisions kept
referring to without defining.** "The profile is declared at
`commit_to`" named no signature, no value set, no invalid-profile
behaviour, and nothing about the existing two-argument callers — so
#227 had no stable API to adopt and the Journey preservation promise
rested on care rather than contract. Q#DC-5 fixes that:
`commit_to(dest, body [, profile])`, a **closed** two-value set,
**omitted means `"document"`** so every existing call keeps all four
preflight checks by definition, and an unrecognized profile **errors**
rather than falling back.

**Revision 3 decides Q#DC-4, which revision 2 left contradicting
Q#DC-2 — on the primary panel API.** Q#DC-2 concluded a panel needs
only a live frontend; Q#DC-4 still returned `nil` without a document
window and told git to fall back to ambient behaviour, which is the
very bug this lane removes. Resolved: the destination's document pair
is **optional**, `capture_destination()` is **profile-blind and
argument-free**, the profile is declared at `commit_to`, and a
document-profile commit without a document pair is refused. §4 and
Q#DC-1 were updated to match rather than left to disagree.

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
    (`builtin/runtime/listview.lua:550`). It **requests** the bottom
    panel rather than a document window — *requests*, because a side
    request FALLS BACK into a document window on a frontend that is not
    `panel_capable` or whose one slot is dedicated elsewhere. That
    fallback is this lane's blocker; §3 and Q#DC-2 carry it.
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
panel — WHILE THE PANEL REALLY IS A PANEL, which is the qualification
the rest of this document exists to add.** A git status panel that
lands in the bottom panel does not replace the captured window's
buffer; it opens beside it. Refusing to show it because the user
switched files in the document window would be a refusal with no
relationship to what the continuation actually does, and that case
would inherit a check about a window it never touches.

**Read the previous paragraph with its condition attached, not as a
standing fact.** Panel placement **falls back** into an ordinary
document window when the frontend is not `panel_capable` or its one
side slot is dedicated elsewhere — and then the panel case *does* touch
the captured window, replacing whatever the user put there. That
fallback is this lane's correctness blocker, and the unqualified
version of this claim is precisely what made revision 5's matrix
unsound. The resolution is below, at the end of Q#DC-2: the preflight
measures whether this frontend places side requests in the panel, and
the mutations that would falsify that measurement mid-commit are
refused.

Meanwhile the diff case *is* a document replacement, and wants exactly
the dired semantics.

So a single one-size destination either **over-refuses** the panel case
or **under-checks** the document case. Q#DC-2 is where that gets
decided, and it is the substance of this lane.

## 4. The change, in outline

- **A Lua-reachable capture**, returning the same nonconstructible
  userdata for the *current* frontend, **with** its document window and
  buffer when it has one and without them when it does not (Q#DC-4).
  The capture takes no arguments and is profile-blind; the profile is
  declared at `commit_to`.
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

**Check 1 is the entire panel profile ONLY WHEN THE PLACEMENT REALLY IS
A PANEL — revision 5's matrix was unsound, and this is the correction.**

The matrix rested on "the panel never touches the captured window's
buffer". **That is false when panel placement falls back.**
`editor_core.rs:4138-4148` says so in its own comment: *"Reaching
`Ordinary` while a side was REQUESTED means the request fell back (not
panel-capable, or the one slot is dedicated elsewhere)"* — and the
result is then installed into an ordinary **document** window. So a
`"panel"` commit on a non-panel-capable frontend replaces a document
view while skipping every check that exists to stop it replacing a
*newer* one. That reintroduces exactly the stale-intent failure the
API was built to prevent, which makes it a correctness defect and not
a strictness preference.

**The rule, restated:** the panel profile's relaxation is conditional
on the placement actually being a panel. Whenever placement **can**
fall back to a document window, the panel profile runs the **full
document preflight**.

**PREFLIGHT STAYS WHERE IT IS; THE MUTATION THAT WOULD INVALIDATE IT IS
REFUSED. Revisions 6 and 7 were both wrong, in opposite directions.**

Revision 6 predicted the fallback at preflight and argued the body
could not change it. **False**: the await refusal stops *concurrent
interleaving*, not the body, which is arbitrary synchronous Lua and can
dedicate the side slot itself.

Revision 7 then moved enforcement to the placement boundary. **That
breaks the invariant `commit_to` exists for.** `docs/agent-handoff.md`
`docs/agent-handoff.md:748` states it without qualification:

> [`commit_to`] preflights every precondition *before* invoking the
> callback — dired mutates handle state, `prev`, and paint long before
> it reaches anything that could refuse, so **validating at display
> time is four mutations too late**.

Refusing at placement means refusing *after* arbitrary callback code has
created buffers, handles and paint. A late refusal is not a refusal; it
is a partial commit with an error return.

**So neither predict nor refuse late — forbid the mutation.** Inside a
panel-profile commit, the operations that could change the placement
outcome are **refused**, exactly as `Handle:await` is refused inside a
commit scope and for the identical reason: something that would
invalidate the scope's guarantee is rejected rather than predicted
around. With them refused, the preflight measurement cannot go stale,
and refusal stays mutation-free on the normal `(false, reason)` path.

**"Inside a panel-profile commit" MEANS THE WHOLE BODY, INCLUDING ANY
NESTED `commit_to` (revision 9), and the unqualified version of that
phrase is what revision 8 got wrong.** Contracts **compose**: the core
holds a stack, `commit_to` pushes and pops rather than swapping, and the
guard consults every contract in force rather than the innermost. Read
every "inside a `\"panel\"` commit" below with that scope attached.
Nesting itself is *not* refused — only the mutation is, so a nested
commit that touches no dedication runs exactly as it did.

**The mutation surface is narrow, which is what makes this tight rather
than aspirational:**

- `dedicated` **is** writable from Lua — and it is one of only two
  writable window fields (`window_panel.rs:888`, *"Only `fixed_rows`
  and `dedicated` are writable (Q#BP2c)"*).
- `panel_capable` has **no Lua binding at all** — checked across
  `src/lua_bindings/`. A body cannot make a frontend panel-incapable.

**FIVE WRITES REACH DEDICATION.** Review found the second *after* the
first was specified, which is the evidence that guarding one named call
site is not a design — and the enumeration below, performed against the
tree rather than by recall, found three more. The two review named
first are:

1. **`set_params`** — the writable-field path (`window_panel.rs:888`).
2. **`display(buf, { side = …, dedicated = true })`** — writes
   `request.dedicated` straight into the side window
   (`editor_core.rs:4535`). A body can take this route, then request a
   second panel buffer and cause the fallback. **An implementation
   guarding only route 1 passes revision 8's test while keeping the
   original defect.**

**THE ENUMERATION, PERFORMED. It is CLOSED as an enumeration of WRITE
SITES, and it is closed for a structural reason rather than by inspection
stopping when it ran out of ideas.** Recorded here as the framing
required, with what was looked for, what was found, and what cannot be
ruled out.

**Read "closed" as scoped to the question it answers (revision 9).** It
answers *which writes can dedicate the side slot*, and that answer
survived review of the nesting defect intact — every site below is real
and every one is still guarded. It says nothing about *when the guard is
in force*, and that is the axis revision 8 got wrong: a nested
`commit_to` used to mask the enclosing contract, so all five reachable
sites were momentarily unguarded together. A complete list of write sites
is not a complete argument until the guard's extent is stated too, which
is what the composing-contracts paragraph above now does.

*Step 1 — how few pieces of state can matter.* `resolve_placement`
reaches `Ordinary` from a side request through exactly two branches, so
only two pieces of state are levers at all: `FrontendView::panel_capable`,
and the one side window's `Window::params.dedicated`. Everything else a
body can touch is irrelevant by construction, which is what makes the
enumeration finite instead of "every mutation in the editor".

*Step 2 — `panel_capable` is unreachable, not merely unguarded.* It is
written **only** where a `FrontendView` is constructed, and no
`FrontendView` is constructed, registered or unregistered anywhere in
`src/lua_bindings/` — `register_frontend_view` and
`unregister_frontend_view` have callers only in `daemon.rs` (attach and
detach) and in core unit tests. A body cannot reach it.

*Step 3 — every write to `dedicated`, from `rg 'params\.dedicated\s*='
src/`, classified.* Eight sites, no exceptions:

| # | site | verdict |
|---|---|---|
| 1 | `apply_placement`, `Side` **created** | reachable — `display{side, dedicated}` with no panel yet |
| 2 | `apply_placement`, `Side` **replacing** | reachable — `display{side, dedicated}`, different buffer |
| 3 | `apply_placement`, `Side` **non-replacing** | reachable — `display{side, dedicated}`, same buffer |
| 4 | `apply_placement`, `Ordinary` (`!fell_back`) | harmless — every `Ordinary` target is filtered `!is_side`, so it is never the slot |
| 5 | `apply_placement`, `Ordinary` (clear) | harmless — only ever writes `false` |
| 6 | `set_params` | reachable — the direct write (Q#BP2c) |
| 7 | `quit_window`, `QuitAction::Restore` | **unreachable**, see below |
| 8 | an `EditorCore` unit test | not Lua-reachable |

*Step 4 — the guards, sited where the property converges rather than at
each caller.* Sites 1, 2, 3 (and 4, 5) are all reached through
`apply_placement`, which has **exactly one caller**, `display_buffer`.
So one guard there covers every request-driven dedication, including
routes that do not exist yet. `set_params` is a genuinely separate write
and is guarded separately — dedication does *not* converge before the
field itself, and that is stated rather than papered over. Two live
guards, five reachable sites.

*Step 5 — what was looked for and found NOT to be a route.* Closing the
side window is **not** one: with no side leaf `side_window_for` returns
`None` and `resolve_placement` **creates** a fresh panel rather than
falling back, so quitting or hiding the panel mid-commit is safe, and
`panel_hidden` is not consulted by placement at all. `params.side` is
likewise unreachable — `set_params` refuses it and only
`apply_placement`'s created branch writes it, so a body cannot promote
an already-dedicated document window into the slot.

*Step 6 — site 7 is unreachable, and this is the one finding that
surprised.* `QuitAction::Restore` carries the outgoing `dedicated` flag,
so quitting the panel looked like a route with no `dedicated` argument
at the call site at all. It cannot be constructed: `Restore` is only
ever *stored* on a **replacing** side placement, and a dedicated slot
can never be the target of one — a side request with a different buffer
falls through to `Ordinary`, and an exact-target request is refused by
`window_accepts_buffer`. So `Restore { dedicated: true }` has no
producer. It is guarded anyway, defensively and labelled as such,
because its unreachability is an emergent property of two rules in a
different function.

**What this does NOT rule out.** The enumeration is closed over the
current tree, not over future edits: relaxing `resolve_placement`'s
dedicated arm, or adding a binding that writes `params.dedicated`
directly, reopens it. `Window::params.dedicated` is a public field, so
the compiler does not enforce the funnel — the acceptance rows are what
would catch a regression, one per reachable site.

**And it never ruled out a defect in the guard's EXTENT, which is what
revision 9 found.** Nothing above is about *when*
`panel_commit_dedication_refusal` answers; a list of write sites cannot
notice that the contract it reads was masked by a nested scope. The
acceptance suite now drives the same write-site rows at **two depths** —
directly in a `"panel"` body, and through a nested `commit_to` — so a
route guarded at one depth and not the other fails loudly rather than
being covered by the enumeration's word "closed".

**If the enumeration had turned out open-ended**, the fallback was to
**collapse the two profiles** — run all four checks always, losing the
panel relaxation. That is safe, simple, and honest; it is not the
preferred answer only because it makes the parameterization pointless.
Choosing it is a design decision needing its own approval, not a
silent retreat. **It was not needed.**

**What is NOT the fix: refusing a panel commit that would fall back.**
Falling back to an ordinary window is existing, deliberate behaviour
for a frontend without panel capability; refusing would turn a
graceful degradation into an error and regress consumers that work
today. The panel profile relaxes checks; it does not get to change
where things land.

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

### Q#DC-5 — the exact Lua call shape for the profile **(new in rev 4)**

Revisions 2 and 3 said "the profile is declared at `commit_to`" and
never said **how**. That is not a detail: today's binding accepts
exactly `(dest, body)` (`window_panel.rs:453-456`), so without a
specified form #227 has no stable API to adopt against, and the
promise that existing callers keep their semantics is a hope rather
than a contract.

**The signature:**

```lua
pmacs.window.commit_to(dest, body)             -- document profile
pmacs.window.commit_to(dest, body, "panel")    -- panel profile
```

- **`profile` is an OPTIONAL THIRD argument, typed `mlua::Value` at
  the binding — NOT `Option<String>`.**

  **Revision 4 said `Option<String>` and that contradicted its own
  error requirement.** mlua rejects a number or table *during argument
  conversion*, before the closure body runs, so the promised message
  naming `"document"` and `"panel"` would be **unreachable** — a caller
  passing `42` would get mlua's generic conversion error instead. This
  is the identical trap the existing binding already documented for
  `dest`, in a comment revision 4 cited while making the same mistake
  one argument to the right:

  > Typed as `Value` rather than `AnyUserData` so this message is
  > REACHABLE: with the narrower type mlua rejects a table during
  > argument conversion, and a caller who fabricated one got "error
  > converting Lua table to userdata" — true, but it names neither the
  > rule nor how to get a real destination.

  So: accept `Value`, and validate in the body.
  - **`Nil` or absent → `"document"`.** Both spellings, since
    `commit_to(dest, body, nil)` is what a Lua caller threading an
    optional variable produces, and it must not be a third behaviour.
  - **`String` → must be `"document"` or `"panel"`**, else refused,
    naming both accepted values.
  - **Anything else → refused by the SAME message**, which now names
    the accepted values *and* says a string was expected. That message
    only exists if the type is `Value`.
- No arity sniffing and no table-or-function dispatch on argument 2 —
  a polymorphic second argument would put the *destination*'s error
  message back at risk, which is what that comment was protecting.
- **Trailing, and readable in practice.** A profile after a long inline
  closure would read badly, but that is not the call shape in use:
  dired defines `local function commit() … end` and calls
  `commit_to(opts.dest, commit)` (`builtin/runtime/dired.lua:670,717`).
  Against a named body, `commit_to(dest, commit, "panel")` reads fine.
- **The value set is CLOSED: `"document"` and `"panel"`.** Exactly the
  two profiles in Q#DC-2's matrix. Not an open string namespace — a
  third profile is a decision, not a spelling.
- **Omitted means `"document"`.** This is the load-bearing part: every
  existing `commit_to(dest, fn)` call keeps **all four** preflight
  checks, unchanged, by definition of the signature. `journey_acceptance`
  passing untouched (§7) then follows from the API shape rather than
  from care.
- **An unrecognized profile is an ERROR**, naming the accepted values —
  **not** a silent fall back to `"document"`. A fallback would hand a
  caller stricter or looser checks than it asked for, which is the
  failure mode the whole parameterization exists to prevent. A
  non-string profile errors the same way.

**Which profile each of git's continuations takes**, so #227's adoption
is decided here rather than rediscovered: `*git-status*` → **panel**
(it lands in the bottom panel, `listview.lua:550`); `*git-diff*` →
**document** (it replaces a document window deliberately,
`git.lua:852-854`).

### Q#DC-4 — what happens when there is no document window? **(DECIDED in rev 3)**

**Revision 2 left this contradicting Q#DC-2 and it is the primary panel
API, so it is decided here rather than voted on.** Q#DC-2 concluded a
panel profile depends only on a live frontend — so it can commit with
no document window at all — while this question still said the capture
returns `nil` in exactly that case, and told git to fall back to
ambient behaviour. Those cannot both hold, and the fallback advice was
independently wrong: falling back to ambient **is** the P1a bug this
lane exists to remove.

**The decision:**

- **`ViewDestination { frontend, window: Option<WindowId>, buffer:
  Option<BufferId> }`.** The frontend is always present; the document
  pair is optional and absent exactly when the frontend has no document
  window.
- **`capture_destination()` is NOT profile-aware and takes no
  arguments.** It records what is there. Making capture profile-aware
  would force the caller to know at *capture* time what it will do at
  *commit* time, which is the opposite of why capture exists — the
  whole point is to freeze the truth early and decide later.
- **The profile is declared at `commit_to`**, which is where Q#DC-2's
  parameterization already lives. One place makes the decision, and it
  is the place that knows. **Its exact call shape is Q#DC-5**, which
  revisions 2 and 3 left unspecified.
- **A document-profile commit on a destination with no document pair is
  REFUSED**, with a reason naming that, joining the four preflight
  refusals rather than being a separate failure mode.
- **Capture therefore never returns `nil`** while a frontend exists,
  and the "adopter degrades to ambient" advice is **withdrawn**. An
  adopter with nowhere to land gets a refusal it can report; it does
  not get permission to guess.

**What this changes elsewhere, so the decision does not sit alone:**
§4's outline says the capture returns userdata "for the *current*
frontend and its document window" — it returns one for the current
frontend, **with** its document window when there is one. Q#DC-1's "no
arguments" answer is unchanged and now load-bearing rather than
incidental: no arguments is what keeps capture profile-blind.

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
- **A legacy two-argument `commit_to(dest, body)` gets the DOCUMENT
  profile** (Q#DC-5), witnessed by a check the panel profile omits —
  a stale-buffer refusal. Asserting merely that it does not error would
  pass on a call silently downgraded to the panel profile, which is the
  regression that would quietly void Journey Stage 1a's guarantees.
- **A `"panel"` commit that FALLS BACK to a document window is checked
  against the document preconditions**, witnessed for **both** causes
  separately — a non-panel-capable frontend, and a dedicated side slot.
  Each asserts the stale-intent refusal fires: capture A, make B newer,
  commit `"panel"`, observe the refusal rather than B being replaced.
- **A BODY THAT TRIES TO CREATE THE FALLBACK IS REFUSED AT THE ATTEMPT**,
  in its own test: the callback dedicates the side slot **mid-commit**.
  Three assertions, and the second and third are the ones that matter:
  the dedication call itself is **refused**; the side slot is **still
  undedicated afterwards**; and no partial result was installed.
  **One row per reachable WRITE SITE** (§3), which is four and not two:
  `set_params`, and `display{side, dedicated}` in each of
  `apply_placement`'s **created**, **replacing** and **non-replacing**
  arms. A single row against one route is what would let another keep
  the defect — and rows per *call spelling* would have missed that one
  spelling reaches three different writes. The
  two bullets above cannot catch this — both establish their fallback
  state *before* `commit_to` is entered, so a preflight-snapshot design
  passes them.

  **Asserting only "document B was not replaced" is insufficient**, and
  revision 7's version of this test made exactly that mistake: it
  passes on a design that lets the body mutate freely and merely
  declines the final installation, leaving every other side effect
  behind. The refusal must land on the mutation, not on the outcome.
- **THE SAME WRITE-SITE ROWS, DRIVEN THROUGH A NESTED `commit_to`**
  (revision 9), in their own test: an outer `"panel"` commit whose body
  opens a nested **`"document"`** commit — a perfectly valid one, whose
  destination is captured fresh inside the outer body so it passes all
  four of its own checks and its callback really runs — and *that*
  callback attempts the dedication. Asserted: the attempt is **refused**,
  the slot is **still undedicated** afterwards, and the outer commit's
  destination is **intact** (its result lands in the panel; the user's
  newer document buffer survives). The bullet above cannot catch this —
  its mutation runs at commit depth 1, where revision 8's single-slot
  contract was the right one to read. Rows per write site rather than one
  row, because a fix that reinstated the outer contract for only one site
  would pass a single-row version.
- **ORDINARY NESTING STILL WORKS**, asserted rather than assumed: a
  nested `commit_to` that touches no dedication is accepted, its body
  runs, and its return value comes back through both frames. This is the
  pin against the other candidate fix — prohibiting nested `commit_to`
  outright — which would close the hole by forbidding a shape no rule
  objects to. Two further assertions, and the second is the one a
  `pop`-shaped fix gets wrong: the enclosing restriction is **back in
  force after the nested commit returns** (popped, not cleared), and
  **outside every commit dedication is ordinary again**, so the fix
  leaked no permanent restriction onto the editor.
- **THE CROSS-FRONTEND EXCEPTION IS PINNED POSITIVELY**, over **two**
  frontends: while an outer `"panel"` commit for A is in force, a nested
  commit for **B** dedicates **B's** side slot and is **allowed** — and
  B's slot is asserted really dedicated afterwards, not merely
  unrefused. The far side runs in the same test: A's slot is still
  undedicated and A's result still lands in A's panel, so this cannot
  pass by having weakened the restriction generally. **This is the one
  row asserting that something is permitted**; every other in the suite
  asserts a refusal, and without it, deleting the `fid` comparison —
  making any outer panel contract *globally* restrictive — passes the
  whole file, because both nesting rows above drive a single frontend.
  The exception is real and not a convenience: `resolve_placement`
  consults only the requesting frontend's `panel_capable` and its own
  one side window, so nothing done to B can change where A's side
  request lands.
- **A `"panel"` commit that really lands in the panel still skips
  checks 2–4** — otherwise the fix has quietly collapsed the two
  profiles into one and the parameterization buys nothing.
- **An unrecognized profile string is REFUSED**, with a message naming
  the accepted values — not silently treated as `"document"`.
- **An invalid-UTF-8 profile is refused by that SAME message.** Lua
  strings are byte strings, so a `string.char(255)` profile reaches
  `to_str()` and produces mlua's generic conversion error *before*
  the documented message is ever constructed — the same reachability
  class as the `Option<String>` defect, one layer deeper. Compare
  bytes, or map the conversion failure onto the message; asserted on
  content, in the bad-profile matrix beside the number and table rows.
- **A non-string profile (a number, a table) is refused by that SAME
  message**, asserted **on its content**, not merely that an error
  occurred. This is the bullet that fails if the argument is ever
  retyped to `Option<String>`: mlua would reject the value during
  conversion and the assertion on the message would stop matching. The
  test is therefore the guard on the type choice, not just on the
  behaviour.
- **An explicit `nil` profile takes the document profile**, identical
  to omitting it — witnessed separately, because a Lua caller threading
  an optional variable produces `nil` rather than absence, and a third
  behaviour there would be invisible until someone hit it.
- **Capture SUCCEEDS with no document window** (Q#DC-4), returning a
  destination whose document pair is absent — asserted as a successful
  capture, not as `nil`.
- **A panel-profile commit on that destination SUCCEEDS**, and a
  **document-profile commit on it is REFUSED** with a reason naming the
  missing document window. Both halves, because asserting only the
  refusal would pass on a capture that refuses everything.
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
