# CI red signatures — the triage registry

**This file is the authority for judging a red run whose failure might
be a product defect.** A row records what was seen, what is known about
why, and what would retire it. A row is not a claim that the failure is
harmless, and it is deliberately not called a flake.

Everything that was once here and is not a possible product defect has
been closed, with the date and the mechanism that closed it, in the
table at the end. New intermittent reds do not get rows here. They are
filed as GitHub issues with the `intermittent-red` template (selector,
job, required fragments, log link), one issue per mechanism, with later
occurrences as comments on the issue. The `intermittent-red` label is
the list of them and is the thing to read; the first three filed that
way were #248 (a compile-mode wait ending before the finalization
pass), #250 (`ETXTBSY` on a freshly written stub) and #251 (a bundled
package failing to load during a parallel library test run). No number
here is a total.

## How a row matches

A test-name match is never sufficient. A red matches a row only when
the exact test selector matches, the job or flavor matches, and every
required fragment is present in the failure output. Where a fragment
lists alternatives (`ESRCH` / `No such process`), any one satisfies it.
Fragments are normalized, never pasted verbatim: pids, elapsed times
and rendered OS-error suffixes vary between runs. A failure in a listed
test that does not carry that row's fragments is a new incident, judged
on its own; two rows can share a test and differ in one fragment.

## The rerun rule

A green rerun after a red establishes non-reproduction and nothing
more: not environmental cause, not harmlessness, not retirement. The
same signature on the rerun is a second occurrence, not a coincidence,
and it stays blocking pending a merge-base control, which is what
distinguishes "this branch caused it" from "this tree has it". A
different signature is a new incident. Retirement is causal: a row
closes when its mechanism is removed or explained, never by a count of
green runs. A red matching a closed row and postdating its closure
reopens the question; one predating it corroborates the row.

## What one control run establishes

A merge-base control is one CI run at the base, and one run is one
sample --- but the two directions are not the same sample, and no
record may describe them as if they were.

**A positive observation settles existence from one sample.** One run
at the base in which the signature *does* appear establishes,
deductively, that the signature is on the base. That is all it
establishes: existence, never a rate.

**A negative observation settles nothing about absence.** One run at
the base in which the signature does *not* appear is the rerun rule
above, applied at the base instead of at the head. A load-dependent
test is green most of the time over a live defect, so a green control
is non-reproduction and nothing more.

Both must therefore be hedged, and hedging them **in the same words**
is right --- both are one sample. Claiming they are the **same weight**
is wrong, and it is the specific error this section exists to stop
being rediscovered: a single positive and a single negative are not
equally weighted evidence. What a green control changes is which way
the burden falls --- the branch becomes a candidate rather than
excluded --- not how much evidence there is. It diagnoses nothing.

U17's control is the first kind and this branch's macOS control is the
second. Each row says which it is, and neither may borrow the other's
strength.

## Counts of record

The counts this file is the register for, stated once here rather than
recomputed from the run sections, and corrected here where a run
section states them lower.

**Landed on `main` rather than on a phase branch**, under the
registry-location ruling of 2026-09-09. A record committed on a phase
branch cannot describe its own head: the commit moves the tip and
starts the run it would then have to record. `7b6c519` is that fixed
point's proof rather than its assertion — it exists only to record run
34369540895, and its own push produced 34373256548, which it therefore
could not name. On `main` the loop breaks. A commit here changes no
branch, and under `ci.yml`'s changed-paths rule — `code=false` when
every changed path is under `docs/` — it starts none of the nine jobs
that compile or run tests, so it can describe a branch's run after
that run has completed.

Three phases in a row shipped this debt into the next phase's first
commit (E0's rode E1's as `559e8bf`, E1's rode E2's as `f6f36bb`).
E2's does not. E3's three PR runs and `main`'s run after its merge are
recorded below by E4's opening registry commit, on `main`, before E4's
first push.

**A count of record is written as a tally beside its enumeration**: one
line beginning `Tally (<id>):`, and `tests/docs_consistency.rs` asserts
every tally in this file against what it counts --- the rows of the
table or the items of the list under it, the rows of the table above it
carrying a named cell, the distinct values of a column, a sum of
addends, or the range of an enumerated sample. The forms are the
test's, and a count stated in this section without a tally is a defect
of this file. Added 2026-09-10 in E4's fix round 1, because four
consecutive phases shipped a count here that its own enumeration
contradicted (E2: the family at eleven over twelve rows; E3:
`daemon_reships…` at three over four; E4: #259 at 35–47 polls over
samples of 16, 38, 47 and 35), each caught by a reviewer recounting by
hand.

### `main` after E3: run 34483416251 at `2ca2094`, red on the family's first trunk sample

| field | value |
|---|---|
| run | 34483416251, `push`, one attempt |
| head | `2ca2094`, E3's squash merge (PR #265 at `049d81b`, `--match-head-commit`) |
| window | started 2026-09-10T13:33:48Z, completed 13:56:01Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the failure | `Test (macos-latest / luajit)`, job 102891626286 |
| failing target | `daemon_reships_the_summary_after_a_real_buffer_round_trip`, `tests/theme_faces_acceptance.rs:1039:50`, `read Hello: Io(Os { code: 35, kind: WouldBlock, … })`, `test result: FAILED. 26 passed; 1 failed` |
| the skip | `Docs consistency`, correctly: the merge changed code |

`WouldBlock` appears **exactly once** in the job, against 122 `test
result: ok`. This is the **`read Hello` family's fourteenth** occurrence
and its **first on `main`**: the two earlier post-merge runs read for it
(34205653191 at `d97e137`, 34396945488 at `ea8c93a`) carried zero
`WouldBlock`. Under "What one control run establishes" above, one
positive sample settles existence: the signature is on the trunk. It is
**not #258's own selector** — `a16_26_…` ran on the leg and passed — so
**#258 stays at four and D33's revocation condition is not met.**
Recorded on #258 on 2026-09-10; not re-run.

### E3's PR runs, which E3 did not record here

| run | sha | verdict | what |
|---|---|---|---|
| 34404999196 | `34c208f` (C3) | 17 success, 1 skipped, 0 failures | no registry selector fired |
| 34452014666 | `450ef26` (fix round 1) | 16 success, 1 skipped, 2 failures | the family's **thirteenth** (`daemon_reships_…`, `theme_faces_acceptance.rs:1039:50`, macOS luajit) and **#266's second** (`Test (ubuntu-latest / luajit, no crdt)`, `67.007468 ms`, `1 polls`) |
| 34474086323 | `049d81b` (C3 closed) | 17 success, 1 skipped, 1 failure | **#259's fourth** (macOS lua54, `5.007875083s, 58 polls`) |

Each is on its issue with the log link. **#266 is fixed at `f457328`**
(the assertion is `>= 1` and was bitten by deleting the increment) and
its fix was confirmed green on the leg it failed on in 34474086323; two
occurrences, closed by mechanism removal, not by a green count.

Tally (266): 2 items in the list below.

- gate log `20260910T074133Z-120546`, `06-sweep`, at `450ef26` (filed
  from E3's fix round 1)
- run 34452014666 at `450ef26`, `Test (ubuntu-latest / luajit, no
  crdt)`, `67.007468 ms`, `1 polls`

### #256 at TWO, with no row until the owner rules on the archived rate

`process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`,
`live runtime probe`, both local Linux under full-sweep load.

Tally (256): 2 items in the list below.

- gate log `20260908T104328Z-2589144`, `05-sweep`
- gate log `20260910T115801Z-321737`, `06-sweep` (passed in the same
  run's `sweep-luajit`)

The project's own archive
(`docs/archive/framings/ci-crdt-coverage-framing.md:622-629`) parked
this expression at "~1 in 5 under parallel full-suite load" with the
discriminator — a serial full-suite bite — never run, and as a product
defect hypothesis. Whether that archived rate counts toward a total is
the owner's call; until it is taken this file carries the two
occurrences it can vouch for and no rate.

### E4's opening gate at `bbc4dca`: #264's second

`scripts/gate` log `20260910T135544Z-49662`: `clippy` red on E4's own
defect (fixed at `45d1f9d`), and `05-sweep` red on
`daemon_attach::tests::ensure_running_invokes_spawner_then_waits_for_socket_to_appear`
with both of #264's fragments (`expected Ok, got Err(AutoStartTimeout`,
`src/daemon_attach.rs:849:9`), `2205 passed; 1 failed` in the `--lib`
target at 21.78 s. `src/daemon_attach.rs` has no diff on the branch.
**#264 is at two**; on the issue, not re-run.

Tally (264): 2 items in the list below.

- gate log `20260909T204440Z-3628136`, `06-sweep`, `2201 passed; 1
  failed` (E3's fix-round tree; filed as #264)
- gate log `20260910T135544Z-49662`, `05-sweep`, `2205 passed; 1
  failed`, at `bbc4dca`

### The poll cadence on the hosted macOS runners, measured (E4's opening, run 34484377105)

`measure/poll-cadence` at `b059c13` (`main` at `2ca2094` plus an
instrument in `tests/common/ready.rs` and a report step; never to
merge), one `workflow_dispatch`, both macOS legs run twice — at cargo's
default parallelism and under `--test-threads=1`. Every wait through
`wait_with` and `tick_until` records its step, probe and sleep time.
The distributions, per leg, for `wait_with` at the 20 ms poll:

| leg | waits (>= 2 polls) | ms/iteration p50 / p90 / max | mean sleep per `sleep(20 ms)` call, quantiles over waits, p50 / p90 | probe p50 / p90 | waits over 2x the poll | `tick_until`: mean sleep per `sleep(2 ms)` call, p50 | #259's wait: polls, ms/iteration, elapsed |
|---|---|---|---|---|---|---|---|
| macOS luajit, parallel | 225 | 86.2 / 306.2 / 334.7 | 98.0 / 158.2 ms | 0.34 / 251.2 ms | 206 of 225 | 12.5 ms | 38, 92.2 ms, 3.51 s |
| macOS luajit, `--test-threads=1` | 224 | 94.1 / 275.7 / 334.8 | 114.2 / 159.2 ms | 0.22 / 232.7 ms | 208 of 224 | 16.5 ms | 16, 86.3 ms, 1.38 s |
| macOS lua54, parallel | 223 | 75.2 / 112.5 / 154.2 | 102.8 / 159.0 ms | 0.28 / 57.8 ms | 207 of 223 | 13.3 ms | 47, 90.2 ms, 4.24 s |
| macOS lua54, `--test-threads=1` | 224 | 77.2 / 112.3 / 163.6 | 106.2 / 161.7 ms | 0.10 / 65.7 ms | 188 of 224 | 17.5 ms | 35, 98.0 ms, 3.43 s |
| ubuntu luajit | 224 | 13.4 / 16.1 / 45.0 | 20.1 / 20.1 ms | 0.04 / 2.5 ms | 1 of 224 | 2.1 ms | 4, 15.1 ms, 0.06 s |
| ubuntu lua54 | 224 | 13.4 / 16.1 / 81.0 | 20.1 / 20.1 ms | 0.02 / 2.2 ms | 2 of 224 | 2.1 ms | 8, 17.6 ms, 0.14 s |
| ubuntu luajit, no crdt | 90 | 13.4 / 15.0 / 35.1 | 20.1 / 20.1 ms | 0.00 / 0.6 ms | 0 of 90 | 2.1 ms | 6, 16.8 ms, 0.10 s |

**`--test-threads=1` changes nothing**: serial and parallel legs of one
flavor agree to within a few milliseconds at every quantile. **The
sleep column is a mean of means, and the pair that settles the timer
reading is below it** (qualified 2026-09-10, E4 fix round 1; this
paragraph first said "the time is in the sleep" flat). `cadence::Meter`
accumulates `sleep_asked`, `sleep_got` and `sleeps` per wait and records
**no individual sleep**, and `scripts/poll-cadence-report` prints
`sleep_got / sleeps` under its own label "mean actual sleep per call",
so a `thread::sleep(20 ms)` "returning after about 100 ms at the median,
160 ms at p90" and a 2 ms sleep "after 12–17 ms" are quantiles over
per-wait MEANS, against 20.07 and 2.06 ms on the three Linux legs. A
median of means cannot by itself separate a uniformly slow timer from
rare long deschedules; the report's mean against max sleep overshoot
within a wait can, and at p50 it is:

| leg | mean overshoot per call, p50 | max overshoot within a wait, p50 | ratio |
|---|---|---|---|
| macOS luajit, parallel | 78.0 ms | 106.0 ms | 1.36 |
| macOS lua54, parallel | 82.8 ms | 104.9 ms | 1.27 |
| macOS luajit, `--test-threads=1` | 94.2 ms | 116.9 ms | 1.24 |
| macOS lua54, `--test-threads=1` | 86.2 ms | 104.9 ms | 1.22 |

With four or five sleeps in a typical wait, one 400 ms outlier among
20 ms siblings would put that ratio near 4; at 1.2–1.4 the overshoot is
broadly uniform across a wait's sleeps, which **corroborates** the
reading that it is a property of the runner's timer and not of load ---
the inference E4's checkpoint drew from the serial/parallel agreement
and the idle unit tests, now with the column that bears on it.

**Where the time is, per leg --- and it is in the sleep on two legs of
four, not on every macOS leg.** The report's share of elapsed for
`wait_with` at the 20 ms poll:

| leg | asked sleep | sleep overshoot | probe |
|---|---|---|---|
| macOS luajit, parallel | 10.9 % | 41.7 % | **47.4 %** |
| macOS luajit, `--test-threads=1` | 10.9 % | 45.6 % | **43.4 %** |
| macOS lua54, parallel | 17.7 % | 68.1 % | 14.2 % |
| macOS lua54, `--test-threads=1` | 17.9 % | 67.3 % | 14.8 % |
| ubuntu luajit | 95.8 % | 0.4 % | 3.7 % |

On both luajit legs the probe is 43–47 % of all elapsed time in these
waits, and the named-waits block says where: `wait_for_daemon`, n=90 of
225 waits, cadence p50 289.4 ms, probe-max p50 **502.10 ms** --- the
`set_read_timeout(500 ms)` followed by `let _ = read_message::<Hello>`
at `tests/common/ready.rs:209-211`, a read whose result is **discarded**.
Roughly one second in every two that a readiness wait spends on macOS
luajit is a `Hello` read the wait throws away, inside this repository's
own code and not in the runner's timer: the largest single controllable
cost the measurement found. Whether the read stops being discarded is
the owner's, beside `pump_until`'s 2 s and #264's 500 ms, as a fourth
item of the same kind with a measured price on the platform that fails.
At #259's own site the probe is 0.05–0.86 ms and the time is genuinely
in the sleep. So a
deadline written as milliseconds buys about one fifth of the probes its
author counted on macOS, which bears on #259 (its own wait succeeded at
16, 38, 47 and 35 polls --- 1.38, 3.51, 4.24 and 3.43 s of its 5 s ---
in the four green samples: the child is spawned and slow), on
`pump_until`'s fixed 2 s and on #264's 500 ms window (a prediction
there: it has only fired on Linux). On #266's shape, a one-poll 60 ms
window is routine on macOS whenever the probe is a socket read and did
not occur on Linux in this run. Full report on #259, 2026-09-10.
Nothing was fixed from it.

**Limits, so the table is not read as a rate**: one dispatched run, one
tree, one sample of each job's conditions; two macOS runners per flavor
(four VMs), not one machine measured twice; the two modes of one flavor
are one population (286 `wait_with` waits and 448 in total on every
macOS leg, 287 and 449 on ubuntu luajit because Linux arms five
`PMACS_REQUIRE_*` variables macOS does not); and #259's own wait is
**n=1 per leg** in the named-waits block. The report's `Running` count
failed on the colored log, so "122 result lines, 0 FAILED" per leg is
the completeness statement.

**#259's four green samples, and what they change (2026-09-10, E4 fix
round 1).** This section first said "35–47 polls, 3.4–4.2 s", which
drops the luajit serial leg's 16 polls at 86.31 ms; the four are **16,
38, 47, 35 polls** and **1.38, 3.51, 4.24, 3.43 s**, read from the
report's named-waits block, one observation per VM, and the spread
across four VMs is a factor of three, not the tight band the first
sentence implied. Against the 5 s budget that is **28 %, 70 %, 85 % and
69 %** (each leg's polls times its ms/iteration, over 5000), and the
four reds are 5.04, 5.03, 5.03 and 5.01 s at 53, 54, 58 and 58 polls
--- over by 43, 31, 29 and 8 milliseconds, not by a second. Four
unbiased samples whose maximum uses 85 % of the budget make #259 **a
fixture budget calibrated inside the platform's own distribution**, not
an intermittent of unknown mechanism: the readiness file arrives in
every green sample, late, and what is slow is a `python3` interpreter
start on a hosted macOS runner, upstream of anything the fixture polls.
Its title said "never writes its readiness file", which the four
samples contradict; it is retitled and re-disposed on the issue on
2026-09-10. A budget change there is a choice about margin and not a
fix, and it is the owner's.

Tally (259-green-polls): 16–47 over 16, 38, 47, 35.

Tally (259-green-elapsed): 1.38–4.24 s over 1.38, 3.51, 4.24, 3.43.

Tally (259-green-share): 28–85 % over 28, 70, 85, 69.

Tally (259-red-polls): 53–60 over 53, 54, 58, 58, 53, 60, 56.

Tally (259-red-elapsed): 5.00–5.08 s over 5.04, 5.03, 5.03, 5.01, 5.07, 5.08, 5.00.

**#259's fifth occurrence, and its first on `main`**, arrived in the
run this round's own registry push started (34501572442 at `c6afed8`,
below): `5.066744875s, 53 polls`, over by **67 ms** at 95.6 ms per
iteration --- still inside the distribution the four green samples
bound, so the re-disposition stands and the tail is one sample wider.
The two red tallies above carry five values since that run --- and
**six** since the run E4's fix-round push started twenty minutes later
(34502504655 at `352d32f`, below): `5.079980541s, 60 polls`, over by
80 ms at 84.7 ms per iteration, in the same job as the family's
fifteenth. Two reds in two consecutive runs on two trees are two
occurrences, not a rate.

### PR #267's head run 34490620616 at `8e4ed3a`: `M5 Perf Gates` at its 25-minute ceiling, then green on the authorized rerun

Recorded from `main` on 2026-09-10 in E4's fix round 1, after both
attempts had completed; the branch could not record it. Read from the
jobs endpoint, the job logs and the check-run annotations, not from the
verdict line.

| field | value |
|---|---|
| run | 34490620616, `pull_request`, **two attempts** |
| head | `8e4ed3a`, C4's tip (E4, base `8e4ab78`) |
| attempt 1 | created 14:40:38Z, closed 15:10:57Z; 19 jobs: **17 success, 1 skipped, 1 cancelled** |
| the cancel | `M5 Perf Gates` (a required check), job 102916181524, 14:40:56Z – 15:10:56Z, `cancelled`; step 5 left `in_progress`, steps 9 and 10 `pending`, **no log ever uploaded** (`BlobNotFound`) --- it produced nothing. **Filed as #268**, a first occurrence |
| the ceiling | the job's own `timeout-minutes: 25` (`ci.yml:582`): the check-run annotation reads `The job has exceeded the maximum execution time of 25m0s`. The thirty minutes is 25 + 5 --- the deadline fell at 15:05:56Z and the record closed at 15:10:56Z, GitHub's force-termination window for a runner that does not acknowledge a cancel. No second run existed in the PR's `concurrency` group, and the cache key attempt 2 restored full-match carries no attempt id and predates attempt 1, so cold cache is excluded |
| attempt 2 | the authorized rerun of that job alone, 15:28:51Z – 15:31:28Z: job 102934171842, **success in 2 m 29 s** --- release build 2 m 05 s, `running 1 test`, `p50 127.429 µs / p90 164.688 µs / p99 295.954 µs / max 792.185 µs` against a 10 ms threshold, `1 passed`. The run now reads `conclusion: success` at `run_attempt: 2`; attempt 1's record survives at `/attempts/1` |
| the six `Test` legs | all green, read from the logs: zero `WouldBlock`, zero `test result: FAILED`, zero panics, 125–127 `test result: ok` per leg; **no registry selector fired**; all nineteen rows E4 added printed `... ok` on every leg, the two macOS legs included |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (268): 1 item in the list below.

- run 34490620616 attempt 1, job 102916181524, `cancelled` at the
  25-minute ceiling with no log

Under the rerun rule attempt 2 establishes that the ceiling was not hit
that time and nothing more; #268 stays at one. What #268 also says
since this round: `tests/m5_perf_acceptance.rs` bounds every operation
and nothing in aggregate (1100 iterations at up to `PER_KEY_TIMEOUT`
admit ninety-one minutes with nothing printed) and `drain_pending` at
`:153` is the file's one unbounded wait, so a slow attach and a hung
attach are the same object to that witness --- a test that cannot bound
itself cannot tell you which happened. Not fixed here; the owner's
sweep.

**The stale sentence, so the next phase reads it here.** PR #267's body
said, after attempt 1, that the head was not green on a required check
and that a rerun was the owner's; the rerun was taken in the same
review round and the sentence was still there when the round read the
body. The owner counts that as the **fifth stale CI sentence in five
phases' merge artifacts**; the passes name E1's three (review 1's High,
review 2's High 1, the end-to-end round's High 1, each a run or a
refresh behind), E2 review 2's resume table one commit behind, and this
one. The body is mutable and moves no tip, so the correction is free
and the rule is now in the resume: the CI paragraph of the merge
artifact is re-read against the API at the close of every round, after
any rerun, and states every attempt.

### `main` at `c6afed8`: run 34501572442, red on #259's fifth, its first on the trunk

The run E4's fix-round registry push started --- a full run, because
`1f8ba11` reaches `tests/`. Read from the jobs endpoint and the failing
job's log.

