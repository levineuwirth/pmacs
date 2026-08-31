# Active work — cross-machine resume ledger

**Snapshot: 2026-08-01.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

**Updated 2026-08-11, second pass — the file-watcher arc is DONE.**
**#235 merged** (`main` @ `122b8e8`) and **issue #233 is CLOSED**; per
rule 4 the file-watcher lane is REMOVED — its durable facts are in
`docs/agent-handoff.md` §1's arc bullet, its full review history in
`docs/lsp-file-watch-d3-framing.md` and the two PRs. The one framed
option deliberately left unbuilt (kernel notification) lives in the
framing, triggered only if the 4 s idle-latency cap ever matters.
The canonical-base line below and the handoff anchor moved to
`122b8e8`.

**Updated 2026-08-11 — two merges and a discharge.** The LSP file
watcher D1+D2 landed as **#234** (`ae84d58`) after one review round
(P1 form-from-the-pattern, P2 cancelled-scan emission — both
bite-verified), and git integration Stage 1 landed as **#227**
(`b867f64`) immediately after, refreshed onto the merged base and
re-gated. The 2026-08-10 hold on #227 is **discharged in order**, not
overridden. Both lanes below are rewritten to their remainders (D3;
git Stage 2), their durable facts absorbed into
`docs/agent-handoff.md` §1. **Next, by user ruling: D3.** The
canonical-base line below and the handoff anchor both moved to
`b867f64`. Several other lane headers still say OPEN for PRs that have
since merged (#224–#232) — per this file's own rule, trust the
canonical-base line over any lane header; those absorptions remain
owed by their own lanes.

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
- Canonical base at this snapshot: **`githubsucks/main` @ `d038f71`** —
  **GUI Stage 1-pre #237**, the input seam (`App::window_event` from 655
  lines to four; no behaviour change, no wire change), atop `f8ad3e7`
  **GUI arc Stage 0 #236**, the arc framing and its documentation sweep.
  Beneath them `e67ad07`,
  the file-watcher arc absorption, atop `122b8e8`
  the file-watcher D3 **#235** (closes issue #233), atop `b867f64`
  git integration Stage 1 **#227**, atop `ae84d58` the LSP file-watcher
  fix **#234**, atop `0e4c58d` destination capture **#231**, `3cc1b85`
  worker identity Stage 1 **#232**, `0857bf4` discovery Stage 2
  **#228**, `0190102` LSP LaTeX coverage **#230**, `7cf4653` the gate
  `--protocol` build step **#229**, `4bc55e8` per-worktree gate target
  dirs **#225**, `dcb852e` the R8 fixture fix **#226** and `b833b13`
  the QoL docs retirement **#224**. Beneath those, `9a26ac8`:
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
  **Protocol schema support is `v6..=v23`; the production server-first
  `Hello` still advertises v20** — two different facts, and #184 landed
  only the first. The upper bound moved to **v23 at #228**
  (`MinibufferPromptRows`), after **v22 at #221**, which added
  `InstanceMessage::LineWrapFacts`; `ADVERTISED_PROTOCOL_VERSION` did
  not move and must not be edited to chase it. Verified against
  `pmacs-protocol/src/message.rs`, not carried forward: this line said
  `v6..=v21` for two merges after that stopped being true.
  **Historical `v21` statements elsewhere in this file are correct
  where they describe a stage as it landed** — only this
  current-state paragraph tracks the live range.
  **The recovery floor advances with the base**, so the check below
  now requires **`d038f71`** or newer; a tree at `db1bbe9` no longer
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

The `git log` command must expose `d038f71` — the base named above — or a
newer intentional main. Keep this threshold and the canonical-base line in
step: a recovery check that accepts an older commit than the base it
declares canonical will pass on a tree the rest of this file does not
describe.
If it does not, stop and repair the remote/fetch configuration.

**LAST EXERCISED AT `d038f71`, 2026-08-12**, from an empty directory
rather than by having its SHA swapped — **which also discharges the
outstanding obligation this paragraph carried at `e67ad07`**, where the
base advanced without the procedure being re-run. That debt is settled,
not inherited: the advance recorded here was exercised as part of making
it.

Every documented step ran, in order, on git **2.55.0**:

1. `git clone` the canonical URL into an empty directory. The clone
   names it `origin`; **`git remote get-url githubsucks` errors with
   `No such remote`**, exactly as the section above anticipates.
2. Add the alias, confirm the URL, `git fetch githubsucks --prune`.
3. `git log -1 --oneline githubsucks/main` → `d038f71`.
4. **The floor check, run against the OLD DECLARED FLOOR and the new
   one**: `e67ad07` and `d038f71` are each an ancestor of the tip, so
   advancing the floor is valid rather than merely plausible.
   *(`9a26ac8` is the previous last-EXERCISED anchor, which is a
   different thing from the previous declared floor — the two had
   drifted apart, and checking the exercised anchor in the floor's place
   would have verified the wrong claim.)*
5. The three-argument `git worktree add <path> -b <local>
   githubsucks/<branch>` form → a real branch with its upstream set
   (`## recovered-local...githubsucks/gui-stage1-pre`).
6. **The documented trap, reproduced rather than assumed**: the
   two-argument form on a remote-only branch **succeeds** and leaves
   `## HEAD (no branch)`. The correction below still holds on 2.55.0.

That distinction is the whole point of this paragraph: advancing the
base is exactly when the recovery commands are most likely to have
rotted, and a swapped SHA reads identically to a verified one.

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

## Manual CI dispatch — MERGED as #245 (`7b82e14`); D2 and D3 DISCHARGED

- **MERGED 2026-08-30T10:42:18Z** at approved head `c21eee6`, merge
  commit `7b82e14`, via `--match-head-commit`. 14/14 CI green on that
  head; 8-stage local gate green on it too.
- **D3 PASS** — dispatched run `33307137965` on `main`. **All 14 jobs
  started and concluded `success` or `failure`**; none `skipped`,
  `cancelled` or `timed_out`. The run's own conclusion was `failure`,
  which D3 permits by design: its contract is that the matrix RAN, not
  that it passed.
- **D2 PASS**, with every guard the framing demanded actually
  exercised:
  - A = `33308891808`, dispatched and reaching `in_progress`;
  - B = `33308921103`, dispatched while A ran;
  - **the required overlap was OBSERVED** — A `in_progress`, B
    `pending` — so the witness is not the vacuous version that passes
    when A finishes first;
  - `headSha(A) == headSha(B) == 7b82e14`, so both really shared
    `ci-<sha>`;
  - **A concluded `success`, not `cancelled`.**
  - B was cancelled by hand afterwards to save macOS minutes; B's fate
    is not part of D2.
- **An unplanned observation, worth more than either witness.** A
  `main` PUSH run was already `in_progress` at `7b82e14` when the first
  dispatch was issued. The dispatch **queued behind it and did not
  cancel it**, then started when the push finished. That is the
  same-SHA push/dispatch interaction §5a described and no witness
  covered.
- **THE FIRST DISPATCH IMMEDIATELY FOUND A RED ON `main`**, which is
  what this lane was built for. See the proptest entry below.

## CRDT identity-replace undo — LANE TAKEN, PR #246 OPEN

**Branch `crdt-identity-undo`, PR #246, based on `aae5b35`.** Framing
`docs/crdt-identity-undo-framing.md`, **APPROVED at revision 4** after
four review rounds, then **revision 5** as a correction pass answering
implementation review.

**Every head of this branch is gated head-exact**, with `HEAD` and
`git status --porcelain` captured before and after each run and
identical. The two that matter:

| commit | what it carries | gate | CI |
|---|---|---|---|
| `2c24303` | the code, and the framing at revision 5 | all 8 green, `20260830T193305Z-4167110`, loadavg 0.90 | 14/14 |
| `6ddce0f` | the registry corrections (U14/U15 split, load claim narrowed) | all 8 green, `20260830T201908Z-84597` | 14/14 |

**This line goes stale the moment another commit lands, which is the
defect review already caught here once** — it named `db24ae3` long after
the branch had moved past it. It is written as a table so the next
update is an added row rather than a rewrite.

**The decision, ruled:** a visible TEXT delta and a CRDT-VERSION delta
are **independent dimensions** of `Edit`. The invariant is keyed on
**provenance**, and enumerated over three axes rather than defaulted:
an **empty text delta** is a shape both paths reach, and `crdt_op` is
what separates them — `None` forward (the three syntactically empty
`EditOp` forms short-circuit at `is_no_op_edit`), `Some` from
`undo`/`redo`, **required** there, because the op is the whole content
of such an edit.

**Revision 5 fixed three things review caught in the implementation:**

- the predicate **conflated the empty text delta with a version delta**,
  calling every empty-range/zero-insertion edit `version_only` and then
  accepting `(History, empty, None)` through a wildcard — contradicting
  the lane's own "the op must survive". It is now a full enumeration,
  and C5 asserts all four empty-delta quadrants instead of two. **Both
  new quadrants were mutation-checked, and neither is caught by the
  proptest** — no generated input reaches either;
- the **public `Edit` doc was factually false**: it said forward
  `apply_edit` never produces the empty-delta shape, while C2b proves
  all three forward empty forms do;
- **R7's write-up overstated what the paired gate runs exclude.** They
  exclude the source tree. They do not narrow the cause to three
  candidates — scheduler load, kernel and socket timing, and unrelated
  machine state all varied too, and a `BrokenPipe` on a socket handshake
  is exactly what those can drive.

**A GATE COVERAGE GAP, found the expensive way.** The local gate's
clippy step is `cargo clippy --workspace --all-targets -- -D warnings`
— **default features**, so **no `#[cfg(feature = "crdt")]` code is ever
linted locally**. CI lints it (`--no-default-features --features
luajit,crdt`), so a crdt-only lint passes eight green gate stages and
then reds `Test (crdt)`. That is what happened here: a
`clippy::match_same_arms` on the new enumeration, invisible to five
consecutive local gate runs.

The lint itself is `#[allow]`ed with a reason — collapsing the three
`Ok(())` arms is exactly the conflation this lane removes, and would
hide that `(forward, empty, None)` and `(history, empty, Some)` are
valid for opposite reasons. **The gap is not fixed here**: adding a
second clippy flavor to `scripts/gate` is a change to shared
infrastructure and belongs in its own lane. Recorded so the next lane
touching crdt-gated code does not rediscover it at CI.

**Four registry rows moved on this lane:**

- **R7's eighth occurrence** — the green/red pair whose heads differ by
  one markdown file. It excludes the SOURCE TREE and nothing more; an
  earlier write-up of mine narrowed the cause to three gate-state
  candidates and that overstatement is withdrawn in the row;
- **U6 went from one occurrence to five** — four on 2026-08-30, two out
  of gate and two in. Its first reproduction ever. **A direction claim I
  made here ("runs the OPPOSITE way to R7", resting on four green
  `04-lib-crdt` stages) was falsified by the next gate run and is
  withdrawn in the row;**
- **U14, new** — four selectors across **four** unrelated subsystems
  (async runtime, optimistic orchestration, editor composition, LSP
  dispatch) red in one gate run, spread over three stages;
- **U15, new** — the rotated cluster 40 minutes later. It carries a
  single `/proc/loadavg` reading of **34.04**, which makes severe
  unrelated load a **measured presence contemporaneous with a multi-red
  run — not a measured cause.** The reading is one point taken after the
  fact and the margins are not monotonic
  (`composition_overhead` ran 1.182x, 1.592x, 1.527x), so no
  dose-response is claimed. **It is the first contemporaneous load
  reading for a U6 occurrence, and a second data point beside the one
  U7 has carried since 2026-08-09** — not this registry's first. Two
  earlier versions of that write-up overreached: one said a load average
  of 34 "explains it without any help", the other that U6 and U7 had
  both wanted a number since August. Both are corrected in place.

- **U16, new** — a `git` invocation in `packages::fetcher` found its
  working directory **deleted**. Not a budget: the only row in the
  registry that arrives with a **named candidate mechanism inside the
  test suite**, and the load-bearing step is **child inheritance**.
  `src/file_io.rs:434` mutates process-global cwd; concurrently
  `run_git` calls `run_git_inner(None, …)`, which sets `current_dir`
  only when `cwd` is `Some` (`fetcher.rs:329`–`:330`), so the spawned
  `git` **inherits** the temp cwd. **Restoring the parent's cwd does
  nothing for that child**, and the `TempDir` then drops underneath it.
  Candidate, not a demonstrated chain — 8 full parallel `--lib` runs did
  not reproduce it, which establishes **intermittence and nothing
  more**. The controls that would settle it are in the row and **none is
  run here**; note that a serial guard around `set_current_dir` tests is
  *not* among them, since the child outlives the guard. The structural
  fix belongs to whoever owns `file_io`, not to a CRDT invariant lane.

- **R6's SECOND occurrence** — 26 days after the first, macOS `lua54`,
  a **full three-condition match** including both required fragments.
  **The log was read before anything was rerun**, which is U3's lesson
  and U8's fourth-violation warning finally honoured on a macOS job. A
  **merge-base control was dispatched** at `aae5b35` rather than
  arguing from an unrelated diff. **Not the `workflow_dispatch` key's
  first use** — #245's own D2/D3 witnesses dispatched three runs right
  after it merged — but **the first use for a live merge-base
  control**, which is the case U11 motivated it for. It came back **green on the macOS legs**, so the
  inference it could have supplied is **unavailable**; recorded as a
  null result, as R1's row had to record its own;
- **U17, new** — that same control run **redded `Test (crdt)` on `main`
  at `aae5b35`**: `read_dir_supersede_cancels_in_flight_predecessor`,
  `first read_dir must be superseded; got ok`. It fails the **opposite**
  way to R1 and R5 — not a missed deadline. What `got ok` proves is
  narrow: the predecessor **completed successfully before cancellation
  took effect**, which does not say when the supersede arrived. The job
  runs `--test-threads=1`, which serializes test **functions within one
  executable** — **not** the test-**binary** concurrency U9's control
  named, and cargo runs binaries serially anyway. A PR run could show
  this failure too; what only a `main`-side run establishes is that it
  fails **on `main`**, with no observing branch to suspect.

U14 and U15 are two rows rather than one because the second run's
selector set had **rotated**, and this registry matches on the exact
set — recording it as a second U14 occurrence was a matching-rule
violation, caught in review.

**Two claims THIS BLOCK made are corrected by measurement:**

- it said the fixture had verified that *"replicas stay converged — the
  op IS broadcast"*. **That was inspection of the call sites, not
  execution.** Nothing had ever replayed the op on a replica, and text
  equality alone cannot detect a lost version advance — the drop-the-op
  mutant leaves the text identical, and the failure that catches it
  reads `version vector diverged undo`. C3 establishes convergence
  properly, by seeding a replica with the forward ops first;
- it said the buffer-end range location was *"genuinely arbitrary either
  way"*. **The §4 census rules it**: five consumers inert, three
  permitted, none harmed — and for `TextView`, the one whose cost
  depends on the location, the buffer end is the **cheapest** rebuild.

**What the four review rounds caught, none of it by me.** Revision 1
posed the decision instead of answering it, and its C3 passed its own
drop-op mutant. Revision 2's C4 contradicted the implementation
(`mark_stale` is unconditional and range-independent) and named two
consumers out of six. Revision 3's C4 claimed guard mutations that
**survive** — at the buffer end, deleting the fold or style guard
changes nothing — and its C9 guarded a file set and a count, which a
same-file substitution walks straight through.

**15 mutation checks were run, each on a clean tree and reverted.** All
behaved as the framing predicted, including the two asymmetries the
framing states rather than assumes: C2b is **masked** for the Insert and
Delete forms by their defensive early returns (`buffer.rs:1177`,
`:1192`) and **dies** for Replace, which has none; and C4b **survives**
the style-guard deletion, which is why C4c injects an INTERIOR empty
edit where the fragmenting is reachable.

**Deliberately not done:** the proptest regression seed is NOT
committed. It duplicates a deterministic fixture and would make a
disputed assertion fail permanently rather than occasionally.

**It does NOT reorder the roadmap.** GUI arc 1b remains the next product
lane per `COHERENCE.md` §20.

### Superseded lane state, kept for the record

**Written with the branch's FIRST commit**, per the standing correction
from #171 and #215.

- **Branch `ci-manual-dispatch`**, base `githubsucks/main` @
  **`2e9f62b`** exactly, in worktree
  `/home/jeans/Repos/personal/pmacs-ci-dispatch`.
  **`githubsucks/ci-manual-dispatch` is the authoritative tip** (the
  ref, not a SHA).
- **Framing `docs/ci-manual-dispatch-framing.md`, revision 3 —
  APPROVED.** It took three revisions and every one was a correction:
  revision 1 claimed four registry rows needed this (**one does**),
  revision 2's D2 could pass without the two runs ever overlapping, and
  its D3 accepted an aborted job as proof the matrix ran.
