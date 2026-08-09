# Worker identity — Stage 1: what is running, and who asked for it

**Status: framing pass, revision 1. Pre-implementation. Awaiting
approval.**

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

- **A third-party job's own name is retained nowhere.**
  `pmacs.workers.dispatch(name, args, opts)`
  (`builtin/runtime/async.lua:369`) looks `name` up in a `handlers`
  table and calls it; the handler must itself call one of the builtin
  dispatchers, so the job records that builtin's `JobKind` and **`name`
  is discarded at the call**. The audit's "every third-party job renders
  under a builtin's label" is exact, and the fix is cheap: the name is
  in hand at the one place that throws it away.

- **`ProcessSpec` has one identity field and it is a convention**
  (`src/process.rs:193-235`): `label: String`, documented as
  "human-readable ... surfaced in events and the `pmacs.process.list`
  output". No owner, no purpose, no parent. Callers spell it however
  they like (`lsp:{name}`, a terminal buffer name).

And the two findings that actually shape the design:

- **A statusline provider API already exists, with three Lua adopters.**
  `pmacs.statusline.register` is live in `terminal.lua:477`,
  `syntax.lua:551` and `lsp.lua:1145`, taking
  `{ name, side, priority, face, fn(ctx) }` and returning a string or
  `nil`. An activity indicator is a **fourth registration**, not a new
  mechanism.

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

**Stage 1 (this lane): identity on the job and the process, and the
first indicator. NO WIRE CHANGE.**

- `owner` and `purpose` on `PendingJob`, carried through the single
  allocation funnel, and on `ProcessSpec` alongside the existing
  `label`.
- `pmacs.workers.dispatch` stops discarding the registered handler name.
- `*workers*` renders owner and purpose.
- **A statusline activity indicator** — the fourth provider
  registration, and the part a user feels on day one.

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
  ownership tree fall out of them."* Stage 1 takes owner and purpose.
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
  until Stage 2 — but it makes the label structured rather than
  conventional, which is the prerequisite.

## 5. Open questions

### Q#W-1 — how is identity supplied at the allocation funnel?

The existing shape is `allocate(kind, supersede, stream)` delegating to
`allocate_with_resource(kind, supersede, stream, resource)`. Adding two
more positional parameters gives a five-argument function and a
six-argument variant, and the next lane adds a seventh.

*My vote: **collapse the pair into one funnel taking a struct***, e.g.
`allocate(JobSpec { kind, supersede, stream, resource, identity })`,
with `JobSpec` carrying a `Default`-derived constructor so the ten
dispatchers read as named-field literals rather than positional soup.
Ten call sites plus `register_external` is a bounded, mechanical edit,
and it removes the `_with_resource` wart rather than adding beside it.

**The counter-argument, which is real:** this touches every dispatcher
in a lane whose subject is identity, which is scope the reviewer did not
ask for. **If review prefers the minimal edit**, the alternative is one
more parameter on the existing pair, and the collapse becomes its own
small lane. I would rather be told than assume.

### Q#W-2 — what IS an owner? **(the hard one)**

This is the question that decides whether the field is useful or
decorative, and I do not think it should be answered by whatever is
convenient at the call site.

Candidates: the **package** that registered the code (P3's
`CurrentlyLoadingPackage` signal already exists and §20 P3 names
owner-carrying registrations as its work unit); the **command** that
the user invoked; or the **subsystem** (lsp, syntax, git, compile).

*My vote: **`owner` is a package-or-builtin identity, `purpose` is the
human sentence.*** Concretely: `owner = "lsp"` / `purpose = "indexing
src/editor.rs"`. The reasons:

- It is the only one of the three that a **third party** can be
  attributed by, which is the whole point of attribution — a user
  wanting to know why their editor is busy is usually asking *whose
  code* is doing it.
- It aligns this arc with P3 rather than duplicating it. §20 says P3's
  ownership arc "unblocks ... package-scoped task cancellation in §9",
  so the two are meant to share a notion of owner.

**Named risk, stated rather than hidden:** P3 has not been built, so
Stage 1 populates `owner` from a **static per-subsystem constant** at
each dispatcher, not from a live package signal. That is honest for
builtins and gives third-party Lua nothing better than today until P3
lands. **If review thinks a field that third parties cannot populate is
premature, deferring `owner` and shipping only `purpose` is a coherent
smaller lane** — and it would still fix the indicator, which is the felt
part.

### Q#W-3 — what does the indicator actually show?

*My vote: **a count with the busiest purpose, and nothing when idle***
— e.g. `⋯2 lsp: indexing`, absent entirely at zero.

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

*My vote: **no**, and this is where I would most expect to be
overruled.* The audit names owner/purpose/**parent** together as the
prerequisite, so leaving one out is a deviation I should justify.

The justification: owner and purpose are **values a dispatcher already
knows** at the call site. A parent is not — it is whatever job is
*currently running* when a child is dispatched, which means either an
ambient context (a mechanism, with re-entrancy and cleanup failure
modes) or threading a parameter through every intermediate layer. A
`parent` field that nothing populates is worse than no field: it renders
as `None` everywhere and reads as "this job has no parent" rather than
"this system does not track parents".

Stage 3 builds the ambient and the field together, where the field can
be tested by a populated case.

### Q#W-6 — is any of this configurable?

*My vote: **one boolean, `ui.activity-indicator` (default `true`),
through `pmacs.config.define`.*** §11 grades the registry "partial
(foundation only)" and this document's sibling framings have both
resisted speculative settings — but a permanently-visible statusline
element is different in kind from an internal behaviour: it costs width
on every frame, and "I do not want this in my modeline" is a
preference someone will genuinely hold on day one rather than a
hypothetical. `git.enabled` and `ui.line-wrap` are the precedent shape.

No setting for owner/purpose capture itself — that is substrate, not
preference.

## 6. Verification

- **Every job carries an identity, asserted at the funnel, not per
  dispatcher.** The point of a single allocation site is that one
  assertion covers all ten dispatchers plus `register_external`; a test
  that checks three dispatchers individually would pass while a
  fourteenth added later carries nothing.
- **A `pmacs.workers.dispatch("name", ...)` job reports `"name"`**, not
  the builtin `JobKind` label underneath it — the exact defect §9 names,
  witnessed on a handler registered from Lua.
- **`register_external` jobs carry identity too** (MCP and LSP), since
  they bypass the worker pool entirely and are the ones most likely to
  be missed.
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
- **A spawned process carries structured owner/purpose alongside its
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
(Q#W-4), that cancellation can range over an owner (Stage 3), or that a
third-party package's own identity flows through (Q#W-2 — blocked on
P3).

Gates via `scripts/gate --acceptance <the new suite>`. **No
`--protocol`**: this lane has no wire change, which is the property that
lets it run beside the two lanes already in flight.

## 7. Not in scope

`Workspace` and `Location` fields (§7/§8 — the entities do not exist).
`parent`/`children` and the ownership tree (Stage 3, Q#W-5). Scoped
cancellation of any kind — cancel-all, by-kind, by-buffer, by-owner,
by-subtree (Stage 3; there is nothing to range over until identity
exists). The unified activity view joining the four planes (Stage 2).
Making terminal PTYs visible (Stage 2, Q#W-4). Widening `JobKind` or
making it open — third-party jobs are attributed by `owner`/`purpose`,
which is the point, and reopening a closed wire-adjacent enum is a
separate decision. Latency classes and resource budgets (§9 names them;
neither has a consumer yet). Supersession coverage — §9 notes parse jobs
and MCP requests pass `None`, which is a real defect and a **different**
one. P3's package-ownership signal (Q#W-2 depends on it and says so).
