# Worker identity — Stage 1: what is running, and what it is doing

*(Revision 1 was subtitled "and who asked for it". With `owner`
removed that title overclaimed the lane: it answers **what**, and —
under `pmacs.workers.dispatch` — **under which registered handler**.
Neither is who owns it.)*

**Status: revision 4, APPROVED 2026-08-09. IMPLEMENTED — see
`docs/active-work.md` for the commits, the gate outcome and the review
rounds.**

**Revision 4 scopes rule 1's claim to what it can actually enforce, and
takes Q#W-7 into this lane.** Revision 3 said the rule covered "all
yield points"; it covers **the two supported pmacs yield APIs**. Raw
`coroutine.yield` stays reachable — R46 is a convention, and the
scheduler diagnoses a non-Handle yield only *after* the coroutine has
suspended (`async.lua:197` resumes, `:212` inspects), so no refusal
sited in a yield helper can intercept it. The residual is named in §2
rather than papered over.

**Revision 3 closes a hole in revision 2's ambient: the extent it
called "synchronous" is not.** A registered handler is arbitrary Lua
and may `Handle:await()`, parking the coroutine with the name still
pushed so that unrelated later work inherits it. Rule 1 now **enforces**
non-yieldability rather than assuming it, following the guard this file
already carries for `pmacs.window.commit_to`. Scouting that guard
turned up a second supported yield API it does not cover — Q#W-7, a
pre-existing defect in another lane's invariant. Revision 3 reported it
rather than patching it in silence; **revision 4 fixes it here, on
approval**, since it is the same helper, the same invariant and the
same edit family.

**Revision 2 removes `owner` and respecifies the handler-name path,
after review found the first dishonest and the second unbuildable as
described.** `owner` populated from static per-subsystem constants is
an *origin*, not an owner, and would misattribute third-party work at
exactly the point §9 wants attribution. And "the name is in hand at the
one place that throws it away" was **wrong about the call chain** — it
is thrown away across three layers, one of which callers are documented
to bypass. Both re-scouted in the tree.

---

## 1. Why this, and why now

`COHERENCE.md` §9 grades the worker model **mechanism without
identity**, and §0 names **step 11 (background-work ownership)** as one
of the two remaining thin ends of the golden journey. §20 Priority 1 is
blunt about where that leaves things:

> **The remaining thin end is no longer inside this priority.** Step 1
> is install, which is **P8**; step 11 is background-work ownership,
> which is §9.

So this is the last of Priority 1's own journey, sitting in another
section's arc. Everything else P1 named has landed.

**The felt gap is smaller and sharper than the arc.** §9's audit ends
with a claim that is checkable, and I checked it:

> **No progress indicator exists anywhere** — no statusline spinner, no
> busy count.

`grep -c -i "spinner\|progress\|busy" src/statusline.rs` returns **0**.
So §3's promise of "visible asynchronous work" is **false today** unless
the user knows to run `M-x editor.list-workers`. Every build, LSP index,
grep, parse and — as of the lane merging beside this one — every `git
status` runs with no indication that anything is happening at all.

**And the git Stage 1 lane in flight right now makes it worse, by its
own admission.** `docs/git-integration-framing.md` Q#G-5 states it
plainly: git runs as a spawned process, spawned processes do not appear
in `*workers*`, and the lane therefore "adds a fifth thing that runs in
the background and is not attributable from one place". It accepted that
cost because these are short-lived reads. This lane is the one that
repays it.

## 2. Ground truth

Scouted in the tree, not recalled from the audit — and the audit has
drifted in one place, recorded below.

- **The audit's `PendingJob` field list is stale, and the drift is
  informative.** §9 lists seven fields; the struct
  (`src/async_runtime.rs:367-411`) carries **eight**. The addition is
  `resource: Option<ResourceOp>`, from dired Stage 2a — and **its doc
  comment cites `COHERENCE.md` §9 by name** as the reason it is a field
  on the job rather than a side map:

  > `COHERENCE.md` §9 is why this is a field on the job and not a side
  > map — the parse job→buffer link already lives in a side map and §9
  > names that as the defect.

  So the precedent for putting identity **on the job** is already set,
  already argued, and already merged. This lane extends a decision
  rather than introducing one.

- **There is a SINGLE allocation funnel, and that is what makes this
  tractable.** Every job in the system is born in `allocate`
  (`src/async_runtime.rs:746`), which delegates to
  `allocate_with_resource` (`:757`). The ten `dispatch_*` methods
  (`:803`–`:980`) and `register_external` (`:1011`, used by MCP and LSP)
  all pass through it. An identity field added there reaches every job
  by construction — there is no second birth site to miss.