- **What it does:** adds `workflow_dispatch:` to `ci.yml`. One key. No
  job, matrix, step, permission or timeout changes, and **no inputs**.
- **Why, and the scope is ONE ROW.** **CI never invokes `scripts/gate`
  — zero occurrences.** Every local-gate red therefore has a merge-base
  control needing no CI at all. **U11 alone needs this**, because it is
  macOS-specific and this project has no Mac: when it recurred on #243
  the only contemporaneous `main` control available was a re-run of a
  run **eight days old**.
- **D1 PASSES pre-merge and BITES**: the working-tree file parses, and
  `workflow_dispatch` is a key under `on` — dropping it fails the
  assertion. Note `on:` parses as the **boolean `True`** under YAML 1.1,
  so the witness looks up both keys; a naive `d['on']` would raise
  before it ever checked anything.
- **PR #245 CI: 13/14, red on `Test (macos-latest / lua54)`.** The
  failure is **R1** — `supersede_cancels_in_flight_job_within_50ms`,
  fragment `supersede did not cancel within 50ms` — an existing row,
  matched by selector and fragment. Its recorded flavor is `luajit` and
  this is `lua54`; **flavor is not part of signature matching**.
- **The merge-base control was run rather than the PR job.** The same
  job at the branch's exact base `2e9f62b`, rerun the same hour: green,
  `1985 passed; 0 failed`, both logs preserved. **A green control does
  not establish environmental cause and does not retire R1**; a red one
  would have shown the branch did not introduce the occurrence, and
  that inference is unavailable. R1 stays live under its
  measurement-design disposition.
- **R1's assertion also omits its measurement**, which surfaced in the
  #244 sweep I discarded as noise. **It is deliberately NOT fixed
  here**: it would sharpen the next failure and repair nothing about
  the measurement design R1 is actually about. That is the
  async-runtime lane's, with Q#MCI3.
- **D2 AND D3 ARE OWED POST-MERGE**, not skipped. GitHub offers
  `workflow_dispatch` only for a workflow already on the default
  branch, so neither can run before this lands. Their exact procedures
  are in the framing §6, including the void-and-retry rule that stops
  D2 passing when the two runs never overlap.

## Parse-budget diagnosability — MERGED as #244 (`a85205a`)

- **MERGED 2026-08-29T22:04:17Z** at approved head `7b50682`, merge
  commit `a85205a`, via `--match-head-commit`. **14/14 CI green** on
  that head and the 8-stage local gate green on it too.
- **Both `duration_ms < 100` assertions now report the observed value
  and the budget.** Neither budget moved.
- **THREE ROWS IN ONE SESSION were written to assert a condition
  without keeping what would explain its violation** — the two parse
  budgets fixed here, and **U13**'s `let (out, _, _)`, which discards a
  child's success status and stderr so its durable failure cannot
  distinguish wrong-but-successful output from a refused invocation.
  U13 is not this lane's and is left as recorded, but the pattern is
  now a pattern rather than an oversight, and each occurrence costs a
  review round to establish nothing.
- **R7 gained its sixth and seventh occurrences here**, on a branch
  touching no `pmacs-gpu` file, and the registry gained a **bounded
  observation window** so the in-gate/out-of-gate ratio cannot drift
  with review activity — plus a correction: seventeen green
  out-of-gate runs are **not** exclusions, because nothing outside the
  gate has ever reproduced R7 at all.
- **U13 was recorded during this lane's review** and is unrelated to
  it: `scripts/gate` and `tests/gate_script_acceptance.rs` are
  byte-identical to the base.

### Superseded lane state, kept for the record

**Written with the branch's FIRST commit**, per the standing correction
from #171 and #215.

- **Branch `parse-budget-diagnosability`**, base `githubsucks/main` @
  **`3557779`** exactly, in worktree
  `/home/jeans/Repos/personal/pmacs-parse-budget`.
  **`githubsucks/parse-budget-diagnosability` is the authoritative
  tip** (the ref, not a SHA). Recover with `git fetch githubsucks &&
  git checkout parse-budget-diagnosability`.
- **Framing `docs/parse-budget-diagnosability-framing.md`, revision 2 —
  APPROVED.** Revision 1 was reviewed and had two defects worth
  keeping: it claimed `dispatch_parse_round_trips_a_rust_source_file`
  was the codebase's **sole** measurement-omitting assertion (false —
  `tests/m4_acceptance.rs:244` is the same budget on the same
  measurement), and it bundled `workflow_dispatch`, which this ledger
  had already recorded as its own lane.
- **What it does:** both `duration_ms < 100` assertions now report the
  observed value and the budget. **Neither budget moves.**
- **Why:** `dispatch_parse_round_trips_a_rust_source_file` redded twice
  on macOS/`lua54` — U11, then again on #243 — and **both margins were
  unrecoverable**, so the second red could not be compared with the
  first. A 1ms overshoot and a 900ms overshoot are different failures
  and produced identical logs.
- **D1/D2 verified against REAL PANIC MESSAGES**, by forcing only the
  comparison bound to `0` in a scratch build:
  - `trivial parse should be fast: took 0ms against a 100ms budget`
  - `200-line parse should be quick: took 11ms against a 100ms budget`
  The second is the stronger demonstration: a non-zero observed value
  cannot be mistaken for a literal.
- **THE SCRATCH PANIC PROVES ONLY HALF.** It exercises a budget of
  **0** while printing `100ms`, so it says nothing about the committed
  threshold. **D3 carries that half separately** by pinning the literal
  `100` in both files. The two are a proof together; neither
  substitutes for the other.
- **NOT an assertion-hygiene audit.** The framing's §3 withdraws
  revision 1's completeness claim rather than repairing it: a sweep
  wide enough to be complete also catches `Instant::now() < deadline`
  loop guards and `eval::<bool>` turbofish, and a sweep narrow enough
  to be accurate proves nothing about completeness.
- **GATE RUN 1 IS NOT A RESULT.** It ran in a background task that was
  killed at 314s; `07-sweep.log` ends in `Terminated`. Stages 1–5 and 8
  were green and stage 6 (`gpu`) genuinely failed — that one completed
  and reported — but the run as a whole proves nothing and must not be
  read as a gate outcome.
- **Those `gpu` failures are R7's SIXTH and SEVENTH occurrences**, in
  `docs/ci-red-signatures.md`. All three required fragments, isolated
  selector green three times, and **this lane touches no `pmacs-gpu`
  file at all** — its whole diff is two `assert!` message strings and
  three docs.
- **GATE RUN 2: 7/8, `gpu` red on R7 again** (head `45d438c`, log
  `20260829T150011Z-429115`). `sweep` ran fully green this time.
  **Two consecutive in-gate R7 failures prompted a narrowing**: 17 green
  runs outside the gate, in four configurations, never reproduced it.
  **A third in-gate run was then GREEN**, so "in-gate always fails" is
  false and the registry's first write-up — which called those runs
  *exclusions* — is corrected there: nothing outside the gate has ever
  reproduced this, so matching one gate condition at a time outside it
  cannot isolate an in-gate cause. In-gate is 2 failures in 4;
  out-of-gate is 0 in 17, over a **bounded observation window** the
  registry defines so the ratio cannot drift with review activity.
  Recorded there; **not this lane's to solve**.
- **PR #244** — https://github.com/levineuwirth/pmacs/pull/244.
- **Every review round ends with a head-exact 8-stage gate**, green
  each time, with `HEAD` and a clean worktree captured before and after
  and the result read from the eight stage logs rather than inferred
  from stage exits. **The PR body carries the current head and log
  id**; they are not duplicated here, because a docs commit answering a
  review moves both, and a ledger line naming them is stale the moment
  it is written — the lesson §5b learned twice.
- **Review follow-up found U13, separately recorded.** On signed head
  `756c2b8`, gate `20260829T171606Z-1087848` was 7/8: `07-sweep`
  reddened only
  `gate_script_acceptance::skipped_directories_are_reported_with_a_reason`.
  The child command's stdout was empty, but the row discards its stderr
  and status, so the mechanism is unrecoverable. The exact selector and
  the full 36-test binary both passed immediately afterwards —
  intermittence only. This lane changes neither the gate script nor
  that acceptance binary; diagnostic hardening is a separate lane.
