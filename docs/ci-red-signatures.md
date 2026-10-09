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

### #256 at THREE, with no row until the owner rules on the archived rate

`process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`,
`live runtime probe`, all three local Linux under full-sweep load.

Tally (256): 3 items in the list below.

- gate log `20260908T104328Z-2589144`, `05-sweep`
- gate log `20260910T115801Z-321737`, `06-sweep` (passed in the same
  run's `sweep-luajit`)
- gate log `20261003T181411Z-1242042`, `05-sweep`, E7i review 1's gate at
  `fa176de` (PR #309), niced with `CARGO_BUILD_JOBS=6` beside the owner's
  own use (load 8.7 to 10.6 on sixteen threads): panicked at
  `src/process.rs:5155:53`, `2250 passed; 1 failed; 12 ignored` in the
  `--lib` target, the sweep's only red over 186 targets (5,177 passed, 1
  failed, 62 ignored); `src/process.rs` has no diff between `a013d46`
  and `fa176de`; commented on the issue, not re-run; the log is kept
  under `~/build/e7i-review1/gate-logs/`

The project's own archive
(`docs/archive/framings/ci-crdt-coverage-framing.md:622-629`) parked
this expression at "~1 in 5 under parallel full-suite load" with the
discriminator — a serial full-suite bite — never run, and as a product
defect hypothesis. Whether that archived rate counts toward a total is
the owner's call; until it is taken this file carries the three
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
CORRECTED at E7's fix round 1 (2026-09-18, review 1's Low 3): "#253
moves to three and #277 opens at one" is superseded by the section
below --- #277 is folded into #253 as one mechanism under three
selectors, and #253 is at five; the sentence stands where it was
written, as this file's correction form requires.

### #277 folded into #253: one mechanism under three selectors, and the opposite sign to U17

Filed at E7's opening as two issues under the fragment rule (the
`target snapshot` fragment at `gpu_invocation_acceptance.rs:487` on
one, `read initial BufferSnapshot` at two `read_initial_snapshot`
helpers on the other), the three reds of run 35270380496 are one
mechanism, and review 1 showed it from the two job logs: on the red
`Test (ubuntu-latest / lua54)` job (105367961691) against the same leg
of the head's green run (105391445243), the three red suites each
carry about the 5 s bound over their green time --- 7.56 vs 0.71 s,
6.90 vs 1.39 s, 17.94 vs 9.30 s --- while their neighbors run at their
usual speed (`compile_mode_acceptance` 39.76 vs 39.39 s) and the job
as a whole runs slower (649 vs 433 s summed over the 141 targets both
jobs name, a median per-target ratio of 1.36 over the 131 with a
nonzero green time; the review's own matching read 650 vs 465 s and
1.15, the same direction). Each is a freshly spawned daemon whose
first `BufferSnapshot` took longer than a 5 s read timeout sized for
an idle machine, on a runner instance that was slow throughout, and
`--test-threads=1` on that leg means nothing in the suite competed.
One mechanism, so one issue: #277 is closed as a duplicate on
2026-09-18 with the fold stated on both, #253 carries the two added
selectors and the `read initial BufferSnapshot` fragment, and the fix
#253 already names stands (print the elapsed against the bound at the
read sites, widen past ten times the observed, per D12).

**The sign, so the two rows are never collapsed.** U17's mechanism
was a worker finishing *too fast* for a synchronous flip --- the
predecessor complete before the cancel took effect, `got ok` where
`cancelled` was expected, a race a serialized or idle runner makes
*easier* to lose. #253's is a daemon starting *too slowly* for a
fixed bound --- a wait that runs out, a race a loaded or slow runner
makes easier to lose. Opposite signs: load or slowness argues for
#253 and against U17, and a red of either is not evidence about the
other. U17 is closed causally above and #253 is open; a future red
that fits "slow runner, bounded read" belongs here, and one that fits
"fast worker, synchronous flip" reopens U17's question and nothing
else.

#253's occurrences, counted one per test target per run as its first
two were (five rows of one target in one run counted once each):

Tally (253): 6 items in the list below.