- **The two-function split is itself a warning.** `allocate_with_resource`
  exists only because one prior lane needed one extra parameter. A
  second lane doing the same produces
  `allocate_with_resource_and_identity`, and a third produces something
  worse. This is the point to collapse it (Q#W-1).

- **`JobKind` is still a closed 12-variant enum**
  (`src/async_runtime.rs:305-343`) — Sleep, ComputeSum, EmitN, Grep,
  Parse, FsReadDir, FsStat, FsRename, FsChmod, FsRemove, McpRequest,
  LspRequest. Confirmed unchanged since the audit.

- **A third-party job's own name is retained nowhere, and recovering it
  is NOT cheap. Revision 1 said it was, and was wrong about the call
  chain.** The full path, read rather than assumed:

  ```
  pmacs.workers.dispatch(name, args, opts)   -- async.lua:369
    → handlers[name](args, opts)             -- arbitrary Lua
      → dispatch_grep(spec, opts)            -- Lua wrapper, :312
        → async_mod._dispatch_grep(spec, supersede_key(opts), max_batch)
          → the Rust binding → allocate()
  ```

  **`name` is not a parameter of any layer below the first.** The Rust
  dispatchers accept job arguments, a supersede key and stream data —
  nothing else. So revision 1's "change the allocation funnel and the
  name is recovered" is false: changing `allocate` gives the name
  nowhere to arrive *from*.

  **And the wrapper layer cannot be the capture point either.**
  `async.lua:337-345` deliberately exposes `pmacs.workers._new_handle` /
  `_new_stream` so that "other builtin runtime files (`pmacs.fs` in
  M8.1, future siblings) can construct handles for ids dispatched
  through **their own raw `_dispatch_*` primitives**". A handler that
  goes straight to `async_mod._dispatch_*` bypasses `dispatch_grep` and
  friends entirely — and those are precisely the callers doing
  non-standard work, i.e. the ones attribution is for.

  The audit's "every third-party job renders under a builtin's label"
  is exact. The mechanism that fixes it is Q#W-2, and it is a real
  mechanism, not a parameter.

- **`ProcessSpec` has one identity field and it is a convention**
  (`src/process.rs:193-235`): `label: String`, documented as
  "human-readable ... surfaced in events and the `pmacs.process.list`
  output". No owner, no purpose, no parent. Callers spell it however
  they like (`lsp:{name}`, a terminal buffer name).

- **A dynamic scope that must not be yielded out of ALREADY EXISTS
  here, guard and rationale included.** `Handle:await()` refuses to run
  inside `pmacs.window.commit_to` (`builtin/runtime/async.lua:87-90`),
  raising *"await: cannot await inside pmacs.window.commit_to; await
  first, then commit"*. Its comment states the hazard in general terms:
  yielding out of the extent "would restore the scope while this
  coroutine is still parked, so the rest of the commit would resume
  ambient". `commit_to` itself is "an RAII guard on the Rust stack" —
  the same shape this lane needs.

- **There are TWO SUPPORTED yield APIs, not one.** `Handle:await()`
  yields at `async.lua:95`; **`pmacs.async.yield_to_next_tick()` yields
  at `async.lua:244`** and is public (`pmacs.async` is `async_public`,
  `:247`). Any rule about a non-yieldable extent has to cover both. The
  `commit_to` guard covers only the first — see Q#W-7.

- **Raw `coroutine.yield` remains reachable, and NO guard of this shape
  can cover it.** R46 is a convention — *"package code uses `:await()`
  rather than `coroutine.yield`"* (`async.lua:26-27`) — not an
  enforcement. The scheduler does diagnose a non-Handle yield
  (`async.lua:217-223`, *"use Handle:await() per R46"*), **but only
  after the fact**: `step` calls `coroutine.resume(co)` at `:197` and
  inspects what came back at `:212`, by which point the coroutine has
  already suspended. A refusal placed in a yield helper is never
  consulted, and the enclosing `pmacs.workers.dispatch` never returns
  to run its pop.

  So the honest bound is: a package that violates R46 *inside* a
  dispatch-name scope can leak the name. It is not silent — the
  scheduler raises it through `pmacs.error` into `*errors*` — but the
  scope is not restored, and this framing does not claim otherwise.

And the two findings that actually shape the design:

