# Manual CI dispatch — a contemporaneous run on any ref

**Status: revision 3 — AWAITING APPROVAL. Nothing implemented.**

Revision 3 answers review of 2: D2 could pass without the two runs ever
overlapping, D3 accepted an aborted job as evidence the matrix ran, the
merge-base instruction named the wrong commit and the wrong command, and
"when it runs automatically" was still too strong.

Revision 2 answered review of 1, whose first change was a large
reduction in what this claims to be worth.

## 1. What this fixes

`ci.yml` triggers on `push` to `main` and on `pull_request`. There is no
way to run it on demand, so **a CI run on `main` can only be obtained
from whenever `main` last happened to be pushed** — or by re-running a
job from that stale run.

## 2. Why, and exactly which row needs it

**Revision 1 claimed "four registry rows name a control they cannot
obtain". That was wrong, and the correction is worth stating plainly:
CI never invokes `scripts/gate` — zero occurrences in `ci.yml`.** It
runs `cargo test` directly.

So every local-gate red in the registry has a merge-base control that
needs no CI at all. **Stated precisely, because the sloppy version of
this sentence is itself a trap:** run **the exact failing command, with
its flags**, at **the recorded merge base** — not at whatever `main`
happens to be now — **in a separate worktree**, which this repository's
shared-checkout discipline requires anyway. `git checkout main &&
./scripts/gate` is none of those three things.

Measured against the registry:

| row | where it reds | needs this? |
|---|---|---|
| **U11** | **`Test (macos-latest / lua54)`** | **YES** |
| R7 | local — originally `cargo test --workspace --features crdt` under sweep load, and **its later occurrences at `scripts/gate`'s `gpu` step**, including the two on the parse-budget lane | no |
| U6 | local, `scripts/gate` step `04-lib-crdt` | no |
| U9 | local, `scripts/gate` step `11-sweep` | no |
| U10 | local, `scripts/gate` steps `13-sweep` / `15-sweep-crdt` | no |
| U12 | local, `scripts/gate` step `04-lib-crdt` | no |
| U13 | local, `scripts/gate` step `07-sweep` | no |

**One row, not four.** U11 needs it because it is macOS-specific and
this project has no Mac: its control cannot be run locally at all. When
U11 recurred on PR #243, the only way to get a contemporaneous
`main`-side macOS run without touching `main` or opening a PR was to
re-run a job from a run **eight days old**. That is the whole of the
demonstrated need.

## 3. What this is, and is NOT, for the other experiments

**A no-input dispatch of the unchanged workflow runs exactly what a
push runs.** It therefore:

- **directly enables** a contemporaneous `main`-side run of the
  existing fourteen checks — U11's case, and any future CI-side,
  platform-specific red;
- **does NOT run `scripts/gate`**, which CI does not invoke, so it does
  nothing for R7's in-gate/out-of-gate question;
- **does NOT vary R7's gate conditions**, which would need the gate in
  CI plus a way to change one condition per run;
- **does NOT select U9's alternate test commands**, which would need
  workflow inputs or a branch carrying different commands.

Those remain separate work. **This key is infrastructure they would
build on, not a substitute for them**, and revision 1 blurred that.

## 4. What lands

```yaml
on:
  push:
    branches: [main]
  pull_request:
  workflow_dispatch:
```

One key. No job, matrix, step, permission or timeout changes.

## 5. The two interactions, both checked

### 5a. Concurrency — the guarantee, stated at its real strength

```yaml
group: ci-${{ github.event.pull_request.number || github.sha }}
cancel-in-progress: ${{ github.event_name == 'pull_request' }}
```

**`cancel-in-progress` is evaluated for the INCOMING run.**

**The defensible guarantee is narrow: once a dispatched run is
`in_progress`, a second same-SHA dispatch will not cancel it.**

Revision 1 said "two dispatches cannot cancel each other", which is too
broad. `cancel-in-progress: false` **protects a running run; it does not
protect a PENDING one** — GitHub still replaces an existing pending run
in the same concurrency group. And a `main` push and a dispatch at the
same SHA **do** share `ci-<sha>`, so the two are not isolated from each
other the way revision 1 implied.