- local, 2026-09-06, a run of the suite during the daemon-reaper work: `gpu_initial_target_acceptance`, the five `gpu_invocation_acceptance::crdt::` rows, `target snapshot: … WouldBlock` at `:487` and `raw target result: … WouldBlock` at `:542` (the issue's first occurrence)
- local, gate `20260906T104039Z-54318`, `05-sweep.log`: the same target, the same five rows, `14 passed; 5 failed`, the sweep's only red target; the rerun `20260906T104738Z-152565` green (non-reproduction)
- `main` at `7002308`, run 35270380496, `Test (ubuntu-latest / lua54)` (105367961691): `gpu_initial_target_acceptance`, `gpu_invocation_acceptance::crdt::dedup_upgrade_publishes_the_snapshot_to_preexisting_grid_replicas`, `target snapshot: Io(Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" })` at `tests/gpu_invocation_acceptance.rs:487`, `20 passed; 1 failed` in 17.94 s at 20:36:55Z --- the first on CI, one row of the five
- the same run and job: `compile_mode_crdt_acceptance` `compile_run_converges_and_replica_edit_triggers_recovery`, `read initial BufferSnapshot: Io(Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" })` at `tests/compile_mode_crdt_acceptance.rs:33`, `7 passed; 1 failed` in 7.56 s at 20:35:12Z --- filed on #277, folded here
- the same run and job: `e6_review1_gpu_route_probes` `gpu_route_plain_typing_undoes_through_the_daemon_arbiter`, the same fragment at `tests/e6_review1_gpu_route_probes.rs:35`, `5 passed; 1 failed` in 6.90 s at 20:35:36Z --- filed on #277, folded here
- PR #284 at `1847805`, run 35455311600, `Test (crdt)` (105929452599): `undo_across_peers_acceptance` `a_command_after_optimistic_typing_is_its_own_undo_step`, `read initial BufferSnapshot: Io(Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" })` at `tests/undo_across_peers_acceptance.rs:34`, `18 passed; 1 failed` in 15.17 s at 16:50:23Z --- a fourth selector under the same fragment (E7c's close, 2026-09-19)

Tally (253-fold): 5 = 3 + 2.

The signature #253 now matches: selector any of the three suites'
rows above; job any CI test leg or the local sweep; required fragment
`WouldBlock` on the first snapshot read after an attach, spelled
`target snapshot: …`, `raw target result: …` or `read initial
BufferSnapshot: …`. A `WouldBlock` on a `Hello` read is the `read
Hello` family, not this; a `late.sock` refusal with `m5_5` is #271.

### PR #278's head run 35277461945 at `0c6f8ff`, and it is GREEN

E7's delivered tip (`e7/git-and-lsp-affordances`, base `7002308`),
read at C7's close on 2026-09-17 from the jobs endpoint and three test
legs' logs after the run had completed; not re-run.

| field | value |
|---|---|
| run | 35277461945, `pull_request`, one attempt |
| head | `0c6f8ff`, PR #278's opening head |
| window | created 2026-09-17T21:34:46Z, updated 21:56:35Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35277461945-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Format | 105391401664 | success |
| Changed paths | 105391401949 | success |
| Lint (lua54) | 105391402005 | success |
| Lint (luajit) | 105391402067 | success |
| Commit attribution (D9) | 105391402149 | success |
| GPU Render (headless) | 105391445115 | success |
| M1 Acceptance Gates | 105391445158 | success |
| Test (crdt) | 105391445175 | success |
| Test (macos-latest / luajit) | 105391445182 | success |
| M4 Perf Gates | 105391445217 | success |
| Test (ubuntu-latest / luajit) | 105391445218 | success |
| Perf budgets (debug) | 105391445222 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105391445232 | success |
| Test (ubuntu-latest / lua54) | 105391445243 | success |
| M5 Perf Gates | 105391445253 | success |
| M10 Perf Gates (crdt) | 105391445256 | success |
| Test (macos-latest / lua54) | 105391445290 | success |
| M6 Perf Gates | 105391445326 | success |
| Docs consistency | 105391446634 | skipped |

On both macOS legs (`Test (macos-latest / luajit)` 105391445182,
`Test (macos-latest / lua54)` 105391445290) `WouldBlock` appears zero
times and `did not become ready` zero times, against 155 `test result:
ok` and zero `FAILED` in each (the four E7 suites are the four targets
over the 151 of `main`'s last green run); on `Test (ubuntu-latest /
lua54)` (105391445243), the leg the base control was red on, 154 ok,
zero `FAILED`, `WouldBlock` zero --- the three first-snapshot reads of
#253 and #277 did not recur here, which is a green sample and nothing
more. U17's witness ran `ok` on all three legs. So neither the `read
Hello` family, nor #259, nor U4, nor #271, nor #276, nor #253, nor
#277 sampled on this run; every count stands where the section above
left it. **The head is green**, stated at the moment of writing.

### PR #278's review-round-1 runs at `4717415` and `024b548`: red by design on the review's own probe

Review round 1 (2026-09-18) committed its six behavioral probes as
`4717415` (`tests/e7_review1_probes.rs`) and gated one helper as
`024b548`; the suite's prune probe was red until the fix round's line
landed, so both runs are red on the branch's own row and on nothing
else. Read at the fix round's close from the jobs endpoint and the
review's copies of the twelve test-leg logs; not re-run.

| field | value |
|---|---|
| run | 35342121854, `pull_request`, one attempt |
| head | `4717415`, the review's witness commit |
| window | created 2026-09-18T11:56:47Z, updated 12:12:38Z |
| verdict | 19 jobs: **12 success, 1 skipped, 6 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the failures | the four ubuntu legs on `e7_review1_probes` `10 passed; 1 failed` (9 and 1 without `crdt`), the prune probe alone, with 48, 152, 152 and 152 `test result: ok` beside it; both macOS legs red at compile (`errors_text` a dead helper under `-D warnings`, its only reader the Linux-only killed-server probe), zero tests run, 417 and 410 log lines |

Tally (run-35342121854-jobs): 19 = 12 + 1 + 6.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 105590178946 | success |
| Format | 105590179105 | success |
| Lint (lua54) | 105590179119 | success |
| Lint (luajit) | 105590179158 | success |
| Changed paths | 105590179186 | success |
| Test (crdt) | 105590219321 | failure |
| GPU Render (headless) | 105590219350 | success |
| M6 Perf Gates | 105590219370 | success |
| M1 Acceptance Gates | 105590219396 | success |
| Test (ubuntu-latest / lua54) | 105590219416 | failure |
| Test (ubuntu-latest / luajit) | 105590219417 | failure |
| M10 Perf Gates (crdt) | 105590219435 | success |
| M5 Perf Gates | 105590219443 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105590219460 | failure |
| Test (macos-latest / luajit) | 105590219480 | failure |
| M4 Perf Gates | 105590219496 | success |
| Test (macos-latest / lua54) | 105590219511 | failure |
| Perf budgets (debug) | 105590219518 | success |
| Docs consistency | 105590221010 | skipped |

| field | value |
|---|---|
| run | 35343712597, `pull_request`, one attempt |
| head | `024b548`, the helper gated with its consumer |
| window | created 2026-09-18T12:15:31Z, updated 12:36:17Z |
| verdict | 19 jobs: **12 success, 1 skipped, 6 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the failures | every test leg on `e7_review1_probes` `10 passed; 1 failed` (9 and 1 where a probe is compiled out), `e7_review1_the_prune_misleads_an_older_completion_carry` alone, with 48, 152, 152, 152, 153 and 153 `test result: ok` beside it; the macOS legs now compiling and running the suite |

Tally (run-35343712597-jobs): 19 = 12 + 1 + 6.

| job | id | result |
|---|---|---|
| Format | 105595264786 | success |
| Commit attribution (D9) | 105595264900 | success |
| Lint (luajit) | 105595265009 | success |
| Changed paths | 105595265085 | success |
| Lint (lua54) | 105595265122 | success |
| Test (crdt) | 105595305219 | failure |
| M4 Perf Gates | 105595305263 | success |
| GPU Render (headless) | 105595305271 | success |
| M6 Perf Gates | 105595305274 | success |
| M1 Acceptance Gates | 105595305283 | success |
| Test (macos-latest / luajit) | 105595305312 | failure |
| Test (ubuntu-latest / lua54) | 105595305350 | failure |
| M10 Perf Gates (crdt) | 105595305391 | success |
| Test (ubuntu-latest / luajit) | 105595305437 | failure |
| Perf budgets (debug) | 105595305469 | success |
| Test (macos-latest / lua54) | 105595305491 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 105595305509 | failure |
| M5 Perf Gates | 105595305616 | success |
| Docs consistency | 105595306518 | skipped |

`WouldBlock`, `got ok` and `did not become ready` appear zero times
in all twelve logs; U17's witness ran `ok` on the five legs of the
second run that reached it. The 48 on `Test (crdt)` in both runs is
that job stopping at `e7_review1_probes`, the 49th target, because it
ran without `--no-fail-fast` (`ci.yml:536` at these heads; the matrix
legs have carried the flag since E0): every target sorted after the
probe suite, `m8_1_acceptance` among them, ran nothing and said
nothing there. The fix round adds the flag (`c68409d`, review 1's
Low 4) and the next section shows the job at its whole corpus. So
neither the `read Hello` family, nor #259, nor U4, nor #271, nor
#276, nor #253 sampled on either run; every count stands. Both heads
are not green, on the review's probe and by design, stated at the
moment of writing; the line that turns it green is the fix round's.

### E7h's PR runs and local samples, recorded at its second fix round

**Why these were late.** E7h recorded the runs of PR #297 and the intermittents it met in the PR body and in its passes instead of here, and its first fix round said so as a choice ("E7h recorded its PR heads' runs in the PR body and the passes, and this round follows it"). Every phase from E1 to E7d had recorded its PR runs on `main`; E7e and E7g stopped recording the green ones, and E7h was the first to leave out red runs and local samples too. So by E7h's review 2 (2026-10-01) the counts of record here were stale while the bodies were right: #283 at six against seven, #291 at one against five, #298 and #300 without a word. A PR body is mutable and read once; this file is what the next phase reads first and what a row's disposition is decided from. **An occurrence goes here when it is seen**, by a commit on `main`, and the PR body and the passes cite this file, not the other way round. Recorded 2026-10-01 at E7h's fix round 2, each run read again from the jobs endpoint and each sample from its issue comment and gate log.

The eight runs PR #297 had before fix round 2, each `pull_request` at attempt 1 of 1 (`CI` 19 jobs, `Grammar fuzz` one):

Tally (pr297-runs): 8 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 36648396729 | CI | `7e3af6e` | failure |
| 36648396732 | Grammar fuzz | `7e3af6e` | success |
| 36653190278 | CI | `569e21e` | cancelled |
| 36653190348 | Grammar fuzz | `569e21e` | failure |
| 36653672405 | CI | `0a85d18` | success |
| 36653672430 | Grammar fuzz | `0a85d18` | success |
| 36761534199 | CI | `17c3c8f` | failure |
| 36761534142 | Grammar fuzz | `17c3c8f` | success |

#### `CI` 36648396729 at `7e3af6e`: the branch's own growth row on both macOS legs

Created 2026-09-30T00:03:37Z, updated 00:29:32Z. 19 jobs: **16 success, 1 skipped, 2 failures**; read from the jobs endpoint.

Tally (run-36648396729-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 109676627864 | success |
| Format | 109676628019 | success |
| Lint (lua54) | 109676628042 | success |
| Changed paths | 109676628136 | success |
| Lint (luajit) | 109676628142 | success |
| M10 Perf Gates (crdt) | 109676687198 | success |
| Perf budgets (debug) | 109676687208 | success |
| M4 Perf Gates | 109676687226 | success |
| Test (crdt) | 109676687227 | success |
| GPU Render (headless) | 109676687235 | success |
| M1 Acceptance Gates | 109676687254 | success |
| M5 Perf Gates | 109676687293 | success |
| Test (ubuntu-latest / luajit) | 109676687303 | success |
| M6 Perf Gates | 109676687311 | success |
| Test (macos-latest / luajit) | 109676687320 | failure |
| Test (ubuntu-latest / lua54) | 109676687326 | success |
| Test (ubuntu-latest / luajit, no crdt) | 109676687333 | success |
| Test (macos-latest / lua54) | 109676687375 | failure |
| Docs consistency | 109676688970 | skipped |

Tally (run-36648396729-failures): 2 rows of the table above with `result` = `failure`.

Both failures are `e7h_grammar_gate_acceptance::e7h_a_parse_that_grows_without_returning_is_a_hang_not_an_allocation` (`tests/e7h_grammar_gate_acceptance.rs:535`), each leg `173` `test result: ok` and one `FAILED`: the row held the growth plant's memory signature, and on macOS the harness reads no memory (no `/proc`), so the hang limit caught the parse instead. The branch's own row, fixed in `569e21e`; no registered row.

#### `CI` 36653190278 at `569e21e`: cancelled by the next push

Created 01:02:23Z, updated 01:10:06Z. 19 jobs: **12 success, 1 skipped, 6 cancelled**; the six test legs were cancelled when `0a85d18` was pushed (the workflow's `concurrency` group); nothing ran to a verdict there.

Tally (run-36653190278-jobs): 19 = 12 + 1 + 6.

| job | id | result |
|---|---|---|
| Format | 109691748814 | success |
| Changed paths | 109691748987 | success |
| Commit attribution (D9) | 109691749032 | success |
| Lint (luajit) | 109691749049 | success |
| Lint (lua54) | 109691749058 | success |
| Test (crdt) | 109691794917 | cancelled |
| M4 Perf Gates | 109691794964 | success |
| GPU Render (headless) | 109691794972 | success |
| M10 Perf Gates (crdt) | 109691794975 | success |
| M1 Acceptance Gates | 109691795010 | success |
| M6 Perf Gates | 109691795032 | success |
| Test (macos-latest / luajit) | 109691795047 | cancelled |
| Test (ubuntu-latest / luajit, no crdt) | 109691795051 | cancelled |
| Perf budgets (debug) | 109691795059 | success |
| M5 Perf Gates | 109691795091 | success |
| Test (macos-latest / lua54) | 109691795106 | cancelled |
| Test (ubuntu-latest / luajit) | 109691795112 | cancelled |
| Test (ubuntu-latest / lua54) | 109691795203 | cancelled |
| Docs consistency | 109691796379 | skipped |

Tally (run-36653190278-cancelled): 6 rows of the table above with `result` = `cancelled`.

#### `CI` 36653672405 at `0a85d18`, and it is GREEN

Created 01:08:20Z, updated 01:34:07Z. 19 jobs: **18 success, 1 skipped, 0 failures**; the skip `Docs consistency`, correctly. The PR body's leg counts: `Test (crdt)` 174 `test result: ok`, both macOS legs 176, the three ubuntu legs 175 each.

Tally (run-36653672405-jobs): 19 = 18 + 1 + 0.

#### `CI` 36761534199 at `17c3c8f`: #283's seventh

Created 18:51:14Z, updated 19:20:51Z. 19 jobs: **17 success, 1 skipped, 1 failure**; read from the jobs endpoint.

Tally (run-36761534199-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Format | 110045042048 | success |
| Lint (luajit) | 110045042260 | success |
| Commit attribution (D9) | 110045042413 | success |
| Changed paths | 110045042506 | success |
| Lint (lua54) | 110045042979 | success |
| M4 Perf Gates | 110045110064 | success |
| GPU Render (headless) | 110045110104 | success |
| Test (crdt) | 110045110109 | success |
| Test (ubuntu-latest / lua54) | 110045110183 | success |
| M1 Acceptance Gates | 110045110199 | success |
| Perf budgets (debug) | 110045110221 | success |
| M6 Perf Gates | 110045110253 | success |
| Test (macos-latest / luajit) | 110045110267 | success |
| Test (ubuntu-latest / luajit, no crdt) | 110045110270 | success |
| Test (macos-latest / lua54) | 110045110295 | failure |
| Test (ubuntu-latest / luajit) | 110045110303 | success |
| M10 Perf Gates (crdt) | 110045110356 | success |
| M5 Perf Gates | 110045110365 | success |
| Docs consistency | 110045111302 | skipped |

Tally (run-36761534199-failures): 1 row of the table above with `result` = `failure`.

`Test (macos-latest / lua54)` (110045110295): `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`, `pump timeout waiting for the accept and its import`, the mirror's `popup_rows=0 anchor=None`, `9 passed; 1 failed; 0 ignored`: all three of #283's fragments, **#283's seventh occurrence**, on the leg of its first, third and fourth; 174 `ok` and that one on the leg; the other five legs `ok`, the row among them. Off the branch's path; commented on the issue; not rerun.

#### The four `Grammar fuzz` runs

Each one job, `Grammar fuzz (every grammar, ASan)` before `a830270` renamed it `Grammar fuzz`:

Tally (pr297-fuzz): 4 items in the list below.

- 36648396732 at `7e3af6e`, 00:03:37Z to 01:00:43Z: success; 22 grammars, six of them 600 s by `changed --since`; no crash and no reproduced hang.
- 36653190348 at `569e21e`, 01:02:23Z to 01:03:50Z: failure, before fuzzing anything: the check that every grammar crate was compiled under the sanitizers read a wrapper log a restored cache did not carry. The branch's own; fixed in `0a85d18`.
- 36653672430 at `0a85d18`, 01:08:20Z to 02:02:55Z: success; six grammars 600 s, no crash or reproduced hang.
- 36761534142 at `17c3c8f`, 18:51:14Z to 19:57:29Z: success; eight grammars 600 s, no crash, reproduced hang or memory cut; two markdown_inline hangs not reproduced alone (E7h review 2 later reproduced the class on the laptop, #301).

#### #283, fourteen occurrences

#283 is `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import` on a macOS leg with all three fragments (`pump timeout waiting for the accept and its import`, `popup_rows=0 anchor=None`, `9 passed; 1 failed`). Each occurrence, one run and job:

Tally (283): 14 items in the list below.

- run 35437135435 at `361bb3b` (`main` after E7b), job 105881622751, `Test (macos-latest / lua54)`;
- run 35455311600 at `1847805` (PR #284), job 105929452757, `Test (macos-latest / luajit)`;
- run 35474631618 at `09798eb` (PR #284), job 105981654430, `Test (macos-latest / lua54)`;
- run 35504020999 at `0a287d6` (PR #284), job 106060744338, `Test (macos-latest / lua54)`;
- run 35505799659 at `9eb89c1` (PR #284), job 106065308768, `Test (macos-latest / luajit)`;
- run 35507517448 at `3de1e1f` (PR #284), job 106069747083, `Test (macos-latest / luajit)`;
- run 36761534199 at `17c3c8f` (PR #297), job 110045110295, `Test (macos-latest / lua54)`;
- run 36942304451 at `b9d00fe` (PR #297), job 110636448104, `Test (macos-latest / lua54)` (recorded at E7h's fix round 3, below);
- run 37080922545 at `2d4c848` (PR #309), job 111081065656, `Test (macos-latest / lua54)` (recorded at C7i, below);
- run 37161567765 at `c540a68` (PR #309), job 111315929548, `Test (macos-latest / luajit)` (recorded at E7i's fix round 1, below);
- run 37203840409 at `db697a2` (PR #309), job 111440850223, `Test (macos-latest / lua54)` (recorded at E7i's fix round 2, below);
- run 37451551936 at `d2de579` (PR #315), job 112229134676, `Test (macos-latest / lua54)` (recorded at E8's fix round 2, below);
- run 37451551936 at `d2de579` (PR #315), job 112229134735, `Test (macos-latest / luajit)`, the same run's other macOS leg: the first run with both red on it (recorded at E8's fix round 2, below);
- run 37451551936 at `d2de579` (PR #315), **attempt 2**, job 112253935087, `Test (macos-latest / luajit)`, the same merge as attempt 1 rerun on the owner's instruction: red on the row again on this leg, while the lua54 leg passed it (recorded at E8's fix round 3, below).

#283 is at fourteen (thirteen until E8's fix round 3, eleven until E8's fix round 2, ten until E7i's fix round 2); the list above is the one the count is read from.

#### #291, sixteen occurrences, all local

`e7e_haskell_acceptance::e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error`, armed by `PMACS_REQUIRE_HLS` and run only on this laptop, with the three fragments `HLS reports the type error; store []`, a trace reaching `LSP:ready·setting` and never `processi`, and `4 passed; 1 failed; 0 ignored`. Each occurrence:

Tally (291): 16 items in the list below.

- 2026-09-27, `scripts/gate` on `e7g/grammar-fuzz` at `549d1c5`, `20260927T124215Z-1010709` (the first, recorded above);
- 2026-09-29, a hand run of the touched suites under `--no-default-features --features lua54,crdt` on `e7h/grammar-gate` at `7f98c03`, beside a niced fuzz run holding four cores, 135.22 s;
- 2026-09-30, a hand run in the gate's environment on `e7h/grammar-gate` at `1036723` with fix round 1's first change uncommitted, beside a niced GCC 16 compile matrix, 128.21 s;
- 2026-09-30, `scripts/gate` at `17c3c8f`, `20260930T185138Z-270502`, beside three niced fuzz runs (load 9 to 20), 128.61 s;
- 2026-09-30, `scripts/gate` at `17c3c8f`, `20260930T202751Z-465064`, the whole gate niced beside a game, the handshake 17.7 s, 141.18 s;
- 2026-10-02, `scripts/gate` at `5daf85d`, `20261002T012705Z-3618489`, on a nearly idle machine (load about 1.5 at its start, nothing of E7h's beside it), 125.52 s (recorded at E7h's fix round 3, below);
- 2026-10-03, `scripts/gate` on `e7i/parse-containment` at `2d4c848`, `20261002T233722Z-2508081`, 125.51 s;
- 2026-10-03, `scripts/gate` at `0e2998b`, `20261003T004422Z-2872349` (recorded at C7i, below, with a base control: `a013d46` 5 of 8, `0e2998b` 2 of 8);
- 2026-10-03, `scripts/gate` on `e7i/parse-containment` at `45186c9`, `20261003T150924Z-395472`, the owner gaming beside it (load 6 to 78), 129.81 s (recorded at the E7i build, below);
- 2026-10-04, `scripts/gate` on `e7i/parse-containment` at `12c3a6c`, `20261003T230816Z-2486117`, 125.43 s (recorded at E7i's fix round 1, below);
- 2026-10-04, `scripts/gate` on `e7i/parse-containment` at `b1c36d7`, `20261004T170334Z-438134`, 126.00 s, with #311's condition holding on the laptop (recorded at E7i's fix round 3, below).
- 2026-10-04, `scripts/gate --protocol` on `e8/hover-and-signature` at `4fc5599`, `20261004T202450Z-1217913`, step `06-sweep`, 126.55 s, the trace reaching `LSP:ready·setting` and never `processi`; the same gate's `07-sweep-luajit` failed the row with #311's fragments instead (recorded at E8's checkpoint, below).
- 2026-10-05, `scripts/gate --protocol` on `e8/hover-and-signature` at `1ace8b3` (the tip), `20261004T234355Z-3389009`, step `06-sweep`, 126.71 s; that gate's `07-sweep-luajit` failed the row with #311's fragments (recorded at E8's checkpoint, below).
- 2026-10-05, `scripts/gate --protocol` at `1ace8b3`, E8 review 1's gate `20261005T081704Z-3789356`, step `07-sweep-luajit`, 126.03 s, `store []`, the trace reaching `LSP:ready·setting` and never `processi`; that gate's `06-sweep` failed the row with #311's fragments (recorded at E8's fix round 1, below).
- 2026-10-09, `scripts/gate --protocol` at `2ad0b6c`, the pre-release PR's fix round 2 gate `20261009T122310Z-3757634`, step `06-sweep`, 126.47 s, `store []`, the trace reaching `LSP:ready·setting` and never `processi`; that gate's `07-sweep-luajit` failed the row with #311's fragments (recorded with that round, below).
- 2026-10-09, `scripts/gate --protocol` at `29b5b81`, the same round's tip gate `20261009T135200Z-4043146`, step `07-sweep-luajit`, 126.46 s, `store []`, the trace reaching `LSP:ready·setting` and never `processi`; that gate's `06-sweep` failed the row with #311's fragments (recorded with that round, below).

#291 is at sixteen; each is commented on the issue. Until E7h's third fix round this paragraph attributed it to load, as "four of them E7h's and all four beside heavy load": niced fuzz runs, a compile matrix, a game. **The sixth falsifies that.** `20261002T012705Z-3618489` began on a nearly idle machine (load about 1.5, rising to 3 to 10 as other sessions resumed) with nothing of the round's beside it, and the row failed on all three fragments. Nor did the attribution hold before it. The first sample's alone runs in the gate's environment, eight and then sixteen interleaved pairs, failed four times in forty with no load recorded, and the third occurrence came at a load of about 4 on sixteen threads. Load is not this row's cause, and this file attributes it none. Its trace shows haskell-language-server finishing its session setup and never processing the typed edit, which does not separate the server from pmacs's side of the exchange. The row passed in E7h review 2's two gates on a quieter machine (`20261001T083032Z-718803` at `17c3c8f` and `20261001T155727Z-1465534` at `d1cbca6`), which is non-reproduction and nothing more.

#### #298, first sample, local

`scripts/gate` on `e7h/review-1` at `7f0f223` (PR #297's `0a85d18` plus two files under `tests/`), `20260930T131808Z-2237827`, step `05-sweep`: `e7c_didsave_acceptance::e7c_4_a_server_message_reaches_the_log_and_the_status_line`, `the warning is on the status line: ""` (`tests/e7c_didsave_acceptance.rs:630:5`), `8 passed; 1 failed; 0 ignored` in 90.78 s, four of the suite's other rows past 60 s. No row matched; filed by E7h review 1 as **#298**. #298 is at one.

#### #300, first sample, local

`scripts/gate` at `17c3c8f`, `20260930T202751Z-465064` (the run of #291's fifth), the whole gate niced beside a game, load about 10: `--lib process::tests::shutdown_still_force_kills_a_group_a_failed_escalation_marked_killed`, `precondition: the entry is marked killed by a SIGKILL that failed`, `left: None` / `right: Some(true)`, `test result: FAILED. 2250 passed; 1 failed; 12 ignored`. No row matched; filed by E7h's fix round 1 as **#300**; the row alone at the same tip passed five times in five. #300 is at one.

So, across E7h: neither the `read Hello` family, nor #259, nor U4, nor U17, nor #271, nor #276, nor #253, nor #282, nor #288 sampled; #283 moves to seven and #291 to five; #298 and #300 are at one each.

### E7h's second fix round: PR #297's runs, `main` at `5359176`, and the gate's samples

Recorded 2026-10-01 by the round that wrote the rule above, under that rule. Each run was read from the jobs endpoint at attempt 1. The PR's runs are `pull_request` runs of the merge commit with `main`.

**`main` at `5359176`**, the registry commit above: `CI` run 36899885972, `push`.

Tally (run-36899885972-jobs): 15 = 5 + 1 + 9.

Five success, one failure, nine skipped; the skips are correct for a docs-only commit. The failure is `Commit attribution (D9)`, on that commit's own message: "53591762… carries a trailer block, and commits carry no trailers". Its final paragraph's second line began with `#291's`, which git's trailer parser skips as a comment, so the lone `Validation:` line read as a trailer. `main` is not rewritten. Later pushes check only their own range, but the whole-history fallback will fail on it until the owner names an exception or moves the trailers epoch. This is no intermittent and no registered row: the commit's author's defect, recorded here because this file is where a red on `main` is accounted for.

**PR #297's runs at the round's heads:**

Tally (pr297-round2-runs): 20 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 36899274977 | CI | `cbcff4b` | failure |
| 36899274860 | Grammar fuzz | `cbcff4b` | cancelled |
| 36906446130 | CI | `a213a99` | cancelled |
| 36906446229 | Grammar fuzz | `a213a99` | cancelled |
| 36907198504 | CI | `792cdfa` | cancelled |
| 36907198549 | Grammar fuzz | `792cdfa` | cancelled |
| 36909344264 | CI | `90605f8` | cancelled |
| 36909344319 | Grammar fuzz | `90605f8` | cancelled |
| 36909902610 | CI | `02fe7af` | cancelled |
| 36909903029 | Grammar fuzz | `02fe7af` | cancelled |
| 36910548493 | CI | `f1819a6` | cancelled |
| 36910548872 | Grammar fuzz | `f1819a6` | cancelled |
| 36911490396 | CI | `23c7b1f` | cancelled |
| 36911490387 | Grammar fuzz | `23c7b1f` | cancelled |
| 36914436990 | CI | `c1fd1ed` | cancelled |
| 36914437009 | Grammar fuzz | `c1fd1ed` | cancelled |
| 36917782629 | CI | `38c9287` | failure |
| 36917782552 | Grammar fuzz | `38c9287` | failure |
| 36927231233 | CI | `4c521c7` | failure |
| 36927231200 | Grammar fuzz | `4c521c7` | success |

Tally (pr297-round2-cancelled): 15 rows of the table above with `verdict` = `cancelled`.

Each cancelled run was cancelled by the next push, through the workflows' `concurrency` groups. Nothing in it ran to a verdict that the next run did not repeat.

#### `CI` 36899274977 at `cbcff4b`: the round's own red, on every test leg

Tally (run-36899274977-jobs): 19 = 12 + 1 + 6.

Twelve success, `Docs consistency` skipped, and all six test legs failed on the same rows:
- `e7g_review1_every_scanner_on_the_aliasing_array_header_is_recorded_and_built_without_strict_aliasing`;
- `e7h_review1_the_crates_relying_on_the_flag_are_the_recorded_residual`.

Each failed on `cargo metadata --offline --locked`: "failed to download `android-activity v0.6.1` … --offline was specified". The rows passed at `17c3c8f`. Unfiltered, cargo metadata needs every locked crate's source, and the runner's restored registry held only the host's. That is the branch's own fragility, not an intermittent of the product. `c1fd1ed` passes `--filter-platform` with the host, and on the laptop a host-only `CARGO_HOME` reproduces the failure and the fix. At `23c7b1f` (`CI` 36911490396, cancelled by the next push after two legs had finished, both failed) the same failure took four rows on the crdt leg, the round having added two callers. The no-crdt leg failed those four and the row below.

#### Two cancelled runs that had already failed: the round's own rows

`CI` 36911490396 at `23c7b1f` and 36914436990 at `c1fd1ed` were cancelled by the next push, but some test legs had finished first, and they are recorded here because they failed.

Tally (run-36911490396-jobs): 19 = 12 + 1 + 2 + 4.

Tally (run-36914436990-jobs): 19 = 14 + 1 + 3 + 1.

The terms are success, skipped, failure, cancelled.
- At `23c7b1f`, `Test (crdt)` failed the four metadata rows. `Test (ubuntu-latest / luajit, no crdt)` failed those and `e7h2_a_parse_whose_memory_grows_with_its_input_is_judged_on_the_input_as_found`.
- At `c1fd1ed`, the metadata fix in, `Test (crdt)`, `Test (ubuntu-latest / luajit, no crdt)` and `Test (ubuntu-latest / luajit)` failed that row alone. Its note printed "its minimum (20 bytes), alone: returned in 53 ms", where the row required 14 bytes. Both macOS legs passed; `Test (ubuntu-latest / lua54)` was cancelled.

The row passed on `23c7b1f`'s crdt leg, so the minimum's size is the runner's: the minimizer's oracle is a 50 ms RSS poll. `38c9287` holds the row to its claim. Both are the branch's own defects, fixed on the branch. No registered row and no intermittent of the product.

#### `CI` 36917782629 at `38c9287`: #305, a first sample

Tally (run-36917782629-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure: `Test (macos-latest / luajit)`, job 110556464685, its only failure against 175 `test result: ok`. The failing row is `e7c_indicator_acceptance::e7c_fix_3_typing_after_a_save_moves_nothing_on_the_mode_line`, "no request outlived the threshold, so no indicator at 3861 ms" with "⋯1 parse rust" on the activity indicator. That is the row #288 names, but not #288's fragment (`the check's diagnostic landed`), so it was filed as **#305**, first occurrence.

The branch does touch the parse path the indicator reports on (`run_parse`'s deadline, E7h.2), so this sample is not one the branch is clear of. The other five legs ran it `ok`. The round's two CI fixes held there: the four metadata rows and the scaled row ran `ok` on `Test (crdt)` and `Test (ubuntu-latest / luajit)`.

#### `Grammar fuzz` 36917782552 at `38c9287`: the new leg's first full run, red on the harness

Tally (run-36917782552-jobs): 2 = 1 + 1.

`Grammar fuzz` (the `ubsan` leg) succeeded, 66.1 min. `Grammar fuzz (asan-strict)` failed, 62.9 min, on bash: two `hang`s, "never returned, growing past 1024 MB", whose input as found did not return in 120 s and whose minimum returned in 2.6 s. That is a large slow input the harness's time boundary should have filed slow, and it did not because the first parse had met the memory limit. The round's own defect, fixed in `4c521c7`; no registered row.

#### `CI` 36927231233 at `4c521c7`, the round's head: #306, a first sample

Tally (run-36927231233-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure: `Test (macos-latest / lua54)`, job 110587518878, against 175 `test result: ok`. The row is `compile_mode_acceptance::acc05_kill_reaps_backgrounded_descendant`, "kill must produce a signaled exit marker; buffer:", with the shell's "Terminated: 15 sleep 30" in the buffer and no `[compile killed by` marker within 10 s. It has no registered row and touches nothing the branch changes, so it was filed as **#306**, first occurrence; not rerun.

Every leg's `running` lines pair one to one with its result lines (176, 176, 177, 177, 177 and 178), and `WouldBlock`, `did not become ready` and `got ok` appear zero times. The round's six `e7h2_` rows ran `ok` on every leg.

#### `Grammar fuzz` 36927231200 at `4c521c7`: both legs green

Both legs succeeded:
- `Grammar fuzz` (job 110587446735), 21:13:45–22:20:31Z;
- `Grammar fuzz (asan-strict)` (job 110587447401), 21:13:46–22:20:06Z.

Each printed `image: ubuntu24 20260927.320.1` and GCC 13.3.0. The decision step said `run=true` (`.cargo/config.toml changed`). No crash and no reproduced hang in any grammar on either leg. The `asan-strict` leg filed bash's 28,272-byte minimum, the input it had failed as a hang at `38c9287`, as slow.

#### Local samples in the round's gates

- `20261001T190159Z-1887994` at `f1819a6`: stopped by its author at the doc stage (exit 143) to commit the tripwire first. Not a result.
- `20261001T190344Z-1908417` at `23c7b1f`, beside a niced 600 s fuzz arm, a tripwire run and a niced test build, load 25 to 30: five of six; the sweep 179 targets, 5,140 passed, 2 failed, 62 ignored. Its two reds are first samples, filed:
  - **#303**, `git_status_stage1_acceptance::g6_4_d_answers_every_row_class`, "the status panel must render; status was "git: rev-parse returned no worktree root"";
  - **#304**, `pmacs-gpu`'s `caret_inside_a_span_shows_source_exactly_as_if_math_were_disabled`, the frames differing by a caret.

  #291's, #298's and #300's rows ran `ok`.
- `20261001T192607Z-2093589` at `c1fd1ed`, beside the 600 s arm's last two niced workers: five of six; the sweep 179 targets, 5,140 passed, 2 failed, 62 ignored, both in `pmacs-gpu`:
  - **#304's second occurrence**, the same fragments to the pixel;
  - `tests::overwide_status_runs_never_wrap_and_keep_the_suffix_pinned`, "assertion failed: min_y >= band_top && max_y <= height", recorded on #304 as the same candidate mechanism (two frames compared across a caret blink), not filed apart.

  #291's, #298's, #300's and #303's rows ran `ok`.
- `20261001T195235Z-2267767` at `01c8c75`: stopped by its author at the doc stage (exit 143) when CI showed the scaled row's defect. Not a result.
- `20261001T195503Z-2281145` at `38c9287`, the round's tip, beside other sessions' load and none of the round's own fuzzing: **six of six**, 179 targets, 5,142 passed, 0 failed, 62 ignored; every intermittent's row above ran `ok`.

So, at E7h's second fix round: #303 at one, #304 at two, #305 and #306 at one each; #283, #291, #298 and #300 not sampled, and every count above stands.

### E7h's third fix round: PR #297's runs, #283's eighth, #291's sixth, #304's third and #307's first

Recorded 2026-10-02 under the rule above, each run read from the jobs endpoint at attempt 1. The PR's runs are `pull_request` runs of the merge commit with `main`.

Tally (pr297-round3-runs): 12 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 36936488505 | CI | `4c0a435` | cancelled |
| 36936488672 | Grammar fuzz | `4c0a435` | cancelled |
| 36936860604 | CI | `3dbcc91` | cancelled |
| 36936860703 | Grammar fuzz | `3dbcc91` | cancelled |
| 36938198364 | CI | `7f87b7d` | cancelled |
| 36938198394 | Grammar fuzz | `7f87b7d` | cancelled |
| 36938496051 | CI | `1836541` | success |
| 36938496066 | Grammar fuzz | `1836541` | cancelled |
| 36942304451 | CI | `b9d00fe` | failure |
| 36942304502 | Grammar fuzz | `b9d00fe` | success |
| 36950991085 | CI | `5daf85d` | failure |
| 36950991088 | Grammar fuzz | `5daf85d` | failure |

Tally (pr297-round3-cancelled): 7 rows of the table above with `verdict` = `cancelled`.

The cancelled runs were each cancelled by the next push. `CI` 36938496051 at `1836541` ran to the end:

Tally (run-36938496051-jobs): 19 = 18 + 1.

That is eighteen success and `Docs consistency` skipped, correctly: green.

#### `CI` 36942304451 at `b9d00fe`, the round's head: #283's eighth

Tally (run-36942304451-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure: `Test (macos-latest / lua54)`, job 110636448104, its only failure against 175 `test result: ok`. The row is `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`, and all three of #283's required fragments are present:
- `pump timeout waiting for the accept and its import`;
- the mirror's `text="fn main() {\n    println\n    \n}\n// tail\n" popup_rows=0 anchor=None`;
- `test result: FAILED. 9 passed; 1 failed; 0 ignored`.

That is **#283's eighth occurrence**, on the same leg as its seventh (this PR's run at `17c3c8f`); commented on #283, not rerun. The row ran `ok` on the four other legs that build it, and the branch changes nothing on its path.

#### `Grammar fuzz` 36942304502 at `b9d00fe`: green, and what the extended confirmation costs

Both legs succeeded:
- `Grammar fuzz`, job 110636622755, 23:44:23–01:18:45Z, 94.4 min;
- `Grammar fuzz (asan-strict)`, job 110636623044, 23:44:25–01:23:15Z, 98.8 min.

Round 2's legs ran 66 min. The difference is the owner's ruling at the round, the extended confirmation, which ran nineteen times (ten under `ubsan`, nine under `asan-strict`) on bash, cmake, html, python and yaml inputs of 20 to 260 KB. Each returned and was filed slow, each taking its input's real time, 398 s to 2,674 s, about 8,970 worker-seconds a leg. The job's limit is 120 min.

#### `CI` 36950991085 at `5daf85d`, the round's head: #307, a first sample

Tally (run-36950991085-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure: `Test (macos-latest / luajit)`, job 110663724009, against 175 `test result: ok`. The row is `e7b_review_wire_acceptance::a_save_sends_did_save_and_rust_analyzer_flychecks_on_it`, "the check showed as a suffix on ready, or as the tracker's busy title under a reload", with `["9 = LSP:ready"]` and `["9 b -"]`: the flycheck on neither surface. It is not #285's signature (that was `LSP:idx` across the check, fixed by `3de1e1f`) nor #295's (`wait_warm`), so it was filed as **#307**, first occurrence; not rerun. Every leg's `running` lines pair with its result lines; `WouldBlock`, `did not become ready` and `got ok` appear zero times; the round's three `e7h3_` rows ran `ok` on every leg.

#### `Grammar fuzz` 36950991088 at `5daf85d`: the extended confirmation calls a cmake quadratic a hang

Tally (run-36950991088-jobs): 2 = 1 + 1.

- `Grammar fuzz (asan-strict)` (job 110663677904), 01:27:09–03:07:19Z: success.
- `Grammar fuzz` (job 110663677707), 01:27:09–03:11:55Z: **failure**, on cmake `hang` `one parse over 10000 ms`, reproduced alone, its minimum 19,755 bytes.

Its note: the input as found (228,316 bytes, almost all spaces) "did not return in 120 s", its minimum "returned in 10254 ms", and "alone again under ten times that limit: did not return in 1200 s", which is the owner's fix-round-3 rule for a hang.

On the laptop the same input under the same build **returned in 375 s**, and its minimum's growth is quadratic (3.0, 11.7 and 47.2 s at 1×, 2× and 4×). CI's runner was 3.4 times slower on the minimum. So it is cmake's whitespace quadratic, not a parse that does not terminate, and ten times the confirmation's limit was not enough for it under ASan on the runner. This is no intermittent and no registered row: it is the harness's verdict under the ruling, on the owner to rule (E7h's fix-round-3 pass), and recorded here because this file is where a red is accounted for. Fifteen other inputs took the extended run at this head and returned.

#### Local samples in the round's gates

- `20261001T230200Z-3082799` at `1836541`, beside three niced fuzz workers: **six of six**, 179 targets, 5,146 passed, 0 failed, 62 ignored.
- `20261001T234437Z-3363671` at `b9d00fe`, beside three niced 600 s fuzz arms building and fuzzing (load 23 to 27): five of six, 5,145 / 1 / 62. The one failure is **#304's third occurrence**, the same 44 pixels at x = 102–103; commented on #304.
- `20261002T001509Z-3523798` at `b9d00fe`, on a quiet machine (load about 1.5): **six of six**, 179 targets, 5,146 passed, 0 failed, 62 ignored; every intermittent's row `ok`.
- `20261002T012705Z-3618489` at `5daf85d`, the round's head, on a nearly idle machine (load about 1.5 at its start): five of six, 5,145 / 1 / 62. The one failure is **#291's sixth occurrence**, all three of its fragments (`HLS reports the type error; store []`, a trace through `LSP:ready·setting` and never processing, `4 passed; 1 failed; 0 ignored`); commented on #291, and its list above extended. It is the first sample not beside heavy load.
- `20261002T013808Z-3707117` at `5daf85d`, run again: **six of six**, 179 targets, 5,146 passed, 0 failed, 62 ignored; #291's row `ok`, which is non-reproduction and nothing more.

So, at E7h's third fix round: #283 moves to eight, #291 to six and #304 to three; #307 is at one; #298, #300, #303, #305 and #306 not sampled, and every count above stands.

### E7h's fourth fix round: PR #297's runs, R7's twenty-first and #308's first

Recorded 2026-10-02 under the rule above, each run read from the jobs endpoint at attempt 1. The PR's runs are `pull_request` runs of the merge commit with `main`.

Tally (pr297-round4-runs): 6 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 36992701054 | CI | `d07d8b7` | success |
| 36992701122 | Grammar fuzz | `d07d8b7` | success |
| 37001969427 | CI | `972aa44` | cancelled |
| 37001969422 | Grammar fuzz | `972aa44` | cancelled |
| 37002984277 | CI | `64d396b` | failure |
| 37002984373 | Grammar fuzz | `64d396b` | success |

Tally (pr297-round4-cancelled): 2 rows of the table above with `verdict` = `cancelled`.

Both were cancelled by the next push, `CI` 37001969427 with twelve jobs finished green, its six test legs cancelled and `Docs consistency` skipped.

#### `CI` 36992701054 at `d07d8b7`

Tally (run-36992701054-jobs): 19 = 18 + 1.

Eighteen success and `Docs consistency` skipped, correctly: green. Every leg's target headers pair one to one with its result lines, all `ok`: `Test (crdt)` 176, both macOS legs 178, the three Ubuntu legs 177. `FAILED`, `WouldBlock`, `did not become ready` and `got ok` appear zero times, and the round's growth rows ran `ok` on every leg.

#### `Grammar fuzz` 36992701122 at `d07d8b7`: the owner's growth rule on CI

Both legs succeeded:
- `Grammar fuzz`, job 110792350344, 09:57:00–11:34:15Z, 97.3 min;
- `Grammar fuzz (asan-strict)`, job 110792349637, 09:57:00–11:24:12Z, 87.2 min.

The owner's ruling at the round judges a slow input by the budget its growth projects, and calls a hang only what does not return inside a cap of 180 times the limit. **Round 3's red, cmake's 228,316-byte whitespace input, came up again in both legs, byte-identical, and is filed slow:**
- exponent 1.98 under `ubsan`, 1,011 s against a projected 987 s;
- 1.97 under `asan-strict`, 1,154 s against 1,135 s.

Two inputs under `ubsan` returned past their budgets and are filed slow and mispredicted:
- cmake, exponent 0.55, 777 s against a 150 s budget;
- yaml, exponent 1.01, 515 s against 225 s.

Each minimum ends in a token that closes its slow context, so repeating it grows the time linearly where the input grows another way.

Under `asan-strict` the first half of a slow markdown_inline input passed the 4 GB cap and was filed as its own memory finding, which the list's #296 row accepts.

The legs ran 97.3 and 87.2 min against round 3's 104.8 and 100.2 at `5daf85d`; the grammars' summed seconds fell 9% and 14%. That is not round 2's 66: what remains is the quadratic inputs' own time. This is no intermittent; it is recorded because this file is where a fuzz leg's time has been accounted since round 3.

#### `CI` 37002984277 at `64d396b`, the round's head: #308, a first sample

Tally (run-37002984277-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure: `Perf budgets (debug)`, job 110825077473, on `editor::tests::composition_overhead_under_ten_percent`, "composition machinery added more than 10% overhead: 1.102 (single=90612 ns, dispatch=99840 ns)", the dispatch overhead printed as 10.2% against the budget's 10%. Every other target `scripts/perf-budgets` runs reported `ok`. U6 and U20 were retired by moving this budget into this job, so no row matched; it was filed as **#308**, first occurrence, not rerun. `src/editor.rs` has no diff on the branch, and the same job was green at `d07d8b7`, whose code differs from `64d396b` in comments only. The six test legs pair their target headers one to one with their result lines, all `ok` (176, 178, 178, 177, 177, 177); `FAILED`, `WouldBlock`, `did not become ready` and `got ok` appear zero times, and the round's growth rows ran `ok` on every leg.

#### `Grammar fuzz` 37002984373 at `64d396b`

Both legs succeeded:
- `Grammar fuzz`, job 110824871973, 11:48:33–13:12:56Z, 84.4 min;
- `Grammar fuzz (asan-strict)`, job 110824872140, 11:48:32–13:18:26Z, 89.9 min.

cmake's 228,316-byte input came up again in each leg and is filed slow:
- exponent 1.96 under `asan-strict`, 1,048 s against a projected 1,012 s;
- exponent 1.49 under `ubsan`. There its whitespace minimum at two times over took 21.4 s where a quadratic takes about 40 (10.2, 21.4 and 79.9 s at one, two and four times), so it projected 386 s for an input that took 1,261 s, which is inside the 1,546 s budget of four times that.

No input returned past its budget. A prefix of the slow markdown_inline input again passed the 4 GB cap in both legs, accepted as #296. The legs' time is a second sample beside `d07d8b7`'s: 84.4 and 89.9 min against 97.3 and 87.2.

#### Local samples in the round's gates

- `20261002T095734Z-57751` at `d07d8b7`: **six of six**, 179 targets, 5,149 passed, 0 failed, 62 ignored. The rows of #283, #291, #298, #300 and #303 to #307 all ran `ok`.
- `20261002T113708Z-303920` at `972aa44`: **six of six**, 179 targets, 5,149 passed, 0 failed, 62 ignored; the same rows `ok`.
- `20261002T114815Z-408166` at `64d396b`, the round's head, the load average 3.4 at its start and about 18 over five minutes at its end (other sessions' work): five of six, 179 targets, 5,148 passed, 1 failed, 62 ignored. The one failure is **R7's twenty-first occurrence**, all three of its fragments at `attach.rs:1971`, `test result: FAILED. 373 passed; 1 failed`; its row above is extended. Every other intermittent's row ran `ok`.
- `20261002T115902Z-518091` at `64d396b`, run again: **six of six**, 179 targets, 5,149 passed, 0 failed, 62 ignored; R7's row `ok`, which is non-reproduction and nothing more.

So, at E7h's fourth fix round: R7 moves to twenty-one, and #308 is at one; #283, #291, #298, #300 and #303 to #307 not sampled, and every count above stands.

### E7i's checkpoint: PR #309's runs, #283's ninth, and #291's seventh and eighth with a base control

Recorded 2026-10-03, each run read from the jobs endpoint at attempt 1. The PR's runs are `pull_request` runs of the merge commit with `main`.

Tally (pr309-runs): 4 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 37080922545 | CI | `2d4c848` | cancelled |
| 37080922546 | Grammar fuzz | `2d4c848` | cancelled |
| 37082904461 | CI | `0e2998b` | success |
| 37082904517 | Grammar fuzz | `0e2998b` | success |

Tally (pr309-cancelled): 2 rows of the table above with `verdict` = `cancelled`.

Both were cancelled by the push of `0e2998b`.

#### `CI` 37080922545 at `2d4c848`: the branch's own, and #283's ninth

Tally (run-37080922545-jobs): 19 = 13 + 4 + 1 + 1.

Thirteen success, four failure, `Docs consistency` skipped, `Test (macos-latest / luajit)` cancelled.

- **Three Ubuntu test legs**, jobs 111081065680 (`luajit`), 111081065724 (`luajit, no crdt`) and 111081065769 (`lua54`), failed while linking test binaries: `ld terminated with signal 7 [Bus error]`, `No space left on device`, and the runner's warning of 51 MB free. That is the branch's own. Its wasm candidate linked wasmtime and Cranelift into the `pmacs` library, about 160 MB more in every debug test binary (`m4_acceptance` 278.9 MB without, 438.9 MB with). It is fixed at `fe65e1d` by a non-default `wasm-unit` feature. It is not an intermittent.
- **`Test (macos-latest / lua54)`**, job 111081065656, failed two targets:
  - the branch's own three E7i process rows: the worker exited because `setrlimit(RLIMIT_AS)` returned `EINVAL` on macOS. Fixed at `0e2998b`.
  - **#283's ninth occurrence**, with all three fragments (`pump timeout waiting for the accept and its import`, `popup_rows=0 anchor=None`, `9 passed; 1 failed`). Commented on #283, not rerun.
- `Test (crdt)` succeeded.

#### `CI` 37082904461 at `0e2998b`, the head: green

Tally (run-37082904461-jobs): 19 = 18 + 1.

Eighteen success, and `Docs consistency` skipped, correctly. Every leg's target headers pair one to one with its result lines (each macOS log's one unpaired `running 1 test` is the adapter step's own `grep` text), all `ok`: `Test (crdt)` 177, `luajit, no crdt` 178, both macOS legs 179, the two other Ubuntu legs 178. `FAILED`, `WouldBlock`, `did not become ready` and `got ok` appear zero times. The E7i process rows ran `ok` on every leg: three on Linux, and two on macOS, where #296's memory row is ignored because `RLIMIT_AS` is refused. #283's selector ran `ok` on both macOS legs, which is non-reproduction and nothing more.

#### `Grammar fuzz` 37082904517 at `0e2998b`

Both legs succeeded:
- `Grammar fuzz`, job 111087337525, 00:39:43–01:27:07Z, 47.4 min;
- `Grammar fuzz (asan-strict)`, job 111087337212, 00:39:42–01:29:40Z, 50.0 min.

Each printed `image: ubuntu24 20260927.320.1` and GCC 13.3.0. The decision step said `run=true` (`Cargo.lock changed`, by the branch's new crates). No `tree-sitter` package and nothing under `vendor/` or `builtin/queries/` changed since `a013d46`, so no grammar counted as changed since `cf63a2f` and all twenty-two ran their 15 s smoke. Neither leg found a crash, a hang, a memory finding, an unconfirmed one or an accepted one; markdown_inline's #296 input, which `a013d46`'s 600 s run reached, is not reached in 15 s. Slow findings, each with its exponent: five in each leg, cmake three, cuda and lua one each. One is mispredicted: cmake under `ubsan`, exponent 0.56, minimum 21,647 bytes, returned in 407,788 ms against a budget of 153,404. zig reached 33 mutated inputs under `ubsan` and 28 under `asan-strict`.

#### Local samples in the checkpoint's gates, and a base control for #291

- `20261002T233722Z-2508081` at `2d4c848`: five of six, 185 targets, 5,156 passed, 1 failed, 65 ignored. The failure is **#291's seventh**, all three fragments; commented.
- `20261003T000314Z-2657125` at `2d4c848`: six of six, 5,157 / 0 / 65. #291's row `ok`.
- `20261003T003848Z-2861426` at `0e2998b`: the sweep failed on a full local disk (`mold: failed to write to an output file. Disk full?`). It is no evidence.
- `20261003T004422Z-2872349` at `0e2998b`: five of six, 5,156 / 1 / 65. The failure is **#291's eighth**, all three fragments; commented.
- `20261003T011317Z-3001607` at `0e2998b`, the head: **six of six**, 185 targets, 5,157 passed, 0 failed, 65 ignored. #291's row `ok`, which is non-reproduction and nothing more.

**The control.** #291's row alone, eight interleaved pairs in the gate's environment, at `a013d46` (the base, its own worktree and target) and `0e2998b`, 03:05 to 03:30 local on a quiet machine. **`a013d46` failed 5 of 8 and `0e2998b` 2 of 8**, every failure with `store []` and no `processi` at 125.8–131.2 s, every pass in 7.5–9.0 s. The branch is excluded as the cause for this sample, and the row's rate on this machine tonight is far above E7g's 2 of 16. Commented on #291.

So, at C7i: #283 moves to nine and #291 to eight; R7, #298, #300 and #303 to #308 not sampled, and every count above stands.

### The E7i build: PR #309's runs, #282's third, and #291's ninth

Recorded 2026-10-03, after the owner's ruling, each run read from the jobs endpoint at attempt 1. The PR's runs are `pull_request` runs of the merge commit with `main`.

Tally (pr309-build-runs): 12 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 37127164189 | CI | `32cf223` | failure |
| 37127164469 | Grammar fuzz | `32cf223` | cancelled |
| 37130407757 | CI | `cfaa81a` | success |
| 37130407788 | Grammar fuzz | `cfaa81a` | cancelled |
| 37132283583 | CI | `45186c9` | cancelled |
| 37132283539 | Grammar fuzz | `45186c9` | cancelled |
| 37133953154 | CI | `0b7a5bb` | failure |
| 37133953201 | Grammar fuzz | `0b7a5bb` | success |
| 37137958321 | CI | `fd53449` | cancelled |
| 37137958314 | Grammar fuzz | `fd53449` | cancelled |
| 37138741327 | CI | `fa176de` | success |
| 37138741372 | Grammar fuzz | `fa176de` | success |

Tally (pr309-build-cancelled): 6 rows of the table above with `verdict` = `cancelled`.

Each was cancelled by the push that followed it.

#### `CI` 37127164189 at `32cf223`: #282's third

Tally (run-37127164189-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure.

The failure is **`Test (crdt)`**, job 111214728970, with 175 `test result: ok` and one `FAILED`. It is **#282's third occurrence** and the first on that job, with all of its fragments:

- `*lsp* names the request and the code: []`;
- `WIRE *errors* gained ["[lsp] LSP: default-rust refused textDocument/prepareRename as a client error, -32602 InvalidParams: No references found at position"]` and `WIRE label "ready", last_error None`;
- `test result: FAILED. 4 passed; 1 failed; 2 ignored`, the count moving with the suite as at the second occurrence.

The row's trace holds 98 `$/progress` frames, 78 of them `cachePriming` reports, against the 64-entry ring. The same row ran `ok` on the run's three other Linux legs. It is not the branch's: the head adds a cgroup report, an arming row and an E7i witness, and nothing reaches LSP. Commented on #282, not rerun.

The other five test legs:

- `ubuntu lua54` 111214729006, `luajit` 111214729049 and `luajit, no crdt` 111214729102: 178 `ok` each;
- `macos lua54` 111214729079 and `luajit` 111214729114: 179 each.

Zero `FAILED`, `WouldBlock`, `did not become ready` and `got ok` on those legs. Every leg printed the editor's cgroup attempt: refused at `mkdir` beside `/system.slice/hosted-compute-agent.service` on Ubuntu, no `/proc/self/cgroup` on macOS.

#### `CI` 37130407757 at `cfaa81a`: green

Tally (run-37130407757-jobs): 19 = 18 + 1.

Eighteen success, and `Docs consistency` skipped, correctly. All six test legs read:

| leg | job | `ok` |
|---|---|---|
| `Test (crdt)` | 111224260631 | 177 |
| `luajit, no crdt` | 111224260672 | 178 |
| `macos luajit` | 111224260680 | 179 |
| `macos lua54` | 111224260732 | 179 |
| `ubuntu lua54` | 111224260780 | 178 |
| `ubuntu luajit` | 111224260791 | 178 |

Zero `FAILED`, `WouldBlock`, `did not become ready` and `got ok`. All thirteen E7i rows `ok` on every leg, macOS included.

#### `CI` 37133953154 at `0b7a5bb`: the branch's own red

Tally (run-37133953154-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure.

The failure is **`Test (ubuntu-latest / luajit, no crdt)`**, job 111234611782: its test step's 176 results are 175 `ok` and one `FAILED`, and the failure skipped the leg's doc-test and `pmacs-protocol` steps. The failing row is the branch's own, `e7i_296s_32_kb_paragraph_is_stopped_at_the_editor_s_defaults`: `death=time: parse ran past its deadline of 5000 ms and was cancelled after 5100 ms`, where the row demanded a memory death. On that runner the debug build grew slower than the deadline allowed it to reach 1 GiB. The row now accepts either limit and prints which fired (`fd53449`). It is not an intermittent, and no issue is commented.

The other five test legs: `Test (crdt)` 111234611431 177 `ok`, `ubuntu lua54` 111234611494 and `luajit` 111234611505 178 each, `macos luajit` 111234611490 and `lua54` 111234611613 179 each. Zero `FAILED`, `WouldBlock`, `did not become ready` and `got ok` on those legs, and all twenty E7i rows `ok` on each.

`Grammar fuzz` 37133953201 at the same head is green on both legs, with no crash, hang or memory finding. It is CI's first replay through the worker, and the replay crashed nothing.

#### `CI` 37138741327 at `fa176de`, the head: green

Tally (run-37138741327-jobs): 19 = 18 + 1.

Eighteen success, and `Docs consistency` skipped, correctly. All six test legs read:

| leg | job | `ok` |
|---|---|---|
| `Test (crdt)` | 111248673840 | 177 |
| `luajit, no crdt` | 111248673940 | 178 |
| `ubuntu lua54` | 111248673942 | 178 |
| `ubuntu luajit` | 111248673968 | 178 |
| `macos lua54` | 111248673890 | 179 |
| `macos luajit` | 111248673976 | 179 |

Zero `FAILED`, `WouldBlock`, `did not become ready` and `got ok`. All twenty-two E7i rows `ok` on every leg. The 32 KB row that was red at `0b7a5bb` printed its stopping limit: memory on four legs, and the deadline on `ubuntu lua54` and `ubuntu luajit`.

`Grammar fuzz` 37138741372 at the head is green on both legs, with no crash, hang or memory finding and none mispredicted. The replay crashed nothing.

#### Local samples in the build's gates

- `20261003T150924Z-395472` at `45186c9`: five of six, 186 targets, 5,170 passed, 2 failed, 62 ignored, no daemon survived.
  - **#291's ninth**, all three fragments, 129.81 s, the owner gaming beside it; commented.
  - An E7h witness row of the branch's own (`index out of bounds` reading an in-process tree that the branch's new default had moved into a worker), fixed at `e27d347`. It is not an intermittent.
- `20261003T153819Z-531575` at `0b7a5bb`, `20261003T160232Z-667691` at `6122fc4`, `20261003T161417Z-787953` at `fd53449` and `20261003T164622Z-966978` at `fa176de`: six of six each, 186 targets, 5,176, 5,178, 5,178 and 5,178 passed, 0 failed, 62 ignored, no daemon survived. No intermittent sampled.

So, at the E7i build: #282 moves to three and #291 to nine; #283, R7, #298, #300 and #303 to #308 not sampled, and every count above stands.

### E7i's fix round 1: PR #309's runs, #283's tenth and #291's tenth; review 2's gates and R7's twenty-second

Recorded 2026-10-04 at E7i's fix round 2, where review 2's Medium 2 found them missing: the fix round recorded these in PR #309's body and its pass, and none here. Each run is read from the jobs endpoint at attempt 1 of 1. The PR's runs are `pull_request` runs of the merge commit with `main`.

Tally (pr309-fix1-runs): 6 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 37161567765 | CI | `c540a68` | failure |
| 37161567771 | Grammar fuzz | `c540a68` | cancelled |
| 37163241023 | CI | `fb8fc2c` | failure |
| 37163241005 | Grammar fuzz | `fb8fc2c` | cancelled |
| 37164795518 | CI | `cd6cd52` | success |
| 37164795489 | Grammar fuzz | `cd6cd52` | success |

Tally (pr309-fix1-cancelled): 2 rows of the table above with `verdict` = `cancelled`.

Each was cancelled by the push that followed it.

#### `CI` 37161567765 at `c540a68`: the branch's own orphan row on macOS, and #283's tenth

Tally (run-37161567765-jobs): 19 = 16 + 1 + 2.

Sixteen success, `Docs consistency` skipped, and two failures, both macOS legs:

- **`Test (macos-latest / luajit)`**, job 111315929548: 176 `test result: ok` and 2 `FAILED`. One is the branch's own, `e7i_review1_a_worker_does_not_outlive_its_editor` (`tests/e7i_review1_probes.rs:324`): it read the worker's state from `/proc`, which macOS lacks, so its new precondition failed there; fixed at `fb8fc2c`, not an intermittent. The other is **#283's tenth occurrence**, with all three fragments: `pump timeout waiting for the accept and its import` (`popup_rows=0 anchor=None`) and `test result: FAILED. 9 passed; 1 failed`. Commented on #283, not rerun.
- **`Test (macos-latest / lua54)`**, job 111315929573: 177 `ok` and 1 `FAILED`, the same orphan row.

The four Linux legs: `Test (crdt)` 111315929558 178 `ok`, `ubuntu lua54` 111315929571, `luajit` 111315929633 and `luajit, no crdt` 111315929600 179 each, none failed.

#### `CI` 37163241023 at `fb8fc2c`: the crash scenario's schedule on macOS lua54

Tally (run-37163241023-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure: **`Test (macos-latest / lua54)`**, job 111320886081, 177 `ok` and 1 `FAILED`, the branch's own `e7i_review1_the_editor_knows_its_worker_crashed` reading `burst=2`, three edits 250 ms apart having outlasted the first second's back-off on that runner. Fixed at `cd6cd52`, not an intermittent. The orphan row passed on both macOS legs. The other five legs: `Test (crdt)` 111320885998 178 `ok`, `macos luajit` 111320886052 180, `ubuntu lua54` 111320886073, `luajit` 111320886030 and `luajit, no crdt` 111320886103 179 each, none failed.

#### `CI` 37164795518 at `cd6cd52`, the round's head: green

Tally (run-37164795518-jobs): 19 = 18 + 1.

Eighteen success, and `Docs consistency` skipped, correctly. All six test legs read:

| leg | job | `ok` |
|---|---|---|
| `Test (crdt)` | 111325437127 | 178 |
| `luajit, no crdt` | 111325437154 | 179 |
| `ubuntu lua54` | 111325437117 | 179 |
| `ubuntu luajit` | 111325437152 | 179 |
| `macos lua54` | 111325437143 | 180 |
| `macos luajit` | 111325437128 | 180 |

Zero `FAILED`. `Grammar fuzz` 37164795489 at the same head is green on both legs (`Grammar fuzz` 111325453398, `Grammar fuzz (asan-strict)` 111325453561): no crash, hang or memory cut; the replay sent 945 and 941 inputs through each arm's worker and crashed none, and all five `fuzz/regress/markdown/` inputs were contained.

#### Local samples in the round's gates and review 2's

- `20261003T230816Z-2486117` at `12c3a6c`: five of six, 187 targets, 5,200 passed, 1 failed, 62 ignored, no daemon survived. The one is **#291's tenth**, all three fragments, 125.43 s; the branch has no diff in that suite or the LSP path. Commented, not rerun.
- `20261003T232249Z-2582992` at `c540a68`, `20261003T235450Z-2670953` at `fb8fc2c` and `20261004T002336Z-2779234` at `cd6cd52`: six of six each, 187 targets, 5,201 passed, 0 failed, 62 ignored, no daemon survived.
- Review 2's `20261004T104358Z-3360185` at `cd6cd52`: five of six, 187 targets, 5,200 passed, 1 failed, 62 ignored, no daemon survived. The one is **R7's twenty-second**, in its list above. R7 has a row and no issue, so it is recorded here and not filed.
- Review 2's `20261004T112802Z-3536917` at `cd6cd52`, the same tree: six of six, 5,201 passed, 0 failed, 62 ignored; R7's row passed, which is non-reproduction and nothing more.

So, at E7i's fix round 1 and review 2: #283 moves to ten, #291 to ten and R7 to twenty-two; #256 stands at three (`6193b67`); #282, #298, #300 and #303 to #308 not sampled, and every other count above stands.

### E7i's fix round 2: the head at `db697a2`, #283's eleventh, R7's twenty-third, and #311

Recorded 2026-10-04 beside the section above, in the same push. Review 2's
Medium 2 asked for fix round 1's records; the round that answered it produced
these, which the commit answering it therefore could not hold. Each run is
read from the jobs endpoint at attempt 1 of 1.

Tally (pr309-fix2-runs): 2 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 37203840409 | CI | `db697a2` | failure |
| 37203840437 | Grammar fuzz | `db697a2` | success |

#### `CI` 37203840409 at `db697a2`, the round's head: #283's eleventh

Tally (run-37203840409-jobs): 19 = 17 + 1 + 1.

Seventeen success, `Docs consistency` skipped, and one failure:
**`Test (macos-latest / lua54)`**, job 111440850223, 178 `ok` and 1 `FAILED`,
**#283's eleventh occurrence** on all three fragments (`pump timeout waiting
for the accept and its import`, `popup_rows=0 anchor=None`, `9 passed; 1
failed`). `tests/e7_review1_probes.rs`, `completion.lua` and `src/lsp.rs` have
no diff on the branch, and that leg ran the three E7i suites whole (27, 22 and
8 passed). Commented on #283, not rerun. The other five test legs:

| leg | job | `ok` |
|---|---|---|
| `Test (crdt)` | 111440850195 | 179 |
| `luajit, no crdt` | 111440850177 | 180 |
| `ubuntu lua54` | 111440850171 | 180 |
| `ubuntu luajit` | 111440850199 | 180 |
| `macos luajit` | 111440850201 | 181 |

Zero other `FAILED`, and the round's three new rows `ok` on every leg.

#### `Grammar fuzz` 37203840437 at `db697a2`: green on both legs

`Grammar fuzz` 111440817995, 56 min, and `Grammar fuzz (asan-strict)`
111440818192, 43 min: no crash, no hang, no memory cut, no unconfirmed and no
accepted finding. The replay sent 943 and 941 inputs through each arm's worker
and crashed none. All five `fuzz/regress/markdown/` inputs were contained on
both legs: the underscores and the asterisks by the worker's watch, 596 with
its recorded edits, 7672 and the nested openers at the deadline. Slow
findings: seven under `ubsan` (bash, cmake three, cuda, lua, python) and five
under `asan-strict` (cmake three, cuda, python).

#### The round's gates, and R7's twenty-third

- `20261004T123921Z-3949386` at `79d2895`: five of six. Sweep 188 targets,
  188 `running` lines and 188 result lines, 5,205 passed, 4 failed, 62
  ignored; no daemon survived. Two of the four are the round's own, a
  statusline inventory row that pins the built-in providers and a crash row
  that found the mode line by its text, fixed at `f892f13` and `db697a2`; one
  is **R7's twenty-third**, in its list above; one is **#311** below.
- `20261004T125601Z-4051540` at the tip `db697a2`: five of six. Sweep 188
  targets, 188 `running` and 188 result lines, **5,208 passed, 1 failed, 62
  ignored**; no daemon survived. The one is #311. R7's row passed, which is
  non-reproduction and nothing more.

#### #311, deterministic since the upgrade, local: this laptop's GHC, and not #291

`e7e_haskell_acceptance::e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error`
on both gates above, filed as **#311**. It is the same test as #291 and not
the same signature, and the two must not be read as one row: #311's store
holds `Could not find module ‘Prelude’` and its trace reaches
`processi`, where #291's store is empty (`store []`) and its trace reaches
`LSP:ready·setting` and never `processi`. At 14:07 on 2026-10-04,
mid-round, this laptop's system upgrade took `ghc` and `ghc-libs` from
9.6.6-3 to 9.6.7-1 and `haskell-language-server` to 2.4.0.0-5. Since then
`/usr/bin/ghc` 9.6.7, which the fixture pins because the installed server was
built against it, cannot compile `main = print 1`: "Could not find module
‘Prelude’ / There are files missing in the ‘base-4.18.3.0’
package". With `-dynamic` the same module compiles and links, and `ghc-static`
is not installed. Confirmed outside the harness at this push: `/usr/bin/ghc`
fails on the trivial module and succeeds with `-dynamic`, and the ghcup 9.10.3
first on `PATH` succeeds, so the fixture's pin is what meets the break. It is
the machine and not the branch, which has no diff in that suite or the LSP
path, and it fails every local gate on this laptop until `ghc-static` is
installed or the fixture passes `-dynamic`.

Tally (311-runs): 36 items in the list below.

- `20261004T123921Z-3949386` at `79d2895` (E7i fix round 2's first gate);
- `20261004T125601Z-4051540` at `db697a2` (the same round's tip);
- `20261004T145947Z-40985` at `db697a2` (E7i review round 3's gate, the second
  occurrence recorded, below);
- `20261004T202450Z-1217913` at `4fc5599` (E8.1's gate; `07-sweep-luajit` only,
  its `06-sweep` failing the row with #291's fragments instead);
- `20261004T210335Z-1636352` at `ae160fa` (E8.2's gate, both sweeps);
- `20261004T212802Z-1897687` at `f4b7d5c` (E8.3's first gate, stopped by its
  author after `06-sweep`, the one sweep read);
- `20261004T215311Z-2163018` at `a2e18c5` (E8.3's gate, both sweeps);
- `20261004T222129Z-2395408` at `8ce4b0c` (E8.4's gate, both sweeps);
- `20261004T225031Z-2642055` at `a8104f4` (E8.5's gate, both sweeps);
- `20261004T231827Z-3005940` at `35e74fa` (E8's first tip gate, both sweeps);
- `20261004T234355Z-3389009` at `1ace8b3` (E8's tip gate; `07-sweep-luajit` only,
  its `06-sweep` failing the row with #291's fragments instead);
- `20261005T081704Z-3789356` at `1ace8b3` (E8 review 1's gate; `06-sweep` only,
  its `07-sweep-luajit` failing the row with #291's fragments instead);
- `20261005T182116Z-597588` at `44dfb24` (E8 fix round 1's gate, both sweeps);
- `20261005T193708Z-986096` at `7505eea` (the same round's gate at its second head,
  both sweeps);
- `20261005T213945Z-1605050` at `57a980e` (the same round's gate at its third head,
  both sweeps);
- `20261005T224931Z-2046695` at `57a980e` (E8 review 2's gate, both sweeps);
- `20261006T104137Z-2974993` at `d2de579` (E8 fix round 2's first gate, ended by the
  machine's restart in `07-sweep-luajit`; `06-sweep` read whole, the partial luajit
  sweep red on the row too);
- `20261006T111111Z-85507` at `d2de579` (the same round's gate, both sweeps).
- `20261006T150249Z-1162130` at `e23510a` (PR #321's tip gate, its one sweep, `05-sweep`).
- `20261006T164749Z-1441387` at `ab06caa` (PR #321's second head's gate, `05-sweep`).
- `20261006T175402Z-1776889` at `05dfa6a` (E8 fix round 4's gate, both sweeps; recorded at
  that round, below).
- `20261006T220742Z-2454838` at `b374cd8` (E8b.1's gate, `05-sweep`; recorded at E8b's
  checkpoint, below).
- `20261006T222430Z-2574647` at `3764454` (E8b.2's gate, `05-sweep`).
- `20261006T224358Z-2710606` at `0001946` (E8b's tip gate, `05-sweep`).
- `20261007T101629Z-2858399` at `0001946` (E8b review 1's gate, `05-sweep`; recorded at
  E8b's fix round 1, below).
- `20261007T154827Z-3509142` at `70d69d0` (E8b fix round 1's tip gate, `05-sweep`).
- `20261007T183201Z-3888421` at `226f5f0` (the cache-budget PR's tip gate, `05-sweep`;
  recorded with that PR, below).
- `20261007T204535Z-472312` at `1dc7658` (the same PR's addendum gate, `05-sweep`).
- `20261008T132308Z-224598` at `1218ba1` (the same PR's second addendum gate, `05-sweep`).
- `20261008T195204Z-1042685` at `bca69d4` (the pre-release PR's gate, both sweeps; recorded with
  that PR, below).
- `20261008T232032Z-2370178` at `334cc13` (the pre-release PR's fix round 1 gate, both sweeps;
  recorded with that round, below).
- `20261009T080847Z-2899901` at `334cc13` (the pre-release PR's review 1 gate, both sweeps;
  recorded with fix round 2, below).
- `20261009T122310Z-3757634` at `2ad0b6c` (the pre-release PR's fix round 2 gate,
  `07-sweep-luajit`; recorded with that round, below).
- `20261009T135200Z-4043146` at `29b5b81` (the same round's tip gate, `06-sweep`; recorded
  with that round, below).
- `20261009T161057Z-421242` at `f45fa10` (the pre-release PR's fix round 3 gate, both sweeps;
  recorded with that round, below).
- `20261009T205956Z-1456466` at `2c890aa` (#347's pull request's gate, `05-sweep`; recorded with
  it, below).

#311 is **deterministic, not intermittent**: every local gate on this laptop fails
it until `ghc-static` is installed or the fixture passes `-dynamic`. So what the
list counts is gate runs read, and no occurrence count or rate is claimed for it
--- unlike every intermittent row in this file, where a count is the whole point.
Fix round 2's section below recorded its first two runs together as the sample
that filed it. E8's gates failed the row in every sweep they ran, with #311's
fragments in all but two: E8.1's `06-sweep` and the tip's carried #291's instead
(store empty, the trace never reaching `processi`), with the condition holding, as
E7i's fix round 3 also saw. Which signature the row fails with is not this row's to claim.

So, at E7i's fix round 2: #283 moves to eleven and R7 to twenty-three; #311
is recorded and deterministic, its runs listed in its own block (three read by
review round 3, below). #291 is not sampled here. **This paragraph read "#291 is not
sampled and cannot be on this laptop while #311 stands, since #311 takes the same row
first"; fix round 3's gate falsified it**, and that section below carries the
falsification beside the occurrence. The claim was an inference about which of two
signatures a broken toolchain must produce first, written into this file as a record:
when the server never processes the typed edit, the row fails on #291's fragments
before the GHC break can show, so both shapes are reachable while the toolchain
stands. #256 stands at three
(`6193b67`), #282, #298, #300 and #303 to #308 are not sampled, and every
other count above stands.

### E7i's review round 3: the record of `a3ea968`'s push, #311's second, and #312

Recorded 2026-10-04 by the owner, after the narrow review round 3 of fix
round 2's four core commits at `db697a2`. The round found no High and no
Medium and ran no CI of its own; its probes are on `e7i/review-3` at
`ae63d6f` with no pull request, so the only runs here are the registry
push's own and the round's one gate.

#### The push of `9904d79` and `a3ea968` to `main`

Tally (push-a3ea968-jobs): 15 = 6 + 9.

`CI` 37209683738 at `a3ea968`, attempt 1 of 1, conclusion success: six jobs
succeeded and nine were skipped. **This sentence first read "the form every
`push` run to `main` takes (the matrix legs are a `pull_request` trigger)",
which is false**, and E7i's merge push falsified it the same day: `CI`
37223480185 at `9780407` runs nineteen jobs on a `push` to `main`. The real
mechanism, read from `.github/workflows/ci.yml` rather than inferred, is the
`changes` job (`name: Changed paths`): it classifies the push's own diff and
exports `code`, and every test, perf and GPU job carries
`if: needs.changes.outputs.code == 'true'`, with one job guarded on `'false'`
so that a required check still reports. A docs-only push to `main` therefore
skips the nine and runs six; a push that touches code runs them all. So
`15 = 6 + 9` is the shape of a **registry push**, not of a push.
`Docs consistency` succeeded,
which is what asserts both commits' tallies. `Grammar fuzz` 37209683763 at
the same head is green.

Review 3's Low 2 was written against this run: the owner's close, and resume
§5 and PR #309's body after it, read it as "green, 15 of 15", having grouped
the jobs endpoint's rows by `status` --- where every row of a finished run
reads `completed` --- rather than by `conclusion`. The four registry pushes
before it are each recorded "green 15 = 6 + 9 skipped", so the correct form
stood in the rows above the wrong one. Both records are corrected and this
paragraph is the file's copy of the figure.

#### `scripts/gate` `20261004T145947Z-40985` at `db697a2`: #311's second and #312's first

Five of six. Sweep: 188 targets, 5,207 passed, 2 failed, 62 ignored; no
daemon survived, and the other five stages ok. The two failures:

- **#311's second occurrence**, in its list above: the laptop's GHC, expected
  on every local gate here until `ghc-static` is installed or the fixture
  passes `-dynamic`.
- **#312's first sample**, below.

#### #312, first sample, local

`-p pmacs --lib process::tests::pty_mode_child_sees_a_tty`, with the fragments
`tty(1) should report a pty path in PTY mode; got ""` and `test result:
FAILED. 2250 passed; 1 failed; 12 ignored`. The panic site `src/process.rs:3845`
moves with the file and is not a fragment. No row named this selector, so the
round filed it as **#312**. `src/process.rs` has no diff on `db697a2` against
`a013d46`, which is an argument from untouched files and not a measurement.

It is filed alone and not folded into **#289**
(`m6_1_pty_mode_lifecycle_started_then_exited`, 2026-09-26), because the
matching rule keys a row to its selector and fragments and these differ. Read
from the code the two share a mechanism, which neither has measured: both call
`drain_until(&mut sup, id, Duration::from_secs(5), has_exited)` and then assert
on the PTY's stdout, and `has_exited` ends the drain on the first `Exited` or
`Signaled` event, while the output reaches the queue through a reader thread ---
so an exit reaped before that thread delivers its line ends the drain with the
output in flight. Ten rows in `src/process.rs` wait that way. What would settle
it is a drain that waits for the reader's EOF as well as the exit, or a second
sample under the same fragments; until one of those, the shared mechanism is a
reading and the rows stay separate. #312 is at one.

So, at E7i's review round 3: a third run of #311 is read and #312 is at one; R7 passed
its row in this gate, which is non-reproduction and nothing more, and #283,
#291, #256, #282, #298, #300 and #303 to #308 are not sampled. Every other
count above stands.

### E7i's fix round 3: the head at `b1c36d7`, `0c7aa43`'s push, and #291's eleventh with #311's condition holding

Recorded 2026-10-04 by E7i's fix round 3, on review 3's two observations ruled
into a round. Each run is read from the jobs endpoint by conclusion, at
attempt 1 of 1.

#### The push of `0c7aa43` to `main`

Tally (push-0c7aa43-jobs): 15 = 6 + 9.

`CI` 37215279845 at `0c7aa43`, attempt 1 of 1, conclusion success: six jobs
succeeded and nine were skipped, as every `push` run to `main` does. `Docs
consistency` succeeded, which is what asserts that commit's tallies. `Grammar
fuzz` 37215279899 at the same head is green, both of its jobs success.

#### PR #309's runs at the round's head

Tally (pr309-fix3-runs): 2 rows in the table below.

| run | workflow | head | verdict |
|---|---|---|---|
| 37219072957 | CI | `b1c36d7` | success |
| 37219072953 | Grammar fuzz | `b1c36d7` | success |

Tally (run-37219072957-jobs): 19 = 18 + 1.

`CI` 37219072957: eighteen success and `Docs consistency` skipped; no failure.
Each test leg's `ok` count, read from its log with its `running` lines paired
(each macOS leg's log carries one more `running` line, the workflow's own
`grep -q 'running 1 test'` in its adapter probe):

| leg | job | `ok` |
|---|---|---|
| `Test (crdt)` | 111485550287 | 180 |
| `luajit, no crdt` | 111485550431 | 181 |
| `ubuntu lua54` | 111485550361 | 181 |
| `ubuntu luajit` | 111485550345 | 181 |
| `macos lua54` | 111485550403 | 182 |
| `macos luajit` | 111485550427 | 182 |

Zero `FAILED`, and the round's six new rows `ok` on every leg. #283's row passed
on both macOS legs, which is non-reproduction and nothing more.

#### `Grammar fuzz` 37219072953 at `b1c36d7`: green on both legs

`Grammar fuzz` 111485522860, 56 min, and `Grammar fuzz (asan-strict)`
111485522670, 51 min: no crash, no hang, no memory cut, no unconfirmed and no
accepted finding. The replay sent 941 and 942 inputs through each arm's worker
(929 and 927 answered, 12 and 15 contained) and crashed none. All five
`fuzz/regress/markdown/` inputs were contained on both legs: the underscores
and the asterisks by the worker's watch, 596, 7672 and the nested openers
killed at the deadline. Slow findings: five under `ubsan` (cmake three, cuda,
python) and six under `asan-strict` (cmake three, cuda, lua, python).

#### `scripts/gate` `20261004T170334Z-438134` at `b1c36d7`: #291's eleventh, not #311

Five of six. Sweep: 189 targets, 189 `running` lines and 189 result lines,
5,219 passed, 1 failed, 62 ignored; no daemon survived, and the other five
stages ok. The one failure is
`e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` on all
three of **#291's** fragments: `HLS reports the type error; store []`, a trace
reaching `LSP:ready·setting` and never `processi`, and `4 passed; 1 failed; 0
ignored`, 126.00 s. It is **#291's eleventh**, in its list above, and is
commented on #291. It is not #311's shape, whose store holds `Could not find
module ‘Prelude’` and whose trace reaches `processi`.

#311's condition held all the same. Checked at the gate's close, the system
`/usr/bin/ghc` 9.6.7-1, with no `ghc-static`, still fails `main = print (1 ::
Int)` with `Could not find module ‘Prelude’`. So the sentence above, at E7i's
fix round 2, that #291 "cannot be sampled on this laptop while #311 stands,
since #311 takes the same row first", is **falsified**. When the server never
processes the typed edit, the row fails before the GHC break can show, on
#291's fragments. #311's list of gate runs read is unchanged at three, since
this run's red is not its shape. #312's row ran `ok`, which is
non-reproduction and nothing more.

So, at E7i's fix round 3: #291 moves to eleven; #311 is not sampled and its
condition stands; #312 stays at one; #283 and R7 passed their rows, which is
non-reproduction; #256, #282, #298, #300 and #303 to #308 are not sampled.
Every other count above stands.

### `main` after E7i: run 37223480185 at `9780407`, RED on attempt 1, green on attempt 2

Read 2026-10-04 at the merge's close from the jobs endpoint by `conclusion`
and from the job logs. `9780407` is E7i's squash merge (PR #309 at `b1c36d7`,
merged 18:11:55Z, pinned with `--match-head-commit`).

Tally (main-after-e7i-attempt1): 19 = 17 + 1 + 1.

| field | value |
|---|---|
| run | 37223480185, `push`, **two attempts** |
| attempt 1 | `cancelled`: seventeen success, `Docs consistency` skipped, and `Test (macos-latest / lua54)` (job 111498310951) cut by its own `timeout-minutes: 45` |
| attempt 2 | `success`: the same job rerun alone (job 111510684534), 19:17:38Z to 19:38:49Z, 182 `test result: ok`, no orphan terminated |
| `Grammar fuzz` | 37223480135 at the same head, green on both legs |

Attempt 1's one red is **#313's first sample**, filed: `r6_every_spawned_fallback_server_is_bounded` in `tests/lean4_server_acceptance.rs` printed its sixty-second notice and then no result line at all, and because `cargo test --all-targets` runs each target's binary in turn, nothing behind it completed either. All 100 of that job's passing lines landed inside the first 10m44s of its test step and none in the thirty minutes after.

**This entry's first draft, and #313's, called it a throughput regression** --- "100 targets in 41 minutes against 182 in 16" --- which is wrong in method: it divided a total by a count while all the lost time sat in one test. Partitioned by timestamp the two runs are the same speed, and to the 99th target `main` was 24 seconds **faster** than the pull request (+10m44s against +11m08s). Corrected before a second sample, on review. The cold `rust-cache` on attempt 1 (`No cache found`, its key taking in `pmacs-parse-unit/Cargo.toml`, which E7i added, so E7h's cache could not match) cost about a minute: attempt 2 hit the cache and still spent 16m22s in its test step against the pull request's 15m52s.

**Attempt 2 is non-reproduction and nothing more**, and a rerun re-executes the same commit, so it is recorded beside attempt 1 rather than in place of it. #313 stays open on one sample.

`main`'s tip after the merge is `60bfee7`, two registry commits above it, whose own `CI` 37223720891 is green at 15 = 6 + 9 skipped and `Grammar fuzz` 37223720906 green --- the registry-push shape described under `a3ea968` above.

### E8's checkpoint: `0736378`'s runs, the phase's local samples, and PR #315's runs

Read 2026-10-05 at C8 from the jobs endpoint by `conclusion` and from the gate
logs. E8 is wire-bearing (v26) and ran `scripts/gate --protocol` at every row's
commit.

`0736378`, `main`'s tip and this branch's base, ran `CI` 37229601069 green, 15 = 6
+ 9 skipped (a registry push), and `Grammar fuzz` 37229601229 green on both legs.
The base control is `9780407`'s, recorded in the section above: attempt 1 red on
#313's first sample, attempt 2 green, which is non-reproduction and nothing more.

Local samples, each in its own block above: **#291's twelfth and thirteenth**
(E8.1's gate and the tip's, `06-sweep`), **R7's twenty-fourth and twenty-fifth** (E8.1's and E8.5's gates,
`07-sweep-luajit`), and #311 in every E8 gate (its list). One red that was no
intermittent: E8.1's `07-sweep-luajit` failed both rows of
`e7b_gpu_chord_acceptance` at the attach (status 4, a daemon without `crdt`
advertising no semantic render), deterministically, because that suite alone of
the ten that drive the GPU probe carried no `cfg(feature = "crdt")`; no `--protocol`
gate had run since E7b added it. Fixed on the branch (`427489d`). Two gate runs are
no evidence: `20261004T201733Z-1108366` swept a mutated build its author had
restored with `cp -p`, keeping the file's old mtime, and was stopped; and
`20261004T212802Z-1897687` was stopped after `06-sweep` showed a count pin of the
branch's own (`theme_faces_acceptance`, fixed on the branch).

PR #315's runs at its head `1ace8b3`, each attempt 1 of 1:

Tally (pr315-ci): 19 = 17 + 1 + 1.

- `CI` 37244883640: `cancelled`, seventeen success, `Docs consistency` skipped, and
  `Test (macos-latest / lua54)` (111560721445) cut by its own 45-minute timeout.
  That is **#313's second sample**, the first's shape: `tests/lean4_server_acceptance.rs`
  began at 00:00:27Z and its other rows passed by 00:00:33.9Z, then
  `r6_every_spawned_fallback_server_is_bounded has been running for over 60 seconds`
  at 00:01:32Z and nothing until `The operation was canceled.` at 00:30:40Z; 5,004
  log lines, 103 `test result: ok`, no `FAILED`. Commented on #313, not rerun. #313
  is at two.
- `Grammar fuzz` 37244883728: green on both legs.

### E8's review round 1 and fix round 1: #291's fourteenth, #316's first, R7 at twenty-seven, and PR #315's runs at three heads

Read 2026-10-05 at E8's fix round 1 from the gate logs and from the jobs
endpoint by `conclusion`. Review round 1 ran no CI of its own: its probes are on
`e8/review-1` at `d15d4b5` with no pull request, and the fix round committed
them to the branch as they were left.

#### `scripts/gate --protocol` `20261005T081704Z-3789356` at `1ace8b3` (review 1's gate)

Six of eight. `06-sweep`: 192 targets, 5,266 passed, 1 failed, 62 ignored, the
one #311's (its list above). `07-sweep-luajit`: 192 targets, 4,861 passed, 2
failed, 48 ignored, the two **#291's fourteenth** (its list above: `store []`,
the trace reaching `LSP:ready·setting` and never `processi`) and **#316's first
sample**, below. No daemon survived. **R7 passed its row in both sweeps**
(`managed_retry_survives_transients_and_uses_the_successful_stream ... ok`, and
no `transient sequence must attach` in either log), so this gate adds no R7
occurrence and R7 stays at twenty-five. The logs are copied to
`~/build/e8-review1/gate-logs-20261005T081704Z-3789356/`.

#### #316, first sample, local

`-p pmacs --test e7i_review3_probes
e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`,
in the `07-sweep-luajit` stage of the gate above, with the fragments `kill 1:
nothing held the switch's follow-up parse, which started a worker at once and
installed`, a report line holding `tree=true`, `unit=pid_`, `death=killed` and a
mode line still reading `parse:killed`, and `test result: FAILED. 10 passed; 1
failed; 0 ignored`. The panic site `tests/e7i_review3_probes.rs:853` moves with
the file and is not a fragment. The same row passed in the same run's
`06-sweep`. No row named this selector, so the review filed it as **#316**.

Its candidate mechanism, from the issue and read from the code rather than
measured: the row waits on the worker slot's report, which shows the follow-up
parse installed, while the mode-line mark is cleared by the runtime's Lua
settle a tick later. E8 adds a call at the head of `paint_frame` and a
`process.after-tick` subscriber, so the branch is not argued innocent. #316 is
at one: the row passed in both sweeps of E8's fix round 1's gate (below),
which is non-reproduction and nothing more.

#### `scripts/gate --protocol` at `44dfb24`, `7505eea` and `57a980e` (fix round 1's gates)

`20261005T182116Z-597588` at the round's first head `44dfb24`, six of eight,
the tree untouched for its run. `06-sweep`: 194 targets (review 1's two suites added), 5,292 passed, 1 failed,
62 ignored. `07-sweep-luajit`: 194 targets, 4,873 passed, 1 failed, 48 ignored.
Both reds are **#311** (its list above: the store holding `Could not find module
‘Prelude’`, the trace reaching `processi`), so #291 is not sampled here. R7's,
#316's, #312's and #313's rows passed in both sweeps: non-reproduction and
nothing more. No daemon survived.

The round's second head, `7505eea`, changed three test files after the first
head's CI (below); its gate, `20261005T193708Z-986096`, is six of eight too. `06-sweep`: 194 targets, 5,291
passed, 2 failed, 62 ignored, the two #311 and **R7's twenty-sixth** (its list
above). `07-sweep-luajit`: 194 targets, 4,873 passed, 1 failed, 48 ignored, the
one #311. #316's row passed in both sweeps. No daemon survived.

The third head, `57a980e`, changed one test file after `7505eea`'s CI (below):
`20261005T213945Z-1605050`, six of eight. `06-sweep`: 194 targets, 5,292 passed,
1 failed, 62 ignored, the one #311. `07-sweep-luajit`: 194 targets, 4,872 passed,
2 failed, 48 ignored, the two #311 and **R7's twenty-seventh** (its list above).
#316's row passed in both sweeps. No daemon survived. The three gates' logs are
copied to `~/build/e8-fix1/gate-logs/`.

#### PR #315's runs at `44dfb24`, `7505eea` and `57a980e`

At the round's first head `44dfb24`, each attempt 1 of 1:

Tally (pr315-fix1-ci-44dfb24): 19 = 13 + 5 + 1.

- `CI` 37358723033: `failure`, thirteen success, five failed and
  `Docs consistency` skipped. The five are test legs, and every red is a row
  this round had just committed, none a registry row:
  - `review1_switching_the_windows_buffer_closes_the_popup`
    (`tests/e8_review1_probes.rs`, "the window switched buffers") on `Test (crdt)`
    111927843329, both macOS legs (111927843820, 111927843263) and
    `ubuntu luajit` 111927843611: review 1's row read the switch's snapshot from a
    fixed 2 s window, which those runners did not meet;
  - every rust-analyzer row of `tests/e8_review1_real_server_probes.rs` on both
    macOS legs, the server initializing (`positionEncoding "utf-8"`) and answering
    no hover or signature help in 120 s: filed as **#317**;
  - `fr1_c_c_h_opens_the_last_popup_after_a_motion_closed_it` on `ubuntu luajit`
    and `luajit, no crdt` (111927843487), which waited out one `C-c H` that opened
    neither `*lsp-help*` nor said "no hover info" within 60 s.

  All three are the round's own. `7505eea` fixed two (the `C-c H` row presses
  again; the rust-analyzer rows run on Linux, #317) and made the switch and kill
  rows wait for the snapshot, which did not fix the switch row (below); `57a980e`
  did. `ubuntu lua54` 111927843352 passed all
  186 targets. Each leg's `running` lines pair with its result lines. #313's row
  passed on both macOS legs, which is non-reproduction and nothing more.
- `Grammar fuzz` 37358723025: green on both legs (111927636475, 111927636793), the
  skip path for a branch that changes no grammar.

At the head `7505eea`, which changed three test files after those runs, `CI`
37364522422 and `Grammar fuzz` 37364522424 ran nothing of the code in their first
two attempts. Every job that was not skipped was `cancelled` with the annotation
"The job was not acquired by Runner of type hosted even after multiple attempts",
during GitHub's incident of that evening ("delays in assigning GitHub-hosted
runners"); the skipped jobs are those that wait on `Changed paths`.

Tally (pr315-fix1-ci-7505eea-attempt1): 15 = 0 + 5 + 10.

Tally (pr315-fix1-ci-7505eea-attempt2): 15 = 1 + 4 + 10.

Each reads success, cancelled and skipped: attempt 1 cancelled `Changed paths`,
`Commit attribution (D9)`, `Format` and both lints; attempt 2 ran `Format` green
and cancelled the other four. `Grammar fuzz` cancelled both its jobs in both
attempts. The reruns were taken because no job had run at all: there was no red
to rerun away.

The third attempt ran in part, the incident still open:

Tally (pr315-fix1-ci-7505eea-attempt3): 19 = 8 + 7 + 3 + 1.

Success, cancelled (unacquired, as before), failure and skipped (`Docs
consistency`). `ubuntu lua54` 111970313598 passed all 186 targets. The three
failed legs carry two rows of the round's own, fixed on the branch at `57a980e`:

- review 1's buffer-switch row again, on both macOS legs (111970313460,
  111970313437) and `ubuntu luajit` 111970313520, now waiting 20 s for the
  snapshot: the wait was never the cause. `C-x b b.rs RET` kept the window on
  `a.rs` --- buffers are named by path, and `b.rs` matched `a.rs`'s path as a
  subsequence --- filed as **#318** and measured in-process; the row now switches
  with `C-x <right>`;
- `fr1_a_v25_session_is_told_the_signature_its_typing_asks_for` on `macos lua54`,
  whose v26 control typed `(` before the fake server had initialized; it now asks
  a hover first.

`Grammar fuzz` attempt 3: `Grammar fuzz` 111968420608 green, `Grammar fuzz
(asan-strict)` 111968420806 cancelled unacquired. Not rerun: the fix went out as
a new head.

At the third head `57a980e`, each attempt 1 of 1, both green:

Tally (pr315-fix1-ci-57a980e): 19 = 18 + 1.

- `CI` 37377329510: `success`, eighteen jobs and `Docs consistency` skipped. The
  six test legs: `Test (crdt)` 111989867116 185 `test result: ok`, both macOS legs
  (lua54 111989867379, luajit 111989867641) 187 each, `ubuntu lua54`
  111989867318, `ubuntu luajit` 111989867420 and `luajit, no crdt` 111989867414
  186 each; zero `FAILED`, every leg's `running` lines paired with its result
  lines. #313's row passed on both macOS legs, which is non-reproduction and
  nothing more.
- `Grammar fuzz` 37377329537: green on both legs (111989781295, 111989780931).

So, at E8's fix round 1: #291 moves to fourteen and R7 to twenty-seven; #316 is
at one and #311's list of runs read at fifteen; #317 and #318 are filed from the
round's CI and are not intermittents (#317 written here as a platform's
provisioning, which E8's review round 2 falsified: it is pmacs's, a canonical
root beside a document URI that keeps a symlink, and macOS's `TMPDIR` is one;
#318 a switcher's ranking); #313's row passed in every run here, and #313 stays
at two. Every other count above stands.


### E8's review round 2 and fix round 2: R7 at twenty-nine, #316's second, #283's twelfth and thirteenth, and PR #315's runs at `d2de579`

Read 2026-10-06 at E8's fix round 2 from the gate logs and from the jobs
endpoint by `conclusion`. Review round 2 ran no CI of its own: its probes are on
`e8/review-2` at `617f9ff` with no pull request (no run for that commit), and the
fix round fast-forwarded the branch onto them as they were left.

#### `scripts/gate --protocol` `20261005T224931Z-2046695` at `57a980e` (review 2's gate)

Six of eight. `06-sweep`: 194 targets, 194 `running` and 194 result lines,
5,292 passed, 1 failed, 62 ignored, the one #311's (its list above: the store
holding `Could not find module ‘Prelude’`). `07-sweep-luajit`: 194 targets,
4,872 passed, 2 failed, 48 ignored: #311 and **R7's twenty-eighth** (its list
above). #291 is not sampled, #311 holding the row. #316's, #312's and #313's rows
passed. No daemon survived. The logs are copied to
`~/build/e8-review2/gate-logs/`.

#### `scripts/gate --protocol` at `d2de579` (fix round 2's gates)

`20261006T104137Z-2974993` is no verdict: the laptop restarted at 13:03 local
during its `07-sweep-luajit`, at 69 of 197 targets, and the stage's `cargo test`
ended on `Hangup`. Its five stages before were ok, and its
`06-sweep` completed: 197 targets (review 2's three suites added), 197 `running`
and 197 result lines, 5,311 passed, 1 failed, 64 ignored (the two rows the round
holds ignored for #317), the one #311. R7's row passed in it. The partial
luajit sweep's one red is #311 again. No process carrying the run's `TMPDIR`
survived it.

`20261006T111111Z-85507`, the round's gate at the same head, the tree untouched
for its run: six of eight, fmt, clippy, clippy-luajit, doc, build and diff-check
ok, and no daemon survived, identified by environ. `06-sweep`: 197 targets, 197
`running` and 197 result lines, 5,310 passed, 2 failed, 64 ignored: #311 and
**#316's second sample** (below). `07-sweep-luajit`: 197 targets, 4,885 passed, 2
failed, 50 ignored: #311 and **R7's twenty-ninth** (its list above). #312's and
#313's rows passed in both sweeps, #316's in the luajit one and R7's in the
first. The logs of both gates are copied to `~/build/e8-fix2/gate-logs/`.

#### #316, second sample, local

The same selector as its first, `-p pmacs --test e7i_review3_probes
e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`,
in the `06-sweep` stage of `20261006T111111Z-85507`, with all its fragments: `kill
1: nothing held the switch's follow-up parse, which started a worker at once and
installed`, a report line holding `tree=true`, `unit=pid_`, `death=killed` and a
mode line still reading `parse:killed`, and `test result: FAILED. 10 passed; 1
failed; 0 ignored`. The first sample was in a `07-sweep-luajit`; this one is in
the default sweep, so neither flavor is the discriminator. `tests/e7i_review3_probes.rs`
has no diff on the branch, and the round touched neither `paint_frame` nor the
`process.after-tick` subscriber the first sample's entry names. Commented on the
issue.

#### PR #315's runs at `d2de579`

Each attempt 1 of 1 when read here. `CI` 37451551936's two failed macOS legs
were rerun at E8's fix round 3 on the owner's instruction; this entry is its
attempt 1, and attempt 2 is recorded at that round, below.

Tally (pr315-fix2-ci-d2de579): 19 = 16 + 2 + 1.

- `CI` 37451551936: `failure`, sixteen success, two failed and `Docs
  consistency` skipped. The two are both macOS test legs, `Test (macos-latest /
  lua54)` 112229134676 and `Test (macos-latest / luajit)` 112229134735, each on
  **#283** with all three of its fragments and nothing else: 187 `test result:
  ok` and the one `FAILED`, 188 `running` lines paired with 188 result lines.
  They are #283's twelfth and thirteenth (its list above), the first run with
  both macOS legs red on it. The branch's diff from `57a980e`, whose run passed
  the row on both legs, is the signature-label parse, the kept hover's place and
  revision, tests and a comment in `scripts/gate`; that is read, not a
  demonstration of innocence. Commented on the issue, not rerun. The four Linux
  test legs are green: `Test (crdt)` 112229134544 188 `test result: ok`, `ubuntu
  luajit` 112229134630, `ubuntu lua54` 112229134702 and `luajit, no crdt`
  112229134608 189 each, zero `FAILED`. #313's row passed on both macOS legs,
  which is non-reproduction and nothing more. On the macOS legs the
  rust-analyzer rows still skip (#317 stands; review 2's real-server suite 6
  passed and 2 ignored in 4.72 s there, 22 to 35 s on the Linux legs).
- `Grammar fuzz` 37451551922: `success` on both legs (`Grammar fuzz`
  112229063549, `Grammar fuzz (asan-strict)` 112229063267), the skip path for a
  branch that changes no grammar.

So, at E8's fix round 2: R7 moves to twenty-nine (the review's gate and the
round's), #283 to thirteen and #316 to two; #311's list of runs reads at
eighteen; #291 is not sampled, #311 holding its row, and #313 stays at two; #317
is a product defect and not an intermittent (above). Every other count above
stands.


### E8's fix round 3: `CI` 37451551936 at `d2de579`, attempt 2, #283's fourteenth and #319's first

Read 2026-10-06 at E8's fix round 3 from the jobs endpoint by `conclusion` and
from the two rerun legs' logs. The round changed no code and ran no gate (its
first item was written early, for the owner's ruling), so its only run is the
owner's instructed rerun of the head run's two failed macOS legs. The rerun
checked out the merge attempt 1 tested: all four macOS logs, both attempts on
both legs, read `HEAD is now at b545c81 Merge d2de579… into 8b25ed8…`.

Tally (pr315-fix3-ci-d2de579-attempt2): 19 = 16 + 2 + 1.

- `CI` 37451551936 attempt 2, rerun at 11:50:43Z and completed at 12:24:28Z:
  `failure`. Both macOS test legs ran again and both failed; the other
  seventeen keep attempt 1's conclusions, sixteen success listed under new ids
  with attempt 1's times and `Docs consistency` 112253936735 skipped.
  - `Test (macos-latest / luajit)` 112253935087: **#283's fourteenth** (its
    list above), all three fragments, its only failure against 187 `test
    result: ok`, 188 `running` lines paired with 188 result lines. The same
    leg's attempt 1, 112229134735, was #283's thirteenth: one commit red on
    the row twice on this leg.
  - `Test (macos-latest / lua54)` 112253934977: #283's row passed, which is
    non-reproduction and nothing more (attempt 1's 112229134676 was its
    twelfth). The leg failed instead on
    `e7c_indicator_acceptance::e7c_fix_3_a_request_answered_under_the_threshold_never_appears`,
    `and never reached the indicator: [(52, "⋯1 parse rust …")]` and
    `test result: FAILED. 4 passed; 1 failed; 0 ignored`, its only failure
    against 187 `test result: ok`, 188 `running` lines paired with 188 result
    lines. No row here matches it: #305 is the same suite's typing row with its
    own fragment, though its indicator also showed `⋯1 parse rust`. Filed as
    **#319**, first occurrence, and not widened into #305. The row passed on
    both legs in attempt 1 and on luajit in attempt 2. The branch's diff touches
    the hover command the row invokes and not the parse path or the indicator;
    that is read, not a demonstration.
  - #313's row passed on both legs, non-reproduction and nothing more.
    `WouldBlock` and `did not become ready` appear zero times in either log.
    The two E8 real-server suites again report their rust-analyzer rows `ok`,
    though off Linux each returns before starting a server while #317's skip
    stands: review 1's suite 9 passed in 0.01 s, review 2's 6 passed and 2
    ignored in 8.14 s (lua54) and 3.93 s (luajit), as in attempt 1.
- `Grammar fuzz` 37451551922 was not rerun: attempt 1 of 1, green (above).

Not rerun again, on the owner's instruction. So, at E8's fix round 3: #283
moves to fourteen and #319 is at one; #313 stays at two; R7, #291, #311 and
#316 are not sampled, no gate having run. Every other count above stands.

### The required-checks PR: PR #321's runs and #322's first, its gate, #283 reproduced on demand, and #320's first

Recorded 2026-10-06 with PR #321
(`ci/required-checks-mean-what-they-say`, five commits on `97956ff`), which is not a
phase: it changes the rows of #283, #313 and #319 and makes each test leg print its
arming and the rows that returned without running. Its runs were read
from the jobs endpoint by conclusion and through all six test legs' logs; neither was
rerun.

#### `CI` 37486414088 at `e23510a`, attempt 1 of 1: #322's first

`pull_request`, the merge `a6b2df8` (`e23510a` into `97956ff`, read from all six test
logs' checkout line), created 15:18:46Z and completed 15:52:33Z: `failure`.

Tally (pr321-ci-e23510a): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 112347460053 | success |
| Format | 112347459741 | success |
| Changed paths | 112347459987 | success |
| Lint (lua54) | 112347460028 | success |
| Lint (luajit) | 112347460070 | success |
| GPU Render (headless) | 112347630475 | success |
| M10 Perf Gates (crdt) | 112347630562 | success |
| Test (crdt) | 112347630574 | success |
| M1 Acceptance Gates | 112347630634 | success |
| M4 Perf Gates | 112347630655 | success |
| Test (ubuntu-latest / luajit, no crdt) | 112347630662 | success |
| Test (ubuntu-latest / lua54) | 112347630704 | success |
| M5 Perf Gates | 112347630719 | success |
| Test (macos-latest / luajit) | 112347630743 | failure |
| Test (macos-latest / lua54) | 112347630802 | success |
| M6 Perf Gates | 112347630815 | success |
| Perf budgets (debug) | 112347630824 | success |
| Test (ubuntu-latest / luajit) | 112347631300 | success |
| Docs consistency | 112347632844 | skipped |

Tally (pr321-ci-e23510a-failures): 1 row of the table above with `result` = `failure`.

- `Test (macos-latest / luajit)`, its only failure against 179 `test result: ok`:
  `e7i_review3_probes::e7i_review3_the_editor_s_own_sigkill_and_an_outside_one_are_told_apart`,
  `the deadline's kill did not become ready within 60s` with the last report holding
  `deaths=3`, `busy=false`, `pending=0`, `tree=true` and `death=killed`, and `test
  result: FAILED. 10 passed; 1 failed; 0 ignored`. The panic site
  `tests/e7i_review3_probes.rs:918:9` and the unit's pid are not fragments. No row
  here names it, so it is filed as **#322**, first occurrence, and not rerun. The
  same row passed on the other five legs. The branch changes nothing under `src/`,
  `builtin/` or that suite; that is read, not a demonstration.
- The rows the branch changes passed on every leg that builds them: #283's
  (`gpu_route::…`, five legs, not the no-crdt one, which compiles it out), #313's
  and #319's, and the two new rows, which is non-reproduction and nothing more for
  #313 and #319.
- Each leg's `running` lines pair with its result lines: 180 on `Test (crdt)`, 181 on
  each Ubuntu leg, and on the macOS legs 180 and 182, each log's one unpaired
  `running 1 test` being the adapter step's own `grep` text. `WouldBlock` and `got
  ok` appear on no leg; `did not become ready` once, #322's own line.
- **The legs' new steps, read in every log.** `Arming report (this leg)` printed
  each variable as the leg set it: on the Ubuntu legs `LSP`, `SHELLS`, `LUA`,
  `SETSID` and `BASH` armed with their tools present, and `PYRIGHT`, `HLS` and
  `CGROUP` unset with their tools absent; on the macOS legs every variable unset,
  `BASH` and `CARGO_BUILD` with their tools present; on `Test (crdt)` `GPU`,
  `SETSID` and `BASH` armed. `Rows that returned without running` then listed 2
  rows on each Ubuntu leg (basedpyright's and the HLS row), 14 on each macOS leg and
  17 on `Test (crdt)`, each by suite, row, call site, tool and variable.

#### `Grammar fuzz` 37486414312 at `e23510a`

Attempt 1 of 1, `success`: `Grammar fuzz` 112347460903 and `Grammar fuzz
(asan-strict)` 112347460472, eleven seconds each: the job's path for a change that
touches no grammar, which fuzzes nothing (`scripts/grammar-fuzz-needed`, `ci.yml`
among its non-grammar paths).

#### The tip's gate, `20261006T150249Z-1162130` at `e23510a`

Five of six, the default plan: fmt, clippy, doc, build and diff-check ok, and no
daemon survived. `05-sweep`: 189 result lines, 5,221 passed, 1 failed, 62 ignored.
The one is #311's row with #311's fragments (the store holding `Could not find module
‘Prelude’`, the trace reaching `processi`), so #311's list gains this gate. R7,
#291 and #316 did not fire, which is non-reproduction and nothing more. The rows of
#283, #313 and #319 passed in it, in their new forms.

#### #283 reproduced on demand, twice, and not counted

Two variants of `main`'s row, each from test-side Lua, failed with #283's three
fragments and its message character for character, `pump timeout waiting for the
accept and its import; text="fn main() {\n    println\n    \n}\n// tail\n"
popup_rows=0 anchor=None cursor=Some(28)` at `tests/e7_review1_probes.rs:774:13`:
with the popup's first open held 2.5 s (11.67 s), and with the fake server started
2 s late, so that the letters met a server not yet initialized (11.71 s). The
fragments cannot tell those two apart, and which one the macOS runners met is not
known. These were made, not met, so they are not occurrences, and #283 stays at
fourteen. The row as PR #321 changes it passed both variants (2.69 s and 2.22 s).

#### #320, first sample, local, outside the gate

`e7_review1_probes::e7_review1_a_late_answer_past_the_default_bound_never_touches_an_edited_buffer`,
in a hand run in the gate's environment on the PR's branch before its first commit
(the log closed 15:12:16+02:00, load average 15.65 on 16 threads from other
builds on the machine): `answered in time: 11.062716943s` at the row's second save, whose
bound is 3000 ms, and `test result: FAILED. 10 passed; 1 failed; 0 ignored`. The
panic site `tests/e7_review1_probes.rs:330:5` moves with the file and is not a
fragment. No row here names the selector, so it is filed as **#320**. A rerun of the
suite failed the same row at its first save with another fragment (`the save waits
the default bound and gives up before the answer: 5.08476519s`), but that run was in
flight across the laptop's suspend (`Suspending...` 15:12:57, `PM: suspend exit`
16:39:59, its log closed 16:39:59.7), so it is no evidence and is not counted; the
other two reruns passed. The branch's diff in that suite is #283's row only, and the
failing row's path is untouched; that is read, not a demonstration. The row passed
in the tip's gate above.

So, at PR #321: #283 stays at fourteen, not moved by the two reproductions; #320 is
at one; #311's list of runs reads at nineteen; #313 stays at two and #319 at one, their rows passing on
every leg; #322 is at one; R7,
#291 and #316 are not sampled. Every other count above stands.

### The required-checks PR's second head: `ab06caa`'s runs, #306's second and #323's first, #322 folded in, and #283, #313 and #319 closed

Recorded 2026-10-06 with PR #321 at `ab06caa` (six commits on `97956ff`): the fifth row of
the class, #322's, given an input no deadline races. Its runs were read from the jobs endpoint
by conclusion and through all six test legs' logs; neither was rerun.

#### `CI` 37498475119 at `ab06caa`, attempt 1 of 1: #306's second and #323's first

`pull_request`, the merge `a97734b` (`ab06caa` into `9b2dc8b`, read from all six test logs'
checkout line), created 16:47:30Z and completed 17:20:59Z: `failure`.

Tally (pr321-ci-ab06caa): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 112389030032 | success |
| Format | 112389030503 | success |
| Lint (luajit) | 112389030633 | success |
| Lint (lua54) | 112389030667 | success |
| Commit attribution (D9) | 112389030779 | success |
| M1 Acceptance Gates | 112389146620 | success |
| M5 Perf Gates | 112389146639 | success |
| GPU Render (headless) | 112389146736 | success |
| M4 Perf Gates | 112389146780 | success |
| Perf budgets (debug) | 112389146842 | success |
| Test (crdt) | 112389146849 | success |
| Test (ubuntu-latest / luajit) | 112389146897 | success |
| M10 Perf Gates (crdt) | 112389146900 | success |
| Test (ubuntu-latest / lua54) | 112389146951 | success |
| M6 Perf Gates | 112389146966 | success |
| Test (macos-latest / lua54) | 112389147010 | success |
| Test (macos-latest / luajit) | 112389147024 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 112389147113 | success |
| Docs consistency | 112389148171 | skipped |

Tally (pr321-ci-ab06caa-failures): 1 row of the table above with `result` = `failure`.

- `Test (macos-latest / luajit)`, two failures against 178 `test result: ok`:
  - `compile_mode_acceptance::acc05_kill_reaps_backgrounded_descendant`, with #306's three
    fragments (`kill must produce a signaled exit marker; buffer:`, the shell's
    `Terminated: 15          sleep 30` and no `[compile killed by …]` marker, and `test
    result: FAILED. 74 passed; 1 failed; 0 ignored`): **#306's second occurrence**, the first
    on this leg (its first, `CI` 36927231233, was on lua54). The buffer ends `[compile exited
    with code 0]`. Commented on #306, not rerun.
  - `e7i_review1_probes::e7i_review1_parse_now_on_a_buffer_mid_parse`, `the report did not
    reach the expected state in 120s; last:` with nothing written, no `_parse_now mid-parse in
    none:` line in the log, and `test result: FAILED. 21 passed; 1 failed; 0 ignored`. No
    row here names it, so it is filed as **#323**, first occurrence, not rerun. On the legs
    where it finished, its `none` mode blocked the main thread 30,571 to 100,920 ms across this
    run and PR #321's first, against the row's 120 s wait, the macOS luajit leg's 100,920 at
    `e23510a` the nearest.

  The branch changes nothing under `src/` or `builtin/`, nor either suite but #283's row; that
  is read, not a demonstration.
- The rows of #283, #313, #319 and #322 and the PR's two new rows passed on every leg that
  builds them (#283's on five, the no-crdt leg compiling it out). Each leg's `running` lines
  pair with its result lines, the macOS legs' one unpaired line apiece being the adapter
  step's `grep` text. `WouldBlock`, `did not become ready` and `got ok` appear on no leg.
  The legs' new steps printed as at `e23510a`: 2 skips on each Ubuntu leg, 14 on
  each macOS leg and 17 on `Test (crdt)`.

#### `Grammar fuzz` 37498475125 at `ab06caa`

Attempt 1 of 1, `success`: `Grammar fuzz` 112389024126 and `Grammar fuzz (asan-strict)`
112389023611, the path for a change that touches no grammar.

#### The tip's gate, `20261006T164749Z-1441387` at `ab06caa`

Five of six, the default plan: `05-sweep` read 189 result lines, 5,221 passed, 1 failed, 62
ignored, the one #311's row with #311's fragments, so #311's list gains it. R7, #291 and #316
did not fire, which is non-reproduction and nothing more. #322's row passed in it.

#### #322 folded in, reproduced on demand

`ab06caa` runs the row's fourth parse on #301's nested openers instead of #296's paragraph.
On the laptop `main`'s row failed with #322's fragments exactly (`the deadline's kill did not
become ready within 60s`, the last report `deaths=3 … busy=false … pending=0 … tree=true …
death=killed`) at a 100 ms deadline, and at its own 300 ms with the worker throttled to a
quarter of its speed; the row at `ab06caa` passed under both. These were made, not met, and
#322 stays at one.

#### #283, #313 and #319 closed

Closed at 16:48:30Z, 16:48:33Z and 16:48:36Z, each with a comment naming the mechanism PR #321
removes and the witness that fails without it. Their counts stand as recorded above: #283 at
fourteen, #313 at two, #319 at one.

So, at PR #321's second head: #306 moves to two; #323 is at one; #311's list of runs reads at
twenty; #322 stays at one; R7, #291 and #316 are not sampled. Every other count above stands.

### E8's fix round 4: `05dfa6a`'s runs green on every leg, the base control `main` at `93752c6`, and R7's thirtieth

Recorded 2026-10-06 with PR #315 at `05dfa6a`: the merge `85e9415` (`main` at `93752c6`, PR
#321's squash, into `d2de579`) and one commit that gives E8's off-Linux rust-analyzer rows a
line in the leg's skip record. The runs were read from the jobs endpoint by conclusion and
through all six test legs' logs; none was rerun.

#### `CI` 37508342547 at `05dfa6a`, attempt 1 of 1: green

`pull_request`, the merge `dc0b8d0` (`05dfa6a` into `93752c6`, read from all six test logs'
checkout line), created 18:03:27Z and completed 18:38:30Z: `success`.

Tally (pr315-fix4-ci-05dfa6a): 19 = 18 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 112422644510 | success |
| Lint (luajit) | 112422644595 | success |
| Format | 112422644670 | success |
| Lint (lua54) | 112422644757 | success |
| Commit attribution (D9) | 112422644816 | success |
| Test (crdt) | 112422739584 | success |
| M5 Perf Gates | 112422739589 | success |
| GPU Render (headless) | 112422739593 | success |
| M4 Perf Gates | 112422739650 | success |
| Test (ubuntu-latest / luajit) | 112422739677 | success |
| Test (macos-latest / lua54) | 112422739680 | success |
| M1 Acceptance Gates | 112422739724 | success |
| M10 Perf Gates (crdt) | 112422739727 | success |
| Perf budgets (debug) | 112422739729 | success |
| M6 Perf Gates | 112422739750 | success |
| Test (macos-latest / luajit) | 112422739764 | success |
| Test (ubuntu-latest / luajit, no crdt) | 112422739780 | success |
| Test (ubuntu-latest / lua54) | 112422739862 | success |
| Docs consistency | 112422742325 | skipped |

Tally (pr315-fix4-ci-05dfa6a-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs its `running` lines with its `test result: ok` lines and has no
  `FAILED`: 190 on each macOS leg, 189 on each Ubuntu leg and 188 on `Test (crdt)`.
  `WouldBlock` appears on no leg.
- The rows of #283, #313, #319 and #322 passed on every leg that builds them (#283's on
  five, the no-crdt leg compiling it out), the first run of #315's to carry PR #321's fixes.
  #306's row and #323's passed on all six, which is non-reproduction and nothing more.
- The legs' skip records: 28 on each macOS leg, 21 on `Test (crdt)`, 4 on each Ubuntu leg.
  Of each macOS leg's 28, 14 are E8's two real-server suites: the eleven unheld
  rust-analyzer rows, each at its own call site, `` `rust-analyzer` off Linux while #317
  stands and PMACS_REQUIRE_LSP unset ``, and three rows `skip_or_fail` records (the two
  basedpyright rows and gopls's). The same eleven lines on both legs. Review 1's suite still
  reads 9 passed in 0.01 s there; the record is what says they did not run.

#### `Grammar fuzz` 37508342623 at `05dfa6a`

Attempt 1 of 1, `success`: `Grammar fuzz` 112422647898 and `Grammar fuzz (asan-strict)`
112422648411, the path for a change that touches no grammar.

#### The base control: `main` at `93752c6`, PR #321's squash

`CI` 37506100801, `push`, attempt 1 of 1, completed 18:16:35Z, `success`, 19 = 18 success +
1 skipped (`Docs consistency` 112415122710), every test leg green: `Test (macos-latest /
lua54)` 112415121602, `Test (macos-latest / luajit)` 112415121515, `Test (crdt)`
112415121488, `Test (ubuntu-latest / luajit)` 112415121794, `Test (ubuntu-latest / lua54)`
112415121624, `Test (ubuntu-latest / luajit, no crdt)` 112415121555. Its macOS luajit leg
read 182 paired results and 14 recorded skips, none in an E8 suite, which `main` does not
carry. `Grammar fuzz` 37506101008, `success` on both legs (112415033740, 112415033359).

Tally (main-93752c6-ci): 19 = 18 + 1.

#### The head's gate, `20261006T175402Z-1776889` at `05dfa6a`

`scripts/gate --protocol` over the head's tree (the owner's uncommitted blank line in
`src/lsp.rs` beside it, as at every E8 gate): six of eight, `06-sweep` 197 result lines,
5,313 passed, 1 failed, 64 ignored, and `07-sweep-luajit` 197, 4,887 passed, 2 failed, 50
ignored. Both sweeps failed #311's row with #311's fragments, so #311's list gains the run.
`07-sweep-luajit` also failed `attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`
with R7's three fragments, **R7's thirtieth**. #291 and #316 did not fire, which is
non-reproduction and nothing more.

So, at E8's fix round 4: R7 moves to thirty; #311's list of runs reads at twenty-one; #283,
#306, #313, #319, #322 and #323 do not move, their rows passing. Every other count above
stands.


### `main` after E8: run 37514576127 at `24593f1`, red on #316's third sample, its first in CI

Read 2026-10-06 at E8b's opening (its row E8b.0, before any code) from the jobs endpoint by
`conclusion` and from all six test legs' logs; not rerun. `24593f1` is E8's squash merge
(PR #315 at `05dfa6a`, merged 18:51:55Z with `--match-head-commit`), and the run is a
`push` that checked out `24593f1` itself (the `Test (crdt)` log's fetch line). It is
`main`'s base control for E8b: the last code-bearing commit on `main`.

#### `CI` 37514576127 at `24593f1`, attempt 1 of 1: `failure`

Created 18:51:58Z, completed 19:25:20Z.

Tally (main-after-e8-ci): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 112444085059 | success |
| Lint (luajit) | 112444085313 | success |
| Commit attribution (D9) | 112444085323 | success |
| Lint (lua54) | 112444085410 | success |
| Format | 112444085418 | success |
| M1 Acceptance Gates | 112444170929 | success |
| Test (crdt) | 112444170937 | failure |
| M5 Perf Gates | 112444170949 | success |
| Test (ubuntu-latest / lua54) | 112444171009 | success |
| Test (ubuntu-latest / luajit, no crdt) | 112444171054 | success |
| Test (macos-latest / luajit) | 112444171067 | success |
| M6 Perf Gates | 112444171100 | success |
| Perf budgets (debug) | 112444171101 | success |
| Test (ubuntu-latest / luajit) | 112444171107 | success |
| M10 Perf Gates (crdt) | 112444171163 | success |
| M4 Perf Gates | 112444171184 | success |
| Test (macos-latest / lua54) | 112444171187 | success |
| GPU Render (headless) | 112444171249 | success |
| Docs consistency | 112444174141 | skipped |

Tally (main-after-e8-ci-success): 17 rows of the table above with `result` = `success`.

- Every test leg pairs its `running` lines with its result lines: 190 on each macOS leg and
  189 on each Ubuntu leg, all `ok`; 187 on `Test (crdt)`, 186 `ok` and one `FAILED`.
  `WouldBlock` appears on no leg. `Test (crdt)` read 188 at `05dfa6a`: the extra line was
  its `cargo test --doc` step's `Doc-tests pmacs`, and here that step (step 11) is `skipped`
  after the sweep step failed, read from the job's steps; the 181 `Running` targets of the
  two runs are the same list.
- The legs' skip records are `05dfa6a`'s: 28 on each macOS leg, 21 on `Test (crdt)`, 4 on
  each Ubuntu leg.

#### #316, third sample, its first in CI and its first on `main`

`-p pmacs --test e7i_review3_probes
e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`, in
`Test (crdt)` (Linux, default features, `--test-threads=1`), at
`tests/e7i_review3_probes.rs:853` (not a fragment), `test result: FAILED. 10 passed; 1
failed; 0 ignored`. The assertion is #316's, `nothing held the switch's follow-up parse,
which started a worker at once and installed`, but on the loop's **third** kill:

`kill 3: … deaths=3 crashes=0 busy=true held=false pending=0 stopped=false tree=true
unit=pid_25243 death=killed, " +* … (rust) … parse:killed !3  L1:C1 Top"`

**Two of the issue's fragments as written do not match it**, and the record says so rather
than counting past them. They name the loop's first iteration: `kill 1:`, and a mode line
opening `" +  `, the buffer unmodified before the first edit. Here it is `kill 3:` and
`" +* `, the buffer modified by the two edits before it. The assertion is one template over
`kill {kill}` and the same wait precedes every iteration, so this is counted as #316's
third, with the fragments read as the template (`kill N:`, `" +` then `parse:killed`); the
issue is commented with that reading, and splitting it out is the owner's call.

**It also differs in a way the issue's candidate mechanism does not cover.** Both local
samples' found reports read `busy=false` (`20261005T081704Z-3789356`'s `07-sweep-luajit`,
`unit=pid_3900391`; `20261006T111111Z-85507`'s `06-sweep`, `unit=pid_128600`): the
follow-up parse had installed and the mark outlived it, which is the candidate, the mark
cleared by the runtime's settle a tick after the report shows the install. This one reads
`busy=true` with a fresh unit: a parse was in flight at the read, after `quiesce` had seen
`busy=false` and `pending=0` and ticked five times, so a parse was dispatched during those
five ticks. The same iteration then installed the edit's parse in 18 ms. Both shapes are
the row's wait returning before the state its assertion reads; which object dispatched the
late parse is not measured. The row passed on the other five legs.

`24593f1` carries E8's `paint_frame` call and `process.after-tick` subscriber, which the
issue named as not shown innocent; on `main` they no longer discriminate a branch.

#### `Grammar fuzz` 37514576116 at `24593f1`

Attempt 1 of 1, `success`: `Grammar fuzz` 112444084091 and `Grammar fuzz (asan-strict)`
112444084436.

So, at `main` after E8: #316 moves to three (its first on Linux CI, read under the
template); every other count above stands, and no other row was sampled red.

### E8b's checkpoint: PR #325's runs at `0001946` green on every leg, three gates, and #311's list at twenty-four

Recorded 2026-10-07 at C8b, with PR #325 at `0001946`: three signed commits on `main` at
`422b2c4`, which carries this phase's opening record (`main` after E8, above). The runs were
read from the jobs endpoint by conclusion and through all six test legs' logs; none was rerun.

#### `CI` 37591971331 at `0001946`, attempt 1 of 1: green

`pull_request`, the merge `b618e89` (`0001946` into `422b2c4`, read from the `Test (crdt)`
log's checkout line), created 08:10:07Z and completed 08:47:11Z: `success`.

Tally (pr325-ci-0001946): 19 = 18 + 1.

| job | id | result |
|---|---|---|
| Format | 112695372384 | success |
| Lint (luajit) | 112695372673 | success |
| Commit attribution (D9) | 112695372851 | success |
| Lint (lua54) | 112695372852 | success |
| Changed paths | 112695372874 | success |
| M5 Perf Gates | 112695444281 | success |
| GPU Render (headless) | 112695444345 | success |
| M4 Perf Gates | 112695444351 | success |
| M1 Acceptance Gates | 112695444390 | success |
| Test (ubuntu-latest / lua54) | 112695444415 | success |
| Test (ubuntu-latest / luajit, no crdt) | 112695444464 | success |
| Perf budgets (debug) | 112695444466 | success |
| M10 Perf Gates (crdt) | 112695444474 | success |
| M6 Perf Gates | 112695444496 | success |
| Test (macos-latest / luajit) | 112695444507 | success |
| Test (macos-latest / lua54) | 112695444511 | success |
| Test (crdt) | 112695444549 | success |
| Test (ubuntu-latest / luajit) | 112695444575 | success |
| Docs consistency | 112695446216 | skipped |

Tally (pr325-ci-0001946-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs its `running` lines with its `test result: ok` lines and has no
  `FAILED`: 191 on each macOS leg, 190 on each Ubuntu leg and 189 on `Test (crdt)`, one more
  than `05dfa6a`'s on each, the new suite. `WouldBlock` appears on no leg.
- E8b's rows passed on every leg that builds them: the three in
  `tests/minibuffer_accept_acceptance.rs` and the minibuffer unit rows on all six, the three
  daemon rows on the five crdt legs. #316's row passed on all six, which is non-reproduction
  and nothing more.
- The legs' skip records are `05dfa6a`'s: 28 on each macOS leg, 21 on `Test (crdt)`, 4 on
  each Ubuntu leg.

#### `Grammar fuzz` 37591971313 at `0001946`

Attempt 1 of 1, `success`: `Grammar fuzz` 112695373032 and `Grammar fuzz (asan-strict)`
112695373183, the path for a change that touches no grammar.

#### `main` at `422b2c4`, E8b.0's registry push

`CI` 37536504580, `push`, attempt 1 of 1, `success`, 15 = 6 success + 9 skipped (`Docs
consistency` 112518819432 among the six); `Grammar fuzz` 37536504649 `success`.

#### The phase's gates

`scripts/gate`, the default plan, at each row: `20261006T220742Z-2454838` at `b374cd8`,
`20261006T222430Z-2574647` at `3764454` and `20261006T224358Z-2710606` at `0001946`, each
five of six. Their `05-sweep` read 197, 197 and 198 paired result lines, 5,317 / 1 / 64,
5,318 / 1 / 64 and 5,326 / 1 / 64. The one failure in each is #311's row with #311's
fragments (`Could not find module ‘Prelude’` in the store, the trace reaching `processi`),
so #311's list gains the three runs. No daemon survived. #316's row passed in all three. The
default plan has no luajit sweep, so R7's row did not run. The logs are copied to
`~/build/e8b/`.

So, at E8b's checkpoint: #311's list of runs reads at twenty-four; no other count moves.

### E8b's fix round 1: PR #325's runs at `70d69d0`, #323's second, #327's and #328's first, and #311's list at twenty-six

Recorded 2026-10-07 at E8b's fix round 1, with PR #325 at `70d69d0`: six signed commits on
`0001946`, after review round 1. The runs were read from the jobs endpoint by conclusion and
through all six test legs' logs; none was rerun.

#### `CI` 37648231070 at `70d69d0`, attempt 1 of 1: #328, #323's second and #327

`pull_request`, the merge `d253906` (`70d69d0` into `042e777`, read from the test logs'
checkout line), created 15:57:47Z and completed 16:43:41Z: `failure`.

Tally (pr325-ci-70d69d0): 19 = 16 + 1 + 1 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 112884485108 | success |
| Format | 112884485156 | success |
| Lint (lua54) | 112884485210 | success |
| Lint (luajit) | 112884485257 | success |
| Commit attribution (D9) | 112884485313 | success |
| GPU Render (headless) | 112884564211 | success |
| M1 Acceptance Gates | 112884564265 | success |
| Perf budgets (debug) | 112884564278 | success |
| M10 Perf Gates (crdt) | 112884564341 | success |
| Test (ubuntu-latest / luajit) | 112884564372 | failure |
| M6 Perf Gates | 112884564415 | success |
| M4 Perf Gates | 112884564431 | success |
| Test (crdt) | 112884564503 | success |
| Test (ubuntu-latest / luajit, no crdt) | 112884564524 | success |
| Test (ubuntu-latest / lua54) | 112884564530 | success |
| Test (macos-latest / lua54) | 112884564569 | cancelled |
| M5 Perf Gates | 112884564571 | success |
| Test (macos-latest / luajit) | 112884564598 | success |
| Docs consistency | 112884565446 | skipped |

Tally (pr325-ci-70d69d0-success): 16 rows of the table above with `result` = `success`.

- `Test (ubuntu-latest / luajit)`, `failure`, ran no test: its step 9 (`cargo test --all-targets
  …`) died compiling, `collect2: fatal error: ld terminated with signal 7 [Bus error], core
  dumped` and `could not compile `pmacs` (test "e7c_indicator_acceptance")`, its 825-line log
  holding no `running` line, after step 6's `cargo build --workspace --all-targets` had
  finished. Neither the log nor the job's annotations say the disk was short, which E7i's three
  Ubuntu reds at PR #309's first head both said. The same suites built and linked on the run's
  other three Ubuntu-hosted legs, which passed: Ubuntu lua54 and the no-crdt leg through the
  same two steps, `Test (crdt)` through `cargo build --workspace` and its `cargo test
  --all-targets`. No row here names
  it, so it is filed as **#328**, first occurrence, not attributed to the branch, not rerun.
- `Test (macos-latest / lua54)`, `cancelled`: `The job has exceeded the maximum execution time
  of 45m0s`, its step 9 cancelled after 38 minutes with 122 result lines.
  - `e7i_review1_probes::e7i_review1_parse_now_on_a_buffer_mid_parse`, with #323's three
    fragments (`the report did not reach the expected state in 120s; last:` with nothing after
    it, no `_parse_now mid-parse in none:` line in the log, and `test result: FAILED. 21
    passed; 1 failed; 0 ignored`): **#323's second occurrence**, its first on this leg.
    Commented on #323, not rerun.
  - `lsp_spawn_guidance_acceptance::j1b2_g_refreshes_the_lsp_panel_after_recovery` was still
    running when the job was cut. It printed its sixty-second notice at 16:22:58Z after the
    suite's fifteen other rows passed, and nothing more until the cancel at 16:43:33Z. The
    suite takes 1.5 to 6.4 s on every leg where it finished, this run's and `0001946`'s. This
    leg's step 9 took 15 to 18 minutes in all at `0001946`, `05dfa6a`, `ab06caa` and `24593f1`.
    No row here names it, so it is filed as **#327**, first occurrence, not rerun.

  The branch changes nothing under `src/lsp*`, `src/parse_isolation.rs`,
  `builtin/runtime/syntax.lua` or either suite; that is read, not a demonstration.
- The other four test legs pair their `running` lines with `test result: ok` and have no
  `FAILED`: 194 on macOS luajit, 193 on each of Ubuntu lua54 and the no-crdt leg, 192 on
  `Test (crdt)`, each three more than at `0001946`, this round's three new suites. This round's
  rows passed on every one of those legs that builds them. Its two GPU rows ran on `Test (crdt)`
  and macOS luajit (their suite 69.9 and 45.4 s) and returned early on Ubuntu lua54 (5.6 s),
  where E8's GPU suite does the same (0.76 s).

#### `Grammar fuzz` 37648231088 at `70d69d0`

Attempt 1 of 1, `success`: `Grammar fuzz` 112884476903 and `Grammar fuzz (asan-strict)`
112884477696, the path for a change that touches no grammar.

#### The round's gate, and review 1's

`scripts/gate`, the default plan, `20261007T154827Z-3509142` at `70d69d0`: five of six, its
`05-sweep` 201 paired result lines, 5,348 / 1 / 64. Review 1's, `20261007T101629Z-2858399` at
`0001946`: five of six, 198 paired, 5,326 / 1 / 64, read from the review's copy of its logs. The
one failure in each is #311's row with #311's fragments (`Could not find module ‘Prelude’`, the
trace reaching `processi`), so #311's list gains both. No daemon survived either. #316's row
passed in both, which is non-reproduction and nothing more. The default plan has no luajit
sweep, so R7's row did not run. The gate target holding both runs' logs was later deleted from
outside the session; this round's logs were copied to `~/build/e8b-fix1/gate/` before it, and
review 1's are at `~/build/e8b-review1/`.

So, at E8b's fix round 1: #311's list of runs reads at twenty-six; #323 moves to two; #327 and
#328 are at one; every other count above stands.

### The cache-budget PR: #328's second and its cold-cache rerun, #330 and #331 filed, PR #329's runs, #282's fourth, #307's second, and #311's list at twenty-seven

Recorded 2026-10-07 by the cache-budget pull request, #329 at `226f5f0` on `602c7ed`: one
signed commit, a workflow change the owner's brief granted to it alone (`rust-cache` saves
from `main` only). The runs were read from the jobs endpoint by conclusion and through the
logs named below.

#### `main` at `602c7ed`, E8b fix round 1's registry push

`CI` 37655163947, `push`, attempt 1 of 1, `success`, 15 = 6 success + 9 skipped (`Docs
consistency` 112908387495 among the six); `Grammar fuzz` 37655163928 `success`.

#### `CI` 37648231070 at `70d69d0`, attempt 2: #328's second

The owner's rerun of attempt 1's two unsuccessful jobs, 17:12:20Z to 17:36:52Z, on attempt
1's merge (`HEAD is now at d253906` in both rerun logs): `failure`.

Tally (pr325-ci-70d69d0-attempt2): 19 = 17 + 1 + 1.

- `Test (ubuntu-latest / luajit)` 112915370760, `failure`: **#328's second occurrence**,
  its fragments in three binaries (`e6d_review1_cursor_probes`, `m8_1c_acceptance`,
  `e7b_review_tab_acceptance`) where attempt 1 had one, its 860-line log holding no
  `running` line. Each failure asks for an LLVM bug report, so the linker `collect2` names
  `ld` is LLVM's. Both attempts restored `v0-rust-test-Linux-x64-5dc78561-10e4f63a`, full
  match, 678,605,842 bytes, and ran on `ubuntu-24.04` image `20260927.320.1`, whose
  toolchain step updated rustc 1.98.1 to 1.99.0; attempt 1's three other Ubuntu-hosted legs
  ran on `20261004.327.1`. That entry is `main`'s, saved once at 2026-10-04T18:41:49Z by
  run 37223480185's `luajit, no crdt` leg; 36 distinct Ubuntu legs in the 12 runs since
  restored it, 31 passing, this leg nine times (`0001946`'s among them). Commented on #328.
- `Test (macos-latest / lua54)` 112915370769, `success`: 194 `running` lines paired with
  194 `test result: ok`, no `FAILED`. #327's row passed, its suite's sixteen in 1.80 s, and
  #323's passed after its sixty-second notice, its suite's twenty-two in 114.89 s; both are
  non-reproduction and nothing more.
- The other seventeen keep attempt 1's conclusions under new ids: sixteen success and `Docs
  consistency` 112915374354 skipped.

#### `CI` 37648231070 at `70d69d0`, attempts 3 to 5: the brief's cold-cache rerun, and #330

The cache-budget brief's discriminator: the entry #328's leg restored deleted by id
(8488722453, `204` at 18:24:21Z), leaving only PR #309's copy, which PR #325's runs cannot
read, and the leg rerun on the same merge.

- Attempt 3, job 112947328819, image `20260927.320.1`, `No cache found.`: `cancelled` at the
  job's 45 minutes in its tool-install step, never reaching `cargo build`. Its `sudo apt-get
  update` saw every `azure.archive.ubuntu.com` index `Ign`, fell back to
  `archive.ubuntu.com`, and printed nothing after `Get:5 https://archive.ubuntu.com/ubuntu
  noble-security InRelease [126 kB]` (18:25:42Z) until the cancel (19:10:02Z). Filed as
  **#330**, first occurrence.
- Attempt 4, job 112968169387, image `20260927.320.1`, `No cache found.`: the same stall,
  its `Get:5` line at 19:13:15Z; the run cancelled by the session at 19:22:06Z, nine minutes
  into a step whose longest earlier run was 145 s. **#330's third occurrence** (its second
  is PR #329's, below).
- Attempt 5, job 112972995696: Attempt 5, job 112972995696, image `20260927.320.1` with the same rustc update,
  `No cache found.`: `failure`. The leg's own build linked everything: `cargo build
  --workspace --all-targets` passed in 4 m 34 s, and step 9 ran the same 191 test binaries as
  attempt 1's Ubuntu lua54 leg (the two lists of `Running` lines identical), 190 `test result:
  ok`. The one failure is
  `e7i_review1_probes::e7i_review1_the_aliasing_guard_reaches_the_worker_s_build`, whose own
  `cargo build` into a target under `/tmp` died with **#328's linker fragment beside `No space
  left on device`**: `collect2: fatal error: ld terminated with signal 7 [Bus error], core
  dumped` twice (the `nix` and `libc` build scripts) and gcc's `error writing to /tmp/cc….s:
  No space left on device`. It does not carry #328's `could not compile `pmacs` (test …)`, so
  it is filed as **#331**, first occurrence, not rerun; on #328 it is commented as the first
  log in which the linker's SIGBUS on these runners stands beside ENOSPC. That a cold leg
  linked where two warm ones did not is evidence about the carrier and not a diagnosis.

Tally (pr325-ci-70d69d0-attempt3): 19 = 17 + 1 + 1.

Tally (pr325-ci-70d69d0-attempt4): 19 = 17 + 1 + 1.

Tally (pr325-ci-70d69d0-attempt5): 19 = 17 + 1 + 1.

Each attempt's other eighteen jobs keep their earlier conclusions under new ids: seventeen
success and `Docs consistency` skipped (112947329548, 112968170595, 112972997003).

#### `CI` 37667841100 at `226f5f0`, attempt 1 of 1: #282's fourth, #307's second, #330's second

`pull_request`, the merge `fbdfb73` (`226f5f0` into `602c7ed`, read from the test logs'
checkout line), created 18:34:14Z and completed 19:20:11Z: `failure`.

Tally (pr329-ci-226f5f0): 19 = 15 + 2 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 112951553833 | success |
| Changed paths | 112951554196 | success |
| Lint (lua54) | 112951554325 | success |
| Lint (luajit) | 112951554588 | success |
| Format | 112951554601 | success |
| M4 Perf Gates | 112951656453 | success |
| M1 Acceptance Gates | 112951656522 | success |
| M5 Perf Gates | 112951656699 | success |
| Test (ubuntu-latest / luajit) | 112951656708 | cancelled |
| Test (macos-latest / luajit) | 112951656709 | failure |
| Test (crdt) | 112951656711 | success |
| M10 Perf Gates (crdt) | 112951656729 | success |
| Test (ubuntu-latest / luajit, no crdt) | 112951656742 | failure |
| Test (ubuntu-latest / lua54) | 112951656769 | success |
| GPU Render (headless) | 112951656792 | success |
| M6 Perf Gates | 112951656793 | success |
| Test (macos-latest / lua54) | 112951656842 | success |
| Perf budgets (debug) | 112951656854 | success |
| Docs consistency | 112951658686 | skipped |

Tally (pr329-ci-226f5f0-success): 15 rows of the table above with `result` = `success`.

- `Test (macos-latest / luajit)`, `failure`:
  `e7b_review_wire_acceptance::a_save_sends_did_save_and_rust_analyzer_flychecks_on_it` with
  all three of #307's fragments (`the check showed as a suffix on ready, or as the tracker's
  busy title under a reload`; the wire after the save `["11 = LSP:ready"]` with the busy
  field `["11 b -"]`; `test result: FAILED. 4 passed; 1 failed; 2 ignored`): **#307's second
  occurrence**, its only failure against 187 `test result: ok`. Commented, not rerun.
- `Test (ubuntu-latest / luajit, no crdt)`, `failure`:
  `e7b_review_wire_acceptance::a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`
  with all three of #282's fragments, the result line's count moved with the suite as at its
  second and third (`*lsp* names the request and the code: []`; the `-32602` line in
  `*errors*` and `WIRE label "ready", last_error None`; `test result: FAILED. 4 passed; 1
  failed; 2 ignored`), the row's trace holding 103 `$/progress` frames, 87 of them
  `cachePriming`: **#282's fourth occurrence**, its only failure against 186 `test result:
  ok`. Commented, not rerun.
- `Test (ubuntu-latest / luajit)`, `cancelled` at 45 minutes in its tool-install step, on
  image `20261004.327.1`: #330's fragments, its `Get:5` line at 18:35:03Z. **#330's second
  occurrence**. The run's three other Ubuntu-hosted legs reached the Azure mirror (`Hit:2
  http://azure.archive.ubuntu.com/ubuntu noble InRelease`) and finished their own `apt-get`
  steps in 50 s to 1 m 48 s (`Test (crdt)`'s is its lavapipe install).
- Ubuntu lua54 and the no-crdt leg logged `No cache found.` (`main`'s shared Linux entry
  deleted above) and built and linked every binary cold on image `20260927.320.1`; neither
  saved, and the listing at 19:11:59Z held the same 15 entries as at 18:29:39Z. The macOS
  legs and `Test (crdt)` restored `main`'s entries in full. The rows of #282 and #307 passed
  on every other leg that reached them.
- Neither red is the branch's: it changes only `save-if` in two workflows, and nothing
  reaches LSP or these suites.

#### `Grammar fuzz` 37667841057 at `226f5f0`

Attempt 1 of 1, `success`: `Grammar fuzz` 112951554177 and `Grammar fuzz (asan-strict)`
112951554699, each fuzzing (`reason=.github/workflows/grammar-fuzz.yml changed`) after
restoring `main`'s fuzz entry in full.

#### The pull request's gate

`scripts/gate`, the default plan, `20261007T183201Z-3888421` at `226f5f0`: five of six, its
`05-sweep` 197 paired result lines, 5,313 / 1 / 64, the counts of E8 fix round 4's default
sweep on the same code. The one failure is #311's row with #311's fragments (`Could not
find module ‘Prelude’`, the trace reaching `processi`), so #311's list gains it. No daemon
survived. #316's row passed, which is non-reproduction and nothing more. The default plan
has no luajit sweep, so R7's row did not run. The logs are copied to
`~/build/cache-budget/gate/`.

So, at the cache-budget PR: #311's list of runs reads at twenty-seven; #328 moves to two;
#282 moves to four and #307 to two; #330 is at three and #331 at one; #323 and #327 stand at
two and one, their rows passing in attempt 2; #316's row passed in the gate and R7's did not
run; every other count above stands.

### The cache-budget PR's addendum: the disk printed, PR #329's runs at `1dc7658` and `d453609`, and #311's list at twenty-eight

Recorded 2026-10-08 by the cache-budget pull request's addendum, #329 at `d453609` (five
signed commits on `602c7ed`): the Ubuntu test legs print the disk, `apt-get update` is
bounded (#330), and #331's row is cheaper and names a full disk. The runs were read from the
jobs endpoint by conclusion and through every Ubuntu test leg's log and the red legs'.

#### `main` at `a69744f`, the cache-budget PR's registry push

`CI` 37679277562, `push`, attempt 1 of 1, `success`, 15 = 6 success + 9 skipped (`Docs
consistency` 112990863021 among the six); `Grammar fuzz` 37679277448 `success`.

#### `CI` 37684404247 at `1dc7658`, attempt 1 of 1: the branch's own red on both macOS legs, and the disk

`pull_request`, the merge `2e6e4aa` (`1dc7658` into `a69744f`, read from the test logs'
checkout line), created 20:45:08Z and completed 21:24:05Z: `failure`.

Tally (pr329-ci-1dc7658): 19 = 16 + 2 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 113008381739 | success |
| Lint (luajit) | 113008382124 | success |
| Lint (lua54) | 113008382137 | success |
| Commit attribution (D9) | 113008382160 | success |
| Format | 113008382812 | success |
| M6 Perf Gates | 113008489408 | success |
| Perf budgets (debug) | 113008489518 | success |
| M1 Acceptance Gates | 113008489549 | success |
| M10 Perf Gates (crdt) | 113008489593 | success |
| M5 Perf Gates | 113008489596 | success |
| Test (crdt) | 113008489598 | success |
| M4 Perf Gates | 113008489604 | success |
| GPU Render (headless) | 113008489630 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113008489734 | success |
| Test (ubuntu-latest / lua54) | 113008489754 | success |
| Test (ubuntu-latest / luajit) | 113008489835 | success |
| Test (macos-latest / lua54) | 113008489854 | failure |
| Test (macos-latest / luajit) | 113008489953 | failure |
| Docs consistency | 113008491056 | skipped |

Tally (pr329-ci-1dc7658-success): 16 rows of the table above with `result` = `success`.

- Both macOS legs, `failure`, each on the two script rows this head added to
  `tests/gate_script_acceptance.rs` and nothing else (187 `test result: ok` each). That is
  the branch's own: `ci_disk_prints_each_filesystem_and_directory_and_never_fails_a_leg`
  found no line for `/` (BSD `df` takes no `-B1M`, and BSD `du` read it as a block size,
  6144 MB for 3 MB), and `ci_apt_update_bounds_a_stalled_update_and_retries_it` found no
  `timeout` (`exec: timeout: not found`). Fixed at `d453609`: `scripts/ci-disk` reads
  POSIX `df -Pk` and `du -sk`, and the apt row is ignored off Linux with its reason.
- The four Ubuntu test legs passed (189, 189, 189 and 188 `test result: ok`, no `FAILED`),
  #331's row among them. The three `test` legs logged `No cache found.`: `main`'s shared
  Linux entry was deleted for #328's experiment, and under this pull request no pull
  request saves one. Each Ubuntu leg printed its disk on a 147,719 MB root filesystem:

| leg | free before the build | free after the build | free after the tests | target after the tests |
|---|---|---|---|---|
| luajit | 86,581 MB | 42,469 MB | 1,918 MB | 84,067 MB |
| lua54 | 86,580 MB | 42,940 MB | 2,872 MB | 83,112 MB |
| luajit, no crdt | 86,581 MB | 57,443 MB | 31,795 MB | 54,176 MB |
| `Test (crdt)` | 84,421 MB | 82,770 MB | 42,568 MB | 43,869 MB |

  The two crdt `test` legs' test step recompiled 35 crates, `pmacs` among them, and added
  about 40 GB of target after the workspace build's 43.6 GB; `Test (crdt)`, which builds
  `cargo build --workspace` without `--all-targets` (its restored target 2,522 MB), ended at
  43.9 GB. So on `main`'s tree, cold, the luajit leg, the one #328 and #331 were seen on,
  ends its tests with 1,918 MB free, 954 MB less than lua54. Whether #328 and #331 are one
  defect is not settled by these figures; #328's comment says what is and is not read, and
  both stay open.
- #330's precondition without its stall: on the luajit leg and `Test (crdt)` every Azure
  index was `Ign` (26 and 12 lines) and the fallback finished (`Fetched` in 12 s and 1 s);
  no attempt was cut. Not an occurrence.

#### `Grammar fuzz` 37684404318 at `1dc7658`

Attempt 1 of 1, `success`: `Grammar fuzz` 113008385439 and `Grammar fuzz (asan-strict)`
113008385728.

#### `CI` 37769314441 at `d453609`, attempt 1 of 1: green

`pull_request`, the merge `43404ed` (`d453609` into `a69744f`, read from the test logs'
checkout line), created 11:19:46Z and completed 11:54:55Z: `success`.

Tally (pr329-ci-d453609): 19 = 18 + 1.

| job | id | result |
|---|---|---|
| Changed paths | 113284640277 | success |
| Format | 113284640426 | success |
| Commit attribution (D9) | 113284640805 | success |
| Lint (luajit) | 113284640887 | success |
| Lint (lua54) | 113284640923 | success |
| M4 Perf Gates | 113284714219 | success |
| Test (crdt) | 113284714229 | success |
| GPU Render (headless) | 113284714295 | success |
| M10 Perf Gates (crdt) | 113284714340 | success |
| M5 Perf Gates | 113284714419 | success |
| Test (ubuntu-latest / lua54) | 113284714428 | success |
| Test (ubuntu-latest / luajit) | 113284714429 | success |
| Test (macos-latest / lua54) | 113284714460 | success |
| M6 Perf Gates | 113284714501 | success |
| Test (macos-latest / luajit) | 113284714506 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113284714533 | success |
| Perf budgets (debug) | 113284714551 | success |
| M1 Acceptance Gates | 113284714581 | success |
| Docs consistency | 113284715750 | skipped |

Tally (pr329-ci-d453609-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs its `running` lines with `test result: ok` and has no `FAILED`: 190
  on each macOS leg, 189 on each Ubuntu `test` leg, 188 on `Test (crdt)`. On both macOS legs
  `ci_disk_prints_each_filesystem_and_directory_and_never_fails_a_leg` passed and
  `ci_apt_update_bounds_a_stalled_update_and_retries_it` reads `ignored, scripts/ci-apt-update
  runs on the Ubuntu legs, which have apt and coreutils' timeout`. #331's row passed on all
  six legs.
- The disk again, within two MB of `1dc7658`'s on every Ubuntu leg: free after the tests
  1,917 MB on luajit, 2,871 MB on lua54, 31,795 MB on the no-crdt leg and 42,516 MB on
  `Test (crdt)`, the targets 84,066, 83,112, 54,176 and 43,869 MB. No apt attempt was cut.

#### `Grammar fuzz` 37769314473 at `d453609`

Attempt 1 of 1, `success`: `Grammar fuzz` 113284642178 and `Grammar fuzz (asan-strict)`
113284642670.

#### The addendum's gate

`scripts/gate`, the default plan, `20261007T204535Z-472312` at `1dc7658`: five of six, its
`05-sweep` 197 paired result lines, 5,318 / 1 / 64, the first gate's 5,313 and the five rows
this head adds. The one failure is #311's row with #311's fragments, so #311's list gains it.
No daemon survived. #316's row passed, which is non-reproduction and nothing more; R7's is
not in the default plan. `d453609` changes a CI script, a workflow comment and one row's
attribute, and was not gated again; its four rows passed by hand in the gate's environment.
The logs are copied to `~/build/cache-budget/gate/`.

So, at the cache-budget PR's addendum: #311's list of runs reads at twenty-eight; #330's
precondition recurred twice without a stall; every other count above stands.

### The cache-budget PR's second addendum: the workspace build without `--all-targets`, PR #329's run at `1218ba1`, and #311's list at twenty-nine

Recorded 2026-10-08 by the cache-budget pull request's second addendum, #329 at `1218ba1`
(seven signed commits on `602c7ed`): the matrix test legs build `cargo build --workspace`
without `--all-targets`, and the no-crdt leg alone links the test targets of
`pmacs-protocol`, `pmacs-syntax` and `pmacs-parse-unit`. The run was read from the jobs
endpoint by conclusion and through all six test legs' logs.

#### `main` at `ae9761d`, the addendum's registry push

`CI` 37775485751, `push`, attempt 1 of 1, `success`, 15 = 6 success + 9 skipped (`Docs
consistency` 113305215737 among the six); `Grammar fuzz` 37775485681 `success`.

#### `CI` 37783868963 at `1218ba1`, attempt 1 of 1: green, and the disk

`pull_request`, the merge `cd80a50` (`1218ba1` into `ae9761d`, read from the test logs'
checkout line), created 13:22:45Z and completed 13:54:29Z: `success`.

Tally (pr329-ci-1218ba1): 19 = 18 + 1.

| job | id | result |
|---|---|---|
| Lint (lua54) | 113333509781 | success |
| Format | 113333509951 | success |
| Commit attribution (D9) | 113333509958 | success |
| Lint (luajit) | 113333509997 | success |
| Changed paths | 113333510099 | success |
| Test (crdt) | 113333594763 | success |
| Perf budgets (debug) | 113333594889 | success |
| M1 Acceptance Gates | 113333594891 | success |
| M4 Perf Gates | 113333594897 | success |
| GPU Render (headless) | 113333594907 | success |
| M6 Perf Gates | 113333594910 | success |
| M10 Perf Gates (crdt) | 113333594967 | success |
| Test (ubuntu-latest / luajit) | 113333594988 | success |
| Test (ubuntu-latest / lua54) | 113333594994 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113333595000 | success |
| M5 Perf Gates | 113333595020 | success |
| Test (macos-latest / luajit) | 113333595074 | success |
| Test (macos-latest / lua54) | 113333595089 | success |
| Docs consistency | 113333597453 | skipped |

Tally (pr329-ci-1218ba1-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs its `running` lines with `test result: ok` and has no `FAILED`: 190
  on each macOS leg, 189 on each Ubuntu `test` leg, 188 on `Test (crdt)`, the counts at
  `d453609`. The three `test` legs logged `No cache found.`; `Test (crdt)` restored its own
  key.
- The disk on the Ubuntu legs, against the same legs at `1dc7658` (also cold), every one on
  a 147,718 MB root filesystem:

| leg | free after the tests, `1dc7658` | free after the tests, `1218ba1` | target after the tests, `1dc7658` | target after the tests, `1218ba1` |
|---|---|---|---|---|
| luajit | 1,918 MB | 42,095 MB | 84,067 MB | 43,869 MB |
| lua54 | 2,872 MB | 42,621 MB | 83,112 MB | 43,346 MB |
| luajit, no crdt | 31,795 MB | 57,040 MB | 54,176 MB | 28,927 MB |
| `Test (crdt)` | 42,568 MB | 42,568 MB | 43,869 MB | 43,869 MB |

  The luajit crdt leg now ends at `Test (crdt)`'s target size, 43,869 MB, to the megabyte.
  Its workspace build left a 3,342 MB target where it had left 43,588 MB.
- The no-crdt leg's new step (`cargo build -p pmacs-protocol -p pmacs-syntax -p
  pmacs-parse-unit --all-targets`) took 11 s and 267 MB: the target went from 3,139 to
  3,406 MB, the root filesystem's use from 64,778 to 65,045 MB.
- Time, against `CI` 37769314441 at `d453609`: the workspace build step took 131, 91 and
  129 s on the luajit, lua54 and no-crdt legs, where it had taken 282, 269 and 259 s; the
  test step took 1,667, 1,661 and 1,476 s, against 1,648, 1,656 and 1,527 s; the jobs ended
  130 to 182 s sooner.
- No apt attempt was cut on any leg.
- The figures are all from cold `test` legs. `main` holds no Linux test entry, so a warm
  luajit crdt leg's margin under this build is not measured; it arrives with the second code
  run after this pull request merges, the first saving the entry.

#### `Grammar fuzz` 37783868837 at `1218ba1`

Attempt 1 of 1, `success`: `Grammar fuzz` 113333509292 and `Grammar fuzz (asan-strict)`
113333509514.

#### The second addendum's gate

`scripts/gate`, the default plan, `20261008T132308Z-224598` at `1218ba1`: five of six, its
`05-sweep` 197 paired result lines, 5,320 / 1 / 64, the last gate's 5,318 and the two rows
this round adds. The one failure is #311's row with #311's fragments, so #311's list gains
it. No daemon survived. #316's row passed, which is non-reproduction and nothing more; R7's
is not in the default plan. The logs are copied to `~/build/cache-budget/gate/`.

So, at the cache-budget PR's second addendum: #311's list of runs reads at twenty-nine; #328,
#330 and #331 stay open and are not sampled; every other count above stands.

### E8b's close: PR #325's last runs, `main` at `a27cdc2` and `4fade33`, #332, #333 and #334 filed, and the disk on E8b's tree, cold and warm

Recorded 2026-10-08 after PR #329 merged as `a27cdc2` (14:38:57Z) and PR #325 as `4fade33`
(17:17:35Z, the squash of `ca8b2f6`, whose tree it is: `git diff ca8b2f6 4fade33` is empty).
The runs were read from the jobs endpoint by conclusion and through every test leg's log;
none was rerun. No gate ran: this records CI and changes no tree, so #311's list stands at
twenty-nine.

#### `main` at `8bacad0`, the second addendum's registry push

`CI` 37791288514, `push`, attempt 1 of 1, `success`, 15 = 6 success + 9 skipped (`Docs
consistency` 113359193392 among the six); `Grammar fuzz` 37791288523 `success`.

#### `CI` 37648231070 at `70d69d0`, attempt 6: #333, a first sample

The sections above end at attempt 5, and attempt 6 was recorded nowhere until now. It re-ran
`Test (ubuntu-latest / luajit)` alone, job 112999008021, 2026-10-07 20:23:52Z to 20:59:39Z,
on attempt 1's merge (`HEAD is now at d253906`): `failure`. By their start times the
attempt's other eighteen jobs are copies carried from earlier attempts: seventeen success
and `Docs consistency` 112999047715 skipped.

Tally (pr325-ci-70d69d0-attempt6): 19 = 17 + 1 + 1.

- Image `ubuntu-24.04` `20260927.320.1`, its toolchain step updating rustc 1.98.1 to 1.99.0,
  and `No cache found.`. The leg's own build and link passed: step 9 ran 191 test binaries
  for 191 result lines, 190 `test result: ok` and one `FAILED`.
- The failure is `e7i_review1_probes::e7i_review1_the_aliasing_guard_reaches_the_worker_s_build`,
  `test result: FAILED. 21 passed; 1 failed; 0 ignored` (`finished in 124.14s`). Its nested
  `cargo build` died on `collect2: fatal error: ld terminated with signal 7 [Bus error], core
  dumped`, twice, under `could not compile `nix` (build script)` and `could not compile
  `tree-sitter-go` (build script)`, each beside the LLVM linker's bug-report request.
  **`No space left on device`, `error writing to` and `error reading` appear zero times.**
- So it is neither #331, whose ENOSPC fragment it lacks (and `tree-sitter-go` stands where
  #331 had `libc`), nor #328, which died in the leg's own compile with no test run. Filed as
  **#333**, first sample, between the two; #331 is commented. Nothing printed `df` then
  (`scripts/ci-disk` reached the legs with `a27cdc2`), so like #328's two it has no disk
  figure.
- Read from the code, not measured: the aliasing-guard row on `main` names #331 and prints
  `df` only when the nested check's stderr holds `No space left on device`
  (`aliasing_guard_check_verdict`), so #333's shape would still fail it as the guard's
  absence, with no figure.

The run's conclusion is `failure` at attempt 6 of 6.

#### `main` at `a27cdc2`, PR #329's squash: #282's fifth and #316's fourth

`CI` 37794192379, `push`, attempt 1 of 1, created 14:39:00Z and completed 15:12:33Z:
`failure`. The first code-bearing run on `main` after #329, and the base of #325's last run,
which tested its merge into this commit.

Tally (main-a27cdc2-ci): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 113369228634 | success |
| Format | 113369229078 | success |
| Lint (lua54) | 113369229097 | success |
| Lint (luajit) | 113369229154 | success |
| Changed paths | 113369229307 | success |
| Test (crdt) | 113369313348 | failure |
| M4 Perf Gates | 113369313375 | success |
| M1 Acceptance Gates | 113369313383 | success |
| Perf budgets (debug) | 113369313386 | success |
| Test (ubuntu-latest / luajit) | 113369313462 | success |
| GPU Render (headless) | 113369313464 | success |
| M10 Perf Gates (crdt) | 113369313526 | success |
| Test (ubuntu-latest / lua54) | 113369313528 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113369313619 | success |
| M6 Perf Gates | 113369313693 | success |
| M5 Perf Gates | 113369313701 | success |
| Test (macos-latest / lua54) | 113369313724 | success |
| Test (macos-latest / luajit) | 113369313784 | success |
| Docs consistency | 113369316807 | skipped |

Tally (main-a27cdc2-ci-success): 17 rows of the table above with `result` = `success`.

- `Test (crdt)` 113369313348, `failure`, on two rows. Each carries its issue's every required
  fragment, read against the issue's text and not matched by selector alone:
  - `e7b_review_wire_acceptance::a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`,
    at `tests/e7b_review_wire_acceptance.rs:1218` (not a fragment): `*lsp* names the request
    and the code: []`; `WIRE *errors* gained ["[lsp] LSP: default-rust refused
    textDocument/prepareRename as a client error, -32602 InvalidParams: No references found
    at position"]` and `WIRE label "ready", last_error None`; and `test result: FAILED. 4
    passed; 1 failed; 2 ignored` (`running 7 tests`), the count moved with the suite as at
    the second to fourth. Between the row's stdout header and its panic the trace holds 121
    `$/progress` frames, 101 of them `rustAnalyzer/cachePriming` reports, against `*lsp*`'s
    64-entry ring. **#282's fifth occurrence**, its second on `main`.
  - `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`,
    at `tests/e7i_review3_probes.rs:853` (not a fragment): `kill 1: nothing held the switch's
    follow-up parse, which started a worker at once and installed`, its report `busy=false …
    tree=true unit=pid_26141 death=killed, " +  /tmp/…/a.rs  (rust) … parse:killed !1  L1:C1
    Top"`, and `test result: FAILED. 10 passed; 1 failed; 0 ignored` (`running 11 tests`,
    `finished in 17.31s`). **#316's fourth sample**, its second on `main`. It matches the
    fragments as written, where the third matched only as the template, and its `busy=false`
    is the two local samples' shape, not the third's.
  - Both issues are commented. The job pairs 187 `running` lines with 187 result lines, 185
    `ok`; its `cargo test --doc` step was skipped after the sweep failed. Both rows ran `ok`
    on the run's other five legs; on macOS the rename row returns without its server.
- `Test (crdt)` restored its own key in full (`v0-rust-crdt-test-Linux-x64-5dc78561-10e4f63a`,
  `full match: true`) and read **42,566 MB available after the tests, its target 43,869
  MB**. The disk is not in these two reds.
- The other five test legs pair every `running` line with `test result: ok` and carry no
  `FAILED`: 189 on each Ubuntu `test` leg and 190 on each macOS leg, whose logs hold one more
  `running 1 test`, in the adapter probe's script text. `WouldBlock` appears on no leg.
- The three Ubuntu `test` legs logged `No cache found.` and built cold. The no-crdt leg
  finished first and saved `v0-rust-test-Linux-x64-5dc78561-10e4f63a` (690,199,046 bytes
  sent at 15:05:51Z); the luajit and lua54 legs logged `Unable to reserve cache with key
  v0-rust-test-Linux-x64-5dc78561-10e4f63a, another job may be creating this cache`.
- The disk on the Ubuntu legs, every one on a 147,718 MB root filesystem, available MB:

| leg | before the build | after the build | target after the build | after the tests | target after the tests |
|---|---|---|---|---|---|
| luajit | 86,579 | 82,720 | 3,342 | 42,099 | 43,870 |
| lua54 | 86,575 | 82,758 | 3,300 | 42,616 | 43,346 |
| luajit, no crdt | 86,579 | 82,923 | 3,139 | 57,038 | 28,927 |
| `Test (crdt)` | 84,419 | 82,768 | 3,760 | 42,566 | 43,869 |

  The no-crdt leg's small crates' test targets left 82,656 MB available and a 3,406 MB
  target. `Test (crdt)`'s target was 2,522 MB before its build, restored.

`Grammar fuzz` 37794192417, attempt 1 of 1, `success`: `Grammar fuzz` 113369229545 and
`Grammar fuzz (asan-strict)` 113369229199.

#### `CI` 37794436707 at `ca8b2f6`, attempt 1 of 1: #332, a first sample, and the disk on E8b's tree

`ca8b2f6` merges `main` at `a27cdc2` into the branch at `70d69d0` (its two parents), so that
#325's run would test a `main` carrying #329. The run is a `pull_request` on the merge
`8681d5f` (`ca8b2f6` into `a27cdc2`, read from the test logs' checkout line), created
14:40:48Z and completed 15:14:42Z: `failure`. It is #325's first run after #329 and its last.

Tally (pr325-ci-ca8b2f6): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 113370078000 | success |
| Lint (luajit) | 113370078630 | success |
| Format | 113370078782 | success |
| Lint (lua54) | 113370078809 | success |
| Changed paths | 113370078959 | success |
| M4 Perf Gates | 113370216322 | success |
| Test (ubuntu-latest / lua54) | 113370216401 | success |
| Test (crdt) | 113370216424 | success |
| GPU Render (headless) | 113370216435 | success |
| M6 Perf Gates | 113370216489 | success |
| Test (macos-latest / lua54) | 113370216491 | success |
| Test (macos-latest / luajit) | 113370216504 | success |
| M1 Acceptance Gates | 113370216520 | success |
| Perf budgets (debug) | 113370216583 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113370216636 | success |
| Test (ubuntu-latest / luajit) | 113370216644 | failure |
| M10 Perf Gates (crdt) | 113370216653 | success |
| M5 Perf Gates | 113370216814 | success |
| Docs consistency | 113370219103 | skipped |

Tally (pr325-ci-ca8b2f6-success): 17 rows of the table above with `result` = `success`.

- `Test (ubuntu-latest / luajit)` 113370216644, `failure`, on one row:
  `m10_11_acceptance`'s copy of `common::ready::tests::readiness_is_a_served_hello_not_a_connect`
  at `tests/common/ready.rs:423:9`, `Err(Timeout { what: "a daemon serving on
  /tmp/.tmpJDRwKe/late.sock", deadline: 10s, elapsed: 10.000113846s, polls: 473, last:
  "connect: Connection refused (os error 111)" })`, `running 13 tests` at 15:10:09Z and
  `test result: FAILED. 9 passed; 1 failed; 3 ignored` at 15:10:21Z. The job's only failure
  against 190 `test result: ok`; its `cargo test --doc` and `pmacs-protocol` steps were
  skipped after it. `WouldBlock`, `did not become ready` and `B must receive CrdtOp` appear
  zero times. The same test ran `ok` in 57 other suites' copies in the job, among them
  `gpu_invocation_acceptance`'s and `lsp_dispatch_seams_acceptance`'s (#271's) and
  `vterm_stage2_acceptance`'s (#276's), and #271's two other targets ran `ok` too.
- **Neither #271 nor #276, filed as #332.** The fragments are #271's first group and the
  whole of #276's: `a daemon serving on <tmp>/late.sock` + `deadline: 10s` + `connect:
  Connection refused`. #271 names this job, but requires its other two groups too (the
  `WouldBlock` snapshot read, the missed broadcast at `m5_5_acceptance.rs:1169`) in four
  exact targets, and `m10_11_acceptance` is not among them. #276 requires this group alone,
  but its selector is the `vterm_stage2_acceptance` copy and its job `Test (crdt)`. A red
  matches a row only on the selector, the job and every fragment, and a row widens only on a
  demonstrated shared object, none of which is shown here. That is the reading that filed
  #276 apart from #271 at E6d's fix round 1, applied in both directions. It is the lone
  shape's second CI sample and its first on #271's leg; whether #271, #276 and #332 are one
  mechanism is the owner's ruling. #271 and #276 are commented, and both stay at one.
- The other five test legs pair every `running` line with `test result: ok` and carry no
  `FAILED`: 193 on Ubuntu lua54 and the no-crdt leg, 192 on `Test (crdt)` and 194 on each
  macOS leg (their logs carry the adapter probe's extra `running 1 test`), each four more
  than #329's at `1218ba1`: the lists of `Running` lines differ from `a27cdc2`'s lua54 leg by
  E8b's four new suites and nothing else. #282's and #316's rows ran `ok` on every leg, which
  is non-reproduction and nothing more.
- The disk, cold on the three `test` legs, which logged `No cache found.` at 14:41Z; `main`'s
  shared test entry was saved by `a27cdc2`'s run at 15:05:51Z (above). Available MB:

| leg | before the build | after the build | target after the build | after the tests | target after the tests |
|---|---|---|---|---|---|
| luajit | 86,579 | 82,717 | 3,345 | **41,225** | 44,741 |
| lua54 | 86,579 | 82,760 | 3,302 | 41,768 | 44,198 |
| luajit, no crdt | 86,579 | 82,921 | 3,141 | 56,643 | 29,323 |
| `Test (crdt)` | 84,420 | 82,766 | 3,763 | 41,696 | 44,742 |

  The luajit crdt leg, the one #328 and #331 name, ended E8b's tree with 41,225 MB available,
  where it ended `main`'s tree with 1,918 MB before #329 (`1dc7658`) and 42,099 MB at
  `a27cdc2` after it, both cold. Its linker succeeded and its tests ran: `signal 7` and `No
  space left on device` appear zero times, so this run reached neither #328's fragments nor
  #331's. **These figures do not retire #328.** Its own two failures have no disk figure,
  because nothing printed one then; they were warm, and this leg was cold. The warm figure
  on this tree is `4fade33`'s, below.

`Grammar fuzz` 37794436694, attempt 1 of 1, `success`: `Grammar fuzz` 113370084313 and
`Grammar fuzz (asan-strict)` 113370084260.

#### `main` at `4fade33`, E8b's squash: #323's third and #334, a first sample, and the warm disk

`CI` 37815431889, `push`, attempt 1 of 1, created 17:17:39Z and completed 17:49:46Z:
`failure`. `4fade33` is `main`'s last code-bearing commit as recorded here.

Tally (main-4fade33-ci): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Lint (luajit) | 113442731130 | success |
| Lint (lua54) | 113442731421 | success |
| Commit attribution (D9) | 113442731454 | success |
| Changed paths | 113442731519 | success |
| Format | 113442731670 | success |
| M1 Acceptance Gates | 113442840324 | success |
| M6 Perf Gates | 113442840341 | success |
| GPU Render (headless) | 113442840369 | success |
| Test (crdt) | 113442840380 | success |
| M5 Perf Gates | 113442840429 | success |
| Perf budgets (debug) | 113442840443 | success |
| Test (macos-latest / lua54) | 113442840458 | failure |
| M10 Perf Gates (crdt) | 113442840483 | success |
| Test (ubuntu-latest / lua54) | 113442840550 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 113442840561 | success |
| Test (macos-latest / luajit) | 113442840572 | success |
| M4 Perf Gates | 113442840626 | success |
| Test (ubuntu-latest / luajit) | 113442840680 | success |
| Docs consistency | 113442842302 | skipped |

Tally (main-4fade33-ci-failures): 2 rows of the table above with `result` = `failure`.

- `Test (macos-latest / lua54)` 113442840458, `failure`:
  `e7i_review1_probes::e7i_review1_parse_now_on_a_buffer_mid_parse`, **#323's third
  occurrence**, its first on `main`. All three fragments are present: `the report did not
  reach the expected state in 120s; last:` with nothing after `last:`; no `_parse_now
  mid-parse in none:` line in the job; and `test result: FAILED. 22 passed; 1 failed; 0
  ignored` (`running 23 tests`, `finished in 131.48s`), the count moved with the suite, which
  gained a row with #329. It was the job's only failure, against 191 `test result: ok`.
  Where the row finished on macOS, mode `none`'s `blocked=` read 102,291 and 99,917 ms at
  `a27cdc2` (lua54, luajit), 99,461 and 103,504 at `ca8b2f6`, and 55,122 on this run's
  luajit leg. Commented on #323.
- `Test (ubuntu-latest / lua54)` 113442840550, `failure`:
  `e7i_parse_containment_acceptance::e7i_a_fresh_worker_s_query_compile_is_not_under_the_deadline`,
  `the report did not reach the expected state in 60s; last:` with nothing after `last:`,
  and `test result: FAILED. 26 passed; 1 failed; 0 ignored` (`running 27 tests`). It was the
  job's only failure, against 190 `test result: ok`. No row or issue names this selector, and
  it is not #323, which names another row, bound and leg. Filed as **#334**, first sample. The
  row ran 60.7 s here; on the other seventeen test legs of the three code runs above it
  passed, and on the eleven Linux ones it returned 0.48 to 0.95 s after the row before it.
- Each red leg's `cargo test --doc` and `pmacs-protocol` steps were skipped after it. The
  other four test legs pair every `running` line with `test result: ok`: 193 on the Ubuntu
  luajit and no-crdt legs, 192 on `Test (crdt)` and 194 on macOS luajit. #282's and #316's
  rows passed on every leg, `Test (crdt)` among them, which is non-reproduction and nothing
  more. The readiness self-test passed in every copy on every leg, `m10_11_acceptance`'s
  (#332's) among them: 58 per crdt leg, 39 on the no-crdt leg. `WouldBlock` appears on no
  leg.
- **The warm figure #329 left unmeasured.** The three Ubuntu `test` legs restored
  `v0-rust-test-Linux-x64-5dc78561-10e4f63a` in full, 690,199,046 bytes: the entry
  `a27cdc2`'s no-crdt leg saved. `Test (crdt)` restored its own key. They ran on image
  `20261004.327.1` with rustc 1.99.0 unchanged. Available MB:

| leg | before the build | target before the build | after the build | target after the build | after the tests | target after the tests |
|---|---|---|---|---|---|---|
| luajit | 83,812 | 2,539 | 82,157 | 3,781 | **40,767** | 45,075 |
| lua54 | 83,816 | 2,539 | 82,139 | 3,804 | 41,252 | 44,598 |
| luajit, no crdt | 83,816 | 2,539 | 82,366 | 3,576 | 56,524 | 29,324 |
| `Test (crdt)` | 84,368 | 2,522 | 82,714 | 3,763 | 41,644 | 44,742 |

  The tree is `ca8b2f6`'s, so the luajit crdt leg's warm figure stands beside its cold one on
  the same tree: 40,767 MB against 41,225, a target 334 MB larger. The no-crdt leg's small
  crates' test targets left 82,232 MB and a 3,711 MB target. The luajit and no-crdt legs
  logged `Cache up-to-date` at their post step.

`Grammar fuzz` 37815431881, attempt 1 of 1, `success`: `Grammar fuzz` 113442729654 and
`Grammar fuzz (asan-strict)` 113442729130.

#### Squashes

`a27cdc2` and `4fade33` are both signature `E`, GitHub's key, as every merge since `d97e137`.
Both have empty trailers, no attribution and no line beginning with `#`. `a27cdc2`'s only diff
from `1218ba1` is this file, the registry commits `main` gained; `4fade33`'s diff from
`ca8b2f6` is empty. Both are faithful.

#### `main` goes toward the release red on four filed rows

Said plainly, because the tag's notes must say what the tagged commit's own CI says.
`main`'s last two code-bearing runs are both `failure`, each on rows filed before or by this
record and none of them rerun:

- at `a27cdc2`, two red rows on `Test (crdt)`: #282's fifth and #316's fourth;
- at `4fade33`, `main`'s tip, two red rows on the two `lua54` legs: #323's third (macOS) and
  #334's first (Ubuntu). `Test (crdt)` passed there, which retires neither #282 nor #316.

A tag on `4fade33`, or on a docs-only commit above it (whose own run skips every test leg
under `ci.yml`'s changed-paths rule), has `CI` 37815431889 as the last run of its code.

#### #328 reopened

GitHub closed #328 at 14:38:58Z, a second after #329 merged, as `completed`: #329's body says
"It may not fix #328", GitHub reads `fix #328` there as a closing keyword, and #329's closing
references list #328 alone. No ruling closed it, and nothing above retires it, so it is
reopened with that said. #330 and #331 stay open; neither is sampled here.

So, at E8b's close: #282 moves to five, #316 to four and #323 to three; #332, #333 and #334
open at one each; #271 and #276 stay at one; #328 is reopened, and #330 and #331 stay open,
none of the three sampled here; #311's list stands at twenty-nine. Every other count above
stands.

### The pre-release PR: PR #337's run at `bca69d4`, #316's fifth, its gate with #251's third and R7's thirty-first, and #311's list at thirty

Recorded 2026-10-08 by the pre-release pull request, #337 at `bca69d4` (five signed commits on
`391bf96`): the workspace to 2.0.0, `syntax.isolation`'s `none` removed, markdown's highlight
overlays, the release's replay of its archived worker, and the release notes. The run was read
from the jobs endpoint by conclusion and through all six test legs' logs; none was rerun.

#### `main` at `391bf96`, E8b's closing registry push

`CI` 37820758591, `push`, attempt 1 of 1, `success`, 15 = 6 success + 9 skipped (`Docs
consistency` 113460959927 among the six); `Grammar fuzz` 37820758476 `success`.

#### `CI` 37835832078 at `bca69d4`, attempt 1 of 1: #316's fifth

`pull_request`, the merge `fc1dab2` (`bca69d4` into `391bf96`, read from the test logs' checkout
line), created 19:57:30Z and completed 20:27:55Z: `failure`.

Tally (pr337-ci-bca69d4): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Format | 113512480482 | success |
| Lint (lua54) | 113512480884 | success |
| Commit attribution (D9) | 113512480891 | success |
| Changed paths | 113512480972 | success |
| Lint (luajit) | 113512481006 | success |
| M4 Perf Gates | 113512555207 | success |
| M5 Perf Gates | 113512555213 | success |
| Perf budgets (debug) | 113512555350 | success |
| Test (crdt) | 113512555371 | success |
| GPU Render (headless) | 113512555453 | success |
| Test (macos-latest / lua54) | 113512555503 | failure |
| M10 Perf Gates (crdt) | 113512555517 | success |
| Test (macos-latest / luajit) | 113512555524 | success |
| M1 Acceptance Gates | 113512555531 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113512555545 | success |
| M6 Perf Gates | 113512555551 | success |
| Test (ubuntu-latest / lua54) | 113512555655 | success |
| Test (ubuntu-latest / luajit) | 113512555744 | success |
| Docs consistency | 113512557608 | skipped |

Tally (pr337-ci-bca69d4-failures): 1 row of the table above with `result` = `failure`.

- `Test (macos-latest / lua54)` 113512555503, `failure`:
  `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`,
  **#316's fifth occurrence**, its first on a macOS leg and its third in CI. All three
  fragments are present: `kill 1: nothing held the switch's follow-up parse, which started a
  worker at once and installed`; `death=killed, " +  ` followed in the same line by
  `parse:killed`, with `tree=true` and `unit=pid_35917` in the report; and `test result:
  FAILED. 10 passed; 1 failed; 0 ignored` (`finished in 26.08s`). It was the job's only
  failure, against 191 `test result: ok`. The failing kill is the first, on `a.rs`. The
  branch touches this suite only to retheme `MARK` from `text` to the markdown overlays' six
  capture names, and `builtin/runtime/syntax.lua`'s dispatch only to read the boundary
  through one local function that returns the same `process`; that is read, not a
  demonstration. Commented on #316, not rerun.
- The other five test legs pair every `running` line with `test result: ok`: 194 on macOS
  luajit, 193 on each Ubuntu `test` leg, 192 on `Test (crdt)`. The branch's new rows passed on
  every leg that builds them.
- **#323's row** passed on all six legs. The branch removes `syntax.isolation`'s `none`, and
  the row's in-editor arm with it, so it runs its `process` arm alone: `_parse_now mid-parse in
  process:` blocked 2,992 to 3,069 ms. #334's and #282's rows passed on every leg, which is
  non-reproduction and nothing more.

#### `Grammar fuzz` 37835830812 at `bca69d4`

Attempt 1 of 1, `success`, created 19:57:30Z and completed 20:55:59Z: `Grammar fuzz`
113512479860 and `Grammar fuzz (asan-strict)` 113512480320. The job fuzzed, the branch adding
two query overlays under `builtin/queries/`, but its log reads `fuzz-grammars: changed since
391bf96…, run 600 s:` with no grammar named, so every grammar, markdown and markdown_inline
among them, ran 15 s and at least 300 mutations: **#336**, filed on this branch, seen on its
own run. The ten-minute markdown run is taken locally (the pull request's body).

#### The pull request's gate

`scripts/gate --protocol`, `20261008T195204Z-1042685` at `bca69d4`: six of eight. fmt, clippy,
clippy-luajit, doc, build and diff-check ok. `06-sweep`: 201 paired result lines, 5,359
passed, 3 failed, 64 ignored; `07-sweep-luajit`: 201 paired, 4,920 / 1 / 50. Each failure
matched by its required fragments:

- #311's row in both sweeps (`Could not find module ‘Prelude’`, the trace reaching
  `processi`), so #311's list gains it;
- R7's row in `06-sweep`, **R7's thirty-first** (the list below its table), passing in
  `07-sweep-luajit`;
- `--lib daemon::tests::a_panel_pointer_without_a_present_declaration_is_dropped` in
  `06-sweep`, **#251's third occurrence**: `bundled package `repl` failed to load` and
  `pmacs/bundled-load:1: bundled package failed to load: repl`, both verbatim, the panic site
  moved to `src/editor.rs:1143` with the file; the lib binary passed in `07-sweep-luajit`.
  Every gate's ambient data root is fresh, so the version bump renames the directory the
  race is over and widens nothing. Commented on #251.

Another project's gate ran on the laptop during the sweep (a one-minute load average of 48.98
near its start). The gate's own summary and survivor post-step printed to a file the session
could no longer read. Checked by hand after it: no `pmacs --daemon` carried the run's `TMPDIR`
in its environment, and the run's `TMPDIR` was gone, which the gate removes at its end. #316's
row passed in both sweeps, which is non-reproduction and nothing more.

So, at the pre-release PR: #316 moves to five, #251 to three, R7 to thirty-one; #311's list of
runs reads at thirty; #323 stands at three, its row's in-editor arm removed on the branch;
every other count above stands.

### The pre-release PR's fix round 1: PR #337's run at `334cc13`, #282's sixth, its gate with #316's sixth and #339's first, and #311's list at thirty-one

Recorded 2026-10-09 by the pre-release pull request's fix round 1, #337 at `334cc13` (eight
signed commits on `391bf96`, three of them this round's: the grid's syntax painter under wrap,
markdown's strong and emphasis faces, and the notes). The run was read from the jobs endpoint by
conclusion and through all six test legs' logs; none was rerun.

#### `CI` 37861280461 at `334cc13`, attempt 1 of 1: #282's sixth

`pull_request`, the merge `ac884c4` (`334cc13` into `69e3f55`, read from the test logs' checkout
line), created 23:46:30Z and completed 00:18:37Z: `failure`.

Tally (pr337-ci-334cc13): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 113597336670 | success |
| Format | 113597336802 | success |
| Lint (lua54) | 113597336895 | success |
| Changed paths | 113597336901 | success |
| Lint (luajit) | 113597337025 | success |
| GPU Render (headless) | 113597401919 | success |
| Test (crdt) | 113597401984 | success |
| M6 Perf Gates | 113597401985 | success |
| M1 Acceptance Gates | 113597402009 | success |
| Perf budgets (debug) | 113597402015 | success |
| Test (macos-latest / lua54) | 113597402033 | success |
| Test (ubuntu-latest / lua54) | 113597402043 | success |
| M5 Perf Gates | 113597402069 | success |
| M10 Perf Gates (crdt) | 113597402076 | success |
| Test (ubuntu-latest / luajit) | 113597402086 | success |
| M4 Perf Gates | 113597402104 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113597402108 | failure |
| Test (macos-latest / luajit) | 113597402143 | success |
| Docs consistency | 113597403860 | skipped |

Tally (pr337-ci-334cc13-success): 17 rows of the table above with `result` = `success`.

Tally (pr337-ci-334cc13-failures): 1 row of the table above with `result` = `failure`.

- `Test (ubuntu-latest / luajit, no crdt)` 113597402108, `failure`:
  `e7b_review_wire_acceptance::a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`,
  **#282's sixth occurrence**, all three fragments read against the job's own log: `*lsp* names
  the request and the code: []`; `WIRE *errors* gained ["[lsp] LSP: default-rust refused
  textDocument/prepareRename as a client error, -32602 InvalidParams: No references found at
  position"]` and `WIRE label "ready", last_error None`; and `test result: FAILED. 4 passed; 1
  failed; 2 ignored` (`running 7 tests`, `finished in 57.86s`). Between the row's stdout header
  and its panic the trace holds 94 `$/progress` frames, 78 of them `rustAnalyzer/cachePriming`
  reports. It was the job's only failure, against 190 `test result: ok`. The round's commits
  touch nothing on the LSP path; that is read, not a demonstration. Commented on #282, not
  rerun.
- The other five test legs pair every `running` line with `test result: ok`: 194 on each macOS
  leg, 193 on each other Ubuntu `test` leg, 192 on `Test (crdt)`. The round's new rows (the
  wrapped render_frame row, the painter's oracle, the two layout rows and the faces' pin) passed
  on every leg.
- **#323's row** passed on all six legs, its `process` arm alone blocking 2,992 to 3,163 ms.
  #316's and #339's rows passed on every leg, which is non-reproduction and nothing more.

#### `Grammar fuzz` 37861280552 at `334cc13`

Attempt 1 of 1, `success`, created 23:46:30Z and completed 00:36:12Z: `Grammar fuzz`
113597336867 and `Grammar fuzz (asan-strict)` 113597337181, both on GCC 13.3.0. Its log again
reads `fuzz-grammars: changed since 391bf96…, run 600 s:` with no grammar named, so
markdown_inline, whose overlay the round changed, ran 15 s after its seeds like every grammar:
300 mutated inputs, 763 inputs, 8,366 parses, no finding, its worker replay 48 inputs, 47
answered, 1 contained, 0 crashed, in each job: **#336**, seen again. The ten-minute
markdown_inline run is taken locally (the pull request's body).

#### The pull request's gate

`scripts/gate --protocol`, `20261008T232032Z-2370178` at `334cc13`: six of eight. fmt, clippy,
clippy-luajit, doc, build and diff-check ok; no `pmacs --daemon` survived (identified by
environ). `06-sweep`: 201 paired result lines, 5,366 passed, 1 failed, 64 ignored;
`07-sweep-luajit`: 201 paired, 4,923 / 3 / 50. Each failure matched by its required fragments:

- #311's row in both sweeps (`Could not find module ‘Prelude’`, the trace reaching
  `processi`), so #311's list gains it;
- `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
  in `07-sweep-luajit`, **#316's sixth occurrence** under the reading its third was counted by:
  the assertion's template, `kill 2: nothing held the switch's follow-up parse, which started a
  worker at once and installed`, and `death=killed, " +* ` followed in the same line by
  `parse:killed`, with `busy=false`, `tree=true` and `unit=pid_2605001`; `test result: FAILED.
  10 passed; 1 failed; 0 ignored` (`finished in 15.02s`). Its two literal fragments name the
  first kill, so as written they do not match; commented on #316, a split being the owner's
  call. The row passed in `06-sweep`;
- `e7h_grammar_gate_acceptance::e7h_a_crash_only_a_sequence_brings_back_is_found_and_fails_the_run`
  in `07-sweep-luajit`, **#339, filed, its first occurrence**: the report row holding
  `"crashes=0"`, `"in_sequence=0"` and `"unconfirmed=1"`, `- **lua** hang `one parse over 1000
  ms`, reproduced: no`, and `test result: FAILED. 22 passed; 1 failed; 2 ignored`. It passed
  in `06-sweep` and in both sweeps of the previous gate.

A niced single-grammar `scripts/fuzz-grammars` run (one fuzz worker) ran beside the gate. R7's
and #251's rows passed in both sweeps, which is non-reproduction and nothing more.

So, at the pre-release PR's fix round 1: #282 moves to six and #316 to six; #339 is filed at
one; #311's list of runs reads at thirty-one; #323 stands at three, its row's in-editor arm
removed on the branch (the pull request's body names it and #310 as closing at the merge);
every other count above stands.

### The pre-release PR's fix round 2: review 1's gate and the round's two, PR #337 green at `c915c51` and cancelled at `29b5b81`, #291 at sixteen, #316 at nine, #304's fourth, #340, #342, #343 and #344 filed, and #311's list at thirty-four

Recorded 2026-10-09 by the pre-release pull request's fix round 2, #337 at `29b5b81` (fourteen
signed commits on `391bf96`, six of them this round's: the bounded wrapped layout, #338's three
painters on it, and four to the notes, the last the CI paragraph). Review 1's gate, which its
record left for this update, is read here from its own logs. The runs were read from the jobs
endpoint by conclusion and through every test leg's log; none was rerun.

#### Review 1's gate: `20261009T080847Z-2899901` at `334cc13`

`scripts/gate --protocol`, run by review 1 from the clean release worktree and left by it for
this update: six of eight. fmt, clippy, clippy-luajit, doc, build and diff-check ok; no
`pmacs --daemon` survived (identified by environ). `06-sweep`: 201 paired result lines, 5,364
passed, 3 failed, 64 ignored; `07-sweep-luajit`: 201 paired, 4,922 / 4 / 50. Read from its logs
by fix round 2, each failure by its fragments:

- #311's row in both sweeps (`Could not find module ‘Prelude’`, the trace reaching
  `LSP:ready·processi`), so #311's list gains it;
- `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
  in both sweeps, on `kill 1: nothing held the switch's follow-up parse, which started a worker
  at once and installed`, the report holding `busy=true`, `tree=true`, `unit=pid_` and
  `death=killed` and the mode line `" +  … parse:killed !1`. In `07-sweep-luajit` with
  `test result: FAILED. 10 passed; 1 failed; 0 ignored`, every literal fragment of the issue:
  **#316's eighth**. In `06-sweep` the same, but `test result: FAILED. 9 passed; 2 failed; 0
  ignored`, the binary's second failure being #340's row (below): **#316's seventh**, its
  result-line fragment differing for that reason and no other. The review commented both on
  #316 and recommended amending the issue's signature to list its iteration and result-line
  variants rather than splitting it; that amendment is the owner's;
- `e7i_review3_probes::e7i_review3_a_deadline_that_cuts_only_layers_keeps_its_current_spans`
  in `06-sweep`, panicking `the warm parse installed` at `tests/e7i_review3_probes.rs:665`:
  **#340, filed by the review, its first occurrence**. It passed in `07-sweep-luajit`;
- in `07-sweep-luajit`, `pmacs-gpu`'s `tests::caret_inside_a_span_shows_source_exactly_as_if_math_were_disabled`
  (`caret-inside must render the raw source, exactly`), the two frames differing in exactly 44
  pixels at x = 102–103, y = 16–37, the review's parse of the 230,400-byte arrays: **#304's
  fourth**; and beside it `tests::overwide_status_runs_never_wrap_and_keep_the_suffix_pinned`
  (`assertion failed: min_y >= band_top && max_y <= height`), recorded on #304 as its sibling
  has been since E7h; `test result: FAILED. 373 passed; 2 failed; 0 ignored`. Commented on #304
  by the review.

#### The round's gate: `20261009T122310Z-3757634` at `2ad0b6c`

`scripts/gate --protocol` at `2ad0b6c`, the round's code with its first two notes commits
(`c915c51` changes only the notes): six of eight. fmt, clippy, clippy-luajit, doc, build and
diff-check ok; no `pmacs --daemon` survived (identified by environ). `06-sweep`: 201 paired
result lines, 5,372 passed, 2 failed, 65 ignored; `07-sweep-luajit`: 201 paired, 4,932 / 1 /
51. Each failure by its fragments:

- `e7e_haskell_acceptance::e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error`
  in `06-sweep` with **#291's** three fragments, not #311's: `HLS reports the type error; store
  []`, the trace `[(0, "LSP:init"), (5256, "LSP:ready"), (5459, "LSP:ready·setting"), (6942,
  "LSP:ready")]`, reaching `LSP:ready·setting` and never `processi`, and `test result: FAILED. 4
  passed; 1 failed; 0 ignored` (126.47 s): **#291's fifteenth**, its list above extended. In
  `07-sweep-luajit` the same row failed with #311's fragments (`Could not find module
  ‘Prelude’`, the trace reaching `LSP:ready·processi`), so #311's list gains the gate. Both
  commented;
- `compile_mode_acceptance::acc14_malformed_rule_containers_fail_closed` in `06-sweep`:
  `built-in defaults still parse under a non-table container`, `left: 0`, `right: 1`, and
  `test result: FAILED. 74 passed; 1 failed; 0 ignored` (`running 75 tests`). No row matched,
  so it is filed as **#342**, first occurrence, with what `compile.lua`'s ordering reads as
  (the pending line is parsed before the exit marker is appended); it passed in
  `07-sweep-luajit`.

R7's, #251's, #282's, #304's two, #316's, #339's and #340's rows passed in both sweeps, which is
non-reproduction and nothing more; so did every row the round added.

Beside the gate, a hand run of the whole `gate_script_acceptance` binary in the gate's
environment failed `ci_apt_update_bounds_a_stalled_update_and_retries_it` twice in fourteen
runs with `/bin/sh: bad interpreter: Text file busy` on its freshly written `bin/sudo` stub,
and passed thirty of thirty alone: #250's mechanism on a new selector, commented on #250, its
count there the owner's. Not a gate stage and not counted here.

#### `CI` 37932862797 at `c915c51`, attempt 1 of 1: green on every job

`pull_request`, the merge `26e4478` (`c915c51` into `c346a58`, read from the test logs' checkout
line), created 12:52:01Z and completed 13:24:38Z: `success`.

Tally (pr337-ci-c915c51): 19 = 18 + 1.

Tally (pr337-ci-c915c51-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 113827617732 | success |
| Changed paths | 113827617992 | success |
| Format | 113827618010 | success |
| Lint (luajit) | 113827618011 | success |
| Lint (lua54) | 113827618201 | success |
| GPU Render (headless) | 113827678989 | success |
| M4 Perf Gates | 113827678997 | success |
| M1 Acceptance Gates | 113827679033 | success |
| M10 Perf Gates (crdt) | 113827679043 | success |
| Test (crdt) | 113827679062 | success |
| M6 Perf Gates | 113827679085 | success |
| Test (ubuntu-latest / luajit) | 113827679090 | success |
| M5 Perf Gates | 113827679093 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113827679141 | success |
| Test (ubuntu-latest / lua54) | 113827679177 | success |
| Test (macos-latest / lua54) | 113827679247 | success |
| Perf budgets (debug) | 113827679256 | success |
| Test (macos-latest / luajit) | 113827679307 | success |
| Docs consistency | 113827681060 | skipped |

Tally (pr337-ci-c915c51-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 192 on `Test (crdt)`, 193 on
  each Ubuntu `test` leg, 194 on each macOS leg, none failed. The round's rows (the layout's
  oracle, screenful, prefix and fold rows, the painter's screenful row, the three wrap-suite rows
  and the diagnostics row) passed on every leg, and `Perf budgets (debug)` ran the new
  `wrapped_paint_of_a_five_megabyte_line_stays_within_budget`, `ok`.
- #282's, #316's, #339's, #340's and #342's rows passed on all six legs, which is
  non-reproduction and nothing more.

#### `Grammar fuzz` 37932862776 at `c915c51`

Attempt 1 of 1, `success`, created 12:52:01Z and completed 13:49:56Z: `Grammar fuzz` 113827617893
and `Grammar fuzz (asan-strict)` 113827617554, GCC 13. The round touches no grammar, query or
vendored file, so every grammar ran 15 s after its seeds. Both jobs reported the findings the
run at `334cc13` (37861280552) reported, none a crash and each contained in the worker replay:
json's allocation, `one input grew RSS by over 1024 MB`, at the same 135,120-byte minimum
(returning in 4,409 ms at a peak of 1.1 GB under ASan), and slow cmake, CUDA and Lua inputs, #293's
class. `--fail-on crashes` passes them, as it is meant to.

#### The tip's gate: `20261009T135200Z-4043146` at `29b5b81`

`scripts/gate --protocol` at the tip, whose tree differs from `2ad0b6c`'s only in the release
notes: six of eight. fmt, clippy, clippy-luajit, doc, build and diff-check ok; no `pmacs
--daemon` survived (identified by environ). `06-sweep`: 201 paired result lines, 5,373 passed, 1
failed, 65 ignored; `07-sweep-luajit`: 201 paired, 4,931 / 2 / 51. Each failure by its
fragments:

- the HLS row in `06-sweep` with #311's fragments (`Could not find module ‘Prelude’`, the trace
  reaching `LSP:ready·processi`), so #311's list gains the gate;
- the same row in `07-sweep-luajit` with **#291's** three: `HLS reports the type error; store []`,
  the trace `[(0, "LSP:init"), (5243, "LSP:ready"), (5441, "LSP:ready·setting"), (6915,
  "LSP:ready")]`, never `processi`, and `test result: FAILED. 4 passed; 1 failed; 0 ignored`
  (126.46 s): **#291's sixteenth**, its list above extended;
- `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
  in `07-sweep-luajit` on its **second** kill, the sixth sample's shape: `kill 2: nothing held
  the switch's follow-up parse, which started a worker at once and installed`, the report
  `deaths=2 … busy=false … tree=true unit=pid_16122 death=killed`, the mode line `" +* …
  parse:killed !2`, and `test result: FAILED. 10 passed; 1 failed; 0 ignored`: **#316's ninth**
  under the template reading. It passed in `06-sweep`.

R7's, #251's, #282's, #304's two, #339's, #340's and #342's rows passed in both sweeps, which is
non-reproduction and nothing more; so did every row the round added. All three occurrences are
commented on their issues.

#### `CI` 37939900668 at `29b5b81`, attempt 1 of 1: cancelled on one leg, #343's and #344's first

`pull_request`, the merge `966001c` (`29b5b81` into `c346a58`), created 13:51:48Z and completed
14:37:35Z: `cancelled`. `29b5b81` changes only the release notes from `c915c51`; a pull
request's run classifies the whole pull request's diff from `main`, so it is a full run.

Tally (pr337-ci-29b5b81): 19 = 17 + 1 + 1.

Tally (pr337-ci-29b5b81-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (luajit) | 113851314040 | success |
| Lint (lua54) | 113851314173 | success |
| Commit attribution (D9) | 113851314274 | success |
| Changed paths | 113851314393 | success |
| Format | 113851314418 | success |
| Test (crdt) | 113851391449 | success |
| M1 Acceptance Gates | 113851391456 | success |
| GPU Render (headless) | 113851391513 | success |
| M6 Perf Gates | 113851391599 | success |
| M10 Perf Gates (crdt) | 113851391650 | success |
| M4 Perf Gates | 113851391654 | success |
| M5 Perf Gates | 113851391732 | success |
| Test (macos-latest / luajit) | 113851391742 | success |
| Perf budgets (debug) | 113851391785 | success |
| Test (ubuntu-latest / luajit) | 113851391800 | success |
| Test (ubuntu-latest / lua54) | 113851391865 | success |
| Test (macos-latest / lua54) | 113851391866 | cancelled |
| Test (ubuntu-latest / luajit, no crdt) | 113851391997 | success |
| Docs consistency | 113851394494 | skipped |

Tally (pr337-ci-29b5b81-success): 17 rows of the table above with `result` = `success`.

Tally (pr337-ci-29b5b81-cancelled): 1 row of the table above with `result` = `cancelled`.

- `Test (macos-latest / lua54)` 113851391866, `cancelled` at the job's 45-minute limit (its
  annotation `The job has exceeded the maximum execution time of 45m0s`, the `cargo test
  --all-targets …` step cut at 14:37:27Z), on two rows, each filed with its fragments:
  - `e7c_indicator_acceptance::e7c_fix_3_typing_after_a_save_moves_nothing_on_the_mode_line`,
    `no request outlived the threshold, so no indicator at 1360 ms` with the activity cell
    `⋯1 parse rust`, `left: Some("⋯1 parse rust                   ")`, `right: None`, and
    `test result: FAILED. 4 passed; 1 failed; 0 ignored`: **#343, its first occurrence**, the
    sibling row of #319's, which #321 retired by changing #319's row alone;
  - `lsp_latex_acceptance::two_markerless_documents_in_different_directories_do_not_share_a_server`,
    its sixty-second notice at 14:09:19.7Z and nothing after until the cut, the suite's other
    twenty rows `ok` and no `test result:` line: **#344, its first occurrence**, in #327's
    class (a row on that leg that did not return) but another row and suite.
  The leg had paired 119 suites with `test result: ok` and one with `FAILED` before the cut.
- The other five test legs pair every `running` line with `test result: ok` (192 on `Test
  (crdt)`, 193 on each Ubuntu `test` leg, 194 on macOS luajit). Both rows passed on all five,
  and on all six legs at `c915c51`, the same code. Not rerun; a rerun is the owner's.

#### `Grammar fuzz` 37939900956 at `29b5b81`

Attempt 1 of 1, `success`, completed 14:48:36Z: `Grammar fuzz` 113851314402 and `Grammar fuzz
(asan-strict)` 113851313899. The same standing findings as at `c915c51` and `334cc13` (json's
allocation, slow cmake, CUDA and Lua inputs), none a crash, each contained in the replay.

So, at the pre-release PR's fix round 2: #291 moves to sixteen; #316 to nine (its seventh and
eighth from review 1's gate and its ninth from the tip's, under the reading its third and sixth
were counted by, the seventh's result line differing only by #340's failure beside it); #304
to four; #340 is at one (filed by review 1), and #342, #343 and #344 at one each; #311's list of
runs reads at thirty-four; every other count above stands.

### The pre-release PR's fix round 3: its gate, PR #337 green at `f45fa10`, R7's thirty-second and #311's list at thirty-five

Recorded 2026-10-09 by the pre-release pull request's fix round 3, #337 at `f45fa10` (fifteen
signed commits on `391bf96`, one of them this round's: the selection under wrap on the screen's
layout, #341). The runs were read from the jobs endpoint by conclusion and through every test
leg's log; none was rerun.

#### The round's gate: `20261009T161057Z-421242` at `f45fa10`

`scripts/gate --protocol` at the tip: six of eight. fmt, clippy, clippy-luajit, doc, build and
diff-check ok; no `pmacs --daemon` survived (identified by environ). `06-sweep`: 201 paired
result lines, 5,375 passed, 1 failed, 65 ignored; `07-sweep-luajit`: 201 paired, 4,933 / 2 /
51. Each failure by its fragments:

- the HLS row in both sweeps with #311's fragments (`Could not find module ‘Prelude’`, the trace
  reaching `LSP:ready·processi`, `test result: FAILED. 4 passed; 1 failed; 0 ignored`), so
  #311's list gains the gate;
- in `07-sweep-luajit`, `pmacs-gpu`'s
  `attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`, `transient
  sequence must attach: Attach(Handshake(Io(Os { code: 32, kind: BrokenPipe, message: "Broken
  pipe" })))` at `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed; 0 ignored`: all
  three of R7's fragments, **R7's thirty-second**, its list above extended. `attach.rs` has no
  diff on the branch, and the row passed in the same gate's `06-sweep`.

No other row failed in either sweep: #316's, #340's, #342's, #304's two and the round's two rows
passed in both, which for the filed rows is non-reproduction and nothing more. The HLS row's
failures carry #311's fragments, not #291's.

#### `CI` 37960188510 at `f45fa10`, attempt 1 of 1: green on every job

`pull_request`, the merge `e39c28a` (`f45fa10` into `2cb4c49`, read from the test logs' checkout
line and the commit's parents), created 16:35:15Z and completed 17:07:12Z: `success`.

Tally (pr337-ci-f45fa10): 19 = 18 + 1.

Tally (pr337-ci-f45fa10-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (luajit) | 113920759637 | success |
| Changed paths | 113920759880 | success |
| Format | 113920759905 | success |
| Lint (lua54) | 113920759946 | success |
| Commit attribution (D9) | 113920760099 | success |
| M1 Acceptance Gates | 113920836092 | success |
| M4 Perf Gates | 113920836140 | success |
| Test (crdt) | 113920836194 | success |
| GPU Render (headless) | 113920836197 | success |
| Test (macos-latest / lua54) | 113920836357 | success |
| Test (ubuntu-latest / luajit) | 113920836379 | success |
| M6 Perf Gates | 113920836427 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113920836437 | success |
| Test (macos-latest / luajit) | 113920836449 | success |
| M5 Perf Gates | 113920836537 | success |
| M10 Perf Gates (crdt) | 113920836557 | success |
| Test (ubuntu-latest / lua54) | 113920836641 | success |
| Perf budgets (debug) | 113920836679 | success |
| Docs consistency | 113920838393 | skipped |

Tally (pr337-ci-f45fa10-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 192 on `Test (crdt)`, 193 on
  each Ubuntu `test` leg, 194 on each macOS leg, none failed. The round's two rows passed on
  every leg.
- On `Test (macos-latest / lua54)`, where `29b5b81`'s run was cancelled, #343's row
  (`e7c_fix_3_typing_after_a_save_moves_nothing_on_the_mode_line`) and #344's
  (`two_markerless_documents_in_different_directories_do_not_share_a_server`) passed. No row
  failed on any leg, so every filed row that ran there passed, which is non-reproduction and
  nothing more.

#### `Grammar fuzz` 37960188503 at `f45fa10`

Attempt 1 of 1, `success`, created 16:35:15Z and completed 17:34:14Z: `Grammar fuzz` 113920758667
and `Grammar fuzz (asan-strict)` 113920759115, GCC 13 on `ubuntu24 20261004.327.1`. The round
touches no grammar, query or vendored file, so every grammar ran 15 s after its seeds (seed 1).
Both jobs reported the standing findings of the runs at `29b5b81` and `c915c51`, none a crash:
json's allocation (`one input grew RSS by over 1024 MB`, minimal 135,120 and 133,383 bytes),
CUDA's slow input (245,920 bytes) and cmake's three (growth exponents 1.43 to 2.00), each
contained in the arm's worker replay, json's by the worker's memory watch and the rest at the
deadline. Lua's slow finding, which `29b5b81`'s `asan-strict` arm reported, did not come back;
`ubsan`'s replay stopped its slowest lua input at the deadline. **One finding is new to this
pull request's runs:** a markdown_inline allocation, `one
input grew RSS by over 1024 MB`, reproduced alone and minimized to 79,572 bytes (`ubsan`) and
79,652 (`asan-strict`), edits 0, a paragraph of `*_f` repeated after leading spaces; each arm's
replay contained it by the worker's memory watch. The runs at `29b5b81` and `c915c51` reported
no markdown_inline finding. Its shape is #296's delimiter run; that it is #296's defect is not
shown, and it is commented there. `--fail-on crashes` passes all of them, as it is meant to.

So, at the pre-release PR's fix round 3: R7 moves to thirty-two; #311's list of runs reads at
thirty-five; #343 and #344 did not recur; every other count above stands.

### #347's pull request: its gate, PR #348's runs at `2c890aa`, `main` green at `cd730c9` and `1d7306d`, R7's thirty-third and #311's list at thirty-six

Recorded 2026-10-09 by #347's pull request, #348 at `2c890aa` (one signed commit on `1d7306d`: the
fuzz harness's replay and race headers name the memory mechanism each worker has where it runs).
The runs were read from the jobs endpoint by conclusion and through every test leg's log; none was
rerun.

#### `main` at `cd730c9` and `1d7306d`, this branch's base

`cd730c9` is #337's squash, the 2.0.0 pull request, and `main`'s last code-bearing commit, so it
is this branch's base control; `1d7306d` changes `docs/releases/2.0.0.md` alone, and
`v2.0.0-rc.1` is cut on it. Neither run was recorded here before.

`CI` 37971932411 at `cd730c9`, attempt 1 of 1, `push`, created 18:14:50Z and completed 18:45:13Z:
`success`.

Tally (main-ci-cd730c9): 19 = 18 + 1.

Tally (main-ci-cd730c9-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 113960468172 | success |
| Changed paths | 113960468561 | success |
| Format | 113960468575 | success |
| Lint (luajit) | 113960468615 | success |
| Lint (lua54) | 113960468771 | success |
| GPU Render (headless) | 113960550007 | success |
| M1 Acceptance Gates | 113960550012 | success |
| M6 Perf Gates | 113960550065 | success |
| M4 Perf Gates | 113960550077 | success |
| M5 Perf Gates | 113960550084 | success |
| Test (crdt) | 113960550095 | success |
| Test (macos-latest / lua54) | 113960550147 | success |
| Perf budgets (debug) | 113960550152 | success |
| M10 Perf Gates (crdt) | 113960550205 | success |
| Test (ubuntu-latest / lua54) | 113960550219 | success |
| Test (ubuntu-latest / luajit) | 113960550231 | success |
| Test (ubuntu-latest / luajit, no crdt) | 113960550307 | success |
| Test (macos-latest / luajit) | 113960550340 | success |
| Docs consistency | 113960552034 | skipped |

Tally (main-ci-cd730c9-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 192 on `Test (crdt)`, 193 on
  each Ubuntu `test` leg, 194 on each macOS leg, none failed. Every filed row that ran there
  passed, which is non-reproduction and nothing more.

`Grammar fuzz` 37971932467 at `cd730c9`, attempt 1 of 1, `success`, created 18:14:50Z and completed
19:14:22Z: `Grammar fuzz` 113960469126 and `Grammar fuzz (asan-strict)` 113960468820, GCC 13 on
`ubuntu24 20261004.327.1`, both fuzzing (`reason=Cargo.lock changed`). No crash. Each arm reported
json's allocation (`one input grew RSS by over 1024 MB`, minimal 135,120 bytes), CUDA's slow input
(245,920 bytes), cmake's three slow inputs (exponents 1.30 to 2.25) and lua's slow ones (two under
`ubsan`, one under `asan-strict`), each with its exponent, and the arm's worker replay stopped every
one at the deadline, json's allocation included. The `ubsan` arm also reported two hangs that did
not reproduce alone, which do not fail a run: **haskell** (`one parse over 10000 ms`, `reproduced:
no`, minimal 107,087 bytes, edits 8) and **markdown_inline** (`reproduced: no`, 262,144 bytes, edits
8); the replay stopped each at the deadline. No issue or row names a haskell hang; it is recorded
and not filed. Each arm's replay header read `memory 1024 MiB by its watch`, the wording #348
changes.

`CI` 37979157781 at `1d7306d`, attempt 1 of 1, `success`, created 19:17:03Z and completed 19:19:13Z:
the docs-only path, `Docs consistency` 113985034406 among the six that ran and every test leg
skipped. `Grammar fuzz` 37979158009 `success`, fuzzing nothing (`run=false`, `no changed path can
change the grammar set`).

Tally (main-ci-1d7306d): 15 = 6 + 9.

`Release` 37979237576 at `1d7306d`, the tag `v2.0.0-rc.1`, attempt 1 of 1, `success`: `Preflight
(tag/version, ancestry)` 113985224690, `Build linux-x86_64` 113985354727, `Build macos-arm64`
113985354736 and `Publish release` 113989356541, each `success`. Both builds replayed their staged
worker and found no crash. Their replay headers are #347: `Build macos-arm64` line 1171 reads
`memory 1024 MiB by RLIMIT_AS, deadline 5000 ms` above line 1198's `markdown contained:
…/296-underscores-16k.input, memory: the worker's watch`, on the platform that refuses
`RLIMIT_AS`; `Build linux-x86_64` lines 1264 and 1291 read `RLIMIT_AS` twice, rightly.

#### The pull request's gate: `20261009T205956Z-1456466` at `2c890aa`

`scripts/gate` at the tip: five of six. fmt, clippy, doc, build and diff-check ok; no `pmacs
--daemon` survived (identified by environ). `05-sweep`: 201 paired result lines, 5,376 passed,
2 failed, 65 ignored. Each failure by its fragments:

- the HLS row with #311's fragments (`Could not find module ‘Prelude’`, the trace reaching
  `LSP:ready·processi`, `test result: FAILED. 4 passed; 1 failed; 0 ignored`), so #311's list
  gains the gate;
- `pmacs-gpu`'s `attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`,
  `transient sequence must attach: Attach(Handshake(Io(Os { code: 32, kind: BrokenPipe, message:
  "Broken pipe" })))` at `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed; 0
  ignored`: all three of R7's fragments, **R7's thirty-third**, its list extended. `pmacs-gpu`
  has no diff on the branch.

No other row failed, so every filed row the sweep ran passed, which is non-reproduction and
nothing more. The sweep's total is the pre-release fix round 3 gate's `06-sweep` (5,375 + 1, the
same code) plus the branch's two new rows.

#### `CI` 37992725769 at `2c890aa`, attempt 1 of 1: #307's third and #349 filed, on macOS luajit

`pull_request`, the merge `e1fb242` (`2c890aa` into `1d7306d`, read from the test logs' checkout
line and the commit's parents), created 21:19:17Z and completed 22:05:08Z: `cancelled`, one job cut
at its limit.

Tally (pr348-ci-2c890aa): 19 = 17 + 1 + 1.

Tally (pr348-ci-2c890aa-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 114030789425 | success |
| Changed paths | 114030789650 | success |
| Commit attribution (D9) | 114030789665 | success |
| Lint (lua54) | 114030789756 | success |
| Lint (luajit) | 114030789818 | success |
| M5 Perf Gates | 114030854658 | success |
| GPU Render (headless) | 114030854708 | success |
| M4 Perf Gates | 114030854728 | success |
| M10 Perf Gates (crdt) | 114030854761 | success |
| M1 Acceptance Gates | 114030854762 | success |
| Test (crdt) | 114030854771 | success |
| M6 Perf Gates | 114030854774 | success |
| Test (macos-latest / lua54) | 114030854885 | success |
| Perf budgets (debug) | 114030854893 | success |
| Test (ubuntu-latest / luajit) | 114030854899 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114030854907 | success |
| Test (ubuntu-latest / lua54) | 114030854947 | success |
| Test (macos-latest / luajit) | 114030854995 | cancelled |
| Docs consistency | 114030856379 | skipped |

Tally (pr348-ci-2c890aa-success): 17 rows of the table above with `result` = `success`.

Tally (pr348-ci-2c890aa-cancelled): 1 row of the table above with `result` = `cancelled`.

- `Test (macos-latest / luajit)` (114030854995), cancelled with the annotation `The job has
  exceeded the maximum execution time of 45m0s`, two reds in it:
  - `e7b_review_wire_acceptance::a_save_sends_did_save_and_rust_analyzer_flychecks_on_it`, all
    three of #307's fragments (`the check showed as a suffix on ready, or as the tracker's busy
    title under a reload`; the wire after the save `["6 = LSP:ready"]` with the busy field
    `["6 b -"]`; `test result: FAILED. 4 passed; 1 failed; 2 ignored`): **#307's third**,
    commented, not rerun;
  - then `m4_acceptance::rd11b_a_dangling_symlink_counts_as_present` ran at least 19 minutes
    without returning (its sixty-second notice at 21:45:44.8Z, `The operation was canceled.` at
    22:05:00.6Z; 181 of the suite's 182 rows reported, 175 `ok` and 6 `ignored`, and no `test
    result:` line): **#349, filed, its first occurrence**. By the matching rule it is neither #327
    nor #344, other rows in other suites, both under `lua54`. Before it the leg had paired 129
    suites with `test result: ok` and one with `FAILED`.
- Every other test leg pairs each `running` line with a `test result: ok`: 192 on `Test (crdt)`,
  193 on each Ubuntu `test` leg, 194 on `Test (macos-latest / lua54)`; none failed.
- #347's five rows passed on all six test legs. On the two macOS legs the replay's default
  enforcement is the worker's watch, so there the header and the findings agreed on the platform
  that refuses `RLIMIT_AS`, which `v2.0.0-rc.1`'s log did not.

#### `Grammar fuzz` 37992725848 at `2c890aa`

Attempt 1 of 1, `success`, created 21:19:17Z and completed 22:13:59Z: `Grammar fuzz` 114030794048
and `Grammar fuzz (asan-strict)` 114030793775, GCC 13 on `ubuntu24 20261004.327.1`, both fuzzing
because the harness changed (`reason=src/bin/pmacs_grammar_fuzz.rs changed`). No crash, and no
worker crashed in either replay. Both reported the standing findings of `main`'s run at `cd730c9`:
json's allocation (135,120 bytes), CUDA's slow input (245,920), cmake's three (exponents 1.49 to
2.20) and lua's (252,415 bytes, exponent 0.94 under `ubsan` and 0.99 under `asan-strict`); `ubsan`
also its markdown_inline hang that did not reproduce alone (262,144 bytes), which does not fail a
run. The haskell hang `main`'s `ubsan` arm reported did not come back. **The change, run in CI:**
each arm's replay, which passes `--enforcement watch`, printed `memory 1024 MiB by the worker's
watch` above its #296 findings' `memory: the worker's watch`, where `main`'s arms printed `by its
watch`.

So, at #347's pull request: R7 moves to thirty-three and #307 to three; #349 is filed at one;
#311's list of runs reads at thirty-six; every other count above stands.

### `main` after E7h: run 37018782385 at `a013d46`, and it is GREEN

Read on 2026-10-02 at E7i.0 from the jobs endpoint and all six test
legs' logs; not re-run. `a013d46` is E7h's squash merge (PR #297 at
`64d396b`, merged 14:17:01Z) and E7i's base control.

| field | value |
|---|---|
| run | 37018782385, `push`, one attempt |
| head | `a013d46`, E7h's squash merge |
| window | created 2026-10-02T14:17:05Z, updated 14:50:52Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |
| the six test legs | `Test (crdt)` 176 `test result: ok`, `Test (ubuntu-latest / luajit)`, `Test (ubuntu-latest / lua54)` and `Test (ubuntu-latest / luajit, no crdt)` 177 each, both macOS legs 178 each; `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; every log's `running N tests` lines paired one to one with its result lines, each macOS log's one unpaired `running 1 test` being the adapter step's own `grep` text |
| `Perf budgets (debug)` | success; #308's selector, `composition_overhead_under_ten_percent`, `ok` |

Tally (run-37018782385-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 110876175642 | success |
| Lint (luajit) | 110876176030 | success |
| Commit attribution (D9) | 110876176137 | success |
| Format | 110876176143 | success |
| Lint (lua54) | 110876176302 | success |
| GPU Render (headless) | 110876256823 | success |
| M1 Acceptance Gates | 110876256837 | success |
| Test (crdt) | 110876256892 | success |
| Test (ubuntu-latest / luajit, no crdt) | 110876256918 | success |
| M10 Perf Gates (crdt) | 110876256986 | success |
| Test (macos-latest / lua54) | 110876256999 | success |
| Test (ubuntu-latest / lua54) | 110876257028 | success |
| M4 Perf Gates | 110876257042 | success |
| M5 Perf Gates | 110876257054 | success |
| Perf budgets (debug) | 110876257062 | success |
| M6 Perf Gates | 110876257071 | success |
| Test (macos-latest / luajit) | 110876257105 | success |
| Test (ubuntu-latest / luajit) | 110876257294 | success |
| Docs consistency | 110876258957 | skipped |

#### `Grammar fuzz` 37018782512 at `a013d46`

Both legs succeeded:
- `Grammar fuzz`, job 110876177142, 14:17:09–15:46:36Z, 89.5 min;
- `Grammar fuzz (asan-strict)`, job 110876176755, 14:17:09–15:49:14Z, 92.1 min.

Eight grammars changed since `3f3d0fe` and ran 600 s (bash, haskell,
html, lean4, markdown, markdown_inline, python, yaml); the other
fourteen ran their 15 s smoke. Neither leg found a crash or a
reproduced hang. In each, one markdown_inline input passed the 4 GB cap
(its minimum 130,027 bytes, cut at 4.0 GB after 20.7 s under `ubsan`
and 19.5 s under `asan-strict`) and was reported "known, accepted
(#296)". The `ubsan` leg filed two hangs that did not reproduce alone,
bash (235,155 bytes) and lua (26,566), which do not fail a run. Slow
findings, each with its exponent: 14 under `ubsan` and 18 under
`asan-strict`, one of them mispredicted (cmake under `asan-strict`,
exponent 1.04, returned in 1,153,616 ms against a budget of 460,346).
zig reached 28 mutated inputs in each leg, as before.

Nothing sampled: R7, #283, #291, #298, #300 and #303 to #308 did not
recur here, which is non-reproduction and nothing more; every count
stands. The head is green, stated at the moment of writing.

### `main` after E7g: run 36495882005 at `4ad9f3e`, #295 filed and #254's second occurrence

Read at E7h's opening on 2026-09-29, from the jobs endpoint and all six
test legs' logs; not re-run. E7g review 1 found it unrecorded. `4ad9f3e`
is code-bearing (E7g's squash of `691aae6`, merged before review), so it
is E7h's base control itself.

| field | value |
|---|---|
| run | 36495882005, `push`, one attempt |
| head | `4ad9f3e`, E7g's squash merge (PR #294 at `691aae6`) |
| window | created 2026-09-28T23:03:01Z, updated 23:27:47Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |
| the six test legs | `Test (crdt)` 171 `test result: ok`, `Test (ubuntu-latest / luajit)` and `Test (ubuntu-latest / lua54)` 172 each, `Test (macos-latest / luajit)` 173, `Test (macos-latest / lua54)` 170 and one `FAILED`, `Test (ubuntu-latest / luajit, no crdt)` 169 and one `FAILED`; every log's `running N tests` lines paired one to one with its result lines; `WouldBlock`, `did not become ready` and `got ok` zero on every leg; E7g's nine `e7g_` rows `ok` on every leg |

Tally (run-36495882005-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Lint (luajit) | 109175225516 | success |
| Changed paths | 109175225770 | success |
| Format | 109175225785 | success |
| Commit attribution (D9) | 109175225821 | success |
| Lint (lua54) | 109175225888 | success |
| M1 Acceptance Gates | 109175268654 | success |
| M5 Perf Gates | 109175268694 | success |
| Perf budgets (debug) | 109175268711 | success |
| M4 Perf Gates | 109175268745 | success |
| Test (crdt) | 109175268751 | success |
| GPU Render (headless) | 109175268755 | success |
| M6 Perf Gates | 109175268787 | success |
| Test (ubuntu-latest / luajit) | 109175268838 | success |
| Test (macos-latest / luajit) | 109175268839 | success |
| M10 Perf Gates (crdt) | 109175268873 | success |
| Test (macos-latest / lua54) | 109175268935 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 109175268948 | failure |
| Test (ubuntu-latest / lua54) | 109175268989 | success |
| Docs consistency | 109175270310 | skipped |

Tally (run-36495882005-failures): 2 rows of the table above with `result` = `failure`.

The reds:

Tally (run-36495882005-reds): 2 items in the list below.

- `Test (macos-latest / lua54)` (109175268935): three rows of
  `e7b_review_wire_acceptance` ---
  `a_save_sends_did_save_and_rust_analyzer_flychecks_on_it`,
  `a_keystroke_after_the_save_resends_it_behind_the_did_change`,
  `a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`
  --- each failing its `wait_warm` precondition with `the label reads
  exactly ready for two seconds`, `test result: FAILED. 2 passed; 3
  failed; 2 ignored` (`finished in 97.26s`), the suite started at
  23:13:58Z and the rows failed at 23:15:00, 23:15:30 and 23:15:35. No
  row carries the fragment and no issue did: **filed as #295**, a first
  sample. The candidate is the three rows' concurrent rust-analyzer
  warm-ups on a three-core runner keeping each label off `ready` for
  the two-second window; the same rows ran `ok` on the other five legs.
  Log:
  https://github.com/levineuwirth/pmacs/actions/runs/36495882005/job/109175268935
- `Test (ubuntu-latest / luajit, no crdt)` (109175268948):
  `m6_5_repl_spawns_zsh`, `pump predicate did not become true within
  15000ms; chunk:` at `tests/m6_5_repl_acceptance.rs:115`, the chunk
  beginning `local h = _G.h` and polling for `status.kind ==
  "running"` --- both of #254's required fragments, so **#254's second
  occurrence**, on the sibling selector (#254 names
  `m6_5_repl_spawns_fish`; its fish-first-start candidate does not
  obviously apply to zsh, said on the issue). `test result: FAILED. 10
  passed; 1 failed` (`finished in 19.90s`) at 23:16:02Z, the leg's only
  failure; the row ran `ok` on the other five legs. Log:
  https://github.com/levineuwirth/pmacs/actions/runs/36495882005/job/109175268948

Neither touches E7g's code (grammars, the fuzz harness, the workflow);
`Grammar fuzz` 36495882068 at the same sha is green. So neither the
`read Hello` family, nor #259, nor U4, nor U17, nor #271, nor #276, nor
#253, nor #282, nor #283, nor #286, nor #288, nor #289, nor #291
sampled; #254 is at two and #295 at one. The head is not green, on two
rows off E7g's path, stated at the moment of writing.

### `main` after E7e: run 36315238338 at `0b72108`, #288's second occurrence

Read at E7g's opening on 2026-09-27, from the jobs endpoint and all
six test legs' logs; not re-run. `0b72108` is code-bearing (E7e's
squash), so it is E7g's base control itself.

| field | value |
|---|---|
| run | 36315238338, `push`, one attempt |
| head | `0b72108`, E7e's squash merge (PR #290 at `10cd443`) |
| window | created 2026-09-27T11:17:10Z, updated 11:40:28Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the merge changed code |
| the six test legs | `Test (crdt)` 169 `test result: ok`, `Test (ubuntu-latest / luajit)` 167 and one `FAILED`, `Test (ubuntu-latest / lua54)` and `Test (ubuntu-latest / luajit, no crdt)` 170 each, both macOS legs 171 each; every log's `running N tests` lines paired one to one with its result lines; `WouldBlock`, `did not become ready` and `got ok` zero on every leg; E7e's four `e7e_` rows `ok` on every leg, the HLS row by skipping (CI installs no HLS); #289's selector `ok` on all six legs and #286's on the five that build it |

Tally (run-36315238338-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Format | 108608591763 | success |
| Lint (lua54) | 108608591819 | success |
| Lint (luajit) | 108608591867 | success |
| Commit attribution (D9) | 108608591880 | success |
| Changed paths | 108608591905 | success |
| GPU Render (headless) | 108608614221 | success |
| Test (crdt) | 108608614226 | success |
| M5 Perf Gates | 108608614264 | success |
| Test (ubuntu-latest / luajit) | 108608614267 | failure |
| Test (ubuntu-latest / lua54) | 108608614268 | success |
| M10 Perf Gates (crdt) | 108608614271 | success |
| M4 Perf Gates | 108608614273 | success |
| M1 Acceptance Gates | 108608614279 | success |
| Perf budgets (debug) | 108608614286 | success |
| Test (macos-latest / lua54) | 108608614296 | success |
| M6 Perf Gates | 108608614306 | success |
| Test (ubuntu-latest / luajit, no crdt) | 108608614315 | success |
| Test (macos-latest / luajit) | 108608614317 | success |
| Docs consistency | 108608614905 | skipped |

The red:

- `Test (ubuntu-latest / luajit)` (108608614267): **#288's second
  occurrence** and its first on this leg ---
  `e7c_fix_3_typing_after_a_save_moves_nothing_on_the_mode_line`,
  `the check's diagnostic landed: ["pmacs-fake-lsp"]`, `test result:
  FAILED. 4 passed; 1 failed; 0 ignored` (`finished in 3.77s`), at
  11:29:08Z, the job's only failure. Both required fragments present,
  so it matches; the candidate stands as the row's `settle` returning
  at the first quiet tick before the save's check diagnostic lands, a
  wait weaker than its assertion. E7e touched neither the suite, the
  fake nor the save path. The same row ran `ok` on the other five legs.
  Log:
  https://github.com/levineuwirth/pmacs/actions/runs/36315238338/job/108608614267

So neither the `read Hello` family, nor #259, nor U4, nor U17, nor
#271, nor #276, nor #253, nor #282, nor #283, nor #286, nor #289
sampled; #288 is at two. The head is not green, on one row off E7e's
path, stated at the moment of writing.

### `main` after E7d: run 36262456503 at `dd2ccdb`, and it is GREEN

Owed since E7e, which read this run by its jobs' conclusions only;
read here on 2026-09-27 from the jobs endpoint and all six test legs'
logs; not re-run. `dd2ccdb` was E7e's base control.

| field | value |
|---|---|
| run | 36262456503, `push`, one attempt |
| head | `dd2ccdb`, E7d's squash merge (PR #287 at `ac61f97`) |
| window | created 2026-09-26T18:24:58Z, updated 18:46:26Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |
| the six test legs | `Test (crdt)` 168 `test result: ok`, `Test (ubuntu-latest / luajit)`, `Test (ubuntu-latest / lua54)` and `Test (ubuntu-latest / luajit, no crdt)` 169 each, both macOS legs 170 each; `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; every log's `running N tests` lines paired one to one with its result lines; #288's selector `ok` on all six legs |

Tally (run-36262456503-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 108460751198 | success |
| Format | 108460751395 | success |
| Lint (lua54) | 108460751410 | success |
| Lint (luajit) | 108460751429 | success |
| Commit attribution (D9) | 108460751459 | success |
| Test (crdt) | 108460775686 | success |
| GPU Render (headless) | 108460775707 | success |
| M1 Acceptance Gates | 108460775722 | success |
| M5 Perf Gates | 108460775732 | success |
| Test (ubuntu-latest / luajit, no crdt) | 108460775744 | success |
| M4 Perf Gates | 108460775752 | success |
| Perf budgets (debug) | 108460775760 | success |
| Test (macos-latest / lua54) | 108460775767 | success |
| Test (macos-latest / luajit) | 108460775776 | success |
| M6 Perf Gates | 108460775787 | success |
| Test (ubuntu-latest / luajit) | 108460775798 | success |
| M10 Perf Gates (crdt) | 108460775816 | success |
| Test (ubuntu-latest / lua54) | 108460775874 | success |
| Docs consistency | 108460776693 | skipped |

Nothing sampled: #288, red on PR #287's head run, did not recur here,
which is non-reproduction and nothing more; every count stands. The
head is green, stated at the moment of writing.

### #291's first sample, local, on E7g's branch

`scripts/gate` on `e7g/grammar-fuzz` at `549d1c5`, log
`20260927T124215Z-1010709`, step `05-sweep`:
`e7e_haskell_acceptance::e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error`,
`HLS reports the type error; store []` with a trace that reaches
`LSP:ready·setting` and never `LSP:ready·processi`, `test result:
FAILED. 4 passed; 1 failed; 0 ignored` in 125.85 s, the stage's only
failure (174 targets, 5096 passed, 1 failed, 59 ignored). The row is
armed by `PMACS_REQUIRE_HLS` and runs only on this laptop, never on
CI. No row matched; filed as **#291**. Measured before filing, alone
in the gate's environment: 1 in 8 at `549d1c5`, then 16 interleaved
pairs, `0b72108` 2 of 16 and `549d1c5` 1 of 16, every failure with
the same fragments and every pass (36) through `processi`. So E7e's
row was intermittent before E7g removed the Haskell grammar; not the
branch's. #291 is at one.

### PR #287's head run 36145942251 at `ac61f97`: #288, a first sample on an E7c witness row, not the branch's

E7d's head at C7d (word wrap on both frontends, the diagnostic at
point, the menu on screen, and one fixture). Read on 2026-09-26 from
the jobs endpoint and all six test legs' logs after the run had
completed; not re-run.

| field | value |
|---|---|
| run | 36145942251, `pull_request`, one attempt |
| head | `ac61f97`, PR #287 |
| window | created 2026-09-25T14:12:35Z, updated 14:37:50Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-36145942251-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Format | 108107019777 | success |
| Changed paths | 108107019947 | success |
| Lint (lua54) | 108107020018 | success |
| Lint (luajit) | 108107020171 | success |
| Commit attribution (D9) | 108107020196 | success |
| M6 Perf Gates | 108107100988 | success |
| M4 Perf Gates | 108107100993 | success |
| Perf budgets (debug) | 108107101108 | success |
| GPU Render (headless) | 108107101115 | success |
| Test (crdt) | 108107101138 | success |
| Test (ubuntu-latest / luajit) | 108107101158 | success |
| Test (macos-latest / lua54) | 108107101160 | success |
| Test (ubuntu-latest / lua54) | 108107101164 | failure |
| M5 Perf Gates | 108107101219 | success |
| M1 Acceptance Gates | 108107101266 | success |
| Test (macos-latest / luajit) | 108107101332 | success |
| M10 Perf Gates (crdt) | 108107101338 | success |
| Test (ubuntu-latest / luajit, no crdt) | 108107101350 | success |
| Docs consistency | 108107102880 | skipped |

Tally (run-36145942251-failures): 1 row of the table above with `result` = `failure`.

Every test leg's log read: `Test (crdt)` 168 `test result: ok`,
`Test (ubuntu-latest / luajit)` 169, `Test (ubuntu-latest / luajit,
no crdt)` 169, both macOS legs 170, each with zero `FAILED`; `Test
(ubuntu-latest / lua54)` 166 `ok` and one `FAILED`; every log's
`running N tests` lines paired one to one with its result lines (the
macOS legs' one extra match is the adapter step's own `grep -q
'running 1 test'`). `WouldBlock`, `did not become ready` and `got ok`
appear zero times on every leg; the branch's eight word-wrap rows,
three diagnostic-at-point rows and its store row `ok` on every leg,
its eight GPU rows `ok` in `GPU Render (headless)` (pmacs-gpu 374/0),
the fixture it corrected `ok` on every leg, and #286's selector `ok`
on the five legs that build it. The red:

- `Test (ubuntu-latest / lua54)`: **#288's first occurrence** ---
  `e7c_fix_3_typing_after_a_save_moves_nothing_on_the_mode_line`,
  `the check's diagnostic landed: ["pmacs-fake-lsp"]`, `4 passed; 1
  failed; 0 ignored`, the job's only failure. E7c fix round 3's
  witness: its `settle` returns at the first tick with no LSP request
  in flight and the label `ready`, a predicate weaker than the
  assertion after it (the save's check diagnostic in the store). Not
  the branch's: E7d touches neither the suite, the fake nor the save
  path; the row was `ok` on all six legs of the base control's run
  35588597754; 30 of 30 alone and 15 of 15 in its suite locally under
  `lua54,crdt` at `ac61f97`, non-reproduction and nothing more.

So neither the `read Hello` family, nor #259, nor U4, nor U17, nor
#271, nor #276, nor #253, nor #282, nor #283 sampled; #288 is at one.
The base control is `ff4aa18` (run 35588597754, green, recorded as
`807f8e3`). The head is not green, on one row off its path, stated at
the moment of writing; the disposition is the owner's, in D33's form.

### #286's first sample, local, on E7d's branch

`scripts/gate` on `e7d/prose` at `df516bc`, log
`20260925T132215Z-324100`, step `05-sweep`:
`m5_5_acceptance::m10_10_f14_production_path_keystroke_flows_to_broadcast`,
`F14: B must receive A's CrdtOp broadcast end-to-end` after its 2 s
deadline, `test result: FAILED. 40 passed; 1 failed` in 2.26 s, under
the default sweep's load, beside one fixture of the branch's own
(fixed as `ac61f97`). No row matched: #271's missed broadcast is
another selector (`m5_5_acceptance.rs:1169`) on a CI leg with two
other fragment groups, so this is filed alone as **#286**, #271 not
moved. The row passed alone three times, in its suite (41/0) and in
the next gate at `ac61f97` (`20260925T140322Z-413145`, six of six, 171
targets 5083/0/59), and on the five CI legs of the head run above that
build it: green samples, which retire nothing. #286 is at one.

### `main` after E7c: run 35588597754 at `ff4aa18`, and it is GREEN

Read at E7d's opening on 2026-09-25, from the jobs endpoint and all
six test legs' logs; not re-run. `ff4aa18` is code-bearing (E7c's
squash), so it is E7d's base control itself.

| field | value |
|---|---|
| run | 35588597754, `push`, one attempt |
| head | `ff4aa18`, E7c's squash merge (PR #284 at `3de1e1f`) |
| window | created 2026-09-21T10:24:45Z, updated 10:48:12Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |
| the six test legs | `Test (crdt)` 166 `test result: ok`, `Test (ubuntu-latest / luajit)`, `Test (ubuntu-latest / lua54)` and `Test (ubuntu-latest / luajit, no crdt)` 167 each, both macOS legs 168 each; `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; every log's `running N tests` lines paired one to one with its result lines (the macOS legs' one extra match is the adapter step's own `grep -q 'running 1 test'` echoed into the log); #282's selector `ok` on all six legs and #283's on the five that build it; the twenty-seven `e7c_` rows `ok` on every leg |

Tally (run-35588597754-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 106297625942 | success |
| Changed paths | 106297626239 | success |
| Format | 106297626251 | success |
| Lint (lua54) | 106297626402 | success |
| Lint (luajit) | 106297626537 | success |
| M5 Perf Gates | 106297688270 | success |
| GPU Render (headless) | 106297688282 | success |
| M4 Perf Gates | 106297688288 | success |
| Test (crdt) | 106297688301 | success |
| Test (ubuntu-latest / lua54) | 106297688335 | success |
| M1 Acceptance Gates | 106297688340 | success |
| Test (macos-latest / luajit) | 106297688341 | success |
| M6 Perf Gates | 106297688347 | success |
| M10 Perf Gates (crdt) | 106297688362 | success |
| Test (ubuntu-latest / luajit) | 106297688393 | success |
| Perf budgets (debug) | 106297688427 | success |
| Test (macos-latest / lua54) | 106297688454 | success |
| Test (ubuntu-latest / luajit, no crdt) | 106297688544 | success |
| Docs consistency | 106297689953 | skipped |

Nothing sampled: #282 and #283, red on the base control `361bb3b` and
on E7c's tip run, did not recur here, which is non-reproduction and
nothing more; every count stands. The head is green, stated at the
moment of writing.

### PR #284's fix-round-2 head run 35474631618 at `09798eb`: #283's third and a new row of the branch's own, #285

E7c's head after fix round 2 (the diagnostic-drop rule, switch-buffer's
accept policy, a mode-line measurement). Read at the round's close on
2026-09-20 from the jobs endpoint and all six test legs' logs after the
run had completed; not re-run.

| field | value |
|---|---|
| run | 35474631618, `pull_request`, one attempt |
| head | `09798eb`, PR #284 |
| window | created 2026-09-19T22:54:20Z, updated 23:15:48Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35474631618-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Format | 105981637327 | success |
| Commit attribution (D9) | 105981637534 | success |
| Changed paths | 105981637550 | success |
| Lint (lua54) | 105981637564 | success |
| Lint (luajit) | 105981637592 | success |
| M4 Perf Gates | 105981654303 | success |
| GPU Render (headless) | 105981654310 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105981654350 | success |
| Test (macos-latest / luajit) | 105981654353 | success |
| M6 Perf Gates | 105981654358 | success |
| Test (ubuntu-latest / luajit) | 105981654361 | failure |
| M5 Perf Gates | 105981654379 | success |
| Test (ubuntu-latest / lua54) | 105981654386 | success |
| Perf budgets (debug) | 105981654410 | success |
| M10 Perf Gates (crdt) | 105981654415 | success |
| M1 Acceptance Gates | 105981654420 | success |
| Test (macos-latest / lua54) | 105981654430 | failure |
| Test (crdt) | 105981654438 | success |
| Docs consistency | 105981654766 | skipped |

Tally (run-35474631618-failures): 2 rows of the table above with `result` = `failure`.

Every test leg's log read: `Test (crdt)` 165 `test result: ok`,
`Test (ubuntu-latest / lua54)` 166, `Test (ubuntu-latest / luajit, no
crdt)` 166, `Test (macos-latest / luajit)` 167, each with zero
`FAILED`; `Test (ubuntu-latest / luajit)` 163 `ok` and one `FAILED`;
`Test (macos-latest / lua54)` 164 `ok` and one `FAILED`; every log's
`running N tests` lines paired one to one with its result lines.
`WouldBlock`, `did not become ready` and `got ok` appear zero times on
every leg; U17's witness `ok` on all six; the phase's twenty `e7c_`
rows (the fourteen of C7c, the round's four acceptance rows, the two
wire unit rows) `ok` on every leg, the round's switch-buffer row too.
So neither the `read Hello` family, nor #259, nor U4, nor U17, nor
#271, nor #276, nor #253, nor #282 sampled here. The two reds are each
the job's only failure:

- **#283's third occurrence**, `Test (macos-latest / lua54)`
  105981654430: `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`
  (`tests/e7_review1_probes.rs`, C7 review 1's probe) --- `pump timeout
  waiting for the accept and its import`, `text="fn main() {\n
  println\n    \n}\n// tail\n" popup_rows=0 anchor=None`, `test
  result: FAILED. 9 passed; 1 failed; 0 ignored`, `finished in 12.32s`
  --- all three fragments, on the leg of its first; commented on the
  issue. Off the branch's path (`src/daemon.rs` untouched; nothing on
  the accept arm).
- **#285, first occurrence, the branch's own row**, `Test
  (ubuntu-latest / luajit)` 105981654361:
  `a_save_sends_did_save_and_rust_analyzer_flychecks_on_it`
  (`tests/e7b_review_wire_acceptance.rs`, E7c.1's row) --- `one didSave
  for one save:` with `("textDocument/didSave", 2)` in the histogram,
  `left: 2` / `right: 1`, `test result: FAILED. 3 passed; 1 failed; 2
  ignored`, `finished in 39.88s`. The wire it prints: the `didSave` at
  3 ms, the server's first frame of any kind at 1198 ms, the retry's
  resend at 1509 ms with no begin yet seen, a `rustAnalyzer/Fetching`
  reload at 1978 ms, the first flycheck's begin at 2052 ms and the
  resent save's at 2246 ms. The check was not lost: the server was
  slow to begin it, E7c.1's 1.5 s timer outran the server, and the
  resend cost a second `didSave` and a second check, which the row
  counts as a failure. Filed rather than re-run, with the mechanism and
  the ruling it bears on (the resend's trigger, the owner's since fix
  round 1: the event in place of the timer would have sent nothing
  here, no `didChange` having followed the save). Not fixed in the
  round, whose brief left the trigger to the owner; the row's claim is
  E7c.1's to change under that ruling.

The head is not green, on the branch's own row and by the retry's
timer, stated at the moment of writing.

### PR #284's fix-round-3 head run 35504020999 at `0a287d6`: two witness rows of the round's own, and #283's fourth

E7c's head after fix round 3's two ruling commits (the save retry's
trigger moved from the 1.5 s timer to the `didChange` that goes out
before the check begins, closing #285; the activity indicator's 300 ms
threshold, method-only label, fixed width and leftmost slot, the busy
suffix's fixed slot). Read at the round's close on 2026-09-20 from the
jobs endpoint and all six test legs' logs after the run had completed;
not re-run --- the two rows of the round's own were fixed and pushed
as a new head, whose run has its own section below.

| field | value |
|---|---|
| run | 35504020999, `pull_request`, one attempt |
| head | `0a287d6`, PR #284 |
| window | created 2026-09-20T10:04:08Z, updated 10:29:32Z |
| verdict | 19 jobs: **15 success, 1 skipped, 3 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35504020999-jobs): 19 = 15 + 1 + 3.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 106060648312 | success |
| Format | 106060648374 | success |
| Changed paths | 106060648393 | success |
| Lint (lua54) | 106060648403 | success |
| Lint (luajit) | 106060648479 | success |
| Perf budgets (debug) | 106060744295 | success |
| M10 Perf Gates (crdt) | 106060744299 | success |
| M1 Acceptance Gates | 106060744303 | success |
| GPU Render (headless) | 106060744311 | success |
| M6 Perf Gates | 106060744316 | success |
| M4 Perf Gates | 106060744319 | success |
| Test (ubuntu-latest / luajit) | 106060744323 | success |
| Test (macos-latest / lua54) | 106060744338 | failure |
| M5 Perf Gates | 106060744343 | success |
| Test (ubuntu-latest / lua54) | 106060744346 | success |
| Test (ubuntu-latest / luajit, no crdt) | 106060744347 | failure |
| Test (crdt) | 106060744382 | success |
| Test (macos-latest / luajit) | 106060744424 | failure |
| Docs consistency | 106060744865 | skipped |

Tally (run-35504020999-failures): 3 rows of the table above with `result` = `failure`.

Every test leg's log read: `Test (ubuntu-latest / luajit)` 167 `test
result: ok`, `Test (ubuntu-latest / lua54)` 167, `Test (crdt)` 166,
each with zero `FAILED`; `Test (macos-latest / lua54)` 165 `ok` and
one `FAILED`; `Test (ubuntu-latest / luajit, no crdt)` 164 and one;
`Test (macos-latest / luajit)` 165 and one; every log's `running N
tests` lines paired one to one with its result lines. `WouldBlock`,
`did not become ready` and `got ok` appear zero times on every leg;
U17's witness `ok` on all six; the two wire rows against
rust-analyzer (`a_save_sends_did_save_and_rust_analyzer_flychecks_on_it`,
#285's, and the round's `a_keystroke_after_the_save_resends_it_behind_the_did_change`)
`ok` on all six. The three reds:

- `Test (macos-latest / lua54)`: **#283's fourth occurrence** ---
  `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`,
  `pump timeout waiting for the accept and its import`, `popup_rows=0
  anchor=None`, `9 passed; 1 failed`, all three fragments, the leg of
  its first and third; off the branch's path; commented on the issue.
- `Test (ubuntu-latest / luajit, no crdt)`: **the round's own**,
  `e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`
  --- "and in the wire's right group at 300 ms: [`LSP:ready         `]":
  the row read the wire's `StatuslineSegments` a few milliseconds before
  the evaluator at the threshold's edge, and asserted both carried the
  indicator on the same frame; a witness race on the `--test-threads=1`
  leg, not a product defect (the indicator was on the evaluator, the
  grid and the wire on the following frames). Fixed on the branch as
  `9eb89c1`: the row asserts each surface carried the method at the
  width on some shown frame.
- `Test (macos-latest / luajit)`: **the round's own**,
  `a12_builtin_lsp_provider_tracks_real_attachment_and_unknown_label`
  (`statusline_segments_acceptance`) --- `left: "⋯1 parse rust
  "`, `right: "LSP:?"`: the row read `right[0]` as the LSP segment,
  and since `0a287d6` the activity indicator is the leftmost right
  segment whenever a job has been in flight past its threshold, which a
  `parse rust` job was on that runner; a witness assumption about the
  right group's order, not a product defect. Fixed on the branch as
  `9eb89c1`: the row finds the LSP segment by face.

So neither the `read Hello` family, nor #259, nor U4, nor U17, nor
#271, nor #276, nor #253 sampled; #283 moves to four. The base control
is `361bb3b`, the last code-bearing commit on `main` (run 35437135435,
19 = 16 + 1 + 2 on #282 and #283, recorded as `cbac12b`). The head is
not green, on two witness rows of its own and by #283, stated at the
moment of writing; the next head's run is the one below.

### PR #284's fix-round-3 second head run 35505799659 at `9eb89c1`: two more witness rows of the round's own, and #283's fifth

E7c's head after the first witness fix (`9eb89c1`, the LSP segment
read by face and the indicator's wire read on any shown frame). Read
at the round's close on 2026-09-20 from the jobs endpoint and all six
test legs' logs after the run had completed; not re-run --- the two
rows of the round's own were fixed and pushed as a new head, whose run
has its own section below.

| field | value |
|---|---|
| run | 35505799659, `pull_request`, one attempt |
| head | `9eb89c1`, PR #284 |
| window | created 2026-09-20T10:42:05Z, updated 11:02:15Z |
| verdict | 19 jobs: **15 success, 1 skipped, 3 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35505799659-jobs): 19 = 15 + 1 + 3.

| job | id | result |
|---|---|---|
| Changed paths | 106065289108 | success |
| Format | 106065289176 | success |
| Commit attribution (D9) | 106065289213 | success |
| Lint (lua54) | 106065289214 | success |
| Lint (luajit) | 106065289240 | success |
| M10 Perf Gates (crdt) | 106065308683 | success |
| M4 Perf Gates | 106065308706 | success |
| GPU Render (headless) | 106065308725 | success |
| M5 Perf Gates | 106065308728 | success |
| Perf budgets (debug) | 106065308732 | success |
| M6 Perf Gates | 106065308735 | success |
| M1 Acceptance Gates | 106065308755 | success |
| Test (macos-latest / luajit) | 106065308768 | failure |
| Test (ubuntu-latest / luajit) | 106065308774 | success |
| Test (crdt) | 106065308776 | success |
| Test (ubuntu-latest / lua54) | 106065308785 | failure |
| Test (macos-latest / lua54) | 106065308799 | success |
| Test (ubuntu-latest / luajit, no crdt) | 106065308856 | failure |
| Docs consistency | 106065309529 | skipped |

Tally (run-35505799659-failures): 3 rows of the table above with `result` = `failure`.

Every test leg's log read: `Test (ubuntu-latest / luajit)` 167 `test
result: ok`, `Test (macos-latest / lua54)` 168, `Test (crdt)` 166,
each with zero `FAILED`; `Test (macos-latest / luajit)` 165 `ok` and
one `FAILED`; `Test (ubuntu-latest / lua54)` 164 and one; `Test
(ubuntu-latest / luajit, no crdt)` 164 and one; every log's `running
N tests` lines paired one to one with its result lines. `WouldBlock`,
`did not become ready` and `got ok` appear zero times on every leg;
U17's witness `ok` on all six; the resend row
`a_keystroke_after_the_save_resends_it_behind_the_did_change` `ok` on
all six, and the previous run's two rows (`a12_…`, the slow-request
row's wire read) `ok` on every leg they ran. The three reds:

- `Test (macos-latest / luajit)`: **#283's fifth occurrence**, the leg
  of its second ---
  `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`,
  `pump timeout waiting for the accept and its import`, `popup_rows=0
  anchor=None`, `9 passed; 1 failed`, all three fragments; off the
  branch's path; commented on the issue. Two occurrences in two
  consecutive runs of the branch, forty minutes apart, one per macOS
  leg.
- `Test (ubuntu-latest / lua54)`: **the round's own, and E7c.1's row**
  `a_save_sends_did_save_and_rust_analyzer_flychecks_on_it` (#285's) ---
  "the check showed as a suffix on ready: [`5 = LSP:idx`, `1104 =
  LSP:ready`, `3054 = LSP:idx`, `4562 = LSP:ready`]": the server's
  cache priming restarted with the typed edit and the label read `idx`
  from the save to past the check, which masks the busy suffix by
  E7b.1's own rule (the kind wins); the `didSave` count --- the row's
  claim and #285's --- held at one, the assertion that failed being
  the later one on the label. A witness assumption that the check runs
  under `ready`, not a product defect. Fixed on the branch as
  `3de1e1f`: the watch logs the tracker's busy title beside the label
  and the row accepts the check's title there when the label hid it.
- `Test (ubuntu-latest / luajit, no crdt)`: **the round's own**,
  `e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`
  --- "it appeared after the threshold and before the answer: 299 ms":
  a frame's stamp precedes its three reads by a few milliseconds on the
  `--test-threads=1` leg, so a frame whose evaluator read was past 300
  ms carried a 299 ms stamp; not a product defect. Fixed on the branch
  as `3de1e1f`: the edge is judged with the 250 ms slack the row's
  early check already uses.

So neither the `read Hello` family, nor #259, nor U4, nor U17, nor
#271, nor #276, nor #253 sampled; #283 moves to five. The base control
is `361bb3b` (run 35437135435, recorded as `cbac12b`). The head is
not green, on two witness rows of its own and by #283, stated at the
moment of writing; the next head's run is the one below.

### PR #284's fix-round-3 tip run 35507517448 at `3de1e1f`: #283's sixth and #282's second, neither the branch's

E7c's head at fix round 3's close (`3de1e1f`, the second witness fix:
the one-save row reading the check on the tracker under a reload, the
indicator row's edge with slack). Read at the round's close on
2026-09-20 from the jobs endpoint and all six test legs' logs after
the run had completed; not re-run.

| field | value |
|---|---|
| run | 35507517448, `pull_request`, one attempt |
| head | `3de1e1f`, PR #284 |
| window | created 2026-09-20T11:19:45Z, updated 11:39:26Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35507517448-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Format | 106069729614 | success |
| Changed paths | 106069729687 | success |
| Lint (luajit) | 106069729696 | success |
| Lint (lua54) | 106069729707 | success |
| Commit attribution (D9) | 106069729713 | success |
| M1 Acceptance Gates | 106069747046 | success |
| Test (crdt) | 106069747051 | success |
| GPU Render (headless) | 106069747074 | success |
| M4 Perf Gates | 106069747076 | success |
| Test (macos-latest / luajit) | 106069747083 | failure |
| M5 Perf Gates | 106069747091 | success |
| Perf budgets (debug) | 106069747093 | success |
| M10 Perf Gates (crdt) | 106069747108 | success |
| Test (ubuntu-latest / luajit) | 106069747112 | success |
| M6 Perf Gates | 106069747131 | success |
| Test (ubuntu-latest / luajit, no crdt) | 106069747153 | failure |
| Test (ubuntu-latest / lua54) | 106069747174 | success |
| Test (macos-latest / lua54) | 106069747183 | success |
| Docs consistency | 106069747561 | skipped |

Tally (run-35507517448-failures): 2 rows of the table above with `result` = `failure`.

Every test leg's log read: `Test (crdt)` 166 `test result: ok`,
`Test (ubuntu-latest / luajit)` 167, `Test (ubuntu-latest / lua54)`
167, `Test (macos-latest / lua54)` 168, each with zero `FAILED`;
`Test (macos-latest / luajit)` 165 `ok` and one `FAILED`; `Test
(ubuntu-latest / luajit, no crdt)` 164 and one; every log's `running
N tests` lines paired one to one with its result lines. `WouldBlock`,
`did not become ready` and `got ok` appear zero times on every leg;
U17's witness `ok` on all six; the round's twenty-five `e7c_` rows
(fix round 2's twenty, the five indicator rows), the two wire rows
against rust-analyzer (the one-save row, #285's, and the resend row)
and the four witness rows the two earlier heads corrected `ok` on
every leg. The two reds:

- `Test (macos-latest / luajit)`: **#283's sixth occurrence** ---
  `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`,
  `pump timeout waiting for the accept and its import`, `popup_rows=0
  anchor=None`, `9 passed; 1 failed`, all three fragments; off the
  branch's path; commented on the issue. Three occurrences in this
  branch's three consecutive runs of the day.
- `Test (ubuntu-latest / luajit, no crdt)`: **#282's second
  occurrence**, on the ubuntu luajit flavor's no-crdt leg ---
  `a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`,
  `*lsp* names the request and the code: []`, `WIRE *errors* gained
  ["[lsp] LSP: default-rust refused textDocument/prepareRename as a
  client error, -32602 InvalidParams: …"]`, `WIRE label "ready",
  last_error None`; the result line `4 passed; 1 failed; 2 ignored`
  where the first occurrence read `3 passed`, the suite having gained
  a row this round, so that count moves with the suite. The trace
  shows the filed mechanism: a `rustAnalyzer/cachePriming` cycle ran
  through the rename and its reports pushed the error line out of
  `*lsp*`'s 64-entry ring. Not the branch's: the round touched this
  suite's label helper and its watch's busy logging, neither writing
  to `*lsp*`; commented on the issue.

So neither the `read Hello` family, nor #259, nor U4, nor U17, nor
#271, nor #276, nor #253 sampled; #283 moves to six and #282 to two.
The base control is `361bb3b` (run 35437135435, 19 = 16 + 1 + 2 on
#282's first and #283's first, recorded as `cbac12b`): the two rows
red on the base control are the two rows red here. The head is not
green, on two registered rows off its path, stated at the moment of
writing; the disposition is the owner's, in D33's form.

### PR #284's fix-round-1 head run 35464890800 at `28413d0`, and it is GREEN

Read at C7c fix round 1's close on 2026-09-19, from the jobs endpoint
and all six test legs' logs after the run had completed; not re-run.
The head is the round's tip (`e7c/cargo-check-diagnostics`, base
`cbac12b`): C7c's eight commits plus one that changes three comments
in `builtin/runtime/lsp.lua` and no line of code.

| field | value |
|---|---|
| run | 35464890800, `pull_request`, one attempt |
| head | `28413d0`, PR #284 |
| window | created 2026-09-19T19:36:58Z, updated 19:58:04Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the six test legs | `Test (crdt)` 165 `test result: ok` (the 164 targets of the previous head plus the job's `cargo test --doc` step, which runs only after a green `--all-targets` step and so was absent from that head's 163 + 1), `Test (ubuntu-latest / lua54)` 166, `Test (ubuntu-latest / luajit)` 166, the no-crdt leg 166, `Test (macos-latest / lua54)` 167, `Test (macos-latest / luajit)` 167; every log's `running N tests` lines paired one to one with its result lines; `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; U17's witness `ok` on every leg; the phase's fourteen `e7c_` rows `ok` on every leg; the previous head's two reds (#253's sixth on `Test (crdt)`, #283's second on macOS luajit) did not recur, which is non-reproduction and nothing more |

Tally (run-35464890800-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Lint (lua54) | 105955311279 | success |
| Format | 105955311282 | success |
| Lint (luajit) | 105955311323 | success |
| Changed paths | 105955311346 | success |
| Commit attribution (D9) | 105955311348 | success |
| M4 Perf Gates | 105955335861 | success |
| M6 Perf Gates | 105955335864 | success |
| M5 Perf Gates | 105955335869 | success |
| M10 Perf Gates (crdt) | 105955335877 | success |
| Test (crdt) | 105955335890 | success |
| M1 Acceptance Gates | 105955335902 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105955335914 | success |
| Test (ubuntu-latest / luajit) | 105955335925 | success |
| Test (macos-latest / lua54) | 105955335930 | success |
| GPU Render (headless) | 105955335935 | success |
| Perf budgets (debug) | 105955335937 | success |
| Test (macos-latest / luajit) | 105955335953 | success |
| Test (ubuntu-latest / lua54) | 105955335963 | success |
| Docs consistency | 105955336760 | skipped |

### PR #284's head run 35455311600 at `1847805`: #253's sixth and #283's second, neither the branch's

E7c's head after the control row's fix. Read at C7c's close on
2026-09-19 from the jobs endpoint and all six test legs' logs after
the run completed; not re-run.

| field | value |
|---|---|
| run | 35455311600, `pull_request`, one attempt |
| head | `1847805`, `e7c/cargo-check-diagnostics` (base `cbac12b`) |
| window | created 2026-09-19T16:32:56Z, updated 16:54:24Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35455311600-jobs): 19 = 16 + 1 + 2.

Tally (run-35455311600-test-legs): 6 rows in the table below.

| job | id | result |
|---|---|---|
| Test (crdt) | 105929452599 | failure |
| Test (ubuntu-latest / lua54) | 105929452692 | success |
| Test (ubuntu-latest / luajit) | 105929452733 | success |
| Test (macos-latest / luajit) | 105929452757 | failure |
| Test (macos-latest / lua54) | 105929452772 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105929452786 | success |

Tally (run-35455311600-test-failures): 2 rows of the table above with `result` = `failure`.

The thirteen other jobs --- the five lint, format, attribution and
changed-paths jobs, the six perf and acceptance gates, the headless
GPU render, and `Docs consistency` skipped --- are the sixteen
successes and the skip with the four green legs above. Every test
leg's log read: `Test (ubuntu-latest / lua54)` 166 `test result: ok`,
`Test (ubuntu-latest / luajit)` 166, the no-crdt leg 166, `Test
(macos-latest / lua54)` 167, each with zero `FAILED`; `Test (crdt)`
163 `ok` and one `FAILED`; `Test (macos-latest / luajit)` 164 and one.
`did not become ready` and `got ok` zero on every leg; `WouldBlock`
once, on `Test (crdt)`, the red read itself; U17's selector `ok` on
all six; the phase's fourteen `e7c_` rows `ok` on every leg, the
control row fixed at `1847805` among them. The two reds are each the
job's only failure and neither is on this branch's path:

- **#253's sixth occurrence**, `Test (crdt)`:
  `undo_across_peers_acceptance`
  `a_command_after_optimistic_typing_is_its_own_undo_step`, `read
  initial BufferSnapshot: Io(Os { code: 11, kind: WouldBlock, message:
  "Resource temporarily unavailable" })` at
  `tests/undo_across_peers_acceptance.rs:34`, `18 passed; 1 failed` in
  15.17 s at 16:50:23Z --- the row's required fragment on a fourth
  suite's own `read_initial_snapshot` helper, the same shape as the
  two folded from #277, on the serialized leg; the row's sign (a slow,
  bounded read). Counted on the row above.
- **#283's second occurrence**, `Test (macos-latest / luajit)`:
  `e7_review1_probes`
  `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`,
  `pump timeout waiting for the accept and its import`, `popup_rows=0
  anchor=None`, `9 passed; 1 failed` in 13.36 s at 16:44:25Z --- all
  three fragments, the same text to the byte as the first occurrence
  (lua54, `main` at `361bb3b`); the fixed settle window now seen on
  both macOS flavors in two consecutive runs of two trees.

So the head is **not green**, on two rows that predate the branch and
have issues, and no row of the branch's own is red; the E7c.3 control
row that reddened the first head ran `ok` on all six legs. Both
occurrences are commented on their issues; no rerun was taken.

### PR #284's head run 35453990704 at `fcd1248`: red on one of the branch's own rows, closed on the branch

E7c's checkpoint push. Read at C7c's close on 2026-09-19 from the jobs
endpoint and all six test legs' logs after the run completed; not
re-run --- the row was fixed and a new head pushed.

| field | value |
|---|---|
| run | 35453990704, `pull_request`, one attempt |
| head | `fcd1248`, `e7c/cargo-check-diagnostics` (base `cbac12b`) |
| window | created 2026-09-19T16:08:20Z, updated 16:30:46Z |
| verdict | 19 jobs: **17 success, 1 skipped, 1 failure** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35453990704-jobs): 19 = 17 + 1 + 1.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 105925941944 | success |
| Lint (lua54) | 105925942072 | success |
| Format | 105925942078 | success |
| Lint (luajit) | 105925942109 | success |
| Changed paths | 105925942155 | success |
| M4 Perf Gates | 105925962905 | success |
| M5 Perf Gates | 105925962907 | success |
| GPU Render (headless) | 105925962912 | success |
| Test (ubuntu-latest / luajit) | 105925962929 | success |
| Perf budgets (debug) | 105925962950 | success |
| M1 Acceptance Gates | 105925962960 | success |
| M6 Perf Gates | 105925962961 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105925962972 | failure |
| Test (macos-latest / luajit) | 105925962974 | success |
| M10 Perf Gates (crdt) | 105925962982 | success |
| Test (crdt) | 105925962987 | success |
| Test (ubuntu-latest / lua54) | 105925962994 | success |
| Test (macos-latest / lua54) | 105925963004 | success |
| Docs consistency | 105925963512 | skipped |

Tally (run-35453990704-failures): 1 row of the table above with `result` = `failure`.

Every test leg's log read: `Test (crdt)` 165 `test result: ok`,
`Test (ubuntu-latest / luajit)` 166, `Test (ubuntu-latest / lua54)`
166, both macOS legs 167, each with zero `FAILED`; `Test
(ubuntu-latest / luajit, no crdt)` 163 `ok` and one `FAILED`.
`WouldBlock`, `did not become ready` and `got ok` appear zero times on
every leg; U17's selector `ok` on all six; the phase's fourteen `e7c_`
rows `ok` on five legs and thirteen of them on the sixth. So no
registered row and neither #282 nor #283 sampled here, and every count
stands. The one red is the branch's own:
`e7c_3_without_check_sources_the_republished_check_error_lands_a_line_too_high`
(`tests/e7c_positions_acceptance.rs`, E7c.3's control row, the one
that shows what `check_sources` buys by leaving it empty), `the
check's set is taken for the typed text and misses: ["rustc error
2:12-2:19", "pmacs-fake-lsp warning 2:12-2:19"]`, `test result:
FAILED. 3 passed; 1 failed`, at 16:18:02Z. The row waited for the
store's next epoch after the typed line and read that republish as
the final one; the completion driver flushes a `didChange` mid-word as
the line is typed, the fake answers it with the check's set at a base
the carry still reaches, and that intermediate republish put the
error on its text. Closed on the branch as `1847805`: the row waits
for the state it demonstrates, the check's error one line too high
once the last `didChange` is answered. A fixture race in a row this
phase wrote, not a product defect; the same row was `ok` on the other
five legs and in five local runs before and after the fix.

### `main` after E7b: run 35437135435 at `361bb3b`, red on two rows of its own suites, filed as #282 and #283

E7b merged as `361bb3b` (squash of `c0230bf`, PR #280,
`--match-head-commit`) on 2026-09-19, and its post-merge `push` run
is **35437135435**: 19 jobs on one attempt, **16 green, 1 skipped, 2
red**. Read at E7c's opening on 2026-09-19 from the jobs endpoint and
all six test legs' logs after the run completed; not re-run. The head
of PR #280, the same tree but for `main`'s registry commits, was green
on every leg (35432322590, the section below), so both reds are
first samples of their fragments on a tree whose code CI had already
run green once.

| field | value |
|---|---|
| run | 35437135435, `push`, one attempt |
| head | `361bb3b`, E7b's squash merge (PR #280 at `c0230bf`, `--match-head-commit`) |
| window | created 2026-09-19T10:20:32Z, updated 10:42:03Z |
| verdict | 19 jobs: **16 success, 1 skipped, 2 failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |

Tally (run-35437135435-jobs): 19 = 16 + 1 + 2.

| job | id | result |
|---|---|---|
| Format | 105881606071 | success |
| Changed paths | 105881606110 | success |
| Lint (lua54) | 105881606121 | success |
| Commit attribution (D9) | 105881606122 | success |
| Lint (luajit) | 105881606143 | success |
| GPU Render (headless) | 105881622691 | success |
| M10 Perf Gates (crdt) | 105881622693 | success |
| M4 Perf Gates | 105881622695 | success |
| M5 Perf Gates | 105881622706 | success |
| Perf budgets (debug) | 105881622722 | success |
| Test (macos-latest / luajit) | 105881622730 | success |
| Test (ubuntu-latest / luajit) | 105881622732 | failure |
| M1 Acceptance Gates | 105881622741 | success |
| Test (macos-latest / lua54) | 105881622751 | failure |
| Test (crdt) | 105881622760 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105881622767 | success |
| Test (ubuntu-latest / lua54) | 105881622825 | success |
| M6 Perf Gates | 105881622849 | success |
| Docs consistency | 105881623490 | skipped |

Tally (run-35437135435-failures): 2 rows of the table above with `result` = `failure`.

Every test leg's log read: `Test (crdt)` 162 `test result: ok`,
`Test (ubuntu-latest / lua54)` 163, `Test (ubuntu-latest / luajit, no
crdt)` 163, `Test (macos-latest / luajit)` 164, each with zero
`FAILED`; `Test (ubuntu-latest / luajit)` 160 `ok` and one `FAILED`;
`Test (macos-latest / lua54)` 161 `ok` and one `FAILED`. `WouldBlock`,
`did not become ready` and `got ok` appear zero times on every leg, and
U17's selector `read_dir_supersede_cancels_in_flight_predecessor` ran
`ok` on all six. So neither the `read Hello` family, nor #259, nor U4,
nor U17, nor #271, nor #276 sampled here, and every count stands where
the PR #280 sections left it. The two reds are each the job's only
failure, each a first sample of its fragments, each on a row a review
round wrote, and each filed as its own issue under the registry's
template rather than re-run:

- **#282**, `Test (ubuntu-latest / luajit)` 105881622732:
  `a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`
  (`tests/e7b_review_wire_acceptance.rs`, C7b review 1's row, retried on
  `-32801` at fix round 1) failed at its `*lsp*` read --- `*lsp* names
  the request and the code: []` --- with the label `ready`, `last_error`
  `None` and `*errors*` holding its one line, `test result: FAILED. 3
  passed; 1 failed; 2 ignored`. The candidate mechanism is in the
  row's own trace: `*lsp*` is a 64-entry ring and rust-analyzer
  re-primed its cache inside the three-second window after the rename
  (`LSP:idx` twice, 104 `$/progress` frames, 84 of them `cachePriming`
  reports with a message, each a push), so the `response error:
  textDocument/prepareRename` line was evicted before the read. The
  product's surfaces under the fix-round ruling (the label and
  `*errors*`) were correct; the row reads a capped log after a window
  the server can overflow. Not a product defect that this run shows.
- **#283**, `Test (macos-latest / lua54)` 105881622751:
  `gpu_route::e7_review1_gpu_route_accept_after_a_letter_typed_since_the_request_carries_the_import`
  (`tests/e7_review1_probes.rs`, C7 review 1's probe) --- `pump timeout
  waiting for the accept and its import`, the replica's text
  `"fn main() {\n    println\n    \n}\n// tail\n"` with `popup_rows=0
  anchor=None`, `test result: FAILED. 9 passed; 1 failed; 0 ignored`,
  `finished in 12.43s`. RET reached the daemon as a newline with its
  auto-indent, which is what RET does with no popup open, so the
  fake's completion answer had not opened the popup when the probe's
  fixed 1500 ms settle window closed and RET went out; the row ran
  `ok` on the five other legs. A fixed window on the leg the registry
  measures as slowest, not a wait on the popup; not a product defect
  that this run shows (E7.4's accept arm passed on every other leg).

The base control for E7c is `361bb3b`, the last code-bearing commit on
`main`; its own run is this one, red on two fixture rows neither of
which is on E7c's path, and PR #280's head run at `c0230bf` on the same
code is green on every leg.

### PR #280's fix-round-1 head run 35432322590 at `c0230bf`, and it is GREEN

Read at C7b fix round 1's close on 2026-09-19, from the jobs endpoint
and all six test legs' logs after the run had completed; not re-run.
The head is the round's tip (`e7b/status-prompt-keys`, base
`e190f80`): the review's `c4c0dfb` plus seven commits, the last two
closing the two reds of the previous section.

| field | value |
|---|---|
| run | 35432322590, `pull_request`, one attempt |
| head | `c0230bf`, PR #280 |
| window | created 2026-09-19T08:33:37Z, updated 08:56:24Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the six test legs | `Test (crdt)` 162 `test result: ok`, the three ubuntu legs 163 each, both macOS legs 164 each (the reviewed head's 161 / 162 / 163 plus the round's one suite); `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; U17's witness `ok` on every leg; the round's eleven rows `ok` on every leg, the rename row running on the four Linux legs under `PMACS_REQUIRE_LSP=1` and skipping on macOS; the two reds of `ab8e488` did not recur, and the `clientfault` row on both lua54 legs is the reading of `6bcae0c` |

Tally (run-35432322590-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 105869005401 | success |
| Lint (lua54) | 105869005594 | success |
| Lint (luajit) | 105869005617 | success |
| Commit attribution (D9) | 105869005618 | success |
| Format | 105869005727 | success |
| Test (crdt) | 105869028699 | success |
| M1 Acceptance Gates | 105869028781 | success |
| M10 Perf Gates (crdt) | 105869028782 | success |
| GPU Render (headless) | 105869028792 | success |
| M5 Perf Gates | 105869028796 | success |
| Test (macos-latest / lua54) | 105869028809 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105869028824 | success |
| Perf budgets (debug) | 105869028828 | success |
| M4 Perf Gates | 105869028836 | success |
| Test (macos-latest / luajit) | 105869028841 | success |
| Test (ubuntu-latest / luajit) | 105869028855 | success |
| Test (ubuntu-latest / lua54) | 105869028862 | success |
| M6 Perf Gates | 105869028903 | success |
| Docs consistency | 105869029526 | skipped |

### PR #280's fix-round-1 run 35399864211 at `ab8e488`: red on two of the round's own rows, both closed causally on the branch

Read at C7b fix round 1 on 2026-09-19, from the jobs endpoint and all
six test legs' logs after the run had completed; not re-run. The head
is the round's first push (`c4c0dfb` plus five fix commits); the next
push, `c0230bf`, carries the two fixes and is recorded in its own
section.

| field | value |
|---|---|
| run | 35399864211, `pull_request`, one attempt |
| head | `ab8e488`, PR #280 |
| window | created 2026-09-18T22:04:26Z, updated 22:28:41Z |
| verdict | 19 jobs: **15 success, 1 skipped, 3 failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the six test legs | `Test (crdt)` 162 `test result: ok`; `Test (ubuntu-latest / luajit, no crdt)` 163; `Test (macos-latest / luajit)` 164 --- green. `Test (ubuntu-latest / lua54)` and `Test (ubuntu-latest / luajit)` 160 `ok` and 1 `FAILED` each; `Test (macos-latest / lua54)` 161 `ok` and 1 `FAILED`. `WouldBlock`, `did not become ready` and `got ok` zero on every leg; U17's witness `ok` on every leg |

Tally (run-35399864211-jobs): 19 = 15 + 1 + 3.

**Red 1, the two ubuntu legs** (`e7b_review_wire_acceptance`,
`a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`,
`tests/e7b_review_wire_acceptance.rs:921`, "rust-analyzer answered it
with -32602 InvalidParams"): the row's positive control. CI's
rust-analyzer answered the first `textDocument/prepareRename` after
warm-up with `-32801 content modified` (the status line read `LSP: LSP
textDocument/prepareRename error -32801: content modified`), which is
its dispatcher's not-ready arm for a request landing while its VFS is
mid-load; under the round's ruling a retry code is a moot request and
silent, so the label stayed `ready` and `*errors*` gained nothing ---
the verdict the row asserts --- and the control that the server had
answered on the merits failed. The other two Linux legs answered
`-32602` first time. Not a product defect: the row's control asked
for one answer and the server gave a different, correct one. Closed
by `c0230bf` on the branch: the row asks up to six times, a second
apart, until the answer is not a retry code, and names the last status
when it never is.

**Red 2, the macOS lua54 leg** (`e6d_lsp_status_under_typing_acceptance`,
`a_hundred_keystrokes_answered_invalid_params_leave_the_label_ready_and_fill_errors`,
`tests/e6d_lsp_status_under_typing_acceptance.rs:245`, "every line
names the code and the method"): among the 682 `*errors*` lines the
row's flood produced, one was not the fake's --- `[@pmacs/runtime/async.lua:tick]
runtime error: invalid key to 'next'` with a traceback through
`builtin/runtime/syntax.lua:622`, the parse-settle step's `pairs`
loop over `pending_parse_jobs`. **A product defect, latent since the
loop was written**: installing a settled job inside the loop ran
`dispatch_follow_up_if_dirty`, whose `pmacs.parse._dispatch` inserts a
new key into the table being traversed, which the Lua manual leaves
undefined and Lua 5.4 answers with this error once the removed key's
node is reused; LuaJIT tolerates it, so the default sweep (luajit)
never saw it, and the two lua54 legs had not either until a row
appended to `*errors*` hundreds of times while a buffer was being typed
into (every append fires `buffer.after-edit`, which re-requests the
active buffer's parse, so many settles dispatched a follow-up
mid-traversal). Five local runs of the row under `--features
lua54,crdt` against the old file did not reproduce it; the leg is the
witness. Closed causally by `6bcae0c` on the branch: the settled ids
are collected first and installed after the walk, so nothing is
inserted during a traversal.

Required fragments, should either recur: red 1 `error -32801: content
modified` with `prepareRename` in the same status line; red 2 `invalid
key to 'next'` with `syntax.lua` in the traceback.

| job | id | result |
|---|---|---|
| Changed paths | 105777052171 | success |
| Format | 105777052317 | success |
| Lint (luajit) | 105777052354 | success |
| Lint (lua54) | 105777052377 | success |
| Commit attribution (D9) | 105777052458 | success |
| M4 Perf Gates | 105777087578 | success |
| M1 Acceptance Gates | 105777087580 | success |
| GPU Render (headless) | 105777087644 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105777087671 | success |
| Perf budgets (debug) | 105777087680 | success |
| Test (macos-latest / luajit) | 105777087698 | success |
| M6 Perf Gates | 105777087700 | success |
| Test (crdt) | 105777087717 | success |
| M10 Perf Gates (crdt) | 105777087728 | success |
| M5 Perf Gates | 105777087737 | success |
| Test (ubuntu-latest / lua54) | 105777087794 | failure |
| Test (ubuntu-latest / luajit) | 105777087802 | failure |
| Test (macos-latest / lua54) | 105777087825 | failure |
| Docs consistency | 105777089231 | skipped |

### PR #280's head run 35390705543 at `c4c0dfb`, and it is GREEN

Read at E7b review 1's close on 2026-09-18, from the jobs endpoint and
all six test legs' logs; not re-run. The head is the review's witness
commit on E7b (`e7b/status-prompt-keys`, base `e190f80`): three test
suites on top of the reviewed `a78e480`, no code.

| field | value |
|---|---|
| run | 35390705543, `pull_request`, one attempt |
| head | `c4c0dfb`, PR #280 |
| window | created 2026-09-18T20:18:14Z, updated 20:40:37Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed tests |
| the six test legs | `Test (crdt)` 161 `test result: ok`, the three ubuntu legs 162 each, both macOS legs 163 each (the reviewed head's 158 / 159 / 160 plus the three suites); `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; U17's witness `ok` on every leg; the review's ten rows `ok` on every leg, the four rust-analyzer rows running on the four Linux legs under `PMACS_REQUIRE_LSP=1` and skipping on macOS, where the server is not installed |

Tally (run-35390705543-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 105748135722 | success |
| Lint (luajit) | 105748135886 | success |
| Lint (lua54) | 105748136037 | success |
| Changed paths | 105748136080 | success |
| Format | 105748136167 | success |
| M1 Acceptance Gates | 105748184799 | success |
| GPU Render (headless) | 105748184801 | success |
| M10 Perf Gates (crdt) | 105748184837 | success |
| M6 Perf Gates | 105748184838 | success |
| Test (crdt) | 105748184857 | success |
| M4 Perf Gates | 105748184861 | success |
| Test (ubuntu-latest / luajit) | 105748184878 | success |
| Perf budgets (debug) | 105748184885 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105748184896 | success |
| M5 Perf Gates | 105748184903 | success |
| Test (macos-latest / lua54) | 105748184933 | success |
| Test (ubuntu-latest / lua54) | 105748184942 | success |
| Test (macos-latest / luajit) | 105748184949 | success |
| Docs consistency | 105748186851 | skipped |

### PR #280's head run 35381362669 at `a78e480`, and it is GREEN

Read at C7b's close on 2026-09-18, from the jobs endpoint and all six
test legs' logs; not re-run. The branch is E7b (`e7b/status-prompt-keys`,
base `e190f80`), four commits.

| field | value |
|---|---|
| run | 35381362669, `pull_request`, one attempt |
| head | `a78e480`, PR #280 |
| window | created 2026-09-18T18:38:30Z, updated 19:02:08Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |
| the six test legs | `Test (crdt)` 158 `test result: ok`, the three ubuntu legs 159 each, both macOS legs 160 each; `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; U17's witness `ok` on every leg; the phase's 22 `e7b_` rows `ok` on every leg, the GPU chord probe among them on both macOS legs (a wgpu adapter present, not skipped) |

Tally (run-35381362669-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 105718037333 | success |
| Commit attribution (D9) | 105718037550 | success |
| Lint (lua54) | 105718037580 | success |
| Format | 105718037602 | success |
| Lint (luajit) | 105718037620 | success |
| GPU Render (headless) | 105718105198 | success |
| Test (crdt) | 105718105204 | success |
| M6 Perf Gates | 105718105297 | success |
| M4 Perf Gates | 105718105308 | success |
| M10 Perf Gates (crdt) | 105718105313 | success |
| Perf budgets (debug) | 105718105324 | success |
| M5 Perf Gates | 105718105326 | success |
| Test (ubuntu-latest / lua54) | 105718105359 | success |
| Test (ubuntu-latest / luajit) | 105718105362 | success |
| Test (macos-latest / lua54) | 105718105369 | success |
| Test (macos-latest / luajit) | 105718105383 | success |
| M1 Acceptance Gates | 105718105385 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105718105552 | success |
| Docs consistency | 105718106590 | skipped |

### `main` after E7: run 35375379539 at `e190f80`, and it is GREEN

Read at E7b's opening on 2026-09-18, from the jobs endpoint and all
six test legs' logs; not re-run. `e190f80` is code-bearing (E7's
squash), so it is E7b's base control itself.

| field | value |
|---|---|
| run | 35375379539, `push`, one attempt |
| head | `e190f80`, E7's squash merge (PR #278 at `c68409d`, `--match-head-commit`) |
| window | created 2026-09-18T17:36:52Z, updated 17:57:18Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the merge changed code |
| the six test legs | `Test (crdt)` 154 `test result: ok`, the two ubuntu luajit legs and the lua54 leg 155 each, both macOS legs 156 each; `test result: FAILED`, `WouldBlock`, `did not become ready` and `got ok` zero on every leg; U17's witness `ok` on every leg |

Tally (run-35375379539-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 105698717570 | success |
| Lint (luajit) | 105698717984 | success |
| Format | 105698718038 | success |
| Lint (lua54) | 105698718177 | success |
| Commit attribution (D9) | 105698718213 | success |
| M6 Perf Gates | 105698764938 | success |
| M1 Acceptance Gates | 105698764945 | success |
| GPU Render (headless) | 105698764964 | success |
| M4 Perf Gates | 105698765046 | success |
| M5 Perf Gates | 105698765058 | success |
| Perf budgets (debug) | 105698765064 | success |
| Test (crdt) | 105698765107 | success |
| M10 Perf Gates (crdt) | 105698765147 | success |
| Test (ubuntu-latest / luajit) | 105698765196 | success |
| Test (macos-latest / luajit) | 105698765206 | success |
| Test (ubuntu-latest / lua54) | 105698765229 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105698765262 | success |
| Test (macos-latest / lua54) | 105698765374 | success |
| Docs consistency | 105698766180 | skipped |

### PR #278's fix-round-1 head run 35366530302 at `c68409d`, and it is GREEN

E7's fix-round-1 tip (`e7/git-and-lsp-affordances`, base `7002308`;
`024b548` plus Medium 1's line `2aa9486`, the bound's comment
`f081e50` and the `Test (crdt)` flag `c68409d`), read at the round's
close on 2026-09-18 from the jobs endpoint and all six test legs'
logs after the run had completed; not re-run.

| field | value |
|---|---|
| run | 35366530302, `pull_request`, one attempt |
| head | `c68409d`, PR #278's fix-round-1 head |
| window | created 2026-09-18T16:07:02Z, updated 16:28:28Z |
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| the skip | `Docs consistency`, correctly: the push changed code |

Tally (run-35366530302-jobs): 19 = 18 + 1 + 0.

| job | id | result |
|---|---|---|
| Changed paths | 105670232609 | success |
| Lint (luajit) | 105670232771 | success |
| Lint (lua54) | 105670232776 | success |
| Format | 105670232825 | success |
| Commit attribution (D9) | 105670232945 | success |
| M5 Perf Gates | 105670297624 | success |
| M1 Acceptance Gates | 105670297685 | success |
| GPU Render (headless) | 105670297719 | success |
| M4 Perf Gates | 105670297722 | success |
| M6 Perf Gates | 105670297785 | success |
| Test (crdt) | 105670297805 | success |
| M10 Perf Gates (crdt) | 105670297831 | success |
| Test (ubuntu-latest / luajit, no crdt) | 105670297845 | success |
| Test (macos-latest / lua54) | 105670297848 | success |
| Test (macos-latest / luajit) | 105670297937 | success |
| Perf budgets (debug) | 105670297984 | success |
| Test (ubuntu-latest / luajit) | 105670298030 | success |
| Test (ubuntu-latest / lua54) | 105670298175 | success |
| Docs consistency | 105670299161 | skipped |

On all six test legs `WouldBlock`, `got ok` and `did not become
ready` appear zero times and `FAILED` zero times: `Test (crdt)`
(105670297805) 154 `test result: ok`, the ubuntu matrix legs
(105670298175 lua54, 105670298030 luajit, 105670297845 luajit without
`crdt`) 155 each, both macOS legs (105670297937 luajit, 105670297848
lua54) 156 each. `e7_review1_probes` is `11 passed` on the three
`crdt` ubuntu legs and `10 passed` where one probe is compiled out
(the GPU-route probe without `crdt`; the killed-server probe off
Linux), the prune probe `ok` on every leg --- the review's Medium 1
closed on CI as well as locally. **`Test (crdt)` at 154 against 48
at the two runs above is the `--no-fail-fast` flag doing what it
says**: the job now runs its whole corpus, U17's witness `ok` on it
and on the other five legs. The three first-snapshot reads of #253
(#277 folded into it above) did not recur on `Test (ubuntu-latest /
lua54)`, a green sample and nothing more. So neither the `read Hello`
family, nor #259, nor U4, nor #271, nor #276, nor #253 sampled on
this run; every count stands where the sections above left it. **The
head is green**, stated at the moment of writing.

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
| occurrences | at least thirty-three, 2026-08-07 to 2026-10-09, all local, all under sweep load; the panic line moves with `attach.rs` and is not part of the signature. The first twelve are enumerated in this file's history before 2026-09-05; the twenty-one since are the list below this table, with the tallies (added at fix round 1, review 1's Low 3). The count is a floor: nobody has counted runs, so an occurrence is only ever recorded when someone reads the log |
| candidate mechanism | the test drives a scripted transient-then-success sequence over a real socket pair; unknown whether the broken pipe is the fixture's writer closing early or a retry-path defect. Unresolved |
| retirement | hardening that removes the named mechanism plus a discriminating witness, or a diagnosis showing the fixture, not the code, closes the pipe |

Tally (R7): 33 = 12 + 21.

Tally (R7-held): 21 items in the list below.

- thirteenth: gate log `20260905T202734Z-1751532`, step `07-sweep`, load average 14.2, `attach.rs:1889`, all three fragments
- fourteenth: gate log `20260905T205642Z-2051072`, step `05-sweep` of the six-stage gate, `attach.rs:1889`, all three fragments
- fifteenth: gate log `20260907T170429Z-45241`, step `06-sweep-luajit` (the LuaJIT-only sweep `--protocol` adds), `test result: FAILED. 325 passed; 1 failed` on `-p pmacs-gpu --bin pmacs-gpu`, `attach.rs:1889`
- sixteenth: gate log `20260907T185321Z-604527`, step `06-sweep-luajit`, the same result line, `attach.rs:1889`
- seventeenth: gate log `20260910T203556Z-1854124`, step `05-sweep` of E5's tip gate, `attach.rs:1958`, `test result: FAILED. 365 passed; 1 failed` (its own section above)
- eighteenth: gate log `20260917T001151Z-3116741`, step `05-sweep` on E6d's branch, `attach.rs:1971`, `test result: FAILED. 365 passed; 1 failed` (its own section above)
- nineteenth: gate log `20260919T224133Z-2027161`, step `05-sweep` on E7c's branch at `09798eb` (fix round 2's tip), load average 18, `attach.rs:1971`, `test result: FAILED. 366 passed; 1 failed`, all three fragments; the next run on the same tree (`20260919T224739Z-2089471`) six of six, non-reproduction and nothing more
- twentieth: gate log `20260920T110710Z-2592202`, step `05-sweep` on E7c's branch at `3de1e1f` (fix round 3's tip), the fifteen-minute load average 17 (the round's own bites and builds), `attach.rs:1971`, `test result: FAILED. 366 passed; 1 failed`, all three fragments; the next run on the same tree (`20260920T111455Z-2678460`) six of six, non-reproduction and nothing more
- twenty-first: gate log `20261002T114815Z-408166`, step `05-sweep` on E7h's branch at `64d396b` (fix round 4's tip, whose change from the green gate before it is comments in the fuzz harness and one test), the load average 3.4 at its start and about 18 over five minutes at its end (other sessions' work), `attach.rs:1971`, `test result: FAILED. 373 passed; 1 failed`, all three fragments (recorded in E7h's fourth fix round, below); the next run on the same tree (`20261002T115902Z-518091`) six of six, non-reproduction and nothing more
- twenty-second: gate log `20261004T104358Z-3360185`, step `05-sweep`, E7i review 2's gate on `e7i/parse-containment` at `cd6cd52` (PR #309), niced with `CARGO_BUILD_JOBS=6` beside the reviewer's own probes, `attach.rs:1971`, `test result: FAILED. 373 passed; 1 failed`, all three fragments; `pmacs-gpu/src/attach.rs` has no diff on the branch (recorded at E7i's fix round 1 and review 2, below); the next run on the same tree (`20261004T112802Z-3536917`) six of six, non-reproduction and nothing more
- twenty-third: gate log `20261004T123921Z-3949386`, step `05-sweep`, E7i fix round 2's first gate on `e7i/parse-containment` at `79d2895` (PR #309), niced with `CARGO_BUILD_JOBS=6`, `attach.rs:1971`, `test result: FAILED. 373 passed; 1 failed`, all three fragments; `pmacs-gpu/src/attach.rs` has no diff on the branch (recorded at E7i's fix round 2, below); the next run on the same tree (`20261004T125601Z-4051540` at `db697a2`, a test-only change) passed it, non-reproduction and nothing more
- twenty-fourth: gate log `20261004T202450Z-1217913`, step `07-sweep-luajit`, E8.1's gate on `e8/hover-and-signature` at `4fc5599`, `attach.rs:1971`, `test result: FAILED. 373 passed; 1 failed`, all three fragments; `pmacs-gpu/src/attach.rs` has no diff on the branch (recorded at E8's checkpoint, below)
- twenty-fifth: gate log `20261004T225031Z-2642055`, step `07-sweep-luajit`, E8.5's gate on `e8/hover-and-signature` at `a8104f4`, `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed` (375 since E8.3 added a GPU unit row), all three fragments; `attach.rs` has no diff on the branch
- twenty-sixth: gate log `20261005T193708Z-986096`, step `06-sweep`, E8 fix round 1's second gate on `e8/hover-and-signature` at `7505eea`, `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`Handshake(Io(Os { code: 32, kind: BrokenPipe`); `attach.rs` has no diff on the branch (recorded at E8's fix round 1, below)
- twenty-seventh: gate log `20261005T213945Z-1605050`, step `07-sweep-luajit`, E8 fix round 1's third gate on `e8/hover-and-signature` at `57a980e`, `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`Handshake(Io(Os { code: 32, kind: BrokenPipe`); `attach.rs` has no diff on the branch (recorded at E8's fix round 1, below)
- twenty-eighth: gate log `20261005T224931Z-2046695`, step `07-sweep-luajit`, E8 review 2's gate on `e8/hover-and-signature` at `57a980e`, `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`Handshake(Io(Os { code: 32, kind: BrokenPipe`); `attach.rs` has no diff on the branch (recorded at E8's fix round 2, below)
- twenty-ninth: gate log `20261006T111111Z-85507`, step `07-sweep-luajit`, E8 fix round 2's gate on `e8/hover-and-signature` at `d2de579`, `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`Handshake(Io(Os { code: 32, kind: BrokenPipe`); `attach.rs` has no diff on the branch, and the row passed in the same run's `06-sweep` (recorded at E8's fix round 2, below)
- thirtieth: gate log `20261006T175402Z-1776889`, step `07-sweep-luajit`, E8 fix round 4's gate on `e8/hover-and-signature` at `05dfa6a` (the branch with `main` at `93752c6` merged in), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`); `attach.rs` has no diff from `main`, and the row passed in the same run's `06-sweep` (recorded at E8's fix round 4, below)
- thirty-first: gate log `20261008T195204Z-1042685`, step `06-sweep`, the pre-release PR's gate on `release/pre-2.0.0` at `bca69d4` (#337), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`); `attach.rs` has no diff on the branch, and the row passed in the same gate's `07-sweep-luajit` (recorded with the pre-release PR, above)
- thirty-second: gate log `20261009T161057Z-421242`, step `07-sweep-luajit`, the pre-release PR's fix round 3 gate on `release/pre-2.0.0` at `f45fa10` (#337), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`); `attach.rs` has no diff on the branch, and the row passed in the same gate's `06-sweep` (recorded with that round, above)
- thirty-third: gate log `20261009T205956Z-1456466`, step `05-sweep` of the six-stage gate, #347's pull request's gate on `ci/replay-mechanism` at `2c890aa` (#348), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`); `pmacs-gpu` has no diff on the branch (recorded with that pull request, above)

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
| job | GitHub Actions: first the serialized crdt sweep (`--test-threads=1`), since then `Test (macos-latest / luajit)` seven times under cargo's default parallelism and `Test (ubuntu-latest / luajit)` once under `--test-threads=1`, and once the local gate's non-CRDT sweep. CORRECTED at E7's fix round 1 (2026-09-18, review 1's Low 2): this cell read "`Test (macos-latest / luajit)` seven times and `Test (ubuntu-latest / luajit)` once, all under cargo's default parallelism"; the ubuntu legs have run `--test-threads=1` since `d97e137` (`ci.yml` at `8f6784f`, the tree of the ubuntu occurrence, carries the flag, and every matrix leg carried it before E0), and only the macOS legs run at default parallelism. The job is not a discriminator for this row and a match needs the selector and the fragment on any CI test leg |
| required fragments | `first read_dir must be superseded; got ok` |
| occurrences | ten: `main` at `aae5b35`, run 33375945966 (the serialized crdt sweep); `main` at `d97e137`, run 34205653191, job `Test (macos-latest / luajit)`; PR #257 at `8f6784f`, run 34220035122, job `Test (ubuntu-latest / luajit)`; `main` at `dbe40a1`, run 34349759554 (E1's post-merge run), job `Test (macos-latest / luajit)` (102459915513), `tests/m8_1_acceptance.rs:278:5`, `assertion left == right failed: first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed`, the job's only failure against 119 `test result: ok`; and PR #269 review 2's gate at `95a6db4`, log `20260911T102129Z-2474066`, step `sweep-luajit` (the non-CRDT sweep under `--no-default-features --features luajit`), `tests/m8_1_acceptance.rs:278`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 0.33 s, the step's only failure, not rerun; and PR #270 at `e82fcb5`, run 34772792926, job `Test (macos-latest / luajit)` (103765362433), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.34 s, recorded at E6's close on 2026-09-13 and not rerun (the job's other red is the branch's own fixture defect, below); and PR #270 at `e804b97`, run 34774221987, the very next head, job `Test (macos-latest / luajit)` (103769261492), `tests/m8_1_acceptance.rs:278:5`, the same fragment, `9 passed; 1 failed` in 1.93 s, the job's only failure against 133 `test result: ok` --- the same fragments on the next run are a second occurrence, not a coincidence, and this row now has two on consecutive heads of one PR whose diff touches neither `src/dispatch` nor `tests/m8_1_acceptance.rs`, an argument from untouched files and not a measurement; and PR #273 at `c4be8aa`, run 35031658924, job `Test (macos-latest / luajit)` (104591399407), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.26 s, the job's only failure against 140 `test result: ok`, recorded at E6c's close on 2026-09-16 and not rerun; the branch's diff touches neither `src/dispatch` nor `tests/m8_1_acceptance.rs`, and its base control, `main`'s run at `7880c4b`, ran this leg green, which is one sample; and PR #275 at `b1ef903`, run 35221439579, job `Test (macos-latest / luajit)` (105202355414), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.49 s, the job's only failure against 148 `test result: ok`, recorded at E6d's fix round 1 on 2026-09-17 and not rerun; the commit is the review's witness commit and adds two files under `tests/`, touching neither `src/dispatch` nor `tests/m8_1_acceptance.rs`, and the selector ran `ok` on both macOS legs of the previous head `085ae4d`'s run, green samples; and PR #275 at `97b0d0f`, run 35252684064, job `Test (macos-latest / luajit)` (105308655905), `tests/m8_1_acceptance.rs:278:5`, `first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed` in 2.61 s, the job's only failure against 148 `test result: ok`, recorded at E6d's fix round 1 on 2026-09-17 and not rerun; the commit touches one test file of the branch's own, and the selector ran `ok` on both macOS legs of the head before it, `888e5e1`, green samples. The second and fourth run at cargo's DEFAULT parallelism under D23, as do the five macOS occurrences after them, so serialization is not required to produce it; the third is on LINUX under `--test-threads=1`, serialized like the first, so macOS is not required either. CORRECTED at E7's fix round 1 (2026-09-18, review 1's Low 2): this sentence read "The second, third and fourth run at cargo's DEFAULT parallelism under D23, and the third is on LINUX, so neither serialization nor macOS is required to produce it"; `Test (ubuntu-latest / luajit)` has run `--test-threads=1` since `d97e137` and PR #257's tree at `8f6784f` carried the flag, so the third occurrence was serialized, and the inference that serialization is not required stands on the seven macOS occurrences alone. The second is the merge-base control for the third: the same signature is on `d97e137` itself, so the branch did not introduce it. That control is ONE run at the base, which is one sample: it establishes that the signature exists on `d97e137`, not its rate. PR #257's second run, 34222042303 at `e78d184`, is GREEN on `Test (ubuntu-latest / luajit)`, and so are its third, fourth and fifth. Green runs are samples: non-reproduction and nothing more. The fourth occurrence is on `main` after two merges that did not touch `src/dispatch` or `tests/m8_1_acceptance.rs`, which is an argument from untouched files and not a measurement; it is the row's third occurrence on `main` and the second on this leg. The fifth is the gate's non-CRDT sweep on the reviewed head, not CI, and it establishes neither cause nor the branch's innocence; it is recorded here as an occurrence because the selector and the required fragment match exactly |
| candidate mechanism | the predecessor completed before the cancellation took effect. `--test-threads=1` was the first occurrence's candidate: it serializes the test functions in one executable and so removes one source of contention the test's "in flight" depends on. The second occurrence has no such flag, which does not refute the mechanism --- a fast predecessor is a fast predecessor however the runner got there --- but it does mean serialization is not required to produce it, and the remaining common factor is a macOS or Linux CI runner rather than a scheduling flag. The third, on Linux, ran serialized like the first (stated at E7's fix round 1, 2026-09-18, review 1's Low 2, where this cell had implied an unserialized Linux leg): a serialized leg and a fast-worker race are compatible, since serialization removes the contention that keeps a predecessor in flight and so makes the race easier to lose, not harder; the flag is one way a runner gets there and not a discriminator, and the Linux occurrence is read as evidence that macOS is not required, not that serialization is absent. Nothing has measured the predecessor's duration under either, and nothing rules out a real supersede defect |
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