- **A statusline provider API already exists, with three Lua adopters.**
  `pmacs.statusline.register` is live in `terminal.lua:477`,
  `syntax.lua:551` and `lsp.lua:1145`, taking
  `{ name, side, priority, face, fn(ctx) }` and returning a string or
  `nil`. An activity indicator is a **fourth registration**, not a new
  mechanism.

  **The three are named by FILE above and by NAME in the registry, and
  the two do not line up.** `syntax.lua` registers its provider as
  **`"mode"`** (it projects the major mode, `syntax.lua:552`), so the
  registry inventory reads `["mode", "terminal", "lsp"]` — which is what
  `tests/statusline_segments_acceptance.rs` asserts. Recorded because it
  is genuinely surprising: a reader looking for the syntax adopter by
  name does not find one. A fourth registration therefore changes that
  assertion, and where the new name sorts depends on **load order**, not
  on the name: `async.lua` is evaluated before `syntax.lua`,
  `terminal.lua` and `lsp.lua` (`src/editor.rs`), so a provider
  registered there lands first.

  **And it is evaluated per frame**: `evaluate_statusline` is called
  inside `paint_frame` (`src/editor.rs:4560`), before the long mutable
  core borrow. So an indicator updates while work is in flight without
  any new tick machinery — and, decisively for scheduling, **without
  touching the wire**. `EvaluatedStatuslineSegment` is already
  `Vec`-valued on an existing message; a fourth provider adds an element,
  not a variant.