- **Still owed, separately:** `workflow_dispatch` on `ci.yml` (**landed
  as #245**), and U9's discriminating control — named since 2026-08-09,
  never run, and now **VOID**: its premise that `cargo test --workspace`
  runs many test binaries at once is false, cargo runs test targets
  serially, so "pin test-binary concurrency to 1" pins something already
  1. See the correction on U9 in `docs/ci-red-signatures.md`. **A
  replacement control has to be designed**; the budget family no longer
  has one written down.

## Panel-pointer replay (parent acceptance 48) — MERGED as #243 (`6c9bae6`)

- **MERGED 2026-08-29T10:37:10Z** at approved head `b8c51b7`, merge
  commit `6c9bae6`, via `--match-head-commit` so the merge is provably
  of the reviewed head. **14/14 CI green on that exact head**, and the
  16-stage local gate green on it too (log
  `20260828T083948Z-492737`), with `HEAD` and a clean worktree
  captured before and after the run.
- **Acceptance 48 is now implemented.** Panel selection, terminal
  mouse reporting and the vertical wheel all replay; the gesture also
  *ends* correctly, which was the larger half. The horizontal
  document-panel wheel remains a **named deferral to GUI Stage 1b
  B1–B3**, not a gap.
- **GUI arc 1b is UNBLOCKED** and rebases onto this merge.
  `docs/gui-stage1-input-framing.md` revision 13 already amends B1–B3
  to own the panel-document horizontal surface that this lane's
  re-measurement exposed as unowned.
- **`docs/ci-red-signatures.md` gained U12** through this lane, and it
  arrived with the merge rather than needing separate absorption.
- **Two follow-ups are OWED and deliberately unstarted**, both their
  own lanes:
  - **`workflow_dispatch` on `ci.yml`.** A merge-base control could
    only be obtained by re-running an existing week-old job; there is
    no way to run CI on `main` on demand. That shaped the whole U11
    recurrence handling.
  - **The `duration_ms` diagnostic.** `dispatch_parse_round_trips_a_rust_source_file`
    has now redded twice on macOS/`lua54` (U11, then again on #243)
    and passed on rerun both times, with `src/async_runtime.rs`
    byte-identical to main throughout. **Both margins are
    unrecoverable**, because the assertion omits the measured value —
    so occurrence two cannot be compared with occurrence one, and a
    third will be no better. Intermittence is established; cause and
    magnitude are not.

### Superseded lane state, kept for the record

**Written with the branch's FIRST commit**, per the standing correction
from #171 and #215 — the correction the 1b lane missed, honoured here.

- **Branch `panel-pointer-replay`**, base `githubsucks/main` @
  **`72da24a`** exactly, in worktree
  `/home/jeans/Repos/personal/pmacs-panel-replay`.
- **Checkpoint at merge: framing revision 16 APPROVED, implementation
  complete.** No count or SHA was recorded here while the lane ran,
  for the reason the §5b lane learned twice.
  - **LANDED: Q#BP-R4's pre-effect disposition and the lifecycle
    table** — `PanelPointerOutcome` (`Refused`/`Consumed`/`Accepted`)
    decided before any target effect, with the resolution carried so
    the daemon never re-derives.
  - **LANDED: G5k's recorded gesture domain.** The first version was
    reviewed and rejected: it drove tails through the mode-sensitive
    adapter, which re-reads Shift, scroll position and the child's
    modes per event — G5k's named mutation. The press now records
    `PanelGestureDomain` (document / terminal-child-with-encoding /
    terminal-local) and every tail and completion follows it.
  - **LANDED: the record is SELF-CONTAINED.** `TerminalLocal` carries
    the accepted content **viewport** (ambient geometry is `None`
    exactly when a hidden panel needs completing); a press that anchors
    nothing no longer arms on either target; and pointer routing goes
    through the renderer's own `terminal_projection_size` clamp — a
    band wider than `MAX_TERMINAL_COLS` painted fine while every click
    inside it resolved to nothing.
  - **LANDED: `PanelPointerDisposition` is an ENUM.** As
    `{outcome, Option<target>}` the invalid pair — refused, yet
    carrying a target — stayed representable inside `editor.rs`. Now
    `Refused` holds no target at all.
  - **Witnesses: G5k(a)–(d), P1, P2, P3 both legs, P4, P5, P7, P8, P9,
    P10, P11, P12**, each biting its own mutation, including G5k's
    verbatim. **They do NOT all read a target effect, and an earlier
    version of this line said they did.** Two kinds:
    - **Effect rows** — G5k(a)–(d), P1, P3 both legs, P4, P5, P7, P8,
      P11, P12 — read the child's byte stream (**exact bytes**, not a
      count), the terminal drag state, or the document selection.
    - **Arming-gate rows** — **P9 and P10** — read the LATCH, and that
      is correct for them: the defect they fence is a record existing
      for a gesture that never began, so the record IS the artifact.
      Manufacturing an effect assertion for them would not distinguish
      their mutations.
    **P2 is a third case**: its FOCUS assertion is witnessed —
    removing the buffer check accepts the press, which activates the
    panel before replaying — and its classification is checked LAST so
    the row still fails if it stops testing a refusal. Only its
    controller and byte assertions are defence in depth, because that
    mutation routes through a document buffer and touches neither. An
    earlier version asserted the refusal first, which aborted the row
    before dispatch and made every effect assertion unreachable; I
    recorded that ordering limit as a limit of the type boundary, and
    it was not one.
  - **Every fixture asserts its own precondition** (the disposition is
    `Accepted`, or is `Refused`) because four rows in these rounds
    passed vacuously: cells that were out of grid, or that clamped to
    byte 0, exercised a refusal instead of the path they named.
  - **LANDED: the pending-release SLOT and its drain order (task 18).**
    Cancellation parks the record instead of returning it into a
    context that drops it — two of the three cancellation sites are
    inside frame production, where no target effect can run. The drain
    pays it **before any subsequent panel-pointer effect**, **before
    detach teardown**, and **at the projection seam** between
    `render_frame` returning and its messages being written.
    - **A LIVE gesture is ended and PAID before a replacement press
      lands.** The entry drain alone was not enough: it looks for an
      OWED release, and a live gesture owes nothing yet — arming was
      what cancelled it, which happens after the replacement has
      already reached the target. The child saw `old press, new press,
      old release`.
    - **Witnessed: Q1, Q2, Q3, Q4, Q6.** Q3 asserts ORDER, not arrival.
      Q6 sends a second press with the first still live and expects
      exactly `old release, new press` in the child's stream. Both
      layers of Q6 are proven separately: the invariant now asserts at
      the point of ARMING (not inside cancellation) and fires in debug,
      and with that assert compiled out the byte-order assertion
      catches the same defect — which is what a release build relies
      on.
    - **Q5 is CLOSED.** The projection seam is extracted as
      `project_semantic_frame`, which returns its messages **unwritten**
      — so a caller holding them has by construction not sent the
      successor frame, and a release already delivered at that moment
      provably precedes it. The row bites its own drain and no other:
      removing it fails Q5 while Q1–Q4 stay green on the other two
      drain points.
  - **LANDED: the authority-loss matrix (task 19).** §5b wired `Absent`
    and left the other four transitions armed — inert while nothing
    consumed the latch, defects the moment cancellation gained an
    effect. Three are visible in the producer where the declaration is
    built (**window replacement**, **buffer replacement**, and a
    **geometry-epoch change including at an unchanged size**, which
    needed a retained `geometry_epoch` because nothing else the
    producer holds moves with it); **detach** cancels in the dispatcher
    before any teardown.
    - **Witnessed by a TABLE-DRIVEN matrix**: **five** transitions ×
      two families × two targets, **twenty quadrants** — `Absent`
      included, both as the fifth row and as a CONTROL on §5b's own
      cancellation, which a mutation confirms it catches. The count is
      asserted in the row, because a loop that quietly stops covering a
      combination passes exactly as loudly as one that covers them all.
      Each quadrant **drains explicitly and asserts the effect** — the exact release bytes
      for a reporting terminal, the cleared empty selection for a
      document, and an empty slot afterwards. An earlier version
      stopped at `has_pending_release()` and would have passed while
      delivery was broken; the mutation that parks a release and never
      delivers it now fails every row.
    - **Mutation labels, stated correctly this time**: dropping the
      BUFFER comparison misses the buffer transition; dropping the
      WINDOW comparison misses the window transition; dropping the
      geometry comparison misses that one; dropping detach's cancel
      misses detach. Each fails the matrix.
    - **G5m takes both composites the framing names** — changed-size
      geometry (epoch **and** mapping generation) and a buffer
      replacement that also moves the mapping — and reads the child's
      stream rather than a cancellation count, because a count of one
      proves the latch was taken once, not that one release went out.
      It also **asserts the mapping generation actually advances**:
      without that, a same-size geometry change passes as a
      "composite" while being a single cause, and the row would prove
      nothing about coincidence. Peeked rather than read through the
      authoritative accessor, which would advance the key and
      manufacture the second cause.
    - **G5j has two legs and they differ**: an empty selection is
      cleared without moving point, a REAL dragged region survives
      anchor-and-cursor exact. Clearing every selection fails the
      second.
    - **One quadrant asserts less, and says so**: for window
      replacement on a document, the window the gesture belonged to is
      gone, so the completion has nothing left to clear and the
      gesture ending is the whole effect. Written into the row rather
      than left as a silently absent assertion.
  - **GATE RUN 1, head-exact at `6142acc`: 15/16, red on `04-lib-crdt`
    only.** `composition_overhead_under_ten_percent` (1.247× against
    1.10×) and `setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
    failed together; both green on isolated rerun. Recorded as **U12**.
    `src/process.rs` is not touched by this branch at all. **The run
    was knowingly taken on a machine that was quieter but not quiet** —
    load 11.04 at start, 27.79 five-minute at end, two foreign `python`
    processes throughout — so it is reported as a red under stated
    conditions rather than dismissed.
  - **REMAINING: a head-exact gate on a genuinely quiet machine, then
    the PR.** The gate
    wants a QUIET machine: a foreign C++/java build has been running at
    load 114+ through this work, and the wall-clock rows
    (`composition_overhead_under_ten_percent`,
    `m6_2_pty_streaming_respects_byte_ceiling`,
    `full_buffer_summary_flatten_scales_on_large_grammar_file`) redded
    under it and were green in isolation every time. That is U6/U9/U10
    territory and running the gate into it would manufacture another
    rotating-red incident.
  - **Two test seams added for this:** an opt-in child-input tap
    (`start_send_tap_for_test`) and a drag-state read
    (`view_is_dragging_for_test`). Nothing else exposes what the child
    actually received, which is what these rows must assert. §5a's **pre-merge** replay contract was approved at
  revision 12; revisions 14–16 are the post-merge amendment now under
  review. Revision 13 ruled Q#BP-R3 and blocked the lane on a
  protocol-bearing mapping generation; **that block is DISCHARGED** —
  the slice merged as #242 (`47b5463`).
- **MERGED main into this branch** at `b758c2e` rather than rebasing:
  the lane's 12 commits include 10 framing revisions over the same
  800–1000 line doc regions, so a rebase meant twelve rounds of
  large-block resolution — the operation that produced a committed
  diff3 marker on the last lane. Workspace compiles clean, **1964 lib
  and 284 GPU tests pass**. Base is no longer `72da24a`; read it with
  `git merge-base githubsucks/main HEAD`.
- **THE MERGE CREATED ONE DEFECT AND SURFACED ONE COLLISION.**
  - **Defect, fixed at `cf78385`: `b758c2e` DOES NOT COMPILE**, and its
    message claims "Workspace compiles clean". **That claim is
    withdrawn.** I staged the resolution, hit `cannot find value mods`
    at the mapped arm, fixed it, re-checked clean — and committed
    without re-staging, so the verification and the commit were of
    different trees. Not amended away: `b758c2e` keeps its false claim
    with the withdrawal attached, because erasing a bad record is worse
    than carrying a corrected one. **Anything bisecting across
    `b758c2e`..`3cd7b8a` will fail to build.**
  - **Defect, fixed:** the merge kept BOTH copies of §5b — this
    branch's stale pre-split one and main's authoritative one. I
    discarded the uncommitted stub edit as obsolete and missed that its
    **deletion** half was still owed. Removed; exactly one §5a and one
    §5b remain.
  - **Collision, ruled by revision 14 and NOW FIXED IN CODE** (see the
    checkpoint above; this bullet records what it was):
    §5b and this lane gave `dispatch_semantic_panel_pointer`'s `bool`
    different meanings — accepted-as-a-gesture versus consumed-here. A
    mode-line press therefore **armed the latch** --- past tense: the
    fix landed with Q#BP-R4, and P1 pins it.
    **Q#BP-R4** rules a three-state `PanelPointerOutcome`, classified
    **before** target effects. Only an `Accepted` `Down(Left)` arms;
    left `Drag`/`Up` require a live record; an accepted `Up` performs
    ordinary replay once, a consumed/chrome `Up` performs the recorded
    completion once, and a refused `Up` preserves the record.
- **Revision 16 carries** the rows §5b's split table assigned here, a
  **pending-release slot** for the cancellation record §5b leaves
  nowhere to wait, and the **four transitions** that strand a live
  gesture once effects attach. Drain order is executable: before the
  next panel-pointer effect; before detach teardown; and, for a
  projection-raised cancellation, after `render_frame` returns but
  before any returned message is written. Ground truth is re-measured
  at `2c0d3ff`. Document-panel horizontal scrolling is a named deferral
  to GUI Stage 1b B1–B3; a horizontal tick whose terminal precedence
  selects child reporting already emits SGR, and the local terminal
  branch has no horizontal viewport effect.
- **THE BLOCKER, and why the earlier acceptance failed.** A
  `PanelPointer` names a cell; nothing on the wire says which inverse
  mapping the frontend saw, so the daemon inverts against whatever is
  current. Revision 12 accepted that on three bounds and **all three
  were wrong**: a **foreign** edit moves the mapping with `view_top`
  untouched, the error is **unbounded** once ticks/folds/edits/reloads
  accumulate, and the window lasts until the frontend **presents** the
  new frame. §5b adds a **cell-mapping generation** — moves with the
  inverse mapping, stable across focus/styling/cursor/selection-only
  repaints so drags survive — as **appended** wire variants with
  bilateral gating.
- **Chain: ~~§5b (protocol) →~~ panel replay → GUI arc 1b.** §5b took
  **v25**, so 1e's `OpenTarget` is **v26**. **That edit is NOT owed by
  the 1b branch** — this bullet said it was, and said making it here
  would collide at 1b's rebase. **§5b made it, in
  `docs/gui-stage1-input-framing.md`, and merged as #242**: a canonical
  document saying v25 is false the moment v25 is taken, and an expected
  rebase conflict was not grounds for leaving it wrong. Corrected here
  because this bullet and the merged-#242 block below it were saying
  opposite things. Commit one was the ground-truth
  re-measurement; 6 added the four replay edges; **7 answers review of
  6; **8 answers review of 7** — R-c is target × gesture-ORIGIN
  (terminals reject all chrome kinds and raw chrome coords fail the
  reporting bounds check), R-c2 retains the `Down` cell, A3–A5 add
  positive SGR controls, and four witness seams are tightened. **New
  ruling Q#BP-R2**: a chrome wheel over a terminal panel clamps into
  content rather than dropping — a deliberate divergence from the TUI,
  flagged for overrule — **and overruled in 9**.
  **Revision 9** reverses two of 8's decisions: a terminal-chrome wheel
  is **CONSUMED, not clamped** (SGR wheel input is coordinate-bearing,
  so clamping fabricates a hit the user never made, and a wheel has no
  liveness obligation), and the `Down`-cell fallback lives in a
  **separate `gesture_last_content_cell`** rather than
  `last_pointer_cell`, which is cleared on press precisely so the first
  same-cell `Drag` reaches the daemon (`pmacs-gpu/src/main.rs:19841`).
  **`Up` is the only crossing event promised unconditionally**; a
  crossing `Drag` is normalized and then deduped.
  **Revision 10** keeps Q#BP-R2's outcome and moves its **enforcement
  point**: the GPU **cannot know a panel holds a terminal** —
  `PanelFrame` has no target-kind field
  (`pmacs-protocol/src/panel.rs:73`) and `state.terminal` is the
  primary full-window terminal — so a producer-side rule needed a new
  wire field this lane must not add. **The producer is target-blind**
  and sends the chrome wheel for every panel; **the daemon** resolves
  the side window and decides: document → `scroll_window`, terminal →
  consume. Witness: one frontend across a document→terminal
  replacement.
  **Revision 11** fixes the ORDERING: the terminal-chrome wheel is
  consumed **before activation**, not merely before
  `apply_terminal_gesture`. `activates` is `!matches!(kind, Move)` for
  a terminal (`src/editor.rs:2695`), so the wheel already writes focus
  and `active_frontend` at `:2699` ahead of any replay decision — a
  consume check below that would change focus while scrolling nothing
  and claiming no controller. Four-step order, consumption at step 3;
  the witness now asserts **focus and controller identity unchanged**.
  **Revision 12** makes that setup discriminating: leg 2 must **start
  PASSIVE** — primary document window active, terminal side window
  distinct and passive, controller baseline captured — because "focus
  unchanged" is vacuous if the panel is already focused, and the
  below-activation mutation would then call `focus_window` on the
  already-active window and pass. The two assertions are **not
  interchangeable**: **focus** catches the ordering mutation;
  **controller identity** catches the shared-path mutation, since
  `apply_terminal_gesture` claims at `src/editor.rs:3571` before local
  handling and activation alone claims nothing.
- **Why this lane exists, re-measured at `2c0d3ff`.** The branch now
  replays document selection, terminal mouse reporting and vertical
  wheels; the remaining acceptance-48 effect is **listview row
  selection**. Q#BP-R4 and §5b's inherited rows still need
  implementation: pre-effect disposition/latch ordering, fixed-domain
  gesture tails, exact-once termination, cancellation effects and the
  pending-release drains. **GUI arc 1b is BLOCKED on this lane and
  rebases onto its merge commit.**
- **No new framing document.** Acceptance 48 is already ruled in
  `docs/bottom-panel-framing.md`; §5a adds ground truth to it.
- **Current clause split.** DONE: click/focus and terminal activation;
  focused-only auto-scroll with passive `view_top` preserved; lossless
  and coalesced event delivery; panel document selection; terminal
  child reporting/local selection; vertical document and terminal
  wheel effects. MISSING here: listview row selection and the
  lifecycle/cancellation effects above. Horizontal wheel is split:
  child-reporting terminal ticks already emit codes 66/67; the local
  terminal branch is deliberately inert; document-panel `view_left`
  is explicitly GUI Stage 1b B1–B3's effect, matching the production
  comment in `src/editor.rs:2999`–`:3003`.
- **The scoping hazard.** `set_cursor_byte` (`src/editor_core.rs:1216`),
  `begin_selection` (`:4691`) and `clear_selection` are
  **active-window scoped**; used naively they would move the
  DOCUMENT's point, which AC48 forbids. Activation runs before replay
  in the same dispatch — **necessary but NOT sufficient**, which is
  what revision 5 got wrong.
- **FOUR REPLAY EDGES (revision 6), each a place a plausible
  implementation is silently wrong. Three have a precedent in the tree
  the panel path simply does not use.**
  - **R-a — modifiers dropped.** The daemon destructures `mods` into
    `..` (`src/daemon.rs:2425`) and the dispatcher has no modifier
    parameter, but `apply_terminal_gesture` gates child reporting on
    `!shift && … && modes.mouse_sgr` (`src/editor.rs:3534`). **Shift is
    the user's local-selection override**, so a Shift-drag over a
    reporting terminal panel would send SGR to the child. Thread
    `mods`; row: Shift-drag selects locally, child receives no bytes.
  - **R-b — `Drag`/`Up` do not activate**, and another frontend can
    interleave between them, so replay must not read ambient
    active-window state. Name a **side-window cell→byte adapter** and a
    window-TARGETED selection path. `activate_and_position`
    (`src/editor.rs:3795`) is the precedent *and* the trap: its
    conversion is window-scoped, but it calls `set_active_window_id`.
    Rows: interleaved frontend B between A's Down and Drag/Up; orphan
    Drag/Up on a passive panel leaves the document mirror
    byte-identical.
  - **R-c — `panel_grid_size` is the FRAME, not the viewport.** Content
    is `rows − 1` (`src/editor.rs:2499`–`:2500`) but `panel_hit_test`
    reports across the whole frame (`pmacs-gpu/src/main.rs:7184`), so a
    `PanelPointer` can name the **mode-line row**. Terminal viewport is
    `rows − 1`. **The document rule is PER KIND, not "the row is
    inert"** — the TUI guards `Down(Left)`/`Drag(Left)`/`Down(Right)`
    and deliberately not `Up(Left)` or the wheel, so a blanket rule
    would stop mode-line scrolling and leave a content-started gesture
    unterminated. **The producer must also not arm** on a mode-line
    press (`pmacs-gpu/src/main.rs:2878`); a receiver-only rule cannot
    stop the resulting orphan.
  - **R-d — replacement leaves the gesture latch armed.** `Absent`
    clears `pointer_held`/`last_pointer_cell`
    (`pmacs-gpu/src/main.rs:6909`) but **`Present`→`Present` does not**
    (`:6913`). A press on A then A→B emits a Drag/release for B with no
    B press, and **acceptance 49 cannot reject it** — the event carries
    B's *current* epochs. The **divider** drag latch already
    epoch-scopes itself (`:7288`); the pointer latch never did. **Both
    epochs**: a font/scale change advances `geometry_epoch` while
    `panel_epoch` holds (`pmacs-protocol/src/panel.rs:61`) and clears
    neither field, so a held gesture resumes under a new grid with
    valid epochs. **Both fields**: clearing only `pointer_held` leaves
    the successor's first same-cell `Move` suppressed as a duplicate
    (`:7238`). Four mutations, including the negative one — an ordinary
    same-identity refresh must NOT cancel a live gesture.
- **RULED: Q#BP-R1** — a single click **SELECTS a listview row only**.
  RET/SPC remain activation (`listview.lua:610`); no click-to-visit and
  no double-click-to-visit. Keeps document navigation from becoming an
  incidental consequence of replay.
- **Gates:** the four `bottom_panel_*` acceptance suites plus
  `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`. **No `--protocol`** —
  `PanelPointer` and every `MouseKind` it carries already exist on the
  wire.
- **Expected rebase conflict, flagged deliberately:** the `gui-stage1b`
  branch inserts its own lane at this same position and corrects three
  stale headers below (#239/#240 still marked OPEN, "1a is next").
  Those corrections are **left to that branch**; this lane does not
  duplicate them. The conflict is a normal insertion collision.

## Panel cell-mapping generation (v25) — MERGED as #242 (`47b5463`)

- **MERGED 2026-08-20T17:25:01Z** at approved head `61f0faf`, merge
  commit `47b5463`, via `--match-head-commit` so the merge is provably
  of the reviewed head. **14/14 CI green on that exact head**, and the
  full 16-stage local gate green on it too (log
  `20260820T163107Z-879828`), with `HEAD` and a clean worktree captured
  before and after the run.
- **`PROTOCOL_VERSION` is now 25.** `PANEL_MAPPING_MIN_VERSION` is a
  literal 25; `ADVERTISED_PROTOCOL_VERSION` stays pinned at **20**. GUI
  arc 1e's `OpenTarget` moved to **v26**, corrected in
  `docs/gui-stage1-input-framing.md` by the slice itself.
- **Replay effects remain `panel-pointer-replay`'s** — this slice
  shipped the trigger and the state, not the effects. That lane is now
  unblocked and is the next step in the arc.
- **Review round 4 fixed one functional defect**: both inbound arms
  discarded the dispatcher's bool and updated the accepted-gesture
  latch unconditionally, so a rejected `Down` armed and a rejected `Up`
  consumed a real gesture. Both arms now gate on the return, and
  `dispatch_semantic_panel_pointer` is `#[must_use]` so a future
  discarded answer is a clippy failure. Four rows cover it, each with a
  positive control, and three mutations each bite their named rows.
- **`docs/ci-red-signatures.md` gained U10** from this lane's gating.
- **U11 is recorded below from PR #242's comment**, per the deferral
  agreed during review.

### Superseded lane state, kept only for the record

- **BASE IS `5f2015c`**, current `githubsucks/main`. Rebased there
  cleanly, no conflicts; the two commits picked up are docs-only (the
  `rd_precondition` occurrence record and the handoff's `timeout`
  rule).
- The **first** rebase, onto `f13506c` (post-#241 main), took one
  conflict in this file: main's merged-#241 block and this lane's own
  first-commit block landed at the same position; both kept, active
  lane above merged. No code file has overlapped across either rebase —
  #241 touched `scripts/`, `tests/common/` and two test suites; this
  lane touches the protocol, daemon and GPU sources.
- **NO AHEAD-COUNT OR TIP SHA IS RECORDED HERE, deliberately.** Both
  were wrong within one commit the last two times they were written,
  because the commit that updates this file is itself the commit that
  invalidates them. Read them instead:
  `git log --oneline githubsucks/main..HEAD | wc -l` and
  `git rev-parse HEAD`. Immutable EVENT shas — the rebase, a fix, a
  merge — are recorded; the moving tip is not.
- **REVIEW ROUND 4 (2026-08-20) — three blockers, all fixed.**
  - **FUNCTIONAL: rejected gestures mutated the accepted-gesture
    latch.** Both inbound arms discarded the bool from
    `dispatch_semantic_panel_pointer` and called
    `update_accepted_gesture` unconditionally. The ladder authenticates
    the SENDER; only the dispatcher re-derives the TARGET, so an event
    can clear every rung and still be refused for an out-of-grid
    coordinate, an absent side window, or a buffer that is no longer
    the one in the side window. **A rejected `Down` therefore ARMED** —
    so a later authority loss counted a cancellation for a gesture that
    never began, and once replay attaches effects it would release a
    child that was never pressed. **A rejected `Up` CONSUMED a real
    armed gesture** — so the authority loss that should have ended it
    found nothing, and that child holds the button down for good. A
    rejected `Down` on top of a live gesture was worse still:
    `arm_accepted_gesture` ends what it overwrites, so it also counted
    a spurious cancellation.
  - **Fixed** by gating both arms on the return, and by marking the
    dispatcher `#[must_use]` so the class cannot recur silently —
    clippy runs with `-D warnings`, so a future discarded answer is a
    build failure rather than a review finding.
  - **Four new rows**, `g5_substrate_a_refused_{press_never_arms,
    release_never_consumes}_on_the_{legacy,mapped}_arm`. Each drives
    the refusal from a coordinate ONE PAST the last grid row and **ends
    in a positive control differing only in that coordinate** —
    without it a row would pass just as well if an unrelated rung had
    dropped the event, which is how a negative test of this shape rots.
    Mapped rows read the generation through the validator's own
    accessor, so a mapping-rung refusal cannot masquerade as a
    dispatcher refusal. **Three mutations, each biting its named rows:**
    ungating the legacy arm fails exactly the two legacy rows, ungating
    the mapped arm exactly the two mapped rows, and relaxing the
    dispatcher's `>=` to `>` fails all four.
  - **DOCS: committed conflict residue** — an orphaned diff3
    `|||||||` ancestor line survived an earlier resolution in this
    file, with the other three markers removed and both sides' content
    kept. Deleted; the section below it was the only copy.
  - **THE GATE CANNOT SEE THAT CLASS, and this is not fixed here.**
    `git diff --check` compares the WORKTREE against the index, so a
    marker that is already committed passes a clean-worktree gate
    forever. `git diff --check githubsucks/main...HEAD` exits 2 on it.
    Widening the gate's check to the branch range is a **candidate
    lane** — a gate change owes a framing doc, and it should not ride
    in on a §5b review round.
- **The blocker is gone.** #241 fixed the `sweep-crdt` defect this lane
  was held behind, and the SIGINT guard now refuses a gate run started
  with `SIGINT` ignored — which is what made stage 15 unreachable.
- **FULL 16-STAGE GATE GREEN**, log `20260820T153017Z-147411`, one
  unsplit `./scripts/gate --protocol` run with all six touched
  acceptance suites. `sweep-crdt` reports **zero** failures.
- **THE TWO `m4_24_*` FAILURES WERE MY OWN MEASUREMENT ERROR, and the
  "CI is the arbiter" framing built on them is WITHDRAWN.** I had run
  the CRDT sweep OUTSIDE `scripts/gate`, so it inherited
  `TMPDIR=/tmp` instead of the gate's isolated one, and I then reported
  the local sweep as unable to go green for pre-existing reasons. It
  goes green. CI never needed to substitute for it.
  - **MECHANISM, and it is the exact hazard #240 fixed:** `/tmp/.git`
    exists on this machine (an empty directory, 2026-08-15). Project
    detection **walks upward**, so every `tempfile::tempdir()` under
    `/tmp` inherits `/tmp` as a project root. Both failing rows are
    base-resolution rows — `m4_24_bare_string_glob_stays_relative` and
    `m4_24_d3_fallback_base_is_the_smallest_attachment_dir` — so both
    resolve against the wrong base and fail. CI's `/tmp` carries no
    such marker, which is why they passed there.
  - **The gate's `TMPDIR` is disk-backed under `<gate-root>/tmp/` on
    btrfs, in a directory the gate owns**, with no stray ancestor.
    Running the sweep by hand defeats exactly the isolation #240 built
    for this signature.
  - **`/tmp/.git` IS STILL THERE and is not mine to delete** — it is
    outside the repo and I did not create it. Any hand-run test that
    resolves a project root from a temp dir will keep failing on this
    machine until it is removed.
- **HEAD-EXACT GATING IS BLOCKED BY THE BUDGET-ROW FAMILY, recorded as
  `docs/ci-red-signatures.md` U10.** Two consecutive gate runs at
  `70b334d`, worktree verified clean before and after each, were
  **15/16 green each** — and the red **rotated**: run A took
  `13-sweep` (`dired_open_renders_10k_entries_under_200ms`,
  263.961465ms against 200ms), run B took `15-sweep-crdt`
  (`criterion_1_end_of_line_typing_completes_sub_frame_per_keystroke`,
  1.044609ms against 1ms). **Each red is green in the other run**, and
  both are green isolated at load 9.34. Neither path is touched by this
  branch. The earlier `20260820T153017Z-147411` all-green run was on
  the same code, one docs commit earlier.
- **No PR was opened on that evidence.** The standing condition is a
  head-exact GREEN gate, and 15/16-twice-with-a-rotating-red is not it,
  however well the rotation is explained.
- **A prior gate attempt is NOT evidence**: I wrapped it in
  `timeout 580` to fit the session's command cap, which killed the
  sweep mid-run (`Terminated` in its log) and reported it as a stage
  failure. Its two perf-budget failures also ran at load 36.9 with a
  foreign workload present. Recorded so the log is not mistaken for a
  result. The supported way past the cap is the tool-level background
  launch, which measures `safe` under
  `scripts/check-sigint-deliverable`; `docs/agent-handoff.md` §3 now
  says so.

**Written with the branch's FIRST commit**, per the standing correction
from #171 and #215.

- **Branch `panel-mapping-generation`**, base `githubsucks/main` @
  **`72da24a`**, worktree
  `/home/jeans/Repos/personal/pmacs-mapping-gen`.
  **`githubsucks/panel-mapping-generation` is the authoritative tip.**
  Recover with `git fetch githubsucks && git checkout
  panel-mapping-generation`.
- **No PR yet. Code-complete at `5174f73`; the slice-completion gate is
  RED.** The ledger tip is **the branch head itself**
  (`githubsucks/panel-mapping-generation`) — a literal SHA here goes
  stale the moment the next commit lands, which is exactly how this
  line came to name `fb40d88` while the branch was four commits past
  it. Framing revision 16 (§5b)
  APPROVED 2026-08-15 at `7e85a6f`, then eleven implementation commits:
  wire shapes and pins (G0), the authoritative key with its terminal
  half (G1–G4), outbound family selection, inbound family gating, the
  GPU's negotiated family enum, and the nine family-gate rows
  (G6a–b, G7a–b, G8). **Full `--protocol` gate green at `3c06176`.**
  Then three more: G5a's trigger and its latch, the wheel exemption
  with exhaustion and mapped coalescing, and the receiver rows.
  **Every row this slice owns is now witnessed** — G0, G1–G4a, G5a plus
  latch substrate, G6–G8, G9a–c, G10/G10a–c, G11a, G13a–b, G14a–b, G15
  — with G4b, G5b–e/g/i–p, G6c, G7c, G11b and G12a–b left to the replay
  lane per §5b's split table.
- **PROTOCOL-BEARING — v25, and it runs alone.**
  `ADVERTISED_PROTOCOL_VERSION` stays pinned at **20**.
- **Why it exists.** A `PanelPointer` names a cell and nothing on the
  wire says which inverse mapping the frontend saw, so the daemon
  inverts against whatever is current. `buffer_id` catches
  replacement, `panel_epoch` close/reopen, `geometry_epoch` a
  declaration race — **nothing catches "the text under that cell
  changed"**. Q#BP-R3 first accepted this as narrow; that was
  **overruled**, because a **foreign** edit moves the mapping with
  `view_top` untouched, the error is unbounded, and the window lasts
  until the frontend **presents** the new frame.
- **Semantic seam only.** The TUI hit-tests current daemon state
  directly, receives no `PanelFrame`, and carries no mapping token. Its
  existing panel click/drag/wheel effects are a structural control
  against moving the generation gate into shared replay.
- **A generation, not a per-frame token.** A token moving with the
  frame would cancel a drag on the next repaint — the mistake
  `panel_epoch` is stable to avoid. The key moves with the inverse
  mapping (`view_top`, **`view_left`**, grid size, folds, wrap/gutter
  geometry, buffer content, terminal output/scrollback) and holds
  across focus, styling, selection-only repaints and cursor motion
  **that the follow rules absorb**.
- **One authoritative per-frontend key**, used by projection AND
  inbound validation, advanced after any mapping mutation and
  **before** the next inbound pointer — comparing against the last
  emitted frame recreates the hole.
- **Gating is REFUSAL, not fallback.** A ≥ v25 session sending bare
  `PanelPointer` is refused; a ≥ v25 frontend rejects legacy
  `Present`. Only ≤ v24 sessions keep legacy semantics; `Absent` is
  common. The reciprocal wrong-family cases are witnessed too: a v24
  daemon refuses `PanelPointerMapped`, and a v24 frontend rejects
  `PresentMapped`. Compiling the variant in the same crate never
  overrides the negotiated family.
- **Stale tails TERMINATE, they do not vanish.** A dropped `Up` leaves
  an empty selection armed and a reporting child holding a button
  forever, so cancellation is a ruled outcome: producer latch reset,
  daemon selection cleanup, and the child's release delivered.
  Click-chain invalidation is a separate mapping-identity transition;
  it also runs when no gesture is held.
- **Pins ACCUMULATE**: keep `PanelPointer`, add exact `TextInput`
  bytes (previous-final `FrontendEvent`) and the complete nested
  `PanelFrame(Absent)` bytes (previous-final `PanelFramePayload`), then
  add exact encode/decode pins for both new mapped variants with
  distinct same-typed field values. Previous-final pins protect append
  position; new-variant pins protect their own frozen field order.
  **The `PanelPointer` pin DOES exist** at `src/protocol.rs:1975` —
  revision 14 recorded it as missing, which was wrong: it lives in the
  root crate's test module, not under `pmacs-protocol/` or `tests/`.
- **Appended means LAST**: `PresentMapped` after `Absent`,
  `PanelPointerMapped` after `TextInput`. `mapping_generation` is a
  `u64`, **zero invalid**, gated by `PANEL_MAPPING_MIN_VERSION = 25`.
- **Frontend generation is a session high-water mark.** Higher values
  may skip, equal-generation style/selection repaints are valid, lower
  values are rejected atomically, and `Absent` does not erase the
  high-water mark. Detach does.
- **Coordinate-free wheels are EXEMPT** from the freshness check —
  otherwise the first tick advances the key and the next queued tick is
  refused, so the panel scrolls once and dies. Child-reported terminal
  wheels keep the check, because SGR carries row and column.
- **Cancellation is PROACTIVE**, at the transition that revokes
  authority: mapped-generation advance before its frame, before
  daemon-authored `Absent` or panel/buffer replacement, when a new
  geometry declaration is accepted, and at detach. The last four apply
  to both v24 legacy and v25 mapped gestures. It is taken exactly once;
  no producer cancellation tail is sent.
- **Producer reset has cause-specific signals**, not one generic
  replacement frame: accept the valid generation/identity frame;
  accept `Absent`; locally advance the geometry declaration; or tear
  down the frontend. Invalid frames and same-identity repaints retain
  the latch.
- **The accepted-gesture latch is left-button-only and common to both
  wire families.** It arms only after accepted `Down(Left)` and dies on
  accepted `Up(Left)` or any authority loss. Move, wheel, context, and
  right/middle presses do not arm it; otherwise the GPU's press-only
  right path manufactures a later release. A terminal Down also fixes
  child-reporting versus local-selection domain through Drag/Up, so a
  later Shift or reporting-mode change cannot orphan or invent a child
  release.
- **Click-chain state is independent and per frontend.** It survives
  an ordinary `Up` by design, but every mapping identity change clears
  or re-keys it even when no gesture is held. A completed click followed
  by a foreign edit and same-cell click is single, not double; an
  intervening click from another frontend does not erase the first
  frontend's legitimate chain.
- **Terminal identity EXCLUDES style.** Wire `Cell` equality covers
  `glyph`, `style` and `attachment`
  (`pmacs-protocol/src/cell.rs:153`), so keying on it would move the
  revision on a recolour and contradict the stable control. It is
  glyph/row topology plus the view anchor.
- **The terminal key is NOT `Screen`'s generation** — that advances
  from 39 sites including style, bell, tab-stops and cursor-only
  motion, none of which change what a coordinate denotes.
- **G5 is now an explicit transition matrix.** It crosses every common
  authority loss with v24 and v25, distinguishes each producer reset
  signal from invalid/same-identity controls, pins ordinary-Up
  exact-once and left-only arming, and tests click identity without a
  live gesture. This slice owns only the new mapping-generation signal
  and mapped-frame state; the rebased replay lane owns the common
  invalidations and every document, terminal, complete producer-latch
  and click-chain effect through real call sites. No dead
  classification helper is added on a base without replay.
- **G5's latch lands here under SUBSTRATE names, not the deferred
  IDs.** G5a needs something to cancel, so the per-frontend
  `AcceptedPanelGesture` latch and its arming from the accepted inbound
  arms are this slice's. Writing that code decides which events arm,
  that an ordinary `Up` consumes without counting as a cancellation,
  and that the latch is per frontend — pinned as `g5_substrate_*`
  rather than as G5c/G5d/G5g/G5p, which stay on the replay lane per
  §5b's split table. **Claiming an ID in two branches is the merge
  hazard**, and this file already records duplicate ids surviving a
  clean merge once. Cancellations are **counted, not queued**: the
  record queue is what replay drains, and landing it here would grow
  one entry per cancelled drag with no reader. The other G5b
  transitions (panel epoch, buffer replacement, same-size geometry,
  detach) deliberately leave the latch armed on this base — inert
  while nothing consumes it, and a defect only once replay supplies
  effects, in the branch that owns the row.
- **Routing and effects are separate controls.** This slice can prove
  v24/v25 events reach the existing focus path, but selection,
  terminal reporting, stable-generation drag continuation, two-tick
  wheels and exhaustion cancellation remain explicit replay-lane rows;
  no classification-only result is reported as end-to-end input.
- **Review-process correction — carry into the next handoff
  absorption.** Revision 16 replaces finding-at-a-time review with one
  closure pass for this chain and future framings: verify cited ground
  truth; enumerate trigger × protocol family × target × lifecycle;
  separate producer, route, receiver and effect; prove every witness's
  discriminating setup and positive control; check that each proposed
  mutation is actually defective; and assign every row to the branch
  that can reach its real call site. Protocol work also checks all four
  wrong-family quadrants, exact pins and version fallout. Findings are
  returned as one batch. If three revisions do not close a framing,
  stop serial review and perform/fold this complete audit rather than
  opening another one-finding round. The overdue #239/#240 handoff
  absorption must promote this rule; this feature branch does not edit
  the deliberately lagging canonical handoff.
- **Owns the version correction.** `docs/gui-stage1-input-framing.md`
  moves 1e's `OpenTarget` to **v26** here — an expected rebase
  conflict on `gui-stage1b-pointer-scroll` is not grounds for leaving
  the canonical document false.
- **Chain: this slice → `panel-pointer-replay` (rebases onto it) →
  GUI arc 1b.**
- **CI-red observations on this branch, NOT yet in the registry — and
  deliberately so.** `docs/ci-red-signatures.md` here ends at **U9**;
  the `panel-pointer-replay` branch already added a **U10** that has
  not merged. Adding a row from this branch would either duplicate that
  id or invent U11 against an unseen neighbour — and this file's own
  history records exactly that going wrong once already, when two
  branches' entries "merged **without a conflict**, producing duplicate
  ids across four sites". **The rows below are owed to the registry by
  whichever branch merges second**, numbered after the other's.
  - **R7, two occurrences on this branch** (2026-08-15, `gpu` step,
    logs `20260815T095532Z` and `20260815T100719Z`). Fragments verified
    both times: `transient sequence must attach: Attach(Handshake(Io(Os
    { code: 32, kind: BrokenPipe, message: "Broken pipe" })))` at
    `pmacs-gpu/src/attach.rs:1728`. One machine, one day, one branch,
    with a green full-gate run between them. Isolated reruns green.
  - **`composition_overhead_under_ten_percent`, once** (log
    `20260815T102527Z`, step **`04-lib-crdt`**). Fragment:
    `composition machinery added more than 10% overhead: 1.146
    (single=227350 ns, dispatch=260497 ns)`. **`criterion_1` ran in the
    same step and PASSED**, so per U6's own instruction this is one
    selector redding without the other and is a different incident.
    **1.146× is the smallest margin this budget has ever failed by**
    (U6 1.297×, U10 1.343×, U9 1.613×), and the "realistic" figure was
    **negative** (−25.8%) in the same run. Isolated rerun green.
  - **`setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`,
    once** (log `20260815T184808Z`, step `08-sweep`). Fragment: `live
    runtime probe` at `src/process.rs:5155` — `active_reader_probe`
    returned `None` for a process that had just reported `Started`. A
    live-process race under sweep load. Isolated rerun green. **Not in
    the registry under any id**, so it is a new signature, owed like
    the two above.
  - **`composition_overhead_under_ten_percent`, a SECOND time**
    (2026-08-15, plain `cargo test --lib`, not a gate step). Isolated
    rerun green. Same budget as the occurrence above, now seen in two
    different selectors on one branch in one day.
  - **RESOLVED ATTRIBUTION: the sweep-crdt red is PRE-EXISTING ON
    MAIN.** The identical `build-crdt && sweep-crdt` pair run at the
    merge base **`72da24a`**, in the primary worktree with its own
    target dir, fails the SAME test,
    `ctrl_c_on_launcher_group_does_not_reach_spawned_daemon`: **119
    green result summaries and TWO red binaries** —
    `gpu_initial_target_acceptance` includes the suite as a module, so a
    reproducing sweep reds twice (`…-2144707/09-sweep-crdt.log:3097`
    and `:3131`). **This branch is not implicated**, and no branch can
    pass this gate stage on this machine until the underlying defect is
    fixed.
  - **THE ONSET IS DATABLE.** `sweep-crdt` has 17 logs here. Counted
    per test copy: **13** both copies `... ok`, **1** where neither
    executed (the stage died compiling `pmacs`, `error[E0308]`), **3**
    both `FAILED`. Last observed green `20260815T185708Z`, first
    observed red `20260816T063330Z`; boot began 08-14 09:30, so no
    reboot between. So "pre-existing on `main`" holds,
    but **"always broken" is contradicted**. The window is **not yet a
    bisect target**: cleanliness was captured at neither endpoint, so
    the lane's first move is to reproduce `7599661` and `724b785`
    clean, in isolated target dirs, and decide from that.
  - **The red full-sweep count is SEVEN**, not five; each run is
    enumerated with its own log digest in the teardown lane's
    `docs/probe-sigint-evidence.md`.
  - **The defect now has its own lane:
    `gpu-probe-sigint-teardown`** (pushed; framing revision 9 at
    `docs/gpu-probe-sigint-framing.md` (revision 9), run provenance at
    `docs/probe-sigint-evidence.md`). **§5b is held behind it.** That
    lane's framing supersedes every diagnostic claim below; the entries
    here are kept only as the record of how it was found.
  - **MECHANISM STILL NOT ESTABLISHED — and the RETRACTION below was
    itself wrong.** Two corrections, in order:
    - A "mechanism located" claim (launcher in `do_wait`, probe child
      in `futex_do_wait`) was retracted on the argument that the
      failing launcher "must live 8s or more" while the sampler's
      longest-lived was 5s.
    - **That arithmetic is false.** Both reproducing binaries finish in
      **~5.19s including the 5s timeout** (`:3097`, `:3131`), so
      `phase=ready` lands in about a tenth of a second and the failing
      launcher lives roughly **5.1s** — inside what the sampler
      observed. A ">6s selector", proposed as the remedy, would have
      captured **nothing**.
    - So the "located" claim is **not refuted by that argument**. It
      stays **unproven for a different reason**: under `--features
      crdt` the suite spawns root launchers from **six** call sites, so
      a launcher captured by command line alone cannot be attributed to
      this test.
    - **Do not key on process age. Key on the PID the test records**,
      with snapshots before and after its own `kill`.
  - **Five further explanations were tested and REFUTED.** Recorded so
    nobody re-runs them:
    - *Machine load*: refuted. Red on a quiet machine (load 2.77 at
      launch, foreign workload gone).
    - *Memory pressure*: refuted, **by correcting my own instrument**.
      A sampler showed free memory falling to 543 MB and that looked
      damning — but it recorded `free`, which on Linux is not the
      meaningful figure. `available` was **27 G**. The 543 MB was
      reclaimable cache.
    - *Leaked daemons*: refuted. Peak 58 during the sweep, up only 8
      from the resting 50, and the green standalone runs already ran
      at 46-50.
    - *tmpfs starving the box*: refuted **by experiment**, not
      argument. `/tmp` went from 21G used to 1.2G (available memory
      27G -> 45G) and the sweep stayed red, same test. Note the
      earlier `available`-based dismissal was itself unsound — tmpfs
      pages are NOT reclaimable yet still appear in `buff/cache` — so
      the hypothesis deserved the experiment it eventually got.
    - *inotify exhaustion*: refuted. 47 instances in use of 1024.
    - *`--workspace` artifact selection*: **NOT refuted.** Two targets
      under `--workspace` are green, and the preceding targets without
      `--workspace` are green, but neither run isolates the factor —
      see below.
    - *A specific preceding test*: **NOT refuted, and the earlier entry
      here was wrong.** It claimed all 37 preceding targets plus the
      suite run green "same binaries, same order, same tests". The
      **compilations** were not the same: that run executed
      `gpu_initial_target_acceptance-91f51d0b` and
      `gpu_invocation_acceptance-6b4b8223`, while the failing sweeps
      executed `-5d9105cb` and `-d4dae4f0`. Differing Cargo suffixes
      mean differing metadata hashes, so command shape changed the
      compilation and the comparison was never made. **Historical byte
      identity is UNKNOWN** and is not claimed — target dirs have been
      overwritten since.
    - Also withdrawn: that other packages "cannot be implicated"
      because their targets run after the failure. Later-selected
      packages can affect Cargo's build graph and fingerprints
      **before** their tests execute.
    - Also withdrawn: that the cause is "cumulative across the
      preceding 37 binaries". Nothing established that.
    - Note the test exists in **two** binaries: `tests/gpu_initial_
      target_acceptance.rs` includes it as a module, so a reproducing
      sweep fails it twice, at log lines 3083 and 3117.
  - **What the reductions established, and what they did NOT** — they
    were never a bisect, since no run isolated a variable. Green in
    every smaller context tried — the test alone (x3, 0.15s against its
    5s deadline); its whole 15-test suite; a workspace-wide run
    FILTERED to just this test; the lib binary (2145 tests) then the
    suite; the three GPU suites in sweep order — and red in every full
    workspace sweep, seven of them across two trees.
    **No cumulative cause follows from that.** The subset runs and the
    sweeps executed **different Cargo compilations** — differing
    suffixes, hence differing metadata hashes — so the reductions never
    made the comparison they appeared to make. (Calling them
    "byte-different" is withdrawn: the bytes a historical run executed
    are not knowable now, only the suffixes.) What is established is
    only: reproducible in the full sweep, not reproduced in any subset
    attempted so far.
  - **`/tmp` is a 30G tmpfs holding 21G**, almost all stale
    `levshell-*-target` cargo directories belonging to an unrelated
    project, 3-4 days old. Recorded as an observation about this
    machine, **not** as the cause — `available` memory refutes that.
    Not touched: they are not this project's to delete.
  - **The earlier signature entry below overstated its case** and is
    kept only for its measurements. It read the isolation runs as
    proof of an environmental cause; they establish intermittence at
    most, and the base-comparison above is what actually settled
    attribution.
  - **`ctrl_c_on_launcher_group_does_not_reach_spawned_daemon`, a NEW
    signature, red TWICE and reproducibly** (2026-08-16, logs
    `20260816T063330Z` and `20260816T064549Z`, step `09-sweep-crdt`
    both times). Fragment: `child did not exit within 5s` at
    `tests/gpu_invocation_acceptance.rs:180`. **Not in
    `docs/ci-red-signatures.md` under any id.** Green in isolation and
    green with all fourteen of its suite siblings, the latter in
    **0.15s against its own 5s deadline** — a 33x margin. It reds only
    inside the full-workspace crdt sweep.
    - **The row is marginal by construction, and the sweep is the
      worst place for it.** It holds a FIXED 5s wall-clock deadline for
      a spawned child to exit after a signal, and it runs inside the
      heaviest stage the gate has — `cargo test --workspace --features
      crdt`, which saturates all 16 cores by itself. Nothing about it
      touches panels or the wire.
    - **Sweep contention could not be separated from foreign load**,
      and the attempt is recorded as inconclusive rather than dressed
      up: running `09-sweep-crdt`'s exact command alone red the same
      row, but `uptime` hit **59.51** during that run, so the control
      proved nothing it was meant to. Part of that number is the
      sweep's own parallelism.
  - **Three wall-clock-deadline rows red in ONE run, under a MEASURED
    foreign load** (2026-08-16, log `20260816T063330Z`). Steps
    `04-lib-crdt` and `09-sweep-crdt`. Fragments:
    `criterion_1_end_of_line_typing_completes_sub_frame_per_keystroke`
    — "per-keystroke orchestrator time **1.001196ms** exceeds 1ms",
    **0.12% over**, the smallest margin any budget on this lane has
    failed by; `m6_1_pty_mode_lifecycle_started_then_exited` — "must
    observe stdout 'done'"; and
    `ctrl_c_on_launcher_group_does_not_reach_spawned_daemon` — "child
    did not exit within 5s". All three green in isolation, the last in
    **0.15s against its 5s deadline**, a 33x margin.
  - **This is the first entry on this lane with a NAMED confound rather
    than the standing uncontrolled one.** `uptime` during the run:
    load average **14.02 → 28.35**, from an unrelated `turso` test
    suite on the same machine (`./verify_task_state.sh
    turso-without-rowid`, target dir `/opt/target`, one test binary at
    **693% CPU**). It is not a controlled experiment, but it is the
    same evidence U9's synthetic-load control was meant to produce, and
    it points at load. U9 stays owed; its value is now lower.
  - **Two process traps this cost, both worth carrying forward.** The
    Bash tool caps a command at 10 minutes and SIGTERMs it, which the
    gate reports as `FAILED (exit 143)` on whatever stage was running —
    indistinguishable from a real failure in the summary line. Run the
    protocol gate under `setsid` and watch the log. And `pkill -f
    <pattern>` kills the invoking shell when the pattern appears in its
    own command line, so the intended target survives and the operator
    believes it died. **Both mistakes were made here**; the second
    nearly killed an unrelated project's build, because
    `pkill -f "cargo test"` would have matched it. Identify by PID.
  - **Three consecutive full-gate runs, three DIFFERENT unrelated
    failures** (composition budget in `04-lib-crdt`, then composition
    again in `10-sweep-crdt`, then this in `08-sweep`), against a diff
    that touches panels and the wire. Recorded as a rate observation
    only: no mechanism is claimed, and the standing leaked-daemon
    confound is uncontrolled as always.
  - **Cost, stated plainly:** four `--protocol` gate runs on one commit,
    three of them lost to these two signatures. U9's synthetic-load
    control remains unrun and is the cheapest thing that would either
    implicate load or clear it.
- **Rustdoc split, FOUR occurrences on this branch** (`screen_size`,
  `peer_may_send_panel_events`, `send_panel_pointer`, and
  `SemanticRenderState`). Always the same mechanism: inserting an item
  at what reads as a blank gap when the lines directly above are the
  NEXT item's doc comment, which the insertion then adopts. The fourth
  also stole an `#[allow(clippy::struct_excessive_bools)]`, silently
  un-suppressing a lint on the struct that needed it. Three were caught
  by the user in review, one by a `missing_docs` warning. **The check
  is to look UP from the insertion point before writing, not down.**
- **SLICE-COMPLETION GATE STATUS — read before assuming this is
  mergeable.** Nine of ten stages green at `5174f73`; `09-sweep-crdt`
  red on the row above, twice. **A gate with a red stage is a red gate,
  and no PR was opened on it.**
  - **The cause is UNRESOLVED.** The row is **intermittent**: green
    alone, green with all fourteen of its suite siblings, red inside
    the full-workspace crdt sweep. Foreign load is a **measured
    confound**, not an explanation — establishing intermittence is not
    establishing an environmental cause, and no experiment here
    separated sweep contention, foreign load, and a genuine defect in
    the row. Do not record it as environmental until something does.
  - **The gate cannot pass on this machine for ANY branch**, main
    included, until the pre-existing sweep-crdt defect is fixed. That
    is a decision point, not a thing to keep re-running: either the
    defect gets its own lane, or this lane's readiness bar is restated
    against a gate that main itself can pass.
  - **Clean evidence is not obtainable on this machine right
  now** — an unrelated `turso` workload
  (`./verify_task_state.sh turso-without-rowid`, target `/opt/target`)
  is running in a LOOP, holding a 16-core box at load 20-60. The
  remaining work on this lane is one clean gate run, on a quiet
  machine, and nothing else.
- **Gates — THE EXACT INVOCATION, not a list of suites.** Naming the
  suites without the flag form is what let two full runs go by with
  bare `--protocol` and **no acceptance stage at all**; the suites were
  verified by hand instead, which is not the gate and is exactly the
  substitution these records exist to prevent. `--acceptance` is an
  EXPLICIT repeated flag — the gate derives nothing from the diff.

  ```
  ./scripts/gate --protocol \
    --acceptance bottom_panel_stage1_acceptance \
    --acceptance bottom_panel_stage2a_acceptance \
    --acceptance bottom_panel_stage2b_daemon_acceptance \
    --acceptance bottom_panel_stage2b_gpu_acceptance \
    --acceptance bottom_panel_stage2b_protocol_acceptance \
    --acceptance gui_stage1a_wire_acceptance
  ```

  Sixteen stages: ten from `--protocol` plus one per suite. **A run
  that prints ten stages is missing every acceptance stage**, and a run
  that prints eleven is the older single-suite form.
- **The "four `bottom_panel_*` suites" phrasing was WRONG — there are
  five.** `bottom_panel_stage2b_protocol_acceptance` is the fifth and
  belongs in a protocol-bearing lane above all others. The block above
  runs all five rather than guessing which four an earlier writer
  meant; a superset is the safe reading of an ambiguous record.

## GPU launcher / probe SIGINT teardown — MERGED as #241 (`f8033bc`)

- **MERGED 2026-08-20** at approved head `5089715`, merge commit
  `f8033bc`. **14/14 CI checks green on that exact head**, including
  `Test (crdt)` and **both** macOS jobs.
- **A6a CLOSED BY MEASUREMENT**: `status 1 + no token → boundary error,
  never ignored`, green on macOS — the platform whose shell exits 1 for
  an exec failure, which is what produced the original defect.
- **A7 is no longer "satisfied by disclosure".** Both macOS flavours
  (`lua54`, `luajit`) exercised the helper and gate consumers across the
  full 45-case shared set. **The R-d consumer stays Linux-only** — its
  test is crdt-gated and the macOS jobs build without `crdt`, while
  `Test (crdt)` is `runs-on: ubuntu-latest`. Recorded as an open gap,
  not closed.
- **ONE UNEXPLAINED FAILURE, 2026-08-20, AND NO MECHANISM IS RECORDED
  FOR IT.** The row was
  `rd_precondition_validates_the_whole_conformance_set`.
  The row failed once in a `sweep-crdt` run in the `pmacs-mapping-gen`
  worktree and passed on the two sweeps after it. **The failure message
  was not captured**, so there is nothing to reason from — which is
  exactly why no cause is asserted here.
  - **NOT REPRODUCED**: 30 consecutive runs at load ~10.5, 1,350
    successful stub executions, no recurrence (user-run, 2026-08-20).
  - **A "concurrent spawn pressure" explanation was offered and is
    WITHDRAWN as false on its facts.** The test runs its **45 stubs
    SEQUENTIALLY**, plus one intentional nonexistent-path spawn probe.
    There is no concurrency to be pressured, and 46 was a miscount of
    45-plus-a-probe.
  - **ON RECURRENCE, CAPTURE THE EXACT CASE AND ERROR** before
    theorising. A single uncaptured failure supports no diagnosis, and
    the sequential design means the failing case is identifiable.
- **What shipped:** `scripts/check-sigint-deliverable` (validated
  `(status, token)` pair, 0 safe / 1 ignored / 2 error), a gate guard
  that refuses before any stage with no override, a target test that
  reports the precondition instead of a phantom deadline, and the rule
  in `docs/agent-handoff.md` §3 forbidding `setsid nohup … &` for the
  gate and `cargo test`. **No product behaviour changed** — this was
  gate/test correctness only.
- **Two `m4_24_*` rows fail locally under `crdt` and DO NOT reproduce in
  CI**, on this branch and at `72da24a` alike: local-environment
  -specific, not a code defect and not this lane's.
- **`panel-mapping-generation` (§5b) is UNBLOCKED** by this merge. Its
  sixteen-stage gate must run **in the foreground** — the condition its
  stage 15 always needed, and the reason this lane existed.

**Written with the branch's FIRST commit**, per the standing correction
from #171 and #215.

**Written with the branch's FIRST commit**, per the standing correction
from #171 and #215.

- **Branch `gpu-probe-sigint-teardown`**, base `githubsucks/main` @
  **`72da24a`**, worktree
  `/home/jeans/Repos/personal/pmacs-probe-sigint`. Recover with
  `git fetch githubsucks && git checkout gpu-probe-sigint-teardown`.
- **REVISION 13 IMPLEMENTED at `bc7d776`.** Helper emits the token on
  stdout; both consumers validate the `(status, token)` pair. The gate
  owns a guard-local capture dir, keeps `|| status=$?` under `set -eu`,
  selects `expected_token` before any `set -u`-sensitive use, compares
  bytes with `cmp` against both permitted encodings, surfaces the
  helper's stderr **only** for validated verdicts, and prints
  `status=`/`token=` on every refusing branch. R-d validates the same
  pair from `Command::output()` bytes — no capture files, since only
  the shell needs them.
- **Conformance vectors live in `tests/common/sigint_conformance.rs`**
  and are consumed by BOTH validators, so the two copies cannot drift
  while each still reports "45 cases". The **branch-discriminating** cases emit a
  sentinel on stderr, which is what separates `ValidatedError` from
  `Boundary` —
  they share exit 2, so comparing codes alone let a validator that
  accepted every status 2 pass the whole matrix. An `Outcome` enum
  (Safe / ValidatedIgnored / ValidatedError / Boundary) is asserted
  branch-exact in both suites, and each helper arm's exact stdout token
  is asserted too.
- **A8 is complete**: a bounded row points `TMPDIR` at a missing
  directory so `mktemp -d` fails, and asserts boundary error 2, no
  stage, and no residue. Temporary directories are RAII throughout;
  the `keep()`-plus-manual-cleanup shape is gone.
- **36 gate rows, 16 GPU rows.**
- **HEAD-EXACT GATE on `8802d6a`: green, all 8 stages**, log
  `20260820T072102Z-3009434`. The first attempt on the same head
  (`…-2931214`) failed `07-sweep` on
  `composition_overhead_under_ten_percent` — a perf budget unrelated to
  this lane, green in isolation, and a signature the
  `panel-mapping-generation` ledger already records recurring. **Both
  runs are recorded; no causal attribution is made** for the first,
  only that the second is the head-exact evidence.
- **The earlier `…-2647615` run is NOT head-exact evidence**: it
  finished about 30 seconds before `bc7d776` was committed, so it
  describes the implementation tree rather than a committed head.
- **A4 mutations, each biting the MATRIX now**, not a dedicated row:
  accepting any status 2 regardless of token, and surfacing child
  stderr on a boundary failure, both fail
  `gate_validates_the_whole_shared_conformance_set`; token-to-stderr
  fails `sigint_helper_reports_safe_…`. An earlier entry said the
  status-2 mutation was caught only by the dedicated row — that was
  true of the pre-sentinel matrix and is **superseded**: the
  branch-discriminating cases now carry a sentinel, so the matrix can
  see which branch produced the exit 2.
- **X3 and X4 deliberately carry their OWN stderr payloads**, not the
  sentinel — X3 the canonical ignored wording with no token, X4 noise —
  which is what makes them distinct inputs. `shared_cases()` asserts
  uniqueness over `(status, stdout, stderr)`, so the earlier
  45-entries-over-43-inputs collapse cannot recur silently; reverting
  either payload now fails by name.
- **Two PRE-EXISTING crdt-only failures found while gating properly:**
  `m4_24_bare_string_glob_stays_relative` and
  `m4_24_d3_fallback_base_is_the_smallest_attachment_dir`. They
  reproduce in isolation (so not load) and **fail identically at
  `72da24a`**, so they are not this lane's. They are crdt-only — the
  plain gate's `05-m4` stage runs without `crdt` and passes. Recorded,
  not attributed; whether they are environment-specific is for CI to
  say.
- **My local gate did NOT cover what CI covers, and CI caught it.**
  Plain `./scripts/gate` omits `sweep-crdt`, which is the only stage
  that compiles the nested `gpu_initial_target_acceptance` under
  `crdt`; `04-lib-crdt` builds the lib alone. A `crate::common` path
  that cannot resolve when nested, and a clippy lint, both shipped
  green locally. **This lane gates with `--protocol`.**
- **CI ON `916007b`: 12 GREEN, 2 RED — both macOS `Test` jobs**, and it
  is the **pre-declared A7 portability finding**, not an environment
  excuse. Exactly one row:
  `gate_maps_an_unexecutable_helper_to_error_not_ignored`,
  `left: Some(1)  right: Some(2)`. The other five SIGINT rows pass on
  macOS, including both `error` cases, so helper and gate consumers are
  otherwise exercised there.
  - **The gate returned 1 = `ignored` for a helper it could not
    execute** — the exact conflation §7c forbids.
  - **ESTABLISHED, not a hypothesis.** The macOS log shows the shell's
    `Permission denied` followed by the gate's `1 | 2)` message — so
    `sigint_status` was **1**: macOS `/bin/sh` returns **1** for an
    exec failure where Linux returns **126**. The status-only ABI
    cannot separate that from the helper's own `ignored`.
  - **The raw status was NOT printed.** An earlier entry here said the
    stderr carries it; the gate interpolates the number only in its
    catch-all branch, and this failure took the `1 | 2)` branch. The
    path was identified by **which message text appeared**, not by a
    number.
  - **A proposed repair was REJECTED in review and is recorded so it is
    not retried:** moving `ignored` from 1 to 3 relocates the collision
    without closing it, because an exec failure can return **any**
    nonzero status. The generalisation: **no exit status can prove the
    helper ran.**
  - **Framing revision 13 — APPROVED 2026-08-19 at `5dece3e` —
    replaces the ABI with a validated `(status, token)` pair**:
    `0`/`1`/`2` with
    `pmacs-sigint-v1:safe|ignored|error`, token alone on stdout,
    diagnostics on stderr, and any other pair — including macOS's
    status 1 with no token — a boundary error mapped to 2. Every
    refusing branch must print the observed status and token state as
    diagnostic context, never as the classifier. A shared **conformance
    matrix** replaces revision 12's withdrawn "both consumers use the
    same helper so they can never disagree", which stopped being true
    once each consumer validates the pair independently.
  - **A6a is scoped to the GATE.** R-d never sees a shell status: Rust's
    `Command` returns a spawn error with no exit status, and R-d's test
    is crdt-gated so macOS CI does not compile it. R-d on macOS is
    **unexercised**, recorded as a gap.
  - **A7 is no longer "satisfied by disclosure"** — macOS was reached
    and measured: five of six helper/gate rows pass, one defect
    (`gate_maps_an_unexecutable_helper_to_error_not_ignored`), R-d
    Linux-only.
- **PR #241** (`https://github.com/levineuwirth/pmacs/pull/241`), opened
  2026-08-19 from `gpu-probe-sigint-teardown` into `main`. **Not merged;
  awaiting review rounds.**
- **CHECKPOINT — immutable event SHAs, because "this entry's own
  commit" goes stale exactly the way a tip SHA does:** implementation
  landed at **`3206433`**; the A6 both-consumer rows and the bounded
  negative path at **`167d830`**; the two factual corrections at
  **`c9cc8dd`**; the gate-run record at **`7cef9ca`**; the PR record at
  **`d64d300`**. Only the **branch tip** stays symbolic — a literal SHA
  naming a branch's own tip goes stale the
  moment the next commit lands, which this ledger has already recorded
  once. State at `3206433`: pushed, signed `G`, worktree clean,
  `git diff --check` clean, **full default gate green (8/8) in the
  foreground**, 31 gate-acceptance rows passing. After this commit:
  **33 gate-acceptance rows and 16 `gpu_invocation_acceptance` rows**.
- **FULL GATE GREEN ON THE COMMITTED HEAD `c9cc8dd`** — all 8 stages,
  log `20260819T160220Z-2339958`, started at load 3.90. Recorded at
  `7cef9ca`, which is docs-only on top of the gated tree; that
  exemption is what stops gate-result records recursing forever.
- **The preceding attempt on the same head was RED, and is kept.**
  Log `20260819T152209Z-2073040`: `04-lib-crdt` and `07-sweep` failed
  on four wall-clock rows —
  `composition_overhead_under_ten_percent`,
  `full_buffer_summary_flatten_scales_on_large_grammar_file`,
  `dired_renders_10k_entries_within_200ms`,
  `file_progress_notification_is_recorded_for_its_document` — none
  touching this lane's change. Load average was **49.6**, and
  `./verify_task_state.sh review/my-ruff-task golden` was running under
  a separate toolchain at `/usr/local/rustup` with four `rustc`
  processes, having started about three minutes into the run and
  overlapping exactly the two stages that failed.
  **That is evidence of WHEN, not proof of WHY**, and it was recorded
  as "not valid evidence" rather than "environmental" — this lane has
  already retracted one confident environmental attribution. The green
  run on the same commit is what settles it; had any of the four failed
  again on a quiet machine, it would have been a real finding here. (An earlier
  draft said 35: that figure was the `git_status_stage1_acceptance`
  result line immediately below `gate_script_acceptance`'s in the sweep
  log, misread as this suite's.)
- **Framing revision 12 at `docs/gpu-probe-sigint-framing.md`,
  approved 2026-08-19 at `1fc0df6` and IMPLEMENTED; **superseded by
  revision 13, approved 2026-08-19 at `5dece3e`** — revision 10 was
  approved at `4fba9f6` and revision 9 at `15c25ec`; neither approval covered the
  later mechanism finding and remedy selection. **D1/D2 HAVE RUN and
  found the mechanism: `SIGINT` was ignored group-wide
  (`SigIgn=0x1007`) because
  the test runner was launched in the background — `SIG_IGN` is
  inherited across `fork` and survives `exec`, so it reached the
  launcher and probe, and `kill(-pgid, SIGINT)` was a no-op.**
  Controlled arms re-run on committed head `77b623c` with full SHA-256
  captured per arm and byte-identical
  binaries: foreground both copies ok, `setsid nohup … &` both FAILED.
  **I caused this** by adopting background launches on 08-16 to evade
  the Bash tool's ten-minute cap — that is the "onset", and the
  subset-vs-full matrix was confounded with it throughout. A3's
  subset/full obligation is discharged by explanation, so D0b is not
  needed. **Bet 1 withdrawn by scope and A5 retired by scope — D4 was
  never executed**, so no claim is made that a real session behaves
  correctly, only that no evidence of a user-facing defect survives.
  **A3/D0b are SATISFIED by that explanation** — D0b is not owed and
  will not run. The approved revision 12 **selects the remedy**: R-b +
  R-d via one checked-in helper,
  `scripts/check-sigint-deliverable`. Its preserved-status inner probe
  maps to one complete ABI: helper exit **0** = `safe`, **1** =
  `ignored`, **2** = probe `error`; the helper owns the two failure
  diagnostics, and both consumers surface its stderr rather than
  interpreting raw probe statuses. Inability to execute the helper is
  `error`, never evidence of `SIG_IGN`. POSIX shell only: no `/proc`,
  so not Linux-only; no `sigaction`, so no `unsafe`. `scripts/gate`
  fails immediately with the explicit diagnosis; the target test
  reports the same precondition failure if run directly;
  **no override**, because a gate under ignored `SIGINT` cannot produce
  valid evidence. R-c rejected. The Linux-only D1/D2 instrumentation is
  removed once its evidence is portable. A1–A7 witness guard bite,
  direct-test diagnosis, unaffected foreground success, mutation, an
  otherwise unchanged gate, a distinct error outcome in **both**
  consumers, and qualified non-Linux-unix portability.
- **IMPLEMENTED.** `scripts/check-sigint-deliverable` is the shared
  helper (0 safe / 1 ignored / 2 error); `scripts/gate` refuses before
  any stage; the target test reports the precondition instead of the 5s
  deadline; the Linux-only `/proc` instrument is removed.
- **Two bugs shipped in the first guard, both caught in review.** A bare
  invocation under `set -eu` killed the shell at the helper's non-zero
  exit, so the refusal never printed. Replacing it with
  `if ! helper; then status=$?` captured the status of the **negated
  condition** — always 0 — so the gate printed the diagnosis and then
  ran the whole suite anyway. The working shape is
  `helper || status=$?`, the idiom the helper uses internally. The
  guard also moved to immediately after the worktree resolves, so a
  refused run leaves no log dir, ambient root or tmpdir behind.
- **Four durable rows in `gate_script_acceptance`** (31, was 27):
  helper safe / ignored / error, and gate refusal before stage 1.
  **Verified to bite** — mutating the gate back to either shipped bug
  fails `gate_refuses_to_start_when_sigint_is_ignored` and nothing
  else. Their absence is why 27 passing tests missed both.
- **A7 — revision-12 HISTORY, superseded.** It then read: satisfied by
  disclosure, Linux `x86_64` only, no non-Linux unix reachable. **That
  is no longer true** — macOS CI reached it and measured it red. The
  live record is the CI entry above: five of six helper/gate rows pass
  on macOS, one defect, R-d Linux-only because its test is crdt-gated.
- **D0a EXECUTED 2026-08-19 — verdict: difference NOT captured by the
  two commits.** 10 runs, counterbalanced, N=5 per endpoint, clean
  detached worktrees with isolated target dirs, `dirty=0` per run, zero
  voids, zero splits. **A (`7599661`) uniform-red; B (`724b785`)
  uniform-red.** So `7599661`, which passed inside `sweep-crdt` on
  08-15, fails 5/5 clean today: **the two commits do not discriminate
  under current conditions, so no bisect will run.** That is the whole
  claim — "source hypothesis eliminated" and "unreachable by source"
  are **withdrawn**, since a historical regression could be masked by a
  later environmental effect or a source/environment interaction.
  Failing to discriminate is not the same as not differing. The onset
  window is deprioritised, not excluded. No package activity in the
  window (`pacman.log`) — a cheap negative, not pursued further.
- **The useful product is a RELIABLE REPRODUCTION** — 10/10 across two
  commits at ~4 min/run — which is what let D1/D2 run at once. **D1/D2
  are DONE**; the mechanism entry above supersedes this. Per-run provenance in `docs/probe-sigint-evidence.md`
  §D0a, transcribed in full into that committed document — after the
  first transcription corrupted every log digest by one hex character
  and dropped `/tmp` and `MemAvailable`. **`uptime` was never captured**
  and is `UNKNOWN` for all ten runs: §7 names it, the harness kept only
  the load averages, so that condition list was **not fully
  satisfied**. The classifications stand; D1/D2's harness must capture
  the whole list.
- **Why it exists.** `ctrl_c_on_launcher_group_does_not_reach_spawned_daemon`
  fails in gate stage `sweep-crdt` with "child did not exit within 5s".
  **Pre-existing on `main`** — `72da24a` fails it in a clean worktree
  with its own target dir. The correct count is **119 green result
  summaries and TWO red binaries**: `gpu_initial_target_acceptance`
  includes the suite as a module, so a reproducing sweep reds twice
  (log `…-2144707/09-sweep-crdt.log:3097` and `:3131`). While it reds,
  **no branch can present a green sixteen-stage gate, `main`
  included.**
- **`panel-mapping-generation` (§5b) is HELD BEHIND THIS LANE** by
  explicit instruction. That lane is code-complete at `5174f73` with
  its own fifteen stages green; its sixteenth stage is this defect.
- **Reproduction is 7/7 across full sweeps (F1–F7); every reduction
  R1–R10 is green.** Each run is enumerated with exact command, worktree, HEAD,
  cleanliness and log digest in `docs/probe-sigint-evidence.md` — "0/N"
  is not a record.
- **But R9 did NOT run the same compilations as the sweep.** It
  executed `…-91f51d0b…` / `…-6b4b8223…`; the sweeps executed
  `…-5d9105cb…` / `…-d4dae4f0…`. **Differing Cargo suffixes mean
  differing metadata hashes — different compilations.** Historical byte
  identity is **UNKNOWN** and is never claimed: target dirs have been
  overwritten, so a hash computed today is the current occupant's. R9
  establishes **same target names and order**, not same compilations. What the evidence is **consistent
  with**, not what it isolates: prior targets alone (R9) green,
  workspace selection alone (R10) green, both together (F1–F7) red.
  That is an observation, not a finding. `--workspace` selection
  is **not sufficient by itself and not ruled out** — later-selected
  packages can affect the build graph before their tests ever run, so
  "their targets execute after the failure" does not exonerate them.
- **Ruled out by measurement — do not re-run:** machine load; tmpfs
  starving RAM (settled by experiment, not argument — `/tmp` 21G→1.2G,
  available 27G→45G, still red); leaked daemons; inotify.
- **NOT ruled out, contrary to earlier entries here:** `--workspace`
  artifact selection, and the preceding tests. R9 appeared to clear
  them but ran **different Cargo compilations**, so the comparison was
  never made. Both are open.
- **Ground truth, and what it does NOT establish.** `run_gpu`'s own
  path installs no handler: `run_gpu` (`src/main.rs:324`) blocks
  in `command.status()` with no handler, and grepping all of
  `pmacs-gpu/src` for signal machinery returns nothing. **But the
  `pmacs` binary DOES contain signal machinery** —
  `install_signal_handlers` (`src/daemon.rs:628`) registers `SIGINT`
  and `SIGTERM`; it is simply not on `run_gpu`'s path. A source grep
  also cannot exclude a runtime or dependency installing a disposition.
  So the established fact is only: **no explicit installation on
  `run_gpu`'s path**, and "whatever disposition they hold was
  inherited" stays a **hypothesis** until D2 measures it. The probe's
  **event loop** wakes at least every 50ms
  (`pmacs-gpu/src/main.rs:1065`) — but the process is **not** bounded:
  its stdin reader blocks in `read_to_end` (`:1109`) and, once ready,
  the loop leaves only when stdin closes (`:1212`).
  **No claim is made that either process holds the DEFAULT
  disposition** — absence of handler code cannot establish that, and
  inherited ignore is the leading hypothesis precisely because the
  source is silent.
- **TWO retracted claims, both mine.** (a) "Mechanism located" —
  launcher in `do_wait`, probe child in `futex_do_wait`. (b) The
  retraction of (a), which argued the failing launcher "must live ≥8s".
  **(b)'s arithmetic is false**: both reproducing binaries finish in
  ~5.19s *including* the 5s timeout, so the failing launcher lives
  about **5.1s** — inside what the sampler saw, and a ">6s" selector
  would have captured nothing, repeating the error it was meant to
  correct. (a) is therefore not refuted by (b); it stays **unproven for
  a different reason** — under `--features crdt` the suite spawns root
  launchers from **six** call sites, so command line alone cannot
  attribute one to this test. The six (`:509, :534, :544,
  :574, :725, :1097`, all inside `#[cfg(feature = "crdt")] mod crdt`);
  the other two `--gpu` arguments sit under `#[cfg(not(…))]` and are
  compiled out. **Do not key on process age. Key on the PID the test
  records.**
- **Diagnostics must DISCRIMINATE** blocked delivery, inherited ignore,
  and an escaped process group: snapshots **before and after** the
  signal, for test parent / launcher / probe; **per-thread** `SigBlk`
  from `/proc/<pid>/task/*/status`; `SigPnd`/`ShdPnd`; and
  `PID`/`PPID`/`PGID`/`SID`. A post-failure snapshot cannot prove
  inheritance.
- **Why that matters:** `SIG_IGN` is inherited across `fork` and
  survives `exec`, while handlers do not. So a runtime disposition can
  arrive from the test harness, `cargo`, or the invoking shell without
  appearing anywhere in the source. Revision 1's "two processes with
  default disposition" contradicted its own hypothesis and is
  withdrawn; the assertion no longer appears above it either.
- **Run provenance is a pushed document**, `docs/probe-sigint-evidence.md`:
  exact command, worktree, HEAD, cleanliness, the Cargo suffixes
  executed, result and log digest per physical run. Three caveats stated there rather
  than smoothed over — **R1 and R2 have no preserved log** (revision 2
  double-counted one log as both R2 and R6), **cleanliness is UNKNOWN**
  for every pre-manifest run, and **R1–R10 ran in the
  `panel-mapping-generation` worktree**, not at `main`. Log bodies are
  machine-local under
  `/home/jeans/build/pmacs-gate-targets/probe-sigint-evidence/`; `/tmp`
  is a tmpfs and they were nearly lost to a cleanup mid-lane.
- **THE ONSET IS DATABLE, and it reframes the lane.** `sweep-crdt`
  has **17** logs here. Counted per test copy: **13** with both copies
  `... ok`, **1** where neither executed (stage died compiling `pmacs`,
  `error[E0308]`, `…-708693`), **3** with both `FAILED`. Last observed
  green `20260815T185708Z`, first observed red `20260816T063330Z`; boot
  began 08-14 09:30, so no reboot between. "Pre-existing on `main`"
  holds (F1 at `72da24a`) but **"always broken" is contradicted**.
- **The onset is NOT a source boundary.** Reflog/commit times put HEAD
  at `7599661` during the last green (`3c06176` landed 40s after it
  finished) and `724b785` during the first red (`5174f73` landed
  08:45:41, after that run ended 08:42:01). **Cleanliness captured for
  neither.** `72da24a` is an **ancestor** of `7599661` yet fails today
  while `7599661` passed on 08-15 — but those two observations differ
  in commit AND environment AND time, so they are **non-comparable and
  support no causal conclusion of any kind**. Earlier wordings here
  ("no source-monotonic cause does that", "outcome is not determined by
  commit alone") are both **withdrawn**: different commits can
  deterministically produce different outcomes, so the pair says
  nothing about determinism either.
- **D0a is a decision procedure with no predicted outcome**, and one
  run per endpoint decides nothing. That the failure has appeared only
  in the full sweep is **what has been observed so far**, not a
  property established of the defect.
  **N = 5 full `sweep-crdt` runs per endpoint, counterbalanced
  `AB BA AB BA AB`** — not strict `A/B/A/B`, which leaves B always
  following A and owning the final slot; counterbalancing removes
  systematic order confounding, and the residual last-slot asymmetry is
  accepted and stated. Same captured conditions as D0b plus `uptime`,
  `free`, `/tmp` usage and leaked-daemon count.
- **The run classifier is TOTAL**, and reads only the **two copies** of
  the target test: **green** (both `... ok`), **red** (both `FAILED`),
  **split** (copies disagree → stop; that is its own defect), **void**
  (either copy never executed → discard and re-run, budget 3, then
  D0a stops). A sweep red only on **unrelated** rows is a `green` run;
  both outcomes occur in the historical logs — `…-708693` is a void
  (compile failure), and `…-2839374`/`…-830195` are unrelated-red with
  both copies passing. A bisect of `7599661..724b785` is permitted
  **only on the expected-direction clean split** — all N green at
  `7599661`, all N red at `724b785`. The inverted split is a real
  difference but contradicts the onset reading, so it is recorded and
  that reading is re-examined before any bisect. A mixed result means
  the failure is intermittent under fixed source and **no bisect is
  justified**.
- **Red full-sweep count is SEVEN, not five** (F1–F7 in the manifest),
  each with its own log digest; revision 3 said 5/5 while the framing
  separately cited a gate run the manifest never listed.
- **D0 precedes every other diagnostic**, in two parts: **(a)
  reproduce the onset endpoints `7599661` and `724b785` clean, in
  isolated target dirs**, under the N = 5 interleaved clean-split
  contract below — a bare difference decides nothing. **(a) is DONE.**
  **(b) is RETIRED as a precondition** (2026-08-19) because D0a yielded
  a reliable direct reproduction and D1/D2 measure the mechanism
  itself. **Its obligation is now SATISFIED** by §4c's controlled
  explanation of the subset/full difference, so **D0b is not owed and
  will not run**. As originally written it said: its obligation
  survives under A3 — if D1/D2 do not
  account for the subset-vs-full difference, D0b runs before the lane
  closes. It read: re-run
  the matrix at `main` under a harness capturing provenance **and the
  artifact hashes executed at run time**, since command shape silently
  changed the binary once already and a hash computed later reflects
  only what occupies that path now.
- **Coherence: journey steps touched: NONE** (framing §9). Earlier
  entries here assigned 12(a) "closing is clean" on the premise that
  the lane repairs Ctrl-C teardown; §4c withdraws that premise, because
  no product behaviour changes. What the lane affects is **gate
  trustworthiness**.
- **Gates:** `./scripts/gate --protocol --acceptance
  gpu_invocation_acceptance` at minimum. **The old three-consecutive-
  run A2 contract is SUPERSEDED** — it was written for a teardown fix
  whose flakiness was unexplained. The mechanism is now known and
  deterministic, so the acceptance set is framing §8's A1–A7: guard
  bite, direct-test diagnosis, unaffected foreground success, mutation,
  an otherwise unchanged gate, a distinct `error` outcome, and a
  non-Linux-unix statement.

## `scripts/gate` TMPDIR isolation — MERGED as #240 (`72da24a`)

- **MERGED 2026-08-13T20:12:58Z** at head `cf09f5a`, merge commit
  `72da24a`. Absorbed 2026-08-20; the block below was written while the
  PR was open and is kept for its reasoning, not its status.
- **Its isolation earned itself back during §5b review round 4.** Two
  `m4_24_*` base-resolution rows were reported as pre-existing local
  failures that only CI could arbitrate. They were neither: the sweep
  had been run BY HAND, outside `scripts/gate`, so it inherited
  `TMPDIR=/tmp` — and `/tmp/.git` exists on that machine, which project
  detection walks up into. Run through the gate, both rows pass. **The
  hazard this lane fixed is exactly the one that reappeared the moment
  the gate was bypassed.**

**Written with the branch's first commit**, per the standing correction
from #171 and #215.

- **PR #240** — https://github.com/levineuwirth/pmacs/pull/240.
- **Branch `gate-tmpdir-isolation`**, base `githubsucks/main` @
  `ca92796` exactly (the #239 merge). **Recover with `git fetch
  githubsucks && git checkout gate-tmpdir-isolation`.**
- **Framing `docs/gate-script-framing.md`, revision 6 — AWAITING
  APPROVAL, and the PR must not merge before it has it.** An earlier
  version of this bullet claimed "no framing" on the grounds that the
  fix was already recorded as standing. **`AGENTS.md` grants no such
  exception**: its workflow is framing → approval → branch → implement,
  unconditionally. Revision 6 widens §2's existing isolation
  responsibility to `TMPDIR` rather than adding a feature, which is why
  it amends this document instead of opening a new one.
- **What it does:** every gate invocation gets a fresh, disk-backed
  `TMPDIR` at `<gate-root>/tmp/<mktemp>`, exported once so every stage
  and
  every process they spawn inherits it, and reaped by the same exit trap
  as the ambient root. **A gate run no longer needs a `TMPDIR=`
  override.**
- **A CHILD OF `/tmp` WOULD NOT HAVE WORKED.** The hazard is an
  ANCESTOR marker — project detection walks upward — so a fresh
  subdirectory of `/tmp` inherits `/tmp`'s ancestors and the same stray
  `.git`. The directory had to move somewhere the gate already owns.
- **The socket-path limit shaped the layout, and the fix's own gate run
  found it.** The budget is the **supported-platform floor of 103
  usable bytes** — Darwin's 104-byte array minus its terminating NUL,
  not Linux's 108, because a Linux-derived limit passes where it is
  written and bind-fails on the macOS leg. A path cannot exceed that
  and the suites bind sockets inside `TMPDIR`. The first placement,
  `$TARGET/gate-tmp/$STAMP-$$`, produced a 114-byte socket path and
  failed **six** daemon and attach tests with *"path must be shorter
  than SUN_LEN"*. It now hangs off the **gate root** (36 bytes) rather
  than the per-worktree target (60), with a short name: **47 bytes**,
  leaving 61 for fixtures.
- **A startup guard converts that failure class into a named one.** Six
  socket failures deep in a suite name a limit, not a cause. The guard
  fails immediately with the path, its length and what to shorten.
  **Its reserve is measured, not round**: the longest suffix a fixture
  appends is **33** bytes (`/.tmpXXXXXX/directory-target.sock`), so
  **48** leaves ~45% headroom. Two earlier values were wrong in
  OPPOSITE directions — a "generous" 45 that fired on the gate's own
  behaviour tests, then a 30 that sat **below the real maximum** and
  would have passed 76–78-byte paths. The suite now roots its nested
  gates at a short base so it can SATISFY the unchanged production
  guard rather than be exempted from it.
- **Witnesses, each mutation-checked.** `M-G-1b` keeps the assignment
  and removes only `export` → the propagation row; its predecessor
  `M-G-1` deleted both and so never proved inheritance. `M-G-2` stops
  the reaping → the cleanup row. `M-G-3` removes the ancestor check →
  the refusal row. `M-G-4` reverts to existence-only, and `M-G-5`
  reverts **only** the language-marker arm → the marker-type row, which
  is why that row covers a `Cargo.toml` **directory** as well as both
  `.git` shapes. `M-G-6` counts characters → the multibyte row.
  `M-G-7` moves the trap back after the guards → the
  rejection-cleanup row. `M-G-8` restores the old
  `for _anc in $(...)` loop → the canonical-walk row, which is what
  proves the traversal neither word-splits a space-bearing root nor
  misses a marker visible only after `pwd -P`. Propagation is observed
  in a **spawned child**, because the gate exporting a variable would
  only prove the gate can export a variable.
- **`M-G-9` is the one that proves `M-G-6` is not vacuous**, and it
  needs three legs because the hazard is in the *environment*, not the
  code. **Nine mutations in total.**
  - **9a** — mutant gate, probed pair → the row **fails**, and the
    exact-boundary row still passes. Re-run with `/bin/sh` excluded, so
    the CI fallback path is covered too: still fails.
  - **9b** — *same mutant gate*, pair forced to byte-counting → the row
    **passes**. The defect itself, reproduced rather than argued.
  - **9c** — no pair can qualify → the helper **panics** naming what it
    tried. A skip would be indistinguishable from a pass.
- **The multibyte row's axis was wrong, and CI is what proved it.** The
  row set `LC_ALL=C.UTF-8` and assumed a character count followed.
  `${#x}` counting characters is a property of the **shell** first:
  `bash` counts characters under a UTF-8 locale, **`dash` counts bytes
  under every locale**. `/bin/sh` is `bash` here and `dash` on the
  Ubuntu runners, so the row panicked on CI — the loud failure working
  as designed, but on a machine where the distinction is unobservable.
  The helper now probes `(shell, locale)` pairs and invokes the gate
  **through** the qualifying shell. That is not a contrivance: `#!/bin/sh`
  resolves to `bash` on Arch **and on macOS**, which is exactly where a
  `${#VAR}` guard would miscount.
- **Proved against the live hazard:** `/tmp/.git` is still present on
  this machine, and the tests it reddened now pass with **no override**.
- **Gates:** `./scripts/gate --acceptance gate_script_acceptance`, run
  with `env -u TMPDIR` — the point is that it needs no override. No
  `--protocol`: no wire change.
- **Out of scope, deliberately:** the overdue absorption of #239. This
  lane is the gate fix and nothing else.

## GUI arc Stage 1a — `TextInput` at v24 — MERGED as #239 (`ca92796`)

- **MERGED 2026-08-13T13:00:14Z** at head `36a3296`, merge commit
  `ca92796`. Absorbed 2026-08-20; the block below was written while the
  PR was open and is kept for its reasoning, not its status.

**Written with the branch's first commit**, per the standing correction
from #171 and #215.

- **PR #239** — https://github.com/levineuwirth/pmacs/pull/239.
- **Branch `gui-stage1a-textinput`**, base `githubsucks/main` @
  `4f77491` exactly. **`githubsucks/gui-stage1a-textinput` is the
  authoritative tip** — the ref, not a SHA. Recover with
  `git fetch githubsucks && git checkout gui-stage1a-textinput`.
- **No new framing.** `docs/gui-stage1-input-framing.md` already governs
  every Stage 1 slice; A1–A9, the eight Q#S1-9 precedence rules, §8's
  wire contract and §11's gates are ruled there. Writing a 1a framing
  would duplicate an approved document.
- **First commit is a GROUND-TRUTH RE-MEASUREMENT of §2 (revision 12),
  not code.** §2 was taken at `a994f37`, before 1-pre moved almost every
  GPU-side coordinate in it. It is refreshed here rather than in its own
  PR so the contract and the code that depends on it are reviewed
  together. **No ruling changes.**
  - Moved: `window_event` `:2734`/655 lines → **`:4450`/four lines**;
    `translate_key` `:10975` → **`:12053`**; the eight-arms description
    → three family decision functions over nine variants.
  - **CORRECTION, wrong at BOTH anchors: "`KeyEvent.text` is never
    read" is false** — the AltGr rule reads it (`a994f37:2800`, now
    `main.rs:3251`). The true claim is narrower: `text` is never read as
    the text a keypress **inserts**, only as a *discriminator*. This is
    load-bearing for A5, because §5's rule 2 already exempts "printable
    Ctrl+Alt recognized by the existing AltGr rule" — **1a widens `text`
    from discriminator to payload, and that is the change of kind the
    old wording hid.**
  - **CORRECTION: A4's exit site.** 1-pre moved the mechanism without
    changing behaviour. **A4 edits `apply_keyboard`'s branch and return
    type, not `window_event`**, and **`EventOutcome` survives A4** — a
    native close still returns `Exit`.
- **PROTOCOL-BEARING at v24 and therefore SERIALIZED.** `TextInput` is
  an **appended** `FrontendEvent` variant; never widen a field in place,
  because postcard is positional. `PROTOCOL_VERSION` is **23** at this
  base and `ADVERTISED_PROTOCOL_VERSION` stays pinned at **20** and must
  not be edited to chase it. A frozen-byte pin goes on `FrontendEvent`'s
  **previous final variant**, since an appended variant's own round-trip
  cannot detect a discriminant shift.
- **A `PROTOCOL_VERSION` bump's blast radius is every version-sensitive
  test and NONE of them appear in the diff** — handoff §1a records eight
  such failures across six suites on the last bump, of which CI showed
  one because cargo stops at the first failing target. **Sort them:** a
  tripwire `assert_eq!(PROTOCOL_VERSION, N)` is meant to fire; an
  absolute contract expressed as arithmetic on a moving constant is a
  defect. **`ADVERTISED_PROTOCOL_VERSION == 20` must NOT fire.**
- **IMPLEMENTED. All sixteen gates green** under an isolated `TMPDIR`
  (log `20260812T204615Z-215223`):

  ```
  ./scripts/gate --protocol \
    --acceptance gui_stage1a_acceptance \
    --acceptance gui_stage1a_wire_acceptance \
    --acceptance auto_pair_acceptance \
    --acceptance discovery_stage2_acceptance \
    --acceptance statusline_segments_acceptance \
    --acceptance vterm_stage3_acceptance
  ```

  **`--protocol` is required** — 1a changes the wire — and it is what
  adds the crdt build and the second sweep. `gui_stage1a_wire_acceptance`
  is `#![cfg(feature = "crdt")]`, so it runs **2 tests in the crdt sweep
  and 0 in the default one**; checked in the logs rather than assumed,
  because a suite that compiles to nothing reports `ok`.
- **Evidence: 8 + 2 acceptance rows, 6 producer/router rows, and the
  A1–A4 unit witnesses; mutations M-1a-1 … M-1a-6b, each failing the row
  it targets.**
  - `M-1a-1` single-scalar back through the generic insert → the record
    row **and** the auto-pair row.
  - `M-1a-2` `break_command_chain` deleted → the primed-chain row alone.
  - `M-1a-3` `TextInput` selection back below the intercept return →
    the intercepting-producer row alone.
  - `M-1a-4` typed text through `encode_paste` → A8, with the forbidden
    bytes at the PTY.
  - `M-1a-5` inbound gate disabled → the v23 row, with `REFUSED` inside
    the CRDT op.
  - `M-1a-6b` a variant inserted **before** `PanelPointer` → the
    frozen-byte pin, discriminant 15 → 16.
- **`M-1a-6` (the first attempt) was a WRONG MUTATION, not a vacuous
  pin**, and is recorded because the failure mode is instructive: the
  wedge went in *after* `PanelPointer` — where an append belongs — so
  nothing shifted and the pin passed, correctly. A mutation aimed at the
  wrong side of a boundary reports a sound pin as worthless.

## GUI arc Stage 1 — 1-pre MERGED as #237 (`d038f71`); 1a is next, NOT STARTED

**The lane is rewritten, not removed.** Rule 4 removes a lane when its
ARC is done, and the arc is Stage 1 as a whole: **five slices remain**.

- **Framing `docs/gui-stage1-input-framing.md`, revision 11, APPROVED**
  after eight rejected revisions. It is Stage 1's framing for **all**
  slices and governs every later branch; only Stage 0 was framed by the
  arc document itself. Revision 9 is the approved design; 10 recorded a
  scope correction found against the 1-pre implementation; **11 retracts
  10's claim that P2 was satisfied by route classification alone**, and
  corrects the Stage 1a consequence 10 got wrong.
- **Slice order, each its own branch and PR:** ~~`1-pre`~~ → **`1a`**\* →
  `1b` → `1c` → `1d` → `1e`\*. **`1a` and `1e` are the two
  protocol-bearing slices — v24 `TextInput` and v25
  `OpenTarget`/`OpenTargetResult` — and they are SERIALIZED** against
  each other and against every other wire change in the project. 1c is
  **not** protocol-bearing, under Q#S1-8.
- **NEXT: 1a, and nothing of it exists yet** — no branch, no code. It
  carries the **v24** `TextInput` variant, so it must run alone.
  `PROTOCOL_VERSION` is **23** at this base, and
  `ADVERTISED_PROTOCOL_VERSION` stays pinned at 20 and must not be
  edited to chase it. Its nine contracts (A1–A9) and the Q#S1-9
  precedence rules are in the framing; **A4 deletes the idle-Escape
  local quit**, which is **pre-existing behaviour, not something 1-pre
  introduced** — 1-pre preserved it and moved it behind an
  `EventOutcome` return, and 1a removes it. `EventOutcome` survives that
  removal: the native close still returns `Exit`.

### What 1-pre landed, and the facts worth not re-deriving

- **`App::window_event` went from 655 lines to four**: call
  `dispatch_window_event`, exit if it asks. Deciding is
  `route_event(&WindowEvent) -> Route`, composing one decision function
  per family; performing is seven `apply_*` methods. **No behaviour
  change, no wire change.**
- **A route carries the DECISION, not the effect.** A wheel route holds
  a delta; whether that becomes a viewport update, a panel event, a
  terminal event or nothing depends on `State`. This is why P2 needs a
  second harness, and why revision 10's contrary argument was retracted.
- **Two harnesses.** `RoutingHarness` answers *where did this event go*
  (13 rows, no GPU). `EffectHarness` answers *what did it do* (9 rows) —
  a real `AttachClient` over a `socketpair` through the real handshake,
  outbox, writer thread and encoder, a real windowless `State`, and the
  production dispatch. **22 witnesses, 24 mutations M1–M24**;
  twenty-three fail their own rows and **M6 is the P3 exception check,
  which must stay green**.
- **`dispatch_window_event` is why P2 was reachable at all.** Left inside
  `window_event`, the dispatch would force a harness to re-implement it,
  and a harness that re-implements what it tests witnesses its own copy.
- **The sentinel is the success condition; the 30 s read ceiling is only
  an error ceiling.** Steps are delimited by a non-coalesceable sentinel
  key rather than a sleep, so absence is never inferred from duration —
  but an unbounded blocking read would wedge the gate instead of
  reddening it, and a hang looks like slowness until the job is killed.
- **TWO ACCEPTED STRUCTURAL EXCEPTIONS, both measured rather than
  asserted.** **P3**: deleting `window_event`'s whole body leaves all
  **265** `pmacs-gpu` tests green, so no headless test in the crate
  observes the delegation — what it covers is now one `if`. **P1,
  keyboard only, and winit's rather than ours**: `KeyEvent` carries a
  `pub(crate) platform_specific` field, so **no
  `WindowEvent::KeyboardInput` can be constructed outside winit**. It
  does **not** reach the pointer families — `DeviceId::dummy()` exists
  for exactly this and all three pointer events are constructible,
  checked before the exception was written down.
- **`EventOutcome` survives A4, and one producer is not one variant.**
  The crate has **exactly one** executable `event_loop.exit()`, in
  `window_event`. A4 removes the keyboard `Exit` producer, leaving the
  native close; the type stays because `dispatch_window_event` must
  still distinguish `Continue` from `Exit` on every event. What A4
  changes is `apply_keyboard`'s signature.
- **The effect rows execute in exactly ONE CI job**, checked with `cargo
  metadata`: `workspace_default_members` is the root `pmacs` package
  alone, so the `test` matrix and `crdt-test` — both bare `cargo test
  --all-targets` — never compile `pmacs-gpu`'s unit tests. Only
  **`gpu-render`** runs them, with lavapipe, `vulkaninfo` and
  `PMACS_REQUIRE_GPU=1`. The harness's adapter assert is
  **unconditional**, so a missing adapter can never become a quiet `ok`.
- **A stray `/tmp/.git` reddened the first gate run and had nothing to do
  with this branch.** Established on signature, not test name, by four
  literal `--exact` invocations; the marker was left in place and an
  isolated `TMPDIR` used instead. **`scripts/gate` does not isolate
  `TMPDIR`** — recorded in `docs/agent-handoff.md` §1, where the
  standing fix is assigned to the gate lane.


## The GUI arc — Stage 0 MERGED as #236 (`f8ad3e7`)

**Written at the branch's first commit**, with the framing, which is
what this arc's own §5 requires of every PR in it. The standing
correction from #171 and #215 was missed at #224 and #225; this lane
exists to stop the streak rather than to note it again.

**The park is discharged**: #227 merged as `b867f64`, and the
file-watcher arc (#233) closed via #234 and #235. Rebased onto
`e67ad07`; the single framing commit replayed with no conflict.

**Stage 0's absorption scope was RE-DERIVED from the tree rather than
taken from this lane's own earlier text, and the earlier text was
wrong in the optimistic direction.** `add0ba1` absorbed **#227 and
#234 only**; a reading of its −532-line diff as "half of Stage 0's
absorption" was too generous. Five stale lanes remain below, and two
`COHERENCE.md` corrections had not been made at all. What is done here
is listed at the commit that does it, not promised here.

- **MERGED as #236 (`f8ad3e7`).** The branch was `gui-arc-stage0`, based
  on `e67ad07` after the 2026-08-11 rebase and branched at `0e4c58d`.
  **Recovery instructions are removed deliberately**: a merged lane that
  still says "checkout the branch" sends a reader to a tip that no
  longer moves. Its content is on `main`.
- **Framing `docs/gui-arc-framing.md`, revision 3, APPROVED
  2026-08-10** after two review rounds (two blocking findings each
  round, closed). It is **also the framing for Stage 0 itself**, which
  is docs-only; Stages 1–10 each require their own framing before their
  branch.
- **The park is over and the work is MERGED.** The branch was parked at
  its first commit until #227 merged, because #227 was 72 `main` commits
  behind and touched the three files Stage 0's absorption rewrites.
  #227 merged (`b867f64`), #233's arc closed (#234, #235), this branch
  rebased onto `e67ad07`, the absorption ran, and it landed as **#236**.
- **What landed (docs only, no `src/`):** the absorption pass
  enumerated in the framing's §5 — five stale lanes, the
  authority/recovery anchor, `COHERENCE.md`'s `v6..=v21` → `v6..=v23`,
  the U4 correction and the U9 rewrite in `docs/ci-red-signatures.md`,
  the stale right-click backlog line, and journey step 11's verdict
  (falsified by #232) — then the per-frontend journey table, the §16
  product subgrade the scorecard will point at, §20 placement, the
  handoff cross-reference, and the "Arc 8" retirement.
- **Two absorption items that are NOT simple deletions**, recorded here
  because getting them wrong is silent: **#228's lane** must lose only
  its PR-specific block, while the standing **Discovery lane (P4)** is
  rewritten to "Stage 2 merged; later discovery work remains" —
  predicate evaluation, command metadata, help unification and the
  prefix decision are all still open. And **`v6..=v23` must not sweep
  away the same row's "production attach remains v20"**, which is
  correct (`ADVERTISED_PROTOCOL_VERSION` is 20).
- **Verification (2026-08-11, committed branch tip): FULL PRE-PR SUITE
  PASS.** Stage 0 changes no code, so there is no touched acceptance
  suite; that does **not** exempt a docs-only PR from the standing gates
  in `AGENTS.md`. Ran `cargo fmt --check`; `cargo clippy --workspace
  --all-targets -- -D warnings` as its own step; `cargo test --lib`;
  `cargo test --lib --features crdt`; `cargo test --test m4_acceptance --
  --skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`;
  and `git diff --check`.

## Git integration — STAGE 1 MERGED as #227; Stage 2 must be scheduled alone

**PR #227 MERGED 2026-08-11** (`main` @ `b867f64`), after five review
rounds, a macOS CI round, and a base refresh: it was held unmerged
behind issue #233 by the 2026-08-10 ruling, refreshed onto the merged
base (`e2394c7`, a clean merge whose only file shared with #234 was
this ledger), re-gated 11/11 locally and 14/14 on CI. Durable facts
are absorbed in `docs/agent-handoff.md` §1; the framing
(`docs/git-integration-framing.md`, revision 5) and the PR carry the
full five-round review history.

**What shipped:** `*git-status*` — a `listview` panel over
`git --no-optional-locks -C <dir> status --porcelain=v2 --branch -z` —
and `*git-diff*` (file-level, plain generated text), with the
install-once `keys` extension on `listview`. `builtin/runtime/git.lua`,
34 acceptance tests, **no wire change**.

**Stage 2 (gutter markers) is the remainder, and it is NOT freely
schedulable: it needs new `DecorationKind` variants — a
`PROTOCOL_VERSION` bump — so it must run alone**, per the strict
serialization rule on wire changes. No branch, no framing yet.

**Residue that stays live here:**

- **§9 negative impact stands:** git runs as a spawned process, and
  spawned processes do not appear in `*workers*`. A fifth
  unattributable background thing, labelled honestly; a label is not
  attribution. The D3 lane (above) and §9 Stage 2 own the model.
- **The latent macOS sibling:** `tests/gpu_invocation_acceptance.rs`
  writes a non-UTF-8 filename to disk inside
  `#[cfg(feature = "crdt")]`, and the crdt job is ubuntu-only — it
  fails the day that job gains a macOS leg, the same way `g6_2` did
  (handoff §1: macOS cannot hold a non-UTF-8 filename).

## Worker identity Stage 1 (§9) — MERGED as #232 (`3cc1b85`)

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
    25 in the suite — 24 before this round, plus this one; an earlier
    revision of this bullet said 26): the process message contains
    `pmacs.process.list`
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
  - **Gate: all 13 steps green at `cb7730d`** (log
    `20260809T200907Z-2672209`). **The two preceding runs of the same
    command were red on step `12-sweep`, on a DIFFERENT wall-clock
    render-budget test each time** (`20260809T195332Z-2113672`,
    `20260809T200120Z-2427128`; load average 12.9/23.9 with sibling
    lanes building). All three pass in isolated reruns, none reds twice,
    and the diff is two string literals, their doc comments and one
    test — no render path is touched. Recorded as **U7** in
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

## Discovery lane (P4) — STAGES 1 AND 2 MERGED (#207, #228); LATER WORK REMAINS

**Rewritten, not removed, and the PR-specific Stage 2 block above it is
retired into this one.** Rule 4 removes a lane when its ARC is done;
this arc is not. **Deleting this lane on the strength of one merged
stage would have dropped four named pieces of open work** — predicate
evaluation, command metadata, help unification, and the prefix
decision — which is exactly why the coalesce is spelled out rather than
left to judgement. Durable facts from both stages are in
`docs/agent-handoff.md` (§1 for Stage 1; the v23 frozen-variant lesson
in §4 for Stage 2).

- **Landed, Stage 1:** eleven `help.*` commands over the existing
  registries, indexed by `M-x help`, with `editor.describe-command` /
  `editor.describe-setting` kept as forwarders. No Rust, no protocol
  change. §5 moved substrate-without-surface → **Partial**.
- **Landed, Stage 2 (#228, `0857bf4`):** M-x rows carry descriptions
  over **protocol v23** — `MinibufferPromptRows` appended while
  `MinibufferPrompt` stays frozen and still sent for `12..=22`, gated as
  a range on both sides so exactly one variant reaches any peer.
- **Still open, and the reason this lane survives:**
  1. **`Command` gains title / category / aliases / flags /
     arg-schema** — a Rust type change across ~147 definition sites.
     MCP currently works around the missing schema by stuffing rendered
     JSON into the description string.
  2. **Predicate evaluation.** `Command.predicate` is read at
     `src/help.rs:76` and one test, and **evaluated nowhere**. Starting
     to evaluate it makes commands stop being invocable, so it needs its
     own decision about what "unavailable" means at each call site —
     M-x, dispatch, menu. `discovery_acceptance`'s `d9` pins today's
     behaviour with a *raising* predicate, so that stage must change the
     pin knowingly.
  3. **Help-layer unification.** `src/help.rs` is still orphaned. Stage 1
     funnels every command through `pmacs.editor._show_help` and renders
     via named per-subject functions, so the work is enumerated:
     replace the four subjects `src/help.rs` covers (key, mode, hook,
     buffer) and **write three new Rust renderers** for settings, lists
     and apropos.
  4. **The help prefix.** Deliberately untouched by Stage 1 — the
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