A PR run and a dispatch never share a group — PR runs key on
`pull_request.number` — so no test involving a PR exercises the
predicate at all.

### 5b. No job is event-conditional

Measured: `ci.yml` contains exactly one `if:`, and it is
`runner.os == 'Linux'` (line 171). `github.event_name` appears **only**
inside the concurrency predicate (line 28). A dispatched run therefore
expands to the same fourteen jobs a push does.

## 6. Acceptance

**The witnesses split across the merge, and they must.** GitHub offers
`workflow_dispatch` only for a workflow **already on the default
branch**, so no dispatch can be demonstrated before this lands. `gh
workflow view ci.yml` shows a summary and `--yaml` is needed for the
file — but even that reads the default branch's copy, which is why D1
parses the working tree instead.

| # | when | contract | witness | mutation |
|---|---|---|---|---|
| D1 | pre-merge | `ci.yml` is valid YAML and declares the trigger | parse the working-tree file; assert `workflow_dispatch` is a key under `on` | drop the key → the assertion fails |
| D2 | **post-merge** | a second same-SHA dispatch does not cancel a first that is already running | see below | `cancel-in-progress: true` unconditionally → the first ends `cancelled` |
| D3 | **post-merge** | a dispatched run executes the full matrix for real | see below | make any job event-conditional → that job is `skipped` |

**D2's procedure, because the obvious version passes its own mutant.**
"Same ref" does not mean same SHA — `main` can move between the two
dispatches, giving them different groups and letting the mutant
survive. And a cancelled run still reaches status `completed`, so
status is not the discriminator. So:

1. dispatch `main`, record **run id A** and its `headSha`;
2. wait until A is **`in_progress`**, not merely queued — the guarantee
   is about a running run, and a pending one is genuinely replaceable;
3. dispatch `main` again, record **run id B** and its `headSha`;
4. **observe a state in which A is `in_progress` AND B is `queued`.**
   Without this the witness passes vacuously: A can finish naturally
   between steps 2 and 3, and the unconditional-cancellation mutant
   then has nothing to cancel. **If that state is never observed, the
   attempt is VOID — retry it; do not report it as a pass;**
5. **assert `headSha(A) == headSha(B)`**, or the run proves nothing —
   `main` can move between dispatches and hand the two different
   groups;
6. assert **A's final `conclusion` is not `cancelled`**.

**D3's procedure, for the same reason.** A job suppressed by a
job-level `if:` still appears as a check and reports `skipped`, which
rolls up as success — so "all fourteen checks are listed" does not
detect the mutant. So:

1. assert every one of the fourteen expected jobs **started**
   (`started_at` present) **and concluded `success` or `failure`**.
   "Not `skipped`" is not enough — it still admits `cancelled` and
   `timed_out`, neither of which is a job that ran the matrix;
2. keep a **structural** check for step-level conditionals: the only
   `if:` in the file is `runner.os == 'Linux'`, and no expression
   references `github.event_name` or `github.event` outside the
   concurrency block.

**D2 and D3 are recorded as OWED at merge.** This framing does not
pretend a pre-merge check can stand in for them.

## 7. Coherence impact (`COHERENCE.md` §20)

- **Journey steps touched: NONE.** No product behaviour changes.
- **Interaction islands: none added.**
- **Config registry: no entry.**
- **Background work: none started.** A dispatched run is
  human-initiated by definition.

## 8. What this does NOT do

- **It does not make R7, U6, U9, U10, U12 or U13 more answerable.**
  Those are local-gate reds; CI does not run the gate.
- **It does not run any control.** It makes one class of control —
  CI-side, on an arbitrary ref — obtainable without a stale rerun.
- **It does not change what CI runs, or the automatic trigger
  conditions.** Note the narrower wording: SCHEDULING can change, because
  a dispatch sharing `ci-<sha>` may replace an existing **pending**
  `main`-push run. What is unchanged is when CI fires by itself.
- **It does not weaken the PR-superseding saving**, which is keyed on
  `pull_request.number` and gated on the event.