| field | value |
|---|---|
| run | 34501572442, `push`, one attempt |
| head | `c6afed8` (`main`: `8e4ab78` plus this round's four registry commits, touching `docs/ci-red-signatures.md` and `tests/docs_consistency.rs` only) |
| window | started 2026-09-10T16:21:27Z, completed 16:41:07Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the failure | `Test (macos-latest / luajit)`, job 102953434516 |
| failing target | `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel`, `tests/bottom_panel_stage1_acceptance.rs:2447:5`, `ready did not become ready within 5s (waited 5.066744875s, 53 polls)`, `No such file or directory`, `test result: FAILED. 49 passed; 1 failed` |
| the skip | `Docs consistency`, correctly: the push changed code |

Zero `WouldBlock` in the job against 122 `test result: ok`, so the
family is unchanged at fourteen. **#259 is at FIVE**: selector, job
flavor and all three fragments, on a tree whose fixture, panel child
and `tests/common/ready.rs` are byte-identical to `2ca2094`. One
positive sample settles existence: the signature is on the trunk. On
the issue, 2026-09-10; not re-run.

Tally (259): 7 items in the list below.

- run 34220035122 at `8f6784f` (PR #257), macOS lua54, `5.042660041s, 53 polls`
- run 34222042303 at `e78d184` (PR #257), macOS lua54, `5.031151625s, 54 polls`
- run 34269795016 at `04263c6` (PR #257), macOS luajit, `5.029068625s, 58 polls`
- run 34474086323 at `049d81b` (PR #265), macOS lua54, `5.007875083s, 58 polls`
- run 34501572442 at `c6afed8` (`main`), macOS luajit, `5.066744875s, 53 polls`
- run 34502504655 at `352d32f` (PR #267), macOS luajit, `5.079980541s, 60 polls`
- run 34506186662 at `b15d935` (`main`), macOS luajit, `5.003208416s, 56 polls`

### PR #267's fix-round head run 34502504655 at `352d32f`: #259's sixth and the family's fifteenth, in one job

The run E4's fix round 1 push started. Read from the jobs endpoint and
the failing job's log; recorded from `main` after the run completed.

| field | value |
|---|---|
| run | 34502504655, `pull_request`, one attempt |
| head | `352d32f`, fix round 1's tip (the six phase commits rebased onto `c6afed8`, plus `dce37f1` and `352d32f`) |
| window | started 2026-09-10T16:30:18Z, completed 16:51:47Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the failure | `Test (macos-latest / luajit)`, job 102956523577, **two** failing targets |
| target 1 | `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel`, `tests/bottom_panel_stage1_acceptance.rs:2447:5`, `ready did not become ready within 5s (waited 5.079980541s, 60 polls)`, `No such file or directory`, `test result: FAILED. 49 passed; 1 failed` --- **#259's sixth** |
| target 2 | `v15_peer_never_receives_theme_facts_and_v16_does`, `tests/theme_faces_acceptance.rs:1393:54`, `read Hello: Io(Os { code: 35, kind: WouldBlock, … })`, `test result: FAILED. 26 passed; 1 failed` --- the **`read Hello` family's fifteenth**, that selector's third; not #258's own selector, so **#258 stays at four** |
| the skip | `Docs consistency`, correctly: the push changed code |

`WouldBlock` appears **exactly once** in the job against 123 `test
result: ok`. The five other `Test` legs are green. Every row the round
added printed `... ok` on this leg (the pin, the witness, the
docs-consistency tally rows), as did every row the phase added. The
branch's diff over `c6afed8` reaches no PTY fixture, no `Hello` read
and no daemon code, which is an argument from untouched files and not a
measurement; both signatures were on the trunk before this push (#259
in the run above; the family since 34483416251). **Not re-run**: reruns
of a red are the owner's, and under the rerun rule a green would
establish non-reproduction and nothing more. On #259 and #258 as
comments, 2026-09-10.

### `main` after E4: run 34506186662 at `b15d935`, red on #259's seventh

Recorded from `main` on 2026-09-10 at E5's opening, before any code.
Read from the jobs endpoint and the failing job's log, not from the
verdict line.

| field | value |
|---|---|
| run | 34506186662, `push`, one attempt |
| head | `b15d935`, E4's squash merge (PR #267 at `352d32f`) |
| window | created 2026-09-10T17:06:37Z, completed 17:27:44Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the failure | `Test (macos-latest / luajit)`, job 102968894349 |
| failing target | `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel`, `tests/bottom_panel_stage1_acceptance.rs:2447:5`, `ready did not become ready within 5s (waited 5.003208416s, 56 polls)`, `No such file or directory`, `test result: FAILED. 49 passed; 1 failed` |
| the skip | `Docs consistency`, correctly: the merge changed code |

`WouldBlock` appears **zero** times in the job against 124 `test result:
ok`, so the family stays at fifteen and #258 at four. **#259 is at
SEVEN**, over by 3 milliseconds at 56 polls (an 89 ms cadence), the
tightest of the seven reds and inside the distribution the four green
samples bound. On the issue; not re-run. Nothing new appeared.

**What E5 does to these rows, on `e5/errors-exist` (PR #269), stated
here so the next reader of a red knows what changed under it.** E5.0's
wait sweep (`docs/audits/2026-09-10-wait-sweep.md` on the branch, 402
collapsed rows) landed, at `ac4c706`: readiness in
`tests/common/ready.rs`'s `wait_for_daemon` is a **served `Hello`** and
no longer a successful connect --- the mechanism this file's `read
Hello` family and #258 measured, removed for every `TestDaemon`
consumer; a daemon that exits before serving is reported on the probe
that sees it; #264's fixture holds its listener until the caller has
returned; #268's perf gate carries a whole-run ceiling, a drain ceiling
and progress lines. At `efa762a`: the 2 s eventually-deadlines at R5's
site (`editor.rs`'s `pump_async`), #263's (`async_runtime.rs`'s
`pump_until` and its hand-rolled copies), `m8_1` and `m4` are 10 s
under D12, and **#259's 5 s is `ready::DEADLINE`**, a margin choice and
not a fix, the measurement above standing as the record of why. Under
the rerun rule none of this retires a row. CORRECTED at fix round
1 (2026-09-11, review 1's High 2): the boot mechanism is gone from the
helper, and the second mechanism the head's run exposed (the section
below) is production's accept quantum and persists, so the family's
closure is its readers' bounds and not a mechanism's absence --- a
`read Hello` red on the trunk would now mean a reader at 5 s or more
that the quantum's tail exhausted, which no measurement predicts --- and
a #259 red at 10 s would carry its poll count.

### PR #269's head run 34524799346 at `aa5177d`: the family's sixteenth and #258's fifth, on the branch that removed the boot mechanism

Recorded from `main` on 2026-09-10 at C5's close. Read from the jobs
endpoint and the failing job's log.

| field | value |
|---|---|
| run | 34524799346, `pull_request`, one attempt |
| head | `aa5177d`, E5's opening head (PR #269, base `b15d935`) |
| window | created 2026-09-10T20:10:43Z, completed 20:33:08Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the failure | `Test (macos-latest / luajit)`, job 103031218766 |
| failing target | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join`, `tests/statusline_segments_acceptance.rs:997:54`, `Io(Os { code: 35, kind: WouldBlock, message: "Resource temporarily unavailable" })`, `test result: FAILED. 12 passed; 1 failed` |
| the skip | `Docs consistency`, correctly: the push changed code |

`WouldBlock` appears **exactly once** in the job against 129 `test
result: ok`; `M5 Perf Gates` green at its new ceilings; every row E5
added `... ok` on the leg. **The finding is where it happened.** At
`aa5177d` the shared readiness wait already declared readiness on a
served `Hello` (`ac4c706`), so this `Hello` was read from a daemon that
had served one: the boot (the mechanism #258's measurement found and
`ac4c706` removed) was one mechanism and not the only one. The second,
NAMED WRONG when this section was written ("its dispatcher's tick") and
corrected at fix round 1 (2026-09-11, review 1's High 2): no dispatcher
is in a `Hello`'s path. `run_daemon` sets the listener non-blocking and
spawns `accept_loop` on its own thread, which sleeps
`ACCEPT_POLL_INTERVAL` (50 ms, `src/daemon.rs`) on every `WouldBlock`
of `listener.accept()`, and the per-attach thread it spawns writes the
`Hello` before anything reaches the dispatcher. So a fresh connection to
a live daemon pays up to one accept quantum plus a thread spawn ---
which is what the C1 paragraph below measured on Linux (connect-to-
`Hello` p50 50.14 ms, "one whole accept quantum") --- and on a loaded
hosted macOS runner, where a 20 ms sleep returns at 78--94 ms, that
quantum lands past a 200 ms read. The quantum is production and
persists; it is not instrument-class and nothing removed it. What
closes the family is therefore its readers and not the mechanism:
every reader of a fresh `Hello` bounded above the quantum's macOS
tail. `66195b5` moved four of the eight sub-second readers
(`statusline_segments` 200 ms, `theme_faces` 250 ms twice,
`vterm_stage3` 2 s) and said "the four sites"; the four it left ---
`gpu_font_acceptance.rs:234` (250 ms), `m11_5_semantic_acceptance.rs:258`
and `:307` (250 ms), `m5_5_acceptance.rs:2336` (500 ms), three of them
this family's own selectors (table rows 3, 4 and 8 below) --- read
under `ready::DEADLINE` since `3dc975e` on PR #269's fix round 1, after
which a grep over every `Hello` read with its read timeout in force
finds zero under 5 s. Whether the 50 ms poll of a non-blocking listener
is a product latency worth a row is the owner's (roadmap decide-list).
On #258, 2026-09-10 and 2026-09-11; not re-run.

### PR #269's tip run 34528196810 at `66195b5`: green, which retires nothing

| field | value |
|---|---|
| run | 34528196810, `pull_request`, one attempt |
| head | `66195b5`, C5's tip (the call-site fix over `aa5177d`) |
| window | created 2026-09-10T20:44:47Z, completed 21:07:34Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

CORRECTED at fix round 1 (2026-09-11, review 1's Medium 1): the verdict
cell above was written `19 jobs: 17 success, 1 skipped, ZERO failures`
at `2a7f656`, the red head's 17 carried forward with its failure struck,
and did not sum; the jobs endpoint says 18. Every `N jobs:` cell in this
file must now sum (`tests/docs_consistency.rs`, from `6b76bec`), and the
two E5 runs carry the sum form beside their tables.

Tally (run-34528196810-jobs): 19 = 18 + 1 + 0.

Tally (run-34524799346-jobs): 19 = 17 + 1 + 1.

Read from the `Test (macos-latest / luajit)` job log (103042232456)
rather than the verdict line: `WouldBlock` **zero** times, `did not
become ready` zero times, 132 `test result: ok`, zero `FAILED`;
`a16_26_…` and #259's `acc28_…` both `... ok`; the served-Hello witness
`... ok` in all 40 copies. That is one sample of a tree that differs
from the red's by three files, which is weaker than a rerun's
non-reproduction (corrected at fix round 1, review 1's Low 6: it cannot
even say the same tree does not reproduce): #258 stays at five, the
family at sixteen, #259 at seven. What changed under them: the boot
mechanism is gone from the helper, the accept quantum is production's
and remains (the head-run section above, as corrected), and the
family's exposure is its readers' bounds --- four sub-second readers
moved at this tip, four more at fix round 1, zero under 5 s after it.
The trunk's macOS legs are samples, not a retirement.

### PR #269's fix-round tip run 34581501943 at `427ae70`: two attempts, the first red on the instrument this round added

Recorded from `main` on 2026-09-11 at fix round 1's close. Read from
the jobs endpoint and the six failing jobs' logs.

| field | value |
|---|---|
| run | 34581501943, `pull_request`, two attempts |
| head | `427ae70`, fix round 1's tip (eight commits over `66195b5`); the run tests the merge of the head into `main` |
| attempt 1 | created 08:54:33Z, completed 09:14:00Z; 19 jobs: **12 success, 1 skipped, 6 failure** |
| the failures | all six test legs, each on exactly one target: `Test (crdt)` 103205922039, `Test (macos-latest / luajit)` 103205922042, `Test (ubuntu-latest / lua54)` 103205922043, `Test (ubuntu-latest / luajit, no crdt)` 103205922092, `Test (macos-latest / lua54)` 103205922107, `Test (ubuntu-latest / luajit)` 103205922143 |
| failing target | `docs_consistency`'s `ci_red_registry_job_cells_sum` (new at `6b76bec`), `508: 19 jobs but the parts sum to 18`, `test result: FAILED. 17 passed; 1 failed` in every one; the macOS luajit leg otherwise 129 `test result: ok` with every `read Hello` selector `... ok` |
| attempt 2 | the six failed jobs rerun by the fix session at 09:15:22Z, completed 09:34:16Z; 19 jobs: **12 success, 1 skipped, 6 failure** |
| attempt 2's failures | the same six legs, the same target, the same line: `Test (crdt)` 103211483869, `macos-latest / luajit` 103211483827, `ubuntu-latest / lua54` 103211483756, `ubuntu-latest / luajit, no crdt` 103211483804, `macos-latest / lua54` 103211483507, `ubuntu-latest / luajit` 103211483842 |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-34581501943-attempt1-jobs): 19 = 12 + 1 + 6.

Tally (run-34581501943-attempt2-jobs): 19 = 12 + 1 + 6.

**What the red was.** A `pull_request` run checks out the merge of the
head into `main` (`c4b4270`, "Merge 427ae70 into 2a7f656"), and `main`
at `2a7f656` still carried the tip-run cell this file corrects above,
`19 jobs: 17 success, 1 skipped, ZERO failures`. The rule `6b76bec` adds
to the docs test --- every `N jobs:` cell must sum --- read that cell on
every test leg and failed by its line. So the instrument built for
review 1's Medium 1 bit in CI on the defect it was built for, on the
merge ref, before the registry commit fixing the cell (`8e4fd7a`,
pushed to `main` at 09:01:46Z inside `316b574`'s push, run 34582104756,
docs-only, green) had reached it. **The rerun was the session's own
wrong premise, stated as such**: it rerun the six failed jobs expecting
the merge ref to be recomputed against `main` at `316b574`, and a rerun
re-executes the run's original merge commit --- every attempt-2 log
reads `HEAD is now at c4b4270`, the merge into `2a7f656` --- so attempt
2 reproduced attempt 1 exactly, which is what a deterministic red does.
A registry correction on `main` reaches a PR only through a new head:
`95a6db4`, a ninth commit whose diff is the rule's module doc saying
so, and its run is the next section. `WouldBlock` zero and `did not
become ready` zero in all twelve logs: nothing here moves #258 (five),
the family (sixteen) or #259 (seven).

### PR #269's final fix-round tip run 34585031832 at `95a6db4`

Recorded from `main` on 2026-09-11 at fix round 1's close. Read from
the jobs endpoint and the macOS luajit job's log.

| field | value |
|---|---|
| run | 34585031832, `pull_request`, one attempt |
| head | `95a6db4`, fix round 1's final tip (the rule's module doc over `427ae70`); the merge commit `a458ce7`, "Merge 95a6db4 into 316b574" |
| window | created 2026-09-11T09:36:04Z, completed 09:58:33Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-34585031832-jobs): 19 = 18 + 1 + 0.

Read from the `Test (macos-latest / luajit)` job log (103217199976)
rather than the verdict line: `WouldBlock` **zero**, `did not become
ready` zero, 132 `test result: ok`, zero `FAILED`; every `read Hello`
selector `... ok` (`a16_26`, `v16_peer`, `daemon_routes`, `m10_10`,
`v15_peer`, `daemon_reships`), #259's `acc28` `... ok`, and both
registry rules `... ok` against this file at `316b574`. One sample of a
merge tree that differs from the two red attempts' by `main`'s three
registry commits and one doc comment: it shows the corrected cell
sums, which was never in doubt, and retires nothing. #258 stays at
five, the family at sixteen, #259 at seven; the family's readers are
all bounded above the accept quantum, and the trunk's macOS legs after
the merge are samples of that.

### PR #269's fix-round-2 tip run 34592170777 at `b3660f2`

Recorded from `main` on 2026-09-11 at fix round 2's close. Read from
the jobs endpoint.

| field | value |
|---|---|
| run | 34592170777, `pull_request`, one attempt |
| head | `b3660f2`, fix round 2's final tip (seven product fixes over `95a6db4` plus the fmt normalization); the run tests the merge of the head into `main` |
| window | created 2026-09-11T11:03:21Z, completed 11:18:12Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| superseded | run 34591403904 at `1cd5e85` (the seven fixes before fmt) was cancelled by the fmt push and carries no verdict |

Tally (run-34592170777-jobs): 19 = 18 + 1 + 0.

One sample of the fixed tree's merge, and it retires nothing by itself:
the eight probes that reproduce review 2's seven findings fail at
`95a6db4` for the reasons the pass gives and pass at this head, each
with a rebuild between edit and run where the probe reaches a binary.
U17 stays at five with the gate's non-CRDT occurrence above; #258 stays
at five, the family at sixteen, #259 at seven.

### PR #269's fix-round-3 tip run 34721284379 at `c629f8e`

Recorded from `main` on 2026-09-12 at fix round 3's close. Read from
the jobs endpoint.

| field | value |
|---|---|
| run | 34721284379, `pull_request`, one attempt |
| head | `c629f8e`, fix round 3's final tip (three product fixes over `b3660f2` plus the fmt normalization); the run tests the merge of the head into `main` |
| window | created 2026-09-12T21:54:53Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-34721284379-jobs): 19 = 18 + 1 + 0.

One sample of the twice-fixed tree's merge, and it retires nothing by
itself: the five probes that reproduce review 3's three findings fail
at `b3660f2` for the reasons the pass gives and pass at this head, each
with a rebuild between edit and run where the probe reaches a binary.
No new red appeared in the round's gate or in this run.

### PR #269's fix-round-4 tip run 34753854886 at `d93715d`

Recorded from `main` on 2026-09-13, from the jobs endpoint.

| field | value |
|---|---|
| run | 34753854886, `pull_request`, one attempt |
| head | `d93715d`, two product fixes over `c629f8e`; the run tests the merge into `main` |
| window | created 2026-09-13T11:13:00Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`; the push changed code |

Tally (run-34753854886-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 103714801833 | success |
| Changed paths | 103714801878 | success |
| Lint (luajit) | 103714801903 | success |
| Lint (lua54) | 103714801977 | success |
| Format | 103714802033 | success |
| GPU Render (headless) | 103714826722 | success |
| Test (crdt) | 103714826724 | success |
| M1 Acceptance Gates | 103714826738 | success |
| M4 Perf Gates | 103714826742 | success |
| M6 Perf Gates | 103714826755 | success |
| M10 Perf Gates (crdt) | 103714826763 | success |
| Test (macos-latest / lua54) | 103714826773 | success |
| M5 Perf Gates | 103714826774 | success |
| Test (ubuntu-latest / luajit) | 103714826784 | success |
| Test (ubuntu-latest / lua54) | 103714826787 | success |
| Test (macos-latest / luajit) | 103714826790 | success |
| Perf budgets (debug) | 103714826791 | success |
| Test (ubuntu-latest / luajit, no crdt) | 103714826883 | success |
| Docs consistency | 103714827466 | skipped |

Both review-4 regressions fail at `c629f8e` for their stated reasons
and pass with the fixes, rebuilt before running. The local protocol
gate passed eight stages, with 4797/0/51 and 4481/0/37 over 133 targets
in each sweep. No new red appeared. This sample retires no known red.

### `main` after E5: run 34767391694 at `aa1363e`, and it is GREEN

Read at E6's opening on 2026-09-13, from the jobs endpoint and the
two macOS job logs; not re-run.

| field | value |
|---|---|
| run | 34767391694, `push`, one attempt |
| head | `aa1363e`, E5's squash merge (PR #269 at `d93715d`, `--match-head-commit`) |
| window | created 2026-09-13T16:02:43Z, updated 16:19:11Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |

Tally (run-34767391694-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 103750691006 | success |
| Format | 103750691084 | success |
| Commit attribution (D9) | 103750691096 | success |
| Lint (lua54) | 103750691129 | success |
| Lint (luajit) | 103750691134 | success |
| Test (crdt) | 103750712211 | success |
| M4 Perf Gates | 103750712217 | success |
| GPU Render (headless) | 103750712227 | success |
| M5 Perf Gates | 103750712231 | success |
| M1 Acceptance Gates | 103750712256 | success |
| M6 Perf Gates | 103750712259 | success |
| Perf budgets (debug) | 103750712285 | success |
| M10 Perf Gates (crdt) | 103750712289 | success |
| Test (ubuntu-latest / luajit, no crdt) | 103750712307 | success |
| Test (macos-latest / luajit) | 103750712309 | success |
| Test (ubuntu-latest / lua54) | 103750712312 | success |
| Test (macos-latest / lua54) | 103750712347 | success |
| Test (ubuntu-latest / luajit) | 103750712358 | success |
| Docs consistency | 103750712786 | skipped |

On both macOS legs (`Test (macos-latest / luajit)` 103750712309,
`Test (macos-latest / lua54)` 103750712347) `WouldBlock` appears zero
times and `did not become ready` zero times, against 132 `test result:
ok` and zero `FAILED` in each. So neither the `read Hello` family nor
#259 sampled on this trunk run; the family stays at sixteen and #259
at seven, and under the rerun rule a green sample retires nothing.
E5's merge is the third consecutive squash whose post-merge run is
read and recorded before the next phase's first row.

### PR #270's head run 34772792926 at `e82fcb5`: U17's sixth, and two fixture reds of the branch's own

Read at E6's close on 2026-09-13, from the jobs endpoint and the two
macOS job logs; not re-run --- the fixture fix is pushed as a new
head.

| field | value |
|---|---|
| run | 34772792926, `pull_request`, one attempt |
| head | `e82fcb5`, E6 at C6 over `7180c17`; the run tests the merge into `main` |
| window | created 2026-09-13T17:50:19Z, updated 18:10:57Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`; the push changed code |

Tally (run-34772792926-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Format | 103765335401 | success |
| Changed paths | 103765335509 | success |
| Lint (luajit) | 103765335515 | success |
| Commit attribution (D9) | 103765335542 | success |
| Lint (lua54) | 103765335648 | success |
| M6 Perf Gates | 103765362229 | success |
| GPU Render (headless) | 103765362234 | success |
| M4 Perf Gates | 103765362245 | success |
| M5 Perf Gates | 103765362253 | success |
| M1 Acceptance Gates | 103765362277 | success |
| Test (crdt) | 103765362278 | success |
| Perf budgets (debug) | 103765362308 | success |
| Test (ubuntu-latest / lua54) | 103765362312 | success |
| Test (macos-latest / lua54) | 103765362318 | failure |
| Test (ubuntu-latest / luajit) | 103765362331 | success |
| M10 Perf Gates (crdt) | 103765362334 | success |
| Test (ubuntu-latest / luajit, no crdt) | 103765362342 | success |
| Test (macos-latest / luajit) | 103765362433 | failure |
| Docs consistency | 103765363275 | skipped |

Three failing targets across the two red jobs. On `Test (macos-latest
/ luajit)` (103765362433): **U17's sixth occurrence**, selector and
fragment exact, recorded on the row above. On both macOS legs
(103765362318 lua54, 103765362433 luajit): two probes new in this
branch, `switch_buffer_ret_takes_the_typed_name_and_tab_completes_it`
and `write_file_prefills_the_root_and_writes_the_typed_name` in
`tests/minibuffer_accept_acceptance.rs`, each canonicalizing a temp
path that pmacs stores as given --- `/var/folders/...` against
`/private/var/folders/...` --- a fixture defect of the branch's own,
deterministic on macOS, no row, fixed on the branch by reading the
name back from the buffer. `WouldBlock` appears zero times and `did
not become ready` zero times in either job against 133 and 132 `test
result: ok`, so neither the `read Hello` family nor #259 sampled.

Tally (run-34772792926-reds): 3 = 1 + 2.

### PR #270's second head run 34774221987 at `e804b97`: U4's fourth and U17's seventh, the fixture fix confirmed

Read at E6's close on 2026-09-13, from the jobs endpoint and the two
macOS job logs; not re-run.

| field | value |
|---|---|
| run | 34774221987, `pull_request`, one attempt |
| head | `e804b97`, the fixture fix over `e82fcb5`; the run tests the merge into `main` |
| window | created 2026-09-13T18:18:33Z, updated 18:35:14Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`; the push changed code |

Tally (run-34774221987-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Lint (luajit) | 103769237122 | success |
| Changed paths | 103769237214 | success |
| Lint (lua54) | 103769237247 | success |
| Format | 103769237261 | success |
| Commit attribution (D9) | 103769237276 | success |
| M5 Perf Gates | 103769261373 | success |
| M1 Acceptance Gates | 103769261393 | success |
| M4 Perf Gates | 103769261396 | success |
| Perf budgets (debug) | 103769261414 | success |
| M6 Perf Gates | 103769261415 | success |
| Test (crdt) | 103769261420 | success |
| Test (ubuntu-latest / luajit) | 103769261443 | success |
| GPU Render (headless) | 103769261447 | success |
| M10 Perf Gates (crdt) | 103769261452 | success |
| Test (macos-latest / lua54) | 103769261471 | failure |
| Test (macos-latest / luajit) | 103769261492 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 103769261504 | success |
| Test (ubuntu-latest / lua54) | 103769261561 | success |
| Docs consistency | 103769261961 | skipped |

Two failing targets, one per red job, both known rows and neither
this branch's: **U4's fourth** on `Test (macos-latest / lua54)`
(103769261471) and **U17's seventh** on `Test (macos-latest /
luajit)` (103769261492), each recorded on its row above. **The
fixture fix is confirmed on the legs it failed on**: the two probes
that reddened both macOS legs at `e82fcb5` pass on both here
(`minibuffer_accept_acceptance`, `7 passed` in each job). `WouldBlock`
appears zero times and `did not become ready` zero times in either
job against 133 `test result: ok`, so neither the `read Hello` family
nor #259 sampled. So the head is not green, on two rows with open
diagnoses that predate the branch, and a merge decision inherits
them as E3's inherited #259's fourth.

Tally (run-34774221987-reds): 2 = 1 + 1.

### PR #270's fix-round-1 head run 34779061403 at `c844de9`: #271 filed on one ubuntu luajit leg, U4 and U17 green

Read at E6 fix round 1's close on 2026-09-13, from the jobs endpoint and the red job's log; not re-run.

| field | value |
|---|---|
| run | 34779061403, `pull_request`, one attempt |
| head | `c844de9`, the `manual_assert` fix over `8e57fa1`; the run tests the merge into `main` |
| window | created 2026-09-13T19:53:29Z, updated 20:13:52Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`; the push changed code |

Tally (run-34779061403-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 103782631063 | success |
| Commit attribution (D9) | 103782631162 | success |
| Docs consistency | 103782655284 | skipped |
| Format | 103782631045 | success |
| GPU Render (headless) | 103782654522 | success |
| Lint (lua54) | 103782630993 | success |
| Lint (luajit) | 103782631121 | success |
| M1 Acceptance Gates | 103782654512 | success |
| M10 Perf Gates (crdt) | 103782654513 | success |
| M4 Perf Gates | 103782654524 | success |
| M5 Perf Gates | 103782654526 | success |
| M6 Perf Gates | 103782654518 | success |
| Perf budgets (debug) | 103782654562 | success |
| Test (crdt) | 103782654515 | success |
| Test (macos-latest / lua54) | 103782654578 | success |
| Test (macos-latest / luajit) | 103782654568 | success |
| Test (ubuntu-latest / lua54) | 103782654658 | success |
| Test (ubuntu-latest / luajit) | 103782654603 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 103782654533 | success |

Four failing targets, all inside `Test (ubuntu-latest / luajit)` (103782654603) within five minutes of one another, filed as #271 in the `intermittent-red` shape: two copies of `readiness_is_a_served_hello_not_a_connect` (`gpu_invocation_acceptance` and `lsp_dispatch_seams_acceptance`, `tests/common/ready.rs:423:9`), each refusing every connect for the full 10 s deadline against a listener the test itself bound in-process (`last: "connect: Connection refused (os error 111)"`); `dedup_upgrade_publishes_the_snapshot_to_preexisting_grid_replicas` (`tests/gpu_invocation_acceptance.rs:487:64`) on a WouldBlock snapshot read; and `m10_10_crdt_op_from_a_reaches_b_via_daemon_broadcast` (`tests/m5_5_acceptance.rs:1169:5`) on a missed broadcast. The branch's diff at the head touches none of the four paths (two new probe files, a comment, the divergences entry), the same tip is green on the local gate (`20260913T195329Z-1785013`, six of six, 139 targets 4843/0/51), all other legs of the run are green, and the base control `aa1363e` ran this leg green --- one runner's bad window for daemon-serving, stated as a candidate, and the discriminating control (a rerun of the job) has not been run.

Tally (run-34779061403-reds): 4 = 2 + 1 + 1.

The superseded head `8e57fa1` ran as 34778589012, cancelled by the `c844de9` push after its `Lint (luajit)` leg had already failed on this round's own committed probe (`manual_assert` in `e6_review1_undo_probes.rs:66` without `crdt`, reproduced locally by the leg's exact command, fixed at `c844de9`); this run's `Lint (luajit)` leg is green, confirming the fix. U4's and U17's selectors all pass on this run --- green samples, non-reproduction and nothing more, and neither count moves. **So the head is not green, on #271 alone**, and a merge decision inherits it the way the previous head inherited U4's fourth and U17's seventh.

### `main` after E6: run 34785311008 at `c177173`, and it is GREEN

Read at E6b's opening on 2026-09-13, from the jobs endpoint and the
two macOS job logs; not re-run.

| field | value |
|---|---|
| run | 34785311008, `push`, one attempt |
| head | `c177173`, E6's squash merge (PR #270 at `c844de9`, `--match-head-commit`) |
| window | created 2026-09-13T21:56:53Z, updated 22:19:11Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |

Tally (run-34785311008-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Lint (luajit) | 103799593193 | success |
| Lint (lua54) | 103799593314 | success |
| Format | 103799593317 | success |
| Commit attribution (D9) | 103799593368 | success |
| Changed paths | 103799593408 | success |
| M5 Perf Gates | 103799610328 | success |
| M1 Acceptance Gates | 103799610342 | success |
| Test (crdt) | 103799610358 | success |
| M4 Perf Gates | 103799610361 | success |
| GPU Render (headless) | 103799610381 | success |
| Test (macos-latest / luajit) | 103799610382 | success |
| Perf budgets (debug) | 103799610384 | success |
| Test (ubuntu-latest / lua54) | 103799610388 | success |
| M10 Perf Gates (crdt) | 103799610390 | success |
| M6 Perf Gates | 103799610399 | success |
| Test (ubuntu-latest / luajit, no crdt) | 103799610449 | success |
| Test (ubuntu-latest / luajit) | 103799610515 | success |
| Test (macos-latest / lua54) | 103799610533 | success |
| Docs consistency | 103799610877 | skipped |

On both macOS legs (`Test (macos-latest / luajit)` 103799610382,
`Test (macos-latest / lua54)` 103799610533) `WouldBlock` appears zero
times and `did not become ready` zero times, against 138 `test result:
ok` and zero `FAILED` in each. So neither the `read Hello` family, nor
#259, nor U4, nor U17, nor #271 sampled on this trunk run; every count
stands where the PR #270 sections left it, and under the rerun rule a
green sample retires nothing. This is the base control for E6b: the
last code-bearing commit on `main`, with a full matrix run of its own.

### PR #272's head run 34791193724 at `3003ca9`, and it is GREEN

E6b's first and only head, read at C6b on 2026-09-14 from the jobs
endpoint and the two macOS job logs; not re-run.

| field | value |
|---|---|
| run | 34791193724, `pull_request`, one attempt |
| head | `3003ca9`, `e6b/styling-under-edit` (base `c177173`) |
| window | created 2026-09-13T23:58:09Z, updated 2026-09-14T00:20:14Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-34791193724-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Lint (luajit) | 103815610606 | success |
| Format | 103815610694 | success |
| Commit attribution (D9) | 103815610705 | success |
| Changed paths | 103815610711 | success |
| Lint (lua54) | 103815610717 | success |
| GPU Render (headless) | 103815627199 | success |
| Test (crdt) | 103815627221 | success |
| M5 Perf Gates | 103815627222 | success |
| M4 Perf Gates | 103815627223 | success |
| M10 Perf Gates (crdt) | 103815627225 | success |
| Test (ubuntu-latest / luajit, no crdt) | 103815627227 | success |
| Test (ubuntu-latest / luajit) | 103815627230 | success |
| Test (macos-latest / lua54) | 103815627236 | success |
| Perf budgets (debug) | 103815627237 | success |
| M1 Acceptance Gates | 103815627249 | success |
| Test (ubuntu-latest / lua54) | 103815627251 | success |
| Test (macos-latest / luajit) | 103815627266 | success |
| M6 Perf Gates | 103815627299 | success |
| Docs consistency | 103815627980 | skipped |

On both macOS legs (`Test (macos-latest / luajit)` 103815627266,
`Test (macos-latest / lua54)` 103815627236) `WouldBlock` appears zero
times and `did not become ready` zero times, against 140 `test result:
ok` and zero `FAILED` in each. The branch's new GPU probe row
(`e6b_gpu_typing_probe_acceptance`) reports `6 passed` in 3.85 s and
2.74 s on those legs and 2.04 s on `Test (crdt)`, elapsed times a
daemon spawn and a real probe take and a skip does not. So neither the
`read Hello` family, nor #259, nor U4, nor U17, nor #271 sampled; every
count stands where the PR #270 sections left it, and under the rerun
rule a green sample retires nothing.

### `main` after E6b: run 35013609842 at `7880c4b`, and it is GREEN

Read at E6c's opening on 2026-09-15, from the jobs endpoint and the
two macOS job logs; not re-run.

| field | value |
|---|---|
| run | 35013609842, `push`, one attempt |
| head | `7880c4b`, E6b's squash merge (PR #272 at `3003ca9`, `--match-head-commit`) |
| window | created 2026-09-15T19:26:13Z, updated 19:45:23Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |

Tally (run-35013609842-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Lint (lua54) | 104531400717 | success |
| Commit attribution (D9) | 104531400965 | success |
| Changed paths | 104531401025 | success |
| Format | 104531401097 | success |
| Lint (luajit) | 104531401219 | success |
| M1 Acceptance Gates | 104531476039 | success |
| M10 Perf Gates (crdt) | 104531476087 | success |
| GPU Render (headless) | 104531476104 | success |
| M6 Perf Gates | 104531476199 | success |
| M5 Perf Gates | 104531476232 | success |
| M4 Perf Gates | 104531476290 | success |
| Perf budgets (debug) | 104531476359 | success |
| Test (crdt) | 104531476365 | success |
| Test (ubuntu-latest / luajit) | 104531476512 | success |
| Test (macos-latest / lua54) | 104531476576 | success |
| Test (macos-latest / luajit) | 104531476581 | success |
| Test (ubuntu-latest / lua54) | 104531476594 | success |
| Test (ubuntu-latest / luajit, no crdt) | 104531476611 | success |
| Docs consistency | 104531477793 | skipped |

On both macOS legs (`Test (macos-latest / lua54)` 104531476576,
`Test (macos-latest / luajit)` 104531476581) `WouldBlock` appears zero
times and `did not become ready` zero times, against 141 `test result:
ok` and zero `FAILED` in each. So neither the `read Hello` family, nor
#259, nor U4, nor U17, nor #271 sampled on this trunk run; every count
stands where the PR #272 section left it, and under the rerun rule a
green sample retires nothing. This is the base control for E6c: the
last code-bearing commit on `main`, with a full matrix run of its own.

### PR #273's head run 35031658924 at `c4be8aa`: U17's eighth, on macOS luajit

E6c's first head, read at C6c on 2026-09-16 from the jobs endpoint and
the two macOS job logs; not re-run.

| field | value |
|---|---|
| run | 35031658924, `pull_request`, one attempt |
| head | `c4be8aa`, `e6c/undo-across-peers` (base `7880c4b`) |
| window | created 2026-09-15T22:34:42Z, updated 22:54:58Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the failure | `Test (macos-latest / luajit)` (104591399407): **U17's eighth**, the job's only failure |

Tally (run-35031658924-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Format | 104591312877 | success |
| Lint (lua54) | 104591313030 | success |
| Changed paths | 104591313059 | success |
| Lint (luajit) | 104591313139 | success |
| Commit attribution (D9) | 104591313206 | success |
| M1 Acceptance Gates | 104591399231 | success |
| Test (crdt) | 104591399358 | success |
| GPU Render (headless) | 104591399360 | success |
| M4 Perf Gates | 104591399361 | success |
| M5 Perf Gates | 104591399381 | success |
| Test (macos-latest / lua54) | 104591399395 | success |
| Test (macos-latest / luajit) | 104591399407 | failure |
| Perf budgets (debug) | 104591399411 | success |
| M10 Perf Gates (crdt) | 104591399416 | success |
| Test (ubuntu-latest / luajit, no crdt) | 104591399447 | success |
| Test (ubuntu-latest / lua54) | 104591399452 | success |
| M6 Perf Gates | 104591399464 | success |
| Test (ubuntu-latest / luajit) | 104591399474 | success |
| Docs consistency | 104591401367 | skipped |

The one failing target is `read_dir_supersede_cancels_in_flight_predecessor`
(`tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded;
got ok`, `left: "ok"`, `right: "cancelled"`, `9 passed; 1 failed` in
2.26 s), U17's selector and required fragment exactly, the job's only
failure against 140 `test result: ok`. On both macOS legs (104591399407
luajit, 104591399395 lua54) `WouldBlock` appears zero times and `did not
become ready` zero times, so neither the `read Hello` family nor #259
sampled. The branch's diff (`git diff --stat 7880c4b..c4be8aa`) touches
neither `src/dispatch` nor `tests/m8_1_acceptance.rs`, an argument from
untouched files and not a measurement; the base control is `main`'s
run at `7880c4b` (35013609842), green on this leg, one sample. The new
suites report on the failing leg's log by elapsed time rather than a
skip: `undo_across_peers_acceptance` `14 passed` in 5.57 s (the GPU
probe row included, an adapter being present on the runner) and
`undo_arbiter_differential_acceptance` `2 passed` in 7.56 s. **So the head is not
green, on U17 alone**, stated at the moment of writing; no rerun taken,
and a merge decision inherits U17's eighth the way PR #270's head
inherited its seventh.

### PR #273's fix-round-1 head run 35087226398 at `a6687b1`, and it is GREEN

E6c fix round 1's head --- `2cd174c` plus four fix commits: the group
opened at `undo.amalgamate = 0`, a detached frontend's undo groups
handed on, the GPU witness at `== ""`, the F1 pin renamed --- read on
2026-09-16 from the jobs endpoint and the two macOS job logs after
the run completed; not re-run.

| field | value |
|---|---|
| run | 35087226398, `pull_request`, one attempt |
| head | `a6687b1`, `e6c/undo-across-peers` (base `7880c4b`) |
| window | created 2026-09-16T10:51:30Z, updated 2026-09-16T11:11:27Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35087226398-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 104764802176 | success |
| Format | 104764802475 | success |
| Changed paths | 104764802482 | success |
| Lint (luajit) | 104764802543 | success |
| Lint (lua54) | 104764802587 | success |
| M1 Acceptance Gates | 104764856410 | success |
| Test (crdt) | 104764856434 | success |
| M4 Perf Gates | 104764856472 | success |
| GPU Render (headless) | 104764856478 | success |
| M5 Perf Gates | 104764856488 | success |
| Test (ubuntu-latest / luajit, no crdt) | 104764856547 | success |
| Test (macos-latest / lua54) | 104764856551 | success |
| Test (ubuntu-latest / luajit) | 104764856558 | success |
| M6 Perf Gates | 104764856573 | success |
| Test (macos-latest / luajit) | 104764856581 | success |
| Test (ubuntu-latest / lua54) | 104764856618 | success |
| M10 Perf Gates (crdt) | 104764856635 | success |
| Perf budgets (debug) | 104764856698 | success |
| Docs consistency | 104764857972 | skipped |

On both macOS legs (`Test (macos-latest / luajit)` 104764856581,
`Test (macos-latest / lua54)` 104764856551) `WouldBlock` appears zero
times and `did not become ready` zero times, against 145 `test result:
ok` and zero `FAILED` in each. U17's selector
`read_dir_supersede_cancels_in_flight_predecessor` ran `ok` on both,
inside `m8_1_acceptance` `10 passed`: a second green sample after the
eighth at `c4be8aa`, non-reproduction and nothing more, and the count
stays at eight. Neither the `read Hello` family, nor #259, nor U4, nor
#271 sampled. The suites the round touched ran on both legs by
elapsed time, not as skips: `undo_across_peers_acceptance` `18 passed`
(6.86 s luajit, 5.47 s lua54; fourteen rows plus the zero-limit
GPU-route row and the three detach rows), `e6c_review1_undo_probes`
`9 passed` (4.66 s, 0.59 s; the review's Medium 1 witness un-ignored
and passing), `e6c_review1_undo_across_peers_probes` `11 passed`
(3.79 s, 2.70 s), `undo_arbiter_differential_acceptance` `2 passed`
(8.14 s, 6.99 s). Both lint legs green, `Lint (lua54)` included.
**So the head is green**, stated at the moment of writing, one
attempt, nothing rerun.

### PR #273's review-round-1 head run 35037221750 at `2cd174c`, and it is GREEN

E6c review 1's head --- `c4be8aa` plus one witness commit, two test
files and nothing else --- read on 2026-09-16 from the jobs endpoint
and the two macOS job logs after the run completed; not re-run.

| field | value |
|---|---|
| run | 35037221750, `pull_request`, one attempt |
| head | `2cd174c`, `e6c/undo-across-peers` (base `7880c4b`) |
| window | created 2026-09-15T23:47:28Z, updated 2026-09-16T00:04:24Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35037221750-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Format | 104608922757 | success |
| Changed paths | 104608923020 | success |
| Lint (luajit) | 104608923057 | success |
| Lint (lua54) | 104608923068 | success |
| Commit attribution (D9) | 104608923110 | success |
| GPU Render (headless) | 104608957091 | success |
| M4 Perf Gates | 104608957112 | success |
| M5 Perf Gates | 104608957160 | success |
| Test (ubuntu-latest / luajit, no crdt) | 104608957171 | success |
| Test (macos-latest / luajit) | 104608957180 | success |
| Test (crdt) | 104608957196 | success |
| M1 Acceptance Gates | 104608957202 | success |
| M10 Perf Gates (crdt) | 104608957211 | success |
| Test (ubuntu-latest / luajit) | 104608957263 | success |
| M6 Perf Gates | 104608957283 | success |
| Perf budgets (debug) | 104608957303 | success |
| Test (macos-latest / lua54) | 104608957311 | success |
| Test (ubuntu-latest / lua54) | 104608957359 | success |
| Docs consistency | 104608958630 | skipped |

On both macOS legs (`Test (macos-latest / luajit)` 104608957180,
`Test (macos-latest / lua54)` 104608957311) `WouldBlock` appears zero
times and `did not become ready` zero times, against 145 `test result:
ok` and zero `FAILED` in each. U17's selector
`read_dir_supersede_cancels_in_flight_predecessor` ran `ok` on both,
inside `m8_1_acceptance` `19 passed`: a green sample after the eighth
at `c4be8aa`, non-reproduction and nothing more, and the count stays at
eight. Neither the `read Hello` family, nor #259, nor U4, nor #271
sampled. The review's two new suites ran on both legs by elapsed time,
not as skips: `e6c_review1_undo_across_peers_probes` `11 passed`
(2.95 s luajit, 2.72 s lua54) and `e6c_review1_undo_probes` `8 passed;
1 ignored` (3.32 s, 1.77 s), the ignored row being the review's Medium
1 witness, named in its ignore reason. **So the head is green**, stated
at the moment of writing, one attempt, nothing rerun; it does not make
`c4be8aa`'s red innocent, and U17's eighth stands.

### PR #274's head run 35163382662 at `a456d6a`: two reds of the branch's own, both macOS legs

E6d, typing on a real file, stacked on E6c's `523ce99` (#273 open at
the session's start; the PR's base branch is `e6c/undo-across-peers`
and it is retargeted and rebased when #273 merges). Read on 2026-09-17
from the jobs endpoint and the two macOS job logs after the run
completed; not re-run --- the fixes are pushed as a new head, whose
run is the section below.

| field | value |
|---|---|
| run | 35163382662, `pull_request`, one attempt |
| head | `a456d6a`, `e6d/typing-on-a-real-file` (base `523ce99`) |
| window | created 2026-09-16T23:41:13Z, updated 2026-09-17T00:02:01Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35163382662-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Changed paths | 105018958497 | success |
| Lint (luajit) | 105018958617 | success |
| Lint (lua54) | 105018958652 | success |
| Commit attribution (D9) | 105018958679 | success |
| Format | 105018958722 | success |
| GPU Render (headless) | 105018990945 | success |
| M1 Acceptance Gates | 105018990955 | success |
| M5 Perf Gates | 105018990981 | success |
| M4 Perf Gates | 105018990994 | success |
| Test (crdt) | 105018990997 | success |
| Perf budgets (debug) | 105018991069 | success |
| M10 Perf Gates (crdt) | 105018991100 | success |
| M6 Perf Gates | 105018991104 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105018991119 | success |
| Test (ubuntu-latest / luajit) | 105018991156 | success |
| Test (ubuntu-latest / lua54) | 105018991168 | success |
| Test (macos-latest / lua54) | 105018991191 | failure |
| Test (macos-latest / luajit) | 105018991229 | failure |
| Docs consistency | 105018991777 | skipped |

Three failing targets across the two red jobs, every one a row new in
this branch, no row of this registry. On both macOS legs (105018991191
lua54, 105018991229 luajit): the `pmacs` unit target's
`semantic_render::tests::e6d_4_summary_waits_for_quiet_after_an_edit_and_no_longer_than_the_lag_cap`,
`assertion failed: summary_of(&s.render_frame(&state)).is_none()` at
`src/semantic_render.rs:6607`, `test result: FAILED. 2221 passed; 1
failed; 12 ignored` --- a row that slept 30 ms inside a 60 ms lag cap
and asserted nothing had shipped, which a loaded runner's oversleep
defeats (the luajit leg's unit target took 138.59 s where Linux takes
30). On the lua54 leg alone (105018991191):
`tests/e6d_incremental_didchange_acceptance.rs`'s
`the_same_edits_leave_every_server_holding_the_buffer_under_both_sync_kinds`,
`after a burst of five edits the server holds what the buffer holds`
with `left: "fn maxyinlet…"` against `right: "fn maxywinlet…"` at
`:202`, `test result: FAILED. 1 passed; 1 failed` in 1.38 s --- the
witness read the fake server's document at the first new line of its
change sink after a step's flush, and a step that sends two
`didChange`s (a completion request flushes on its own) can be read at
the mid-step one when the second lands after the poll; the luajit leg
ran the same target `2 passed` in 3.11 s. Both are timing defects of
the branch's own rows, fixed on the branch (the unit row now moves the
cache's clocks instead of sleeping and runs the production windows;
the witness waits until the server holds the buffer's text, and
reports what it holds otherwise), and the unit row's rewrite exposed a
clock defect in the code it covers, also fixed there. `WouldBlock`
appears zero times and `did not become ready` zero times in either
job against 145 and 146 `test result: ok`, so neither the `read
Hello` family nor #259 sampled; U17's selector ran `ok` on both legs,
its fragment absent, and no live row's fragments appear in either log.

Tally (run-35163382662-reds): 3 = 2 + 1.

### R7's eighteenth, local, on E6d's branch

`scripts/gate` on `e6d/typing-on-a-real-file` at the two CI fixes'
first form (rewritten before the push; the tree is `a456d6a` plus
those two files), log `20260917T001151Z-3116741`, step `05-sweep`:
`attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`,
`transient sequence must attach: Attach(Handshake(Io(Os { code: 32,
kind: BrokenPipe, message: "Broken pipe" })))` at
`pmacs-gpu/src/attach.rs:1971` (the panic line has moved again, from
`:1958`; not part of the signature), `test result: FAILED. 365 passed;
1 failed` on the `pmacs-gpu` unit target, under the default sweep's
load. All three of R7's fragments. The branch's one touch on
`pmacs-gpu/src/attach.rs` is a thirteen-line connect wrapper for the
latency probe (`connect_with_target_and_sink`), which this test does
not call; the retry path has no diff. The gate was re-run at the
rewritten tip for the clippy stage that failed beside it, and R7's
selector passed there: a green sample, which retires nothing. Recorded
on the row: eighteen.
### PR #274's second head run 35166500864 at `db36faa`, and it is GREEN

`a456d6a` plus the two commits its run cost: `d464029`, the
summary-debounce unit row on a moved clock (and the lag-cap clock it
exposed, corrected), and `db36faa`, the incremental witness waiting
for the server to hold the buffer's text. Read on 2026-09-17 from the
jobs endpoint and the two macOS job logs after the run completed; not
re-run.

| field | value |
|---|---|
| run | 35166500864, `pull_request`, one attempt |
| head | `db36faa`, `e6d/typing-on-a-real-file` (base `523ce99`) |
| window | created 2026-09-17T00:25:30Z, updated 2026-09-17T00:44:29Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35166500864-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Format | 105028664901 | success |
| Changed paths | 105028665067 | success |
| Lint (luajit) | 105028665112 | success |
| Commit attribution (D9) | 105028665115 | success |
| Lint (lua54) | 105028665164 | success |
| M4 Perf Gates | 105028702954 | success |
| GPU Render (headless) | 105028702971 | success |
| Test (crdt) | 105028702977 | success |
| M1 Acceptance Gates | 105028703002 | success |
| M10 Perf Gates (crdt) | 105028703006 | success |
| M5 Perf Gates | 105028703011 | success |
| M6 Perf Gates | 105028703022 | success |
| Perf budgets (debug) | 105028703048 | success |
| Test (ubuntu-latest / luajit) | 105028703049 | success |
| Test (macos-latest / luajit) | 105028703055 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105028703057 | success |
| Test (ubuntu-latest / lua54) | 105028703064 | success |
| Test (macos-latest / lua54) | 105028703130 | success |
| Docs consistency | 105028703861 | skipped |

Both macOS job logs (luajit 105028703055, lua54 105028703130) carry 149
`test result: ok` and zero `FAILED`, `WouldBlock` zero times and `did
not become ready` zero times. The two rows red on the previous head
ran `ok` on both legs: the `pmacs` unit target's
`e6d_4_summary_waits_for_quiet_after_an_edit_and_no_longer_than_the_lag_cap`
(the unit target finishing in 137.25 s on luajit and 24.90 s on lua54,
the same spread as before, which the row no longer feels) and
`e6d_incremental_didchange_acceptance`'s
`the_same_edits_leave_every_server_holding_the_buffer_under_both_sync_kinds`.
U17's selector `read_dir_supersede_cancels_in_flight_predecessor` ran
`ok` on both --- a green sample after its eighth, the count staying at
eight --- and no live row's fragments appear in either log. The base
control is `523ce99`'s run 35111050634, 18/1/0.

### `main` after E6c: run 35191607941 at `250646f`, red on the readiness self-test in `Test (crdt)`, filed as #276

E6c's squash merge (PR #273 at `523ce99`, `--match-head-commit`),
which is E6d's base control --- the last code-bearing commit on
`main` under E6d's rebased branch. Read on 2026-09-17 at E6d's fix
round 1 from the jobs endpoint and the red job's log, after E6d's
review round 1 found it unrecorded; not re-run. The gap it fell
through: E6d.0 read `main`'s post-merge CI after E6c before E6c had
merged (the phase was stacked on #273 by the owner's ruling), so no
post-merge run existed at E6d.0 and this one was owed when #273
landed. The resume's §2 now says a phase branched from an unmerged
predecessor reads the predecessor's post-merge run when it lands.

| field | value |
|---|---|
| run | 35191607941, `push`, one attempt |
| head | `250646f`, `main` (E6c's squash merge) |
| window | created 2026-09-17T06:50:04Z, updated 07:15:04Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the merge changed code |
| the failure | `Test (crdt)` (105105244986): `vterm_stage2_acceptance`'s copy of `common::ready::tests::readiness_is_a_served_hello_not_a_connect`, the job's only failure, filed as **#276** |

Tally (run-35191607941-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 105105209414 | success |
| Format | 105105209590 | success |
| Changed paths | 105105209607 | success |
| Lint (lua54) | 105105209617 | success |
| Lint (luajit) | 105105209622 | success |
| Perf budgets (debug) | 105105244870 | success |
| M1 Acceptance Gates | 105105244871 | success |
| M5 Perf Gates | 105105244910 | success |
| M6 Perf Gates | 105105244930 | success |
| GPU Render (headless) | 105105244937 | success |
| M4 Perf Gates | 105105244952 | success |
| M10 Perf Gates (crdt) | 105105244970 | success |
| Test (ubuntu-latest / lua54) | 105105244977 | success |
| Test (crdt) | 105105244986 | failure |
| Test (macos-latest / luajit) | 105105244997 | success |
| Test (ubuntu-latest / luajit) | 105105245002 | success |
| Test (macos-latest / lua54) | 105105245035 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105105245169 | success |
| Docs consistency | 105105246284 | skipped |

The one failing target is `tests/common/ready.rs`'s own self-test of
`wait_for_daemon`, in its `vterm_stage2_acceptance` copy
(`tests/common/ready.rs:423:9`, `Err(Timeout { what: "a daemon
serving on /tmp/.tmpMxHL7F/late.sock", deadline: 10s, elapsed:
10.000088588s, polls: 459, last: "connect: Connection refused (os
error 111)" })`, `running 14 tests` at 07:06:01Z, `test result:
FAILED. 13 passed; 1 failed` in 14.79 s at 07:06:16Z), the job's only
failure against 138 `test result: ok`; `WouldBlock` and `did not
become ready` appear zero times in the job. The same test ran `ok` in
41 other suites' copies of the same job, one process each. The
fragments are #271's first group --- `a daemon serving on
<tmp>/late.sock` + `deadline: 10s` + `connect: Connection refused`
--- **without its other two** (the `WouldBlock` snapshot read at
`gpu_invocation_acceptance.rs:487`, the missed broadcast at
`m5_5_acceptance.rs:1169`), in a suite #271 does not name
(`vterm_stage2_acceptance`, the fourth to carry that in-process
listener test) and on a job it does not name (`Test (crdt)`,
`--test-threads=1`, where #271 is `Test (ubuntu-latest / luajit)`).
Under this file's matching rule that is a new incident: the
selector, the job and the fragment set each fail to match, and a row
widens only on a demonstrated shared object (U16's precedent, below:
a process-global `set_current_dir` and a 3/400-vs-0/400 experiment),
of which there is none here. **Filed as #276** in the
`intermittent-red` shape with the relation to #271 stated there and
a comment on #271 saying this is not its second occurrence; #271
stays at one. The merge's diff against `7880c4b` touches neither
`tests/common/ready.rs` nor `tests/vterm_stage2_acceptance.rs`, and
the same target is `ok` on every leg of the two PR #275 head runs
built on this commit --- an argument from untouched files and green
samples, not a measurement. What one sample shows is the first kind
above: the signature exists on `main` at `250646f`. Every other
leg is green; on both macOS legs (105105244997 luajit, 105105245035
lua54) `WouldBlock` and `did not become ready` appear zero times, so
neither the `read Hello` family, nor #259, nor U4, nor U17 sampled
here.

### PR #275's opening head run 35191736812 at `085ae4d`, and it is GREEN

E6d's eleven commits rebased onto `250646f` after #273's merge
deleted `e6c/undo-across-peers` and closed #274 (`db36faa` →
`085ae4d`; `git range-diff` all `=`, the tree identical but this
file, now `main`'s). Read on 2026-09-17 at E6d's review round 1 from
the jobs endpoint and the two macOS job logs, recorded here at fix
round 1; not re-run.

| field | value |
|---|---|
| run | 35191736812, `pull_request`, one attempt |
| head | `085ae4d`, `e6d/typing-on-a-real-file` (base `250646f`) |
| window | created 2026-09-17T06:51:42Z, updated 07:12:48Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35191736812-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Lint (lua54) | 105105611701 | success |
| Format | 105105611872 | success |
| Commit attribution (D9) | 105105611930 | success |
| Changed paths | 105105612084 | success |
| Lint (luajit) | 105105612230 | success |
| GPU Render (headless) | 105105796592 | success |
| M4 Perf Gates | 105105796638 | success |
| Perf budgets (debug) | 105105796646 | success |
| Test (crdt) | 105105796653 | success |
| Test (macos-latest / luajit) | 105105796654 | success |
| M6 Perf Gates | 105105796659 | success |
| M1 Acceptance Gates | 105105796668 | success |
| M5 Perf Gates | 105105796684 | success |
| Test (ubuntu-latest / lua54) | 105105796711 | success |
| Test (ubuntu-latest / luajit) | 105105796741 | success |
| Test (macos-latest / lua54) | 105105796777 | success |
| M10 Perf Gates (crdt) | 105105796781 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105105796810 | success |
| Docs consistency | 105105797856 | skipped |

Both macOS job logs (luajit 105105796654, lua54 105105796777) carry
149 `test result: ok` and zero `FAILED`, `WouldBlock` zero times and
`did not become ready` zero times; U17's selector
`read_dir_supersede_cancels_in_flight_predecessor` ran `ok` on both,
a green sample after its eighth, and `Test (crdt)` (105105796653)
ran the self-test #276 names `ok` in every copy. Green samples,
non-reproduction and nothing more; no count moves. The base control
is `250646f`'s own run above, red on #276.

### PR #275's review-round-1 head run 35221439579 at `b1ef903`: U17's ninth, on macOS luajit

`085ae4d` plus review round 1's witness commit, two files under
`tests/` and nothing else (`e6d_review1_incremental_probes.rs`,
`e6d_review1_cursor_probes.rs`). Read on 2026-09-17 at the review's
close from the jobs endpoint and the two macOS job logs, recorded
here at fix round 1; not re-run.

| field | value |
|---|---|
| run | 35221439579, `pull_request`, one attempt |
| head | `b1ef903`, `e6d/typing-on-a-real-file` (base `250646f`) |
| window | created 2026-09-17T12:29:53Z, updated 12:47:39Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the failure | `Test (macos-latest / luajit)` (105202355414): **U17's ninth**, the job's only failure |

Tally (run-35221439579-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 105202308755 | success |
| Format | 105202309005 | success |
| Lint (lua54) | 105202309097 | success |
| Commit attribution (D9) | 105202309100 | success |
| Lint (luajit) | 105202309537 | success |
| Test (crdt) | 105202355130 | success |
| GPU Render (headless) | 105202355171 | success |
| M4 Perf Gates | 105202355233 | success |
| M10 Perf Gates (crdt) | 105202355255 | success |
| M6 Perf Gates | 105202355256 | success |
| M1 Acceptance Gates | 105202355322 | success |
| Perf budgets (debug) | 105202355358 | success |
| Test (macos-latest / luajit) | 105202355414 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 105202355431 | success |
| Test (ubuntu-latest / luajit) | 105202355481 | success |
| Test (ubuntu-latest / lua54) | 105202355492 | success |
| Test (macos-latest / lua54) | 105202355528 | success |
| M5 Perf Gates | 105202355533 | success |
| Docs consistency | 105202356275 | skipped |

The one failing target is `read_dir_supersede_cancels_in_flight_predecessor`
(`tests/m8_1_acceptance.rs:278:5`, `first read_dir must be
superseded; got ok`, `left: "ok"`, `right: "cancelled"`, `9 passed;
1 failed` in 2.49 s), U17's selector and required fragment exactly,
the job's only failure against 148 `test result: ok`. On both macOS
legs (105202355414 luajit, 105202355528 lua54) `WouldBlock` appears
zero times and `did not become ready` zero times, so neither the
`read Hello` family nor #259 sampled; the lua54 leg is 151 `test
result: ok` and zero `FAILED`. The commit touches neither
`src/dispatch` nor `tests/m8_1_acceptance.rs` --- it adds two test
files --- an argument from untouched files and not a measurement;
the selector ran `ok` on both macOS legs of the previous head's run
above. The review's two suites ran on both legs, paired by their own
`running N tests` lines: `e6d_review1_cursor_probes` 7 passed (3.22 s
luajit, 2.48 s lua54) and `e6d_review1_incremental_probes` 2 passed
(luajit; 1.77 s lua54). **So the head is not green, on U17 alone**,
stated at the moment of writing; no rerun taken, and a merge decision
inherits U17's ninth the way PR #270's and #273's heads inherited its
seventh and eighth.

### PR #275's fix-round-1 head run 35249161370 at `888e5e1`: one red of the branch's own, both macOS legs

`b1ef903` plus fix round 1's first five commits --- the rust-analyzer
row's oracle reading the edited line whole, the fallback fixture led
by a comment line, the LSP position rule naming the ranged
`didChange`'s mirror, the probe's doc saying what "warm" means, the
debounce measurement kept at the range pull --- three test files, one
invariant paragraph, two doc comments, no production line. The push
at 16:36:55Z updated the branch ref without a push event, so the pull
request did not synchronize and no run started for fourteen minutes;
this run was started by closing and reopening the pull request
(nothing rewritten, the ref already at this sha). Read on 2026-09-17
at fix round 1's close from the jobs endpoint and the two macOS job
logs; not re-run --- the fix is pushed as a new head, whose run is the
section below.

| field | value |
|---|---|
| run | 35249161370, `pull_request`, one attempt |
| head | `888e5e1`, `e6d/typing-on-a-real-file` (base `250646f`; the run merges into `main` at `d06dc28`) |
| window | created 2026-09-17T16:51:31Z, updated 17:14:25Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the failures | `Test (macos-latest / luajit)` (105296974890) and `Test (macos-latest / lua54)` (105296974926), the same row of the branch's own on both, no registry row |

Tally (run-35249161370-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Changed paths | 105296910794 | success |
| Commit attribution (D9) | 105296911028 | success |
| Lint (luajit) | 105296911116 | success |
| Format | 105296911122 | success |
| Lint (lua54) | 105296911138 | success |
| Test (crdt) | 105296974692 | success |
| GPU Render (headless) | 105296974708 | success |
| M1 Acceptance Gates | 105296974718 | success |
| M4 Perf Gates | 105296974731 | success |
| M6 Perf Gates | 105296974751 | success |
| M10 Perf Gates (crdt) | 105296974768 | success |
| M5 Perf Gates | 105296974783 | success |
| Test (ubuntu-latest / luajit) | 105296974867 | success |
| Test (macos-latest / luajit) | 105296974890 | failure |
| Test (macos-latest / lua54) | 105296974926 | failure |
| Perf budgets (debug) | 105296974987 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105296975018 | success |
| Test (ubuntu-latest / lua54) | 105296975240 | success |
| Docs consistency | 105296976390 | skipped |

One failing target on both legs, E6d.3's own fallback witness
`tests/e6d_cursor_confirm_acceptance.rs`'s
`typing_through_a_floor_timeout_keeps_every_character_once_in_order`,
`:256:5`, `the floor released while the fan-out held the daemon (the
positive control)` with `left: "0"` and `right: "1"` --- `fallbacks=0`
in the probe's report --- `test result: FAILED. 6 passed; 1 failed` in
3.97 s (luajit) and 3.77 s (lua54), each job's only failure against
148 `test result: ok`. The report's trace on each leg says why: the
GPU's floor restarts its clock at every optimistic keystroke and
releases only when a daemon message finds it 500 ms old, and the
probe paces its characters on a 10 ms receive timeout the hosted
macOS runners stretch to about 100 ms, so `c` went at 205 ms (luajit)
and 261 ms (lua54) where Linux types it at ~107, against a 700 ms
render-pass spin whose first message landed at 704--705 ms: an age of
499 and 443, and no release. The row was marginal on that platform
by construction (the same runner timer E4 measured at ~100 ms for a
20 ms sleep) and had passed on both legs of every earlier run of this
branch; the fix-round commit before this run touched only the
fixture's first line, which the pacing does not read. Fixed on the
branch as `97b0d0f` (the spin 1000 ms, the pause 1300, the release
window `SPIN_MS..SPIN_MS + 400`; by hand with `c` paced at 268 and at
487 ms the floor still released at 1004--1006), not re-run, pushed as
the new head. On both legs `WouldBlock` and `did not become ready`
appear zero times, U17's selector ran `ok` on both (a green sample
after its ninth), and no live row's fragments appear in either log;
the other three E6d suites and both review suites are green on both
legs. **The head is not green, on the branch's own row alone**,
stated at the moment of writing.

Tally (run-35249161370-reds): 2 = 1 + 1.

### PR #275's fix-round-1 tip run 35252684064 at `97b0d0f`, U17's tenth, on macOS luajit

`888e5e1` plus the sixth fix commit, the fallback row's macOS margin
(a test file, no `cfg` change). Pushed at 17:25:45Z; the pull request
synchronized and this run started within eleven seconds. Read on
2026-09-17 at fix round 1's close from the jobs endpoint and the two
macOS job logs; not re-run.

| field | value |
|---|---|
| run | 35252684064, `pull_request`, one attempt |
| head | `97b0d0f`, `e6d/typing-on-a-real-file` (base `250646f`; the run merges into `main` at `d06dc28`) |
| window | created 2026-09-17T17:25:49Z, updated 2026-09-17T17:46:07Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the failure | `Test (macos-latest / luajit)` (105308655905): **U17's tenth**, the job's only failure |

Tally (run-35252684064-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Lint (lua54) | 105308597518 | success |
| Changed paths | 105308597547 | success |
| Format | 105308597635 | success |
| Commit attribution (D9) | 105308597665 | success |
| Lint (luajit) | 105308597698 | success |
| M4 Perf Gates | 105308655756 | success |
| M1 Acceptance Gates | 105308655759 | success |
| GPU Render (headless) | 105308655780 | success |
| Test (ubuntu-latest / lua54) | 105308655856 | success |
| Test (macos-latest / luajit) | 105308655905 | failure |
| M6 Perf Gates | 105308655966 | success |
| Test (macos-latest / lua54) | 105308656020 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105308656027 | success |
| M5 Perf Gates | 105308656046 | success |
| Test (crdt) | 105308656053 | success |
| Test (ubuntu-latest / luajit) | 105308656117 | success |
| Perf budgets (debug) | 105308656187 | success |
| M10 Perf Gates (crdt) | 105308656302 | success |
| Docs consistency | 105308657266 | skipped |

The one failing target is `read_dir_supersede_cancels_in_flight_predecessor`
(`tests/m8_1_acceptance.rs:278:5`, `first read_dir must be
superseded; got ok`, `left: "ok"`, `right: "cancelled"`, `9 passed;
1 failed` in 2.61 s), U17's selector and required fragment exactly,
the job's only failure against 148 `test result: ok` --- **U17's
tenth**, its second on this pull request (the ninth was the review's
witness commit, above) and the third head of E6's line to carry it
(#270's seventh, #273's eighth). The `Test (macos-latest / lua54)`
leg (105308656020) is 151 `test result: ok`, zero `FAILED`, the
selector `ok` there; on both legs `WouldBlock` and `did not become
ready` appear zero times, so neither the `read Hello` family nor #259
sampled. The row the previous run reddened,
`e6d_cursor_confirm_acceptance`'s fallback witness, ran `7 passed` on
both legs with its new margin, and every other E6d suite and both
review suites are green on both (paired by their own `running N
tests` lines). The commit touches `tests/e6d_cursor_confirm_acceptance.rs`
alone, neither `src/dispatch` nor `tests/m8_1_acceptance.rs` --- an
argument from untouched files and not a measurement. **So the head is
not green, on U17 alone**, stated at the moment of writing; no rerun
taken, and a merge decision inherits U17's tenth as #270's and #273's
heads inherited its seventh and eighth.

### `main` after E6d: run 35265450358 at `25d2ce5`, and it is GREEN

Read at E7's opening on 2026-09-17, from the jobs endpoint and the
two macOS job logs; not re-run.

| field | value |
|---|---|
| run | 35265450358, `push`, one attempt |
| head | `25d2ce5`, E6d's squash merge (PR #275 at `97b0d0f`, `--match-head-commit`) |
| window | created 2026-09-17T19:32:06Z, updated 19:56:08Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |

Tally (run-35265450358-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 105351364176 | success |
| Format | 105351364413 | success |
| Lint (luajit) | 105351364530 | success |
| Lint (lua54) | 105351364640 | success |
| Commit attribution (D9) | 105351364666 | success |
| GPU Render (headless) | 105351428804 | success |
| M6 Perf Gates | 105351428835 | success |
| Perf budgets (debug) | 105351428861 | success |
| Test (crdt) | 105351428874 | success |
| M4 Perf Gates | 105351428892 | success |
| M1 Acceptance Gates | 105351428904 | success |
| M5 Perf Gates | 105351428943 | success |
| M10 Perf Gates (crdt) | 105351428956 | success |
| Test (ubuntu-latest / lua54) | 105351428989 | success |
| Test (ubuntu-latest / luajit) | 105351429018 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105351429042 | success |
| Test (macos-latest / luajit) | 105351429145 | success |
| Test (macos-latest / lua54) | 105351429151 | success |
| Docs consistency | 105351430435 | skipped |

On both macOS legs (`Test (macos-latest / luajit)` 105351429145,
`Test (macos-latest / lua54)` 105351429151) `WouldBlock` appears zero
times and `did not become ready` zero times, against 151 `test result:
ok` and zero `FAILED` in each, and U17's selector
`read_dir_supersede_cancels_in_flight_predecessor` ran `ok` on both.
So neither the `read Hello` family, nor #259, nor U4, nor U17, nor
#271, nor #276 sampled on this trunk run; every count stands where the
PR #275 sections left it, and under the rerun rule a green sample
retires nothing. This was the last code-bearing commit on `main` at
E7's opening; the U17 witness then landed on `main` as `7002308`, and
its run --- E7's base control --- is the next section.

### `main` at `7002308`: run 35270380496, red on three first-snapshot reads in `Test (ubuntu-latest / lua54)`, #253's third and #277

The U17 witness commit, landed on `main` directly at E7.0 and E7's
base control --- the last code-bearing commit on `main` under E7's
branch. Read at E7's opening on 2026-09-17, from the jobs endpoint,
the red job's log and both macOS logs; not re-run.

| field | value |
|---|---|
| run | 35270380496, `push`, one attempt |
| head | `7002308`, `main` (the U17 witness: `tests/m8_1_acceptance.rs`, `AsyncRuntime::pool()`, U17's closure below) |
| window | created 2026-09-17T20:22:09Z, updated 20:46:35Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the failure | `Test (ubuntu-latest / lua54)` (105367961691): three targets in three suites, each the first `BufferSnapshot` read after an attach timing out at 5 s --- **#253's third occurrence** (`gpu_invocation_acceptance.rs:487`) and **#277** (the two `read initial BufferSnapshot` reads) |

Tally (run-35270380496-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 105367893578 | success |
| Changed paths | 105367893802 | success |
| Lint (luajit) | 105367893865 | success |
| Lint (lua54) | 105367893971 | success |
| Format | 105367894926 | success |
| M1 Acceptance Gates | 105367961598 | success |
| GPU Render (headless) | 105367961622 | success |
| M10 Perf Gates (crdt) | 105367961644 | success |
| Test (crdt) | 105367961668 | success |
| Test (ubuntu-latest / lua54) | 105367961691 | failure |
| M4 Perf Gates | 105367961703 | success |
| Test (ubuntu-latest / luajit) | 105367961706 | success |
| M5 Perf Gates | 105367961709 | success |
| Perf budgets (debug) | 105367961716 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105367961746 | success |
| M6 Perf Gates | 105367961769 | success |
| Test (macos-latest / luajit) | 105367961773 | success |
| Test (macos-latest / lua54) | 105367961787 | success |
| Docs consistency | 105367963202 | skipped |

The three reds, in the order the job ran them:

Tally (run-35270380496-reds): 3 items in the list below.

- `compile_mode_crdt_acceptance` `compile_run_converges_and_replica_edit_triggers_recovery`, `read initial BufferSnapshot: Io(Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" })` at `tests/compile_mode_crdt_acceptance.rs:33`, `test result: FAILED. 7 passed; 1 failed` in 7.56 s at 20:35:12Z --- **#277**
- `e6_review1_gpu_route_probes` `gpu_route_plain_typing_undoes_through_the_daemon_arbiter`, the same fragment at `tests/e6_review1_gpu_route_probes.rs:35`, `5 passed; 1 failed` in 6.90 s at 20:35:36Z --- **#277**
- `gpu_initial_target_acceptance`'s `gpu_invocation_acceptance::crdt::dedup_upgrade_publishes_the_snapshot_to_preexisting_grid_replicas`, `target snapshot: Io(Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" })` at `tests/gpu_invocation_acceptance.rs:487`, `20 passed; 1 failed` in 17.94 s at 20:36:55Z --- **#253's third occurrence and its first on CI**, one row of the five its local occurrences had

`WouldBlock` appears exactly three times in the job, once per red,
against 145 `test result: ok`; `did not become ready` zero times. The
first two share one fragment and one route (both attach through
`tests/common/daemon.rs`'s `attach_multi`, which sets a 5 s read
timeout, then read the first `BufferSnapshot` in their own eight-line
helper), so they are one issue, #277, filed in the `intermittent-red`
shape with #253 named as the sibling mechanism. The third is #253's
own first fragment at its own site and is recorded there. Not #271:
that issue requires its `late.sock` and `m5_5` fragments as well, and
neither appears in this job. The tree's diff against `e91ed19` is the
U17 witness, an accessor on the async runtime and this file, none of
which a daemon's first snapshot reads --- an argument from untouched
files, not a measurement, and the previous code-bearing run
(35265450358 at `25d2ce5`) ran this leg green, which is one sample.
On both macOS legs (105367961773 luajit, 105367961787 lua54)
`WouldBlock` and `did not become ready` appear zero times against 151
`test result: ok` each, and U17's witness ran `ok` on both --- a green
sample of a row closed causally below, and nothing more. So neither
the `read Hello` family, nor #259, nor U4, nor #271, nor #276 sampled
here; #253 moves to three and #277 opens at one. **E7's base control
is this run, red on one ubuntu leg on rows the branch does not touch.**

### R7's seventeenth, local, on E5's tip

`scripts/gate` at `66195b5`, log `20260910T203556Z-1854124`, step
`05-sweep`: `attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`,
`transient sequence must attach: Attach(Handshake(Io(Os { code: 32,
kind: BrokenPipe, message: "Broken pipe" })))` at
`pmacs-gpu/src/attach.rs:1958` (the panic line has moved from `:1889`;
not part of the signature), `test result: FAILED. 365 passed; 1 failed`
on the `pmacs-gpu` unit target, under the default sweep's load. All
three of R7's fragments; `pmacs-gpu/src/attach.rs` has no diff on the
branch. The next run at the same tip (`20260910T204131Z-1922470`) was
six of six, non-reproduction and nothing more. **R7 is at least
seventeen.** The wait sweep (`docs/audits/2026-09-10-wait-sweep.md`,
S3) found that R7's 1 s deadline is armed after the spawn and consulted
only after two synthetic failures, while the `Hello` read and the
`server.join()` that wait carry no bound --- so the row's next
occurrence on a slower host can only be a hang and not a red.

### Run 34373256548, PR #262 at the head `7b6c519`

The run the branch could not record. Read from the jobs endpoint and
the failing job's log.

| field | value |
|---|---|
| run | 34373256548, `pull_request`, one attempt |
| head | `7b6c519` |
| window | started 2026-09-09T15:53:52Z, completed 16:11:45Z |
| verdict | 18 jobs: **16 success, 1 failure, 1 skipped** |
| the failure | `Test (macos-latest / luajit)`, job 102539566398 |
| failing target | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join`, `tests/statusline_segments_acceptance.rs:996:54`, `WouldBlock`, `test result: FAILED. 10 passed; 1 failed` |
| the skip | `Docs consistency`, correctly: the push changed code |

`WouldBlock` appears **exactly once** in the whole job log. This is
**#258's fourth occurrence** and the family's twelfth, and it is the
second consecutive red on this branch carrying that selector. The
previous run, 34369540895 at `e5417f6`, is recorded on the branch in
`7b6c519`.

`src/daemon.rs` has **no diff at all** in `dbe40a1..7b6c519`, so
nothing on the branch can reach a `Hello` read; the branch is a
candidate for none of this family and is excluded from none of it.

### Run 34394087819, PR #262 at its closing tip `ab117cb`

**The first record in this file written from `main` about a branch head
after that head's run had finished.** Under the status quo it could not
exist: recording it would have moved the tip and started the run that
replaced it. That is the whole of what the registry-location ruling
bought, and this section is the demonstration rather than the argument.

| field | value |
|---|---|
| run | 34394087819, `pull_request`, one attempt |
| head | `ab117cb`, C2's closing tip |
| window | started 2026-09-09T19:16:36Z, completed 19:33:38Z |
| verdict | 18 jobs: **17 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

**The branch's first fully green head.** Read from the job log rather
than the verdict line: in the whole **5,571-line** `Test (macos-latest
/ luajit)` job (102609387017), `WouldBlock` appears **zero** times,
there is no `test result: FAILED` against **123** `test result: ok`,
and **#258's own selector ran on the leg it fails on and passed** ---
`a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join ... ok`.

**By the rerun rule above this is non-reproduction and nothing more.**
It does not retire #258, does not show the tree innocent, and does not
lower any count: **#258 stays at four occurrences and the family at
twelve**, and both remain floors. A load-dependent failure is green
most of the time over a live defect, which is exactly why this section
records the green as a sample and not as a closure.

### The `read Hello` family is at SIXTEEN

Sixteen occurrences across **five suites** and **six selectors**,
counted by fetching the failing job of every run on PR #257, PR #262,
PR #265 and PR #267 and `main`'s post-merge runs, grepping
`WouldBlock`, and extracting the panicking thread and site of each hit.
Per-job counts: 1, 1, 4, 1, 2, 2, 1, 1, 1, 1, 1. (Twelve when this
section was first written, 2026-09-09; thirteen and fourteen added
2026-09-10 from E3's fix-round head and from `main` after E3's merge;
fifteen added 2026-09-10 from E4's fix-round head; **sixteen added
2026-09-10 from E5's head at `aa5177d`, the run that falsified half of
E5.0's reading of this family**, below.)

Tally (read-hello-per-job): 16 = 1 + 1 + 4 + 1 + 2 + 2 + 1 + 1 + 1 + 1 + 1.

Tally (read-hello-family): 16 rows in the table below.

| # | run | sha | suite | selector |
|---|---|---|---|---|
| 1 | 34220035122 | `8f6784f` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |
| 2 | 34222042303 | `e78d184` | `theme_faces_acceptance` | `v15_peer_never_receives_theme_facts_and_v16_does` |
| 3 | 34269795016 | `04263c6` | `gpu_font_acceptance` | `v16_peer_never_receives_font_facts_and_v17_does` |
| 4 | 34269795016 | `04263c6` | `m5_5_acceptance` | `m10_10_non_replica_frontend_does_not_receive_cursor_byte` |
| 5 | 34269795016 | `04263c6` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| 6 | 34269795016 | `04263c6` | `theme_faces_acceptance` | `v15_peer_never_receives_theme_facts_and_v16_does` |
| 7 | 34272480226 | `d7fd465` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |
| 8 | 34358682895 | `65648ec` | `m11_5_semantic_acceptance` | `daemon_routes_semantic_family_to_semantic_session_only` |
| 9 | 34358682895 | `65648ec` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| 10 | 34369540895 | `e5417f6` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |
| 11 | 34369540895 | `e5417f6` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| 12 | 34373256548 | `7b6c519` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |
| 13 | 34452014666 | `450ef26` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| 14 | 34483416251 (`main`) | `2ca2094` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| 15 | 34502504655 | `352d32f` | `theme_faces_acceptance` | `v15_peer_never_receives_theme_facts_and_v16_does` |
| 16 | 34524799346 | `aa5177d` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |

The six selectors are `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join`
(**five** since the sixteenth), `daemon_reships_the_summary_after_a_real_buffer_round_trip`
(**five**, the family's most frequent on its own since the fourteenth, tied since the sixteenth),
`v15_peer_never_receives_theme_facts_and_v16_does` (**three** since the fifteenth), and
`v16_peer_never_receives_font_facts_and_v17_does`,
`m10_10_non_replica_frontend_does_not_receive_cursor_byte` and
`daemon_routes_semantic_family_to_semantic_session_only` (one each).

Tally (read-hello-suites): 5 distinct values of `suite` in the table above.

Tally (read-hello-selectors): 6 distinct values of `selector` in the table above.

Tally (read-hello-a16_26): 5 rows of the table above with `selector` = `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join`.

Tally (read-hello-daemon_reships): 5 rows of the table above with `selector` = `daemon_reships_the_summary_after_a_real_buffer_round_trip`.

Tally (read-hello-v15_peer): 3 rows of the table above with `selector` = `v15_peer_never_receives_theme_facts_and_v16_does`.

Tally (read-hello-v16_peer): 1 row of the table above with `selector` = `v16_peer_never_receives_font_facts_and_v17_does`.

Tally (read-hello-m10_10): 1 row of the table above with `selector` = `m10_10_non_replica_frontend_does_not_receive_cursor_byte`.

Tally (read-hello-daemon_routes): 1 row of the table above with `selector` = `daemon_routes_semantic_family_to_semantic_session_only`.

Zero `WouldBlock` in all three failing macOS **lua54** jobs
(102040771394, 102047236847, 102154957233) and **zero at the merge
base**. The count is a floor: nobody has counted runs, so an
occurrence is recorded only when someone reads a log.

### #258 is at FIVE occurrences

Its own selector, job and all three required fragments, five times.

Tally (258): 5 items in the list below.

- run 34220035122 at `8f6784f`
- run 34272480226 at `d7fd465` (the occurrence D30 dispositioned)
- run 34369540895 at `e5417f6`
- run 34373256548 at `7b6c519`
- run 34524799346 at `aa5177d` (PR #269's opening head; the section on it below)

The last two are **consecutive runs on one
branch whose shas differ by 83 lines of markdown**, which excludes the
diff between them as the cause and establishes nothing whatever about
frequency. Two consecutive runs are two occurrences, not a rate.

D30's revocation condition is written for `main`, and runs 34369540895
and 34373256548 are `pull_request` runs on a branch, so neither meets
it. Whether a recurrence on the next phase's head should carry that
weight is the owner's.

### What PR #262's own branch commits say, and why it differs

`ee97842`, `ba3e737` and `7b6c519` carry run sections written on the
branch, each true when written and each one run behind for the
structural reason above. Two of their sentences are superseded by this
section: the family stated **at eleven** with an eleven-row table
(`7b6c519`, which recomputed it before run 34373256548 existed), and
**#258's third occurrence** as the highest count named there. Both
sentences stand where they were written, as this file's correction form
requires; this section is the count of record where they disagree with
it.

### The local gate red at `7b6c519`: seven async pump deadlines, filed as #263

Not a CI red. Gate log `20260909T154216Z-2666527`, step
`07-sweep-luajit`, target `-p pmacs --lib`: `running 2019 tests` ending
`FAILED. 2001 passed; 7 failed; 11 ignored ... finished in 19.41s`.
Seven `async_runtime::tests::` failures, filed as **#263** because the
signature matched no row here.

Tally (263): 7 items in the list below.

- `dispatch_sleep_completes_with_unit`
- `dispatch_sum_completes_with_correct_value`
- `dispatch_parse_round_trips_a_rust_source_file`
- `dispatch_cancel_1000_cycles_no_leak`
- `cancel_in_flight_sleep_yields_cancelled_outcome`
- `many_independent_jobs_complete_concurrently`
- `keyless_dispatch_is_unaffected_by_supersede`

**Required fragments**: (`runtime tick deadline exceeded` **or**
`keyless job stalled`) **and** `did not settle within 2s`, with the
`src/async_runtime.rs` line numbers excluded. The disjunction is
load-bearing: `keyless_dispatch_is_unaffected_by_supersede` panics at a
hand-rolled loop (`:2650`) carrying neither of the other fragments, so
a conjunction would drop a member. The bare `runtime tick deadline
exceeded` string is **older than `68a4a14`**, which only appended the
subject, elapsed time and poll count, and it appears in **exactly one**
retained gate-log directory — this red.

**They are one row, and the log's own ordering proves it.** The seven
are printed consecutively at log lines 58–64, each reporting
2.0000–2.0010 s, so they started and completed together at the head of
the run; forty-nine tests completed before the first was reported.
Inside that window the partition is exact with no counter-example on
either side: **every async_runtime test that waited on a real worker
reply failed (7 of 7), and every one that did not passed (5 of 5)** —
including `a_failed_or_cancelled_resource_job_is_not_harvested`, which
injects its replies with `rt.workers.send`, and
`take_result_while_running_returns_none`, which dispatches a 500 ms
sleep but asserts a disjunction that never waits. The discriminator is
waiting, not dispatching.

**What they share is one process and one ~2-second window — NOT one
runtime.** Each of the seven builds its own `AsyncRuntime`
(`with_pool_size(1|2|4)`, `src/async_runtime.rs:840`) and its own
`WorkerPool` with its own threads, injector, stealers and parker
condvar (`src/worker.rs:196`); neither type holds a static. The pool
sizes across the seven are 1, 1, 1, 2, 2, 2 and 4, and all seven failed
alike. **No object in the tree explains the co-failure, and that
absence is the finding** — it is what separates this row from U16,
which was widened on a *demonstrated* process-global object
(`set_current_dir`, twice in the workspace, both in one test) plus a
3-of-400-against-0-of-400 experiment. There is no such object here and
no such experiment. So the mechanism question is not "why did the pool
stall", there being no *the* pool, but "what made seven independent
worker pools all fail to run inside one two-second window while their
main threads ran at full rate".

**Not U17.** U17's `pump_until` *succeeded* and the **value** was wrong
(`first read_dir must be superseded; got ok`,
`tests/m8_1_acceptance.rs:278`): its predecessor ran too *fast* for the
cancel to land. Every #263 failure is the opposite sign — nothing
settled at all and the deadline fired. U17 must not be widened onto
this row.

**Load is BOUNDED, not established, and the proxy is already in every
stage log.** Forty-nine tests completed before the first of the seven
was reported at t = 2.000 s, so roughly 56 completions in the first two
seconds against 2008 in 19.41 s: **the failure window ran about 3.7×
slower than the rest of its own run.** Across runs, the same
`-p pmacs --lib` luajit target on the same tree, machine and day took
**19.41 s** (this red), **15.23 s** (green) and **5.16 s** (green), and
the default-feature `--lib` target across that day's eleven gate logs
ranged **5.26 s to 20.24 s, every one green**. The red sits at the slow
end of an envelope green runs already occupy. **This diagnoses
nothing.** What it establishes is that the per-target `finished in`
line answers "was this run slow?" without a `/proc/loadavg` sample, so
no new instrument is owed.

**Retirement**: diagnosis, never a green rerun. A later
`scripts/gate --protocol` at `7b6c519` did not reproduce the seven
(log `20260909T162011Z-2832095`, eight of eight); by the rerun rule
above that is non-reproduction and nothing more, and it retires
nothing. The discriminating instrument is whether the **worker**
threads were runnable at the deadline, which the pump does not report.

**One thing for the owner before this is called a product defect.**
`pump_until`'s `DEADLINE` is a fixed 2 s wall-clock assertion running
in the **default** sweep, in a module whose three genuine wall-clock
budgets (`dispatch_parse_stays_under_the_parse_budget`,
`grep_supersede_cancels_predecessor_within_50ms`,
`supersede_cancels_in_flight_job_within_50ms`) are `#[ignore]`d and
print `ignored, wall-clock budget` in this same log. Against a measured
3.8× dispersion on one machine on one day, that is D12's budget rule
applied unevenly.

## Live rows

### R3 — live-leader EPERM with an unobservable group

| field | value |
|---|---|
| selector | `--lib process::tests::a_successful_signal_disposition_depends_on_whether_it_is_fatal` |
| job | macOS / lua54 |
| required fragments | `EPERM` and `measured_group=unobservable(` and (`ESRCH` / `No such process`) and `leader=live` |
| occurrences | one, PR #214 run 30932558752 attempt 1 |
| candidate mechanism | a group-directed `kill` returned EPERM while the leader was observed live, and `measured_group`, the one field able to disagree, could not be read. Unresolved; possible product defect |
| retirement | diagnosis and disposition by whoever next touches process signalling or the reap ledger. Never a green rerun |

Same test as the retired R2 (`leader=exited(signal SIGUSR1)`, a test
race fixed by a readiness gate and an `exec`); only the fragment
separates them. R2's fixture change touched no product code, so a change
in how often this row appears is evidence about frequency, not cause.

### R5 — an async pump deadline in the supersede close path

**Reopened 2026-09-08 as NEVER CLOSED.** Not as a recurrence: the
closure was void when it was written. R5 was closed 2026-09-05 with
"readiness waits migrated to `tests/common/ready.rs`, which reports
elapsed and last-observed state; a recurrence is an issue". This row's
failing site is `pump_async`, a private helper in `src/editor.rs`'s own
`#[cfg(test)]` module. `tests/common/ready.rs` is compiled into the
integration targets and an in-crate unit test cannot use it, so that
migration never reached this site and could not have. The row was never
retired by anything; it was only stopped being looked at. The one thing
the closure asserted that was true is that a wait should report what it
waited on, which is now done here by hand.

| field | value |
|---|---|
| selector | `--lib editor::tests::stream_supersede_delivers_cancelled_to_on_close` |
| job | GitHub Actions, macOS / lua54 |
| required fragments | `async pump deadline exceeded` |
| occurrences | two: `main`, run 30555667095, 2026-07-30; and PR #257 at `e78d184`, run 34222042303, job `Test (macos-latest / lua54)` (102047236847), `src/editor.rs:12842`, `test result: FAILED. 2196 passed; 1 failed; 11 ignored`. The panic line moves with `editor.rs` and is not part of the signature. The next run, 34253949749 at `b2094ac`, ran this selector on both macOS legs and passed: non-reproduction and nothing more |
| candidate mechanism | the test drives a superseded stream to its `on_close` and pumps `tick_async` until the Lua marker appears, under a fixed 2-second deadline the helper sets itself. Whether two seconds is short for a loaded macOS runner or the close notification is genuinely lost is not known, and the first occurrence's log no longer says anything either way. Unresolved; possible product defect |
| retirement | diagnosis. The deadline now reports its subject, its elapsed time and its poll count, so the next occurrence says whether it missed by a millisecond or by two seconds --- that is a step toward the diagnosis and is not itself a closer. Never a green rerun. **THE DISCRIMINATOR, and it is one number that already exists.** R5 shares its waiting shape, its `--lib` binary and the very ancestry of its reporting with #263 (`68a4a14` carried R5's reporting inward to `pump_until`), and the one measurement that separates *the pump was starved* from *the reply never came* is the poll count --- which **R5 has never been observed with**, both occurrences above predating that commit. So the next occurrence decides it, and nothing else has to be built: **~2000 polls in 2 s** means the pump ran at full rate and nothing settled, which excludes starvation, is the same window #263 saw, and merges the two rows under a second selector; **a small count** means the pump itself was starved, which is a different mechanism, and #263 is not R5. Recorded at C2's close so the next reader of this row does not re-derive it. **Since E5.0 (`efa762a` on PR #269) the deadline at this site is 10 s, not 2**, under D12, and the discriminator reads as a rate: the message still prints elapsed and polls, so about a thousand polls per second of elapsed means the pump ran and nothing settled, and a low rate means the pump was starved. The sweep also found the test's own backlog at cancel is unbounded (`emit_n(1_000_000)`, one envelope per item, drained by one `tick`), so a bigger deadline is not the fix until that backlog is measured on the macOS runner (the audit's S5 and S8) |

**The lesson the void closure carries.** A closer that names a mechanism
has to be checked against the failing site, not against the row's
description. R6 and U8 were closed on the same migration in the same
sitting; both of those do fail inside integration targets, so the
migration does reach them, and U8's 2026-09-08 recurrence carried
fragments precisely because it had. R5 was swept along with them on a
resemblance.

### R7 — managed-retry attach hits a broken pipe under sweep load

| field | value |
|---|---|
| selector | `-p pmacs-gpu attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream` |
| job | local (Linux), inside a workspace sweep; never seen in isolation or in CI |
| required fragments | `transient sequence must attach` + `Handshake(Io(` + `BrokenPipe` (or `code: 32`) |
| occurrences | at least eighteen, 2026-08-07 to 2026-09-17, all local, all under sweep load; the panic line moves with `attach.rs` and is not part of the signature. The first twelve are enumerated in this file's history before 2026-09-05; the six since are the list below this table, with the tallies (added at fix round 1, review 1's Low 3). The count is a floor: nobody has counted runs, so an occurrence is only ever recorded when someone reads the log |
| candidate mechanism | the test drives a scripted transient-then-success sequence over a real socket pair; unknown whether the broken pipe is the fixture's writer closing early or a retry-path defect. Unresolved |
| retirement | hardening that removes the named mechanism plus a discriminating witness, or a diagnosis showing the fixture, not the code, closes the pipe |

Tally (R7): 18 = 12 + 6.

Tally (R7-held): 6 items in the list below.

- thirteenth: gate log `20260905T202734Z-1751532`, step `07-sweep`, load average 14.2, `attach.rs:1889`, all three fragments
- fourteenth: gate log `20260905T205642Z-2051072`, step `05-sweep` of the six-stage gate, `attach.rs:1889`, all three fragments
- fifteenth: gate log `20260907T170429Z-45241`, step `06-sweep-luajit` (the LuaJIT-only sweep `--protocol` adds), `test result: FAILED. 325 passed; 1 failed` on `-p pmacs-gpu --bin pmacs-gpu`, `attach.rs:1889`
- sixteenth: gate log `20260907T185321Z-604527`, step `06-sweep-luajit`, the same result line, `attach.rs:1889`
- seventeenth: gate log `20260910T203556Z-1854124`, step `05-sweep` of E5's tip gate, `attach.rs:1958`, `test result: FAILED. 365 passed; 1 failed` (its own section above)
- eighteenth: gate log `20260917T001151Z-3116741`, step `05-sweep` on E6d's branch, `attach.rs:1971`, `test result: FAILED. 365 passed; 1 failed` (its own section above)

What the occurrences establish: the tree is excluded twice over (two
consecutive gate runs on one worktree differing by one markdown file,
green then red; several occurrences on documentation-only commits), and
green reruns number in the dozens; both sweep flavors of the
`--protocol` plan have produced it, so the Lua flavor is not the
discriminator either. What they do not establish: a mechanism, or a
rate, since nobody has counted runs and failures over a fixed window.
The remaining candidates have to be varied inside a gate run, one per
run.

### U4 — a PTY resize with no observed blank, macOS both flavors

**Reopened 2026-09-08 as NEVER CLOSED**, by the audit below and not by
a recurrence. U4 was closed 2026-09-05 with "not reproduced since
2026-08-15; a recurrence is an issue". That is a count of green runs,
which the rerun rule above forbids as a closer in the same sentence it
is written in. No mechanism of U4's was removed and none was explained
--- the row's own `what is NOT` cell said so when it was filed --- so
nothing retired it and the closure was void when written.

| field | value |
|---|---|
| selector | `--test full_grid_resync_acceptance a_pty_resize_blanks_the_host_before_repainting` |
| job | GitHub Actions, `Test (macos-latest / lua54)` **and** `Test (macos-latest / luajit)`. Flavor is NOT a matching key for this row: it was filed from one lua54 red and then reddened twice on luajit with identical fragments |
| required fragments | `FG-INV: the post-resize resync must blank the host` + `no CSI 2 J appeared in the` + `bytes emitted after the first painted frame` |
| occurrences | four: three before the void closure --- PR #229 (lua54) and PR #231 attempts 1 and 2 (luajit), 2026-08-15 and earlier --- and the first since the row was reopened, PR #270 at `e804b97`, run 34774221987, job `Test (macos-latest / lua54)` (103769261471), `tests/full_grid_resync_acceptance.rs:126:13`, all three fragments (`no CSI 2 J appeared in the 30668 bytes emitted after the first painted frame`), `5 passed; 1 failed` in 20.29 s, the job's only failure against 133 `test result: ok`, recorded 2026-09-13 at E6's close and not rerun; the branch's diff touches no PTY, resize or resync code, which is an argument from untouched files and not a measurement. Full evidence in this file's history before 2026-09-05, including #231's five-observation base control and why neither branch diff excludes itself. The Linux observation of 2026-09-07 is #255 and is NOT an occurrence of this row --- the job does not match; see the section below |
| candidate mechanism | none. No blank was OBSERVED after the mark within the test's fixed 20 s deadline; whether it was never emitted, emitted late, or lost in transport is open, and the deliberate reintroduction of the genuine resync defect produces signature-indistinguishable output, so the fragments cannot discriminate a fixture race from a product regression |
| retirement | producer-side emission evidence cross-checked against the collected stream. A longer deadline concludes in one direction only: if the clear arrives, "emitted late" is established; if it does not, that is "not observed by the longer deadline" and nothing more. Never a green rerun |

### U5 — Ctrl-C during a reconnect sleep kills the process

**Reopened 2026-09-08 as NEVER CLOSED**, and separately it has now
recurred. U5 was closed 2026-09-05 with "one occurrence, not
reproduced; a recurrence is an issue" --- a count of green runs, void
when written for the same reason as U4's. The recurrence is real and
postdates the closure, but it is not what reopens the row: the row was
never shut.

| field | value |
|---|---|
| selector | `--test m5_8_acceptance ctrl_c_during_reconnect_sleep_yields_clean_exit` |
| job | GitHub Actions, `Test (macos-latest / lua54)`. The `:LINE` suffix is not a fragment |
| required fragments | `Ctrl-C during reconnect sleep should produce a clean exit` + `ExitStatus { code: 1, signal: Some("Interrupt: 2") }` |
| occurrences | two: PR #229's rerun attempt 2, recorded 2026-08-09 in `ae6a815` --- the run's own date is not in the record and is not asserted here; and PR #257 at `b2094ac`, run 34253949749, job `Test (macos-latest / lua54)` (102154957233), `tests/m5_8_acceptance.rs:546`, `test result: FAILED. 7 passed; 1 failed`. The second is filed as **#260**. The `Test (macos-latest / luajit)` leg of that same run passed this selector, so one run establishes nothing about the flavor either way |
| candidate mechanism | Ctrl-C reached the process as `SIGINT` rather than as the raw-mode key event the test drives, and the process died of the signal instead of exiting cleanly. That is all the exit status shows. Whether the injection preceded raw mode, whether raw mode was lost, or whether the reconnect sleep's handler was not yet installed, are three mechanisms this fragment separates not at all |
| retirement | diagnosis: a readiness record written by the child as it enters raw mode, so the injection is ordered against setup rather than raced against it. Never a green rerun |

### U19 — a terminal bell not observed within a 5 s poll

**Reopened 2026-09-09 as NEVER CLOSED**, on R5's ground and not on a
recurrence. U19 was closed 2026-09-05 with "readiness migration; the
wait reports the last frame seen". Its selector is an in-crate unit
test, and `tests/common/ready.rs` compiles into the integration targets
only, so the migration could not reach the site; and the site is
unchanged --- `src/daemon.rs:5294-5298` is the loop the row was filed
on, untouched since `dc92257`, asserting `Instant::now() < deadline`
with the message `initial terminal bell timed out` and reporting no
elapsed time, no poll count and no last-observed value. The closer
names a report that does not exist.

| field | value |
|---|---|
| selector | `--lib daemon::tests::terminal_bell_baseline_suppresses_history_and_delivers_each_new_bell_once` |
| job | local (Linux), the workspace sweep of `scripts/gate` (step `07-sweep` of the eight-stage gate of the time) |
| required fragments | `initial terminal bell timed out` |
| occurrences | one: 2026-08-31, gate log `20260831T174104Z-3438184`, `src/daemon.rs:5296`, `1988 passed; 2 failed` --- the same run as U16's second occurrence, recorded separately because the selectors and fragments differ. It passed in all three stages of the next gate run, at `ea786a2` (log `20260831T174716Z-3535694`): non-reproduction and nothing more |
| candidate mechanism | a 5-second poll over `bell_count(buffer_id) != Some(1)` with `tick_processes()` and a 10 ms sleep per turn. Whether the bell never arrived or arrived late the loop cannot tell apart, and the panic carries no elapsed value, so this occurrence's margin is unrecoverable. Unresolved |
| retirement | diagnosis. The loop reporting its elapsed time, poll count and last-observed count would be a step toward it and is not a closer --- R5's lesson and U8's. Never a green rerun |

## Closed rows

Each row's full evidence is in this file's history before 2026-09-05.
One line per row: what it was, when it left this section's live half,
and on what grounds.

**The grounds are not all the same kind, and the word says which.** A
row with a bare date is a causal closure: its mechanism was removed or
explained, which is the only retirement the rerun rule allows.

- **VOID** --- the closure was not a closure when it was written. Either
  it named no mechanism at all and rested on a count of green runs
  (U4, U5), or it named one that never reached the failing site (R5,
  U19).
  A void row was never retired; it was only stopped being looked at,
  and it is live again above.
- **INCOMPLETE** --- the closure named a real mechanism that did reach
  the failing site, but that mechanism did not remove the cause. U8 is
  one, falsified rather than suspected; R6 is the other, ruled by the
  form of its closer, and its case is worse: the closer made the row
  unmatchable at its own site.
- **DISCARDED** --- there was never evidence to retire. The fragments
  were lost or never captured, so the row could not match anything.
  This is an admission about the record, not a finding about the code,
  and it must never be counted as a causal closure.
- **MERGED** --- the question moved to another row and is live there.
- **REFERRED** --- not a test, so outside this file; the question lives
  in a GitHub issue.

Twenty-nine rows are listed on twenty-eight lines (A1 and A2 share
one). Eighteen lines are causal closures and stand: a wall-clock
assertion made `#[ignore]`, a duplicated test execution removed by the
one-sweep gate, a fixture race fixed with a readiness gate, a
hermeticity fault fixed, U16's process-global cwd mutation deleted,
and U17's load-dependent predecessor held by the fixture. The ten that
are not now say which word they are. **Recount these
against the table below rather than trusting them**: they were written
as twenty-seven, twenty-six, eighteen and eight one commit before U16
closed and moved into it, then as nineteen and eight while R6 and U19
stood "on notice" with bare dates, and each time were wrong until this
line was rewritten by counting again.

| row | what it was | disposition | grounds |
|---|---|---|---|
| R1 | `supersede_cancels_in_flight_job_within_50ms` missed its 50 ms budget | 2026-09-05 | the assertion is a wall-clock budget; it is `#[ignore]` and runs in the perf jobs and `scripts/gate --perf` |
| R2 | `SIGUSR1` delivered before the trap was installed | 2026-08-05 | test race fixed with a readiness gate and an `exec` |
| R4 | readiness predicate satisfied by an empty file | 2026-08-05 | `wait_for_file` waits for the expected bytes, with three witness tests |
| R5 | `async pump deadline exceeded` in the supersede close path, macOS | 2026-09-05, **VOID** | the closure named the `tests/common/ready.rs` migration, which cannot reach an in-crate unit test and so never applied to the failing site. R5 is live again above, as never-closed |
| R6 | readiness file never published in the panel terminal fixture, macOS | 2026-09-05, **INCOMPLETE** | U8's migration and U8's class, on the same test: it reached the site (`tests/bottom_panel_stage1_acceptance.rs:2446` is `ready::expect`) and changed what the wait reports, not what it waits for, and that test has failed three times since as #259. Worse, the migration replaced `timed out waiting for`, one of this row's two required fragments, so the row cannot match at its own site and its "not recurred" was earned by construction. Ruled at C1's close by the form of the closer; see the note below |
| R8 | LSP listview row rendered relative to a stray ancestor marker | 2026-08-08 | test hermeticity fixed |
| A1, A2 | historical claims with no linked occurrence | 2026-09-05, **DISCARDED** | nothing was ever measured, so there was nothing to retire. Discarded for want of evidence; not a causal closure and not a claim about the code |
| U1 | an unclassifiable local red, fragments not captured | 2026-09-05, **DISCARDED** | the fragments were destroyed by a rerun before anyone read them, so the row could never match anything. Discarded for want of evidence; not a causal closure |
| U2 | `m6_1_pty_raw_mode_disables_kernel_echo`, `stty -a` output empty | 2026-09-05 | readiness migration; the PTY read now waits for the record it asserts on |
| U3 | the R7 selector with fragments lost | 2026-09-05, **MERGED** | folded into R7's occurrence count. Not a retirement at all: the question is live, in R7 |
| U4 | `a_pty_resize_blanks_the_host_before_repainting`, macOS, three occurrences | 2026-09-05, **VOID** | the closer was "not reproduced since 2026-08-15", a count of green runs, which the rerun rule forbids. No mechanism was removed or explained. U4 is live again above, as never-closed; the Linux observation #255 postdates the closure and is discussed below |
| U5 | `ctrl_c_during_reconnect_sleep_yields_clean_exit`, macOS lua54 | 2026-09-05, **VOID** | the closer was "one occurrence, not reproduced", a count of green runs. U5 is live again above, as never-closed, and has since recurred at `b2094ac`: **#260** |
| U6 | `criterion_1` and `composition_overhead` red together | 2026-09-05 | both are wall-clock budgets, now `#[ignore]` |
| U7 | a different render-budget test red each sweep (dired, outline) | 2026-09-05 | render budgets, now `#[ignore]` |
| U8 | `acc28_child_input…`, macOS luajit, fragments destroyed | 2026-09-05, **INCOMPLETE** | the readiness migration (the R6 family) is a real mechanism and it DID reach this site --- which is why the recurrence has fragments at all --- but it changed what the wait reports, not what the wait waits for, so it never removed a cause. Falsified by #259 on 2026-09-08 (macOS lua54) and again in run 34222042303 (job 102047236847), a comment on #259. See the note below |
| U9 | a PTY test and a budget test red together in one sweep | 2026-09-05 | the budget half is `#[ignore]`; the PTY half is U2's mechanism |
| U10 | the budget red rotating between two runs of one commit | 2026-09-05 | budgets `#[ignore]` |
| U11 | `dispatch_parse_round_trips_a_rust_source_file` missed its parse budget, macOS | 2026-09-05 | the round trip stays in the default run; the 100 ms budget is a separate `#[ignore]` test |
| U12 | a budget test and a PTY test together in `lib-crdt` | 2026-09-05 | budgets `#[ignore]`; the gate no longer has a `lib-crdt` stage |
| U13 | `skipped_directories_are_reported_with_a_reason` received empty child stdout inside the sweep | 2026-09-05 | the gate runs each test once, in one sweep; a recurrence is an issue |
| U14 | four selectors red in one gate run across three stages | 2026-09-05 | three are budgets, now `#[ignore]`; the fourth (`state: initializing`) is a readiness wait, migrated |
| U15 | a rotated multi-red cluster with a load reading | 2026-09-05 | all budgets, now `#[ignore]` |
| U16 | a `git` child inheriting a working directory another test deleted | 2026-09-08 | the process-global `set_current_dir` was removed: `bare_filename_saves_in_cwd` now makes the cwd move in a subprocess (`ee28bf8`), and `set_current_dir` no longer occurs anywhere in the workspace. Causal, and demonstrated in both directions --- see below |
| U17 | `read_dir_supersede_cancels_in_flight_predecessor`, the predecessor complete before the cancel took effect, ten occurrences across three CI legs and one local sweep | 2026-09-17 | the witness holds every worker thread behind a channel it releases, so the predecessor is in flight by construction when the supersede flips its token, and the bitten product (the cancel removed from `allocate`) fails it 30 of 30. Causal, and demonstrated in both directions --- see below |
| U18 | a Go checksum-database fetch failed before anything was built | 2026-09-05, **REFERRED** | not a test and so out of this file's scope; the question lives in issue #249 |
| U19 | a terminal bell not observed within a 5 s poll | 2026-09-05, **VOID** | the closer named the `tests/common/ready.rs` migration, which cannot reach an in-crate unit test --- R5's ground exactly --- and the loop at `src/daemon.rs:5294-5298` is unchanged since `dc92257` and reports nothing. Ruled at C1's close. U19 is live again above, as never-closed |
| U20 | `composition_overhead` red alone | 2026-09-05 | a budget, now `#[ignore]` |
| U21 | `m6_1_pty_canonical_mode_keeps_kernel_echo` red alone in `lib` | 2026-09-05 | U2's mechanism; the gate runs each test once |

### U16's mechanism, demonstrated and then removed

U16 is the one row here retired on a demonstration rather than an
argument, so the demonstration stays where the closer can be checked
against it.

**The chain, link by link.** `bare_filename_saves_in_cwd`
(`src/file_io.rs`) called `std::env::set_current_dir` onto a `TempDir`
and back --- the only two occurrences of that call in the entire
workspace. libtest runs a binary's tests on a thread pool, so
"concurrently" was the default and not a contrivance.
`packages::fetcher`'s `run_git` calls `run_git_inner(None, …)`, and
that function sets `cmd.current_dir(d)` only for `Some(d)`, so with
`None` the `git` child inherits the process cwd --- production code,
not fixture code. The parent's restore does nothing for a child already
running, the `TempDir` then drops underneath a live `git`, and `git`
reports `Unable to read current working directory`, which is this row's
first required fragment. That fragment was reproduced standalone, in a
scratch directory with no pmacs code involved.

**The experiment the row's own retirement cell asked for**, run against
the gate's lib test binary, 400 iterations per arm, each arm with its
own `TMPDIR`:

| tree | arm | filters | iterations | failing |
|---|---|---|---|---|
| `b2094ac` | treatment | `packages::fetcher::tests` **plus** `file_io::tests::bare_filename_saves_in_cwd` (19 tests) | 400 | **3** |
| `b2094ac` | control | `packages::fetcher::tests` alone (18 tests) | 400 | 0 |
| `ee28bf8` | treatment | the same 19 tests | 400 | **0** |
| `ee28bf8` | control | the same 18 tests | 400 | 0 |

Adding one test to the process took the failure rate from 0/400 to
3/400, and every failing iteration failed **eight** fetcher selectors at
once, each carrying both required fragments --- the multi-selector shape
the row said its mechanism predicts. That is what makes the first two
rows a demonstration and not a citation.

**What closes the row is the removal, not the last two rows of that
table.** `ee28bf8` takes the cwd move into a subprocess:
`Command::current_dir` is per-child, so nothing repoints this process,
and `set_current_dir` now appears nowhere in the workspace. The
mechanism is gone by inspection. The 0/400 after the fix is
corroboration and could not be a closer --- it is a count of green
runs, which the rerun rule forbids, and 400 iterations of a 3/400
signature would come up empty about five times in a thousand by chance
alone.

**What is not closed.** `run_git` still depends on the ambient cwd, and
a user whose working directory is removed under a package fetch would
get exactly this error. Nothing has measured whether that is reachable,
and this row never claimed it: U16 was always about a test mutating the
process cwd, and it is that which has been removed. The product
question is separate, unmeasured, and not filed --- naming it here is
not a claim that it exists.

**Two pushed commit messages carry the superseded day-count.**
`d0771fc` ("Four occurrences on 2026-09-08") and `b2094ac` ("Five
occurrences on 2026-09-08") state a number the list below did not
support when either was written. They are pushed and are not rewritten;
this is the correction, in the same form as `e78d184`'s below.

**The occurrences it closes on**, kept because a closed row's count is
the evidence the closure had to answer for: **nine**, and the number is
the length of this list, recomputed here rather than incremented:
**(1--3)** three on 2026-08-31 within eleven hours, the third on `main`
after a documentation-only merge; **(4)** gate log
`20260908T101700Z-2279879` and **(5)** `20260908T105335Z-2697918`, local
sweeps within an hour, one selector each; **(6)**
`20260908T110408Z-2820743`, FIVE selectors together ---
`cache_survives_across_fetcher_instances`,
`fetch_after_upstream_tag_removed_surfaces_at_resolve`,
`fetch_clones_into_cache`, `fetch_twice_does_not_reclone_via_sentinel`,
`resolve_branch_returns_branch_head` --- all carrying both fragments;
**(7)** `20260908T154623Z-3116795`, review round 1's six-stage gate at
the branch head `e78d184`, step `05-sweep`:
`cache_survives_across_fetcher_instances` alone at
`src/packages/fetcher.rs:929`, `test result: FAILED. 2199 passed; 1
failed; 11 ignored` --- the first at a branch head and the first the
widened selector caught; **(8)** `20260908T163205Z-3389418`, fix round
1's gate at `68a9767`, step `05-sweep`: TWO selectors,
`cache_survives_across_fetcher_instances` at `:929` and
`fetch_after_upstream_tag_removed_surfaces_at_resolve` at `:1021`, `test
result: FAILED. 2198 passed; 2 failed; 11 ignored`, with step
`06-sweep-luajit` of that run clean at 124 targets 4271/0/35; **(9)**
`20260908T164713Z-3615153`, the very next run at `d0771fc`, step
`06-sweep-luajit`: THREE selectors, the eighth's two plus
`fetch_clones_into_cache` at `:889`, `test result: FAILED. 2004 passed;
3 failed; 10 ignored`, while step `05-sweep` of that same run is CLEAN
at 124 targets 4571/0/49. **Six of the nine are on 2026-09-08**, items 4
through 9 --- counted, not carried: this field said "four" when the list
gave five and "five" when it gave six, having been corrected by
incrementing twice. Every one is on a tree touching neither `packages`
nor `file_io`. The eighth and ninth are mirror images one run apart, so
the sweep flavor is not the discriminator: the row is about the
workspace sweep, either flavor. Several intervening sweeps of the same
tree were green, so the rate on this machine is neither zero nor one.
The count is a floor for the reason R7's is --- an occurrence is
recorded only when someone reads a log --- and the three reproductions
in the retirement experiment below are deliberate and are NOT counted
here

### U17's mechanism, held and then removed

U17 is the second row retired on a demonstration rather than an
argument (U16 is the first), and for the same reason the demonstration
stays where the closer can be checked against it. The row as it stood
live, with the ten occurrences it closed on:


| field | value |
|---|---|
| selector | `--test m8_1_acceptance read_dir_supersede_cancels_in_flight_predecessor` |
| job | GitHub Actions: first the serialized crdt sweep (`--test-threads=1`), since then `Test (macos-latest / luajit)` seven times and `Test (ubuntu-latest / luajit)` once, all under cargo's default parallelism, and once the local gate's non-CRDT sweep. The job is not a discriminator for this row and a match needs the selector and the fragment on any CI test leg |
| required fragments | `first read_dir must be superseded; got ok` |
| occurrences | ten: `main` at `aae5b35`, run 33375945966 (the serialized crdt sweep); `main` at `d97e137`, run 34205653191, job `Test (macos-latest / luajit)`; PR #257 at `8f6784f`, run 34220035122, job `Test (ubuntu-latest / luajit)`; `main` at `dbe40a1`, run 34349759554 (E1's post-merge run), job `Test (macos-latest / luajit)` (102459915513), `tests/m8_1_acceptance.rs:278:5`, `assertion left == right failed: first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed`, the job's only failure against 119 `test result: ok`; and PR #269 review 2's gate at `95a6db4`, log `20260911T102129Z-2474066`, step `sweep-luajit` (the non-CRDT sweep under `--no-default-features --features luajit`), `tests/m8_1_acceptance.rs:278`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 0.33 s, the step's only failure, not rerun; and PR #270 at `e82fcb5`, run 34772792926, job `Test (macos-latest / luajit)` (103765362433), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.34 s, recorded at E6's close on 2026-09-13 and not rerun (the job's other red is the branch's own fixture defect, below); and PR #270 at `e804b97`, run 34774221987, the very next head, job `Test (macos-latest / luajit)` (103769261492), `tests/m8_1_acceptance.rs:278:5`, the same fragment, `9 passed; 1 failed` in 1.93 s, the job's only failure against 133 `test result: ok` --- the same fragments on the next run are a second occurrence, not a coincidence, and this row now has two on consecutive heads of one PR whose diff touches neither `src/dispatch` nor `tests/m8_1_acceptance.rs`, an argument from untouched files and not a measurement; and PR #273 at `c4be8aa`, run 35031658924, job `Test (macos-latest / luajit)` (104591399407), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.26 s, the job's only failure against 140 `test result: ok`, recorded at E6c's close on 2026-09-16 and not rerun; the branch's diff touches neither `src/dispatch` nor `tests/m8_1_acceptance.rs`, and its base control, `main`'s run at `7880c4b`, ran this leg green, which is one sample; and PR #275 at `b1ef903`, run 35221439579, job `Test (macos-latest / luajit)` (105202355414), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.49 s, the job's only failure against 148 `test result: ok`, recorded at E6d's fix round 1 on 2026-09-17 and not rerun; the commit is the review's witness commit and adds two files under `tests/`, touching neither `src/dispatch` nor `tests/m8_1_acceptance.rs`, and the selector ran `ok` on both macOS legs of the previous head `085ae4d`'s run, green samples; and PR #275 at `97b0d0f`, run 35252684064, job `Test (macos-latest / luajit)` (105308655905), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.61 s, the job's only failure against 148 `test result: ok`, recorded at E6d's fix round 1 on 2026-09-17 and not rerun; the commit touches one test file of the branch's own, and the selector ran `ok` on both macOS legs of the head before it, `888e5e1`, green samples. The second, third and fourth run at cargo's DEFAULT parallelism under D23, and the third is on LINUX, so neither serialization nor macOS is required to produce it. The second is the merge-base control for the third: the same signature is on `d97e137` itself, so the branch did not introduce it. That control is ONE run at the base, which is one sample: it establishes that the signature exists on `d97e137`, not its rate. PR #257's second run, 34222042303 at `e78d184`, is GREEN on `Test (ubuntu-latest / luajit)`, and so are its third, fourth and fifth. Green runs are samples: non-reproduction and nothing more. The fourth occurrence is on `main` after two merges that did not touch `src/dispatch` or `tests/m8_1_acceptance.rs`, which is an argument from untouched files and not a measurement; it is the row's third occurrence on `main` and the second on this leg. The fifth is the gate's non-CRDT sweep on the reviewed head, not CI, and it establishes neither cause nor the branch's innocence; it is recorded here as an occurrence because the selector and the required fragment match exactly |
| candidate mechanism | the predecessor completed before the cancellation took effect. `--test-threads=1` was the first occurrence's candidate: it serializes the test functions in one executable and so removes one source of contention the test's "in flight" depends on. The second occurrence has no such flag, which does not refute the mechanism --- a fast predecessor is a fast predecessor however the runner got there --- but it does mean serialization is not required to produce it, and the remaining common factor is a macOS or Linux CI runner rather than a scheduling flag. Nothing has measured the predecessor's duration under either, and nothing rules out a real supersede defect |
| closure | **2026-09-17, causal.** The witness holds the predecessor in flight deterministically: every worker thread of the editor's pool is occupied by a hold that reports it is running and then blocks on a channel the test owns, the two `read_dir`s are dispatched only after every hold has reported, so the first sits in the queue unable to complete while the second's `allocate` flips its token, and only then are the holds released. `read_dir_blocking` polls the token before its first entry, so the released predecessor returns `Cancelled` whatever the directory holds. The mechanism named above --- the predecessor completing before the cancellation took effect --- cannot occur in the witness, because the predecessor cannot start until the fixture lets it. Demonstrated in both directions below |

Tally (U17): 10 = 9 + 1.

**The chain.** `read_dir_supersede_cancels_in_flight_predecessor`
dispatched two `pmacs.fs.read_dir`s under one supersede key in one Lua
chunk and asserted the first settled `cancelled`. `AsyncRuntime::
allocate` flips the predecessor's token synchronously when the
successor is allocated, but the predecessor's worker had already been
running since its own dispatch: the pool's threads take a queued job
within microseconds, and `read_dir_blocking` over 256 empty files
finishes in well under a millisecond on a fast runner. When the
worker's enumeration ended before the flip, its reply was `ok`, and
`tick` surfaced it as such --- the "supersede race lost the other
way". Directory size and scheduler luck decided the outcome; the
product's supersede contract was never what the test measured, which
is why ten occurrences across three CI legs and one local non-CRDT
sweep never converged on a mechanism in the code.

**What was removed** is the test's dependence on the worker being slow.
`AsyncRuntime::pool()` (new, `src/async_runtime.rs`) exposes the pool
beneath the runtime, and the witness dispatches one hold per thread on
it --- a closure that sends on a `started` channel and then blocks
until the test drops its release sender --- and waits for as many
`started` reports as `pool().size()`. The pool steals from its injector
in batches, so a *queued* hold does not occupy a thread; a *started*
one does, which is why the reports are the precondition and not the
dispatches. From that moment no thread is free: the predecessor's
dispatch queues it, the successor's dispatch flips its token, and the
test asserts before releasing that neither job has settled and the key
already names the successor (the positive control). Releasing lets the
predecessor run into its flipped token at the first entry. The test
still asserts the successor returns every entry.

**The experiment**, on the gate's `m8_1_acceptance` binary at this
commit, the same machine that produced the fifth occurrence:

| binary | arm | runs | `got ok` |
|---|---|---|---|
| this commit | the witness alone, 300 runs | 300 | **0** |
| this commit | the whole `m8_1_acceptance` suite (10 tests, default parallelism), 40 runs | 40 | **0** |
| this commit with `job.cancel.cancel()` removed from `allocate` | the witness alone, 30 runs | 30 | **30** |

Tally (U17-bite): 30 = 30 + 0.

The third row is the bite and the reason the first two are not a count
of green runs: with the supersede's cancel removed the witness fails
every time with the row's exact fragment (`first read_dir must be
superseded; got ok`), so what the 340 green runs show is a test that
now decides on the product's behavior and on nothing else. The removal
is what closes the row; the 0/340 is corroboration.

**What is not closed.** A predecessor whose worker has *finished* ---
its `ok` reply on the bus, not yet absorbed by `tick` --- and is then
superseded still settles `ok`, because the runtime decides a job's
outcome from the worker's reply and not from the token at absorption.
That is a product nuance the contract's own words ("in-flight
predecessor") admit, it is not what this row measured, and it is not
filed here; naming it is not a claim that a user can observe it. The
retirement criterion the row carried since C1 was the witness, and
that is what landed.

### What U8 falsified: reporting is not repair

U8 and R6 closed on the same readiness migration in the same sitting,
and R5 was swept along with them. R5's closure was void because the
migration could not reach its site. U8's is a different failure and a
more instructive one: the migration **did** reach it. That is not a
guess --- U8's 2026-09-08 recurrence (#259) carries required fragments
precisely because the migrated wait now reports its elapsed time, its
poll count and what it last observed, which the pre-migration wait did
not. The mechanism named in the closer is real, it applied, and it did
what it claimed.

It was still not a closer. Replacing a fixed drain with a wait that
*reports* what it saw improves the next occurrence's diagnosis; it
removes no cause. The closer inferred repair from better instruments,
and #259 falsified the inference twenty-four days later by producing
the signature again, on the migrated code, on the same platform, twice
in consecutive runs.

**The class, so it is not rediscovered.** A closer of the form "the
wait now reports X" is an INCOMPLETE closure and should never have been
written as a retirement; a closer of the form "the wait now waits for
the record it asserts on" is causal, because the fixed drain that ended
early is gone. R6 (`the wait now reports what the child last wrote`)
and U19 (`the wait reports the last frame seen`) carry the reporting
shape, and at C1's close the rule was applied to them by the form of
the closer rather than by waiting for a red, because U8 is the proof
that a recurrence only makes the incompleteness visible. Checked
against each failing site: R6's site is migrated, so the closer reached
it and removed no cause --- INCOMPLETE, U8's class on U8's own test ---
and the migration replaced the string one of R6's required fragments
names, so the row could never fire again while the failure it was filed
for went on happening under #259's fragments. U19's selector is an
in-crate `--lib` test the migration cannot reach, and its loop reports
nothing --- VOID, R5's ground. U2 (`the PTY read now waits for the
record it asserts on`) and R4 (`wait_for_file waits for the expected
bytes`) are the causal form and are not in question. U14's fourth
closer has the reporting shape too and stays unclassified: its selector
is not recoverable from the row, so its site cannot be read.

### U4's question is open again, on Linux

A failure carrying U4's selector and all three of its fragments was
observed on 2026-09-07 on Linux, in the default-feature sweep of
`scripts/gate --protocol` at `3abc153` (gate log
`20260907T195626Z-978397`, step `05-sweep`; 33,566 bytes emitted after
the first painted frame with no CSI 2 J among them). It postdates U4's
void closure and is filed as #255. It is not what reopened U4 --- the
audit above did that, on the closer's own wording --- but it is a
second reason the question is open.

It is not a recurrence of the row. A row matches only when the job
matches as well as the selector and the fragments, and U4's job is
macOS; its own exemption of the Lua flavor as a matching key covers
lua54 against luajit, not macOS against Linux, so nothing in the row
establishes that one platform's observation is the other's. The
mechanism is unconfirmed. U4's record also holds a deliberate
reintroduction of the genuine resync defect reproducing these same
fragments, so the symptom cannot discriminate a fixture race from a
product regression, and the discrimination in #255 is what would settle
it.

### PR #257's CI runs

**A count of runs does not go in a heading, or anywhere else that has
to stay true.** This section was headed "PR #257 ran CI twice" and
opened "Two runs exist" while a third was already running; the heading
before it said the same about one. A table takes a new row; a sentence
that names a total has to be found and rewritten, and twice it was not.

| run | sha | created | completed | verdict |
|---|---|---|---|---|
| 34220035122 | `8f6784f` | 2026-09-08T11:18:55Z | 11:38:36Z | 14 green, 3 red, 1 skipped |
| 34222042303 | `e78d184` | 11:41:44Z | 12:01:39Z | 15 green, 2 red, 1 skipped |
| 34253949749 | `b2094ac` | 16:54:59Z | 17:09:27Z | 16 green, 1 red, 1 skipped |
| 34269795016 | `04263c6` | 19:35:31Z | 19:59:11Z | 16 green, 1 red, 1 skipped |
| 34272480226 | `d7fd465` | 20:02:42Z | 20:21:44Z | 16 green, 1 red, 1 skipped |

Every verdict is counted from the jobs endpoint,
`repos/levineuwirth/pmacs/actions/runs/<id>/jobs?per_page=100`: each
run has **18 jobs on one attempt**, one of them `Docs consistency`,
skipped correctly because the PR's changed paths include code. **The
first two verdicts are corrected.** Every C1 record until the closing
round stated them as 12 green and 13 green, this table included, and
the wrong pair reached nine artifacts, both review passes among them.
The endpoint gives `{failure: 3, skipped: 1, success: 14}` and
`{failure: 2, skipped: 1, success: 15}`, and no counting convention
yields 12 and 13 here and 16 for the other three. The pushed records
that carry 12 and 13 are not rewritten; this is the correction, in the
form `e78d184`'s commit message is corrected below.

All five are `pull_request` events with conclusion **failure**. The
first two ran trees differing by one markdown file; the third ran fix
round 1's seven commits; the fourth ran fix round 2's first ten; the
fifth, at the head the branch merged from, ran the eleventh, which is
the commit that recorded the fourth.

**This table holds the runs someone has read, and it can never hold the
last one.** Every push starts a run at the new head, and a record
committed by that push is written before its own run exists --- so a
table in the repository is structurally one run behind, and no wording
fixes that. `gh run list --branch e1/first-ten-minutes` is the live
list and is what a merge decision reads.

**And on a pull request only the newest run survives.**
`.github/workflows/ci.yml` sets `concurrency: group:
ci-${{ github.event.pull_request.number || github.sha }}` with
`cancel-in-progress` true for `pull_request` events, so each push
cancels the run still in flight for that PR. Fix round 2 demonstrated
it three times: runs **34267354381** at `8cdcda4`, **34268588682** at
`f7a1221` and **34268667347** at `cc11f17` were each cancelled by the
next push, with no verdict and no logs worth reading. A cancelled run
is not a green one and not a red one --- it is no evidence at all, and
must never be counted as a run in this table.

The consequence for a merge decision is worth stating plainly: **only
the newest completed run describes the current head.** Every completed
run stays readable --- five are tabulated above --- and each is
evidence about the tree it ran and about nothing pushed after it. An
earlier version of this paragraph said the branch had "exactly one
readable CI run at any moment", four lines under a table of four
readable runs; what it meant is the sentence before this one. What
stays true by construction is the other half: the run at whatever
head this text is read from is not tabulated, because the push that
commits the text is what starts it. The branch stopped moving at
`d7fd465` and merged, so its fifth run could at last be read and
tabulated here, from `main`, by the next phase's first commit.

Every C1 record before 2026-09-08 described only the first run, by name
and as "the first run". Fix round 1 named the first two and was written
fifty seconds after the push that started the third.

#### Run 34222042303, at `e78d184`

Three things are in it.

`Test (macos-latest / lua54)`, job 102047236847, failed twice.

1. `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel`,
   `tests/bottom_panel_stage1_acceptance.rs:2447`, `bytes in
   /var/folders/.../T/.tmph8ib1k/ready did not become ready within 5s
   (waited 5.031151625s, 54 polls); last observed: No such file or
   directory (os error 2)`, `test result: FAILED. 49 passed; 1 failed`.
   Selector, job and all three required fragments are #259's: a **second
   occurrence**, twenty minutes after the issue was opened. It is a
   comment on #259, which is what that issue's own first line requires.
2. `editor::tests::stream_supersede_delivers_cancelled_to_on_close`,
   `src/editor.rs:12842`, `async pump deadline exceeded`, `test result:
   FAILED. 2196 passed; 1 failed; 11 ignored`. The message, the path and
   the platform are closed row **R5**'s, and this postdates its closure,
   so it reopens R5's question. R5 is live again above, as never-closed.

`Test (macos-latest / luajit)`, job 102047236931, failed once:
`v15_peer_never_receives_theme_facts_and_v16_does`,
`tests/theme_faces_acceptance.rs:1383`, `read Hello: Io(Os { code: 35,
kind: WouldBlock, message: "Resource temporarily unavailable" })`, `test
result: FAILED. 26 passed; 1 failed`. Under this file's matching rule
that is **not** #258. #258's selector is
`a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` in
`statusline_segments_acceptance`; a test-name match is never sufficient
and here even the test name differs. It is a new incident that shares
#258's failing expression — a fixture-set short read timeout on the
daemon's server-first `Hello`, here 250 ms set four lines above the
failing call. It is recorded as a comment on #258 with that difference
stated, because folding it in on resemblance is what the matching rule
forbids; whether the two share a mechanism is unestablished. The suite
is one this branch edited (two `set_line_numbers('off')` lines in
`editor()` and `open_and_wait_for_parse`, nowhere near `probe`).

`Test (ubuntu-latest / luajit)` is green in this run, so U17 did not
reproduce at the head. That is non-reproduction and nothing more.

#### Run 34253949749, at the pushed head `b2094ac`

Sixteen jobs green, one red: `Test (macos-latest / lua54)`, job
**102154957233**, which ran 120 targets. 119 ended `test result: ok`
and one ended

```
thread 'ctrl_c_during_reconnect_sleep_yields_clean_exit' (63826) panicked at tests/m5_8_acceptance.rs:546:5:
Ctrl-C during reconnect sleep should produce a clean exit; status: ExitStatus { code: 1, signal: Some("Interrupt: 2") }
test result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.73s
```

Selector, job and **both** required fragments are U5's, verbatim; the
`:LINE` suffix is not a fragment. U5 is live above as never-closed, and
this is its second occurrence, filed as **#260**.

**What this run's greens do and do not establish.** Four selectors this
branch is a candidate for ran in it and passed, each on **both** macOS
legs (jobs 102154957233 and 102154957288):

- `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel` (#259);
- `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` (#258);
- `v15_peer_never_receives_theme_facts_and_v16_does` (the `theme_faces`
  incident);
- `editor::tests::stream_supersede_delivers_cancelled_to_on_close` (R5).

That is **non-reproduction and nothing more**. By the rerun rule it
establishes no absence, exonerates nothing and diagnoses nothing; these
are load-dependent tests and one green run at the head is one sample.

**And it is the only head-side evidence the merge decision has.** It is
the single CI run at the pushed sha, and in it the branch's three
candidate regressions and its reopened row each had a chance to recur
across a delta of seven commits, and none did. That is a fact about the
record, not about the code: it is worth having precisely because there
is nothing else at the head, and it is worth nothing more than one
sample is worth.

Both halves of that are the finding. Neither is quotable without the
other: the first half alone reads as exoneration, the second alone
suppresses the only measurement at the head there is.

#### Run 34269795016, at `04263c6`

Sixteen jobs green, one red: `Test (macos-latest / luajit)`, job
**102208453110**, with **five** failing targets in it --- the largest
cluster this branch has produced, and the first on this leg.

| suite | selector | message |
|---|---|---|
| `bottom_panel_stage1_acceptance` | `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel` | `/ready did not become ready within 5s (waited 5.029068625s, 58 polls); last observed: No such file or directory` |
| `gpu_font_acceptance` | `v16_peer_never_receives_font_facts_and_v17_does` | `read Hello: Io(Os { code: 35, kind: WouldBlock, … })` |
| `m5_5_acceptance` | `m10_10_non_replica_frontend_does_not_receive_cursor_byte` | the same `read Hello` WouldBlock |
| `m8_3_acceptance` | `wdired_external_same_size_same_second_rewrite_aborts` | `could not produce same-second rewrite with distinct nanoseconds` |
| `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` **and** `v15_peer_never_receives_theme_facts_and_v16_does` | the same `read Hello` WouldBlock, twice |

**Dispositions, one per mechanism and none folded on resemblance.**

- The first is **#259**'s selector with all three required fragments, on
  macOS **luajit** this time where the two earlier occurrences were
  lua54. #259 states in its own body that the flavor is not the
  discriminator, so it matches: a **third occurrence**, a comment on
  #259.
- The four `read Hello` WouldBlock failures are **not #258** under the
  matching rule --- #258's selector is
  `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` in
  `statusline_segments_acceptance`, and none of these is that test. Its
  occurrence count was one when this was written and is **two** since
  the next run, below; the earlier wording, "stays at one", is
  corrected here rather than rewritten in the pushed commit that
  carries it. The last of them is the second occurrence of the
  `theme_faces` incident already recorded there. All four are a comment
  on #258, which is the issue for that failing expression, with the
  difference stated.
- The `wdired` failure is a **new signature** with no row and no issue:
  filed as **#261**.

**What the four-in-one-job changes, and what it does not.** Every
earlier record of `read Hello: WouldBlock` argued it was a per-fixture
short read timeout, a value the test sets a few lines above the failing
call. Four selectors across three suites failing that way inside one
job is the first observation that reading does not fit comfortably:
either four independent fixtures each chose too small a budget and all
four lost the same race in the same job, or something job-wide on that
runner delayed the daemon's server-first `Hello` everywhere. **Neither
is established** and this file does not choose between them; what would
is a measurement of how long `Hello` actually took against each
fixture's budget, which nothing has done. It is recorded because it is
the first evidence that bears on the question at all.

**None of this was re-run.** The job stands as read.

#### Run 34272480226, at the head `d7fd465`

Sixteen jobs green, one red: `Test (macos-latest / luajit)`, job
**102217330515**, one failing target against 119 `test result: ok`:

```
---- a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join stdout ----
thread 'a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join' (114814) panicked at tests/statusline_segments_acceptance.rs:996:54:
called `Result::unwrap()` on an `Err` value: Io(Os { code: 35, kind: WouldBlock, message: "Resource temporarily unavailable" })
test result: FAILED. 10 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.26s
```

Selector, job and all three required fragments are **#258's**, at the
same site and with the same result line as its first occurrence in run
34220035122: **#258's second occurrence**, commented there on
2026-09-09. Not a rerun --- a different tree twenty-nine commits later
--- and under the rerun rule a second occurrence is a second
occurrence. This head is the one the branch merged from, so this run
is the first on the branch that could be read after the branch stopped
moving, and the first that a record could hold without being one run
behind.

**The `read Hello` family is at seven occurrences**, across four
suites and five selectors, twice under #258's own selector. Counted
from every macOS job log of every completed run on the branch and at
the base, by the failing expression's `WouldBlock` and not by the
literal string `read Hello`, which the two a16_26 occurrences do not
carry (their panic is the bare `Result::unwrap()`; a grep for the
string gives five and a grep for `WouldBlock` gives seven):

| # | run | sha | suite | selector |
|---|---|---|---|---|
| 1 | 34220035122 | `8f6784f` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |
| 2 | 34222042303 | `e78d184` | `theme_faces_acceptance` | `v15_peer_never_receives_theme_facts_and_v16_does` |
| 3 | 34269795016 | `04263c6` | `gpu_font_acceptance` | `v16_peer_never_receives_font_facts_and_v17_does` |
| 4 | 34269795016 | `04263c6` | `m5_5_acceptance` | `m10_10_non_replica_frontend_does_not_receive_cursor_byte` |
| 5 | 34269795016 | `04263c6` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| 6 | 34269795016 | `04263c6` | `theme_faces_acceptance` | `v15_peer_never_receives_theme_facts_and_v16_does` |
| 7 | 34272480226 | `d7fd465` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |

**Zero at the merge base**, where all four of those selectors ran and
passed on both macOS legs of run 34205653191. The comment on #258 of
2026-09-09 says six; it is one short of its own list, and this is the
correction of record.

**SUPERSEDED: the family is at nine.** Run 34358682895 at `65648ec`
added two more, one of them a new suite and selector. The table above is
what was true at `d7fd465` and is not rewritten; the nine-row
enumeration is in *PR #262's CI run at the head* below.

**What #258's mechanism has that no other live row has is a
measurement**, taken by C1's end-to-end round and re-checked at its
close: on Linux, on one machine, bounding a ratio. `run_daemon` binds
the listener (`src/daemon.rs:470`) before it constructs `EditorState`
(`:479`), and `wait_for_daemon` in `tests/common/ready.rs` declares
readiness on a successful `connect` while discarding its own 500 ms
`Hello` read, so a fixture's budget covers only the residual boot and
the failure needs bind-to-serving above roughly 700 ms. Over 1,500
interleaved boots at `d97e137` and `d7fd465` the probe's
connect-to-`Hello` is p50 50.14 ms and 50.13 ms with none of 600 idle
probes over 100 ms, and the boot the branch's 407 added Lua lines
lengthen moves p50 13.93 to 14.28 ms, paired mean +0.401 ms;
`src/daemon.rs` is byte-identical between the shas and
`ACCEPT_POLL_INTERVAL` is 50 ms at both, so the probe pays one whole
accept quantum and a16_26's real headroom is 150 ms, not 200. That is
why D30 calls #258 a fixture/daemon contract defect that predates the
branch. The limit in the same breath: it bounds what the branch added,
and it is not the interval on a macOS runner, which at C1's close no
record had measured. #258 carries anything measured since.

#### The macOS interval was measured on 2026-09-09, and it falsifies the excursion argument on the failing platform

This section is the correction the paragraph above deferred to. It was
owed at `f6f36bb`, which landed at 12:42:26Z --- seven minutes before
the measurement existed --- and none of the six commits after it came
back.

Run **34351035133**, one `workflow_dispatch` on the throwaway branch
`measure/hello-macos` at `50da74e` (`main` plus an instrument, left on
the remote so its sha resolves, never to merge; its `Format` and `Lint`
jobs are red on the instrument and mean nothing). The instrument times
the `Hello` read `tests/common/ready.rs::wait_for_daemon` normally
discards --- first accepted `connect` to first `Hello` --- with the cap
raised from 500 ms to 20 s, and dumps a report from a last-sorted suite.

**One quantile convention for the whole table: the instrument's own
report, read from the job logs.** Every figure below is a column of the
`site budget n p50 p90 max over_budget not_ok` line that suite printed;
nothing here is recomputed. This is stated because the published `594`
was not that: it is the rank-44 lower middle of the 88 raw samples,
while the same row's `881` was taken from the report, so one row carried
two provenances. Recomputing the row nearest-rank instead gives p50
**594.07** and p90 **888.57** --- a 0.6% difference that moves no
decision, which is why the fix is to name a convention rather than to
argue for one.

Boot to first `Hello`, one row per leg, all four from the same
instrument and therefore comparable to each other:

| leg | budget | n | p50 | p90 | max | over budget |
|---|---|---|---|---|---|---|
| `Test (macos-latest / luajit)` | 500 | 88 | **597.50** | **881.21** | **1114.80** | **71** |
| `Test (macos-latest / lua54)` | 500 | 88 | 61.15 | 153.45 | 177.96 | 0 |
| `Test (ubuntu-latest / lua54)` | 500 | 88 | 8.88 | 10.09 | 14.29 | 0 |
| `Test (crdt)` | 500 | 88 | 4.69 | 5.79 | **63.70** | 0 |

`a16_26`'s probe against a daemon already booted, same report:

| leg | budget | n | p50 | p90 | max | over budget |
|---|---|---|---|---|---|---|
| `Test (macos-latest / luajit)` | 200 | 1 | 202.84 | 202.84 | 202.84 | 1 |
| `Test (macos-latest / lua54)` | 200 | 3 | 148.39 | 196.76 | 196.76 | 0 |
| `Test (ubuntu-latest / lua54)` | 200 | 3 | 48.56 | 49.90 | 49.90 | 0 |
| `Test (crdt)` | 200 | 3 | 48.92 | 49.67 | 49.67 | 0 |

`Test (ubuntu-latest / luajit)` produced no samples: it failed on the
gopls fetch before any test ran, U18/#249's mechanism, exactly as it did
in run 34358682895.

**"On Linux under 15 ms" is false for one of the two Linux legs.**
`Test (crdt)`'s max is 63.70 ms. Its p50 is 4.69 and `Test
(ubuntu-latest / lua54)`'s max is 14.29, so the summary held for three
of the four figures it covered and not the fourth.

**The consequence for D30, stated plainly.** D30 justifies E1's merge
with an excursion argument: readiness is a successful `connect`, the
daemon binds before it can serve, so *"failure needs bind-to-serving
above ~700 ms against a measured ~14 ms"* --- roughly fiftyfold, and
therefore safe. On the leg that actually fails, bind-to-serving is p50
**597.50 ms**, p90 **881.21 ms**, max **1114.80 ms**, and **71 of 88**
boots already outrun the 500 ms after which the production readiness
wait declares readiness anyway. The ~700 ms threshold is not a
fiftyfold excursion there; it sits inside the observed distribution.
D30's second bolded clause --- that the job-wide macOS question is
UNTAKEN and that "macOS is two to five times over its budgets" is from
the CI logs rather than a measurement on the platform --- is false as
of 12:49:57Z on 2026-09-09.

**What the measurement does not establish**, in its own terms: it is one
`workflow_dispatch` run, one tree, one arm --- a distribution, not the
paired two-sha comparison C1 ran on Linux, and not a rate over time. The
cross-*leg* rows are four different jobs on four different runners, so
the tenfold macOS luajit/lua54 gap is not attributable to the Lua flavor
or to the runner and is not attributed here. `~14 ms` in D30 is *bind to
`daemon listening`* on a laptop, a different endpoint from this table's
*first connect to first `Hello`*; the comparable Linux figures are this
table's own 8.88 and 4.69. Nothing was fixed from any of it.

**Where this correction has and has not landed.** It is in the comment
on #258 of 2026-09-09T12:49:57Z with these limits, in E2's handoff and
fixes passes, in PR #262's body, in PR #257's body, in the roadmap's C2
entry and in D30's own row, which now carries the original wording
quoted beside it. It is **uncorrectable in `dbe40a1`**, the merge commit
on `main`, whose message reads *"failure needs bind-to-serving above
~700 ms against ~14 ms observed"*; that sentence is wrong for the
platform the accepted red is on and it is in `main`'s permanent history.

### `main` after E1: run 34349759554 at `dbe40a1`

E1 merged as `dbe40a1` (squash of `d7fd465`, PR #257) on 2026-09-09, and
its post-merge `push` run is **34349759554**: 18 jobs on one attempt,
**16 green, 1 red, 1 skipped** (`Docs consistency`, correctly). A
`push` run is keyed by sha in `ci.yml`'s concurrency group, so nothing
cancelled it and it stands as read. The red is `Test (macos-latest /
luajit)`, job 102459915513, with one failing target:

```
---- read_dir_supersede_cancels_in_flight_predecessor stdout ----
thread 'read_dir_supersede_cancels_in_flight_predecessor' (96993) panicked at tests/m8_1_acceptance.rs:278:5:
assertion `left == right` failed: first read_dir must be superseded; got ok
  left: "ok"
 right: "cancelled"
test result: FAILED. 9 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.08s
```

That is **U17's selector and required fragment**, its fourth
occurrence and its third on `main`, recorded on the row above. E0's
post-merge run produced this row's second occurrence on the same leg,
so E1's merge is the second consecutive merge whose post-merge run is
red on U17 alone.

**D30's revocation condition is not met.** D30 revokes E1's merge
disposition if #258's selector recurs on `main` after the merge or a
diagnosis shows a product defect. The job's log carries **zero**
`WouldBlock`, so neither #258's selector nor any member of the `read
Hello` family appeared, and no other selector this file or the
`intermittent-red` issues name appeared either: `a16_26_…`, `acc28_…`,
`v15_peer_…`, `ctrl_c_during_reconnect_sleep_…`,
`stream_supersede_delivers_cancelled_to_on_close` and
`wdired_external_same_size_same_second_rewrite_aborts` all ran on this
leg and passed. One run, non-reproduction and nothing more; what it
establishes is that the condition D30 named did not fire on the one
run that could have fired it.

### `main` after E2: run 34396945488 at `ea8c93a`, and it is GREEN

E2 merged as `ea8c93a` (PR #262) on 2026-09-09, and its post-merge
`push` run is **34396945488**: 18 jobs on one attempt, **17 success, 1
skipped, ZERO failures** (created 19:45:28Z, completed 20:00:29Z). The
skip is `Docs consistency`, correctly, since the push changed code.
**This is the first fully green post-merge run in three phases**: E0's
was 34205653191 and E1's 34349759554, both red, both on U17 alone by the
end.

Read from the job logs and not from the verdict line. Every one of the
six test legs carries **zero** `WouldBlock`, **zero** `test result:
FAILED`, and 122 or 123 `test result: ok`:

| leg | job | `WouldBlock` | `FAILED` | `ok` |
|---|---|---|---|---|
| `Test (macos-latest / luajit)` | 102618943080 | 0 | 0 | 123 |
| `Test (macos-latest / lua54)` | 102618943054 | 0 | 0 | 123 |
| `Test (ubuntu-latest / luajit)` | 102618943024 | 0 | 0 | 123 |
| `Test (ubuntu-latest / lua54)` | 102618942991 | 0 | 0 | 123 |
| `Test (ubuntu-latest / luajit, no crdt)` | 102618943181 | 0 | 0 | 123 |
| `Test (crdt)` | 102618942960 | 0 | 0 | 122 |

**#258's own selector ran on the leg it fails on and passed.**
`a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join ... ok`
appears in five of the six legs — every leg that carries it; it is
absent from the no-crdt leg, which does not build it. U16's
`bare_filename_saves_in_cwd` ran in all six and passed. No `deadline
exceeded`, no `Interrupt: 2`, no `first read_dir must be superseded`.

**D31's revocation condition is not met.** D31 revokes E2's merge
disposition if #258's selector recurs on `main` after the merge or a
diagnosis shows a product defect. Neither happened. By the rerun rule
this is **non-reproduction and nothing more**: it retires nothing, and
the counts of record are unchanged — **#258 stays at four occurrences
and the `read Hello` family at twelve**, which this file states is a
floor. E2's accepted risks stay accepted, undiagnosed and open.

### E3's local gate: two intermittents, both filed, neither this branch's

`e3/gui-scroll-and-current-line` ran `scripts/gate --protocol` twice.
Both runs are recorded here rather than re-run away, and the reds in
both are in code the branch does not touch.

**Run `20260909T203257Z-3490194`** — `doc`, `sweep` and `sweep-luajit`
red. Two of the three were the branch's own and were fixed: a
`[`Self::text_bounds_right`]` intra-doc link written inside a
constant's doc comment, where `Self` is the constant; and
`theme_faces_acceptance`'s literal count of the stage-1 face inventory,
which E3.3's `ui.current-line` moves from thirteen to fourteen. The
third was **#251's second occurrence** — `bundled package `repl` failed
to load`, panic at `src/editor.rs:1113` — matched to #251 **by its
required fragments and not by name**, the selectors being different
from the first occurrence's. Two selectors failed together this time,
five log lines apart at the head of a 2,248-line log.

That occurrence also **settles the question #251 was filed with.**
`write_if_changed` (`src/builtin_packages.rs:258`) is a content compare
followed by a bare `fs::write`, with no temporary-name-and-rename and no
lock; `fs::write` opens `O_TRUNC`, so a concurrent reader can observe a
zero-length or partial `init.lua`. `tests/common/iso.rs:43` justifies
sharing the root on the ground that materialization is "content-gated
and idempotent" — and **idempotent is not atomic**: the gate makes
repeated writes converge, while the failure is a concurrent read of an
intermediate state. So the co-failure has a demonstrated shared object,
which is what this file's own widening rule asks for.

**Run `20260909T204440Z-3628136`** at the fixed tip — seven of eight
stages ok, `sweep-luajit` green over 125 targets, and `sweep` red on
one target: `daemon_attach::tests::ensure_running_invokes_spawner_then_waits_for_socket_to_appear`,
`expected Ok, got Err(AutoStartTimeout`, filed as **#264**. Also a
fixture defect with a demonstrated mechanism: the fixture's spawner
holds its listener for exactly 500 ms, so the test's success window is
that 500 ms and not the 2 s deadline, and a polling thread descheduled
past it finds nothing to connect to and then correctly runs out the
deadline. Widening the deadline would change nothing. The `-p pmacs
--lib` binary took **20.67 s** in that run against **15.08 s** in the
previous one, which is the load proxy.

### The macOS reds have a merge-base control, and it excludes nothing

`git merge-base e78d184 githubsucks/main` is `d97e137`, whose post-merge
run is **34205653191**. Both of its macOS legs were read from their job
logs (`gh run view --job <id> --log`; the API logs endpoint returns
empty for these):

- `Test (macos-latest / lua54)`, job 101994444426: **zero failures in
  the whole job** — 120 `test result: ok` lines and not one `test
  result: FAILED`. It ran
  `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel ...
  ok`, `v15_peer_never_receives_theme_facts_and_v16_does ... ok` and
  `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join ... ok`.
- `Test (macos-latest / luajit)`, job 101994444326: exactly one failure
  in the whole job, `read_dir_supersede_cancels_in_flight_predecessor`
  (U17's second occurrence). It ran `acc28... ok`, `a16_26... ok` and
  `v15_peer... ok`.

So all three macOS signatures this branch produced — #259's, #258's and
the `theme_faces` incident's — ran at the merge base and passed there.
**The branch is a candidate for all three rather than excluded from
them** --- which is a shift in the burden and not a diagnosis, per the
section on what one control run establishes above --- and the C1 records
that reasoned from untouched files are superseded: the branch changed a
default that every window renders through, so an untouched suite is not
an unchanged suite.

**The weight of this control, in the same words as U17's.** It is one
CI run at the merge base, which is one sample. U17's control is one such
run in which the signature *did* appear, which establishes that the
signature exists on `d97e137` and not its rate. This is one such run in
which three signatures did *not* appear, and by the rerun rule above,
applied at the base instead of at the head, that is non-reproduction and
nothing more: a single green run does not establish absence in a
load-dependent test. What the control changes is which way the burden
falls, not how much evidence there is, and it diagnoses nothing.

`e78d184`'s commit message states #258's non-causation flatly — "which
is upstream of everything this branch changes --- the read that failed
precedes the AttachRequest" — without the "an argument from the failing
expression, not a measurement" qualifier that the passes, the PR body
and the four issues all carry. That claim is superseded by the control
above. The commit is pushed and is not rewritten; this is the
correction.

### PR #262's CI run at the head `65648ec`

Run **34358682895**, `pull_request`, head `65648ec`, one attempt, 18
jobs, created 2026-09-09T13:41:21Z, completed 14:02:36Z, conclusion
`failure`: **13 green, 4 red, 1 skipped** (`Docs consistency`, skipped
correctly --- the changed paths include code). Counted from the jobs
endpoint. Nothing was re-run.

| job | id | verdict | disposition |
|---|---|---|---|
| `Lint (luajit)` | 102489798981 | RED | a **product defect of this branch**, not a signature |
| `Test (ubuntu-latest / luajit, no crdt)` | 102489852744 | RED | the same defect |
| `Test (ubuntu-latest / luajit)` | 102489852728 | RED | **U18 / #249**, the gopls fetch, before any test ran |
| `Test (macos-latest / luajit)` | 102489852994 | RED | the `read Hello` family, **two members**, one of them new |

**The two ubuntu reds were one line of this branch's own test code and
are fixed, not dispositioned.** `tests/gui_desktop_basics_acceptance.rs`
declared `fn gpu_binary` at module scope while its only caller is
`#[cfg(feature = "crdt")]`, so the opt-out build saw dead code and both
legs that set `-D warnings` failed to compile the test target:

```
error: function `gpu_binary` is never used
  --> tests/gui_desktop_basics_acceptance.rs:10:4
   = note: `-D dead-code` implied by `-D warnings`
error: could not compile `pmacs` (test "gui_desktop_basics_acceptance") due to 1 previous error
```

Deterministic, reproducible on any machine with one command, and so
outside this file's scope as a *signature*; it is recorded here only
because two of the four reds in this run are it, and a later reader
counting reds against rows would otherwise find two with no row. Fixed
in `0c4eab2`. **No plan `scripts/gate` could print would have caught
it** --- `-D warnings` was set in one stage that ran under default
features, and `sweep-luajit` passes no `RUSTFLAGS`, so `cargo test
--workspace --no-default-features --features luajit --no-run` finished
with it as a warning. `16fddd4` adds `clippy-luajit` to the `--protocol`
plan, which is CI's own `Lint (luajit)` second step verbatim.

**`Test (ubuntu-latest / luajit)` is U18 / #249 again**, and it produced
no test evidence at all: **zero** `test result:` lines in the whole job.
It failed at *Install external tools* on

```
go: golang.org/x/tools/gopls@v0.16.2: loading deprecation for
golang.org/x/tools/gopls: module golang.org/x/tools/gopls: read
"https://proxy.golang.org/golang.org/x/tools/gopls/@v/list": stream
error: stream ID 33; INTERNAL_ERROR; received from peer
```

exit 1. Not a test, so REFERRED as the row says; recorded because a leg
that runs nothing is not a leg that passed.

#### The `read Hello` family is at nine, across five suites and six selectors

`Test (macos-latest / luajit)`, job 102489852994, carries exactly two
`WouldBlock` occurrences, verbatim:

```
---- daemon_routes_semantic_family_to_semantic_session_only stdout ----
thread 'daemon_routes_semantic_family_to_semantic_session_only' (84016) panicked at tests/m11_5_semantic_acceptance.rs:260:47:
semantic read Hello: Io(Os { code: 35, kind: WouldBlock, message: "Resource temporarily unavailable" })
test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.05s
```

```
---- daemon_reships_the_summary_after_a_real_buffer_round_trip stdout ----
thread 'daemon_reships_the_summary_after_a_real_buffer_round_trip' (123219) panicked at tests/theme_faces_acceptance.rs:1034:50:
read Hello: Io(Os { code: 35, kind: WouldBlock, message: "Resource temporarily unavailable" })
test result: FAILED. 26 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.10s
```

The first is a **new suite and a new selector** for this family, and its
message carries a `semantic ` prefix the other members do not. The
second is the third occurrence of member 5.

**The count is nine, recomputed from the enumeration below and not
incremented from seven.** The enumeration is the seven rows recorded at
`d7fd465` plus these two; each is a distinct (run, selector) pair read
from a macOS job log, counted by the failing expression's `WouldBlock`
rather than by the literal string `read Hello`, which the two `a16_26`
occurrences do not carry:

| # | run | sha | suite | selector |
|---|---|---|---|---|
| 1 | 34220035122 | `8f6784f` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |
| 2 | 34222042303 | `e78d184` | `theme_faces_acceptance` | `v15_peer_never_receives_theme_facts_and_v16_does` |
| 3 | 34269795016 | `04263c6` | `gpu_font_acceptance` | `v16_peer_never_receives_font_facts_and_v17_does` |
| 4 | 34269795016 | `04263c6` | `m5_5_acceptance` | `m10_10_non_replica_frontend_does_not_receive_cursor_byte` |
| 5 | 34269795016 | `04263c6` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| 6 | 34269795016 | `04263c6` | `theme_faces_acceptance` | `v15_peer_never_receives_theme_facts_and_v16_does` |
| 7 | 34272480226 | `d7fd465` | `statusline_segments_acceptance` | `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join` |
| 8 | 34358682895 | `65648ec` | `m11_5_semantic_acceptance` | `daemon_routes_semantic_family_to_semantic_session_only` |
| 9 | 34358682895 | `65648ec` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |

Nine rows; five distinct suites (`statusline_segments_acceptance`,
`theme_faces_acceptance`, `gpu_font_acceptance`, `m5_5_acceptance`,
`m11_5_semantic_acceptance`); six distinct selectors (`a16_26_…`,
`v15_peer_…`, `v16_peer_…`, `m10_10_…`, `daemon_reships_…`,
`daemon_routes_…`). The seven-row table under run 34272480226 above is
what was true at `d7fd465` and is left as it stands; this is the
correction of record, in the file's own form.

**#258's own selector did not fire in this run and U17 did not recur.**
The base's post-merge job 102459915513 has zero `WouldBlock` and zero
`read Hello`, so `65648ec` is a **candidate** for members 8 and 9 rather
than excluded from them --- one green run at the base is
non-reproduction and nothing more, and `tests/theme_faces_acceptance.rs`
is a file this branch edits, while `tests/m11_5_semantic_acceptance.rs`
is not. Both are a comment on #258 with the matching-rule difference
stated, as the rule above requires: neither is #258's selector.

### PR #262's CI run at the fix-round head `e5417f6`

Run **34369540895**, `pull_request`, head `e5417f6`, one attempt, 18
jobs, created 2026-09-09T15:19:58Z, completed 15:40:22Z, conclusion
`failure`: **16 green, 1 red, 1 skipped** (`Docs consistency`, skipped
correctly). Counted from the jobs endpoint. Nothing was re-run.

**The build break is gone from CI, on both legs that carried it.**
`Lint (luajit)` (102526900610) and `Test (ubuntu-latest / luajit, no
crdt)` (102526964980) are **green**, against red at `65648ec` on the
same two. `0c4eab2` fixed the defect and `16fddd4` made it a local red
rather than a remote one. **U18/#249 did not recur either**: `Test
(ubuntu-latest / luajit)` (102526965020) is green, so the leg that
produced zero test evidence in the previous run produced a full job
here.

**The one red is `Test (macos-latest / luajit)`, job 102526965146, with
two failing targets — and one of them is #258's own selector.**

```
---- a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join stdout ----
thread 'a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join' (120331) panicked at tests/statusline_segments_acceptance.rs:996:54:
called `Result::unwrap()` on an `Err` value: Io(Os { code: 35, kind: WouldBlock, message: "Resource temporarily unavailable" })
test result: FAILED. 10 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.83s
```

Selector, job leg, site and all three required fragments are **#258's**,
identical to its first occurrence in run 34220035122 and its second in
run 34272480226, down to the `10 passed; 1 failed` result line. **This
is #258's third occurrence, and its first on this branch** --- both
earlier ones are PR #257's. Not a rerun: a different tree.

```
---- daemon_reships_the_summary_after_a_real_buffer_round_trip stdout ----
thread 'daemon_reships_the_summary_after_a_real_buffer_round_trip' (122161) panicked at tests/theme_faces_acceptance.rs:1034:50:
read Hello: Io(Os { code: 35, kind: WouldBlock, message: "Resource temporarily unavailable" })
test result: FAILED. 26 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.42s
```

That is family member 5's **fourth** occurrence, and its second in
consecutive runs on this branch.

**The family is at eleven**, recomputed from the enumeration in the
section above plus these two. Still **five suites and six selectors** ---
both selectors are already in the list, so the count moves and the
spread does not:

| # | run | sha | suite | selector |
|---|---|---|---|---|
| 1–7 | (PR #257) | `8f6784f` … `d7fd465` | four suites | five selectors; enumerated above |
| 8 | 34358682895 | `65648ec` | `m11_5_semantic_acceptance` | `daemon_routes_semantic_family_to_semantic_session_only` |
| 9 | 34358682895 | `65648ec` | `theme_faces_acceptance` | `daemon_reships_the_summary_after_a_real_buffer_round_trip` |
| **10** | **34369540895** | **`e5417f6`** | **`statusline_segments_acceptance`** | **`a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join`** |
| **11** | **34369540895** | **`e5417f6`** | **`theme_faces_acceptance`** | **`daemon_reships_the_summary_after_a_real_buffer_round_trip`** |

**What this bears on, stated rather than decided here.**

- **#258 has now fired on `e2/gui-desktop-basics` under its own
  selector**, which no earlier run on this branch did. D30's revocation
  condition is written for `main` --- "if #258's selector recurs on
  `main` after the merge" --- and this is a `pull_request` run on a
  branch, so **the condition as written is not met by this**. Whether a
  recurrence on the *next* phase's head should carry the same weight is
  the owner's, and it is raised here because the disposition that
  accepted #258 rested on a Linux measurement whose macOS half is now
  taken and adverse.
- **This branch is a candidate for none of it and excluded from none of
  it.** `tests/theme_faces_acceptance.rs` is a file it edits;
  `tests/statusline_segments_acceptance.rs` is not, and nothing in E2 or
  in fix round 1 touches the daemon's boot path --- `src/daemon.rs` has
  no diff in `dbe40a1..e5417f6` at all. Fix round 1's own six commits
  are a moved test helper, a harness stage and its witness, two markdown
  files and one doc comment; none of them can reach a `Hello` read.
- **The measurement is the reading that fits.** On this leg boot to
  first `Hello` is p50 597.50 ms with 71 of 88 boots past the readiness
  wait's 500 ms, so a fixture whose budget starts at "connected" is
  racing a daemon that is not yet serving. Two fixtures losing that race
  in one job is what the measurement predicts, and it is what this job
  shows.

Both are a comment on #258 with the matching-rule difference stated:
target 1 **is** #258, target 2 is the family and not #258.

### PR #273's fix-round-2 head run 35111050634 at `523ce99`, and it is GREEN

E6c's post-owner-pass fix round: `a6687b1` plus one witness commit that
pins `foo(` undoing as one step through the production GPU dispatch (the
owner's window run read it as sometimes leaving `foo`; 800 samples
through the production dispatch removed `foo()` in one undo at every
inter-key cadence). Read on 2026-09-16 from the jobs endpoint and the
two macOS job logs after the run completed; not re-run.

| field | value |
|---|---|
| run | 35111050634, `pull_request`, one attempt |
| head | `523ce99`, `e6c/undo-across-peers` (base `7880c4b`) |
| window | created 2026-09-16T14:48:26Z, updated 2026-09-16T15:09:22Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35111050634-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 104844546313 | success |
| Format | 104844546082 | success |
| Changed paths | 104844546483 | success |
| Lint (luajit) | 104844546345 | success |
| Lint (lua54) | 104844547036 | success |
| M1 Acceptance Gates | 104844623732 | success |
| Test (crdt) | 104844623718 | success |
| M4 Perf Gates | 104844623774 | success |
| GPU Render (headless) | 104844623716 | success |
| M5 Perf Gates | 104844623665 | success |
| Test (ubuntu-latest / luajit, no crdt) | 104844623865 | success |
| Test (macos-latest / lua54) | 104844623770 | success |
| Test (ubuntu-latest / luajit) | 104844623918 | success |
| M6 Perf Gates | 104844623595 | success |
| Test (macos-latest / luajit) | 104844623755 | success |
| Test (ubuntu-latest / lua54) | 104844623736 | success |
| M10 Perf Gates (crdt) | 104844623646 | success |
| Perf budgets (debug) | 104844623565 | success |
| Docs consistency | 104844625249 | skipped |

Both macOS job logs (luajit 104844623755, lua54 104844623770) carry 145
`test result: ok` and zero `FAILED`, `WouldBlock` zero times and `did
not become ready` zero times; `undo_across_peers_acceptance` ran `19
passed` on both (the new `foo(` GPU-dispatch witness among them), and
U17's selector `read_dir_supersede_cancels_in_flight_predecessor` ran
`ok` on both --- a green sample after its eighth, the count staying at
eight. The base control is `7880c4b`, the last code-bearing commit on
`main`, run 35013609842, 18/1/0.