- **`pmacs.process.list` deliberately hides terminal PTYs, and
  un-hiding them is NOT free.** The binding filters to
  `AnsiParserProfile::LineOriented`
  (`src/lua_bindings/mod.rs:8980-8984`). `git log -S` dates that filter
  to `bbc1f33 feat(vterm): add Stage 1 terminal core` — terminals were
  excluded on purpose.

  **Three acceptance suites use `#pmacs.process.list()` as a leak
  detector**: `tests/m6_8_multi_repl_acceptance.rs:385`/`:459` ("size
  must not grow across cycles"), `tests/compile_mode_acceptance.rs:133`/
  `:458` ("process list returns to baseline"), and
  `tests/lean4_stage1_acceptance.rs:327`/`:349`. **Removing the filter
  would inflate every one of those baselines by each open terminal.**

  This is why §9's "a terminal PTY appears in no user-visible activity
  view" is a real defect with a **non-obvious fix**, and why this lane
  does not casually widen the existing accessor (Q#W-4).

## 3. The staging, and why the line falls where it does

§9's full statement wants owner, workspace, buffer, parent, children,
latency class, cancellation scope, resource budget, execution location,
progress, and failure attribution. **Two of those cannot be built at
all right now**: `Workspace` is §7, graded *missing*, and `Location` is
§8, graded *missing (architecture ready)*. A lane that added
`workspace: Option<WorkspaceId>` would be adding a field typed on a
thing that does not exist.

**Stage 1 (this lane): a required `purpose` on the job and the process,
and the first indicator. NO WIRE CHANGE. NO `owner`.**

- **`purpose`, non-optional**, on `PendingJob`, carried through the
  single allocation funnel, and on `ProcessSpec` alongside the existing
  `label`.
- **A dispatch-identity ambient** so `pmacs.workers.dispatch` stops
  discarding the registered handler name (Q#W-2).
- `*workers*` renders `purpose`.
- **A statusline activity indicator** — the fourth provider
  registration, and the part a user feels on day one.

**`owner` is deliberately absent, and revision 1 was wrong to include
it.** The proposal was `owner = "lsp"` populated from a static
per-subsystem constant at each dispatcher. But a generic dispatcher has
no trustworthy knowledge of who invoked it, and `pmacs.process.spawn`
is callable by any package — so a static subsystem label is an
**origin or category, not an owner**, and it would confidently
misattribute third-party work to a builtin at exactly the point §9
wants attribution. A field that asserts a falsehood is worse than an
absent one: `*workers*` would *look* attributed while naming the wrong
party.

**Nor is it retained under a safer name.** Calling it `origin` or
`subsystem` would be honest, but a second string field sitting beside
`purpose` and grouping the view would be *adopted* as ownership by the
next reader regardless of its name — and it would squat on the slot
P3's real package signal has to fill. Stage 2 needs a grouping key; it
should get a real one, not a placeholder promoted by use.

**Stage 2 (separate lane): join the planes.** One activity view over
jobs, processes, LSP servers and terminals. This is what Stage 1's
identity is *for* — the audit's own conclusion is that "the four views
exist precisely because there is no common key to merge them on". It
also owns the terminal-visibility decision (Q#W-4), because that is a
question about the unified view, not about the accessor.

**Stage 3 (unscheduled): the tree and scoped cancellation.**
`parent`/`children`, and cancel-by-owner / by-buffer / by-subtree. This
needs an ambient "currently-running job" context so a child dispatched
inside a job can find its parent without every call site threading it —
a real mechanism with its own failure modes, and the reason parent is
**not** in Stage 1 (Q#W-5).

**Workspace and location are never this arc's**, at any stage. They
arrive from §7 and §8 and this arc consumes them.

**The line falls at the wire on purpose, and it is again a scheduling
decision.** The discovery Stage 2 lane holds the v22→v23 bump slot, and
git Stage 2 is already queued behind it. `PROTOCOL_VERSION` is a strict
serialization point. Stage 1 here touching no wire is what lets it run
beside both.

## 4. Coherence impact (§20)

- **§9 worker ownership — the direct target**, and specifically the
  audit's named prerequisite: *"Owner/purpose/parent fields on the job
  and process specs are the prerequisite; the unified view and the
  ownership tree fall out of them."* **Stage 1 takes ONE of the three
  — `purpose`.** `owner` waits for P3 to supply a package signal worth
  recording (§3); `parent` waits for Stage 3 (Q#W-5). Taking one of
  three named prerequisites is a deviation from the audit, and it is
  stated here rather than left to be noticed.
- **Journey step 11 — the direct target.** §0 names background-work
  ownership as one of two remaining thin ends. This does not close the
  step (Stage 2's unified view is most of that) but it is the first
  thing that makes work *visible*, which is what step 11 is about.
- **§3 zero-configuration state:** repairs a claim that is currently
  false. "Visible asynchronous work" becomes true by default, with no
  configuration and no command to know about.
- **Interaction islands (§6): none added.** The indicator is a
  statusline provider; it intercepts no keys and adds no precedence
  rung.
- **§14 workbench primitives: untouched.** `*workers*` already exists;
  this changes what it renders, not what renders it.
- **Config registry:** one setting at most, and my vote is a *visibility*
  toggle only (Q#W-6).
- **The debt this repays is named and dated.** `git-integration-framing.md`
  Q#G-5 recorded a deliberate negative §9 impact. This lane does not
  fully discharge it — a labelled process is still not in `*workers*`
  until Stage 2 — but it makes the process state *what it is doing* in
  a required field rather than a caller-spelled convention.
- **No P3 alignment is claimed.** Revision 1 argued this lane aligned
  with P3's ownership arc. With `owner` removed, it does not: P3 stays
  entirely ahead of it, and this lane deliberately leaves that slot
  empty rather than filling it with something P3 would have to displace.

## 5. Open questions

### Q#W-1 — how is identity supplied at the allocation funnel?

The existing shape is `allocate(kind, supersede, stream)` delegating to
`allocate_with_resource(kind, supersede, stream, resource)`. Adding two
more positional parameters gives a five-argument function and a
six-argument variant, and the next lane adds a seventh.

*My vote: **collapse the pair into one funnel taking a struct***, e.g.
`allocate(JobSpec { kind, supersede, stream, resource, purpose })`, so
the ten dispatchers read as named-field literals rather than positional
soup. Ten call sites plus `register_external` is a bounded, mechanical
edit, and it removes the `_with_resource` wart rather than adding
beside it.

**`JobSpec` is private, and `purpose` is non-optional.** Private
because the public dispatcher APIs should not grow a parameter every
time this arc adds a field; non-optional because that is what makes the
compiler, rather than a test, the thing that proves every caller
supplied one (§6). A `Default` impl would defeat exactly that, so
`purpose` is not defaulted even if other fields are.

**The counter-argument, which is real:** this touches every dispatcher
in a lane whose subject is identity, which is scope the reviewer did not
ask for. **If review prefers the minimal edit**, the alternative is one
more parameter on the existing pair, and the collapse becomes its own
small lane. I would rather be told than assume.

### Q#W-2 — the dispatch identity path **(rewritten in rev 2, rule 1 added in rev 3)**

Revision 1 treated this as a parameter-passing detail. §2 shows it is
not: `name` dies at `pmacs.workers.dispatch` and nothing below it takes
a name, so the value must be carried *out of band* across an arbitrary
handler.

**Revision 2 then called the extent "synchronous" and assumed it.
Review found that it is not.** A registered handler is arbitrary Lua
running inside `pmacs.async`, and it may call `Handle:await()` — a
legal, yieldable path that the existing tests already exercise inside
`pcall`. While a handler is parked, its pushed name **stays on the
stack**, and every tick callback and every other coroutine that
allocates a job in the meantime inherits it. That is not a corner case;
it is the ordinary shape of a handler that awaits.

So rule 1 below is no longer an observation about how handlers happen
to behave. It is an **enforced** property, and the enforcement already
has a precedent in this exact file (§2a).

**The capture point is Rust, not Lua**, and the reason is the bypass in
§2. If the ambient lived in the Lua wrapper layer, a handler calling
`async_mod._dispatch_*` directly — the documented pattern for runtime
files with their own primitives — would produce an unattributed job,
and those are the callers attribution exists for. Putting it in the
runtime means it is read at `allocate`, **the same single funnel Q#W-1
is already collapsing**. One mechanism, one site, no path around it.

*My vote: **a dispatch-name stack owned by the async runtime***, with
`pmacs.workers.dispatch` bracketing its handler call through two
runtime-internal bindings (`_push_dispatch_name` / `_pop_dispatch_name`).

**The contract, in full:**

1. **THE EXTENT IS NON-YIELDABLE, AND THAT IS ENFORCED, NOT ASSUMED.**
   Awaiting inside a dispatch-name scope is **refused**, because
   yielding would park the coroutine with the name still pushed and
   hand it to whatever allocates next.

   The guard is modelled on the one already in the file (§2):
   `_in_dispatch_name_scope()` joins `_in_commit_scope()` as a refusal
   in the same place, with the same shape of message and the same
   remedy — **await first, then dispatch**.

   Three details that decide whether the guard actually holds:

   - **It rejects BEFORE parking.** The `commit_to` guard is the first
     thing in `await`, ahead of the `_is_complete` check and the
     `coroutine.yield`. The new one sits beside it, for the same
     reason: a guard that fires after the yield has already happened
     guards nothing.
   - **It rejects UNCONDITIONALLY, not only when the handle is
     incomplete.** A guard that fires only when a yield would really
     occur has behaviour depending on whether the job happened to
     finish first — it would pass under test and fail in production,
     intermittently. `commit_to`'s guard is unconditional and this one
     matches it.
   - **It covers BOTH SUPPORTED YIELD APIs — and that is the exact
     extent of the claim.** `pmacs.async.yield_to_next_tick()`
     (`async.lua:243-245`) yields too, and is public, so it gets the
     same refusal; guarding only `await` would leave the hole open
     through a second door (and Q#W-7 is the proof that this happens,
     because `commit_to` has exactly that gap today).

     **What rule 1 does NOT cover is raw `coroutine.yield`** (§2).
     R46 forbids it to package code by convention only, and the
     scheduler's diagnostic fires *after* suspension, so no refusal
     sited in a yield helper can intercept it. Revision 3 said "all
     yield points" and was overclaiming. The property is: **the
     supported ways to yield are refused inside the scope; an R46
     violation can still leak the name, loudly.**
2. **Work dispatched later is NOT covered, deliberately.** A job
   dispatched from an `on_complete` callback or a resumed coroutine
   runs ticks later, outside the extent, and carries only its own
   `purpose`. Pretending otherwise would need the asynchronous
   lifetime mechanism this lane defers (Q#W-5).
3. **Nesting is a stack; innermost wins.** Handler `a` calling
   `pmacs.workers.dispatch("b", …)` gives jobs allocated inside `b` the
   name `b`, and restores `a` on return.
4. **Fan-out shares the name.** A handler dispatching five jobs
   produces five jobs named alike. They *were* all dispatched under it;
   that is the fact being recorded, not a collision.
5. **Unwind-safe, and this is the one that makes a naive version worse
   than none.** A handler that errors must still pop — otherwise one
   failure poisons every subsequent dispatch in the session with a
   stale name, and the feature silently starts lying. `pmacs.workers.
   dispatch` runs the handler under `pcall`, pops, and rethrows.
6. **Precedence over a caller-supplied purpose: COMPOSE, do not
   replace.** Where the dispatch site supplied its own purpose, the
   recorded value is `"<name>: <purpose>"`; where it did not, the
   recorded value is `"<name>"`. Replacing would recreate blocker 1 in
   a new place — `dispatch_grep` supplies `"grep: …"`, and letting that
   win would lose the third party again, while letting the name win
   would discard the only description of the actual work. Composition
   is capped at the innermost name by rule 3, so no unbounded chain.
7. **Outside any extent, nothing changes.** A builtin invoked directly
   records its own `purpose`.

**A known and accepted property, stated rather than discovered later:**
the ambient captures *causal* extent, not *intent*. If a handler
triggers unrelated work within its extent — an edit that schedules a
parse — that job takes the name. Because rule 1 refuses both supported
yield APIs, that window is bounded by a single un-parked call for any
caller obeying R46, and within such a window I think "this ran because
that handler ran" is the honest reading. (A caller violating R46 is
outside this property, and outside rule 1 — §2.) It is also the only definition enforceable at a single
funnel. **If review disagrees, the alternative is
capture-at-the-Lua-wrapper**, which is narrower and misses the raw
`_dispatch_*` callers — a trade of false positives for false negatives,
and I would rather over-attribute inside a bounded call than silently
drop the third-party case.

**Why this ambient is admissible while Q#W-5's is not.** They are not
the same mechanism — **and revision 2 was entitled to that claim only
after rule 1 made it true.** As written in revision 2 the extent could
be parked by any awaiting handler, which is most of the way to the
asynchronous lifetime I used as the reason for deferring `parent`.
With rule 1 the difference is real and enforced: this is a
single-threaded dynamic extent that **cannot** be suspended, with a
deterministic pop on both the normal and the error path. A `parent`
ambient must span a job's asynchronous lifetime by design — across
ticks, through callbacks that run after the parent settled — and cannot
be fixed by refusing to yield, because yielding is the whole point. The
first is a stack; the second is a lifetime model.

### Q#W-3 — what does the indicator actually show?

*My vote: **a count plus the oldest in-flight job's `purpose`, and
nothing when idle*** — e.g. `⋯2 lsp: indexing`, absent entirely at
zero. With `owner` gone (§3) `purpose` is the only identity there is,
which is also why it is required rather than optional.

**Oldest, not newest or "busiest".** Revision 1 said "busiest", which
is not a defined quantity — jobs carry no cost estimate. Oldest is
computable from `dispatched_at`, which `PendingJob` already has, and it
answers the question a user actually asks of a stuck editor: *what is
taking so long?*

- **Absent at zero, not `0 jobs`.** A statusline segment that is always
  present costs width forever to say "nothing is happening". The
  existing providers already return `nil` to render nothing
  (`lsp.lua:1156`), so this is the established idiom.
- **A count, not a spinner.** A spinner needs an animation frame clock
  and says only "something"; a count says how much. Per-frame evaluation
  makes either possible, so this is a product choice, not a constraint.
- **Not names plural.** One purpose keeps it to a bounded width; the
  full list is what `*workers*` is for.

### Q#W-4 — do terminal PTYs become visible in Stage 1?

**No — and the reason is evidence, not caution.** `pmacs.process.list`
filters to `LineOriented`, and three acceptance suites assert on
`#pmacs.process.list()` as a leak baseline (§2). Widening that accessor
would inflate all three with every open terminal, and "fix the tests"
is the wrong response to a test that is correctly detecting a semantic
change.

*My vote: **leave the accessor alone in Stage 1**, and let Stage 2's
unified view introduce a **separate** enumeration that includes PTYs.*
The leak detectors keep asserting what they were written to assert; the
new surface answers the new question. Two accessors with different
contracts is better than one accessor whose meaning silently changed
under its existing callers.

### Q#W-5 — does `parent` belong in Stage 1?

*My vote: **no.*** The audit names owner/purpose/**parent** together as
the prerequisite, and after revision 2 this lane takes only `purpose` —
so both omissions need justifying, not just this one. `owner`'s is in
§3; `parent`'s is here.

`purpose` is a **value the dispatcher already knows** at the call site.
A parent is not — it is whatever job is *currently running* when a
child is dispatched. A `parent` field that nothing populates is worse
than no field: it renders as `None` everywhere and reads as "this job
has no parent" rather than "this system does not track parents".

**And the objection this has to answer, since the lane now builds an
ambient of its own (Q#W-2):** why is one admissible and not the other?
Because Q#W-2's extent **cannot be suspended** — rule 1 refuses both
yield points, so it is bounded by one un-parked call with a
deterministic pop on the normal and the error path.

**That distinction is only load-bearing because rule 1 exists.**
Revision 2 asserted this same paragraph while its ambient *could* be
parked by any awaiting handler, which made the two mechanisms far more
alike than the argument admitted. The honest version: a `parent`
ambient must identify the running job *across ticks* — a job dispatched
from an `on_complete` callback should name the job whose completion
fired it, and that callback runs after the parent settled, outside any
dispatch call. Refusing to yield cannot rescue it, because yielding is
the mechanism it needs. That is a lifetime model, not a stack, and it
is Stage 3's subject rather than a field this lane can add cheaply.

Stage 3 builds the lifetime model and the field together, where the
field can be tested by a populated case.

### Q#W-7 — the same hole exists in `commit_to` today — **RESOLVED, fixed here (rev 4)**

Found while scouting rule 1, and reported rather than quietly patched.

`Handle:await()` refuses to run inside `pmacs.window.commit_to`
(`async.lua:87-90`) precisely so a coroutine cannot park with the
frontend scope pushed. **But `pmacs.async.yield_to_next_tick()`
(`async.lua:243-245`) also yields, is public, and carries no such
refusal.** A coroutine inside `commit_to` can therefore park through
that door and produce exactly the misrouting the `await` guard exists
to prevent. Journey Stage 1a's Q#JR14b invariant has a second entrance.

I have **not** verified that a real caller does this — the reachability
of the bug is unproven, and I would rather say so than dress a
code-reading up as a repro.

**RESOLVED — approved for this lane.** It is the same supported yield
helper, the same invariant, and the same `async.lua` edit family;
splitting it would preserve a known hole without reducing integration
risk. So `yield_to_next_tick` gains **both** refusals — the new
`_in_dispatch_name_scope()` and the missing `_in_commit_scope()` — and
the `commit_to` gap closes in the same commit as rule 1.

**Its witnesses are the same pair as rule 1's, not a smoke test:** the
refusal fires, **and** the commit scope is restored afterwards. A guard
that raises while leaving the scope pushed converts a silent misrouting
into a noisy one and fixes nothing.

Reachability by a real caller stays **unproven** — this is a defect
found by reading, and the tests pin the guard rather than reproducing a
user-visible bug. That distinction belongs in the commit message too,
so nobody later cites this as evidence the bug was observed.

### Q#W-6 — is any of this configurable?

*My vote: **one boolean, `ui.activity-indicator` (default `true`),
through `pmacs.config.define`.*** §11 grades the registry "partial
(foundation only)" and this document's sibling framings have both
resisted speculative settings — but a permanently-visible statusline
element is different in kind from an internal behaviour: it costs width
on every frame, and "I do not want this in my modeline" is a
preference someone will genuinely hold on day one rather than a
hypothetical. `git.enabled` and `ui.line-wrap` are the precedent shape.

No setting for `purpose` capture itself — that is substrate, not
preference.

## 6. Verification

- **Presence is enforced by the COMPILER, not by a test.** `purpose` is
  non-optional in `JobSpec`, so a dispatcher that supplies none does not
  build. Revision 1 claimed a single funnel assertion proved "every job
  carries an identity"; **it does not** — a funnel test proves the
  funnel stores what it was handed, and says nothing about whether
  fourteen callers handed it anything meaningful. Presence is a type
  obligation; the tests below are for *semantics*.
- **Representative entry paths assert the semantics**, one per distinct
  shape rather than one per dispatcher: a pool dispatcher, an
  `register_external` job (MCP/LSP bypass the worker pool entirely and
  are the likeliest to be missed), and a spawned process.
- **A `pmacs.workers.dispatch("name", …)` job reports `"name"`**, and
  the witness is **a handler registered from Lua that calls a real
  dispatcher** — not a synthetic funnel test. A test that pushes the
  ambient by hand proves the stack works and leaves the actual defect
  (`name` dying in an arbitrary handler) unwitnessed.
- **Awaiting inside a handler is REFUSED, and the scope restores after
  the refusal** (Q#W-2 rule 1). Two assertions, and the second is the
  load-bearing one: a guard that raises but leaves the name pushed has
  converted a silent misattribution into a silent misattribution plus
  an error. The witness dispatches again after the rejection and
  asserts the new job carries **no** stale name.
- **`pmacs.async.yield_to_next_tick()` inside a handler is refused
  too**, with the same restore-after assertion. Guarding one supported
  yield API and not the other leaves the hole open through a second
  door (§2).
- **`yield_to_next_tick` inside `pmacs.window.commit_to` is refused,
  and the commit scope restores after the refusal** (Q#W-7) — the
  pre-existing gap, closed here. Both halves asserted, for the same
  reason as rule 1's: a refusal that leaves the scope pushed has
  swapped a silent fault for a loud one.
- **NOT asserted, and deliberately: that a raw `coroutine.yield`
  inside either scope is prevented.** It is not (§2). Writing a test
  that "proves" coverage this design does not have would be worse than
  the gap, and the gap is recorded instead.
- **The refusal fires even when the awaited handle is already
  complete** (rule 1) — the case that separates an unconditional guard
  from one whose behaviour depends on a race.
- **The ambient survives a failing handler** (Q#W-2 rule 5): a handler
  that errors, then a subsequent unrelated dispatch, asserting the
  second job does **not** carry the first's name. This is the
  regression that would otherwise appear as intermittent
  misattribution long after the lane lands.
- **Nesting and fan-out** (rules 3–4): a handler dispatching two jobs
  gives both its name; a handler dispatching through another registered
  handler gives the inner jobs the inner name and restores the outer.
- **Composition, not replacement** (rule 6): a handler calling a
  dispatcher that supplies its own purpose yields `"<name>: <purpose>"`
  — asserted for both halves, since a test on the prefix alone passes
  when the description is dropped.
- **Work dispatched from an `on_complete` callback carries no handler
  name** (rule 2) — the boundary of the extent, asserted deliberately
  so it reads as designed rather than broken.
- **The statusline shows nothing at idle**, asserted as *absent
  segment*, not as empty string — a zero-width segment still consumes a
  separator.
- **The statusline shows a count while work is in flight**, witnessed
  through the real per-frame evaluation path (`paint_frame`), not by
  calling the provider function directly. A provider that works in
  isolation and never gets evaluated is the failure this must exclude.
- **The indicator honours `ui.activity-indicator = false`** (Q#W-6),
  witnessed as an absent segment with work genuinely in flight — the
  case that separates "disabled" from "idle".
- **`#pmacs.process.list()` is UNCHANGED for every existing caller**
  (Q#W-4). The three leak-detector suites
  (`m6_8_multi_repl_acceptance`, `compile_mode_acceptance`,
  `lean4_stage1_acceptance`) are the assertion, and they must pass
  untouched. **If any of them needs editing, the design is wrong**, and
  that is the signal to stop rather than to adjust a baseline.
- **A spawned process carries a required `purpose` alongside its
  existing `label`**, and **`label`'s current callers keep working
  unchanged** — `lsp:{name}` and terminal buffer names are live
  conventions with existing consumers.
- **Both frontends render the segment**, since it rides the existing
  `StatuslineSegments` path — asserted for the grid TUI and
  `pmacs-gpu`, because "it is on an existing message" is a claim about
  the producer and says nothing about whether a consumer draws it.

**What this will NOT prove:** that background work is attributable from
one place (that is Stage 2's unified view — this lane makes it
*possible*, not *done*), that a terminal PTY is visible anywhere
(Q#W-4), that cancellation can range over an owner (Stage 3), or **that
any job is attributed to the PACKAGE responsible for it** — `purpose`
records what work is being done and, under `pmacs.workers.dispatch`,
which registered handler it ran under. Neither is package ownership,
which waits for P3 (§3).

Gates via `scripts/gate --acceptance <the new suite>`. **No
`--protocol`**: this lane has no wire change, which is the property that
lets it run beside the two lanes already in flight.

## 7. Not in scope

**Making raw `coroutine.yield` safe inside either dynamic scope** (§2,
rule 1). R46 forbids it by convention and the scheduler diagnoses it
after the fact; closing it properly means enforcement the runtime does
not have, and this lane claims only the two supported yield APIs.
**`owner`, in any spelling** — including `origin` or `subsystem` (§3).
The slot stays empty until P3 can fill it with a package signal;
nothing in this lane may be promoted into it later by use.
`Workspace` and `Location` fields (§7/§8 — the entities do not exist).
`parent`/`children` and the ownership tree (Stage 3, Q#W-5). Scoped
cancellation of any kind — cancel-all, by-kind, by-buffer, by-owner,
by-subtree (Stage 3; there is nothing to range over until identity
exists). The unified activity view joining the four planes (Stage 2).
Making terminal PTYs visible (Stage 2, Q#W-4). Widening `JobKind` or
making it open — third-party jobs are described by `purpose`, which is
the point, and reopening a closed wire-adjacent enum is a separate
decision. Latency classes and resource budgets (§9 names them;
neither has a consumer yet). Supersession coverage — §9 notes parse jobs
and MCP requests pass `None`, which is a real defect and a **different**
one. P3's package-ownership signal — §3 defers `owner` to it and makes
no claim of alignment with it.
