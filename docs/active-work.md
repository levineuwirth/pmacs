# Active work — cross-machine resume ledger

**Snapshot: 2026-08-01.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

**Updated later the same day, on a new machine.** Development moved to
the laptop; the recovery path in "Repository authority" below was
exercised from this checkout and the `githubsucks` alias was absent and
had to be added, exactly as that section anticipates. **One lane opened:
CI CRDT coverage**, which had been sitting under "NEEDS A LANE" with no
branch and no owner since #166. It is implemented on
`ci-crdt-coverage` and its block replaces the old one below.

**Updated 2026-08-05.** One lane opened: **macOS CI signal integrity**
(#215, in review), which this file required a lane for and did not have
until review caught it — the #171 defect recurring. Its block is below.

**Updated later the same day.** #215 **merged** (`main` @ `12f2970`) and
that lane is **rewritten, not removed** — rule 4 removes a lane when its
ARC is done, and Stage 2 is the arc. Stage 2 (**hardening**) ran on
`ci-signal-hardening`, with its own lane block, its own checkpoint
table, and a lane written **before** the PR was opened rather than
after review asked for it. **It merged as #216 on 2026-08-05**, which
completes the arc: R2 and R4 are retired with discriminating
witnesses.

**The lane is kept, not yet removed, and that is a deferral rather than
a judgement.** Rule 4 would remove it now — but its residue must be
re-homed first, or removing it loses the state: **R1** is referred to
the async-runtime lane (Q#MCI3) and **R3** is an unresolved possible
product defect owned by the process-signal / PTY lane, neither of which
has a block here yet. Retiring this arc and opening those two is an
**absorption pass**, and doing it inside an unrelated feature PR is how
a ledger acquires edits nobody reviewed.

**Updated 2026-08-04.** Four PRs landed since: the CI CRDT coverage
lane #209, Distribution Stage 1 #211 (released as **v1.1.0**), the
post-release accuracy pass #212, and **bottom-panel Stage 3 #213 —
which completes Arc 7**. Its lane is **removed** per rule 4: the arc is
done *and* its durable facts are in `docs/agent-handoff.md` §1. The
distribution and CI-CRDT lanes are rewritten rather than removed,
because each keeps named follow-ons.

**This snapshot is an absorption pass, taken with ZERO open PRs** —
the one window in which a ledger refresh has nothing to re-conflict
with, and taken deliberately before a machine move. Nine PRs landed
since the previous anchor (#199–#207). Two arcs completed and their
lanes are **removed**, their durable facts now in
`docs/agent-handoff.md` §1: **Journey Stage 1** (1a plus the whole 1b
split) and **test ambient-root isolation**. Discovery and reap-ledger
merged a stage each and keep lanes rewritten to what remains.
`docs/agent-handoff.md` §1a now carries the whole board — every arc,
open lane, deferred item and standing hazard in one view.

*(Earlier note, retained because its reasoning still governs:)*
**This snapshot is an absorption pass.** Eight PRs merged on 2026-07-29
and 2026-07-30 (#188, #190, #191, #194, #195, #196, #197, #198) and the
ledger had drifted to 1,854 lines carrying six lanes whose work was
already on `main`. Those six are removed and their load-bearing
decisions are in `docs/agent-handoff.md` §1 — rule 4's precondition,
satisfied rather than deferred. The file is now 609 lines.

**A lane is removed when its ARC is done, not when a PR merges.** Two
lanes survive their sub-stage merges and are rewritten to the remaining
plan rather than deleted: generated-buffer immutability (Stage 1 merged,
Stage 2 not started) and bottom-panel (Stage 2 complete, Stage 3 ahead).
The bottom-panel block said so in its own text — *"this lane is not
removed at 2B-3's merge"* — and a wholesale removal keyed on "the PR
merged" would have discarded live planning. Read each block before
cutting it.

**#188's lane arrived with #188**, which is the point: with several PRs
open, a lane written on `main` for work that lands elsewhere
re-conflicts on every merge. Written on its own branch it costs one
conflict, at the merge that would have happened anyway.

The PTY terminate
diagnostic (#176) was the last lane retained past its merge — retained
because rule 4 removes a
merged lane only *after* its durable facts reach
`docs/agent-handoff.md`, and that absorption was unowned. The
2026-07-28 snapshot
owned it: #176's facts are now in the handoff (§1's arc bullet and §5's
two ops lessons about ticking observers and proving child exit), so its
lane is gone. The Lean 4, GPU-terminal-input, inline-math (#172), dired
(#169), and terminal config + copy mode lanes were removed the same way
— the last of these was #180's work, folded into #182 so two open PRs
would stop re-conflicting in this file.

**Trust the canonical-base line below over any lane header**: if a PR
number appears in `git log --first-parent githubsucks/main`, it has
landed regardless of what a lane says.

**Two open PRs had no lane here at all before the 2026-07-28
snapshot** — #174 and #171. An open PR is exactly the volatile work this
file exists to record, so its absence is a ledger defect rather than a
tidy omission: #171 drifted **153 commits** while invisible here, and
its still-green old CI run described a tree nobody had looked at since.
**When a PR is opened, give it a lane.** All three have since merged —
#174, #171 and #186 — so per rule 4 their lanes are gone again and
their durable facts are in `docs/agent-handoff.md` (§5 for #174's
lesson, §1 for the two framings).

## Repository authority

- Canonical development URL:
  `https://github.com/levineuwirth/pmacs.git`. This ledger uses the
  normalized local alias `githubsucks` so its refs and recovery commands
  are identical on every machine. Remote names are otherwise
  machine-local: `origin` may name this canonical URL, a release mirror,
  or something else, and therefore has no authority by name alone.
- Canonical base at this snapshot: **`githubsucks/main` @ `9a26ac8`** —
  GPU horizontal scroll **#223**, which **closes the QoL arc**, atop
  `2b56d16` TUI horizontal scroll **#222**, `02f3ec3` `ui.line-wrap`
  **#221** (protocol v22), `218d2e7` GUI zoom **#220** and `da56bec`
  `full_grid` **#219**. Beneath those, `db1bbe9`:
  the tree primitive **#217**, atop `2657568` the macOS CI
  signal-integrity **Stage 2 #216** (which retired R2 and R4), atop
  `12f2970` its Stage 1 registry **#215**, atop `f186253`:
  bottom-panel Stage 3 **#213**, which completes Arc 7, atop the
  post-release accuracy pass **#212**, Distribution Stage 1 **#211**
  (released as **v1.1.0**, the first release with prebuilt binaries),
  and the CI CRDT coverage lane **#209**. Beneath those, `cfc1710`:
  discovery Stage 1 #207, the ambient-root isolation implementation
  #206, Journey Stage 1b-3 #205, 1b-2 #204 and 1b-1 #203, the
  reap-ledger diagnostic #202, the isolation framing #201, the
  process-signal diagnostic #200 and the ledger absorption #199.
  **Protocol schema support is `v6..=v22`; the production server-first
  `Hello` still advertises v20** — two different facts, and #184 landed
  only the first. The upper bound moved to **v22 at #221**, which added
  `InstanceMessage::LineWrapFacts`; `ADVERTISED_PROTOCOL_VERSION` did
  not move and must not be edited to chase it. Verified against
  `pmacs-protocol/src/message.rs`, not carried forward: this line said
  `v6..=v21` for two merges after that stopped being true.
  **Historical `v21` statements elsewhere in this file are correct
  where they describe a stage as it landed** — only this
  current-state paragraph tracks the live range.
  **The recovery floor advances with the base**, so the check below
  now requires `9a26ac8` or newer; a tree at `db1bbe9` no longer
  passes — it would lack the entire QoL arc, which this file and the
  handoff both describe as complete. That is deliberate — a check accepting an older commit than
  the declared base passes on a tree the rest of this file does not
  describe.
  **Lanes below that name an older base have not been re-based; derive
  their integration surface from `git diff <their base>..main`.**
- On the transfer source, `origin/main` named a release mirror at
  `d3fa632` and lagged badly. On the current destination, `origin` names
  the canonical URL. This difference is why all recovery begins by
  verifying URLs and normalizing `githubsucks` rather than trusting
  `origin/main`.
- The shared desktop checkout contained unrelated uncommitted work. The
  branches below were prepared in isolated worktrees; never clean or
  overwrite the shared checkout to recover them.

Start on another machine by inspecting its remotes:

```sh
git remote -v
git remote get-url githubsucks
```

If the second command says the alias is absent, add it; if it prints a
different URL, stop and resolve that collision rather than overwriting an
unknown remote:

```sh
git remote add githubsucks https://github.com/levineuwirth/pmacs.git
```

Then recover current refs:

```sh
git fetch githubsucks --prune
git log -1 --oneline githubsucks/main
git worktree list
git status --short --branch
```

The `git log` command must expose `9a26ac8` — the base named above — or a
newer intentional main. Keep this threshold and the canonical-base line in
step: a recovery check that accepts an older commit than the base it
declares canonical will pass on a tree the rest of this file does not
describe.
If it does not, stop and repair the remote/fetch configuration.

**This path was exercised, not asserted, at this snapshot** — re-run
from an empty directory on 2026-08-08 when the base advanced to
`9a26ac8`, rather than having its SHA swapped. That distinction is the
whole point of this paragraph: advancing the base is exactly when the
recovery commands are most likely to have rotted, and a swapped SHA
reads identically to a verified one. `git clone` the canonical URL, add
the `githubsucks` alias, `git fetch githubsucks --prune`, confirm
`9a26ac8` is an ancestor of `githubsucks/main`, and recover with the
three-argument `git worktree add <path> -b <local> githubsucks/<branch>`
form. All four steps ran clean.

**Correction, found by re-running it.** This file claimed the
two-argument form fails for a remote-only branch with `fatal: invalid
reference`. **It does not fail.** On git 2.55.0 it *succeeds* and
leaves a **detached HEAD** — no branch, no upstream, `git status`
reporting `## HEAD (no branch)`.

Still use `-b`, but for the opposite reason to the one recorded: the
danger is not an error that stops you, it is that nothing stops you.
Work committed in that worktree sits on no branch, is not pushed by a
bare `git push`, and is exactly the "uncommitted work does not travel"
hazard in a shape that looks committed. **A documented error message
that never appears is worse than no documentation**, because the reader
waits for a signal that is not coming.

## `scripts/gate` — PR #225 OPEN (build tooling)

**PR #225** — https://github.com/levineuwirth/pmacs/pull/225. Written
**after** the PR existed, again, and again because review asked. Two
lanes in a row have now been added late; the correction from #171 and
#215 is not sticking, and recording that is more useful than a
back-dated block that pretends it did.

- **Branch `gate-script`**, base `githubsucks/main` @ `b833b13` (the
  #224 merge). **`githubsucks/gate-script` is the authoritative tip** —
  the ref, not a SHA. Recover with
  `git fetch githubsucks && git checkout gate-script`.
- **Framing `docs/gate-script-framing.md`**, approved at revision 4
  after four review rounds; revision 5 records two safety defects found
  against the implementation, not the design.
- **Scope:** `scripts/gate` (per-worktree `CARGO_TARGET_DIR`, five
  ambient roots, durable per-gate logs, the fixed gate suite),
  `tests/gate_script_acceptance.rs`, and a handoff §3 rewrite pointing
  at the script while §3 keeps policy and acceptance-suite selection.
  No `src/`, no crate, no manifest, no protocol.
- **Verification:** 15 acceptance tests over the no-gates paths, each
  isolated by `PMACS_GATE_TARGET_ROOT`. Mutation-tested; the two prune
  guards are redundant by design and only fail the test when **both**
  are removed, which is recorded in the test itself. Observed real runs
  confirm failed-gate naming, log paths, ambient creation and reaping,
  and distinct log directories per run.

**GATE STATUS — R8 RESOLVED.** This lane was blocked because
`scripts/gate` exited 1 on a clean tree: **R8** failed `m4_acceptance`
and therefore the sweep. That was never a footnote — #225 is the lane
that makes the gate suite authoritative, and a tool shipping with its
own gate red teaches the opposite of what it exists to teach.

**R8 was fixed and retired in #226** (`dcb852e`), which bounded the LSP
fixture's project detection. This branch is rebased onto it.

**RE-GATED 2026-08-09: `scripts/gate` exits 0.** All nine gates green in
one command — fmt, clippy, `--lib`, `--lib --features crdt`, both named
acceptance suites, `m4_acceptance`, `-p pmacs-gpu`, and the full
workspace sweep. That is #225's own acceptance criterion, and it is the
first time the tool has passed the suite it exists to run.

**Rebase resolution, per the standing rule that #226's R8 documentation
wins.** Three conflicts, all in R8 text this branch had written while
the row was still an open investigation: two in
`docs/ci-red-signatures.md` (both resolved to #226's retired row, this
branch's pre-fix copy dropped), and the framing-doc pair
(`e71e1bd` added it, `7cfba73` removed it — both **skipped**, since they
are net-zero here and `main` owns the file authoritatively; replaying
the second would have deleted `main`'s copy). Two now-stale lanes were
also removed: this branch's "R8 NEEDS A LANE" investigation block, and
#226's own lane, which Rule 4 retires now that it has merged — its
durable facts are in the retired registry row and the handoff §6
census.

## Worker identity Stage 1 (§9) — IMPLEMENTED, no PR yet

**Written with the lane's first commit**, per the standing correction
from #171 and #215.

**Branch `worker-identity-stage1`**, base `githubsucks/main` @
`4bc55e8` (the #225 merge). **`githubsucks/worker-identity-stage1` is
the authoritative tip** — the ref, not a SHA. Recover with
`git fetch githubsucks && git checkout worker-identity-stage1`.

- **Framing `docs/worker-identity-framing.md`, revision 4, APPROVED
  2026-08-09** after four review rounds.
  Scope: `COHERENCE.md` §9's "mechanism without identity", and journey
  step 11 — the last of Priority 1's own work, sitting in another
  section's arc.
- **Revision 2 took two blockers.** `owner` is **removed entirely**:
  populated from static per-subsystem constants it is an origin, not an
  owner, and would misattribute third-party work at the exact point §9
  wants attribution. It is not retained under a safer name either —
  `origin`/`subsystem` would be adopted as ownership by use and would
  squat on the slot P3 must fill. And the handler-name recovery was
  **respecified as a mechanism**: revision 1 claimed the name was "in
  hand at the one place that throws it away", which was wrong about the
  call chain (`dispatch` → arbitrary handler → Lua wrapper → Rust
  binding, with the wrapper layer documented as bypassable).
- **Revision 3 took a third blocker: the ambient's extent is not
  synchronous.** A handler may `Handle:await()` and park with the name
  still pushed, leaking attribution to unrelated later work. Rule 1 now
  **enforces** non-yieldability, modelled on the existing
  `_in_commit_scope()` refusal in `Handle:await`
  (`builtin/runtime/async.lua:87-90`) — rejecting before the park,
  unconditionally rather than only when a yield would occur, and
  covering **both** yield points.
- **Q#W-7 — a pre-existing defect found while scouting that guard, and
  APPROVED for repair in this lane.** `pmacs.async.yield_to_next_tick()`
  (`async.lua:243-245`) is public, yields, and carries **no**
  `_in_commit_scope` refusal — so Journey Stage 1a's Q#JR14b invariant
  has a second entrance. Same helper, same invariant, same edit family,
  so splitting it would have preserved a known hole without reducing
  integration risk. **Reachability by a real caller is UNPROVEN** — the
  defect was found by reading, and the tests pin the guard rather than
  reproducing a user-visible bug. That belongs in the commit message so
  nobody later cites this as an observed failure.
- **Revision 4 also scoped rule 1's claim to what it enforces.**
  Revision 3 said "all yield points"; it covers **the two supported
  pmacs yield APIs**. Raw `coroutine.yield` stays reachable — R46 is a
  convention, and the scheduler diagnoses a non-Handle yield only after
  the coroutine has suspended (`async.lua:197` resumes, `:212`
  inspects), so no refusal in a yield helper can intercept it. Recorded
  as a residual, and explicitly **not** covered by a test that would
  imply otherwise.
- **NO WIRE CHANGE**, which is what lets this run beside the two lanes
  already in flight. The statusline activity indicator is a **fourth**
  `pmacs.statusline.register` provider (terminal/syntax/lsp are the
  three existing adopters), evaluated per frame inside `paint_frame`
  (`src/editor.rs:4560`) and riding the existing `StatuslineSegments`
  vector. No variant, no bump.
- **Scope:** a **required** `purpose` on `PendingJob` and `ProcessSpec`
  through the single allocation funnel (`src/async_runtime.rs:746`,
  which every dispatcher and `register_external` passes through), a
  runtime-owned dispatch-name ambient recovering the handler name that
  `pmacs.workers.dispatch` currently discards, the `*workers*`
  rendering, and the indicator. Non-optional so the **compiler**, not a
  test, proves every caller supplied one.
- **Two scouting findings that shaped the design**, both verified:
  `PendingJob` carries **eight** fields, not the audit's seven, and the
  eighth's doc comment **cites §9 by name** as the reason identity
  belongs on the job rather than in a side map — so this extends a
  merged decision. And **`pmacs.process.list` filters to
  `LineOriented`** (`src/lua_bindings/mod.rs:8980`), with **three
  acceptance suites using `#pmacs.process.list()` as a leak detector**,
  so making terminal PTYs visible is deferred to Stage 2 with a
  separate accessor rather than by widening this one.
- **Deliberate deviation from the audit, flagged for review:** §9 names
  owner/**purpose**/parent together as the prerequisite; Stage 1 takes
  **only `purpose`** — one of the three, not two. `owner` was removed in
  revision 2: nothing in the runtime knows which package asked for a
  job, so an `owner` field could only have been filled with the same
  handler name `purpose` already carries, and an empty one reads as
  "unowned" rather than "not tracked". `parent` is out for the matching
  reason — it needs an ambient "currently-running job" context, and an
  unpopulated `parent` reads as "no parent" rather than "not tracked"
  (Q#W-5). The package-ownership slot stays **deliberately empty** until
  P3 can fill it with a real signal (framing §3, §7).
- **Gates:** `scripts/gate --acceptance worker_identity_acceptance
  --acceptance journey_acceptance --acceptance
  statusline_segments_acceptance --acceptance compile_mode_acceptance
  --acceptance m8_6_acceptance`. No `--protocol` — no wire change.
  `compile_mode` and `m8_6` joined at review round 1, which moved their
  spawn call sites; `m8_6` covers the `pmacs-magit` fixture, and a newly
  required field is exactly the kind of change that breaks a package
  fixture quietly.
- **IMPLEMENTED at `1aca0ee`**, with review round 1's blocker fixed at
  `2162737` and review round 2's three findings at `6661125`.
  `tests/worker_identity_acceptance.rs` is the new suite: **24 tests**,
  plus one consumer-side witness beside the private renderer in
  `pmacs-gpu`.
- **`journey_acceptance` passed UNTOUCHED (47/47)** — the stop signal
  did not fire. Q#W-7 edits the `commit_to` guard family, so any of its
  established pins needing an edit would have meant this altered Journey
  Stage 1a's semantics rather than closing a gap in them. Its diff
  versus `main` is empty, and so is the diff for all three
  `#pmacs.process.list()` leak-detector suites
  (`m6_8_multi_repl_acceptance`, `compile_mode_acceptance`,
  `lean4_stage1_acceptance`) — Q#W-4's preservation claim, checked the
  way the framing asked.
- **One pre-existing assertion did change, and it is an inventory
  rather than a contract**: `statusline_segments_acceptance`'s builtin
  provider list becomes `["activity", "mode", "terminal", "lsp"]`.
  `activity` sorts first because `async.lua` is loaded before
  `syntax.lua`, `terminal.lua` and `lsp.lua`. That assertion exists to
  grow when a builtin provider is added; it is listed here so the change
  is not mistaken for an accommodation.
- **23 mutation checks, each test falsified by removing its own fix.**
  The ones worth naming: siting the `await` guard *inside* the
  `_is_complete` branch (the already-complete case then slips through —
  which is the whole reason the guard is unconditional); replacing
  `pcall`/pop/rethrow with a bare handler call (a raising handler leaves
  the name pushed and the *next* dispatch inherits it); composing
  `"<name>"` instead of `"<name>: <purpose>"` and vice versa (each half
  passes the other's test); `first()` instead of `last()` on the name
  stack; oldest→newest in `activity_summary`; and, on the GPU side,
  painting an unthemed modeline face as the band colour, which would
  have made the indicator invisible without failing anything else.
  One of the twenty is a **preservation** check rather than a new
  claim: bracketing `pmacs.workers.dispatch` with
  `local ok, result = pcall(...)` truncates a handler that returns more
  than one value, which every other test in the suite tolerates. Round
  1 added three more against the spawn refusal: restoring the
  label fallback, accepting an empty/whitespace-only purpose, and
  reading the field non-raw so a metatable can smuggle one in.
- **Two residuals, stated rather than tested around.** Raw
  `coroutine.yield` inside either dynamic scope still leaks the scope —
  loudly, through `pmacs.error`, but it leaks; no refusal sited in a
  yield helper can intercept it (framing §2). And Q#W-7's reachability
  by a real caller stays **unproven**: the commit message says so, and
  the test pins the guard rather than reproducing a fault.
- **Review round 1 blocker — `pmacs.process.spawn` now REQUIRES
  `purpose`.** The first implementation made it optional at the Lua
  surface, falling back to `label`. That preserved compatibility and
  delivered nothing: §9's complaint about `ProcessSpec` is exactly that
  `label` is "caller-supplied, unvalidated convention", so a purpose
  defaulting to it hands every caller back the convention the lane exists
  to replace. Refused on five shapes — absent, empty, whitespace-only,
  wrong type, metatable-provided — each asserting the process list is
  unchanged, since a validation that rejects after spawning has already
  done the thing it rejected.
- **That is a BREAKING CHANGE to a public Lua API, taken now on
  purpose.** §10 grades extension trust "missing (one class)" and P7
  package lifecycle has not started, so the third-party population is
  ~zero and the cost only rises later. Checked for a reason that would be
  wrong and found none: `pmacs.process.spawn` has no API-reference
  documentation and no stability promise in `docs/` (the package-author
  guide's only mentions are an audit-rule classification and a pointer to
  the bundled REPL; its semver language governs packages' own versioning,
  not pmacs's Lua surface), and `lua_to_spec` has exactly one caller.
  **Eleven executable call sites updated**, each with a real description
  rather than the label copied across: `repl/init.lua`, `compile.lua`,
  `lean.lua`, the `pmacs-magit` fixture, and seven in tests. The two
  `pmacs.process.spawn("ls")` occurrences in `src/audit/mod.rs` and
  `tests/m7_9_acceptance.rs` are **audit fixture source text** — lexed,
  never executed — and are deliberately untouched.
- **Review round 2 — the display-text boundary, fixed at `6661125`.**
  Three findings, and the fix is deliberately different in each place
  because the constraint is.
  - **P2a: invalid UTF-8 bypassed the `purpose` diagnostic.**
    `required_purpose` read the field with `value.to_str()?`; Lua strings
    are BYTE strings, so `purpose = string.char(255)` surfaced mlua's
    generic conversion error before this lane's own message existed. It
    refused before spawning, so nothing leaked — the defect was the
    message. **Third occurrence of this class in the project** (the
    destination-capture lane corrected the same shape two rounds ago), so
    the whole diff was audited for it: exactly one more,
    `_push_dispatch_name` taking `name: String`, now `mlua::String` with
    an owned diagnostic. Those two are the only Lua-string reads this
    lane added; every other binding it adds takes `()`. The remaining
    `pmacs.process.spawn` fields (`label`, `command`, `args`, `env`,
    `cwd`) still convert generically — **pre-existing, untouched, and
    named here rather than silently inherited.**
  - **P2b, half one: handler names are refused at the source.**
    `pmacs.workers.register` type-checked and nothing more, which was
    fine while the name died inside `dispatch`. It no longer dies there,
    so the name now gets `purpose`'s meaningful-value standard plus
    control characters.
  - **P2b, half two: purposes are ESCAPED at presentation, not rejected
    at the registry — consistent with the `#228` decision.** A purpose
    may legitimately contain a newline (a path can; `pmacs-magit`'s spawn
    purpose is an argv), so the one-line constraint belongs to the
    surface that has one row. `purpose_for_one_row` states the property
    it exists for — **a row must not be able to forge another row** —
    escapes the Unicode `Cc` class (so ESC cannot open a terminal
    sequence either), borrows unchanged when there is nothing to escape
    (byte-identity is structural, not asserted), and does **not** escape
    backslashes: no number of them makes a second row, and doubling them
    would cost byte-identity for ordinary text. Two callers: the
    `*workers*` rows and `ActivitySummary`, which exists for one consumer
    with exactly one row. `pmacs.workers.snapshot()` is the
    `describe-command` of this lane and stays raw — asserted, so a clip
    that deleted the text everywhere would fail rather than pass.
  - **P3: two stale recovery summaries**, both fixed section-locally —
    the framing doc's "Implementation may proceed", and this file's claim
    that Stage 1 took the "first two" of owner/purpose/parent. It takes
    **one**: `owner` was removed in revision 2, and the claim that
    argument overturned was still standing here.
  - **Seven more mutation checks, each failing its own test and no
    other** (30 for the lane): the two UTF-8 diagnostics, the two
    register guards, the two escaping call sites, and
    `purpose_for_one_row` neutered to the identity — which fails both
    surfaces' tests and nothing else, since it is the shared helper.
  - **All 13 gate steps green at `6661125`** (log
    `20260809T173314Z-1552101`): lib 1920, lib-crdt 2105,
    worker_identity 24, journey **47/47 UNTOUCHED**, statusline 7,
    compile_mode 73, m8_6 12, m4 151, gpu 242. The three
    `#pmacs.process.list()` leak detectors and `journey_acceptance` are
    **byte-identical to `main`** in round 2 — the stop signals did not
    fire, and round 2 edited no test outside its own suite. **The
    preceding run of the same command was red on three tests and none of
    them was this diff's** — R7 for the third time plus two wall-clock
    budget tests; recorded in `docs/ci-red-signatures.md` rather than
    re-run away silently.
- **Review round 3 — a diagnostic that named the wrong surface, fixed
  at `b2e8efd`.** `required_purpose`'s invalid-UTF-8 refusal told the
  caller their process purpose "is displayed to the user in `*workers*`
  and in the modeline". **Neither is a process surface.** Stage 1
  deliberately keeps processes out of both (Q#W-4, framing §3) — a
  process's purpose is exposed through `pmacs.process.list` and nothing
  else — so the message sent the reader looking for their process in two
  places it will never appear. The refusal itself is correct and stays:
  a purpose with no display form anywhere is still refused.
  - **The two UTF-8 refusals now name different surfaces, because they
    reach different ones.** The job-side twin (`_push_dispatch_name`)
    legitimately names `*workers*` and the modeline — a handler name is
    composed into a job's purpose, and a job does render in both — so it
    was made to say so explicitly rather than left at the vaguer "as
    part of every job's purpose", which named no surface at all and
    would have made the divergence unassertable.
  - **A new test asserts both directions, positive and negative**
    (`the_two_utf8_refusals_each_name_the_surface_their_own_text_reaches`,
    26 in the suite): the process message contains `pmacs.process.list`
    and **not** `*workers*`/`modeline`; the job message contains both of
    those and **not** `pmacs.process.list`. The existing row-table
    assertion in `spawning_without_a_real_purpose_is_refused_and_starts_nothing`
    now runs as far as the surface name too. Without the negative half a
    later "unify the wording" edit reintroduces exactly one wrong
    sentence and passes everything else.
  - **Three mutation checks, each red on its own claim:** restoring the
    old process wording fails both content assertions; collapsing the
    job message onto the process wording fails only the new test (which
    is the point — the old job test asserted the prefix alone); and
    restoring the job message's original vague wording fails it too.
  - **The rustdoc carried the same defect risk and was fixed with it** —
    `required_purpose` now states which surface it names and why not the
    other two, and the `_push_dispatch_name` comment states the
    converse. A string literal corrected while its doc comment still
    argues the other way is one refactor from reverting itself.
  - **Gate: 12 of 13 steps green; step `12-sweep` red on a wall-clock
    render-budget test, twice, on a DIFFERENT test each run** (logs
    `20260809T195332Z-2113672` and `20260809T200120Z-2427128`, load
    average 12.9/23.9 with sibling lanes building). All three pass in
    isolated reruns; the diff is two string literals, their doc comments
    and one test, and touches no render path. Recorded as **U5** in
    `docs/ci-red-signatures.md` rather than re-run away silently.
    `journey_acceptance` **47/47 UNTOUCHED** and the three
    `#pmacs.process.list()` leak detectors unedited — the stop signals
    did not fire.
- **Surfaces that changed shape, for anyone rebasing onto this:**
  `AsyncRuntime::allocate`/`allocate_with_resource` collapsed into one
  private `JobSpec`-taking funnel; `register_external` grew a third
  parameter; `ProcessSpec::new` grew a third parameter (~40 call sites,
  nearly all tests); `ActiveJobInfo`/`CompletedJobInfo`/`ProcessSpec`
  each grew a required `purpose` field, and `pmacs.process.spawn`
  requires `purpose` in its spec table.

## Discovery Stage 2 — PR #228 OPEN, **MERGE-BLOCKED**

**PR #228** — https://github.com/levineuwirth/pmacs/pull/228. Opened
2026-08-09 at `2d298dd`. **Open for review, not for merge.**

**The block is a gate-integrity problem, not backlog hygiene.** This
lane's gate is `scripts/gate --protocol`, which promises the CRDT
workspace sweep. That sweep's documented precondition is
`cargo build --workspace --no-default-features --features luajit,crdt`
(handoff §5), and **the script does not run it** — confirmed by reading
its plan emitter. On a fresh per-worktree target directory the sweep
fails on twelve `gpu_invocation_acceptance` tests missing the
`pmacs-gpu` binary, so a `--protocol` result can be decided by the
state of the build directory rather than by the diff.

Latent until #225 gave each worktree its own target dir — a shared one
usually already had `pmacs-gpu` built, satisfying the precondition by
accident. It surfaced on this branch's first gate run.

**Unblocking requires both:** the `scripts/gate` repair, in its own
narrow framing and its own PR (explicitly **not** folded into this
feature branch), and then a **fresh-target rerun of this branch's
protocol gate** under the repaired script.

**Written with the lane's first commit**, per the standing correction
from #171 and #215.

**Branch `discovery-stage2`**, base `githubsucks/main` @ `4bc55e8`
(the #225 merge). **`githubsucks/discovery-stage2` is the authoritative
tip** — the ref, not a SHA. Recover with
`git fetch githubsucks && git checkout discovery-stage2`.

- **Framing `docs/discovery-stage2-framing.md`, revision 3, APPROVED
  2026-08-09** after three review rounds. Each round found the previous
  one reasoning about a mechanism instead of reading it — an in-place
  field change that postcard cannot make compatible, a TUI that never
  reads the message at all, a round-trip test that freezes nothing, a
  cache hazard the per-peer render state makes impossible, and a
  clipping rule unachievable at narrow widths.
  Scope: `COHERENCE.md` §5's "M-x rows are still bare names".
  Descriptions already exist on `Command` and are already rendered by
  `help.list-commands`; they are missing at the one moment they would
  change a decision.
- **PROTOCOL BUMP v22 → v23, and this lane HOLDS THE BUMP SLOT.**
  Additive: a new `MinibufferPromptRows` variant **appended** to the
  enum, with `MinibufferPrompt` **frozen** for v12–v22. An in-place
  field change is a wire break — postcard encodes positionally, and
  that variant is sent to every peer `>= 12` (`src/daemon.rs:1472`).
- **Git Stage 2 (gutter markers) also needs a bump and must wait for
  this to land.** Git Stage 1 is no-wire and runs beside it.
- **Two halves, only one of which is wire work.** `pmacs-gpu` renders
  the new variant. **The grid TUI never reads `MinibufferPrompt` at
  all** — it paints from `core.minibuffer` and renders
  `format!("  [{cand}]")` (`src/editor.rs:5484`), so its half is a
  local formatting change reading the registry directly. A multi-row
  TUI chooser is explicitly NOT this lane.
- **Gates:** `scripts/gate --protocol --acceptance
  discovery_stage2_acceptance --acceptance m9_6_acceptance --acceptance
  m9_7_acceptance --acceptance m9_8_acceptance` — the strengthened
  two-configuration sweep, which is what `--protocol` exists for. The
  three m9 suites are named because the PR #228 review round measured
  them as this change's blast radius (see the description-clip bullet);
  their continued passing is on the record rather than assumed.
  **`--protocol` does NOT run its own documented precondition**
  (`cargo build --workspace --no-default-features --features
  luajit,crdt`, handoff §5) — run it by hand first or twelve
  `gpu_invocation_acceptance` tests fail on a missing `pmacs-gpu`
  binary. That omission is the `gate-protocol-build` lane's, not this
  one's.
- **IMPLEMENTED.** `PROTOCOL_VERSION` is 23,
  `ADVERTISED_PROTOCOL_VERSION` is untouched at 20. New suite
  `tests/discovery_stage2_acceptance.rs`; the daemon half is
  `crdt`-gated (a semantic session is necessarily a text replica) and
  runs one daemon serving a v22 and a v23 session simultaneously.
- **Multi-line descriptions are clipped AT THE SURFACE, and
  registration-level rejection was investigated and REJECTED ON
  EVIDENCE — do not re-propose it.** PR #228 review found the real
  hazard: the GPU dropdown derives its height, visible window and
  highlight offset from `rows.len()` (one logical row per candidate),
  so a detail carrying a line break misaligns every row below it; the
  TUI writes into a single-row band. The obvious fix — reject CR/LF in
  `CommandRegistry::define` — was implemented and measured, and it
  **fails 36 tests across `m9_6`/`m9_7`/`m9_8`**, because MCP tool
  registration renders a whole schema block into `description`
  (`tests/fixtures/pmacs-mcp-tools/init.lua:272`,
  `table.concat(lines, "\n")`, used at `:496`) and
  **`tests/m9_6_acceptance.rs:583-598` asserts four separate lines of
  it** — tool text, `Arguments:`, and two per-argument lines. No
  single-line rendering satisfies those assertions, so a registry guard
  could only go green by deleting a shipped acceptance criterion.
  The one-line constraint belongs to the surfaces that have it:
  `Command::description_first_line` clips, both single-row consumers
  call it, and the full text still reaches `describe-command` /
  `help.list-commands` untouched. Precedent already in-tree — the same
  MCP fixture clips a tool RESULT to its first line because *"a
  multi-line set_status would corrupt the row layout"* (`:277-285`).
  **A startup census is not a corpus census**: booting an
  `EditorState` and scanning all 180 registered descriptions found zero
  offenders, because MCP registers at RUNTIME and builds the string by
  concatenation — invisible to both that census and a grep for literals.
  The workspace sweep is what caught it.
- **The freeze is enforced by LITERAL byte fixtures**, not a round-trip
  — `minibuffer_prompt_v12_wire_bytes_are_frozen` in `src/protocol.rs`,
  the first such fixture in this repo. Bite-verified: reordering two
  fields of `MinibufferPrompt` leaves
  `minibuffer_prompt_round_trips_through_postcard` **passing** and fails
  the fixture, which is exactly the hazard a round-trip cannot see.
- **Version assertions updated (five, each read before editing):**
  `src/protocol.rs` — the `PROTOCOL_VERSION == 22` tripwire (renamed
  `protocol_version_is_twenty_three_for_minibuffer_prompt_rows`) and
  `supported_protocol_versions_resume_ladder_on_v6_floor`'s
  accepted/rejected ranges; `tests/statusline_segments_acceptance.rs`
  (version + supported range + the `!supported` ceiling);
  `tests/bottom_panel_stage2b_gpu_acceptance.rs`;
  `tests/vterm_stage3_acceptance.rs`. **No `ADVERTISED_PROTOCOL_VERSION`
  assertion fired**, which is the pin doing its job.
- **No cross-version cache test, deliberately** (framing §3.2/§6):
  `SemanticRenderState::for_peer` bakes the negotiated version in at
  attach and is dropped at detach, so a cache cannot span two versions.
  A test for an impossible condition passes forever while teaching the
  next reader that the hazard is real.

## LSP LaTeX coverage — IMPLEMENTED, gates green, no PR yet

**Written with the lane's first commit**, per the standing correction
from #171 and #215.

**Branch `lsp-latex-coverage`**, base `githubsucks/main` @ `4bc55e8`
(the #225 merge). **`githubsucks/lsp-latex-coverage` is the
authoritative tip** — the ref, not a SHA. Recover with
`git fetch githubsucks && git checkout lsp-latex-coverage`.

- **Framing `docs/lsp-language-coverage-framing.md`, revision 3 —
  IMPLEMENTATION AUTHORIZED 2026-08-09**, after a summary of its four
  corrections rather than a findings round on the document itself.
  Recorded that way deliberately: the §3 `.texlabroot` verification
  caveat was live and binding, and was step zero of the work rather
  than a footnote it could be read past. **It is now discharged — see
  below.** **Revision 1 was UNTRACKED on `main` in one checkout** and
  therefore did not travel; committing it here is the fix.
- **Scope: one `pmacs.lsp.config.latex` entry plus its root resolver.**
  `texlab` 5.25.1 is installed and unused; a `.tex` buffer highlights
  correctly and offers no completion, diagnostics, or go-to-definition.
- **Revision 2 found Slice 1 is SMALLER than revision 1 framed.** The
  proposed `.tex`/`.latex`/`.sty`/`.cls` filetype mappings are
  redundant: the grammar already carries exactly those extensions
  (`src/syntax.rs:1111`), grammar-extension detection sits **ahead** of
  the LSP filetype map in the precedence chain
  (`docs/latex-grammar-math-substrate-framing.md:166-171`), and
  `lsp.lua:267-270` calls that map "mainly the LSP-only fallback". The
  two systems cannot disagree, because the grammar's extension list is
  what drives detection.
- **Two other corrections.** `haskell-language-server` **is** installed
  on this machine — revision 1 said it was not, which was the whole
  basis of its Slice 1 / Slice 2 split. And Q#LX3's deferral argument
  read `COHERENCE.md:1669` ("first slice in flight") when `:124` and
  `:867` both record multi-root affinity as **merged (#161)**; that
  line contradicts the same document twice and wants a separate fix.
- **Q#LX2 (the LaTeX root) is answered.** An upward marker walk through
  `config.latex.root`, which already accepts a resolver function
  (`lsp.lua:543`), falling back to the file's own directory.
  **`.git` is deliberately excluded**: a repo root is the wrong answer
  for LaTeX, and it is the one place copying the other fourteen
  entries' instinct is actively wrong.
- **STEP ZERO IS DISCHARGED — §3's `.texlabroot` caveat, by
  observation.** Marker 1 **ships**, and the framing's premise for it
  was corrected in the process.
  - **`.texlabroot` is a real texlab marker.** texlab v5.25.1's
    `crates/distro/src/language.rs` maps `.texlabroot`/`texlabroot` →
    Root, `Tectonic.toml` → Tectonic, `.latexmkrc`/`latexmkrc` →
    Latexmkrc; `ProjectRoot::walk_and_find`
    (`crates/base-db/src/deps/root.rs`) walks ancestors testing all
    three, innermost wins. The shipped marker set is **texlab's own**,
    including the bare `texlabroot`/`latexmkrc` spellings the framing
    did not list.
  - **But texlab cannot apply that walk to fix a root pmacs gets
    wrong**, which is the correction that matters. Each arm searches
    `workspace.iter()` — documents ALREADY LOADED — and the workspace
    comes from the folders the CLIENT supplies. Hand-driven LSP
    sessions confirmed it: with `rootUri` at a `chapters/`
    subdirectory, no marker above it (`.texlabroot` included) widened
    texlab's view and its dependency graph never reached the parent
    document; with `rootUri` at the marker directory the parent
    resolved, marker present or not. **texlab honours the root it is
    handed and never corrects a too-narrow one**, so what
    `config.latex.root` returns *is* the project scope. That makes the
    resolver the whole value of the lane rather than a nicety.
  - **`args = {}` is also observed**, not assumed: bare `texlab`
    answers `initialize` with `TexLab 5.25.1` over stdio, so the `run`
    subcommand is not needed.
  - **§3 said the wrong thing and has been corrected — `b5eaf27` IS
    revision 3.** It framed marker 1 as conditional on texlab honouring
    the `.texlabroot` *file*, when the operative fact is that texlab
    honours the *client-supplied root* and never widens it. The caveat
    was discharged by observation, and revision 3 records what that
    established. Nothing about §3 is outstanding.
- **`.git` exclusion needed more than omitting it from the list.**
  `project_root_for` falls through to `pmacs.project.detect` when a
  resolver returns nil, and **that** walk includes `.git` — so a
  resolver declining on a markerless file would hand texlab the
  repository root by the back door. The resolver therefore never
  declines for a file that has a directory. Pinned end to end through
  attach, with the same fixture asserting the shared detector really
  would have answered the repo root.
- **Commit `a9ef37f`** — `builtin/runtime/lsp.lua` plus
  `tests/lsp_latex_acceptance.rs` (14 tests, one per §6 bullet plus the
  boundary and decline cases). No `settings`/`init_options` (Q#LX1); no
  filetype mappings (§2, asserted both ways).
- **Gates: ALL GREEN** via
  `./scripts/gate --acceptance lsp_latex_acceptance` — fmt, clippy,
  lib, lib-crdt, the new suite, m4, gpu, the workspace sweep (115
  suites, zero failures), diff-check. No `--protocol` — a config entry,
  no wire.
- **Seven mutations each fail the suite**: resolver declining on no
  marker (6 tests), no marker walk (4), a redundant `filetypes.tex`
  (1), boundary ignored (1), `io.open` truthiness so a directory counts
  as a marker (1), marker set narrowed (4), command renamed with
  opinionated settings added (1).
- **The boundary has now been the interesting part twice, and the
  second time it was a real defect (fixed in review).** First it was
  hermeticity — every fixture sets `set_search_boundary` at its own
  tempdir because R8's shape (a stray `latexmkrc` above the tempdir)
  would make the markerless assertions pass while testing nothing.
  Then review found `latex_within_boundary` answering a PATH question
  with string arithmetic: `dir:sub(1, #boundary + 1) == boundary .. "/"`
  compares against `"//"` when the boundary is `/`, which no canonical
  path matches, so a root boundary judged **every** ancestor out of
  bounds, ran no marker walk at all, and gave each chapter of a thesis
  its own server — the lane's headline behaviour silently off, with
  every shipped test still green because each one clamps to a tempdir.
  The same trap sat at the other end (`/` was never a walk candidate,
  and `/paper.tex` sliced to an empty directory and declined into the
  `.git`-aware detector). Now segment comparison throughout: the root
  is a boundary with zero segments, contained by construction rather
  than by a special case. Pinned by an ATTACH-level test under a `/`
  boundary — two chapters, one server, marker root — and the
  hermeticity property asserts **both** directions, since "stops at the
  boundary" is also satisfied by a walk that never runs. Suite is 16
  tests. **A reader
  deciding whether to trust this resolver should read it as: the marker
  set and the `.git` exclusion were settled by observation and are
  solid; the boundary arithmetic around them was not, and is the place
  to look first if roots come back wrong.**
- **Trap for the next agent in this worktree:** this machine exports a
  shared `CARGO_TARGET_DIR`, so a bare `cargo test` compiles against a
  sibling worktree's artifacts and fails with errors from code that is
  not in this tree. Use `scripts/gate`, or
  `CARGO_TARGET_DIR="$(./scripts/gate --print-target-dir)"` for ad-hoc
  runs. `scripts/gate`'s own header documents this; the failure looks
  like a broken branch, which is why it is recorded here.
- **No PR opened**, by instruction.

## `scripts/gate --protocol` build step — **MERGED as #229** (`7cf4653`)

**MERGED as PR #229** — https://github.com/levineuwirth/pmacs/pull/229,
at `3b10f9d`, 14/14 CI green including both macOS legs. `main` is now
`7cf4653`. *(This lane still awaits Rule 4 retirement — its durable
facts belong in the handoff before the entry is removed. Corrected here
only because the previous text said "Held, not merged", which the merge
falsified; the retirement itself is not this lane's work.)*

**History, retained:** opened at `93d557a`. Its first CI run went red on
`Test (macos-latest / lua54)`; the rerun turned that selector green and
went red on a **different** one. Both are recorded as **U4** and **U5**
in `docs/ci-red-signatures.md`, as separate incidents per the matching
rule rather than one signature twice.

**Registry numbering conflict, expected at merge.**
`worker-identity-stage1` independently defines its own **U4** and **U5**
on its branch. This lane merges first, so on `main` the rows above are
U4/U5 and **worker identity must renumber its pair on rebase**. Flagged
here because a rebase that resolves the textual conflict without
renumbering produces two different incidents sharing an id, which is
exactly the failure the registry's matching rule exists to prevent.

**Written with the lane's first commit**, per the standing correction
from #171 and #215.

**Branch `gate-protocol-build`**, base `githubsucks/main` @ `4bc55e8`
(the #225 merge). **`githubsucks/gate-protocol-build` is the
authoritative tip** — the ref, not a SHA. Recover with
`git fetch githubsucks && git checkout gate-protocol-build`.

- **Framing `docs/gate-protocol-build-framing.md`, revision 5.** The
  fix itself is implemented and green at `49bc141`; **its regression
  witness landed separately at `677fd25`**, after review found the
  original witness did not reach the step it named. Narrow by design:
  one missing step in one script, plus the boundary question that let
  it go missing. No `src/`, no protocol, no feature work.
- **WAS THE OPEN BLOCKER — the witnesses did not reach the step they
  name. CLOSED at `677fd25`.** `--print-plan` **strips names** before
  printing, so the ordering assertion saw only commands; `--self-test`
  **hardcodes** `build-crdt` inside its own synthetic plan. Review
  demonstrated the consequence: **renaming the real build step to
  `sweep-crdt` left both tests passing.** So the lane had shipped
  without the regression guard it was created to provide. §7 requires
  **both** real emitter pairs — `build-crdt` and `sweep-crdt`, name
  *and* exact command — because the hole is symmetric and revision 4
  closed only half of it. The synthetic `--self-test` stays: it
  witnesses the *runner* (failure naming, `FAILED:` list, log paths,
  non-zero exit, and continuation via the sentinel), which is a
  different thing from attributing the real step, and it may no longer
  stand in for it. **What closed it is the "THE WITNESS DID NOT REACH
  THE STEP" bullet further down** — `--print-plan-named`, with all four
  renames and drifts mutated red.
- **The defect, as found.** `--protocol` adds the CRDT workspace sweep,
  whose documented precondition is `cargo build --workspace
  --no-default-features --features luajit,crdt` — documented in handoff
  **§5** at the time, **§3** now. The plan emitter had **no build step
  at all** — read from the source, not inferred from the failure.
- **Why it was latent, and why that makes it urgent rather than tidy.**
  Before #225 every worktree shared one `CARGO_TARGET_DIR`, which
  almost always already held a `pmacs-gpu` binary, so the precondition
  was satisfied **by accident**. Per-worktree target dirs start empty.
  The hazard is not the red gate that stops you — it is a **green**
  `--protocol` run whose crdt sweep was decided by the state of the
  build directory rather than by the diff. That is a gate reporting
  coverage it does not have, which is what #225 exists to prevent.
- **Observed on PR #228's first gate run:** twelve
  `gpu_invocation_acceptance::crdt::*` failures, all *"build pmacs-gpu
  before this acceptance suite"*, with `debug/pmacs-gpu` absent.
- **The durable half is a boundary question.** `scripts/gate`'s header
  names handoff **§3** as the owner of its reasoning, and this
  precondition lives in **§5** — a coherent cause for the omission, not
  mere oversight.

  **Resolved in framing revision 2: §3 becomes the SOLE normative home,
  §5 keeps the incident and its signature as history, and the script's
  header keeps citing §3 ALONE.** Revision 1 proposed citing both,
  which splits one executable contract across two homes and weakens the
  script's only clean boundary at the same time as Q#GR-4 declines to
  build any automated check for prose drift. This entry recorded that
  superseded decision until now; a recovering machine reading the stale
  version would have rebuilt revision 1's wrong boundary.
- **Q#GR-1 — SETTLED BY OBSERVATION, 2026-08-09, before any fix was
  written.** On a **disposable** target directory (never a live lane's),
  with `debug/pmacs-gpu` asserted **absent before each run** as a
  recorded precondition, each sweep run **alone** from that same cold
  state so neither could have built the binary for the other:

  | sweep | exit | result | `pmacs-gpu` after |
  |---|---|---|---|
  | default | **0** | green, 114 test targets | **still absent** |
  | crdt | **101** | exactly **12** failures, all `gpu_invocation_acceptance::crdt::*`, all *"build pmacs-gpu before this acceptance suite"* | still absent |

  So framing §3's inference **holds** and §4's *"only under
  `--protocol`"* is correct — the default sweep never builds the binary
  and never needs it. **Mechanism, now established rather than
  guessed:** `pmacs-gpu` has no `tests/` directory, so cargo never
  uplifts its bin to `debug/pmacs-gpu`; only an explicit `cargo build`
  produces it.

  **Found while doing it, and worse than the twelve:**
  `bottom_panel_stage2b_gpu` a54 reported **`ok`** in that cold crdt
  sweep. Its only path that does not spawn `pmacs-gpu` is its skip
  branch, so a test whose whole purpose is real wgpu rendering passed
  having rendered nothing. The missing build does not merely fail
  twelve tests — it voids coverage in tests that report green.
  (`vterm_stage3` a37 has the same shape by source read; cargo captures
  passing tests' output, so the skip is invisible in the log.)
- **What landed.** A named `build-crdt` step emitted immediately before
  `sweep-crdt` under `--protocol`, carrying the exact §5 invocation —
  **not** folded into the sweep command, because `cargo build … &&
  cargo test …` reports a *build* failure under the name `sweep-crdt`.
  Plus **`--self-test`** (Q#GR-5): a hardcoded three-line synthetic plan
  — pass, fail-named-`build-crdt`, **pass sentinel** — driven through
  the *real* runner loop, which is what makes the **runner's** failure
  naming *and* continuation observable at all. (It does **not** witness
  the real step's name — see the round-two entry below, which is where
  that gap was found and closed.) `PLAN_FILE` is deliberately **not**
  injectable: that would turn the runner's `eval` into a general command
  executor, the same defect this script's review caught in
  `--acceptance`.
- **THE WITNESS DID NOT REACH THE STEP — found in review of the
  implementation, closed at `677fd25`.** The lane shipped without the
  regression guard it was created to provide, because **neither witness
  could see a name**: `--print-plan` renders `emit_plan | cut -f2-`, so
  the ordering test compared *commands* with the names cut off, and
  `--self-test` hardcodes the string `build-crdt` in its **own
  synthetic** plan, so it proves things about the runner and nothing
  about the real emitter. Review demonstrated it directly: **renaming
  the real build step to `sweep-crdt` left both tests passing** — a plan
  that would report a build failure under the sweep's name, sitting
  green, which is the exact misattribution the separate step exists to
  prevent.

  **The fix is `--print-plan-named`**: a second *rendering* of the same
  `emit_plan`, printing the `name<TAB>command` text the runner reads
  back from `PLAN_FILE`, asserted by **whole-line equality** so name and
  command are pinned together, and `sweep-crdt`'s pair asserted too
  (asserting only the build's name leaves the identical hole open in the
  other direction). **`PLAN_FILE` remains uninjectable** — a test that
  supplied the runner's plan would turn its `eval` into a general
  command executor, the defect the `--acceptance` refusal exists to
  prevent — and **`--self-test` stays**, witnessing the *runner* (failure
  naming, `FAILED:`, log paths, non-zero exit, continuation via the
  sentinel), which it may no longer *stand in for* attribution of the
  real step. A companion test pins `--print-plan` as that rendering
  minus its names, so the two cannot drift into asserting a name the
  runner never uses. Both new tests are on the **no-gates** paths.

  **Mutated individually, each now red** (the first is the one the
  previous round passed): build renamed `build-crdt` → `sweep-crdt`;
  sweep renamed `sweep-crdt` → `crdt-sweep`; build features
  `luajit,crdt` → `luajit`; build emitted **after** the sweep. Suite is
  20 tests.
- **AUDITED FOR THE SAME DEFECT ELSEWHERE, and one instance is left
  open deliberately.** Renaming **every other** plan step — `fmt`,
  `clippy`, `lib`, `m4`, `gpu`, `sweep`, `diff-check`,
  `acceptance-<suite>` — leaves all 20 tests green: no test asserts any
  step name but `build-crdt` and `sweep-crdt`. For most that is only a
  log filename and a `FAILED:` entry. **`sweep` is not**: the runner's
  end-of-run listing globs `"$LOGDIR"/*-sweep.log` and
  `*-sweep-crdt.log`, so renaming that step silently empties the *"read
  these, do not re-run and grep"* listing that is the U2/U3 remedy, with
  the suite still green. **Not closed here**: the listing only exists on
  the *run* path, and every test in this file is deliberately no-gates,
  so there is no cheap witness for it — recorded rather than papered
  over.
- **Blocks PR #228 (discovery Stage 2).** That lane's `--protocol`
  result needs re-establishing on a fresh target dir under the repaired
  script. Deliberately **not** folded into that feature branch, and it
  happens **after** this lands, not inside it.
- **Acceptance criterion, witnessed 2026-08-09.**
  `scripts/gate --acceptance gate_script_acceptance --protocol` on a
  target root that **did not exist** (precondition recorded, not
  assumed): all eleven steps green, `09 build-crdt ok` producing
  `debug/pmacs-gpu`, and `gpu_invocation_acceptance` at **15 passed /
  0 failed** where the same suite was 3/12 without the build step.
  Zero occurrences of the *"build pmacs-gpu"* signature in the sweep
  log, and a54/a37 ran for real rather than taking their skip branches.
  **No manual build anywhere** — which is the thing that was false.
- **UNEXPLAINED RED, recorded rather than swept up.** An earlier
  attempt at the same cold run failed step 10 with **36 + 4 + 6 + 4
  failures across `m5_5`/`m5_6`/`m5_7`/`m5_8`**, all real-daemon
  suites, all with signature *"daemon exited with exit status: 101
  before socket appeared; socket=/tmp/.tmpXXXX/pmacs.sock — `<stderr
  empty>`"*. **Not** the `pmacs-gpu` signature, and no row in
  `docs/ci-red-signatures.md` matches it. Re-running the same test
  binary from the same target directory gave 36/36 green, which by
  that registry's own rule establishes **intermittence only, never
  environmental cause** — so this stays open rather than being
  attributed to the load (~25–30 across four concurrent lanes' gates).
  **De-implicated from `build-crdt` by construction, not by the green
  rerun:** the root crate's `default = ["luajit"]`, so
  `--no-default-features --features luajit,crdt` enables *exactly* the
  same feature set as the sweep's `--features crdt`. The build step
  cannot hand the sweep a differently-featured binary, so it has no
  mechanism by which to break a daemon suite. Local, not CI, so not a
  registry row; noted here for whoever sees it next.
- **Gates:** `scripts/gate --acceptance gate_script_acceptance`. Note
  the recursion — this lane edits the script that runs its own gates,
  so `--print-plan`, `--print-plan-named`, `--help` and `--self-test`
  were also checked by hand after each edit: a change that breaks the
  script cannot be reported honestly by the script. The assertions were
  **mutation tested**: wrong features, wrong position, unconditional
  emission, an aborting runner, the build folded into `sweep-crdt`, and
  — added in the second round — a **rename of either** the build or the
  sweep step each fail the suite.
||||||| parent of 72bbb96 (docs: LSP LaTeX coverage framing revision 2, on a branch at last)
||||||| parent of 312ec7a (docs: frame Discovery Stage 2 (revision 2) — M-x rows)
||||||| parent of 8f86908 (docs: frame worker identity Stage 1 (revision 1))

## QoL arc retirement — PR #224 OPEN (docs only)

**PR #224** — https://github.com/levineuwirth/pmacs/pull/224. Written
**after** the PR existed rather than with the lane's first commit —
which is the standing correction from #171 and #215 being missed again,
and it took review asking. Recorded that way rather than quietly
back-dated: this file requires a lane for **every open PR**, including
the PR that retires other lanes.

- **Branch `retire-long-lines-lane`**, base `githubsucks/main` @
  `9a26ac8` (the #223 merge). **`githubsucks/retire-long-lines-lane` is
  the authoritative tip** — the ref, not a SHA, since any edit to this
  block advances past whatever SHA it records. Recover with
  `git fetch githubsucks && git checkout retire-long-lines-lane`.
- **Docs only.** Two files, `docs/active-work.md` and
  `docs/agent-handoff.md`. No `src/`, no crate, no manifest, no test
  changes.
- **Scope:** Rule 4 for the QoL arc, closed at #223. Remove the three
  merged lane blocks (long lines, Stage 1 #219, Stage 2 #220) **after**
  re-homing their durable residue to the handoff; advance the handoff's
  date and `main` anchor and this file's canonical-base record and
  recovery floor; add capability-aware keymap resolution as a named
  handoff §6 backlog item — cross-cutting, **not started**, needs its
  own framing.
- **Verification:** `git diff --check` clean. **The recovery path was
  re-exercised, not SHA-swapped** — fresh clone into an empty
  directory, `githubsucks` alias, `--prune` fetch, `9a26ac8` confirmed
  an ancestor of `githubsucks/main`, worktree recovered with the
  three-argument form; all four steps clean. Swept for dangling
  references to the removed lanes and their branches. The full gate
  suite is **not** re-run for a change that cannot reach it, and that
  is stated rather than left as a gap.
- **Retire this block in the next absorption after #224 merges.** It
  describes a docs PR; once merged there is nothing volatile left.

## Docs absorption after #217 — MERGED as #218 (2026-08-06 09:59Z)

**PR #218** — https://github.com/levineuwirth/pmacs/pull/218. **This
block was written with the lane's first commit, before the PR
existed**, so the row above was filled in rather than invented. That is the standing correction from #171 (153 commits of
drift while invisible here) and #215 (no lane until review caught it):
this file requires a lane for every open PR, and the way that stops
recurring is writing it now rather than after someone asks.

- **Branch `docs-absorption-217`**, base `githubsucks/main` @ `db1bbe9`
  (the #217 merge). `githubsucks/docs-absorption-217` is the
  authoritative tip — any edit to this block advances past whatever SHA
  it records, so the ref is the thing to trust. Recover with `git fetch
  githubsucks && git checkout docs-absorption-217`.
- **Docs only.** No `src/`, no crate, no manifest, no test changes.
  Gates run are fmt, `git diff --check`, `--lib`, and
  `listview_acceptance`; the full suite is not re-run for a change that
  cannot reach it, and that is stated rather than left as a gap.
- **Scope:** retire the tree and macOS-CI arcs per rule 4, re-home
  their residue first, file R5 and R6, and carry four durable lessons
  into the handoff.

### Not in scope

Diagnosing R5 or R6, or auditing the three readiness helpers — those
are the lanes this one creates, not work it does. Retiring the
CI-CRDT, Distribution, or reap-ledger lanes: each still owns undone
work and rule 4 does not apply to them.

## Empty-content readiness, a fourth and fifth instance — FOR THE R6 AUDIT

Found 2026-08-06 while gating this lane, recorded here because it
widens an existing lane's scope rather than starting one.

A loaded `--features crdt` run failed `m6_1_pty_raw_mode_disables_kernel_echo`
and `m6_1_pty_canonical_mode_keeps_kernel_echo` with
**`stty -a output was: ""`** — read-before-write on the child's output.
That is the **same family as R4** (readiness predicate satisfied by an
empty file) and **R6** (readiness file never published), and it means
the readiness-helper audit's scope is not just three `wait_for_file`
copies under `tests/`: `src/process.rs`'s own tests carry the shape
too.

Both passed isolated and the full suite was green on a quiet machine,
so this is load-sensitive and **undiagnosed** — recorded as a scope
note for the audit, not as a registry row: these were local, and the
registry judges red **CI** runs.

## Pre-checkout CI reds — a class the registry has no row shape for

Seen 2026-08-06 on #220, three times across two runs (`M4`+`M5`, then
`M5` again on the rerun):

```
Prepare all required actions
Getting action download info
Failed to resolve action download info. Error: Internal Server Error
##[error]Failed to resolve action download info.
```

**This is not a flake and not a test failure.** The job dies inside
`Set up job`, before `actions/checkout` — a `grep` for
`checkout|cargo|test result:` over the full job log returns **0**. No
repo code is fetched, so the red carries *zero* information about the
commit, in either direction.

Two things follow, and both matter for signal integrity:

- **Re-running is a first execution, not a retry-to-green.** The rerun
  rule governs a test that ran and failed; nothing ran here. The
  discriminator is objective and cheap — did the job reach checkout?
- **It cannot be a registry row as the registry is written.** Matching
  requires an *exact test selector* plus fragments, and there is no
  test. Recorded here rather than forced into a shape it does not fit.

The second `M5` failure took **4m52s**, which read like a real run;
the duration was entirely retry backoff. Duration is not evidence that
a job executed — **the log is**.

Whether `docs/ci-red-signatures.md` should grow a short non-row section
for this class is an open question for its owner, not something this
lane decided.

## Tree primitive (P5) — MERGED as #217; adoption is the open work

**The lane is gone, not the work.** Rule 4 removes a lane after merge,
and the primitive is merged: `listview` rows take optional `depth` and
`id`, collapse is primitive-owned, selection re-seats by id, and the
LSP outline is the one adopter. `COHERENCE.md` §14 is ◐ and §20 says
adoption rather than construction, which is where the remaining work is
recorded — **not here**, because none of it is in flight.

What review found is worth carrying forward, since all four were
invisible to a passing suite:

- **A selection test that toggled the root proved nothing.** The root
  sits on line 1 before and after collapsing, so it passed unchanged
  under the line-keyed re-seating that id-keyed re-seating replaced.
  A moving-node witness (`tr_4`) was the fix.
- **TAB was bound on every listview**, so flat panels lost their
  fall-through to the global binding and the Q#P3 read-only intercept.
  Delegation restored it.
- **`item` was effectively required.** `line_to_item` is sparse when a
  row omits the optional `item`, and `seat_cursor` took `#` of it — a
  display-only tree stranded the cursor on the header. Every existing
  test supplied `item`, so none could reach it.
- **"Opaque, compared by equality" was two contracts.** Selection uses
  `==`; collapse keys a table, which consults no `__eq`. Narrowed to
  string-or-number, and with it uniqueness and not-NaN, all enforced
  where rows enter.

The last two are the durable lesson and it is in the handoff: **an
optional field that a data structure's shape depends on is not
optional**, and a contract that two mechanisms must honour is only as
strong as the weaker mechanism.

## Leaked daemons from `gpu_invocation_acceptance` — NEEDS A LANE

**Found 2026-08-05 while cleaning up after the tree-primitive work. No
branch, no framing.**

- **42 orphaned `pmacs --daemon` processes** were resident on the
  development machine, **the oldest 3 days 23 hours old**. All had been
  **reparented to systemd** (`ppid=1`) and all had **deleted sockets**,
  so nothing could ever reach or reap them.
- **Source: `tests/gpu_invocation_acceptance.rs`** — the one-command
  tests, whose daemons carry `--socket <tempdir>/one-command.sock`. The
  tempdir is cleaned up; the daemon is not.
- **Rate measured, not estimated: 3 per sweep.** A single isolated
  `--features luajit,crdt` sweep leaked exactly three. 42 is what
  several days of sweeps accumulate to.
- **This predates the tree work** — the oldest is four days old — so it
  is a standing leak, not something a current lane introduced.

**Why it belongs to the reap-ledger family.** This is precisely the
shape that lane exists for: a process that outlives its supervisor with
nothing left watching it. The ledger arms only for `spec.group`, and
these are daemons spawned by a test harness rather than by compile mode,
so **nothing in the existing ledger covers them**.

**Why it matters beyond tidiness.** Dozens of resident daemons were
present during every local sweep run this week, including the one that
produced the unclassified failure recorded in the **tree-primitive lane
above** (and, in full, in that lane's framing §6a). That
makes them a **rival explanation** to the shared-target-dir mechanism
for that occurrence, and neither can be tested against it now — the
signatures were not captured. A leak that quietly changes the
environment of every subsequent test run is a measurement problem as
well as a resource one.

**First questions for whoever takes it:** does the test harness fail to
reap, or does the daemon fail to exit when its socket disappears? Those
have different fixes, and the second would be a product defect rather
than a test one.

## macOS CI signal integrity — ARC RETIRED (#215, #216); residue re-homed

**Both stages merged and the arc is done**, so rule 4 removes the lane.
Stage 1 built `docs/ci-red-signatures.md` and audited the incumbents;
Stage 2 retired **R2** and **R4** with discriminating witnesses. The
framing `docs/macos-ci-signal-integrity-framing.md` and the registry
both survive the lane — the registry is the durable artifact this arc
existed to produce.

**Retiring it required re-homing the residue first**, which is why this
did not happen at merge. A lane removed while it still owns undone work
does not close that work, it hides it. What it owned:

- **R1** (supersede cancellation budget, *measurement design*) → the
  **async-runtime lane**, below. Its retirement condition is Q#MCI3:
  replace or justify the measurement. Widening the budget would make it
  pass and measure nothing more.
- **R3** (live-leader EPERM, **UNRESOLVED — possible product defect**)
  → the **reap-ledger lane**, below, which already parks every
  disposition change pending exactly this question.
- **R5** and **R6**, added 2026-08-06 and neither diagnosed → the
  async-runtime lane and a **readiness-helper audit** respectively.

### The rows are the state now, not this block

An occurrence scan on 2026-08-06 (last 25 `main` runs: 23 green, 2 red)
turned up two things worth recording as method rather than as trivia.

**A signature very nearly got misfiled by theme.** `main` run
30710662474 is the same test as R3, with `EPERM` and
`measured_group=unobservable(ESRCH…)` — and R3 requires `leader=live`
where that run reads `leader=exited(signal SIGUSR1)`, which is R2's
exact fragment. Read by test name and shared fragments it looks like
R3; read by required fragment it is R2, four days before R2's
retirement, on the *other* macOS flavor. **Attaching a live unresolved
product-defect row to an occurrence of a retired test race is precisely
the error the exact-fragment rule prevents**, and it was caught by
checking the fragment rather than the resemblance.

**A second red matched nothing at all** and became R5, rather than
being folded into R1 because both involve supersede under a deadline on
macOS. Sharing a subject is not sharing a signature.

### Async-runtime lane — NOT STARTED, owns R1 and R5

No branch, no framing. Owns **R1** (Q#MCI3: the 50ms budget starts
before the second dispatch and is consumed by the test's own pump, so
it measures when the test was scheduled) and **R5**
(`stream_supersede_delivers_cancelled_to_on_close`, `async pump
deadline exceeded`, undiagnosed). Filed together because both are the
async runtime under a deadline; they are **not** assumed to share a
cause.

### Readiness-helper audit — NOT STARTED, owns R6

No branch, no framing. **Three independently written readiness helpers
now exist**, and they disagree: `vterm_stage2_acceptance`'s waits for
expected content (hardened by #216), `wait_for_published_file` was
fixed with it, and
`tests/bottom_panel_stage1_acceptance.rs:2446` carries only the
zero-byte half. R4's disposition predicted this recurrence under a new
selector, and R6 is it.

**Scope is the audit, not the call site**: how many helpers exist,
whether they can be one, what each promises. Patching `acc28` alone
leaves the question open under a fourth selector — which is the same
mistake as fixing `wait_for_file` and leaving
`wait_for_published_file`, already made once in this arc.

## CI CRDT coverage — MERGED (#209); kept for its three follow-ons

**Rewritten, not removed.** Rule 4 removes a lane when its ARC is done;
the coverage itself has landed but three named follow-ons have not.
Durable facts — the corrections, the traps, the census tool — are in
`docs/agent-handoff.md` §§1/5 per rule 3, so they are not repeated here.

- **MERGED as #209** (`main` @ `c5f7501`, 2026-08-01, two review rounds),
  framing `docs/ci-crdt-coverage-framing.md` revision 5. The lane had sat
  under "NEEDS A LANE" with no branch and no owner since #166.
- **CI compiles and runs the CRDT corpus for the first time.**
  `Test (crdt)` runs **3,766** tests where none ran before;
  `M10 Perf Gates (crdt)` covers two suites no workflow had ever named;
  `cargo clippy --features crdt` is enforced going forward rather than
  being a required local gate that rots in CI. **275 of 279 dark tests
  recovered**, the other four excluded with stated reasons.
- Branch `ci-crdt-coverage` retained; it carries nothing unmerged.

### Two CI weaknesses Stage 3 exposed — decisions, not defects

Both surfaced while gating #213 and belong to this lane because it owns
the crdt job.

- **CI does not pass `--no-fail-fast`, so a multi-suite break reports as
  a single-suite one.** `cargo test` halts after a failing binary, so
  #213's first crdt failure showed **one** suite when a local
  `--no-fail-fast` sweep of the same tree showed **thirteen**. The
  Stage 3 census hit the identical trap and recorded it; CI has it too.
  The cost of the flag is running the remaining suites on a red build,
  which is usually what you want when diagnosing.
- **The crdt job pairs the heaviest build with real-PTY deadlines.** It
  installs lavapipe, builds the full workspace with `crdt`, runs the
  largest test count, and includes real-PTY smokes with **5-second**
  waits. #213 saw two different such suites fail on two runs of the
  same commit. One of those was a real regression the flip caused; the
  other was load. **That ambiguity is the problem** — a job where noise
  and signal look alike trains people to rerun rather than read.
  **The triage half of this is now owned by
  `docs/ci-red-signatures.md`** — signature-keyed rows and a rerun rule
  that refuses to treat a green rerun as an all-clear. What remains here
  is the job-cost question: whether the crdt job should carry real-PTY
  deadlines at all.
  Candidate: longer deadlines for real-PTY assertions in this job
  specifically, or serialize the PTY suites.

### Still owned by this lane, not yet done

- **The macOS `crdt` leg.** Deliberately ubuntu-only at first, "decide
  about macOS from evidence." **That evidence now exists** and is
  favourable: the non-crdt macOS legs pass at 3,474 (thirteen fewer than
  ubuntu's 3,487 — `cfg`-compilation of the Linux-gated process tests,
  not lost coverage), and no crdt-specific failure appeared anywhere.
  This is now a small, decidable addition rather than an open question.
- **The `--lib --features crdt` flake.**
  `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
  failed ~1 run in 5 with `active_reader_probe` returning `None`.
  **It did not reproduce in #209's runs** — `--lib --features crdt` was
  2,081/2,081 and the full serialized sweep clean — but that is *not*
  evidence against the hypothesis: every run was `--test-threads=1` and
  the trigger was observed under **parallel** full-suite load. The
  `drain_until` explanation (draining for `Started` also ticks, and a
  tick can reap the leader before the following probe) remains an
  inference from control flow, not a falsified root cause. **Its own
  PR** — a product-defect hypothesis, where everything in #209 was
  workflow configuration.
- **`InstanceCapabilities::crdt_replica`'s serde default.**
  `#[serde(default = "default_true")]` is a *third* default mechanism,
  unconditional, and therefore disagrees with the `Default` impl in a
  non-CRDT build. Exercising it needs a self-describing format and
  `pmacs-protocol`'s only serde dependency is postcard, which is not
  one. Adding `serde_json` as a dev-dependency to test a divergence
  #209 did not introduce was refused as scope creep. Parked, not
  forgotten.
- **The two unattributed CRDT failures from #178's round-2 gating.** No
  test names were captured, so there is nothing to reproduce.

Recovery, only if a follow-on needs the branch:

```sh
git worktree add ../pmacs-ci-crdt \
  -b ci-crdt-coverage-followup \
  githubsucks/ci-crdt-coverage
```

## Distribution (P8) — STAGE 1 SHIPPED as v1.1.0 (#211)

**Rewritten as a lane rather than removed**: the arc is not done — Stage
1 was scoped to binaries-on-tag and everything else in §17 is untouched.
Framing `docs/distribution-stage1-framing.md` revision 3. Durable facts
are in `docs/agent-handoff.md` §1.

- **Released 2026-08-01.** `v1.1.0-rc.1` (prerelease) then `v1.1.0`,
  **both cut from the same commit `000b6cd`** — the #211 merge SHA — so
  the final release was built from byte-identical source to the one whose
  artifacts were verified.
- **Verified against the DOWNLOADED artifacts, both tags, not the build
  logs:** archive member lists (exactly `pmacs`, `pmacs-gpu`, README, two
  licenses), executable bits, absence of `pmacs-audit` /
  `pmacs_fake_lsp` / `pmacs_fake_mcp`, `pmacs --version` = `1.1.0` and
  `pmacs-gpu --version` = `1.1.0 (protocol v21)`, `SHA256SUMS`, the
  glibc floor, and CRDT presence **against a non-CRDT negative control**
  (1,576 `loro` strings shipped versus **0** in a control build — the
  control is what makes the number mean anything).
- **`pmacs` alone needs only glibc 2.34; `pmacs-gpu` needs 2.35.** The
  stated floor is the pair's, 2.35, not the more flattering single-binary
  number. That is why RHEL 9 (2.34) is excluded even though the editor
  binary would run there.

### Still owned by this lane, not yet done

Each is a stated non-goal of Stage 1 (framing §5), not an oversight.
**The next increment is a decision about which of these the project
wants, not a continuation of a plan.**

- **Channels** (stable/nightly), **in-place update**, **rollback**.
- **Signing and notarization.** macOS binaries are Gatekeeper-quarantined
  today; the release notes say so.
- **RHEL 9 and older glibc** — needs a container or cross-build, not a
  runner change.
- **Intel macOS**, **Windows**, **reproducible builds**,
  **package-manager distribution**.
- **Runtime dependency checking.** §17 asks first launch to identify
  optional external tools; `/bin/sh`, `stty`, git and tar are documented
  and never checked. That is §18 onboarding work.

## Discovery lane (P4) — STAGE 1 MERGED (#207); STAGE 2 IS NEXT

**Rewritten, not removed.** Rule 4 removes a lane when its ARC is done;
this arc is not — Stage 1 built the command surface and everything that
needs a Rust change is still ahead. Stage 1's durable facts are in
`docs/agent-handoff.md` §1.

- **Landed:** eleven `help.*` commands over the existing registries,
  indexed by `M-x help`, with `editor.describe-command` /
  `editor.describe-setting` kept as forwarders. No Rust, no protocol
  change. §5 moved substrate-without-surface → **Partial**.
- **Stage 2 candidates, in rough dependency order:**
  1. **Richer M-x rows** — a **protocol change**:
     `MinibufferPrompt.candidates` is `Vec<String>`, while
     `CompletionPopupRow` already carries `kind`/`detail`, so the wire
     pattern is solved and the bump is the work.
  2. **`Command` gains title / category / aliases / flags /
     arg-schema** — a Rust type change across ~147 definition sites.
     MCP currently works around the missing schema by stuffing rendered
     JSON into the description string.
  3. **Predicate evaluation.** `Command.predicate` is read at
     `src/help.rs:76` and one test, and **evaluated nowhere**. Starting
     to evaluate it makes commands stop being invocable, so it needs its
     own decision about what "unavailable" means at each call site —
     M-x, dispatch, menu. `discovery_acceptance`'s `d9` pins today's
     behaviour with a *raising* predicate, so that stage must change the
     pin knowingly.
  4. **Help-layer unification.** `src/help.rs` is still orphaned. Stage 1
     funnels every command through `pmacs.editor._show_help` and renders
     via named per-subject functions, so the work is enumerated:
     replace the four subjects `src/help.rs` covers (key, mode, hook,
     buffer) and **write three new Rust renderers** for settings, lists
     and apropos.
  5. **The help prefix.** Deliberately untouched by Stage 1 — the
     decision is one for the whole family, and `C-h` is **not** free
     (non-kitty terminals cannot disambiguate Ctrl+Backspace from
     Ctrl+H; both produce byte 0x08). `F1` / `C-c ?` / a rebind are the
     candidates.
- **Two Stage-1 facts a Stage-2 author needs.** Completion is
  **assistance, not validation** — `resolve_accepted_value` returns the
  literal typed text when no candidate is selected, so closed-set
  acceptance is unbuilt Rust work. And **`invoke_interactive` is not the
  M-x path**; the forwarders learned that the hard way in CI, since
  `pmacs.command.invoke` is a real caller of the old names.

## Generated-buffer immutability lane (Arc: workbench primitives) — STAGE 1 MERGED; STAGE 2 IS NEXT

**Framing #188 (revision 7) and Stage 1 #191 are both on `main` @
`4cd4a7b`.** Their durable facts are in `docs/agent-handoff.md` §1 —
including the contract-ownership rule (the framing owns the acceptance
criteria; an implementation adopts them and may not restate or narrow
them) and why `dired`/`listview` were the correct first two families.

- **Stage 2 is not started and has no branch.** It owns everything with
  new Rust in it: `Buffer::apply_generated_edit` + `GeneratedOutcome` +
  the `{ generated = true }` option and its `run_buffer_edit` arm;
  `set_generated_contents` reimplemented over it; Q#GB10's path-backed
  refusal and `mark_clean`; Q#GB15's `identity_protected`; Q#GB13/GB18
  for `compile.lua` and the search panel; Q#GB5's `ensure_slot` lock;
  the remaining 13 write sites; and the three `compile_mode_acceptance`
  intruder tests converted per Q#GB12.
- **It collides with dired Stage 2b**, which changes `paint`'s callers.
  Whichever starts second integrates first.


## Reap-ledger silent failures — MERGED (#202); kept for its parked follow-ons

- **Branch `reap-ledger-silent-failures`**, worktree
  `../pmacs-reap-ledger`, based on `githubsucks/main` @ `22df6ab`.
  `docs/reap-ledger-silent-failures-framing.md`, **revision 4**;
  approved at revision 3 after two review rounds (round 1: three
  blocking, two major; round 2: two blocking, two major; all accepted).
  Revision 4 records implementation findings, not a new design round.
- **All four bets resolved.** Bet 1 (every site takes a directed
  outcome) and Bet 2 (every consequence is reachable) hold. **Bet 3
  resolves the shutdown coupling as real and measured** — under 500ms
  with a failed force-kill plus an errored probe, versus the full 2s
  bound with only the force-kill failing. **Bet 4 is falsified: no
  reporting channel exists**, so reporting becomes its own lane.
- **The in-drain pin's first fixture was vacuous, and the bite caught
  it.** `poll_one` TERMs the group on leader exit, so an untrapped
  descendant died before writing its late marker — absent on *both*
  paths. With the seam reverted the pin failed only the consumed-plan
  check, never the content assertion. Fixed with `trap '' TERM` behind
  the readiness gate.
- **Gates: 10/10 green** on the pushed tree, all five bootstrap-storage
  variables controlled — fmt, diff-check, clippy, `--lib` (1888),
  `--lib --features crdt` (2073), compile-mode (67), copy-mode in both
  feature configurations (18/19), M4 with the basedpyright skip (149),
  required GPU (221). The five new process pins ran **15/15** as a
  repetition set, since supervisor tests are load-sensitive.
- **Unparked from PR #200's §5.** #200 retired the premise that
  justified the ledger's leniency and deliberately changed no
  disposition; this lane owns what it refused.
- **Now also owns R3**, re-homed 2026-08-06 when the macOS CI
  signal-integrity arc retired. `docs/ci-red-signatures.md` R3 is a
  group-directed `kill` returning **EPERM while the leader was observed
  live**, with `measured_group` — the one field able to disagree —
  unreadable. It is the **same group-target question** #176 and #200
  circled and this lane parks every disposition change pending, which
  is why it lands here rather than staying with a retired CI lane. Its
  registry entry is explicit that it is an **unresolved possible
  product defect** and that **a green rerun never retires it**; that
  constraint travels with the row, not with whoever inherits it.

  Note what it is *not*: R2 is the same test with `leader=exited(signal
  SIGUSR1)`, a test race, retired. An occurrence scan on 2026-08-06
  nearly filed a second R2 occurrence here as R3 on the strength of the
  shared test name and shared `EPERM` fragment. **`leader=live` is the
  fragment that separates a product-defect candidate from a fixed
  fixture bug**, and it is the whole reason the row lists it.
- **Four sites, not the two #200 named.** In the persistent ledger: a
  probe error of any errno drops the entry and cancels escalation; a
  failed escalating `SIGKILL` is marked as succeeded, so **no later tick
  retries it**; and `shutdown()` discards its own force-kill result the
  same way — on the path that exists specifically to stop a leak at
  editor exit. Those last two are **distinct, not cumulative**:
  `shutdown()` force-kills every entry with no `!entry.killed` guard, so
  a failed escalation still gets one attempt at exit, while a failed
  force-kill leaks the group past exit with nothing left to try.
  **Plus the in-drain
  twin** `final_drain_runtime`, which collapses every errno to "dead"
  while no tick runs; a false "dead" there cancels the readers, so its
  failure mode is truncated output rather than a leaked process.
- **The blast radius is exactly what the ledger exists for:** a
  TERM-ignoring descendant that outlived its leader with output
  redirected. Neither leader state nor reader state can see it; only
  group liveness can. A silent drop leaks the one process nothing else
  is watching. **Journey step 9 (build/test), not step 8** — the ledger
  arms only for `spec.group`, which spawn rejects for PTY mode, so no
  terminal reaches it; compile mode is the only production caller.
- **`shutdown()`'s final loop terminates when the ledger empties**,
  which happens via the same silent drop — so the probe error that hides
  a leak can also end the cleanup loop early. That coupling is why the
  probe cannot be made strict on its own. **Its precondition is
  `any_running()` already false**, so the fixture must be a leader that
  exited leaving a survivor; any other shape tests the other arm of the
  disjunction.
- **None of the three has been observed.** #200 saw an explicit
  `SIGTERM` fail in `signal()`, not a ledger call. The premise is
  falsified and the path exposed; the occurrence is not evidence these
  fire.
- **They are also untestable today**: `tick_reap_ledger` and
  `shutdown()` call `nix` directly and consult no injection seam, unlike
  `signal()`'s `forced_kill_errno`. All five existing ledger tests
  exercise the success path only. The seam must be **site-directed and
  multi-outcome**: `shutdown()` calls `self.signal()` before its ledger
  force-kill, so a single global slot would be consumed by the wrong
  call, and the coupling test needs two pending outcomes at once. The
  in-drain probe repeats every 1 ms but only cancels readers after 50 ms
  of false "dead", so it needs a directed full-drain override rather
  than a one-shot error. Test state is per-supervisor and shared into
  the drain context, never global; teardown proves its intended site was
  reached. The in-drain SIGKILL's local flag has no independently
  observable outer-path consequence, so it is named but not given a
  dead injection seam. The first PR is the seam **plus** the tests that
  exercise it — a seam without tests does not show it reaches the
  intended calls.
- **Diagnosis first, no disposition change proposed.** Stage A of the
  signal lane had three tolerance rules rejected across three revisions
  for the same shape of error on the same data structure.
- Recovery from a clean checkout:

  ```sh
  git fetch githubsucks
  git worktree add ../pmacs-reap-ledger \
    -b reap-ledger-silent-failures \
    githubsucks/reap-ledger-silent-failures
  ```

## Folding lane (Arc 6) — Stages 1 and 2 MERGED; Stage 3 (GPU) is next

Both shipped stages are on `main`; nothing in this arc is in flight. Stage 3
has **no branch and no framing yet**.

- Stage 1 (headless fold engine) merged as **#142**, Stage 2 (grid/daemon
  collapse) as **#149** — both under "Closed since the last snapshot".
- Retained, carrying nothing unmerged: branches `folding` / `folding-tui`
  and worktrees `../pmacs-folding` / `../pmacs-folding-tui`. The framings
  `docs/folding-framing.md` (rev 5) and `docs/folding-stage2-framing.md`
  (rev 4) are the approved artifacts Stage 3 re-scouts against.
- **Stage 3 (GPU) obligations, already named by the framings** — the
  starting point for its own framing doc: GPU collapse at TUI parity;
  caret/hit-test fold-awareness; the `BufferSnapshot` **fold-mirror clear**
  (parent R2-4 — without it, empty-after-revert diff suppression leaves
  stale folds on the GPU, the same trap class as #120); CRDT-origin and
  GPU-optimistic interactive unfold (parent R2-3); and flipping
  `FrontendView.fold_projection` to `true` for semantic frontends, which
  Stage 2 deliberately left `false` (Q#FD21).

## Parked lane: kill-ring browser + persistence

- Portable branch: `githubsucks/kill-ring-browser`
- Parked framing head: `503c489`
- State: framing only, revision 2; no implementation and no PR.
- Status: explicitly parked by the user on 2026-07-20.
- Its original scout was based on `0efb5cd`. The preserved framing marks
  this ground truth stale and requires a complete re-scout against the
  then-current `githubsucks/main` before implementation.
- Compile-mode has merged since the original scout, so old
  “compile-mode in flight” keybinding/touch-set assumptions are not
  authoritative.

Recovery worktree, only when the user un-parks it:

```sh
git worktree add --track \
  -b kill-ring-browser \
  ../pmacs-kill-ring-browser \
  githubsucks/kill-ring-browser
```

## Closed since the last snapshot

- **Process-signal diagnostic completeness — MERGED as #200**
  (`main` @ `a2a92bb`), atop Stage A #176. `docs/process-signal-diagnostic-completeness-framing.md`
  revision 6; framing approved at revision 4 after three rounds, then
  five review rounds on the implementation. Durable facts are in
  `docs/agent-handoff.md` §1. **Evidence collection only** — no
  tolerance rule, no retargeting, no disposition change.
  - **Bet 1 was falsified by CI and the framing's own fallback shipped.**
    `bash -m` diverges the terminal's foreground group on Linux and
    never on macOS. The divergent case is pinned by injection
    everywhere; a Linux-only corroboration drives a real shell and is
    the **only** test exercising `pty_foreground_group` end-to-end, so
    **on macOS that lookup has no end-to-end coverage**.
  - **Group identity remains unprovable**, and the pre-kill sample does
    not change that: moving `getpgid` before the `kill` removed a
    post-hoc reading, it did not make the reading contemporaneous.
  - **Still parked, each needing its own lane:** the reap ledger's
    silent cancellation (an EPERM probe drops the entry; a failed
    `SIGKILL` is marked killed) — **being scoped next**; retargeting to
    the measured pgid; any EPERM/ESRCH tolerance rule; Q#PS6; and
    `signal_target`'s read-then-kill of `tcgetpgrp` on the PTY path.

- **Six lanes removed by the 2026-07-30 absorption pass**, all merged,
  all with their durable facts in `docs/agent-handoff.md` §1:
  **#190** resource-op delete-guard implementation; **#196** dired
  Stage 2a (rename/delete reconciliation — Stage 2b remains, unstarted
  and without a lane); **#188** generated-buffer immutability framing
  (revision 7, the governing contract); **#194** silent-skip arming;
  **#195** CI timeouts and concurrency; **#197** the process teardown
  stdin deadlock.
  - **#194 and #195 kept their lessons in §3 and §5 rather than §1**,
    which is why a PR-number search of the handoff finds them only once
    each. That is sufficient under rule 3 — durable knowledge has a
    home, not a required section.
  - **A census by PR number is a proxy, not a measurement.** Counting
    `#NNN` in the handoff said five of these lanes had no record at all;
    counting by *content* found most already documented, with the real
    gap being the implementation PRs specifically (#190, #191, #196)
    while their framings were recorded. The absorption written from the
    first count would have duplicated existing entries.

- **Terminal configuration + copy mode arc — BOTH STAGES MERGED, lane
  removed.** Stage 1 **#173** (`main` @ `cf54270`, one review round) and
  Stage 2 **#178** (`main` @ `fe8b8ba`, **four review rounds**, twelve
  checks green on head `1b44c69` — verified by `head_sha`, not by the
  check summary), both 2026-07-26, both with no protocol change.
  Approved framing: `docs/terminal-config-and-copy-mode-framing.md` rev
  4, committed as the first commit of Stage 1's branch; its Q#TC6a
  carries a superseded-in-part box rather than a silent rewrite. Durable
  facts moved to `docs/agent-handoff.md` §1 (the arc bullet) and §4 (the
  `set_generated_contents` invariant) per rule 3 below, and to
  `COHERENCE.md` §14. **Stage 2 ships eight of nine criteria and the
  missing one is named** — criterion 17 needs a real GPU frontend, so it
  waits on the `a37` footing; the handoff records what it must assert.
  Branches `githubsucks/terminal-config` and
  `githubsucks/terminal-copy-mode` with worktrees
  `../pmacs-terminal-config` and `../pmacs-terminal-copy-mode` are
  retained. The gate-run flake found while gating #178 moved to the CI
  `crdt`-coverage lane above, which owns its discrimination.
- **Dired Stage 1 (the directory view) — MERGED as #165** (`main` @
  `c8ec8f3`, 2026-07-25, after one review round). pmacs has a directory
  surface: `C-x d` / `C-x C-j`, one read-only buffer per directory named
  `*dired:<canonical path>*`, a `dired` major mode carrying
  `RET`/`f`, `^`, `n`/`p`, `g`, `q`, `s`. No wire change (v20). The Rust is
  two things — a per-entry-tolerant `read_dir` (Q#DR6), which had to be
  Rust because `read_dir_blocking` fails a whole listing on any of five
  per-entry conditions and a tolerant wrapper cannot be written in Lua at
  all, and `normalize_buffer_path` going `pub` as
  `pmacs.path.canonicalize` (Q#DR2's preferred end state, so no Lua mirror
  exists and Stage 2 owes no mirror removal). The frozen m8_1/m8_2/m8_3
  counts are unchanged, which is the additivity gate. 15 claims
  bite-verified; one came back VACUOUS (acceptance 3c cannot pin descent
  routing — dired holds focus in its own panel, so dedication is the only
  discriminator) and is documented at the assertion rather than
  relabelled. Its branch (`dired-stage1`) and worktree
  (`../pmacs-dired-stage1`) are done; the abandoned `dired` branch
  (`ffdd642`, `../pmacs-dired-arc`) was superseded by a fresh cut and
  carries nothing unmerged. **Stage 2 (marks and operations) and Stage 3
  (wdired) each still need their own framing**, and the frozen fixture
  shrinks after Stage 3. Durable substrate facts and both new ops lessons
  live in `docs/agent-handoff.md` §§1/5; the implementation notes are
  `docs/dired-framing.md` §0, S1-1…S1-12. Two named forward items for
  Stage 2: `apply_resource_op`'s rename rebind is exact-PathBuf-equality,
  first-match-only, looked up with the raw path while stored paths are
  normalized — so a directory rename strands every buffer under it, and
  `pmacs.fs.rename` has zero production callers, so it can be fixed at
  the primitive; and Q#DR5's seam is the main-thread drain
  `AsyncRuntime::tick`, not `_take_result`, where rename settles as an
  undifferentiated `ReplyKind::FsUnit` and so must be keyed on
  `JobKind::FsRename`.

- **GPU terminal input (the double terminal-layout sync) — MERGED as #166**
  (`main` @ `b889873`, 2026-07-25, one review round, all twelve checks green
  after a macOS PTY-timing rerun). The dispatcher applied **both**
  terminal-layout syncs to **every** attached frontend; a semantic session
  satisfies both conditions, so its PTY was resized twice per tick forever and
  the child took a `SIGWINCH` storm that made a GPU terminal untypable while
  output still flowed. `sync_terminal_layout` is now split into a
  frontend-kind-neutral half (panel reconcile + controller liveness) and a
  grid-only geometry half, with the loop body extracted to
  `sync_terminal_layouts_for_tick` so the exclusivity is structural. No
  protocol change (v20). Durable lessons are in `docs/agent-handoff.md` §5;
  the framing (`docs/gpu-terminal-input-framing.md` rev 2) carries three
  falsified hypotheses, the two-pre-image bite matrix, and two named
  out-of-scope items (Q#GT5 interactive-shell echo on a raw PTY, which
  reproduces in-process and so is not the GUI/TUI asymmetry; and a geometry
  change appearing to clear the visible screen, which reproduces pre-fix).
  Branch `gpu-terminal-input` and worktree `../pmacs-gui-term-input` retained.
  **Its landed-doc pair MERGED as #168** (`main` @ `1b6a084`,
  2026-07-26): #166 recorded as landed, the CI `crdt`-coverage gap
  measured (**264 tests dark workspace-wide**, 177 in the library — a
  reading taken at `1b6a084` and kept here only as history. **The CI
  `crdt`-coverage lane above is the authority for the live figure**;
  do not quote this one forward), the
  vterm audit corrected — "only 3 of 9 acceptances drive a real daemon"
  was optimistic; without the frontend binary the honest number is
  **2** — and the a37 findings folded into the coverage lane.
- **Inline-math slice — MERGED as #158** (`main` @ `5aa9044`,
  2026-07-25). Detect → parse → layout → draw for `$…$`, entirely inside
  `pmacs-gpu`, no protocol change. Verified by the user's manual pass on
  a real paper after the landing. What is worth carrying forward:
  - **The v0 subset is 34 Greek symbols, sub/superscript, and `\frac`.**
    An unsupported command fails the **whole span** back to source, so on
    a real document most inline spans still show LaTeX. Widening the
    symbol map is the highest-value next increment — ahead of display
    math, which is also deferred.
  - **A stale frontend binary is invisible from the source tree.** The
    slice lives only in `pmacs-gpu`, so after it merged the feature was
    absent until `cargo build --release -p pmacs-gpu` and a client
    restart; the daemon needs neither. Diagnose with `strings` on the
    binary (`Latin Modern Math`, `MathBox`) rather than by re-reading the
    checkout, which was already current.
  - **Main was integrated three times in one day** (`8c86d34`,
    `46a1b8f`, `b889873`), merged not rebased to preserve review anchors.
    Two conflicts, both this ledger and nothing else. **The dangerous
    case was the one that did NOT conflict**: #166 auto-merged into
    `pmacs-gpu/src/main.rs`, the file this lane rewrites, because the two
    edits sat in different regions of it. Decide whether to integrate
    from the shared-**file** set, never from whether git complained.
  - **Integration was proved by test-count reconciliation**, not by a
    green run: predict what the other side adds, then check the deltas.
    GPU 199→202 matched `e547a90`'s 3; later lib 1,826→1,829 and CRDT
    2,003→2,006 matched #166's 3, with GPU unchanged because #166 adds
    none. Suites 91→92 was #161's new binary.
  - **Why the branch had no CI for a day**: a conflicting PR builds no
    merge ref, so no `pull_request` run is created. The ledger previously
    recorded this cause as unidentified; it is not. Check `mergeable` and
    confirm a run exists for the current head SHA.
  - **`m4_5_basedpyright` has no timeout and hangs forever**, parking a
    `--workspace` sweep (observed 2h26m at 38 of 92 suites). It is
    **intermittent**, so an earlier clean sweep proves nothing. Sweep with
    `cargo test --workspace --no-fail-fast -- --skip basedpyright` and
    judge progress by whether the suite count advances.
  - Named v0 approximations: the peer-caret half of acceptance 14 is
    pinned at the mapping level, not pixels; a soft-wrapped spacer draws
    its box whole at the first run's origin; the fit budget reads the
    bundled code face even under a custom `set_font` family.
- **Bottom panel Stage 1 — MERGED as #155** (`main` @ `e745068`,
  2026-07-24, after two review rounds). Window placement, window
  parameters, TUI side windows, the divider, and the adopter `display`
  opt-in, with no protocol change. Both rounds found the same class of
  defect and are worth keeping:
  - **Round 1**: the Q#BP6 side-window split guard had *no production
    caller* — `C-x 2` still reached plain `split_active` — and survived
    because the acceptance test called the core method directly.
  - **Round 2**: Q#BP7's terminal growth re-arm had *never been
    implemented*, and the assertion meant to pin it (`at_bottom`) is a
    geometric readout that a still-anchored view satisfies; the anchor
    assertions beside it compared `""` with `""` because the PTY fixture
    emitted LF-only output.
  - **Post-round-2 self-review**, caught by CI going red on all four Test
    jobs: resolving `pmacs.window.buffer()`'s no-argument arm through the
    acting frontend made a total function partial, and six runtime modules
    silently dropped operations (`kill_ring_acceptance` 30/30 → 25/5).
    Fixed in `9110f9f` before merge.
  - Gating fact found on the way: an isolated `XDG_CONFIG_HOME` prevents
    the real user `init.lua` from installing a local package and leaking
    a status message into painted-frame comparisons. **That isolates the
    observed config symptom only, not the gate:** the ambient-root lane
    above establishes that data/state/cache must be controlled too.
    There is also a latent pre-existing `main` bug in the buffer CRDT
    undo path, unrelated to this arc.
  - `compile_mode_acceptance` is load-sensitive under default
    parallelism (~1 run in 3, a different test each time); verified
    pre-existing by swapping in `main`'s `compile.lua`. It is 67/67 at
    `--test-threads=1`.

- **GPU initial target — MERGED as #148** (`main` @ `0dd16a5`, 2026-07-24,
  after two review rounds). `pmacs --gpu [--socket …] FILE` opens a target
  before the GPU window appears. Protocol bumped 19 → 20: a semantic-session
  `SessionBootstrapRequest` after `AttachRequest`, plus an appended
  `InstanceMessage::InitialTargetResult` pre-window readiness barrier; v6–v19
  wire encodings are unchanged. Root owns launcher tilde/cwd resolution and
  exact raw-byte path transport; the daemon resolves/dedups/loads the target
  and runs load/switch hooks inside one dispatcher transaction, then
  publishes CRDT-upgraded targets to existing grid replicas (gated on
  `upgraded_to_crdt`, independent of the load/create outcome, so a dedup onto
  a hidden not-yet-backed buffer still reaches pre-attached replicas — round
  2 finding). Semantic replicas receive a publication only when displaying
  that buffer, so a second target launch cannot switch an existing GPU
  window. Round 2 also closed a failure-containment gap: every dispatcher-side
  bootstrap failure now shuts down the socket (a dropped write-half clone
  does not close a shared FD), and the dispatcher drops any event from a
  session that was never installed, rather than reaching absent render/size
  state. Integrated cleanly with Folding Stage 2 (#149): fold projection at
  attach is selected from the same negotiated `semantic_render` bit the
  target bootstrap uses. Its lane, worktree (`../pmacs-gpu-initial-target`),
  and branch (`gpu-initial-target`) are done; the `-framing` branch is kept.
  Durable substrate facts and both review-round lessons live in
  `docs/agent-handoff.md` §§1/5 and `docs/gpu-initial-target-framing.md`
  rev 3.

- **Folding Stage 2 (grid/daemon collapse) — MERGED as #149** (`main` @
  `6ed4fe9`, 2026-07-24, after **five** review rounds). The grid TUI now
  renders collapses. Spine (Q#FD12): `src/fold_view.rs`'s `VisibleLineMap`,
  derived from the fold store plus a window's line offsets and **never
  stored**, threaded as `Option<&'a VisibleLineMap>` on a lifetime-bearing
  `Viewport<'a>` that stays `Copy`. No wire schema or protocol change; the
  GPU path is Stage 3. 48 acceptance tests on the real `paint_frame` grid,
  every behavioral claim bite-verified. Durable design points, each a trap
  Stage 3 inherits:
  - the map's unit is a **merged hidden component** (overlapping *or
    adjacent* intervals unioned, keeping the earliest visible head), not a
    fold — folds may cross, and a later fold's own head can be hidden;
  - instances are **per rendered window** and **per command/event
    operation**, never per frame; a command's map follows the operation's
    **target** window, since a wheel event names a pane without activating it;
  - fold projection is **per-frontend** (`FrontendView.fold_projection`) —
    shared `EditorCore` motion would otherwise make a simultaneous unfolded
    GPU session's cursor skip lines it still displays;
  - a hidden cursor normalizes by **position**, not row, and `set_view_top`
    clamps in the setter rather than being repaired at render time;
  - the interactive-Lua unfold keys on the **post-intercept** edit site — a
    managed buffer intercept may legally relocate the op.

  Process notes worth keeping: `main` moved under the arc, and the merge was
  textually clean but **not semantically clean** (#146 added `Viewport`
  literals the new `folds` field invalidated) — a clean `git merge-tree` does
  not mean the merged tree compiles. CI was red at review on the macOS/luajit
  `outline_5_level_100_entry_renders_within_100ms` budget flake and went
  green on rerun.

- **Documentation ledger refresh — MERGED as #147** (`main` @ `0a479ae`,
  2026-07-24). The #142 housekeeping, expanded after review found the ledger
  stale through four merges rather than one. Its own macOS/luajit red was the
  vterm `VTERM_ALT_READY` PTY timeout; green on rerun.

- **Web grammars HTML + CSS — MERGED as #146** (`main` @ `47581f4`,
  2026-07-23). `.html/.htm/.xhtml` and `.css` highlight off the official
  `tree-sitter-html` 0.23 / `tree-sitter-css` 0.25 crate query constants (no
  in-repo overlay), and HTML's `INJECTIONS_QUERY` lights up `<script>` → js
  and `<style>` → css. Durable lesson recorded in
  `docs/web-grammars-html-css-framing.md`: the `highlight.rs` capture table
  is **global**, so adding a capture name retro-paints every other language —
  check the reverse direction and pin it.

- **LaTeX Stage 1 — MERGED as #144**, with its parent inline-math framing
  committed as **#145** (`main` @ `f09b0a1`, 2026-07-23).
  `.tex/.latex/.sty/.cls` highlight via `codebook-tree-sitter-latex` 0.6 plus
  the first in-repo query overlay (`builtin/queries/latex/highlights.scm`,
  `include_str!`) — the reusable pattern for grammars whose crate ships no
  usable queries. The crates.io `tree-sitter-latex` is provably broken (no
  `scanner.c`). The math parser and Tiers 3–4 are deferred to the inline-math
  arc.

- **Folding Stage 1 (headless fold engine) — MERGED as #142** (`main` @
  `c49a8c7`, 2026-07-23, after three review rounds; round 3 clean). The
  instance-side fold store + translating/dropping `View`, the structural
  source (derived head line, closer-aware tail), the Lua data API +
  interactive `C-c @` commands, the command-path pre-edit unfold, and
  authoritative-empty `FoldState` production landed with no protocol bump.
  The `folding` branch and worktree (`../pmacs-folding`) are retained but
  carry nothing unmerged; the `folding-framing.md` framing is preserved.
  CI red at merge was an unrelated environmental perf flake
  (`outline_5_level_100_entry_renders_within_100ms`, macOS/luajit only),
  green on rerun. Stage 2 has since merged as **#149** (above); durable
  substrate seams live in `docs/agent-handoff.md` §1.

- **Vterm Stage 3 (protocol v19 + GPU terminal) — MERGED as #135** (`main`
  @ `cac4961`, 2026-07-22, after two review rounds). Arc 5's terminal stage
  is complete (compile mode #113, Stage 1 #126, Stage 2 #130, Stage 3 #135).
  Its lane, worktree (`../pmacs-vterm-gpu`), and branch are done; durable
  substrate facts live in `docs/agent-handoff.md` and `docs/vterm-framing.md`.

- **Branches deleted 2026-07-22 (authorized):** `vterm-stage3-framing`
  (Revision 8 framing; its content is carried on `vterm-gpu`, verified as a
  superset before deletion — the branch was NOT an ancestor of `vterm-gpu`
  because the framing was copied rather than merged, so it needed a forced
  local delete) and `tab-width-parity` (a clean ancestor of `main` via
  #137). Both removed as worktree + local ref + `githubsucks` ref; the
  `origin` tracking refs were pruned. The `-framing` branches for each are
  deliberately kept.

- **Tab-width rendering parity — MERGED as #137** (`main` @ `2625ec7`,
  2026-07-22). One fixed 8-column `TAB_STOP_COLUMNS` in `pmacs-protocol`
  now drives core/TUI columns, GPU code projection, and minimap width;
  source bytes and protocol ranges are unchanged. Its lane, worktree, and
  `tab-width-parity` branch (local + `githubsucks`) are deleted; the
  `tab-width-parity-framing` branch is kept. This closes the long-standing
  "tab width is a rendering-parity
  bug, NOT a config gap" deferral recorded in `docs/agent-handoff.md` §5.
- **Locals-query processing — MERGED as #134** (with handoff #136), and
  **modeline detection handoff #133**. Both landed between this lane's
  base and its canonical-main integration.

- **Config registry — MERGED as #127** (`main` @ `2e37c04`). Its lane
  (`config-registry`, worktree `../pmacs-config-registry`) is done; the
  branch is kept but carries nothing unmerged. Durable substrate facts
  moved to `docs/agent-handoff.md` §1 per rule 3 below.
- Both this and Vterm Stage 1 ran as **concurrent lanes in sibling
  worktrees off `main`**, with the shared files (`src/editor.rs`,
  `src/lua_bindings/mod.rs`, `src/lib.rs`) assigned to one lane each in
  advance. The rebase of the second lane onto the first had **zero
  conflicts** — worth repeating for future parallel work, along with its
  precondition: agree the file split before either lane starts, and keep
  each lane's footprint in the other's files to a single line.

## Update protocol

Whenever a listed lane changes materially:

1. update its public branch and head/state here;
2. record new verification and remove superseded caveats;
3. keep durable architecture in `docs/agent-handoff.md`, not here;
4. remove the lane after merge or abandonment;
5. verify every recovery command from a clean worktree before calling
   the transfer complete.
