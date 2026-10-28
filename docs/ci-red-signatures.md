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

**Since D38 (the owner's ruling of 2026-10-18) a row's tally counts CI
alone**, and every close records its runs here on `main`'s tip. By the
owner's ruling of 2026-10-20, no local gate sample is counted whatever
its date, those met before 2026-10-18 included: a local sample is
listed on its issue, in the pass, and here in a block no tally counts.
The counts written before E9's close, local samples among them, stand
as written.

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

#264 stays at two. The local samples since E4's, listed and not counted
under the owner's ruling of 2026-10-20 (recorded at E9's close, below):

- gate log `20261015T172141Z-3943`, `05-sweep`, `2293 passed; 1
  failed; 13 ignored`, at `b73abbe` (E8e.3)
- gate log `20261016T100845Z-734980`, `07-sweep-luajit`, `2094 passed; 1
  failed; 12 ignored`, at `772b039` (E9.2)
- gate log `20261017T170736Z-1690674`, `06-sweep`, `2288 passed; 2
  failed; 13 ignored`, the other failure #377's, at `e7c8fd5` (E9's tip)

The panic now sits at `src/daemon_attach.rs:859:9`; the line moves and
the issue says so.

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

#### #291, seventeen occurrences, all local

`e7e_haskell_acceptance::e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error`, armed by `PMACS_REQUIRE_HLS` and run only on this laptop, with the three fragments `HLS reports the type error; store []`, a trace reaching `LSP:ready·setting` and never `processi`, and `4 passed; 1 failed; 0 ignored`. Each occurrence:

Tally (291): 17 items in the list below.

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
- 2026-10-10, `scripts/gate --protocol` on `tests/bounded-rows` at `ae7e8c2` (#353), `20261010T084217Z-2263254`, step `07-sweep-luajit`, 133.91 s, `store []`, the trace reaching `LSP:ready·setting` and never `processi`; that gate's `06-sweep` failed the row with #311's fragments (recorded with that pull request, below).

The local samples since, listed and not counted under the owner's ruling of 2026-10-20 (recorded at E9's close, below):

- 2026-10-17, `scripts/gate --protocol` on `e9/session-identity` at `e7c8fd5` (#378, E9's tip), `20261017T170736Z-1690674`, step `07-sweep-luajit`, 126.72 s, `store []`, the trace reaching `LSP:ready·setting` and never `processi`; that gate's `06-sweep` failed the row with #311's fragments.
- 2026-10-18, `scripts/gate --protocol` at `e04aa9a`, E9 fix round 1's gate `20261018T143623Z-2650551`, step `07-sweep-luajit`, 128.20 s, `store []`, the trace reaching `LSP:ready·setting` and never `processi`; that gate's `06-sweep` failed the row with #311's fragments.

#291 is at seventeen; each is commented on the issue. Until E7h's third fix round this paragraph attributed it to load, as "four of them E7h's and all four beside heavy load": niced fuzz runs, a compile matrix, a game. **The sixth falsifies that.** `20261002T012705Z-3618489` began on a nearly idle machine (load about 1.5, rising to 3 to 10 as other sessions resumed) with nothing of the round's beside it, and the row failed on all three fragments. Nor did the attribution hold before it. The first sample's alone runs in the gate's environment, eight and then sixteen interleaved pairs, failed four times in forty with no load recorded, and the third occurrence came at a load of about 4 on sixteen threads. Load is not this row's cause, and this file attributes it none. Its trace shows haskell-language-server finishing its session setup and never processing the typed edit, which does not separate the server from pmacs's side of the exchange. The row passed in E7h review 2's two gates on a quieter machine (`20261001T083032Z-718803` at `17c3c8f` and `20261001T155727Z-1465534` at `d1cbca6`), which is non-reproduction and nothing more.

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

Tally (311-runs): 50 items in the list below.

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
- `20261010T084217Z-2263254` at `ae7e8c2` (the bounded-rows pull request's gate, `06-sweep`, its
  `07-sweep-luajit` failing the row with #291's fragments instead; recorded with it, below).
- `20261010T094513Z-2620566` at `4ac5289` (E8c's base control, `05-sweep`; recorded with E8c,
  below).
- `20261010T102908Z-2908057` at `d79b5e5` (E8c's first tip gate, a commit rewritten before any push,
  `05-sweep`; recorded with E8c, below).
- `20261010T105213Z-3113170` at `a35b76b` (E8c's tip gate, `05-sweep`; recorded with E8c, below).
- `20261010T122342Z-3576747` at `a35b76b` (E8c review 1's first gate, `05-sweep`; recorded with
  E8c's fix round 1, below).
- `20261010T125112Z-3703490` at `a35b76b` (the same review's second gate, `05-sweep`; recorded
  with E8c's fix round 1, below).
- `20261010T130620Z-3845623` at `a35b76b` (the same review's third gate, `05-sweep`; recorded
  with E8c's fix round 1, below).
- `20261010T141313Z-4170219` at `d5327b6` (E8c fix round 1's tip gate, `05-sweep`; recorded with
  that round, below).
- `20261010T175142Z-30875` at `752811c` (the same round's gate at the head, `05-sweep`; recorded
  with that round, below).
- `20261010T192148Z-332172` at `c33b55c` (#360's pull request's gate, `05-sweep`; recorded with
  it, below).
- `20261011T171633Z-1297485` at `5f53b91` (#365's pull request's gate, `05-sweep`; recorded with
  it, below).
- `20261011T214718Z-1799868` at `1f6d0b9` (#368's pull request's gate, `05-sweep`; recorded with
  it, below).
- `20261011T223022Z-1939930` at `19a163f` (the same pull request's gate at its head, `05-sweep`;
  recorded with it, below).
- `20261012T170517Z-2804859` at `824ca82` (the wall-clock classification pass's gate, `05-sweep`;
  recorded with it, below).

The gate runs read since, listed and not counted under the owner's ruling of 2026-10-20
(recorded at E9's close, below):

- `20261014T095325Z-3770109` at E8d.1's tree, its commit `d41a83b` (E8d's first gate, `05-sweep`).
- `20261014T102208Z-3956285` at E8d.2's tree, `81b8a2e` (`05-sweep`).
- `20261014T104006Z-4087067` at E8d.3's tree, `5560202` (`05-sweep`).
- `20261014T105531Z-4623` at E8d.4's tree, `e4c846d` (`05-sweep`).
- `20261014T111114Z-169009` at E8d.5's tree, `33a981f` (`05-sweep`).
- `20261014T112755Z-283384` at `e4a0cb8` (E8d.6's gate, `05-sweep`).
- `20261014T122022Z-409760` at `d628b21` (E8d's tip gate, `05-sweep`).
- `20261014T135856Z-532336` at `d628b21` (E8d review 1's gate, `05-sweep`).
- `20261014T161137Z-777377` at `22de659` (E8d fix round 1's gate, `05-sweep`).
- `20261014T171638Z-1006348` at `22de659` (the owner's pass gate over E8d, `05-sweep`).
- `20261014T180411Z-1322112` at `7ef32d1` (E8d fix round 2's gate, `05-sweep`).
- `20261015T112921Z-1698319` at `a7f915b` (E8e.1's gate, `05-sweep`).
- `20261015T122545Z-1811476` at `7ae872a` (E8e.2's gate, `05-sweep`).
- `20261015T172141Z-3943` at `b73abbe` (E8e.3's gate, `05-sweep`).
- `20261015T183629Z-118839` at `b73abbe` (E8e review 1's gate, `05-sweep`).
- `20261016T092821Z-377716` at `9b02148` (E9.1's gate, both sweeps).
- `20261016T100845Z-734980` at `772b039` (E9.2's gate, both sweeps).
- `20261016T154436Z-987086` at `fd32170` (E9's gate at E9.2's amend, both sweeps).
- `20261016T162234Z-1345997` at `c9fe513` (E9.3's gate, both sweeps).
- `20261017T170736Z-1690674` at `e7c8fd5` (E9's tip gate; `06-sweep` only, its `07-sweep-luajit` failing the row with #291's fragments instead).
- `20261018T131101Z-2048560` at `e7c8fd5` (E9 review 1's gate, both sweeps).
- `20261018T143623Z-2650551` at `e04aa9a` (E9 fix round 1's gate; `06-sweep` only, its `07-sweep-luajit` failing the row with #291's fragments instead).

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

### The bounded-rows pull request: its gate, PR #353 green at `ae7e8c2`, `main` red at `8b144da` on #316's tenth and #352, #291 at seventeen and #311's list at thirty-seven

Recorded 2026-10-10 by the bounded-rows pull request, #353 at `ae7e8c2` (four signed commits on
`8b144da`: `bounded` shared from `tests/lean4_server_acceptance.rs` as `tests/common/bounded.rs`,
and #327's, #344's and #349's rows run under it). Containment, not repair: the three rows' causes
are unknown and their issues stay open. The runs were read from the jobs endpoint by conclusion
and through every test leg's log; none was rerun.

#### `main` at `8b144da`, this branch's base

`8b144da` is #348's squash and `main`'s last code-bearing commit, so it is this branch's base
control. Its runs were not recorded here before.

`CI` 38001268585 at `8b144da`, attempt 1 of 1, `push`, created 22:49:28Z and completed 23:23:02Z:
`failure`, both macOS legs red.

Tally (main-ci-8b144da): 19 = 16 + 2 + 1.

Tally (main-ci-8b144da-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 114059641971 | success |
| Format | 114059642169 | success |
| Commit attribution (D9) | 114059642193 | success |
| Changed paths | 114059642287 | success |
| Lint (luajit) | 114059642371 | success |
| Test (crdt) | 114059683731 | success |
| GPU Render (headless) | 114059683732 | success |
| M1 Acceptance Gates | 114059683764 | success |
| M4 Perf Gates | 114059683765 | success |
| M5 Perf Gates | 114059683774 | success |
| M6 Perf Gates | 114059683830 | success |
| Test (macos-latest / lua54) | 114059683839 | failure |
| Test (macos-latest / luajit) | 114059683842 | failure |
| Perf budgets (debug) | 114059683846 | success |
| Test (ubuntu-latest / lua54) | 114059683901 | success |
| M10 Perf Gates (crdt) | 114059683954 | success |
| Test (ubuntu-latest / luajit) | 114059683991 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114059684053 | success |
| Docs consistency | 114059685494 | skipped |

Tally (main-ci-8b144da-success): 16 rows of the table above with `result` = `success`.

Tally (main-ci-8b144da-failure): 2 rows of the table above with `result` = `failure`.

- `Test (macos-latest / lua54)` (114059683839):
  `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
  on kill 1, `kill 1: nothing held the switch's follow-up parse, which started a worker at once and
  installed`, the report `deaths=1 crashes=0 busy=true … tree=true unit=pid_38101 death=killed, " +
  …/a.rs  (ruparse:killed !1  L1:C1 Top"`, and `test result: FAILED. 10 passed; 1 failed; 0
  ignored` (26.88 s; the panic site now `tests/e7i_review3_probes.rs:865:9`, not a fragment): all
  three fragments, **#316's tenth**, commented. `8b144da` moves the worker's `RLIMIT_AS` refusal
  string into a constant, its bytes unchanged, and does not touch the suite; read, not shown.
- `Test (macos-latest / luajit)` (114059683842):
  `bottom_panel_stage1_acceptance::acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel`,
  its `pmacs.terminal.terminate` refused: `kill: EPERM: Operation not permitted (signal=SIGTERM,
  target=-15821 via tcgetpgrp, leader_pid=15821, expected_group=-15821, leader=live)`, and `test
  result: FAILED. 51 passed; 1 failed; 0 ignored` (23.85 s): **#352, filed, its first
  occurrence**. By the matching rule it is not R3, another selector whose fragments it lacks
  (`measured_group=unobservable(`, `ESRCH`), nor #259, this row's readiness file arriving late.
- Each red leg paired 191 of its 192 `running` lines with `test result: ok` and one with `FAILED`;
  every other test leg pairs each `running` with a `test result: ok`, 192 on `Test (crdt)` and 193
  on each Ubuntu `test` leg.

`Grammar fuzz` 38001268430 at `8b144da`, attempt 1 of 1, `success`, created 22:49:28Z and
completed 23:46:38Z: `Grammar fuzz` 114059640711 and `Grammar fuzz (asan-strict)` 114059640532,
GCC 13 on `ubuntu24 20261004.327.1`, both fuzzing (`reason=src/bin/pmacs_grammar_fuzz.rs
changed`). No crash, and no worker crashed in either replay. Both reported the standing findings:
json's allocation (135,120 bytes), CUDA's slow input (245,920), cmake's three (exponents 1.47 to
2.21) and lua's (252,415 bytes, exponent 0.93); `ubsan` also its markdown_inline hang that did not
reproduce alone (262,144 bytes), which does not fail a run. Each arm's replay header read `memory
1024 MiB by the worker's watch`, #348's wording, now on `main`.

#### The pull request's gate: `20261010T084217Z-2263254` at `ae7e8c2`

`scripts/gate --protocol` at the tip, as a change under `tests/common` requires: six of eight.
fmt, clippy, clippy-luajit, doc, build and diff-check ok; no `pmacs --daemon` survived (identified
by environ). The load average reached 36 during the sweeps, with other heavy work beside them.

- `06-sweep`: 201 paired result lines, 5,376 passed, 2 failed, 65 ignored:
  - the HLS row with #311's fragments (`Could not find module ‘Prelude’`, the trace reaching
    `LSP:ready·processi`, `test result: FAILED. 4 passed; 1 failed; 0 ignored`, 136.40 s), so
    #311's list gains the gate;
  - `pmacs-gpu`'s `tests::overwide_status_runs_never_wrap_and_keep_the_suffix_pinned`, `assertion
    failed: min_y >= band_top && max_y <= height` at `pmacs-gpu/src/main.rs:25303:13`, `test
    result: FAILED. 374 passed; 1 failed; 0 ignored`: the sibling recorded on #304 since E7h, this
    time alone, #304's own row passing beside it; commented there, not filed apart.
- `07-sweep-luajit`: 201 paired, 4,935 passed, 2 failed, 51 ignored:
  - the HLS row with #291's fragments (`HLS reports the type error; store []`, the trace `[(1,
    "LSP:init"), (11938, "LSP:ready"), (12150, "LSP:ready·setting"), (15172, "LSP:ready")]`
    never reaching `processi`, `4 passed; 1 failed; 0 ignored`, 133.91 s): **#291's
    seventeenth**, its list above extended;
  - `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
    on kill 1, the same message, the report `deaths=1 … busy=false … tree=true unit=pid_2503877
    death=killed` with `parse:killed !1` on its mode line, and `test result: FAILED. 10 passed; 1
    failed; 0 ignored` (14.72 s): **#316's eleventh**. It passed in `06-sweep`.

The four bounded rows passed in both sweeps; so did R7's row and every other filed row the sweeps
ran, which is non-reproduction and nothing more. The four occurrences are commented on their
issues.

#### `CI` 38038965404 at `ae7e8c2`, attempt 1 of 1: green on every job

`pull_request`, the merge `cf99df2` (`ae7e8c2` into `8b144da`, read from the test logs' checkout
line and the commit's parents), created 08:45:06Z and completed 09:15:34Z: `success`.

Tally (pr353-ci-ae7e8c2): 19 = 18 + 1.

Tally (pr353-ci-ae7e8c2-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 114175218655 | success |
| Format | 114175218817 | success |
| Lint (luajit) | 114175218859 | success |
| Changed paths | 114175218865 | success |
| Lint (lua54) | 114175218975 | success |
| M4 Perf Gates | 114175253530 | success |
| Test (crdt) | 114175253541 | success |
| Test (macos-latest / lua54) | 114175253548 | success |
| Perf budgets (debug) | 114175253561 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114175253576 | success |
| M5 Perf Gates | 114175253580 | success |
| Test (ubuntu-latest / lua54) | 114175253591 | success |
| M6 Perf Gates | 114175253597 | success |
| Test (ubuntu-latest / luajit) | 114175253598 | success |
| M10 Perf Gates (crdt) | 114175253600 | success |
| GPU Render (headless) | 114175253604 | success |
| M1 Acceptance Gates | 114175253625 | success |
| Test (macos-latest / luajit) | 114175253668 | success |
| Docs consistency | 114175254249 | skipped |

Tally (pr353-ci-ae7e8c2-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 192 on `Test (crdt)`, 193 on
  each Ubuntu `test` leg, 194 on each macOS leg; none failed. The four bounded rows passed on all
  six. On the macOS legs (lua54, luajit) `lsp_spawn_guidance_acceptance` took 1.35 and 4.33 s,
  `lsp_latex_acceptance` 2.64 and 8.01 s and `m4_acceptance` 88.76 and 101.24 s, and no bounded
  row printed a sixty-second notice there; the rows that did are the three long ones #351 names.

`Grammar fuzz` 38038965360 at `ae7e8c2`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

#### What the bound changes in how #327, #344 and #349 read

Each issue's required fragments end with its leg cancelled at the 45-minute limit. Once #353
merges they cannot recur as written: past its limit the row fails, `the row did not finish within
<limit>: its last step, "<step>", was entered <t> ago and has not returned`, beside the `ps` rows
of every process beneath the binary, its suite prints `test result: FAILED`, and the leg goes on.
Commented on each issue; whether such a red is that issue's next sample or a new issue is the
owner's to rule. A bound on every row is #351, filed and not taken.

So, at the bounded-rows pull request: #316 moves to eleven and #291 to seventeen; #352 is filed at
one; #311's list of runs reads at thirty-seven; every other count above stands.

### E8c, quit must exit: its gates, PR #357 red at `a35b76b`, `main` green at `4ac5289`, #316 at fourteen, #358 filed and #311's list at forty

Recorded 2026-10-10 by E8c, #357 at `a35b76b`. That is four signed commits on `4ac5289`: the fake
server's `abortonopen` and `stopanswering`; generated writes leaving their buffers clean, so quit
no longer asks about the editor's own text (#299's stop); the reviewer's bound as PTY rows; and a
bounded join of a dead child's readers. The runs were read from the jobs endpoint by conclusion
and through every test leg's log; none was rerun.

#### `main` at `4ac5289`, this branch's base

`4ac5289` is #353's squash and `main`'s last code-bearing commit, so it is this branch's base
control. Its runs were not recorded here before.

`CI` 38041890976 at `4ac5289`, attempt 1 of 1, `push`, created 09:34:52Z and completed 10:05:33Z:
`success`.

Tally (main-ci-4ac5289): 19 = 18 + 1.

Tally (main-ci-4ac5289-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 114183618583 | success |
| Lint (luajit) | 114183618713 | success |
| Lint (lua54) | 114183618773 | success |
| Commit attribution (D9) | 114183618793 | success |
| Changed paths | 114183618815 | success |
| M4 Perf Gates | 114183642931 | success |
| Test (crdt) | 114183642978 | success |
| M1 Acceptance Gates | 114183642984 | success |
| GPU Render (headless) | 114183642985 | success |
| M10 Perf Gates (crdt) | 114183643001 | success |
| M5 Perf Gates | 114183643004 | success |
| Perf budgets (debug) | 114183643026 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114183643046 | success |
| Test (macos-latest / lua54) | 114183643057 | success |
| Test (macos-latest / luajit) | 114183643058 | success |
| Test (ubuntu-latest / luajit) | 114183643063 | success |
| Test (ubuntu-latest / lua54) | 114183643080 | success |
| M6 Perf Gates | 114183643116 | success |
| Docs consistency | 114183643815 | skipped |

Tally (main-ci-4ac5289-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 192 on `Test (crdt)`, 193 on
  each Ubuntu `test` leg, 194 on each macOS leg; none failed.
- #353's four bounded rows passed on both macOS legs, and none printed a sixty-second notice.
  #316's row passed on all six legs.

`Grammar fuzz` 38041890794 at `4ac5289`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

#### E8c's gates

Three `scripts/gate` runs of the default plan. `pmacs-protocol` has no diff, so `--protocol` was
not run; CI's luajit lint was run by hand at both tips and was clean. No `pmacs --daemon` survived
any of them (identified by environ).

- `20261010T094513Z-2620566` on `main`'s tree at `4ac5289`, the base control, run beside hand
  reproductions of #299 (load 13 to 130). Five of six; `05-sweep` has 201 paired result lines,
  5,376 passed, 2 failed, 65 ignored:
  - the HLS row with #311's fragments (`Could not find module ‘Prelude’`, the trace reaching
    `LSP:ready·processi`, `4 passed; 1 failed`, 126.60 s), so #311's list gains it;
  - `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
    on kill 2: `kill 2: nothing held the switch's follow-up parse, which started a worker at once
    and installed`, the report `deaths=2 … busy=false … tree=true unit=pid_2741033 death=killed, "
    +* …/a.rs  (rusparse:killed !2`, and `10 passed; 1 failed; 0 ignored` (14.81 s). Its literal
    fragments name kill 1 and `" +  `, so as written they do not match. It is **#316's twelfth
    under the template reading**, the reading its third and sixth were counted by, and commented
    there.
- `20261010T102908Z-2908057` at `d79b5e5`, a first tip rewritten before any push. Five of six;
  `05-sweep` has 202 paired, 5,390 passed, 4 failed, 65 ignored:
  - #311's row, the same fragments (126.35 s), so #311's list gains it;
  - three rows of the branch's own making, `e8b_own_name_acceptance`'s
    `c_x_k_dired_kills_the_dired_buffer_and_says_so` and `c_x_k_keeps_d18_s_four` (`premise: the
    dired buffer reads as modified …`, `premise: *lsp* reads as modified`) and
    `e8b_switch_buffer_acceptance`'s
    `e8b_fr1_the_gpu_kills_the_buffer_its_own_name_names_and_says_so`. They asserted the old flag
    the branch removes, and were changed in the branch's fix commit (`d168e57`). They are not
    signatures.
- `20261010T105213Z-3113170` at `a35b76b`, the head. Five of six; `05-sweep` has 202 paired, 5,393
  passed, 2 failed, 65 ignored:
  - #311's row, the same fragments (131.79 s), so #311's list gains it;
  - the outside-kill row on kill 1, `kill 1: nothing held the switch's follow-up parse, which
    started a worker at once and installed`, the report `deaths=1 crashes=0 busy=true held=false
    pending=0 stopped=false tree=true unit=pid_3212527 death=killed, " +  …/a.rs  (rusparse:killed
    !1  L1:C1 Top"`, and `test result: FAILED. 10 passed; 1 failed; 0 ignored` (15.96 s): all
    three fragments, **#316's thirteenth**, commented.

R7's row passed in all three sweeps, which is non-reproduction and nothing more.

#### `CI` 38047504467 at `a35b76b`, attempt 1 of 1: red on two legs, #316's fourteenth and #358's first

`pull_request`, the merge `5d73a0d` (`a35b76b` into `4ac5289`, read from the test logs' checkout
line), created 11:11:01Z and completed 11:40:28Z: `failure`.

Tally (pr357-ci-a35b76b): 19 = 16 + 2 + 1.

Tally (pr357-ci-a35b76b-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 114199885889 | success |
| Lint (luajit) | 114199886104 | success |
| Changed paths | 114199886106 | success |
| Commit attribution (D9) | 114199886113 | success |
| Lint (lua54) | 114199886119 | success |
| Test (crdt) | 114199917396 | success |
| M5 Perf Gates | 114199917420 | success |
| M1 Acceptance Gates | 114199917423 | success |
| GPU Render (headless) | 114199917473 | success |
| M4 Perf Gates | 114199917475 | success |
| Perf budgets (debug) | 114199917478 | success |
| M10 Perf Gates (crdt) | 114199917491 | success |
| Test (macos-latest / luajit) | 114199917506 | failure |
| Test (macos-latest / lua54) | 114199917510 | success |
| Test (ubuntu-latest / luajit) | 114199917511 | failure |
| Test (ubuntu-latest / lua54) | 114199917527 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114199917530 | success |
| M6 Perf Gates | 114199917734 | success |
| Docs consistency | 114199918347 | skipped |

Tally (pr357-ci-a35b76b-success): 16 rows of the table above with `result` = `success`.

Tally (pr357-ci-a35b76b-failure): 2 rows of the table above with `result` = `failure`.

- `Test (macos-latest / luajit)` (114199917506):
  `process::tests::repeated_terminate_does_not_extend_ledger_deadline`, `earliest deadline must
  win: SIGKILL by ~500ms, not 800ms; took 779.031875ms`, and `test result: FAILED. 2282 passed; 1
  failed; 13 ignored` (124.19 s), the escalation itself asserted and passing. It is **#358, filed,
  its first occurrence**: no row or issue named the selector. The branch changes
  `RuntimeHandles`'s drop on this row's path. The row alone took 0.52 s in all of 20 local runs at
  `a35b76b` and 20 with `src/process.rs` at its parent, so it is not shown innocent on macOS and
  the issue says so.
- `Test (ubuntu-latest / luajit)` (114199917511): the outside-kill row on kill 1, the report
  `deaths=1 crashes=0 busy=false … tree=true unit=pid_30127 death=killed, " +
  /tmp/.tmpPTaisq/a.rs  (rust) … parse:killed !1`, and `test result: FAILED. 10 passed; 1 failed;
  0 ignored` (17.30 s): all three fragments, **#316's fourteenth**, its first in CI since
  `8b144da`'s, commented.
- Each red leg paired every `running` line with a result line, 192 `ok` and one `FAILED` on macOS
  luajit, 191 and one on Ubuntu luajit. Their doc-test and `pmacs-protocol` steps are `skipped`
  after the sweep step failed, read from the jobs' steps, which accounts for the two fewer result
  lines than their siblings. The other test legs pair each `running` with a `test result: ok`: 193
  on `Test (crdt)`, 194 on each other Ubuntu `test` leg, 195 on `Test (macos-latest / lua54)`.
  E8c's suite ran its 17 tests on all six legs and passed.

`Grammar fuzz` 38047504451 at `a35b76b`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

So, at E8c: #316 moves to fourteen; #311's list of runs reads at forty; #358 is filed at one; #355
and #356 are filed, neither a red; every other count above stands.

### E8c's fix round 1: review 1's gates and the round's, PR #357 red on #361's first at `752811c`, two heads red on macOS by the round's own helpers, R7's thirty-fourth, #312's second, #359's first and #311's list at forty-five

Recorded 2026-10-10 by E8c's fix round 1, #357 at `752811c`, fourteen signed commits on `4ac5289`, ten of them this round's on `a35b76b`. They commit review 1's probes and drivers, make a reap that leaves a dead process's readers behind say so, keep quit's state write out of the quit-bound rows, join polling readers without a bound, and give a tick's and a teardown's reaps one shared wait. They also keep a dead PTY's writer open through its drain, gate two test helpers to Linux, and correct the records. The runs were read from the jobs endpoint by conclusion and through the red jobs' logs; none was rerun.

#### E8c review 1's gates, at `a35b76b`

Three `scripts/gate` runs of the default plan by the review, at the head, recorded here because the review commits nothing. Each sweep's figures were recomputed from its `05-sweep.log`. The review reports no surviving daemon for its first gate; that step leaves no log.

- `20261010T122342Z-3576747`: five of six; `05-sweep` has 202 paired result lines, 5,393 passed, 2 failed, 65 ignored:
  - #311's row with #311's fragments (`Could not find module ‘Prelude’`, the trace reaching `LSP:ready·processi`, `4 passed; 1 failed`, 127.53 s), so #311's list gains it;
  - `-p pmacs --test e6b_review1_edit_log c6b_gpu_edit_shapes_correct_when_the_held_answer_lands`, `edit probe failed:`, the paste case's last frame at 293 ms without the held semantic answer, and `test result: FAILED. 2 passed; 1 failed; 0 ignored` (11.24 s): **#359's first occurrence**, filed by the review with those fragments.
- `20261010T125112Z-3703490`: five of six; 202 paired, 5,394 passed, 1 failed, 65 ignored: #311's row, the same fragments (126.75 s), so #311's list gains it. #359's row passed, which is non-reproduction and nothing more.
- `20261010T130620Z-3845623`: five of six; 202 paired, 5,392 passed, 3 failed, 65 ignored:
  - #311's row, the same fragments (128.06 s), so #311's list gains it;
  - `-p pmacs --lib process::tests::pty_mode_child_sees_a_tty`, `tty(1) should report a pty path in PTY mode; got ""` and `test result: FAILED. 2285 passed; 1 failed; 13 ignored` (11.73 s): **#312's second sample**, commented by the review. At `a35b76b`, `src/process.rs` differs from `4ac5289` only in `RuntimeHandles`'s drop, not in the drain #312's reading names; that is a reading, and the row stays separate from #289;
  - **R7's thirty-fourth** (`attach.rs:1971`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`, `374 passed; 1 failed`), its list extended. `pmacs-gpu` has no diff on the branch.

#### The round's gates

- `20261010T141313Z-4170219` at `d5327b6`: five of six; `05-sweep` has 203 paired result lines (the review's probe suite added), 5,407 passed, 1 failed, 65 ignored. The one is #311's row, the same fragments (128.38 s), so #311's list gains it. The rows of #312, #316, #358, #359 and R7 passed, which is non-reproduction and nothing more. No `pmacs --daemon` survived (identified by environ).
- `20261010T143409Z-141932` at `124b955`: fmt, clippy, doc and build ok; the sweep was terminated while compiling, before any test ran, when the session's harness stopped background work for the laptop's memory. It is no sample of anything, and #311's list does not count it. It was not rerun; the
  owner asked for a gate at the head instead.
- `20261010T175142Z-30875` at `752811c`, the head, run at the owner's word once the memory was free:
  five of six; `05-sweep` has 203 paired result lines, 5,408 passed, 1 failed, 65 ignored. The one is
  #311's row, the same fragments (126.63 s), so #311's list gains it. #361's row passed locally, as did
  the rows of #312, #316, #358, #359 and R7, which is non-reproduction and nothing more. No `pmacs
  --daemon` survived (identified by environ).

#### `CI` 38059810024 at `d5327b6`, attempt 1 of 1: two macOS legs red on the round's own helpers, then cancelled

`pull_request`, created 14:29:34Z, cancelled at 14:34:35Z by the push of `124b955`: `cancelled`.

Tally (pr357-ci-d5327b6): 19 = 12 + 2 + 4 + 1.

Tally (pr357-ci-d5327b6-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 114235590367 | success |
| Changed paths | 114235590491 | success |
| Lint (lua54) | 114235590508 | success |
| Lint (luajit) | 114235590533 | success |
| Commit attribution (D9) | 114235590558 | success |
| GPU Render (headless) | 114235625071 | success |
| M1 Acceptance Gates | 114235625132 | success |
| Perf budgets (debug) | 114235625146 | success |
| M4 Perf Gates | 114235625153 | success |
| Test (crdt) | 114235625155 | cancelled |
| Test (macos-latest / lua54) | 114235625163 | failure |
| M10 Perf Gates (crdt) | 114235625171 | success |
| M6 Perf Gates | 114235625182 | success |
| M5 Perf Gates | 114235625192 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114235625205 | cancelled |
| Test (ubuntu-latest / lua54) | 114235625209 | cancelled |
| Test (ubuntu-latest / luajit) | 114235625226 | cancelled |
| Test (macos-latest / luajit) | 114235625293 | failure |
| Docs consistency | 114235625817 | skipped |

Tally (pr357-ci-d5327b6-success): 12 rows of the table above with `result` = `success`.

Tally (pr357-ci-d5327b6-failure): 2 rows of the table above with `result` = `failure`.

Both macOS legs failed building the lib tests: `error: function \`heir_holder_spec\` is never used`, `error: struct \`Heirs\` is never constructed`, `could not compile \`pmacs\` (lib test) due to 2 previous errors`. Those are test helpers the round added for Linux-only rows, dead on macOS under CI's denied warnings, and fixed at `752811c`. They are the branch's own and not a signature.

#### `CI` 38060125815 at `124b955`, attempt 1 of 1: the same two legs, then cancelled

`pull_request`, created 14:34:10Z, cancelled at 14:40:03Z by the push of `752811c`: `cancelled`.

Tally (pr357-ci-124b955): 19 = 11 + 2 + 5 + 1.

Tally (pr357-ci-124b955-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 114236590958 | success |
| Commit attribution (D9) | 114236591057 | success |
| Format | 114236591060 | success |
| Changed paths | 114236591076 | success |
| Lint (luajit) | 114236591110 | success |
| M1 Acceptance Gates | 114236632166 | success |
| M4 Perf Gates | 114236632173 | success |
| GPU Render (headless) | 114236632201 | success |
| Test (crdt) | 114236632210 | cancelled |
| Test (macos-latest / lua54) | 114236632234 | failure |
| Perf budgets (debug) | 114236632235 | success |
| M10 Perf Gates (crdt) | 114236632256 | cancelled |
| M5 Perf Gates | 114236632260 | success |
| Test (ubuntu-latest / luajit) | 114236632264 | cancelled |
| M6 Perf Gates | 114236632265 | success |
| Test (ubuntu-latest / lua54) | 114236632272 | cancelled |
| Test (ubuntu-latest / luajit, no crdt) | 114236632276 | cancelled |
| Test (macos-latest / luajit) | 114236632308 | failure |
| Docs consistency | 114236632860 | skipped |

Tally (pr357-ci-124b955-success): 11 rows of the table above with `result` = `success`.

Tally (pr357-ci-124b955-failure): 2 rows of the table above with `result` = `failure`.

Both macOS legs failed on the same two errors; the branch's own, fixed at `752811c`.

#### `CI` 38060487700 at `752811c`, attempt 1 of 1: red on one macOS leg, #361's first

`pull_request`, the merge `ac6e089` (`752811c` into `1b73532`, read from the macOS lua54 leg's fetch line), created 14:39:35Z and completed 15:09:23Z: `failure`.

Tally (pr357-ci-752811c): 19 = 17 + 1 + 1.

Tally (pr357-ci-752811c-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 114237664410 | success |
| Format | 114237664594 | success |
| Changed paths | 114237664649 | success |
| Lint (lua54) | 114237664732 | success |
| Lint (luajit) | 114237664860 | success |
| GPU Render (headless) | 114237705335 | success |
| M1 Acceptance Gates | 114237705337 | success |
| M10 Perf Gates (crdt) | 114237705377 | success |
| M5 Perf Gates | 114237705392 | success |
| M4 Perf Gates | 114237705406 | success |
| Test (crdt) | 114237705449 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114237705451 | success |
| Test (ubuntu-latest / lua54) | 114237705465 | success |
| M6 Perf Gates | 114237705469 | success |
| Test (macos-latest / lua54) | 114237705474 | failure |
| Test (ubuntu-latest / luajit) | 114237705481 | success |
| Test (macos-latest / luajit) | 114237705507 | success |
| Perf budgets (debug) | 114237705540 | success |
| Docs consistency | 114237706237 | skipped |

Tally (pr357-ci-752811c-success): 17 rows of the table above with `result` = `success`.

Tally (pr357-ci-752811c-failure): 1 row of the table above with `result` = `failure`.

- `Test (macos-latest / lua54)` (114237705474):
  `e8b_switch_buffer_acceptance::e8b_fr1_the_gpu_reaches_a_dired_buffer_by_its_own_name_and_keeps_d18_s_four`,
  `assertion \`left == right\` failed` at the row's `zzz` band, `left` reading `"switch-buffer: showing
  *dired:/var/folders/36/tjdph2t965j8snz9_vkdnw0r0000gn/T/.tmpZrZlE6/nightowls/proj*"` and `right`
  `"no buffer: zzz"`, and `test result: FAILED. 9 passed; 1 failed; 0 ignored` (48.80 s). No row or issue
  named the selector: **#361, filed, its first occurrence**. Its candidate mechanism, from the report and not
  a run: the dired buffer ranks on its whole name, path included, under E8b's rule, and that temporary path
  spells `zzz` case-folded (`snz9`, `.tmpZrZlE6`). The branch's diff touches neither the ranking nor this row.
- Every test leg pairs each `running` line with a result line: 194 on `Test (crdt)`, 195 on each other
  Ubuntu `test` leg, 196 on `Test (macos-latest / luajit)`, and 193 `ok` with one `FAILED` on the red leg,
  whose doc-test and `pmacs-protocol` steps are skipped after its sweep failed. E8c's suite ran its 17 tests
  and the review's probe suite its 10 on all six legs and passed; this round's process rows passed where
  they run, four on each Linux leg and one, the platform-neutral one, on each macOS leg.

`Grammar fuzz` 38059810023 at `d5327b6`, 38060125825 at `124b955` and 38060487701 at `752811c`, each attempt 1 of 1, `success` on both arms, fuzzing nothing (`run=false`, `no changed path can change the grammar set`).

So, at E8c's fix round 1: R7 moves to thirty-four; #312 to two; #359 is at one, filed by the review; #311's list of runs reads at forty-five; #316 stays at fourteen and #358 at one, their rows passing in every sweep of the round's and the review's gates; #361 is filed at one; #360 is filed by the review and is no red; every other count above stands.

### #360's pull request: `main` green at `ea62bb2`, PR #362 red on #363's and #364's first at `c33b55c`, R7's thirty-fifth and #311's list at forty-six

Recorded 2026-10-10 by #360's pull request, #362 at `c33b55c`, two signed commits on `ea62bb2`:
the state store writes without a sync (`state::write`, `file_io::Durability`), and the witnesses,
with `PMACS_REQUIRE_STRACE` in the gate's arming table. The runs were read from the jobs endpoint
by conclusion and through every test leg's log; none was rerun.

#### `main` at `ea62bb2`, this branch's base

`ea62bb2` is E8c's squash of #357 at `752811c`, merged 18:41:21Z, signature `E`, trailers empty;
its only diff from `752811c` is this file. It is `main`'s last code-bearing commit, so it is this
branch's base control. Its runs were not recorded here before.

`CI` 38076790778 at `ea62bb2`, attempt 1 of 1, `push`, created 18:41:23Z and completed 19:12:55Z:
`success`.

Tally (main-ci-ea62bb2): 19 = 18 + 1.

Tally (main-ci-ea62bb2-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 114285336314 | success |
| Lint (luajit) | 114285336438 | success |
| Changed paths | 114285336446 | success |
| Commit attribution (D9) | 114285336479 | success |
| Format | 114285336672 | success |
| M4 Perf Gates | 114285375861 | success |
| M5 Perf Gates | 114285375880 | success |
| M1 Acceptance Gates | 114285375886 | success |
| M6 Perf Gates | 114285375899 | success |
| Test (crdt) | 114285375910 | success |
| Test (ubuntu-latest / lua54) | 114285375917 | success |
| Test (macos-latest / luajit) | 114285375935 | success |
| Test (macos-latest / lua54) | 114285375945 | success |
| Perf budgets (debug) | 114285376017 | success |
| M10 Perf Gates (crdt) | 114285376038 | success |
| GPU Render (headless) | 114285376040 | success |
| Test (ubuntu-latest / luajit) | 114285376044 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114285376196 | success |
| Docs consistency | 114285377054 | skipped |

Tally (main-ci-ea62bb2-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 194 on `Test (crdt)`, 195 on
  each Ubuntu `test` leg, 196 on each macOS leg; none failed. Each macOS log holds one more line
  matching `running N tests`, the workflow's own `grep` for that pattern echoed into the log, which
  is no test binary.

`Grammar fuzz` 38076790763 at `ea62bb2`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

#### The pull request's gate: `20261010T192148Z-332172` at `c33b55c`

`scripts/gate`, the default plan: `pmacs-protocol` has no diff. Five of six; fmt, clippy, doc,
build and diff-check ok; no `pmacs --daemon` survived (identified by environ).
`PMACS_REQUIRE_STRACE` was armed, the branch's new row, and the branch's two quit rows ran and
passed (`7 passed`, the binary's five shared rows among them). The load average passed 40 during
the sweep, with other sessions' work beside it.

- `05-sweep`: 204 paired result lines, 5,414 passed, 3 failed, 65 ignored:
  - the HLS row with #311's fragments (`Could not find module ‘Prelude’`, the trace reaching
    `LSP:ready·processi`, `test result: FAILED. 4 passed; 1 failed; 0 ignored`, 136.32 s), so
    #311's list gains the gate;
  - `pmacs-gpu`'s `attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`
    at `attach.rs:1971`, `transient sequence must attach: Attach(Handshake(Io(Os { code: 32, kind:
    BrokenPipe`: all three fragments, **R7's thirty-fifth**, its list extended;
  - in the same binary, `tests::overwide_status_runs_never_wrap_and_keep_the_suffix_pinned`,
    `assertion failed: min_y >= band_top && max_y <= height` at `pmacs-gpu/src/main.rs:25303:13`,
    `test result: FAILED. 373 passed; 2 failed; 0 ignored`: the sibling recorded on #304 since
    E7h, #304's own row passing beside it; commented there, not filed apart. `pmacs-gpu` has no
    diff on the branch.

#### `CI` 38079735568 at `c33b55c`, attempt 1 of 1: red on both macOS legs, #363's first and #364's first

`pull_request`, the merge `1e27895` (`c33b55c` into `ea62bb2`, read from the test logs' checkout
line), created 19:25:24Z and completed 19:55:42Z: `failure`.

Tally (pr362-ci-c33b55c): 19 = 16 + 2 + 1.

Tally (pr362-ci-c33b55c-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (luajit) | 114293989543 | success |
| Changed paths | 114293989671 | success |
| Commit attribution (D9) | 114293989701 | success |
| Format | 114293989780 | success |
| Lint (lua54) | 114293989809 | success |
| Perf budgets (debug) | 114294020947 | success |
| GPU Render (headless) | 114294020961 | success |
| M1 Acceptance Gates | 114294020966 | success |
| Test (crdt) | 114294020970 | success |
| Test (ubuntu-latest / luajit) | 114294020989 | success |
| Test (ubuntu-latest / lua54) | 114294020991 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114294020999 | success |
| M10 Perf Gates (crdt) | 114294021003 | success |
| M6 Perf Gates | 114294021007 | success |
| M4 Perf Gates | 114294021036 | success |
| Test (macos-latest / luajit) | 114294021062 | failure |
| Test (macos-latest / lua54) | 114294021065 | failure |
| M5 Perf Gates | 114294021080 | success |
| Docs consistency | 114294021819 | skipped |

Tally (pr362-ci-c33b55c-success): 16 rows of the table above with `result` = `success`.

Tally (pr362-ci-c33b55c-failure): 2 rows of the table above with `result` = `failure`.

- `Test (macos-latest / lua54)` (114294021065):
  `e7c_indicator_acceptance::e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`,
  `quiet before the request`, `left: Some("⋯1 parse rust` against `right: None`, and `test
  result: FAILED. 4 passed; 1 failed; 0 ignored` (7.04 s). No row or open issue named the
  selector: **#363, filed, its first occurrence**. Its candidate mechanism, from reading: #319's,
  on the suite's other indicator row, a rust parse past the threshold in the frame the row reads
  before its request; #321 retired it for the under-threshold row only.
- `Test (macos-latest / luajit)` (114294021062):
  `e7_review1_probes::e7_review1_a_late_answer_past_the_default_bound_never_touches_an_edited_buffer`,
  `the save waits the default bound and gives up before the answer: 1.340401042s`, and `test
  result: FAILED. 9 passed; 1 failed; 0 ignored` (3.62 s). #320 names the selector, but its
  fragments are the second save's `answered in time:` and `10 passed; 1 failed`, so this is not
  #320's occurrence: **#364, filed, its first occurrence**, and #320 commented.
- Both failing rows run in-process, where no state directory is installed, so the branch's change
  never runs in them, and the branch leaves the editor's own saves synced; that is a reading.
- Every test leg pairs each `running` line with a result line: 195 on `Test (crdt)`, 196 on each
  other Ubuntu `test` leg, and 194 `ok` with one `FAILED` on each macOS leg. The branch's PTY row
  passed on all six legs. Its strace row ran and passed on the four Linux legs, where `strace` is
  present, and recorded its skip on both macOS legs.

`Grammar fuzz` 38079735547 at `c33b55c`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

So, at #360's pull request: R7 moves to thirty-five; #311's list of runs reads at forty-six; #363
and #364 are filed at one each; #304's sibling is commented and #304 stands; every other count
above stands.

### #365's pull request: `main` red at `a1f0c27` on #282's seventh, PR #366 green at `5f53b91`, #367 filed, #311's list at forty-seven

Recorded 2026-10-11 by #365's pull request, #366 at `5f53b91`, two signed commits on `a1f0c27`:
quit's recovery copies written before its question and synced after it on a thread, and the
question naming a copy autosave could not write. The runs were read from the jobs endpoint by
conclusion and through every test leg's log; none was rerun.

#### `main` at `a1f0c27`, this branch's base: #282's seventh

`a1f0c27` is #360's squash of #362 at `c33b55c`, merged 13:16:54Z, signature `E`, trailers
empty; its only diff from `c33b55c` is this file. It is `main`'s last code-bearing commit, so it
is this branch's base control. Its runs were not recorded here before.

`CI` 38142552045 at `a1f0c27`, attempt 1 of 1, `push`, created 13:16:57Z and completed 13:47:46Z:
`failure`.

Tally (main-ci-a1f0c27): 19 = 17 + 1 + 1.

Tally (main-ci-a1f0c27-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 114479205050 | success |
| Lint (lua54) | 114479205192 | success |
| Commit attribution (D9) | 114479205200 | success |
| Format | 114479205295 | success |
| Lint (luajit) | 114479205304 | success |
| Test (crdt) | 114479244468 | success |
| GPU Render (headless) | 114479244481 | success |
| Perf budgets (debug) | 114479244488 | success |
| Test (ubuntu-latest / lua54) | 114479244495 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 114479244505 | success |
| Test (macos-latest / lua54) | 114479244524 | success |
| M10 Perf Gates (crdt) | 114479244533 | success |
| M5 Perf Gates | 114479244539 | success |
| Test (macos-latest / luajit) | 114479244541 | success |
| M4 Perf Gates | 114479244552 | success |
| Test (ubuntu-latest / luajit) | 114479244589 | success |
| M6 Perf Gates | 114479244606 | success |
| M1 Acceptance Gates | 114479244962 | success |
| Docs consistency | 114479245822 | skipped |

Tally (main-ci-a1f0c27-success): 17 rows of the table above with `result` = `success`.

Tally (main-ci-a1f0c27-failure): 1 row of the table above with `result` = `failure`.

- `Test (ubuntu-latest / lua54)` 114479244495, `failure`:
  `e7b_review_wire_acceptance::a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`,
  **#282's seventh occurrence, its third on `main`**, all three fragments read against the job's
  own log: `*lsp* names the request and the code: []` at `tests/e7b_review_wire_acceptance.rs:1218`;
  `WIRE *errors* gained ["[lsp] LSP: default-rust refused textDocument/prepareRename as a client
  error, -32602 InvalidParams: No references found at position"]` and `WIRE label "ready",
  last_error None, *lsp* errors []`; and `test result: FAILED. 4 passed; 1 failed; 2 ignored`
  (`running 7 tests`, `finished in 48.13s`). Between the row's stdout header and its panic the
  trace holds 91 `$/progress` frames, 75 of them `rustAnalyzer/cachePriming` reports. It was the
  job's only failure: 194 `running` lines against 193 `test result: ok` and the one `FAILED`.
  Commented on #282 at 15:48:55Z, before this round; not rerun.
- The other five test legs pair every `running` line with `test result: ok`: 195 on `Test
  (crdt)`, 196 on each other Ubuntu leg, 197 on each macOS leg. Both macOS legs passed, #363's and
  #364's rows (`e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`,
  `e7_review1_a_late_answer_past_the_default_bound_never_touches_an_edited_buffer`) `ok` on each,
  so neither sampled.

`Grammar fuzz` 38142551945 at `a1f0c27`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

#### The pull request's gate: `20261011T171633Z-1297485` at `5f53b91`

`scripts/gate`, the default plan: `pmacs-protocol` has no diff. Five of six; fmt, clippy, doc,
build and diff-check ok; no `pmacs --daemon` survived (identified by environ). Every
`PMACS_REQUIRE_*` armed but `PMACS_REQUIRE_PYRIGHT`, `PMACS_REQUIRE_STRACE` among them. Other
sessions' builds ran beside the sweep: `uptime` read 5.30 at its start and 3.84, 10.24, 14.48
(one, five, fifteen minutes) midway.

- `05-sweep`: 205 paired result lines, 5,430 passed, 2 failed, 65 ignored:
  - the HLS row with #311's fragments (`Could not find module ‘Prelude’`, the trace reaching
    `LSP:ready·processi`, `test result: FAILED. 4 passed; 1 failed; 0 ignored`, 127.29 s), so
    #311's list gains the gate;
  - `e7i_parse_containment_acceptance::e7i_the_memory_watch_stops_296_and_says_by_how_much`,
    `#296's worker was stopped by its memory watch:` at
    `tests/e7i_parse_containment_acceptance.rs:691`, the death told as `grew past 64 MiB (by
    watch)` with no `bytes over`, and `test result: FAILED. 28 passed; 1 failed; 0 ignored`
    (7.60 s). No row or issue named the selector: **#367, filed, its first occurrence**. Its
    candidate mechanism, from reading: `death()` takes the overshoot from the unit's stderr sink
    without draining it, the sink E7i review 3's Low 1 named for the crashed layer's name. The
    branch has no diff there.
  - The branch's suites passed armed: `autosave_quit_acceptance` 10, `autosave_acceptance` 32,
    `quit_state_write_acceptance` 7 and E8c's `e8c_quit_acceptance` 17. R7's row, #304's
    sibling, #312's, #316's, #358's and #359's rows passed.

#### `CI` 38159090400 at `5f53b91`, attempt 1 of 1: green

`pull_request`, created 17:18:28Z and completed 17:49:09Z: `success`.

Tally (pr366-ci-5f53b91): 19 = 18 + 1.

Tally (pr366-ci-5f53b91-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 114528048171 | success |
| Lint (luajit) | 114528048307 | success |
| Commit attribution (D9) | 114528048310 | success |
| Changed paths | 114528048325 | success |
| Lint (lua54) | 114528048545 | success |
| M4 Perf Gates | 114528092984 | success |
| Test (crdt) | 114528093005 | success |
| M1 Acceptance Gates | 114528093007 | success |
| M10 Perf Gates (crdt) | 114528093019 | success |
| GPU Render (headless) | 114528093021 | success |
| M6 Perf Gates | 114528093091 | success |
| Test (macos-latest / lua54) | 114528093092 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114528093119 | success |
| Test (ubuntu-latest / lua54) | 114528093126 | success |
| Test (ubuntu-latest / luajit) | 114528093137 | success |
| M5 Perf Gates | 114528093146 | success |
| Test (macos-latest / luajit) | 114528093192 | success |
| Perf budgets (debug) | 114528093218 | success |
| Docs consistency | 114528094105 | skipped |

Tally (pr366-ci-5f53b91-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 196 on `Test (crdt)`, 197
  on each Ubuntu `test` leg, 198 on each macOS leg; none failed. The branch's suite passed 10 of 10
  on all six legs, its PTY rows running on every one; its four strace rows ran on the four Linux
  legs, where `strace` is present, and recorded their skip on both macOS legs, which hold five
  `PMACS_REQUIRE_STRACE unset` skips each with #362's one.

`Grammar fuzz` 38159090174 at `5f53b91`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

So, at #365's pull request: #282 moves to seven, its third on `main`; #311's list of runs reads
at forty-seven; #367 is filed at one; R7 stays at thirty-five, #363 and #364 at one each, their
rows passing; every other count above stands.

### #368's pull request: `main` red at `1448046` on #361's mechanism, PR #369 green at `1f6d0b9`, #311's list at forty-eight

Recorded 2026-10-12 by #368's pull request, #369 at `1f6d0b9`, four signed commits on `1448046`:
the periodic sweep's recovery copies staged as quit's are and installed on one thread, a sweep
falling due meanwhile deferred, and the fake LSP modes moved out of `docs/invariants.md` for the
two rules it gains. The runs were read from the jobs endpoint by conclusion and through every test
leg's log; none was rerun.

#### `main` at `1448046`, this branch's base: #361's mechanism on a second row

`1448046` is #365's squash of #366 at `5f53b91`, signature `E`, trailers empty. It is `main`'s
last code-bearing commit, so it is this branch's base control. Its runs were not recorded here
before.

`CI` 38167796490 at `1448046`, attempt 1 of 1, `push`, created 19:23:16Z and completed 19:53:00Z:
`failure`.

Tally (main-ci-1448046): 19 = 17 + 1 + 1.

Tally (main-ci-1448046-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (luajit) | 114553881608 | success |
| Lint (lua54) | 114553881795 | success |
| Changed paths | 114553881804 | success |
| Format | 114553881839 | success |
| Commit attribution (D9) | 114553881888 | success |
| Test (crdt) | 114553924137 | success |
| M5 Perf Gates | 114553924184 | success |
| M4 Perf Gates | 114553924190 | success |
| Test (macos-latest / lua54) | 114553924215 | success |
| GPU Render (headless) | 114553924229 | success |
| M1 Acceptance Gates | 114553924240 | success |
| Test (ubuntu-latest / lua54) | 114553924249 | success |
| Perf budgets (debug) | 114553924280 | success |
| Test (macos-latest / luajit) | 114553924282 | failure |
| M6 Perf Gates | 114553924316 | success |
| Test (ubuntu-latest / luajit) | 114553924322 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114553924325 | success |
| M10 Perf Gates (crdt) | 114553924344 | success |
| Docs consistency | 114553925258 | skipped |

Tally (main-ci-1448046-success): 17 rows of the table above with `result` = `success`.

Tally (main-ci-1448046-failure): 1 row of the table above with `result` = `failure`.

- `Test (macos-latest / luajit)` 114553924282, `failure`:
  `minibuffer_accept_acceptance::switch_buffer_says_which_buffer_ret_reached`, its fragments read
  against the job's own log: `assertion \`left == right\` failed: the refusal is as it was` at
  `tests/minibuffer_accept_acceptance.rs:542:5`, `left: "switch-buffer: showing
  /var/folders/36/tjdph2t965j8snz9_vkdnw0r0000gn/T/.tmpOLzChZ/bin-tests/a.rs"`, `right: "no buffer:
  zzz"`, and `test result: FAILED. 8 passed; 1 failed; 0 ignored` (`running 9 tests`). No row or
  issue named the selector. The message itself shows **#361's mechanism on a second row**: the
  buffer is named by its path as given, the switcher matches a case-folded subsequence, and the
  path spells `zzz` (the `z` of `snz9` in this runner's fixed temporary root, the root of #361's
  first occurrence, and the `z` and `Z` of the random `.tmpOLzChZ`), so the row's last band read a
  switch to `a.rs` instead of the refusal. #361 is one issue per mechanism and its signature names
  another selector, so this was commented on #361 (issuecomment-6113638824) for the owner to rule
  whether it stands there or alone; not rerun. The job paired 196 `running` lines with 195 `test
  result: ok` and this one `FAILED`.
- The other five test legs pair every `running` line with `test result: ok`: 196 on `Test (crdt)`,
  197 on each Ubuntu leg, 198 on `Test (macos-latest / lua54)`. The row passed on all five. #282's
  row (`e7b_review_wire_acceptance::a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`)
  passed on all six legs, its Ubuntu `lua54` leg among them, so #282 did not sample.

`Grammar fuzz` 38167796482 at `1448046`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

#### The pull request's gate: `20261011T214718Z-1799868` at `1f6d0b9`

`scripts/gate`, the default plan: `pmacs-protocol` has no diff. Five of six; fmt, clippy, doc,
build and diff-check ok; no `pmacs --daemon` survived (identified by environ). Every
`PMACS_REQUIRE_*` armed but `PMACS_REQUIRE_PYRIGHT`, `PMACS_REQUIRE_STRACE` among them. `uptime`
read 0.79, 4.17, 5.67 (one, five, fifteen minutes) twelve minutes into the run.

- `05-sweep`: 206 `running` lines, 205 `test result: ok` and one `FAILED`; 5,446 passed, 1 failed,
  65 ignored:
  - the HLS row with #311's fragments (`Could not find module ‘Prelude’`, the trace reaching
    `LSP:ready·processi`, `test result: FAILED. 4 passed; 1 failed; 0 ignored`, 125.87 s), so
    #311's list gains the gate.
  - The branch's suites passed armed: `autosave_periodic_acceptance` 12 (seven rows and the five
    shared readiness rows), `autosave_acceptance` 35 and `autosave_quit_acceptance` 10.
    `docs_consistency` passed 18. #367's row, #361's two rows (the dired one and the one above),
    #282's row, #304's sibling (`pmacs-gpu`, `375 passed`) and R7's row
    (`managed_retry_survives_transients_and_uses_the_successful_stream`) passed; `BrokenPipe`
    appears nowhere in the log.

#### `CI` 38177767878 at `1f6d0b9`, attempt 1 of 1: green

`pull_request`, created 21:46:46Z and completed 22:19:31Z: `success`.

Tally (pr369-ci-1f6d0b9): 19 = 18 + 1.

Tally (pr369-ci-1f6d0b9-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 114583679192 | success |
| Lint (luajit) | 114583679334 | success |
| Lint (lua54) | 114583679348 | success |
| Commit attribution (D9) | 114583679408 | success |
| Format | 114583679448 | success |
| M6 Perf Gates | 114583708687 | success |
| GPU Render (headless) | 114583708706 | success |
| M1 Acceptance Gates | 114583708708 | success |
| Perf budgets (debug) | 114583708712 | success |
| M4 Perf Gates | 114583708720 | success |
| Test (crdt) | 114583708745 | success |
| Test (ubuntu-latest / luajit) | 114583708775 | success |
| Test (macos-latest / luajit) | 114583708778 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114583708781 | success |
| M5 Perf Gates | 114583708790 | success |
| M10 Perf Gates (crdt) | 114583708797 | success |
| Test (macos-latest / lua54) | 114583708816 | success |
| Test (ubuntu-latest / lua54) | 114583708833 | success |
| Docs consistency | 114583709736 | skipped |

Tally (pr369-ci-1f6d0b9-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 197 on `Test (crdt)`, 198 on
  each Ubuntu `test` leg, 199 on each macOS leg; none failed. The branch's new suite passed 12 of
  12 on all six legs: on the four Linux legs its seven `strace` rows ran (139 s a leg), and on both
  macOS legs they recorded their skip, seven `PMACS_REQUIRE_STRACE unset` lines each (0.27 and
  0.42 s). `autosave_acceptance` passed 35 and `autosave_quit_acceptance` 10 on every leg, paired
  by each binary's own `running` line, since on macOS a `Running` line prints before the
  previous binary's result. #361's two rows and #282's row passed on every leg.

`Grammar fuzz` 38177767847 at `1f6d0b9`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

So, at #368's pull request: #361's mechanism is seen on a second row, on `main`, commented on #361
and not filed, so no count here moves for it; #311's list of runs reads at forty-eight; R7 stays
at thirty-five, #282 at seven, #367 at one, their rows passing; every other count above stands.

### #368's pull request, addendum: the head at `19a163f`, #307's fourth, #311's list at forty-nine

Recorded 2026-10-12 by #368's pull request. After the section above was pushed, two commits moved
#369's head from `1f6d0b9`: `28a28b1` and `19a163f`, each dropping from a comment a cause the
session had asserted and not read (a sync failing on a full btrfs disk). Neither changes code. The
runs were read from the jobs endpoint by conclusion and through every test leg's log; none was
rerun.

- `CI` 38180571858 at `28a28b1`, attempt 1 of 1, created 22:28:42Z and completed 22:30:27Z:
  `cancelled`, superseded by the push of `19a163f`, so it carries no verdict.
  `Grammar fuzz` 38180571856 at `28a28b1`, `success`.

Tally (pr369-ci-28a28b1): 19 = 4 + 14 + 1.

- `20261011T222856Z-1929257` at `28a28b1`: stopped by the session in its sweep, once the next
  commit was due. It printed `05-sweep FAILED` because it was stopped; it is no result, and not in
  #311's list.
- `20261011T223022Z-1939930` at `19a163f`, the default plan: five of six; fmt, clippy, doc,
  build and diff-check ok; no `pmacs --daemon` survived (identified by environ);
  `PMACS_REQUIRE_STRACE` armed. `05-sweep`: 206 `running` lines, 205 `test result: ok` and one
  `FAILED`; 5,446 passed, 1 failed, 65 ignored. The failure is the HLS row with #311's fragments
  (`Could not find module ‘Prelude’`, the trace reaching `LSP:ready·processi`, `test result:
  FAILED. 4 passed; 1 failed; 0 ignored`, 125.94 s), so #311's list gains the gate. The branch's
  suites passed (12, 35, 10), `docs_consistency` 18, and #367's, #361's two, #282's, R7's and
  #304's sibling's rows.

#### `CI` 38180663248 at `19a163f`, attempt 1 of 1: #307's fourth

`pull_request`, created 22:30:05Z and completed 23:02:56Z: `failure`.

Tally (pr369-ci-19a163f): 19 = 17 + 1 + 1.

Tally (pr369-ci-19a163f-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 114592421086 | success |
| Format | 114592421290 | success |
| Commit attribution (D9) | 114592421337 | success |
| Lint (luajit) | 114592421379 | success |
| Lint (lua54) | 114592421399 | success |
| M1 Acceptance Gates | 114592457604 | success |
| M4 Perf Gates | 114592457619 | success |
| Test (crdt) | 114592457626 | success |
| GPU Render (headless) | 114592457643 | success |
| M5 Perf Gates | 114592457664 | success |
| Test (macos-latest / lua54) | 114592457696 | success |
| M6 Perf Gates | 114592457727 | success |
| Test (macos-latest / luajit) | 114592457729 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 114592457732 | success |
| M10 Perf Gates (crdt) | 114592457737 | success |
| Test (ubuntu-latest / luajit) | 114592457749 | success |
| Test (ubuntu-latest / lua54) | 114592457774 | success |
| Perf budgets (debug) | 114592458005 | success |
| Docs consistency | 114592458972 | skipped |

Tally (pr369-ci-19a163f-success): 17 rows of the table above with `result` = `success`.

Tally (pr369-ci-19a163f-failure): 1 row of the table above with `result` = `failure`.

- `Test (macos-latest / luajit)` 114592457729, `failure`:
  `e7b_review_wire_acceptance::a_save_sends_did_save_and_rust_analyzer_flychecks_on_it`, all
  three of #307's fragments read against the job's own log (`the check showed as a suffix on
  ready, or as the tracker's busy title under a reload` at `tests/e7b_review_wire_acceptance.rs:692`;
  the labels `["4 = LSP:ready"]` with the busy field `["4 b -"]`; `test result: FAILED. 4 passed;
  1 failed; 2 ignored`): **#307's fourth**. The wire logs the flycheck's `begin (cargo check)`,
  two `report`s and `end` all at 237 ms after the save. Commented on #307
  (issuecomment-6120758409); not rerun. The job paired 197 `running` lines with 196 `test
  result: ok` and this one `FAILED`. The branch has no diff in `src/lsp.rs`, `src/lsp_status.rs`,
  `builtin/runtime/lsp.lua` or the suite, an argument from untouched code.
- The other five test legs pair every `running` line with `test result: ok`: 197 on `Test
  (crdt)`, 198 on each Ubuntu leg, 199 on `Test (macos-latest / lua54)`, and passed #307's row.
  The branch's new suite passed on all six legs, its seven `strace` rows running on the four
  Linux legs and recording their skip on both macOS legs.

`Grammar fuzz` 38180663237 at `19a163f`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

So, at #368's pull request's head: #307 moves to four; #311's list of runs reads at forty-nine;
every other count above stands.

### The wall-clock classification pass: `main` green at `824ca82`, #369's rerun green, R7's thirty-sixth and #311's list at fifty

Recorded 2026-10-12 by the wall-clock classification pass, PR #370 at `968b25f`, an empty commit on
`824ca82`: the pass changes no code. The runs were read from the jobs endpoint by conclusion and
through every test leg's log; this pass reran none.

#### `main` at `824ca82`, the squash of #369: green

`824ca82` is #368's squash of #369 at `19a163f`, merged 09:35:17Z, signature `E`, trailers empty;
its only diff from `19a163f` is `docs/ci-red-signatures.md`, the registry commits `main` gained, so
the squash is faithful.

`CI` 38230858125 at `824ca82`, attempt 1 of 1, `push`, created 09:35:19Z and completed 10:08:31Z:
`success`.

Tally (main-ci-824ca82): 19 = 18 + 1.

Tally (main-ci-824ca82-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 114749690522 | success |
| Lint (lua54) | 114749690911 | success |
| Lint (luajit) | 114749690980 | success |
| Format | 114749690981 | success |
| Changed paths | 114749691142 | success |
| Test (crdt) | 114749762183 | success |
| M4 Perf Gates | 114749762265 | success |
| M5 Perf Gates | 114749762304 | success |
| M1 Acceptance Gates | 114749762320 | success |
| Test (ubuntu-latest / lua54) | 114749762325 | success |
| GPU Render (headless) | 114749762349 | success |
| M10 Perf Gates (crdt) | 114749762350 | success |
| M6 Perf Gates | 114749762360 | success |
| Test (macos-latest / luajit) | 114749762381 | success |
| Perf budgets (debug) | 114749762386 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114749762398 | success |
| Test (ubuntu-latest / luajit) | 114749762418 | success |
| Test (macos-latest / lua54) | 114749762474 | success |
| Docs consistency | 114749764398 | skipped |

Tally (main-ci-824ca82-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 197 on `Test (crdt)`, 198 on
  each Ubuntu `test` leg, 199 on each macOS leg. #307's row
  (`a_save_sends_did_save_and_rust_analyzer_flychecks_on_it`), #282's row, #367's row and #361's
  minibuffer row passed on all six; #361's dired row passed on the five that compile it.

`Grammar fuzz` 38230858128 at `824ca82`, attempt 1 of 1, `success`, fuzzing nothing on either arm
(`run=false`, `no changed path can change the grammar set`).

#### #369's `CI` 38180663248, attempt 2: `Test (macos-latest / luajit)` rerun alone, green

After the section above recorded attempt 1, the owner reran that one job to clear the required
check. Attempt 2 re-executed it alone as job 114737793130, 09:02:18Z to 09:34:34Z, `success`; the
other eighteen rows of attempt 2 carry attempt 1's results and timestamps under new ids. The run's
conclusion now reads `success`.

Tally (pr369-ci-19a163f-attempt2): 19 = 18 + 1.

Tally (pr369-ci-19a163f-attempt2-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Test (macos-latest / luajit) | 114737793130 | success |
| Format | 114737793138 | success |
| Commit attribution (D9) | 114737793372 | success |
| Changed paths | 114737793858 | success |
| Test (crdt) | 114737793908 | success |
| Test (macos-latest / lua54) | 114737794010 | success |
| M5 Perf Gates | 114737794038 | success |
| Docs consistency | 114737794180 | skipped |
| Test (ubuntu-latest / lua54) | 114737794223 | success |
| Perf budgets (debug) | 114737794281 | success |
| M1 Acceptance Gates | 114737794318 | success |
| M4 Perf Gates | 114737794324 | success |
| Test (ubuntu-latest / luajit) | 114737794352 | success |
| M10 Perf Gates (crdt) | 114737794494 | success |
| Test (ubuntu-latest / luajit, no crdt) | 114737794725 | success |
| M6 Perf Gates | 114737794774 | success |
| Lint (luajit) | 114737836299 | success |
| Lint (lua54) | 114737838962 | success |
| GPU Render (headless) | 114737847645 | success |

Tally (pr369-ci-19a163f-attempt2-success): 18 rows of the table above with `result` = `success`.

- The rerun job paired 199 `running` lines with 199 `test result: ok`, and #307's row passed in it
  (`test a_save_sends_did_save_and_rust_analyzer_flychecks_on_it ... ok` at 09:15:07Z).
- **Attempt 2 is non-reproduction and nothing more.** A rerun re-executes the same merge, so it is
  recorded beside attempt 1 rather than in place of it: attempt 1's job 114592457729 stays #307's
  fourth occurrence, and #307 stays at four. What argues that #369 did not cause it is still
  `1f6d0b9`'s green `CI` 38177767878. That run is on code that differs from `19a163f`'s in two
  comments only, a Lua comment in `builtin/runtime/autosave.lua` and a doc comment in
  `tests/autosave_periodic_acceptance.rs` (read from `git diff 1f6d0b9 19a163f`).

#### The pass's gate: `20261012T170517Z-2804859` at `824ca82`

`scripts/gate`, the default plan, on `824ca82`'s tree after the pass's instrumentation was
reverted. Five of six; fmt, clippy, doc, build and diff-check ok; no `pmacs --daemon` survived
(identified by environ). Every `PMACS_REQUIRE_*` armed but `PMACS_REQUIRE_PYRIGHT`. `uptime` read
69.73, 48.67, 27.33 four minutes into the run, its sweep running, which is the sweep's own load.

- `05-sweep`: 206 `running` lines, 205 `test result: ok` and one `FAILED`; 5,446 passed, 1 failed,
  65 ignored. The failure is the HLS row with #311's fragments (`Could not find module ‘Prelude’`,
  the trace reaching `LSP:ready·processi`, `test result: FAILED. 4 passed; 1 failed; 0 ignored`,
  126.16 s), so #311's list gains the gate. R7's row, #361's two rows, #307's, #282's and #367's
  rows, `pmacs-gpu` (`375 passed`, #304's sibling among them) and
  `gate_script_acceptance::every_budget_in_the_list_is_an_ignored_test_in_the_tree` passed.

#### Samples outside the gate: the pass's measurement sweeps

The pass measured in runs of its own, each test binary run directly in the gate's environment (its
target dir, ambient roots, a short `TMPDIR` under `~/build/pmacs-gate-targets/tmp`, the sccache
shim, every armed `PMACS_REQUIRE_*`). They are not gates, so #311's list does not gain them; their
logs are archived with the pass (`~/build/wall-clock/wall-clock-20261012.tar.zst`).

- The first, uninstrumented, with libtest's `--report-time`, 14:44:44Z to 14:56:58Z, the load
  53.78 at its start: `pmacs-gpu`'s `attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`
  failed with all three of R7's fragments (`transient sequence must attach`,
  `Handshake(Io(Os { code: 32, kind: BrokenPipe`, `attach.rs:1971`), `test result: FAILED. 374
  passed; 1 failed`: **R7's thirty-sixth**. In the same run
  `e7i_review1_the_aliasing_guard_reaches_the_worker_s_build` failed on `E0514`, a `cc` compiled
  by rustc 1.99.0. That red is the instrument's: a binary run outside cargo inherits no
  `RUSTUP_TOOLCHAIN`, so its nested `cargo check` took the default toolchain. Every later sweep
  set it, and the row passed in all three. It is no signature.
- The sweep beside a looping 2 GB `fdatasync` writer, 16:51:21Z to 17:03:59Z:
  `e8b_own_name_acceptance::c_x_k_keeps_d18_s_four` failed at
  `tests/e8b_own_name_acceptance.rs:627:5` with `premise: nothing matches \`zzz\``, under a run
  root `/home/jeans/build/pmacs-gate-targets/tmp/ZZVGWa` that spells two of the three. It is
  #361's mechanism on a third row, after the selector it was filed on and
  `minibuffer_accept_acceptance::switch_buffer_says_which_buffer_ret_reached`. Commented on #361,
  not filed, and no count moves.
- The writer was the brief's instrument for disk-bound rows, so the reds it induced are the
  classification's measurements, not occurrences, and no count moves for them:
  - `e7_format_on_save_acceptance`'s `e7_3_a_stalled_formatter_saves_unformatted_within_the_bound`
    and `e7_3_a_late_answer_is_never_applied`;
  - `e7_review1_probes`'s `e7_review1_a_late_answer_past_the_default_bound_never_touches_an_edited_buffer`
    (#364's and #320's selector) and `e7_review1_a_server_killed_during_the_wait_saves_unformatted_and_says_so`;
  - `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`.

  Their values are in the pull request's table.

So, at the wall-clock classification pass: R7 moves to thirty-six; #311's list of runs reads at
fifty; #307 stays at four, its rerun recorded beside it; #361's mechanism is seen on a third row and
commented; every other count above stands.

### E9's close: PR #378's runs, `main` at `587de75`, `53d116c` and `1ff2603`, #306 at three, #316 at sixteen, #373 to #375, #381 and #382 filed, and the local samples E8d, E8e and E9 left, listed

Recorded 2026-10-20 at E9's close, on `main` at `1ff2603`, as D38 asks of every close: the
first registry commit since `de4177d`. E8d (#372), E8e (#376) and E9 (#378) merged without one,
so their runs, their CI occurrences and the samples their gates met are recorded here. Every run
was read from the jobs endpoint by conclusion, and every test leg of every code-bearing run named
below through its own log, fetched with escape sequences allowed and stripped, each `running`
line paired with its own result line. Every gate was read from its log directory under
`~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/`. Every run id, job id and gate log
below was searched for in this file before it was written, and none was here. Nothing was rerun.

#### How D38 applies here: the owner's ruling of 2026-10-20

D38 (the owner's ruling of 2026-10-18) says a row's tally counts CI alone, local gate samples are
listed on the issue and in the pass and never tallied, and the stale counts E8d, E8e and E9 left
(R7, #264, #311, #306, #375) are recorded at E9's close. On 2026-10-20 the owner ruled how that
applies to the samples met before 2026-10-18: strictly. No local gate sample is counted, whatever
its date. So every count here moves by CI alone, and every local sample E8d, E8e and E9 met is
listed in a block no tally counts: in its own row's untallied block for R7, #264, #291 and #311's
list, and in this section's two blocks below, split at the 2026-10-18 ruling only so that a reader
can find each by its pass. This section was first written by `5a0998b`, which counted the samples
met before 2026-10-18; it is corrected in place under the ruling.

#### `main` between `de4177d` and `587de75`, and the wall-clock pass's own runs

Read by conclusion only; none changes code, and each `Grammar fuzz` beside them fuzzed nothing:

- `de4177d`, the wall-clock pass's registry commit: `CI` 38284167116, `push`, 17:26:02Z to
  17:28:12Z, `success`, every test leg skipped; `Grammar fuzz` 38284167192 `success`.
- PR #370, the pass's own pull request, closed unmerged. Its run at `968b25f`, an empty commit
  on `824ca82` that the pass's section could not name, is `CI` 38283856906, 17:23:15Z to
  17:55:13Z, `success`, the whole matrix run; `Grammar fuzz` 38283856876 `success`. At its last
  head `12902e6`, which adds the audit, `CI` 38387606664, `success`, every test leg skipped;
  `Grammar fuzz` 38387606629 `success`.
- `2440089`, the audit landed on `main` (signature `G`, trailers empty; its diff from `12902e6`
  is `de4177d`'s to this file, byte for byte): `CI` 38388301224, `push`, 11:09:42Z to 11:12:17Z,
  `success`, every test leg skipped; `Grammar fuzz` 38388301247 `success`. It is E8d's base.

Tally (main-ci-de4177d): 15 = 6 + 9.

Tally (pr370-ci-968b25f): 19 = 18 + 1.

Tally (pr370-ci-12902e6): 15 = 6 + 9.

Tally (main-ci-2440089): 15 = 6 + 9.

#### E8d: PR #372's four runs

PR #372, `e8d/drift-fixes` on `2440089`. Each `CI` run tested the merge into `2440089` (read from
the test legs' checkout line), each beside a `Grammar fuzz` run that was `success` and fuzzed
nothing (`run=false`, `no changed path can change the grammar set`): 38558729654, 38564925128,
38598124029 and 38612033321.

- `CI` 38558729706 at `e4a0cb8` (the merge `c97e7d2`), 11:44:41Z to 12:19:01Z, `failure`. Both
  macOS legs, `Test (macos-latest / luajit)` 115834305472 and `Test (macos-latest / lua54)`
  115834305507, fail the branch's own E8d.4 row,
  `e8d_drift_acceptance::daemon_rows::e8d_4_a_frontend_dropped_by_a_failed_write_is_detached_exactly_once`
  (`` `frontend.detached` ran once for 3 (log "") ``, `left: 0`, `right: 1`, `test result:
  FAILED. 13 passed; 1 failed; 0 ignored`), 198 `running` lines paired with 198 result lines on
  each: macOS let the write to a half-closed peer succeed, E8d.4's own finding, which `d628b21`
  answered by compiling the row on Linux alone. No signature.
- `CI` 38564925127 at `d628b21` (the merge `abd74cb`), 12:35:06Z to 13:09:49Z, `failure`:
  - `Test (macos-latest / lua54)` 115855152261:
    `minibuffer_accept_acceptance::switch_buffer_says_which_buffer_ret_reached`, `the refusal is
    as it was`, `left: "switch-buffer: showing
    /var/folders/36/tjdph2t965j8snz9_vkdnw0r0000gn/T/.tmpSqVzvz/bin-tests/a.rs"`, `right: "no
    buffer: zzz"`, `test result: FAILED. 8 passed; 1 failed; 0 ignored`, 198 paired: #361's
    mechanism on its second row, in CI, commented there (issuecomment-6169967942). Under the
    owner's calls on #361 a second row's sample moves no count.
  - `Test (ubuntu-latest / luajit)` 115855152447:
    `process::tests::release_joins_every_polling_reader_whatever_the_deadline` at
    `src/process.rs:5482:9`, `only the blocking reader on the master is left; the parser is
    joined`, `left: 0`, `right: 1`, the lib's `test result: FAILED. 2292 passed; 1 failed; 13
    ignored`, 197 paired: **#373, filed, its first occurrence**. No row or issue named the
    selector, and the branch has no diff in `src/process.rs`.
- `CI` 38598123751 at `22de659` (the merge `697de76`), 16:26:43Z to 17:00:52Z, `success`; every
  test leg pairs each `running` line with a `test result: ok`, 199 on `Test (crdt)`, 200 on each
  Ubuntu leg and 201 on each macOS leg.
- `CI` 38612033222 at `7ef32d1` (the merge `a5b855b`), 18:04:20Z to 18:37:39Z, `failure`:
  `Test (ubuntu-latest / lua54)` 116017669722 fails `e8d_drift_acceptance`'s copy of
  `common::ready::tests::readiness_is_a_served_hello_not_a_connect` alone at
  `tests/common/ready.rs:423:9`, `a daemon serving on /tmp/.tmpkuAfvS/late.sock`, `deadline:
  10s`, `polls: 472`, `last: "connect: Connection refused (os error 111)"`, `test result:
  FAILED. 13 passed; 1 failed; 0 ignored` (13.70 s), 199 paired, the self-test `ok` in 65 other
  copies in the job: **#374, filed, its first occurrence**. It is #332's fragment group in
  another suite's copy on another leg, and #374 leaves whether they share a mechanism open.

Tally (pr372-ci-e4a0cb8): 19 = 16 + 2 + 1.

Tally (pr372-ci-d628b21): 19 = 16 + 2 + 1.

Tally (pr372-ci-22de659): 19 = 18 + 1.

Tally (pr372-ci-7ef32d1): 19 = 17 + 1 + 1.

#### `main` at `587de75`, E8d's squash: #306's third and #375's first

`587de75` is the owner's squash of #372 at `7ef32d1`, taken with the required-check bypass the
owner's second pass over E8d ruled; signature `E`, trailers empty, and an empty diff from
`7ef32d1`. The E8e session read this run and commented and filed what it found, but no registry
commit recorded it.

`CI` 38726221304 at `587de75`, attempt 1 of 1, `push`, created 11:17:01Z and completed
11:51:24Z: `failure`.

Tally (main-ci-587de75): 19 = 16 + 2 + 1.

Tally (main-ci-587de75-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 116392628984 | success |
| Changed paths | 116392629369 | success |
| Lint (luajit) | 116392629454 | success |
| Lint (lua54) | 116392629525 | success |
| Commit attribution (D9) | 116392629616 | success |
| M4 Perf Gates | 116392709593 | success |
| M10 Perf Gates (crdt) | 116392709598 | success |
| Perf budgets (debug) | 116392709637 | success |
| Test (crdt) | 116392709650 | success |
| Test (macos-latest / luajit) | 116392709652 | failure |
| M6 Perf Gates | 116392709655 | success |
| GPU Render (headless) | 116392709769 | success |
| M1 Acceptance Gates | 116392709833 | success |
| Test (ubuntu-latest / luajit, no crdt) | 116392709864 | success |
| Test (ubuntu-latest / luajit) | 116392709883 | failure |
| Test (ubuntu-latest / lua54) | 116392709939 | success |
| M5 Perf Gates | 116392709960 | success |
| Test (macos-latest / lua54) | 116392710939 | success |
| Docs consistency | 116392711909 | skipped |

Tally (main-ci-587de75-success): 16 rows of the table above with `result` = `success`.

Tally (main-ci-587de75-failure): 2 rows of the table above with `result` = `failure`.

- `Test (macos-latest / luajit)` 116392709652:
  `compile_mode_acceptance::acc05_kill_reaps_backgrounded_descendant` at
  `tests/compile_mode_acceptance.rs:446:5`, `kill must produce a signaled exit marker; buffer:`,
  the buffer holding the command, its `Directory:` line and `/bin/sh: line 1: 22492 Terminated:
  15          sleep 30` with no `[compile killed by …]` marker but `[compile exited with code
  0]`, and `test result: FAILED. 74 passed; 1 failed; 0 ignored` (26.33 s): all three of #306's
  fragments, **#306's third occurrence**, its first on `main`, commented there
  (issuecomment-6192006145). 200 paired, 199 `ok`.
- `Test (ubuntu-latest / luajit)` 116392709883:
  `autosave_periodic_acceptance::typing_goes_on_while_a_periodic_copy_syncs` at
  `tests/autosave_periodic_acceptance.rs:551:5`, `the editor painted the key before the copy's
  sync returned`, with the strace beside it, and `test result: FAILED. 11 passed; 1 failed; 0
  ignored` (138.98 s): **#375, filed by the E8e session, its first occurrence**. No row or issue
  named the selector. 199 paired, 198 `ok`.
- Each red leg's doc-test and protocol steps were skipped after its test step failed (read from
  the jobs' steps), which is why it pairs fewer lines than its sibling. The other four test legs
  pair each `running` line with a `test result: ok`: 200 on `Test (crdt)`, 201 on each other
  Ubuntu leg, 202 on `Test (macos-latest / lua54)`; #306's row and #375's passed on each.

`Grammar fuzz` 38726221309 at `587de75`, attempt 1 of 1, `success`, fuzzing nothing on either
arm (`run=false`).

#### E8e: PR #376's run and `main` at `53d116c`, E9's base control

- PR #376, `e8e/common-editor` on `587de75`, at `b73abbe` (the merge `9d39b26`): `CI`
  38778897281, 17:37:34Z to 18:12:20Z, `success`; every test leg pairs each `running` line with a
  `test result: ok`, 201 on `Test (crdt)`, 202 on each Ubuntu leg, 203 on each macOS leg.
  `Grammar fuzz` 38778897445 `success`, `run=false`.

Tally (pr376-ci-b73abbe): 19 = 18 + 1.

`53d116c` is the owner's squash of #376 at `b73abbe`, signature `E`, trailers empty, an empty diff
from `b73abbe`. It is `main`'s last code-bearing commit before E9, so it is E9's base control.

`CI` 38796169715 at `53d116c`, attempt 1 of 1, `push`, created 19:43:10Z and completed
20:18:15Z: `success`.

Tally (main-ci-53d116c): 19 = 18 + 1.

Tally (main-ci-53d116c-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 116633016431 | success |
| Lint (luajit) | 116633016775 | success |
| Commit attribution (D9) | 116633016826 | success |
| Format | 116633016906 | success |
| Changed paths | 116633017018 | success |
| GPU Render (headless) | 116633103669 | success |
| M4 Perf Gates | 116633103677 | success |
| M5 Perf Gates | 116633103690 | success |
| M10 Perf Gates (crdt) | 116633103700 | success |
| Perf budgets (debug) | 116633103711 | success |
| M1 Acceptance Gates | 116633103716 | success |
| M6 Perf Gates | 116633103879 | success |
| Test (crdt) | 116633103924 | success |
| Test (ubuntu-latest / luajit, no crdt) | 116633103968 | success |
| Test (macos-latest / lua54) | 116633104038 | success |
| Test (ubuntu-latest / luajit) | 116633104088 | success |
| Test (macos-latest / luajit) | 116633104155 | success |
| Test (ubuntu-latest / lua54) | 116633104477 | success |
| Docs consistency | 116633105746 | skipped |

Tally (main-ci-53d116c-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 201 on `Test (crdt)`, 202 on
  each Ubuntu leg, 203 on each macOS leg. #306's, #373's and #375's rows passed on all six, and
  #361's two rows on every leg that compiles them.

`Grammar fuzz` 38796169391 at `53d116c`, attempt 1 of 1, `success`, `run=false`.

#### E9: PR #378's twelve runs

PR #378, `e9/session-identity` on `53d116c`. Each `CI` run tested the merge into `53d116c`, read
from the test legs' checkout line. Six `CI` runs, three of them red, each red job below with its
fragments read against its own log; six `Grammar fuzz` runs, all `success`, fuzzing nothing on
either arm (`run=false`, `no changed path can change the grammar set`): 39054498396 at
`e7c8fd5`, 39138844511 at `e04aa9a`, 39291657288 at `9024c22`, 39299214633 at `f991f7b`,
39322015544 at `3522b2f` and 39425693709 at `88ce655`.

Tally (pr378-fuzz-runs): 6 items in the list below.

- `Grammar fuzz` 39054498396 at `e7c8fd5`: `asan-strict` 117459590285, `ubsan` 117459590671.
- `Grammar fuzz` 39138844511 at `e04aa9a`: `asan-strict` 117708863143, `ubsan` 117708863437.
- `Grammar fuzz` 39291657288 at `9024c22`: `ubsan` 118193556765, `asan-strict` 118193557127.
- `Grammar fuzz` 39299214633 at `f991f7b`: `ubsan` 118219649852, `asan-strict` 118219650327.
- `Grammar fuzz` 39322015544 at `3522b2f`: `asan-strict` 118298166892, `ubsan` 118298167304.
- `Grammar fuzz` 39425693709 at `88ce655`: `asan-strict` 118637558059, `ubsan` 118637558436.

##### `CI` 39054498392 at `e7c8fd5`, attempt 1 of 1: #316's fifteenth, #363's second, and the branch's E9.4 row on macOS

`pull_request`, the merge `94fbce6`, created 17:36:27Z and completed 18:08:57Z: `failure`.

Tally (pr378-ci-e7c8fd5): 19 = 15 + 3 + 1.

Tally (pr378-ci-e7c8fd5-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (luajit) | 117459590442 | success |
| Changed paths | 117459590599 | success |
| Format | 117459590619 | success |
| Commit attribution (D9) | 117459590715 | success |
| Lint (lua54) | 117459590738 | success |
| M1 Acceptance Gates | 117459636329 | success |
| Test (crdt) | 117459636346 | failure |
| GPU Render (headless) | 117459636349 | success |
| M6 Perf Gates | 117459636356 | success |
| M10 Perf Gates (crdt) | 117459636361 | success |
| Perf budgets (debug) | 117459636362 | success |
| Test (ubuntu-latest / lua54) | 117459636366 | success |
| M5 Perf Gates | 117459636371 | success |
| M4 Perf Gates | 117459636388 | success |
| Test (ubuntu-latest / luajit, no crdt) | 117459636426 | success |
| Test (ubuntu-latest / luajit) | 117459636432 | success |
| Test (macos-latest / luajit) | 117459636470 | failure |
| Test (macos-latest / lua54) | 117459636488 | failure |
| Docs consistency | 117459637612 | skipped |

Tally (pr378-ci-e7c8fd5-success): 15 rows of the table above with `result` = `success`.

Tally (pr378-ci-e7c8fd5-failure): 3 rows of the table above with `result` = `failure`.

- `Test (crdt)` 117459636346:
  `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
  at `tests/e7i_review3_probes.rs:865:9`, `kill 3: nothing held the switch's follow-up parse,
  which started a worker at once and installed`, the report `deaths=3 crashes=0 busy=true
  held=false pending=0 stopped=false tree=true unit=pid_26803 death=killed, " +*
  /tmp/.tmpA2D1nB/a.rs  (rust) … parse:killed !3  L1:C1 Top"`, and `test result: FAILED. 10
  passed; 1 failed; 0 ignored` (18.00 s): the loop's third kill and a modified buffer, so its
  literal fragments do not match and it is **#316's fifteenth, under the template reading** its
  third, sixth and twelfth were counted by; its first on this leg, commented there
  (issuecomment-6250286129). 201 paired, 200 `ok`.
- `Test (macos-latest / luajit)` 117459636470 and `Test (macos-latest / lua54)` 117459636488:
  the branch's own E9.4 row, `e9_session_acceptance::e9_4_a_stale_socket_is_removed_and_a_non_socket_left_alone`
  at `tests/e9_session_acceptance.rs:945:5`, `pmacs: Socket operation on non-socket (os error
  38)`, `test result: FAILED. 18 passed; 1 failed; 0 ignored`: macOS's `connect(2)` returns
  `ENOTSOCK` for a path that is not a socket, which the product did not classify. Review 1's
  High 1, fixed at `0b5e77a`. No signature.
- `Test (macos-latest / lua54)` 117459636488 also fails
  `e7c_indicator_acceptance::e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`
  at `tests/e7c_indicator_acceptance.rs:694:5`, `quiet before the request`, `left: Some("⋯1
  parse rust                   ")`, `right: None`, `test result: FAILED. 4 passed; 1 failed; 0
  ignored` (8.13 s): all three of #363's fragments, **#363's second occurrence**, commented there.
  The macOS luajit leg paired 202 with 201 `ok`; the lua54 leg 202 with 200 `ok`.
- The three other test legs pair each `running` line with a `test result: ok`, 203 on each.
  `WouldBlock` appears in no test leg's log.

##### `CI` 39138844515 at `e04aa9a`, attempt 1 of 1: green

`pull_request`, the merge `aab1a99`, created 15:04:43Z and completed 15:38:19Z: `success`.

Tally (pr378-ci-e04aa9a): 19 = 18 + 1.

Tally (pr378-ci-e04aa9a-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 117708863191 | success |
| Lint (luajit) | 117708863365 | success |
| Lint (lua54) | 117708863399 | success |
| Commit attribution (D9) | 117708863443 | success |
| Changed paths | 117708864094 | success |
| M5 Perf Gates | 117708905503 | success |
| M4 Perf Gates | 117708905523 | success |
| M1 Acceptance Gates | 117708905555 | success |
| GPU Render (headless) | 117708905569 | success |
| Test (ubuntu-latest / lua54) | 117708905587 | success |
| Test (macos-latest / luajit) | 117708905593 | success |
| Test (crdt) | 117708905598 | success |
| Perf budgets (debug) | 117708905604 | success |
| Test (ubuntu-latest / luajit, no crdt) | 117708905605 | success |
| Test (ubuntu-latest / luajit) | 117708905610 | success |
| Test (macos-latest / lua54) | 117708905623 | success |
| M6 Perf Gates | 117708905630 | success |
| M10 Perf Gates (crdt) | 117708905650 | success |
| Docs consistency | 117708906920 | skipped |

Tally (pr378-ci-e04aa9a-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 203 on `Test (crdt)`, 204
  on each Ubuntu leg, 205 on each macOS leg. The E9.4 row passed on both macOS legs, and #316's
  and #363's rows on all six.

##### `CI` 39291657322 at `9024c22`, attempt 1 of 1: two of the branch's own rows on macOS

`pull_request`, the merge `b473a0e`, created 17:09:02Z and completed 17:44:07Z: `failure`.

Tally (pr378-ci-9024c22): 19 = 16 + 2 + 1.

Tally (pr378-ci-9024c22-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 118193556842 | success |
| Commit attribution (D9) | 118193557386 | success |
| Changed paths | 118193557455 | success |
| Lint (luajit) | 118193557506 | success |
| Format | 118193557561 | success |
| Test (crdt) | 118193667406 | success |
| M1 Acceptance Gates | 118193667566 | success |
| GPU Render (headless) | 118193667626 | success |
| Test (ubuntu-latest / luajit) | 118193667644 | success |
| M10 Perf Gates (crdt) | 118193667652 | success |
| M4 Perf Gates | 118193667666 | success |
| Perf budgets (debug) | 118193667678 | success |
| Test (macos-latest / luajit) | 118193667700 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 118193667724 | success |
| Test (macos-latest / lua54) | 118193667832 | failure |
| M6 Perf Gates | 118193667849 | success |
| M5 Perf Gates | 118193667924 | success |
| Test (ubuntu-latest / lua54) | 118193668097 | success |
| Docs consistency | 118193669684 | skipped |

Tally (pr378-ci-9024c22-success): 16 rows of the table above with `result` = `success`.

Tally (pr378-ci-9024c22-failure): 2 rows of the table above with `result` = `failure`.

- `Test (macos-latest / luajit)` 118193667700 and `Test (macos-latest / lua54)` 118193667832
  fail `daemon::tests::an_attach_whose_bootstrap_fails_leaves_no_departure_flag` at
  `src/daemon.rs:5714:14`, `read timeout: Os { code: 22, kind: InvalidInput, message: "Invalid
  argument" }`, the lib's `test result: FAILED. 2288 passed; 1 failed; 13 ignored`; the lua54
  leg also fails `e9_session_acceptance::e9_1_the_view_is_retained_across_detach_and_taken_back_by_its_token`
  at `tests/e9_session_acceptance.rs:369:5`, `the chain continues`, `left: (BufferId(1), 0)`,
  `right: (BufferId(3), 3)`, `test result: FAILED. 21 passed; 1 failed; 0 ignored`. Both are the
  branch's own rows, fixed test-only at `ce664e6` and `f991f7b`. No signature. Each leg paired 204
  with 204.
- The four other test legs pair each `running` line with a `test result: ok`: 204 on `Test
  (crdt)`, 205 on each Ubuntu leg.

##### `CI` 39299214663 at `f991f7b`, attempt 1 of 1: green

`pull_request`, the merge `796a1e9`, created 18:02:26Z and completed 18:35:51Z: `success`.

Tally (pr378-ci-f991f7b): 19 = 18 + 1.

Tally (pr378-ci-f991f7b-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 118219651115 | success |
| Commit attribution (D9) | 118219651470 | success |
| Format | 118219651540 | success |
| Lint (luajit) | 118219651605 | success |
| Lint (lua54) | 118219651803 | success |
| Test (ubuntu-latest / lua54) | 118219735359 | success |
| M1 Acceptance Gates | 118219735370 | success |
| Test (macos-latest / luajit) | 118219735425 | success |
| Test (crdt) | 118219735435 | success |
| GPU Render (headless) | 118219735436 | success |
| M10 Perf Gates (crdt) | 118219735443 | success |
| Perf budgets (debug) | 118219735455 | success |
| Test (macos-latest / lua54) | 118219735489 | success |
| M5 Perf Gates | 118219735525 | success |
| Test (ubuntu-latest / luajit) | 118219735543 | success |
| M4 Perf Gates | 118219735603 | success |
| Test (ubuntu-latest / luajit, no crdt) | 118219735615 | success |
| M6 Perf Gates | 118219735633 | success |
| Docs consistency | 118219738069 | skipped |

Tally (pr378-ci-f991f7b-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 204 on `Test (crdt)`, 205
  on each Ubuntu leg, 206 on each macOS leg; the two rows red at `9024c22` passed on both macOS
  legs.

##### `CI` 39322015547 at `3522b2f`, attempt 1 of 1: #381's and #382's first, and #316's sixteenth

`pull_request`, the merge `42093c4`, created 20:45:44Z and completed 21:20:04Z: `failure`.

Tally (pr378-ci-3522b2f): 19 = 16 + 2 + 1.

Tally (pr378-ci-3522b2f-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 118298167317 | success |
| Commit attribution (D9) | 118298167825 | success |
| Lint (luajit) | 118298167826 | success |
| Changed paths | 118298167955 | success |
| Format | 118298168003 | success |
| M1 Acceptance Gates | 118298255880 | success |
| Test (crdt) | 118298255911 | success |
| GPU Render (headless) | 118298255914 | success |
| M5 Perf Gates | 118298255936 | success |
| Test (ubuntu-latest / lua54) | 118298255952 | success |
| M6 Perf Gates | 118298256033 | success |
| Test (macos-latest / luajit) | 118298256037 | success |
| Test (ubuntu-latest / luajit) | 118298256094 | failure |
| M10 Perf Gates (crdt) | 118298256103 | success |
| M4 Perf Gates | 118298256110 | success |
| Test (macos-latest / lua54) | 118298256135 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 118298256195 | success |
| Perf budgets (debug) | 118298256366 | success |
| Docs consistency | 118298258089 | skipped |

Tally (pr378-ci-3522b2f-success): 16 rows of the table above with `result` = `success`.

Tally (pr378-ci-3522b2f-failure): 2 rows of the table above with `result` = `failure`.

- `Test (ubuntu-latest / luajit)` 118298256094: `gpu_font_acceptance`'s copy of
  `common::ready::tests::readiness_is_a_served_hello_not_a_connect` alone, at
  `tests/common/ready.rs:423:9`, `a daemon serving on /tmp/.tmp26LZ4g/late.sock`, `deadline:
  10s`, `polls: 473`, `last: "connect: Connection refused (os error 111)"`, `test result:
  FAILED. 15 passed; 1 failed; 0 ignored` (13.52 s), the self-test `ok` in 69 other copies in the
  job: **#381, filed, its first occurrence**, the same fragment group as #332 and #374 in a sixth
  suite's copy. 204 paired, 203 `ok`.
- `Test (macos-latest / lua54)` 118298256135, two rows:
  - `e7c_indicator_acceptance::e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`
    at `tests/e7c_indicator_acceptance.rs:726:5`, `nothing on the indicator under the threshold:
    [(0, Some("⋯1 parse rust                   "))]`, the row's own trace reading the rename on
    the indicator at 325 ms and gone at 1576 ms, and `test result: FAILED. 4 passed; 1 failed; 0
    ignored` (7.01 s): **#382, filed, its first occurrence**. It is #363's row, not #363's
    signature: the frame before the request was quiet, and the parse is in the first frame after
    it.
  - `e7i_review3_probes::e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer`
    at `tests/e7i_review3_probes.rs:865:9`, `kill 1: nothing held the switch's follow-up parse,
    which started a worker at once and installed`, the report `deaths=1 crashes=0 busy=false
    held=false pending=0 stopped=false tree=true unit=pid_38960 death=killed, " +  …⋯1 parse
    rust  parse:killed !1  L1:C1 Top"`, and `test result: FAILED. 10 passed; 1 failed; 0 ignored`
    (26.62 s): every fragment as written, **#316's sixteenth**, commented there
    (issuecomment-6286363461). 205 paired, 203 `ok`.
- The four other test legs pair each `running` line with a `test result: ok`: 205 on `Test
  (crdt)`, 206 on the two other Ubuntu legs, 207 on `Test (macos-latest / luajit)`. Each red
  leg's doc-test and protocol steps were skipped after its test step failed.

##### `CI` 39425693648 at `88ce655`, attempt 1 of 1: green, the reviewed head the owner merged

`pull_request`, the merge `9caa745`, created 12:52:31Z and completed 13:28:18Z: `success`.

Tally (pr378-ci-88ce655): 19 = 18 + 1.

Tally (pr378-ci-88ce655-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 118637559046 | success |
| Lint (lua54) | 118637559349 | success |
| Lint (luajit) | 118637559352 | success |
| Format | 118637559503 | success |
| Commit attribution (D9) | 118637559525 | success |
| GPU Render (headless) | 118637648803 | success |
| M6 Perf Gates | 118637648836 | success |
| Test (crdt) | 118637648845 | success |
| M5 Perf Gates | 118637648899 | success |
| Perf budgets (debug) | 118637648942 | success |
| M10 Perf Gates (crdt) | 118637648948 | success |
| Test (ubuntu-latest / lua54) | 118637648962 | success |
| Test (macos-latest / lua54) | 118637648979 | success |
| Test (macos-latest / luajit) | 118637648995 | success |
| M1 Acceptance Gates | 118637649043 | success |
| Test (ubuntu-latest / luajit) | 118637649066 | success |
| Test (ubuntu-latest / luajit, no crdt) | 118637649143 | success |
| M4 Perf Gates | 118637649152 | success |
| Docs consistency | 118637651807 | skipped |

Tally (pr378-ci-88ce655-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 206 on `Test (crdt)`, 207
  on each Ubuntu leg, 208 on each macOS leg. #316's row, the row #363 and #382 share, every copy
  of the readiness self-test and the rows red at `e7c8fd5` and `9024c22` passed on every leg that
  compiles them, which is non-reproduction and nothing more.

#### `main` at `1ff2603`, E9's squash: green

`1ff2603` is the owner's squash of #378, merged 16:59:04Z and pinned to the reviewed `88ce655`,
no bypass; signature `E`, trailers empty, and an empty diff from `88ce655`, so the squash is
faithful.

`CI` 39462183476 at `1ff2603`, attempt 1 of 1, `push`, created 16:59:07Z and completed 17:33:38Z
(the API's `updated_at`): `success`.

Tally (main-ci-1ff2603): 19 = 18 + 1.

Tally (main-ci-1ff2603-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 118764210488 | success |
| Commit attribution (D9) | 118764210903 | success |
| Changed paths | 118764211056 | success |
| Lint (luajit) | 118764211190 | success |
| Lint (lua54) | 118764211221 | success |
| GPU Render (headless) | 118764311651 | success |
| Test (crdt) | 118764311659 | success |
| M1 Acceptance Gates | 118764311709 | success |
| Test (ubuntu-latest / luajit) | 118764311717 | success |
| M5 Perf Gates | 118764311735 | success |
| M6 Perf Gates | 118764311757 | success |
| Test (macos-latest / luajit) | 118764311767 | success |
| Perf budgets (debug) | 118764311807 | success |
| M10 Perf Gates (crdt) | 118764311814 | success |
| Test (macos-latest / lua54) | 118764311836 | success |
| Test (ubuntu-latest / lua54) | 118764311855 | success |
| Test (ubuntu-latest / luajit, no crdt) | 118764311885 | success |
| M4 Perf Gates | 118764312011 | success |
| Docs consistency | 118764313663 | skipped |

Tally (main-ci-1ff2603-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 206 on `Test (crdt)`, 207
  on each Ubuntu leg, 208 on each macOS leg; `WouldBlock` appears in none. #306's, #316's,
  #373's and #375's rows, the row #363 and #382 share, every copy of the readiness self-test,
  #361's two rows and E9's own rows passed on every leg that compiles them, which is
  non-reproduction and nothing more.

`Grammar fuzz` 39462183227 at `1ff2603`, attempt 1 of 1, `push`, created 16:59:07Z and completed
16:59:21Z, `success`, fuzzing nothing on either arm (`run=false`): `ubsan` 118764210026,
`asan-strict` 118764210661.

#### The local samples before the 2026-10-18 ruling: listed, not counted

The gates of E8d, E8e and E9's first three passes, read from their logs: every one five of six
(the default plan) or six of eight (`--protocol`) unless said, `fmt`, the lints, `doc`, `build`
and `diff-check` ok, each sweep's `running` lines paired with as many result lines. #311 failed
the HLS row in every complete sweep below that is not said to carry #291's fragments instead
(`Could not find module ‘Prelude’`, the trace reaching `LSP:ready·processi`, `4 passed; 1
failed`). The other samples:

- E8d's gates, the default plan, `05-sweep`: `20261014T095325Z-3770109` on E8d.1's tree (its
  commit is `d41a83b`), #361's own selector,
  `e8b_switch_buffer_acceptance::e8b_fr1_the_gpu_reaches_a_dired_buffer_by_its_own_name_and_keeps_d18_s_four`,
  with all four of its fragments (`left: "\"switch-buffer: showing
  *dired:/home/jeans/build/pmacs-gate-targets/tmp/ZtCHzp/.tmpuEqaZn/nightowls/proj*\""`,
  `right: "\"no buffer: zzz\""`, `9 passed; 1 failed`), commented there;
  `20261014T105531Z-4623` on E8d.4's tree (`e4c846d`), the outside-kill row on kill 3 with
  `deaths=3 … busy=false … tree=true`, `" +* …`, `parse:killed !3` and `10 passed; 1 failed`
  (13.97 s), #316's template reading; `20261014T112755Z-283384` at
  `e4a0cb8`, #361's mechanism on its second row (`switch_buffer_says_which_buffer_ret_reached`,
  the gate's root `pZykT4` and the fixture's `.tmpzpU3ZF` spelling `zzz`), commented, no count; and
  `20261014T171638Z-1006348`, the owner's pass gate at `22de659`, R7. `20261014T101154Z-3894530`
  was interrupted in its sweep after a clippy red of the round's own and is no evidence.
- E8e's gates, the default plan, `05-sweep`: R7 at `20261015T112921Z-1698319` (E8e.1, `a7f915b`)
  and `20261015T122545Z-1811476` (E8e.2, `7ae872a`), and #264 at `20261015T172141Z-3943` (E8e.3,
  `b73abbe`). `20261015T124522Z-1951698` was interrupted before its sweep reported and is no
  evidence.
- E9's gates, `--protocol`: `20261016T092821Z-377716` at `9b02148` (E9.1 before its amend), its
  one other red in both sweeps the branch's own literal tripwire at
  `tests/statusline_segments_acceptance.rs:825` (`left: 27`, `right: 26`); `20261016T100845Z-734980`
  at `772b039` (E9.2 before its amend, five of eight, its `doc` red on the branch's own link), R7
  in both sweeps and #264 in `07-sweep-luajit`; `20261016T154436Z-987086` at `fd32170`, R7 in
  `06-sweep`; `20261017T170736Z-1690674` at `e7c8fd5`, the tip, #264 and #377's first sample in
  `06-sweep` (`process::tests::m6_1_pty_canonical_mode_keeps_kernel_echo` at `src/process.rs:4201:9`,
  `stty -a output was: ""`, the lib's `2288 passed; 2 failed; 13 ignored`) and #291's fragments
  in `07-sweep-luajit` (`store []`, the trace never reaching `processi`, 126.72 s); review 1's
  `20261018T131101Z-2048560` at `e7c8fd5`, the outside-kill row on kill 3 in `06-sweep`
  (`busy=false`, `" +* …`, `parse:killed !3`, 14.20 s), #316's template reading; fix round 1's
  `20261018T143623Z-2650551` at `e04aa9a`, #291's fragments in `07-sweep-luajit` (128.20 s). The
  handoff's table names R7 in `20261016T100845Z-734980`'s `sweep` alone; its `07-sweep-luajit`
  log carries it too, the third of that sweep's three failures (5,046 / 3 / 51).

None of them is counted. R7's six, #264's three, #291's two and #311's twenty-two are listed in
their rows' untallied blocks too, R7's below and the others above. #316 moves by its two CI
samples since E8c alone:

Tally (316-ci-since-e8c): 2 items in the list below.

- 39054498392 at `e7c8fd5`, `Test (crdt)` 117459636346, kill 3, the template reading;
- 39322015547 at `3522b2f`, `Test (macos-latest / lua54)` 118298256135, kill 1, every fragment.

Tally (316): 16 = 14 + 2.

Its local samples in the same span, listed and not counted: `20261014T105531Z-4623` (E8d.4's gate,
`05-sweep`, kill 3) and `20261018T131101Z-2048560` (E9 review 1's gate, `06-sweep`, kill 3), both
under the template reading, and the three after the ruling in the block below.

#361 stays at one: E8d's local sample on its own selector is listed, not counted (its first, in CI
at `752811c`, is recorded at E8c's fix round 1, above). #377 is filed on a local first sample and
stands at no CI occurrence; its issue names it a recurrence of U21, closed 2026-09-05, which reopens
the question, and reopening the row is the owner's.

#### The local samples after the 2026-10-18 ruling: listed, not counted

Every gate below is `scripts/gate --protocol`, six of eight, `fmt`, both lints, `doc`, `build` and
`diff-check` ok, each sweep's `running` lines paired with as many result lines; each sample is on
its issue already. None moves a count.

- `20261018T155209Z-3093903` at `e04aa9a`, review 2's gate: #291's fragments in `06-sweep`
  (127.02 s), #311's in `07-sweep-luajit` (130.02 s).
- `20261019T170843Z-89933` at `9024c22`, fix round 2's: #311 in both sweeps, and #288's fragments
  in `06-sweep` (`the check's diagnostic landed: ["pmacs-fake-lsp"]`, `4 passed; 1 failed`).
- `20261019T174111Z-309539` at `f991f7b`, fix round 2's: #311 in both, and #305's in
  `07-sweep-luajit` (`no request outlived the threshold, so no indicator at 299 ms`, `left:
  Some("⋯1 parse rust`, `4 passed; 1 failed`).
- `20261019T184241Z-581068` at `f991f7b`, review 3's: #311 in both; #289's in `06-sweep` (`must
  observe stdout 'done'`, the lib's `2294 passed; 1 failed; 13 ignored`); and in
  `07-sweep-luajit` `compile_mode_acceptance::acc16_n_p_walk_error_lines_without_wrap`, `n walks
  forward between error lines`, `74 passed; 1 failed`, filed as #380, a local first sample.
- `20261019T201606Z-867257` at `3522b2f`, fix round 3's: #311 in both; #361's own selector with
  all four fragments in `06-sweep`; the outside-kill row on kill 1 with every fragment in
  `07-sweep-luajit`.
- `20261020T080420Z-25738` at `3522b2f`, fix round 3's: #311 in both; the outside-kill row on
  kill 3, the template reading, in `07-sweep-luajit`.
- `20261020T100133Z-242643` at `3522b2f`, review 4's: #311 in both; #304's shape in
  `07-sweep-luajit` (`caret-inside must render the raw source, exactly`, the two 230,400-byte
  frames differing in 44 pixels at x = 102–103 over y = 16–37, `[233, 233, 240, 255]` against
  the background `[63, 63, 75, 255]`, `380 passed; 1 failed`).
- `20261020T122258Z-558748` at `88ce655`, fix round 4's: #311 in both, nothing else.
- `20261020T135503Z-822983` at `88ce655`, review 5's: #311 in both; the outside-kill row on kill
  2, the template reading, in `06-sweep`; #264 in `07-sweep-luajit` (`src/daemon_attach.rs:859:9`,
  `AutoStartTimeout`, the lib's `2101 passed; 1 failed; 12 ignored`).

R7 is in none of them, nor in the four of E9's gates before the ruling that carry `d86d135`;
see R7's row.

So, at E9's close: `main` is green at `1ff2603`; by CI alone #306 moves to three, #363 to two and
#316 to sixteen, and #373, #374, #375, #381 and #382 are filed at one each, all first seen in CI.
R7 stays at thirty-six, #264 at two, #291 at seventeen, #361 at one and #311's list of runs at
fifty. #377 and #380 are filed on local samples and stand at no CI occurrence. Every local sample
above is listed and not counted, under the owner's ruling of 2026-10-20, and #288, #289, #304 and
#305, which met only local samples, do not move. Every other count above stands.

### E9b's close: PR #392's run at `985ace1`, `main` green at `5a0998b` and `3c313e0`, the gates' samples listed, #391 and #393 filed

Recorded 2026-10-20 at E9b's checkpoint, PR #392 at `985ace1` (two signed commits on `5a0998b`:
E9 review 5's probes, and the fold of the daemon's per-frontend maps into one `Session`). Every
run was read from the jobs endpoint by conclusion, and every test leg of the code-bearing run
through its own log, fetched with escape sequences allowed and stripped, each `running` line
paired with its own result line. Both gates were read from their log directories under
`~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/` (copied to `~/build/e9b/gate-logs/`).
Every run id, job id and gate log below was searched for in this file before it was written, and
none was here. Nothing was rerun.

#### `main` at `5a0998b` and `3c313e0`, E9's close: green

`5a0998b` and `3c313e0` are the two registry commits for E9's close, above; each changes this file
alone. PR #392 was rebased onto `5a0998b` and pushed at 18:19Z; `3c313e0` landed at 18:46Z, and
the branch, which has a drafts entry, is not rewritten for it (D10). It merges into `3c313e0`
without conflict (`git merge-tree`).

`CI` 39468668979 at `5a0998b`, attempt 1 of 1, `push`, created 17:44:15Z and completed 17:46:25Z:
`success`, every test leg skipped, `Docs consistency` 118786744743 among the six.

Tally (main-ci-5a0998b): 15 = 6 + 9.

`CI` 39477730802 at `3c313e0`, attempt 1 of 1, `push`, created 18:46:51Z and completed 18:48:41Z:
`success`, every test leg skipped, `Docs consistency` 118818034939 among the six.

Tally (main-ci-3c313e0): 15 = 6 + 9.

`Grammar fuzz` 39468668925 at `5a0998b` and 39477730726 at `3c313e0`, each `success` with both
arms deciding `run=false`: `ubsan` 118786656599 and 118817941682, `asan-strict` 118786657036 and
118817943095.

#### PR #392's `CI` 39473773462 at `985ace1`: two reds, and no test among them

`CI` 39473773462, attempt 1 of 1 when read here, `pull_request`, testing the merge into
`5a0998b`, created 18:19:48Z and completed 18:55:11Z: `failure`. Its attempt 2, the non-crdt job
rerun alone on the owner's ruling, is recorded at E9b's merge, below.

Tally (pr392-ci-985ace1): 19 = 16 + 1 + 1 + 1.

Tally (pr392-ci-985ace1-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 118804322826 | success |
| Commit attribution (D9) | 118804323215 | success |
| Lint (luajit) | 118804323332 | success |
| Lint (lua54) | 118804323441 | success |
| Changed paths | 118804323695 | success |
| Test (ubuntu-latest / luajit, no crdt) | 118804426656 | failure |
| M4 Perf Gates | 118804426697 | success |
| M1 Acceptance Gates | 118804426766 | success |
| M10 Perf Gates (crdt) | 118804426771 | success |
| Perf budgets (debug) | 118804426815 | success |
| M5 Perf Gates | 118804426838 | success |
| M6 Perf Gates | 118804426862 | success |
| Test (macos-latest / luajit) | 118804426921 | success |
| Test (ubuntu-latest / luajit) | 118804426924 | success |
| GPU Render (headless) | 118804426931 | success |
| Test (macos-latest / lua54) | 118804427002 | success |
| Test (ubuntu-latest / lua54) | 118804427226 | success |
| Test (crdt) | 118804427695 | cancelled |
| Docs consistency | 118804430305 | skipped |

Tally (pr392-ci-985ace1-success): 16 rows of the table above with `result` = `success`.

- `Test (ubuntu-latest / luajit, no crdt)` failed in its tool setup, before any build or test
  (no `test result` line in its 731-line log): `go install golang.org/x/tools/gopls@v0.16.2`
  stopped at `mvdan.cc/gofumpt@v0.6.0: verifying module: mvdan.cc/gofumpt@v0.6.0: reading
  https://sum.golang.org/tile/8/0/x087/358: stream error: stream ID 77; INTERNAL_ERROR; received
  from peer`. #249's checksum-database shape, with another module than its required
  `golang.org/x/telemetry`, so not its exact fragments; commented on #249, as PR #272's
  occurrence was.
- `Test (crdt)` ran every test green, 207 `running` lines paired with 207 `test result: ok`,
  `WouldBlock` absent, and `cargo test --doc`'s one doc test `ok` at 18:55:06Z; the job was then
  `cancelled` with `The job has exceeded the maximum execution time of 35m0s`, 18:20:04Z to
  18:55:10Z. Its test step took 1,955 s and its workspace build 70 s, where `main`'s run at
  `1ff2603` (job 118764311659, 34m14s, `success`) took 1,977 s and 21 s; the PR recompiles the
  `pmacs` crate. No row names the job at its ceiling: filed as **#393**, a first occurrence.
- The four other test legs pair each `running` line with a `test result: ok`: 209 on each macOS
  leg, 208 on each Ubuntu leg, `WouldBlock` in none. #286's row
  (`m10_10_f14_production_path_keystroke_flows_to_broadcast`) passed on all five legs that ran
  tests; E9 review 5's probes passed on the two Ubuntu legs and on `Test (crdt)`, and compile to
  nothing on macOS by their own gate.

`Grammar fuzz` 39473773891 at `985ace1`, `success`, both arms deciding `run=false`: `ubsan`
118804325769, `asan-strict` 118804325169.

#### E9b's gates and hand samples: listed, not tallied

Both gates are `scripts/gate --protocol`, six of eight, `fmt`, both lints, `doc`, `build` and
`diff-check` ok, no daemon surviving, each sweep pairing 216 `running` lines with as many result
lines. Each sample is on its issue. None moves a count.

- `20261020T174241Z-1518534` at `15d0c16`, the branch before its rebase onto `5a0998b` (the same
  code as `985ace1`): #311 in both sweeps; #288's fragments in `06-sweep` (`the check's diagnostic
  landed: ["pmacs-fake-lsp"]`, `4 passed; 1 failed`); #316's outside-kill row on kill 3, the
  template reading, in `07-sweep-luajit`; and in `06-sweep`
  `e7h_grammar_gate_acceptance::e7h2_a_large_slow_input_that_first_meets_the_memory_limit_is_filed_slow`,
  the report row holding `"slow=0"`, `"hangs=0"` and `"unconfirmed=1"`, a **lua** hang over 1000
  ms not reproduced, `22 passed; 1 failed; 2 ignored`: filed as **#391**, a local first sample,
  #339's report fragments on another selector.
- `20261020T181724Z-1913073` at `985ace1`: #311 in both sweeps; #286's fragments in `06-sweep`
  (`F14: B must receive A's CrdtOp broadcast end-to-end`, `40 passed; 1 failed`), its second
  sample and first since 2026-09-25.
- By hand, outside any gate, the suite binary of the second gate run directly: #286's row failed
  once in 50 suite runs at `985ace1` and once in 50 with `src/daemon.rs` and `src/presence.rs`
  copied in from `85714d2`, both fragments each time, load averages 4 to 7. The branch's fold
  touches the relay the row drives, so this control stands where an untouched-files argument
  would otherwise.

So, at E9b's close: `main` is green at `5a0998b` and `3c313e0`; by CI alone #393 is filed at one, and #249,
whose fragments name another module, does not move. #391 is filed on a local sample and stands at
no CI occurrence. #286, #288, #311 and #316 met only local samples here and do not move. Every
other count above stands.

### E9b's merge: PR #392's attempt 2 green, `main` green at `926d25a` and `828248f`, and review 1's gate listed

Recorded 2026-10-21 for E9b's close under D38, on `main` at `828248f`. Every run was read from the
jobs endpoint by conclusion, and every test leg of the code-bearing runs through its own log,
fetched with escape sequences allowed and stripped, each `running` line paired with its own result
line. Review 1's gate was read from its log directory under
`~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/`. Every run id, job id and gate log
below was searched for in this file before it was written, and none was here. This record reran
nothing.

#### PR #392's `CI` 39473773462, attempt 2: the non-crdt job rerun alone, green

After the section above recorded attempt 1, the owner ruled on 2026-10-20 that the non-crdt job
alone be rerun for its coverage. Attempt 2 re-executed it alone as job 118904948075, 21:46:13Z to
22:15:07Z, `success`; `Docs consistency` was evaluated again and skipped; the other seventeen rows
of attempt 2 carry attempt 1's results and timestamps under new ids. The run's conclusion now
reads `cancelled`, and its one `cancelled` row is `Test (crdt)`'s, carried from attempt 1.

Tally (pr392-ci-985ace1-attempt2): 19 = 17 + 1 + 1.

Tally (pr392-ci-985ace1-attempt2-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (luajit) | 118904947634 | success |
| Test (ubuntu-latest / luajit, no crdt) | 118904948075 | success |
| Format | 118904948342 | success |
| Docs consistency | 118904948930 | skipped |
| Lint (lua54) | 118904948990 | success |
| Test (ubuntu-latest / luajit) | 118904949026 | success |
| Perf budgets (debug) | 118904949186 | success |
| M5 Perf Gates | 118904949402 | success |
| M1 Acceptance Gates | 118904949525 | success |
| M10 Perf Gates (crdt) | 118904949548 | success |
| Test (ubuntu-latest / lua54) | 118904949590 | success |
| Test (crdt) | 118904949613 | cancelled |
| M6 Perf Gates | 118904949628 | success |
| GPU Render (headless) | 118904949726 | success |
| M4 Perf Gates | 118904950158 | success |
| Test (macos-latest / luajit) | 118904950418 | success |
| Test (macos-latest / lua54) | 118904950529 | success |
| Commit attribution (D9) | 118905004003 | success |
| Changed paths | 118905009526 | success |

Tally (pr392-ci-985ace1-attempt2-success): 17 rows of the table above with `result` = `success`.

- The rerun job's tool step installed `gopls` this time, downloading `mvdan.cc/gofumpt v0.6.0` at
  21:47:10Z with no error, and the gate's arming line names `gopls` present. It paired 208
  `running` lines with 208 `test result: ok`: 206 in its test step, `cargo test --doc`'s one doc
  test, and `pmacs-protocol`'s `crdt` step; 4,689 passed, 0 failed, 51 ignored; `WouldBlock`
  absent.
- **Attempt 2 is non-reproduction and nothing more.** Attempt 1's job 118804426656 stays as
  recorded above, commented on #249, which does not move. `Test (crdt)` was not rerun, and #393
  stays at its one occurrence.

#### `main` at `926d25a`, E9b's registry commit: green

`CI` 39479807992 at `926d25a`, attempt 1 of 1, `push`, created 19:01:05Z and completed 19:03:15Z:
`success`, every test leg skipped, `Docs consistency` 118825204934 among the six.

Tally (main-ci-926d25a): 15 = 6 + 9.

`Grammar fuzz` 39479807989 at `926d25a`, `success`, both arms deciding `run=false`: `ubsan`
118825087204, `asan-strict` 118825087776.

#### `main` at `828248f`, E9b's squash: green

`828248f` is the owner's squash of #392 at its head `985ace1`, merged 08:42:43Z past `Test
(crdt)`'s attempt-1 cancellation under the owner's ruling of 2026-10-20; signature `E`, trailers
empty. Its diff from `985ace1` is this file alone, the two registry commits `main` gained after
the branch's base, and its diff from `926d25a` is exactly `src/daemon.rs`, `src/presence.rs` and
`tests/e9_review5_probes.rs`, so the squash is faithful.

`CI` 39571345565 at `828248f`, attempt 1 of 1, `push`, created 08:42:45Z and completed 09:16:15Z
(the API's `updated_at`): `success`.

Tally (main-ci-828248f): 19 = 18 + 1.

Tally (main-ci-828248f-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 119126015855 | success |
| Lint (lua54) | 119126016091 | success |
| Lint (luajit) | 119126016101 | success |
| Commit attribution (D9) | 119126016208 | success |
| Changed paths | 119126016245 | success |
| M5 Perf Gates | 119126103021 | success |
| Test (ubuntu-latest / luajit) | 119126103074 | success |
| M4 Perf Gates | 119126103098 | success |
| M10 Perf Gates (crdt) | 119126103104 | success |
| GPU Render (headless) | 119126103108 | success |
| M1 Acceptance Gates | 119126103115 | success |
| Test (macos-latest / lua54) | 119126103117 | success |
| Test (crdt) | 119126103125 | success |
| Test (ubuntu-latest / luajit, no crdt) | 119126103171 | success |
| Perf budgets (debug) | 119126103227 | success |
| Test (ubuntu-latest / lua54) | 119126103259 | success |
| Test (macos-latest / luajit) | 119126103263 | success |
| M6 Perf Gates | 119126103479 | success |
| Docs consistency | 119126106595 | skipped |

Tally (main-ci-828248f-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 207 on `Test (crdt)` (206 in
  its test step and the doc test), 208 on each Ubuntu leg (206, the doc test and
  `pmacs-protocol`'s `crdt` step), 209 on each macOS leg (those and the GPU adapter probe);
  `WouldBlock` appears in none. #286's row passed on the five legs built with `crdt`, and E9 review
  5's probes ran eight tests on `Test (crdt)` and on each Ubuntu `crdt` leg, and `running 0
  tests` on the macOS legs and the no-crdt leg.
- `Test (crdt)` ran 08:43:02Z to 09:15:57Z, 32m55s, under its 35-minute ceiling: its workspace
  build 21.1 s and its test step 1,902.9 s by the log's step timestamps. That is not an
  occurrence of #393's shape, and it is non-reproduction and nothing more.

`Grammar fuzz` 39571345616 at `828248f`, attempt 1 of 1, `push`, created 08:42:45Z and completed
08:43:00Z, `success`, fuzzing nothing on either arm (`run=false`): `ubsan` 119126016537,
`asan-strict` 119126016036.

#### E9b review 1's gate: listed, not tallied

`scripts/gate --protocol` `20261020T194026Z-2442131` at `985ace1`, E9b review 1's independent
gate: six of eight, `fmt`, both lints, `doc`, `build` and `diff-check` ok, each sweep pairing 216
`running` lines with as many result lines. Both samples are on their issues. Neither moves a
count.

- `06-sweep`: #291's fragments on
  `e7e_haskell_acceptance::e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error`
  (`tests/e7e_haskell_acceptance.rs:637:5`, `HLS reports the type error; store []`, the trace
  reaching `LSP:ready·setting` and never `processi`, `4 passed; 1 failed; 0 ignored` in 132.81 s
  at result line 3531); commented on #291 (issuecomment-6302206756).
- `07-sweep-luajit`: #311's fragments on the same selector and line (`Could not find module
  ‘Prelude’` in the store, the trace reaching `LSP:ready·processi`, `4 passed; 1 failed; 0
  ignored` in 127.30 s at result line 3265); commented on #311 (issuecomment-6302203294).

So, at E9b's merge: `main` is green at `926d25a` and `828248f`, and PR #392's attempt 2 is green
on the one job it reran. By CI alone no count moves: #393 stays at one, and #249 does not move.
#291 and #311 met only local samples here and do not move. Every other count above stands.

### E10's close: PR #396's runs at `d2fb9c1` and `0ff624e`, `main` green at `1773b85`, the gates' samples listed, #394 and #395 filed

Recorded 2026-10-21 at E10's checkpoint, PR #396 (five signed commits on `1773b85`: E9b review 1's
probes, E10.1, E10.2, E10.3, and a test-only commit answering the first run). Every run was read
from the jobs endpoint by conclusion, and every test leg of the code-bearing runs through its own
log, fetched with escape sequences allowed and stripped, each `running` line paired with its own
result line. The gates were read from their log directories under
`~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/` (copied to `~/build/e10/gate-logs/`).
Every run id, job id and gate log below was searched for in this file before it was written, and
none was here. Nothing was rerun.

#### `main` at `1773b85`, E9b's close: green

`1773b85` is the registry commit for E9b's close, above; it changes this file alone. PR #396 was
rebased onto it before its first push.

`CI` 39593414937 at `1773b85`, attempt 1 of 1, `push`, created 11:38:12Z and completed 11:40:16Z
(the API's `updated_at`): `success`, every test leg skipped, `Docs consistency` 119199817288
among the six.

Tally (main-ci-1773b85): 15 = 6 + 9.

`Grammar fuzz` 39593415017 at `1773b85`, `success`, both arms deciding `run=false`: `ubsan`
119199742277, `asan-strict` 119199742656.

#### PR #396's `CI` 39614357163 at `d2fb9c1`: one red, the branch's own row

`CI` 39614357163, attempt 1 of 1, `pull_request`, testing the merge into `1773b85`, created
14:09:25Z and completed 14:43:32Z: `failure`.

Tally (pr396-ci-d2fb9c1): 19 = 17 + 1 + 1.

Tally (pr396-ci-d2fb9c1-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 119271299415 | success |
| Changed paths | 119271299418 | success |
| Lint (lua54) | 119271299447 | success |
| Commit attribution (D9) | 119271299742 | success |
| Lint (luajit) | 119271299856 | success |
| GPU Render (headless) | 119271402481 | success |
| Test (crdt) | 119271402563 | failure |
| M4 Perf Gates | 119271402569 | success |
| Perf budgets (debug) | 119271402727 | success |
| M5 Perf Gates | 119271402757 | success |
| M1 Acceptance Gates | 119271402761 | success |
| Test (macos-latest / lua54) | 119271402763 | success |
| Test (ubuntu-latest / luajit) | 119271402765 | success |
| Test (ubuntu-latest / lua54) | 119271402860 | success |
| Test (macos-latest / luajit) | 119271402999 | success |
| Test (ubuntu-latest / luajit, no crdt) | 119271403005 | success |
| M6 Perf Gates | 119271404487 | success |
| M10 Perf Gates (crdt) | 119271404596 | success |
| Docs consistency | 119271405427 | skipped |

Tally (pr396-ci-d2fb9c1-success): 17 rows of the table above with `result` = `success`.

- `Test (crdt)` failed on the row the branch added,
  `e10_restore_acceptance::e10_2_the_gpu_window_itself_reopens_the_note_where_it_was_left`:
  `the report has no text.before; facts: {"phase.done": "false"}` at
  `tests/common/gpu_probe.rs:64:12`, `test result: FAILED. 24 passed; 1 failed; 0 ignored` in
  16.93 s. The row's second arm (the reopening off) ran `pmacs-gpu`'s headless probe against a
  six-second deadline, and on this leg, which runs its tests one at a time, the probe had not
  typed inside it. A defect of the row, not an intermittent: the branch's `0ff624e` drops that arm
  and gives the probe fifteen seconds (below). No row or issue is opened for it. The leg paired 208
  `running` lines with 208 result lines, 5,193 passed, 1 failed, 65 ignored, `WouldBlock` absent;
  `cargo test --doc` did not run after the failed step. The job ran 14:09:40Z to 14:43:32Z, 33m52s,
  its workspace build 28 s and its test step 32m43s; it was not cancelled, so it is not #393's
  shape.
- The five other test legs pair each `running` line with a `test result: ok`: 211 on each macOS leg
  (5,218 passed, 63 ignored), 210 on each Ubuntu leg with `crdt` (5,248 passed, 65 ignored) and 210
  on the no-crdt leg (4,691 passed, 51 ignored); `WouldBlock` in none. The twenty E10 rows passed on
  the four other legs built with `crdt` and compile to nothing on the no-crdt leg; #394's and #395's
  rows passed on every leg that ran them.

`Grammar fuzz` 39614357162 at `d2fb9c1`, `success`, both arms deciding `run=false`: `ubsan`
119271299132, `asan-strict` 119271298774.

#### PR #396's `CI` 39619943886 at `0ff624e`, the head: green

`CI` 39619943886, attempt 1 of 1, `pull_request`, testing the merge into `1773b85`, created
14:47:27Z and completed 15:22:17Z (the API's `updated_at`): `success`.

Tally (pr396-ci-0ff624e): 19 = 18 + 1.

Tally (pr396-ci-0ff624e-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 119290418914 | success |
| Lint (luajit) | 119290419092 | success |
| Changed paths | 119290419218 | success |
| Format | 119290419325 | success |
| Commit attribution (D9) | 119290419413 | success |
| Test (crdt) | 119290546431 | success |
| M1 Acceptance Gates | 119290546471 | success |
| Test (macos-latest / luajit) | 119290546507 | success |
| Test (ubuntu-latest / luajit, no crdt) | 119290546638 | success |
| Test (ubuntu-latest / lua54) | 119290546675 | success |
| Test (macos-latest / lua54) | 119290546683 | success |
| GPU Render (headless) | 119290546727 | success |
| Perf budgets (debug) | 119290546871 | success |
| M10 Perf Gates (crdt) | 119290546942 | success |
| M4 Perf Gates | 119290546961 | success |
| M5 Perf Gates | 119290546972 | success |
| M6 Perf Gates | 119290547250 | success |
| Test (ubuntu-latest / luajit) | 119290547261 | success |
| Docs consistency | 119290549429 | skipped |

Tally (pr396-ci-0ff624e-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: 209 on `Test (crdt)` (208 in
  its test step and the doc test; 5,195 passed, 65 ignored), 211 on each macOS leg (5,218 passed,
  63 ignored), 210 on each Ubuntu leg with `crdt` (5,248 passed, 65 ignored) and 210 on the no-crdt
  leg (4,691 passed, 51 ignored); `WouldBlock` appears in none. The twenty E10 rows passed on the
  five legs built with `crdt`, the GPU-window row among them, and compile to nothing on the no-crdt
  leg; #394's and #395's rows passed on every leg that ran them.
- `Test (crdt)` ran 14:47:44Z to 15:21:24Z, 33m40s, under its 35-minute ceiling: its workspace build
  28 s and its test step 32m30s by the log's step timestamps, the E10 suite 25 passed in 19.53 s.
  That is not an occurrence of #393's shape, and it is non-reproduction and nothing more.

`Grammar fuzz` 39619943896 at `0ff624e`, `success`, both arms deciding `run=false`: `ubsan`
119290418945, `asan-strict` 119290420681.

#### E10's gates: listed, not tallied

All five are `scripts/gate --protocol` in the worktree `pmacs-e8d`, six of eight, `fmt`, both lints,
`doc`, `build` and `diff-check` ok, no daemon surviving, each sweep pairing 218 `running` lines with
as many result lines and `WouldBlock` in none. Each sample is on its issue. None moves a count.

- `20261021T115903Z-3334903` at `0baa312` (E10.1 before the rebase): `06-sweep` 5,612 / 2 / 65,
  #311's fragments on `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error`, and
  `e7i_review3_probes::e7i_review3_a_deadline_that_cuts_only_layers_keeps_its_current_spans` on
  `the warm parse installed`, `10 passed; 1 failed; 0 ignored`, #340's selector and message with
  its required attempt-trace fragment absent from the log, so not #340's by its fragments
  (commented there, issuecomment-6318692050); `07-sweep-luajit` 5,068 / 1 / 51, #311.
- `20261021T125103Z-3876831` at `c14268c` (E10.2 before its last amendment): `06-sweep`
  5,625 / 1 / 65, #311; `07-sweep-luajit` 5,068 / 2 / 51, #291's fragments on the same HLS row
  (`store []`, the trace stopping at `LSP:ready·setting`) and #316's outside-kill row on kill 1,
  the template reading, `busy=false` (issuecomment-6318691285).
- `20261021T132010Z-25590` at `8aeee79` (the rebased tip before E10.2's amendment): `06-sweep`
  5,623 / 3 / 65, #311; `process::tests::m6_2_pty_streaming_respects_byte_ceiling` on `after 200 ms
  producing 10485760 bytes without draining, the channel should have data; got 0`, `2296 passed; 1
  failed; 13 ignored` for the lib target, no row or issue naming it: filed as **#394**, a local
  first sample; and `e9_session_acceptance::e9_2_the_gpu_reconnects_with_a_banner_and_resumes_its_view`
  on `timed out waiting for the note's snapshot`, `26 passed; 1 failed; 0 ignored`, the row's GPU
  half passing and its fresh reader failing, no row or issue naming it: filed as **#395**, a local
  first sample, with a hand count of 2 failed in 60 runs at the branch and 0 in 60 at `1773b85`,
  which does not tell the rates apart; `07-sweep-luajit` 5,069 / 1 / 51, #311.
- `20261021T140736Z-629871` at `d2fb9c1`: `06-sweep` 5,626 / 1 / 65, #291; `07-sweep-luajit`
  5,069 / 1 / 51, #311.
- `20261021T144743Z-991552` at `0ff624e`, the head: `06-sweep` 5,625 / 2 / 65, #311 and #316's
  outside-kill row on kill 1, the template reading (issuecomment-6319647336); `07-sweep-luajit`
  5,069 / 1 / 51, #311 (both #311 samples in issuecomment-6319646914).

#311's six samples from the first four gates and #291's two are on those issues
(issuecomment-6318690250 and issuecomment-6318690596), each with its result line and time.

So, at E10's close: `main` is green at `1773b85`. By CI alone no count moves: PR #396's one red is
the branch's own row, answered by `0ff624e`. #394 and #395 are filed on local samples and stand at
no CI occurrence. #291, #311 and #316 met only local samples here and do not move; #340's
selector met a sample that is not its by fragments. Every other count above stands.

### E10's merge and #398's: `main` red at `fbf73b6` on #364's second and #399's first, PR #398 red at `550ffe5` on #400's first, `main` red at `76343d7` on #363's third, and review 1's gate listed

Recorded 2026-10-22 for E10's merge and #398's under D38, on `main` at `76343d7`. Every run was
read from the jobs endpoint by conclusion, and every test leg of the code-bearing runs through its
own log, fetched with escape sequences allowed and stripped, each `running` line paired with its
own result line. Review 1's gate was read from its log directory under
`~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/`. Every run id, job id and gate log
below was searched for in this file before it was written, and none was here. Nothing was rerun.

#### `main` at `fbf73b6`, E10's squash: red on #364's second and #399's first

`fbf73b6` is the owner's squash of #396 at its head `0ff624e`, merged 2026-10-21T18:55:48Z;
signature `E`, trailers empty. Its diff from `0ff624e` is this file alone, the section above that
`main` gained after the branch's base, so the squash is faithful.

`CI` 39657125876 at `fbf73b6`, attempt 1 of 1, `push`, created 18:55:50Z and completed 19:30:10Z
(the API's `updated_at`): `failure`.

Tally (main-ci-fbf73b6): 19 = 16 + 2 + 1.

Tally (main-ci-fbf73b6-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 119420248950 | success |
| Lint (lua54) | 119420249224 | success |
| Commit attribution (D9) | 119420249378 | success |
| Lint (luajit) | 119420249385 | success |
| Format | 119420249398 | success |
| M1 Acceptance Gates | 119420342419 | success |
| GPU Render (headless) | 119420342505 | success |
| Test (crdt) | 119420342531 | success |
| M10 Perf Gates (crdt) | 119420342676 | success |
| M5 Perf Gates | 119420342684 | success |
| Test (ubuntu-latest / lua54) | 119420342786 | success |
| Test (macos-latest / lua54) | 119420342800 | failure |
| M4 Perf Gates | 119420342809 | success |
| Test (ubuntu-latest / luajit, no crdt) | 119420342810 | failure |
| Perf budgets (debug) | 119420342868 | success |
| Test (ubuntu-latest / luajit) | 119420342870 | success |
| M6 Perf Gates | 119420342983 | success |
| Test (macos-latest / luajit) | 119420343279 | success |
| Docs consistency | 119420344319 | skipped |

Tally (main-ci-fbf73b6-success): 16 rows of the table above with `result` = `success`.

- `Test (macos-latest / lua54)` failed on
  `e7_review1_probes::e7_review1_a_late_answer_past_the_default_bound_never_touches_an_edited_buffer`:
  `the save waits the default bound and gives up before the answer: 1.545113625s`, outside
  [1 s, 1.2 s), at the row's first save (`tests/e7_review1_probes.rs:284:5`), `test result:
  FAILED. 9 passed; 1 failed; 0 ignored` in 2.48 s. Both of #364's required fragments match:
  **#364's second occurrence**, commented there (issuecomment-6325043178). It was the leg's only
  failure, 209 `running` lines paired with 209 result lines, 208 `ok`, 5,163 passed, 1 failed, 63
  ignored; the steps after its failed test step did not run. The row passed on the four other
  test legs of the run that reached their tests, and on all six of PR #398's run below. E10 does
  not touch that file or the format-on-save path; that is read, not measured.
- `Test (ubuntu-latest / luajit, no crdt)` failed in its step `Install external tools that gate
  acceptance tests (Linux)`, at `scripts/ci-apt-update`, before any build: `Err:6
  https://packages.microsoft.com/ubuntu/24.04/prod noble InRelease` with `403  Forbidden` on each
  of three attempts between 18:56:40Z and 18:56:48Z, `The repository
  'https://packages.microsoft.com/ubuntu/24.04/prod noble InRelease' is no longer signed.`, and
  `ci-apt-update: apt-get update did not complete in 3 attempts (#330)`. Those are not #330's
  fragments (every Azure mirror index ignored, then a stall on the `archive.ubuntu.com` fallback):
  the mirror answered here, Microsoft's repository refused, and the step failed in 8 s. No row or
  issue named it: filed as **#399**, its first occurrence. The three other Ubuntu legs of this run
  and the four of PR #398's run fetched the same file, five of those seven on the same image
  (`ubuntu-24.04` `20261012.334.1`); no cause is claimed. The leg has no `running` or result line.
- `Test (crdt)` ran 18:56:05Z to 19:30:09Z, 34m04s, under its 35-minute ceiling: 209 `running`
  lines paired with 209 `test result: ok`, 5,195 passed, 65 ignored. Not an occurrence of #393's
  shape, and non-reproduction and nothing more.
- The three other test legs pair each `running` line with a `test result: ok`: 210 on each Ubuntu
  leg with `crdt` (5,248 passed, 65 ignored) and 211 on `Test (macos-latest / luajit)` (5,218
  passed, 63 ignored). `WouldBlock` appears in no test leg's log.

`Grammar fuzz` 39657125931 at `fbf73b6`, `success`, both arms deciding `run=false`: `ubsan`
119420249281, `asan-strict` 119420249096.

#### PR #398's `CI` 39658370643 at `550ffe5`: red on #400's first, and `Test (crdt)` green past 35 minutes

`CI` 39658370643, attempt 1 of 1, `pull_request`, testing the merge `8224bee` into `fbf73b6`,
created 19:04:14Z and completed 19:43:13Z (the API's `updated_at`): `failure`.

Tally (pr398-ci-550ffe5): 19 = 17 + 1 + 1.

Tally (pr398-ci-550ffe5-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 119424569803 | success |
| Commit attribution (D9) | 119424570347 | success |
| Lint (lua54) | 119424570372 | success |
| Changed paths | 119424570485 | success |
| Lint (luajit) | 119424570651 | success |
| Perf budgets (debug) | 119424688600 | success |
| M4 Perf Gates | 119424688662 | success |
| M10 Perf Gates (crdt) | 119424688685 | success |
| Test (crdt) | 119424688708 | success |
| M6 Perf Gates | 119424688827 | success |
| M5 Perf Gates | 119424688873 | success |
| Test (macos-latest / luajit) | 119424688961 | success |
| Test (ubuntu-latest / luajit) | 119424689026 | success |
| M1 Acceptance Gates | 119424689027 | success |
| GPU Render (headless) | 119424689042 | success |
| Test (ubuntu-latest / lua54) | 119424689118 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 119424689120 | success |
| Docs consistency | 119424690393 | skipped |
| Test (macos-latest / lua54) | 119424690838 | success |

Tally (pr398-ci-550ffe5-success): 17 rows of the table above with `result` = `success`.

- `Test (crdt)` ran 19:04:32Z to 19:40:58Z, 36m26s: past the 35-minute ceiling it ran under on
  `main`, inside the branch's 45, and green, 209 `running` lines paired with 209 `test result:
  ok`, 5,195 passed, 65 ignored. That is the measurement #393 asked for, not an occurrence of its
  shape, which is a cancellation.
- `Test (ubuntu-latest / lua54)` failed on
  `m6_8_multi_repl_acceptance::m6_8_supervisor_reaps_all_children_across_cycles`: `pump predicate
  did not become true within 5000ms`, the predicate ending `return done(_G.pid1) and
  done(_G.pid2) and done(_G.pid3)`, `test result: FAILED. 7 passed; 1 failed; 0 ignored` in 6.17 s.
  No row or open issue named the selector (`m6_5_repl_spawns_zsh`'s entry shares the harness's
  message, not the selector): filed as **#400**, its first occurrence. It was the leg's only
  failure, 208 `running` lines paired with 208 result lines, 207 `ok`, 5,193 passed, 1 failed, 65
  ignored. The PR changes `.github/workflows/ci.yml` alone and the row's file last changed at
  `fb14dc9` (2026-07-31); that is an argument from untouched code. The row passed on the run's
  five other test legs and on the five legs of `main`'s run above that reached their tests.
- The four other test legs pair each `running` line with a `test result: ok`: 210 on `Test
  (ubuntu-latest / luajit)` (5,248 passed, 65 ignored), 210 on the no-crdt leg (4,691 passed, 51
  ignored) and 211 on each macOS leg (5,218 passed, 63 ignored); `WouldBlock` appears in none.

`Grammar fuzz` 39658370559 at `550ffe5`, `success`, both arms deciding `run=false`: `ubsan`
119424570365, `asan-strict` 119424570232.

#### `main` at `76343d7`, #398's squash: red on #363's third

`76343d7` is the owner's squash of #398 at its head `550ffe5`, pinned with `--match-head-commit`
and merged 2026-10-22T09:04:39Z past #400's red under the owner's ruling, with the required-check
bypass; signature `E`, trailers empty. The branch's base was `1773b85`, and the squash's diff
from `fbf73b6` is byte-identical to `550ffe5`'s diff from its parent, `.github/workflows/ci.yml`
alone, so the squash is faithful. Its `Fixes #393` closed #393 at 09:04:41Z.

`CI` 39752614434 at `76343d7`, attempt 1 of 1, `push`, created 09:04:42Z and completed 09:40:15Z
(the API's `updated_at`): `failure`.

Tally (main-ci-76343d7): 19 = 17 + 1 + 1.

Tally (main-ci-76343d7-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (lua54) | 119736647619 | success |
| Commit attribution (D9) | 119736647969 | success |
| Lint (luajit) | 119736647985 | success |
| Format | 119736648035 | success |
| Changed paths | 119736648072 | success |
| M1 Acceptance Gates | 119736725496 | success |
| M6 Perf Gates | 119736725549 | success |
| M4 Perf Gates | 119736725588 | success |
| Test (ubuntu-latest / lua54) | 119736725641 | success |
| Test (crdt) | 119736725654 | success |
| GPU Render (headless) | 119736725709 | success |
| Test (ubuntu-latest / luajit) | 119736725720 | success |
| Test (macos-latest / luajit) | 119736725766 | success |
| Test (ubuntu-latest / luajit, no crdt) | 119736725777 | success |
| Test (macos-latest / lua54) | 119736725858 | failure |
| M5 Perf Gates | 119736725883 | success |
| M10 Perf Gates (crdt) | 119736725902 | success |
| Perf budgets (debug) | 119736726053 | success |
| Docs consistency | 119736727411 | skipped |

Tally (main-ci-76343d7-success): 17 rows of the table above with `result` = `success`.

- `Test (macos-latest / lua54)` failed on
  `e7c_indicator_acceptance::e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`:
  `quiet before the request` at `tests/e7c_indicator_acceptance.rs:694:5`, `left: Some("⋯1
  parse rust                   ")` against `right: None`, `test result: FAILED. 4 passed; 1
  failed; 0 ignored` in 7.51 s: all three of #363's fragments, **#363's third occurrence**,
  commented there (issuecomment-6337698750). It was the leg's only failure, 209 `running` lines
  paired with 209 result lines, 208 `ok`, 5,163 passed, 1 failed, 63 ignored; the steps after its
  failed test step did not run. The row passed on the run's five other test legs. The squash
  changes `ci.yml` alone; that is read, not measured.
- `Test (crdt)` ran 09:04:56Z to 09:40:14Z, 35m18s, past the 35-minute ceiling `76343d7` replaced
  and inside its 45, green: 209 `running` lines paired with 209 `test result: ok`, 5,195 passed,
  65 ignored. It is the second run past 35 minutes, after PR #398's own above.
- The four other test legs pair each `running` line with a `test result: ok`: 210 on each Ubuntu
  leg with `crdt` (5,248 passed, 65 ignored), 210 on the no-crdt leg (4,691 passed, 51 ignored)
  and 211 on `Test (macos-latest / luajit)` (5,218 passed, 63 ignored); `WouldBlock` appears in no
  test leg's log. #364's and #400's rows passed on all six legs.

`Grammar fuzz` 39752614430 at `76343d7`, `success`, both arms deciding `run=false`: `ubsan`
119736647802, `asan-strict` 119736649535.

#### E10 review 1's gate: listed, not tallied

`scripts/gate --protocol` `20261021T172439Z-1492132` at `0ff624e` plus the review's untracked
probe file `tests/e10_review1_probes.rs`, E10 review 1's independent gate: six of eight, `fmt`,
both lints, `doc`, `build` and `diff-check` ok, each sweep pairing 219 `running` lines with as many
result lines. Its temporary root was `~/build/pmacs-gate-targets/tmp/Zr0UmU`. Each sample is on
its issue. None moves a count.

- `06-sweep`, 5,632 / 2 / 65: #311's fragments on
  `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (`4 passed; 1 failed; 0
  ignored` in 126.23 s at result line 3577), commented on #311 (issuecomment-6323950264); and
  #361's mechanism on a fourth row,
  `e8b_own_name_acceptance::c_x_b_reaches_a_dired_buffer_by_its_own_name_beside_files_and_d18_s_four`
  (`premise: nothing matches \`zzz\`` at `tests/e8b_own_name_acceptance.rs:389:5`, `6 passed; 1
  failed; 0 ignored` in 0.33 s at result line 4049).
- `07-sweep-luajit`, 5,067 / 3 / 51: #311 on the same row (129.73 s, line 3279); #361's third row
  again, `c_x_k_keeps_d18_s_four` (`:622:5`, 1.42 s, line 3709), both #361 samples commented
  there (issuecomment-6323950708); and #304's on `pmacs-gpu`'s
  `tests::caret_inside_a_span_shows_source_exactly_as_if_math_were_disabled` (`380 passed; 1
  failed; 0 ignored` in 23.71 s, line 6607), 44 pixels differing at x 102–103, y 16–37 of a
  480×120 frame, commented on #304 (issuecomment-6323951143).

So, at E10's and #398's merges: `main` is red at `fbf73b6` on #364's second occurrence and #399's
first, and red at `76343d7` on #363's third; PR #398's one red is #400's first, and it merged past
it under the owner's ruling. By CI: #363 moves to three and #364 to two; #399 and #400 are filed
at one each; #393 is closed by #398, whose `Test (crdt)` then ran 36m26s on the pull request and
35m18s on `main`, both inside the new ceiling. #304, #311 and #361 met only local samples here and
do not move. Every other count above stands.

### E10b's close: PR #403 green at `0e67309`, `main` green at `31001df`, the gates' samples listed, #401 and #402 filed

Recorded 2026-10-22 at E10b's checkpoint, PR #403 (three signed commits on `31001df`: E10 review 1's
probes, #395's `open_note` keeping the note's snapshot, and #394's byte-ceiling row waiting for its
first chunk). Every run was read from the jobs endpoint by conclusion, and every test leg of the
code-bearing run through its own log, fetched with escape sequences allowed and stripped, each
`running` line paired with its own result line. The gates were read from their log directories under
`~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/` (copied to `~/build/e10b/gate-logs/`).
Every run id, job id and gate log below was searched for in this file before it was written, and none
was here. Nothing was rerun.

#### `main` at `31001df`, the record of E10's and #398's merges: green

`31001df` is the registry commit above; it changes this file alone. PR #403's branch starts from it.

`CI` 39757854769 at `31001df`, attempt 1 of 1, `push`, created 09:45:17Z and completed 09:47:41Z
(the API's `updated_at`): `success`, every test leg skipped, `Docs consistency` 119754407730 among
the six.

Tally (main-ci-31001df): 15 = 6 + 9.

`Grammar fuzz` 39757854612 at `31001df`, `success`, both arms deciding `run=false`: `ubsan`
119754318676, `asan-strict` 119754319128.

#### PR #403's `CI` 39780182014 at `0e67309`: green

`CI` 39780182014, attempt 1 of 1, `pull_request`, testing the merge `7e70c07` into `31001df`,
created 12:39:44Z and completed 13:16:36Z (the API's `updated_at`): `success`.

Tally (pr403-ci-0e67309): 19 = 18 + 1.

Tally (pr403-ci-0e67309-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 119829144835 | success |
| Format | 119829145181 | success |
| Lint (luajit) | 119829145270 | success |
| Lint (lua54) | 119829145307 | success |
| Commit attribution (D9) | 119829145375 | success |
| GPU Render (headless) | 119829231615 | success |
| Test (crdt) | 119829231626 | success |
| M6 Perf Gates | 119829231631 | success |
| M1 Acceptance Gates | 119829231644 | success |
| Test (macos-latest / luajit) | 119829231710 | success |
| Perf budgets (debug) | 119829231720 | success |
| M4 Perf Gates | 119829231756 | success |
| Test (macos-latest / lua54) | 119829231847 | success |
| M10 Perf Gates (crdt) | 119829231855 | success |
| Test (ubuntu-latest / lua54) | 119829231870 | success |
| Test (ubuntu-latest / luajit) | 119829231992 | success |
| M5 Perf Gates | 119829232171 | success |
| Test (ubuntu-latest / luajit, no crdt) | 119829232553 | success |
| Docs consistency | 119829233784 | skipped |

Tally (pr403-ci-0e67309-success): 18 rows of the table above with `result` = `success`.

- `Test (crdt)` ran 12:39:59Z to 13:14:05Z, 34m06s, inside its 45-minute ceiling: 210 `running`
  lines paired with 210 `test result: ok`, 5,202 passed, 65 ignored, seven more than at `0ff624e`
  (the probe file's two probes and five shared readiness tests).
- The five other test legs pair each `running` line with a `test result: ok`: 211 on each Ubuntu
  leg with `crdt` (5,255 passed, 65 ignored), 211 on the no-crdt leg (4,691 passed, 51 ignored)
  and 212 on each macOS leg (5,225 passed, 63 ignored). `WouldBlock` appears in no test leg's log.
  #394's byte-ceiling row passed on all six legs and #395's E9.2 row on the five built with `crdt`;
  #401's and #402's rows passed on all six.

`Grammar fuzz` 39780181453 at `0e67309`, `success`, both arms deciding `run=false`: `ubsan`
119829143500, `asan-strict` 119829143299.

#### E10b's gates: listed, not tallied

Four default-plan gates (`nice -n 19 scripts/gate`) on `e10b/test-only-fixes`, each five of six,
`fmt`, `clippy`, `doc`, `build` and `diff-check` ok and no daemon surviving, each sweep pairing 219
`running` lines with as many result lines, `WouldBlock` in none. Each sample is on its issue. None
moves a count.

- `20261022T095653Z-2300135` at `3d3c257`, `05-sweep` 5,632 / 2 / 65: #311's fragments on
  `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (`4 passed; 1 failed; 0
  ignored` in 126.60 s at result line 3577), commented on #311 (issuecomment-6338337107); and
  `m8_7_acceptance::magit_push_gesture_pushes_to_configured_remote` (`async pump deadline exceeded
  after 5s` at `tests/m8_7_acceptance.rs:52:9`, then `git push exited, but another process still
  holds its output; waited 2.0 s, then left 1 reader thread blocked on it`, `15 passed; 1 failed; 0
  ignored` in 7.42 s at line 6141): no row or issue named it, filed as **#401**, its first
  occurrence.
- `20261022T102250Z-2625500` at `6ebc022`, 5,633 / 1 / 65: #311 (128.39 s, line 3577), commented
  (issuecomment-6338738008).
- `20261022T104356Z-2855967` at `0e67309`, 5,631 / 3 / 65, a sweep the laptop suspended in, from
  10:46:25Z to 12:20:52Z by the journal: #311 (127.07 s, line 3577), commented
  (issuecomment-6340930207); and two rows of `e7h_grammar_gate_acceptance` together,
  `e7h_a_planted_crash_is_found_and_fails_the_run` and
  `e7h_a_crash_only_a_sequence_brings_back_is_found_and_fails_the_run`, each report reading
  `"crashes=0"`, `"in_sequence=0"`, `"unconfirmed=1"` and ``**lua** crash `signal 6`, reproduced:
  no``, `21 passed; 2 failed; 2 ignored` in 102.17 s at line 3705, a suite that ran after the thaw:
  filed together as **#402**, its first occurrence. The second selector is #339's, whose fragment is
  a hang over 1000 ms not reproduced, so it is not #339's by fragments; cross-referenced there
  (issuecomment-6340929720).
- `20261022T123831Z-3032334` at `0e67309`, the tip, 5,633 / 1 / 65: **#291's** fragments on the same
  row (`HLS reports the type error; store []`, the trace reaching `LSP:ready·setting` and never
  `LSP:ready·processi`, 128.31 s, line 3577), commented on #291 (issuecomment-6341998630). This line
  first read "#311", and the comment on #311 (issuecomment-6341747741) quoted a `Could not find
  module ‘Prelude’` store the log does not contain; corrected in place on 2026-10-22, the comment
  with it.

So, at E10b's close: `main` is green at `31001df` and PR #403 green at its head, and nothing moves by
CI. #401 and #402 are filed on local first samples and stand at no CI occurrence; #291 and #311 met
only local samples here and do not move. Every other count above stands.

### E10b fix round 1: PR #403 green at `c45ae5a`, `main` green at `5e5cdb5` and `d2f2238`, the gates' samples listed, #404 filed

Recorded 2026-10-24 at E10b's fix round 1, PR #403 (three more signed commits on `0e67309`: E10b
review 1's byte-ceiling probes, the byte-ceiling row's liveness reading taken from a marker its
producer writes, and the review's E9 patch, a committed row holding #395's order). Every run was
read from the jobs endpoint by conclusion, and every test leg of the code-bearing run through its
own log, fetched with escape sequences allowed and stripped, each `running` line paired with its
own result line. The gates were read from their log directories under
`~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/`. Every run id, job id and gate log below
was searched for in this file before it was written, and none was here. Nothing was rerun.

#### `main` at `5e5cdb5` and `d2f2238`, E10b's close and its correction: green

`5e5cdb5` is the section above as first written and `d2f2238` its correction (the tip gate's HLS
sample #291's, not #311's); each changes this file alone.

`CI` 39786466006 at `5e5cdb5`, attempt 1 of 1, `push`, created 2026-10-22T13:23:54Z and completed
13:26:15Z (the API's `updated_at`): `success`, every test leg skipped, `Docs consistency`
119850975152 among the six.

Tally (main-ci-5e5cdb5): 15 = 6 + 9.

`Grammar fuzz` 39786465813 at `5e5cdb5`, `success`, both arms deciding `run=false`: `ubsan`
119850884517, `asan-strict` 119850883796.

`CI` 39788485825 at `d2f2238`, attempt 1 of 1, `push`, created 2026-10-22T13:37:32Z and completed
13:39:25Z: `success`, every test leg skipped, `Docs consistency` 119857961474 among the six.

Tally (main-ci-d2f2238): 15 = 6 + 9.

`Grammar fuzz` 39788485817 at `d2f2238`, `success`, both arms deciding `run=false`: `ubsan`
119857866537, `asan-strict` 119857866794.

#### PR #403's `CI` 40107778782 at `c45ae5a`: green

`CI` 40107778782, attempt 1 of 1, `pull_request`, testing the merge `31f213a` into `d2f2238`,
created 2026-10-24T15:36:53Z and completed 16:11:14Z (the API's `updated_at`): `success`.

Tally (pr403-ci-c45ae5a): 19 = 18 + 1.

Tally (pr403-ci-c45ae5a-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 120912941988 | success |
| Commit attribution (D9) | 120912942050 | success |
| Lint (lua54) | 120912942079 | success |
| Changed paths | 120912942126 | success |
| Lint (luajit) | 120912942216 | success |
| M4 Perf Gates | 120912986699 | success |
| Test (crdt) | 120912986727 | success |
| M1 Acceptance Gates | 120912986737 | success |
| Test (macos-latest / luajit) | 120912986745 | success |
| GPU Render (headless) | 120912986751 | success |
| Perf budgets (debug) | 120912986754 | success |
| Test (ubuntu-latest / luajit) | 120912986766 | success |
| Test (macos-latest / lua54) | 120912986782 | success |
| M6 Perf Gates | 120912986814 | success |
| M5 Perf Gates | 120912986826 | success |
| M10 Perf Gates (crdt) | 120912986829 | success |
| Test (ubuntu-latest / luajit, no crdt) | 120912986856 | success |
| Test (ubuntu-latest / lua54) | 120912986907 | success |
| Docs consistency | 120912988088 | skipped |

Tally (pr403-ci-c45ae5a-success): 18 rows of the table above with `result` = `success`.

- `Test (crdt)` ran 15:37:09Z to 16:11:13Z, 34m04s, inside its 45-minute ceiling: 211 `running`
  lines paired with 211 `test result: ok`, 5,210 passed, 65 ignored, eight more than at `0e67309`
  (the new probe file's two probes and five shared readiness tests, and the E9 forced-order row).
- The five other test legs pair each `running` line with a `test result: ok`: 212 on each Ubuntu
  leg with `crdt` (5,263 passed, 65 ignored), 212 on the no-crdt leg (4,698 passed, 51 ignored,
  seven more, the E9 suite compiling to nothing there) and 213 on each macOS leg (5,233 passed, 63
  ignored). `WouldBlock` appears in no test leg's log. The byte-ceiling row and both new probes
  passed on all six legs; the E9 forced-order row and E9.2's on the five built with `crdt`; #404's
  row on the four Linux legs built with `crdt` (compiled for Linux alone).

`Grammar fuzz` 40107778778 at `c45ae5a`, `success`, both arms deciding `run=false`: `ubsan`
120912942068, `asan-strict` 120912941973.

#### E10b review 1's gate, and a gate found in the target: listed, not tallied

Two default-plan gates at `0e67309`, each five of six on the sweep alone, `fmt`, `clippy`, `doc`,
`build` and `diff-check` ending clean in their logs, each sweep pairing 219 `running` lines with
as many result lines, `WouldBlock` in none. Each sample is on its issue. None moves a count.

- `20261023T213213Z-24836`, E10b review 1's own, no daemon surviving (the review's record),
  `05-sweep` 5,633 / 1 / 65: #311's fragments on
  `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (`4 passed; 1 failed; 0
  ignored` in 126.30 s at result line 3577), commented on #311 (issuecomment-6386175435).
- `20261023T172159Z-2471370`, found in the target by review 1 and named in no record before it,
  at `0e67309` by the worktree's reflog; its survivor check's verdict went to an output nobody
  kept. `05-sweep` 5,632 / 2 / 65: #311 on the same row (129.74 s, line 3610), on the same
  comment; and #316's under the template reading on
  `e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer` (`kill 2:
  nothing held the switch's follow-up parse`, `busy=false`, `tree=true`, `unit=pid_…`,
  `parse:killed`, `10 passed; 1 failed; 0 ignored` in 14.62 s at line 3950), commented on #316
  (issuecomment-6386175803).

#### E10b fix round 1's gate and hand runs: listed, not tallied, #404 filed

- `20261024T153639Z-103802` at `c45ae5a`, the default plan: five of six, `fmt`, `clippy`, `doc`,
  `build` and `diff-check` ok, no daemon survived (identified by environ); `05-sweep` 5,639 / 3 /
  65, 220 `running` lines paired with 220 result lines, `WouldBlock` in none. #311 on the HLS row
  (145.11 s, line 3590), commented (issuecomment-6386290929); #316's under the template reading
  (`kill 2`, `busy=true`, 17.38 s, line 3930), commented (issuecomment-6386291359); and
  `e9_session_acceptance::write_failure_drain::e9_fr4_a_failed_write_drains_input_until_the_deadline_and_no_later`
  (`nothing was written to the frontend after its failed write`, `left: 2`, `right: 1`, the daemon
  logging `BufferSnapshot publish for BufferId(3) to FrontendId(3) failed` before `write failed
  for FrontendId(3) in dispatcher`, `27 passed; 1 failed; 0 ignored` in 9.30 s at line 4352):
  **#404**'s second occurrence, commented (issuecomment-6386338656).
- Hand runs in the gate's environment. #404's first occurrence, a direct run of the E9 suite at
  `c45ae5a`'s tree, `27 passed; 1 failed` in 10.15 s, no row or issue naming the selector: filed
  as **#404**. Then an A/B on #404 (issuecomment-6386338656): two builds of the suite started six at
  a time, fifteen batches; the row failed in 6 of 45 copies at `0e67309`'s suite and 10 of 45 at
  `c45ae5a`'s, every one on #404's fragments, beside
  `e9_5_an_oversize_snapshot_is_refused_with_a_visible_status_not_skipped` timing out at its 10 s
  wait in 86 of the 90 under that load, the instrument's and not filed.

So, at E10b's fix round 1: `main` is green at `5e5cdb5` and `d2f2238` and PR #403 green at its new
head, and nothing moves by CI. #404 is filed on local samples and stands at no CI occurrence; #311
and #316 met only local samples here and do not move. Every other count above stands.

### E10c's close: PR #407 green at `0adb83e`, `main` green at `1eea479`, the gates' samples listed, #405 and #406 filed

Recorded 2026-10-25 at E10c's checkpoint, PR #407 (six signed commits on `1eea479`: buffer names
by basename through one function, dired's listings named `proj/` through it, the theme rows'
fixture path without an extension, `C-x b` and `C-x k` ranked on names alone, `*buffer-list*`'s
`File` column, the headless probe's `frame:` step). Every run was read from the jobs endpoint by
conclusion, and every test leg of the code-bearing runs through its own log, fetched with escape
sequences allowed and stripped, each `running` line paired with its own result line. The gates
were read from their log directories under `~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/`.
Every run id, job id and gate log below was searched for in this file before it was written, and
none was here. Nothing was rerun.

#### `main` at `1eea479`, E10b's squash: green

`1eea479` is PR #403 squashed by the owner from `c45ae5a`, its tree `c45ae5a`'s but for this file.

`CI` 40194991518, attempt 1 of 1, `push`, created 2026-10-25T12:51:40Z and completed 13:25:28Z
(the API's `updated_at`): `success`.

Tally (main-ci-1eea479): 19 = 18 + 1.

Tally (main-ci-1eea479-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 121173800117 | success |
| Format | 121173800283 | success |
| Lint (lua54) | 121173800305 | success |
| Commit attribution (D9) | 121173800319 | success |
| Lint (luajit) | 121173800409 | success |
| M1 Acceptance Gates | 121173844377 | success |
| Test (macos-latest / luajit) | 121173844397 | success |
| M5 Perf Gates | 121173844398 | success |
| Test (ubuntu-latest / luajit) | 121173844401 | success |
| Perf budgets (debug) | 121173844403 | success |
| M6 Perf Gates | 121173844407 | success |
| GPU Render (headless) | 121173844418 | success |
| Test (macos-latest / lua54) | 121173844420 | success |
| M10 Perf Gates (crdt) | 121173844422 | success |
| Test (ubuntu-latest / luajit, no crdt) | 121173844424 | success |
| M4 Perf Gates | 121173844441 | success |
| Test (crdt) | 121173844443 | success |
| Test (ubuntu-latest / lua54) | 121173844459 | success |
| Docs consistency | 121173845539 | skipped |

Tally (main-ci-1eea479-success): 18 rows of the table above with `result` = `success`.

- `Test (crdt)` ran 12:51:56Z to 13:25:27Z, 33m31s, inside its 45-minute ceiling: 211 `running`
  lines paired with 211 `test result: ok`, 5,210 passed, 65 ignored, as at `c45ae5a`.
- The five other test legs pair each `running` line with a `test result: ok`: 212 on each Ubuntu
  leg with `crdt` (5,263 passed, 65 ignored), 212 on the no-crdt leg (4,698 passed, 51 ignored)
  and 213 on each macOS leg (5,233 passed, 63 ignored), each as at `c45ae5a`. `WouldBlock` appears
  in no test leg's log.

`Grammar fuzz` 40194991538 at `1eea479`, `success`, both arms deciding `run=false`: `ubsan`
121173800338, `asan-strict` 121173800422.

#### PR #407's `CI` 40226556303 at `0adb83e`: green

`CI` 40226556303, attempt 1 of 1, `pull_request`, testing the merge `dd4541b` into `1eea479`,
created 2026-10-25T19:33:00Z and completed 20:07:09Z (the API's `updated_at`): `success`.

Tally (pr407-ci-0adb83e): 19 = 18 + 1.

Tally (pr407-ci-0adb83e-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 121269042236 | success |
| Commit attribution (D9) | 121269042325 | success |
| Format | 121269042419 | success |
| Lint (lua54) | 121269042492 | success |
| Lint (luajit) | 121269042503 | success |
| GPU Render (headless) | 121269077893 | success |
| M6 Perf Gates | 121269077895 | success |
| Test (crdt) | 121269077905 | success |
| M5 Perf Gates | 121269077909 | success |
| M1 Acceptance Gates | 121269077942 | success |
| Test (ubuntu-latest / lua54) | 121269077961 | success |
| Perf budgets (debug) | 121269077967 | success |
| M4 Perf Gates | 121269077975 | success |
| Test (macos-latest / lua54) | 121269077978 | success |
| Test (macos-latest / luajit) | 121269077988 | success |
| M10 Perf Gates (crdt) | 121269077993 | success |
| Test (ubuntu-latest / luajit) | 121269078017 | success |
| Test (ubuntu-latest / luajit, no crdt) | 121269078045 | success |
| Docs consistency | 121269079519 | skipped |

Tally (pr407-ci-0adb83e-success): 18 rows of the table above with `result` = `success`.

- `Test (crdt)` ran 19:33:12Z to 20:07:08Z, 33m56s, inside its 45-minute ceiling: 212 `running`
  lines paired with 212 `test result: ok`, 5,223 passed, 65 ignored, one pair and thirteen more
  than at `1eea479` (the new suite's seven rows, the naming function's eight unit rows, the
  ranker's five unit rows replaced by three).
- The five other test legs pair each `running` line with a `test result: ok`, each one pair and
  thirteen tests more than at `1eea479`: 213 on each Ubuntu leg with `crdt` (5,276 passed, 65
  ignored), 213 on the no-crdt leg (4,711 passed, 51 ignored) and 214 on each macOS leg (5,246
  passed, 63 ignored). `WouldBlock` appears in no test leg's log. The new suite and the naming
  oracle passed on all six legs; the six refused-query rows on all six, but for the GPU row on the
  no-crdt leg, where its suite compiles to nothing.

`Grammar fuzz` 40226556311 at `0adb83e`, `success`, both arms deciding `run=false`: `ubsan`
121269042574, `asan-strict` 121269042298.

#### E10c's gates: listed, not tallied, #405 and #406 filed

Eight default-plan gates, one at a time, each on the sweep alone where it finished: `fmt`,
`clippy`, `doc`, `build` and `diff-check` ending clean in their logs, each finished sweep pairing
its `running` lines with as many result lines, `WouldBlock` in none, no daemon surviving
(identified by environ). Each sample is on its issue. None moves a count.

- `20261025T141536Z-1015372` at the base `1eea479`: `05-sweep` 5,641 / 1 / 65 at 220 pairs; #311's
  fragments on `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (the `Prelude`
  store, the trace reaching `processi`; 133.33 s at result line 3590).
- `20261025T151421Z-1425604` at `b7fb796`: 5,651 / 2 / 65 at 221; #311 on the HLS row (134.73 s,
  line 3622); and `process::tests::a_reaped_pty_child_s_output_holds_only_what_it_wrote`, which no
  row or issue named: filed as **#405**.
- `20261025T153800Z-1569834` at `c2594d1`: cut by a reboot of the laptop, the sweep's exit 143 at 101
  `running` lines and 100 results; no evidence.
- `20261025T165242Z-17612` at `c2594d1`: 5,652 / 4 / 65 at 221; #311 (134.41 s, line 3610); #316's
  under the template reading on
  `e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer` (`kill 2`,
  `busy=true`, `tree=true`, `unit=pid_…`, `parse:killed`, 16.74 s at line 3973); the release
  replay's stand-in worker on `Broken pipe` (line 3887), which no row or issue named: filed as
  **#406**; and `theme_faces_acceptance::snapshot_round_trip_restores_the_themed_summary_at_one_generation`
  (line 6684), the branch's own, fixed by its `0f9eb76`.
- `20261025T171517Z-189362` at `0f9eb76`: 5,654 / 2 / 65 at 221; the HLS row on #291's fragments
  (`store []`, the trace to `LSP:ready·setting` and no `processi`; 134.77 s); and
  `e9_session_acceptance::write_failure_drain::e9_fr4_a_failed_write_drains_input_until_the_deadline_and_no_later`
  (`left: 2`, `right: 1`, the `BufferSnapshot` publish failing before the dispatcher's write; line
  4359): #404's third local sample.
- `20261025T175110Z-377854` at `ab6086f`: 5,652 / 2 / 65 at 221; #311 (127.63 s, line 3608); and
  `e8d_drift_acceptance::e8d_1_a_respawn_whose_executable_is_gone_is_told_as_a_crash_and_re_armed`
  (`spawn the server: "spawn: Text file busy (os error 26)"`, `13 passed; 1 failed` at line 4178):
  #250's mechanism on another selector, not #250's first signature by its fragments.
- `20261025T181821Z-532283` at `619d68f`: 5,654 / 1 / 65 at 221; #311 (125.83 s, line 3609).
- `20261025T184818Z-649277` at the tip `0adb83e`: 5,653 / 2 / 65 at 221; #311 (126.16 s, line
  3609); #316's under the template reading (`kill 3`, `busy=false`, `tree=true`, `unit=pid_…`,
  `parse:killed`, line 3949).

Hand runs in the gate's environment, listed: #316's row in an A/B, 7 of 24 failing alone at the
tip and 5 of 24 with whole-path names planted back, every red on #316's assertion with
`parse:killed` on the line; and the row in E10c.2's first sweep by hand (`kill 3`) and in its
rerun (`kill 2`), both under the template reading.

Comments: #311 issuecomment-6402199493 (the six gates' samples), #291 issuecomment-6402205765,
#316 issuecomment-6402209314 (with the A/B), #404 issuecomment-6402212126, #250
issuecomment-6402215869.

So, at E10c's close: `main` is green at `1eea479` and PR #407 green at its head, and nothing moves by CI.
#405 and #406 are filed on local first samples and stand at no CI occurrence; #250, #291, #311,
#316 and #404 met only local samples here and do not move. Every other count above stands.

### E10c fix round 1: PR #407 red on #373 at `4a13368`, `main` green at `460fded`, the gates' samples listed

Recorded 2026-10-26 at E10c's fix round 1, PR #407 (eight more signed commits on `0adb83e`: buffer
names recomputed only for the paths an event can move, with a budget opening 2,000 files and review
1's measurement; review 1's probes P1 and P2; a held name kept where buffers share a path, P3;
`*buffer-list*`'s columns by display width with paths in `~` form, P4 and P4b; a `*name*` scored
inside its stars, P6; P5; and two comment corrections). Every run was read from the jobs endpoint by
conclusion, and every test leg of the code-bearing run through its own log, fetched with escape
sequences allowed and stripped, each `running` line paired with its own result line. The gates were
read from their log directories under `~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/`.
Every run id, job id and gate log below was searched for in this file before it was written, and
none was here. Nothing was rerun.

#### `main` at `460fded`, E10c's close: green

`460fded` is the section above, E10c's D38 record; it changes this file alone.

`CI` 40229805152 at `460fded`, attempt 1 of 1, `push`, created 2026-10-25T20:14:13Z and completed
20:16:22Z (the API's `updated_at`): `success`, every test leg skipped, `Docs consistency`
121278945417 among the six.

Tally (main-ci-460fded): 15 = 6 + 9.

`Grammar fuzz` 40229805193 at `460fded`, `success`, both arms deciding `run=false`: `ubsan`
121278896087, `asan-strict` 121278895887.

#### PR #407's `CI` 40249159520 at `4a13368`: red on #373's second occurrence

`CI` 40249159520, attempt 1 of 1, `pull_request`, testing the merge `277c111` into `460fded`,
created 2026-10-26T00:23:51Z and completed 00:58:06Z (the API's `updated_at`): `failure`.

Tally (pr407-ci-4a13368): 19 = 17 + 1 + 1.

Tally (pr407-ci-4a13368-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 121337045645 | success |
| Lint (lua54) | 121337045739 | success |
| Format | 121337045792 | success |
| Commit attribution (D9) | 121337045841 | success |
| Lint (luajit) | 121337045847 | success |
| M1 Acceptance Gates | 121337091182 | success |
| Test (crdt) | 121337091207 | success |
| M6 Perf Gates | 121337091211 | success |
| M5 Perf Gates | 121337091213 | success |
| Test (ubuntu-latest / lua54) | 121337091232 | success |
| Perf budgets (debug) | 121337091265 | success |
| GPU Render (headless) | 121337091271 | success |
| M4 Perf Gates | 121337091303 | success |
| Test (macos-latest / luajit) | 121337091304 | success |
| Test (macos-latest / lua54) | 121337091314 | success |
| M10 Perf Gates (crdt) | 121337091321 | success |
| Test (ubuntu-latest / luajit, no crdt) | 121337091341 | success |
| Test (ubuntu-latest / luajit) | 121337091382 | failure |
| Docs consistency | 121337092535 | skipped |

Tally (pr407-ci-4a13368-success): 17 rows of the table above with `result` = `success`.

- `Test (ubuntu-latest / luajit)` 121337091382, a required check, fails
  `process::tests::release_joins_every_polling_reader_whatever_the_deadline` at
  `src/process.rs:5525:9`, `only the blocking reader on the master is left; the parser is joined`,
  `left: 0`, `right: 1`, the lib's `test result: FAILED. 2309 passed; 1 failed; 13 ignored` (67.53 s):
  **#373's second occurrence**, every required fragment, on the leg of its first, commented there
  (issuecomment-6405908178). Its test step paired 213 `running` lines with 213 results, this its only
  failure; its two later steps (`cargo test --doc`, `cargo test -p pmacs-protocol`) were skipped after
  it. The branch has no diff in `src/process.rs`, and the row passed on the five other test legs and
  on this leg at `0adb83e`; that is an argument from untouched code, not a measurement. Not rerun.
- `Test (crdt)` ran 00:24:07Z to 00:58:05Z, 33m58s, inside its 45-minute ceiling: 214 `running`
  lines paired with 214 `test result: ok`, 5,239 passed, 67 ignored, two pairs, sixteen tests and
  two ignored more than at `0adb83e` (review 1's probe file and its measurement; the naming, registry
  and width unit rows' seven; the E10c suite's two rows and its budget, ignored).
- The four other test legs pair each `running` line with a `test result: ok`, each by the same two
  pairs and sixteen tests more: 215 on `Test (ubuntu-latest / lua54)` (5,292 passed, 67 ignored), 215
  on the no-crdt leg (4,727 passed, 53 ignored), 216 on each macOS leg (5,262 passed, 65 ignored).
  `WouldBlock` appears in no test leg's log. `Perf budgets (debug)` 121337091265 ran the new budget,
  `opening_two_thousand_files_stays_within_budget ... ok`.

`Grammar fuzz` 40249159522 at `4a13368`, `success`, both arms deciding `run=false`: `ubsan`
121337045898, `asan-strict` 121337045442.

#### E10c review 1's gate: listed, not tallied

- `20261025T203113Z-1366491` at `0adb83e`, the default plan in the worktree `pmacs-e8d`: five of six,
  `fmt`, `clippy`, `doc`, `build` and `diff-check` ok, no daemon surviving (identified by environ);
  `05-sweep` 5,653 / 2 / 65, 221 `running` lines paired with 221 result lines, `WouldBlock` in none.
  #311's fragments on `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (a store
  holding `Could not find module ‘Prelude’`, the trace reaching `LSP:ready·processi`; 133.14 s at
  result line 3609), commented (issuecomment-6403415074); and #316's under the template reading on
  `e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer` (`kill 3`,
  `busy=true`, `tree=true`, `unit=pid_…`, `parse:killed`; 17.10 s at line 3949), commented
  (issuecomment-6403415459).

#### E10c fix round 1's gates and hand runs: listed, not tallied

- `20261026T000642Z-1910312` at `2c5abe7`, a first series of the round's commits never pushed (its
  code the head's but for comments), the default plan: five of six, `fmt`, `clippy`, `doc`, `build`
  and `diff-check` ok, no daemon surviving; `05-sweep` 5,670 / 1 / 67, 223 `running` lines paired with
  223 result lines, `WouldBlock` in none; #311's fragments on the HLS row (126.05 s, line 3639).
- `20261026T002212Z-2057387` at the head `4a13368`, the default plan: five of six, the same stages
  ok, no daemon surviving (identified by environ); `05-sweep` 5,670 / 1 / 67 at 223 pairs,
  `WouldBlock` in none; #311's fragments on the HLS row (125.75 s, line 3639). Both #311 samples are
  on one comment (issuecomment-6405681120).
- Hand runs in the gate's environment, every red explained and none filed: the touched suites under
  `lua54,crdt` in a side target without the parse worker, four `m9_8` rows failing on `parse unit
  unavailable: no pmacs-parse-unit beside pmacs`, the instrument's, and 17 of 17 with the worker
  built; the round's controls and plants (in the fixes pass), which failed what they were planted to
  fail.

So, at E10c's fix round 1: `main` is green at `460fded`, and PR #407 is red at its new head on #373's
second occurrence, which takes #373 to two, both on `Test (ubuntu-latest / luajit)`. #311 and #316 met
only local samples here and do not move. Every other count above stands.

### E10d's close: `main` green at `73bc1e6`, PR #408 red then green, the gates' samples listed, #409 filed

Recorded 2026-10-26 at E10d's checkpoint, PR #408 (four signed commits on `73bc1e6`: E10d.1's control
holding #404's order, the one write path, its guard, and the path's unit row put on Linux alone after
the head's first run). Every run was read from the jobs endpoint by conclusion, and every test leg of
the code-bearing runs through its own log, fetched with escape sequences allowed and stripped, each
`running` line paired with its own result line. The gates were read from their log directories
under `~/build/pmacs-gate-targets/pmacs-e8d-831ce4db/gate-logs/` (copied to
`~/build/e10d/gate-logs/`). Every run id, job id and gate log below was searched for in this file
before it was written, and none was here but E10's gate `20261021T115903Z-3334903`, cited as a
precedent. Nothing was rerun.

#### `main` at `73bc1e6`, E10c's squash merge of PR #407: green

`73bc1e6` is the owner's squash of PR #407 at `4a13368`, on `2272d16`; its tree is the head's but
for this file.

`CI` 40312792505 at `73bc1e6`, attempt 1 of 1, `push`, created 2026-10-26T11:08:24Z and completed
11:44:06Z (the API's `updated_at`): `success`.

Tally (main-ci-73bc1e6): 19 = 18 + 1.

Tally (main-ci-73bc1e6-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 121537859696 | success |
| Changed paths | 121537859773 | success |
| Commit attribution (D9) | 121537859793 | success |
| Lint (luajit) | 121537860009 | success |
| Lint (lua54) | 121537860038 | success |
| M6 Perf Gates | 121537939104 | success |
| Test (crdt) | 121537939171 | success |
| Perf budgets (debug) | 121537939197 | success |
| M4 Perf Gates | 121537939235 | success |
| Test (ubuntu-latest / luajit, no crdt) | 121537939253 | success |
| M1 Acceptance Gates | 121537939266 | success |
| Test (ubuntu-latest / lua54) | 121537939348 | success |
| GPU Render (headless) | 121537939365 | success |
| Test (ubuntu-latest / luajit) | 121537939377 | success |
| Test (macos-latest / luajit) | 121537939391 | success |
| M10 Perf Gates (crdt) | 121537939413 | success |
| M5 Perf Gates | 121537939499 | success |
| Test (macos-latest / lua54) | 121537939508 | success |
| Docs consistency | 121537941420 | skipped |

Tally (main-ci-73bc1e6-success): 18 rows of the table above with `result` = `success`.

- The six test legs, each through its own log: `Test (crdt)` 121537939171 ran 11:08:38Z to
  11:43:06Z, 34m28s, inside its 45-minute ceiling, pairing 214 `running` lines with 214 `test
  result: ok`, 5,239 passed, 67 ignored; `Test (ubuntu-latest / luajit, no crdt)` 121537939253, 215
  pairs, 4,727 passed, 53 ignored; `Test (ubuntu-latest / lua54)` 121537939348 and `Test
  (ubuntu-latest / luajit)` 121537939377, 215 pairs each, 5,292 passed, 67 ignored; `Test
  (macos-latest / luajit)` 121537939391 and `Test (macos-latest / lua54)` 121537939508, 216 pairs
  each, 5,262 passed, 65 ignored. `test result: FAILED` and `WouldBlock` appear in no test leg's log.
  #373's row passed on every leg.
- `Perf budgets (debug)` 121537939197: `every budget ran and held`,
  `opening_two_thousand_files_stays_within_budget ... ok`.

`Grammar fuzz` 40312792483 at `73bc1e6`, `success`, both arms deciding `run=false` (`no changed path
can change the grammar set`): `ubsan` 121537859946, `asan-strict` 121537860317.

#### PR #408's `CI` 40337444679 at `f5e79dd`: red on the branch's own row and on #409's first occurrence

`CI` 40337444679, attempt 1 of 1, `pull_request`, testing the merge `6210f9b` into `73bc1e6`,
created 2026-10-26T14:12:14Z and completed 14:57:54Z (the API's `updated_at`): `failure`.

Tally (pr408-ci-f5e79dd): 19 = 15 + 2 + 1 + 1.

Tally (pr408-ci-f5e79dd-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 121622707266 | success |
| Commit attribution (D9) | 121622707599 | success |
| Format | 121622707926 | success |
| Lint (luajit) | 121622707954 | success |
| Lint (lua54) | 121622707963 | success |
| GPU Render (headless) | 121622818182 | success |
| M5 Perf Gates | 121622818197 | success |
| M4 Perf Gates | 121622818315 | success |
| Test (crdt) | 121622818330 | success |
| Test (ubuntu-latest / lua54) | 121622818340 | cancelled |
| M1 Acceptance Gates | 121622818359 | success |
| Test (ubuntu-latest / luajit) | 121622818383 | success |
| M6 Perf Gates | 121622818399 | success |
| Perf budgets (debug) | 121622818456 | success |
| M10 Perf Gates (crdt) | 121622818468 | success |
| Test (ubuntu-latest / luajit, no crdt) | 121622818498 | success |
| Test (macos-latest / lua54) | 121622818529 | failure |
| Test (macos-latest / luajit) | 121622818649 | failure |
| Docs consistency | 121622819978 | skipped |

Tally (pr408-ci-f5e79dd-success): 15 rows of the table above with `result` = `success`.

- `Test (macos-latest / lua54)` 121622818529 and `Test (macos-latest / luajit)` 121622818649 fail
  `daemon::tests::e10d_2_the_path_ends_a_session_s_output_at_its_first_failed_write` alone, at
  `src/daemon.rs:5653:9`, `a write to a peer that stopped reading fails`, the lib's `test result:
  FAILED. 2304 passed; 1 failed; 13 ignored`: the branch's own new row, which needs a write to a peer
  that shut its read side to fail, and macOS lets it succeed (E8d.4's finding). No row or issue; fixed
  by the branch's next head, `fd38a8a`, which compiles the row on Linux alone. Each leg paired 215
  `running` lines with 215 results, 5,210 passed, 1 failed, 65 ignored, its doc-test and
  `pmacs-protocol` steps skipped after the lib's failure.
- `Test (ubuntu-latest / lua54)` 121622818340 is cancelled at its 45-minute ceiling (14:12:31Z to
  14:57:53Z) with no test failed: its tool step's `apt-get install` fetched `15.1 MB in 10min 35s
  (23.7 kB/s)` from the Azure mirror, `Ign:` on four packages refetched later, and took 11m29s
  against 1m49s on `Test (ubuntu-latest / luajit)`; its test step then ran 32m33s to the cancel against
  that leg's 32m32s, pairing 214 `running` lines with 214 results, 5,243 passed, 67 ignored, in its
  last suite. Not #330 by its fragments (#330's `apt-get update` stalls after its last index; here the
  update finished in a second): filed as **#409**, a first occurrence. Not rerun.
- The three other test legs pair each `running` line with a `test result: ok`: `Test (crdt)`
  121622818330 (14:12:30Z to 14:46:40Z, 34m10s) 215 pairs, 5,244 passed, 67 ignored; `Test
  (ubuntu-latest / luajit)` 121622818383, 216, 5,297 / 67; `Test (ubuntu-latest / luajit, no crdt)`
  121622818498, 216, 4,730 / 53; each `main`'s figures at `73bc1e6` plus the branch's new suite and
  rows. `WouldBlock` appears in no test leg's log.

`Grammar fuzz` 40337444755 at `f5e79dd`, `success`, both arms deciding `run=false`: `ubsan`
121622708137, `asan-strict` 121622708599.

#### PR #408's `CI` 40356132391 at `fd38a8a`: green

`CI` 40356132391, attempt 1 of 1, `pull_request`, testing the merge `1619eea` into `73bc1e6`,
created 2026-10-26T16:08:01Z and completed 16:50:19Z (the API's `updated_at`): `success`.

Tally (pr408-ci-fd38a8a): 19 = 18 + 1.

Tally (pr408-ci-fd38a8a-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 121688977284 | success |
| Lint (luajit) | 121688977648 | success |
| Commit attribution (D9) | 121688977728 | success |
| Lint (lua54) | 121688977774 | success |
| Format | 121688977781 | success |
| M1 Acceptance Gates | 121689086248 | success |
| Test (crdt) | 121689086365 | success |
| GPU Render (headless) | 121689086389 | success |
| Test (ubuntu-latest / lua54) | 121689086458 | success |
| M4 Perf Gates | 121689086518 | success |
| Test (ubuntu-latest / luajit) | 121689086521 | success |
| M10 Perf Gates (crdt) | 121689086522 | success |
| Test (macos-latest / lua54) | 121689086529 | success |
| Test (macos-latest / luajit) | 121689086576 | success |
| Test (ubuntu-latest / luajit, no crdt) | 121689086596 | success |
| Perf budgets (debug) | 121689086626 | success |
| M6 Perf Gates | 121689086661 | success |
| M5 Perf Gates | 121689086846 | success |
| Docs consistency | 121689088631 | skipped |

Tally (pr408-ci-fd38a8a-success): 18 rows of the table above with `result` = `success`.

- Every test leg pairs each `running` line with a `test result: ok`: `Test (crdt)` 121689086365
  (16:08:16Z to 16:42:54Z, 34m38s) 215 pairs, 5,244 passed, 67 ignored; `Test (ubuntu-latest /
  luajit)` 121689086521 and `Test (ubuntu-latest / lua54)` 121689086458, 216 each, 5,297 / 67; `Test
  (ubuntu-latest / luajit, no crdt)` 121689086596, 216, 4,730 / 53; `Test (macos-latest / luajit)`
  121689086576 and `Test (macos-latest / lua54)` 121689086529, 217 each, 5,264 / 65. `WouldBlock`
  appears in none. E10d.1's row, the sweep row and E9's drain-deadline row passed on the four Linux
  legs built with `crdt`, the unit row on every Linux leg, the guard on all six. `Perf budgets
  (debug)` 121689086626: `every budget ran and held`.

`Grammar fuzz` 40356132410 at `fd38a8a`, `success`, both arms deciding `run=false`: `ubsan`
121688976372, `asan-strict` 121688977141.

#### E10d's gates and hand runs: listed, not tallied

All four are the default plan in the worktree `pmacs-e8d` on `e10d/write-failure-path`, one per row
and one at the last head, each with `fmt`, `clippy`, `doc`, `build` and `diff-check` ok, no daemon
surviving (identified by environ), every `running` line paired with a result line and `WouldBlock`
in none.

- `20261026T131025Z-2595949` at `f80f6da`: five of six, `05-sweep` 5,669 / 2 / 68 at 223 pairs.
  #311's fragments on `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (a store
  holding `Could not find module ‘Prelude’`, the trace reaching `LSP:ready·processi`; 127.03 s at
  result line 3652); and #256's expression, `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
  panicking at `src/process.rs:5430:53` with `live runtime probe`, the lib's `2309 passed; 1 failed;
  13 ignored` in 14.07 s (line 2343), commented (issuecomment-6418174214).
- `20261026T133722Z-2820910` at `6d98870`: five of six, `05-sweep` 5,672 / 2 / 67 at 223 pairs.
  #311's fragments on the HLS row (126.59 s, line 3640); and
  `e7i_review3_probes::e7i_review3_a_deadline_that_cuts_only_layers_keeps_its_current_spans` on `the
  warm parse installed` at `tests/e7i_review3_probes.rs:665:9`, `10 passed; 1 failed; 0 ignored` in
  15.40 s (line 3978), #340's selector and message with its required attempt trace absent from the
  log, so not #340's by its fragments, as at `20261021T115903Z-3334903`; commented there
  (issuecomment-6418174652).
- `20261026T135553Z-3018929` at `f5e79dd`: five of six, `05-sweep` 5,675 / 1 / 67 at 224 pairs;
  #311's fragments on the HLS row (127.07 s, line 3648). These three #311 samples are on one comment
  (issuecomment-6418173482).
- `20261026T160813Z-3365621` at the head `fd38a8a`: five of six, `05-sweep` 5,675 / 1 / 67 at 224
  pairs; #311's fragments on the HLS row (128.11 s, line 3648), commented (issuecomment-6421344645).
- Hand runs in the gate's environment: E10d.1's control at `73bc1e6`'s daemon, 10 of 10 failing
  with #404's fragments, and 10 of 10 passing with the fix; the bites and plants in the handoff, each
  failing what it was planted to fail.

So, at E10d's close: `main` is green at `73bc1e6`, and PR #408 is green at its head `fd38a8a` after a
first head red on the branch's own row on both macOS legs and on #409's first occurrence. #311, #256
and #340's selector met only local samples here and do not move; no count moves.

### E10d fix round 1: PR #408 red on #282's eighth at `50005da`, `main` green at `658051f`, the gates' samples listed

Recorded 2026-10-26 at E10d's fix round 1, PR #408 (four signed commits on `fd38a8a` answering
review 1: a drain only on a socket error, a session's stream private to the registry, review 1's
P1, P1a and P2 committed, and a witness of the mid-loop skip). Every run was read from the jobs
endpoint by conclusion, and every test leg of the code-bearing run through its own log, fetched
with escape sequences allowed and stripped, each `running` line paired with its own result line.
The gates were read from their log directories (review 1's copied to
`~/build/e10d-r1/gate-20261026T180609Z-3811177/`, this round's to
`~/build/e10d/fix1/gate-20261026T200159Z-144875/`). Every run id, job id, gate log and comment
below was searched for in this file before it was written, and none was here. Nothing was rerun.

#### `main` at `658051f`, E10d's close: green

`658051f` is the section above, E10d's D38 record; it changes this file alone.

`CI` 40364393025 at `658051f`, attempt 1 of 1, `push`, created 2026-10-26T16:59:33Z and completed
17:02:03Z (the API's `updated_at`): `success`, every test leg skipped, `Docs consistency`
121718158974 among the six.

Tally (main-ci-658051f): 15 = 6 + 9.

`Grammar fuzz` 40364395528 at `658051f`, `success`, both arms deciding `run=false`: `ubsan`
121718057689, `asan-strict` 121718057966.

#### PR #408's `CI` 40392704632 at `50005da`: red on #282's eighth occurrence

`CI` 40392704632, attempt 1 of 1, `pull_request`, testing the merge `a18f1a0` into `658051f`,
created 2026-10-26T20:06:40Z and completed 20:46:15Z (the API's `updated_at`): `failure`.

Tally (pr408-ci-50005da): 19 = 17 + 1 + 1.

Tally (pr408-ci-50005da-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Lint (luajit) | 121817565698 | success |
| Lint (lua54) | 121817566084 | success |
| Changed paths | 121817566134 | success |
| Commit attribution (D9) | 121817566313 | success |
| Format | 121817566561 | success |
| GPU Render (headless) | 121817676586 | success |
| Test (crdt) | 121817676645 | success |
| M1 Acceptance Gates | 121817676735 | success |
| M4 Perf Gates | 121817676747 | success |
| M10 Perf Gates (crdt) | 121817676800 | success |
| M5 Perf Gates | 121817676849 | success |
| Test (macos-latest / luajit) | 121817676917 | success |
| Perf budgets (debug) | 121817676918 | success |
| M6 Perf Gates | 121817677054 | success |
| Test (ubuntu-latest / lua54) | 121817677062 | success |
| Test (macos-latest / lua54) | 121817677108 | success |
| Test (ubuntu-latest / luajit) | 121817677142 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 121817677378 | success |
| Docs consistency | 121817680567 | skipped |

Tally (pr408-ci-50005da-success): 17 rows of the table above with `result` = `success`.

Tally (pr408-ci-50005da-failure): 1 row of the table above with `result` = `failure`.

- `Test (ubuntu-latest / luajit)` 121817677142, `failure`, a required check:
  `e7b_review_wire_acceptance::a_rename_on_whitespace_leaves_the_label_ready_and_reports_to_errors`,
  **#282's eighth occurrence**, all three fragments read against the job's own log: `*lsp* names the request and the code: []` at
  `tests/e7b_review_wire_acceptance.rs:1214:5`; `WIRE *errors* gained ["[lsp] LSP: default-rust
  refused textDocument/prepareRename as a client error, -32602 InvalidParams: No references found
  at position"]` and `WIRE label "ready", last_error None, *lsp* errors []`; and `test result:
  FAILED. 4 passed; 1 failed; 2 ignored` (`running 7 tests`, `finished in 44.83s`). Between the
  row's stdout header and its panic the trace holds 111 `$/progress` frames, 91 of them
  `rustAnalyzer/cachePriming` reports, against `*lsp*`'s 64-entry ring. It was the job's only
  failure: its test step paired 215 `running` lines with 215 results, 5,254 passed, 1 failed, 67
  ignored, and its doc-test and `pmacs-protocol` steps were skipped after it. Not the branch's: an
  in-process editor against rust-analyzer, which the branch's daemon write path does not reach (read,
  not demonstrated). Commented on #282 (issuecomment-6427081579); not rerun, the rerun or the bypass
  the owner's.
- The other five test legs pair every `running` line with `test result: ok`: `Test (crdt)`
  121817676645 (20:06:57Z to 20:42:56Z, 35m59s) 216 pairs, 5,256 passed, 67 ignored; `Test
  (ubuntu-latest / lua54)` 121817677062, 217, 5,309 / 67; `Test (ubuntu-latest / luajit, no crdt)`
  121817677378, 217, 4,731 / 53; `Test (macos-latest / luajit)` 121817676917 and `Test
  (macos-latest / lua54)` 121817677108, 218 each, 5,265 / 65; each `fd38a8a`'s figures plus the
  round's rows that leg compiles. `WouldBlock` appears in no test leg's log. `Perf budgets (debug)`
  121817676918: `every budget ran and held`.

`Grammar fuzz` 40392713420 at `50005da`, `success`, both arms deciding `run=false`: `ubsan`
121817598081, `asan-strict` 121817597497.

#### E10d review 1's gate and fix round 1's: listed, not tallied

Both are the default plan, each with `fmt`, `clippy`, `doc`, `build` and `diff-check` ok, no daemon
surviving (identified by environ), every `running` line paired with a result line and `WouldBlock`
in none.

- `20261026T180609Z-3811177`, review 1's, at `fd38a8a` in the review's detached worktree
  `pmacs-e10d-r1`: five of six, `05-sweep` 5,675 / 1 / 67 at 224 pairs, recounted from its own log;
  #311's fragments on `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (a store
  holding `Could not find module ‘Prelude’`, the trace reaching `LSP:ready·processi`, `E7E
  with-compiler Some("ghc-9.6.7")`; 127.94 s at result line 3681), commented
  (issuecomment-6424395558).
- `20261026T200159Z-144875`, fix round 1's, at the head `50005da` in `pmacs-e8d`, at a load near 19
  from other work: five of six, `05-sweep` 5,687 / 1 / 67 at 225 pairs; #311's fragments on the HLS
  row (130.81 s at result line 3664), commented (issuecomment-6427012931).
- Hand runs in the gate's environment: the refusal rows failing at `fd38a8a`'s daemon and passing 6
  of 6 with the fix, P2 failing at `73bc1e6`'s with `left: 3`, the mid-loop skip's row failing 3 of
  3 with the skip removed, and the guard's plants, each failing what it was planted to fail.

So, at E10d's fix round 1: `main` is green at `658051f`, and PR #408 is red at its head `50005da` on
#282's eighth occurrence, which takes #282 to eight, on `Test (ubuntu-latest / luajit)`. #311 met
only local samples here and does not move. Every other count above stands.

### E10d's close: PR #408 green on attempt 2 at `50005da`, merged as `4c57353`, `main` green there

Recorded 2026-10-27 at E10d's close. The owner merged PR #408 at 12:39:50Z as `4c57353`, a squash of
`50005da` on `470f5dc` whose diff is the head's against its base, byte for byte; its tree is the
head's but for this file. Every run was read from the jobs endpoint by conclusion, and every test
leg of each code-bearing run through its own log, fetched with escape sequences allowed and
stripped, each `running` line paired with its own result line. Every run id and job id below was
searched for in this file before it was written, and only attempt 1's run id was here. Nothing was
rerun but attempt 2, which the owner ran.

#### PR #408's `CI` 40392704632 at `50005da`, attempt 2: green

Attempt 1 is recorded above, red on #282's eighth occurrence. The owner reran its failed jobs
(`gh run rerun 40392704632 --failed`) on 2026-10-26: attempt 2, `pull_request`, the same merge
`a18f1a0` into `658051f`, ran from 21:15:30Z to 21:49:02Z (the API's `run_started_at` and
`updated_at`): `success`. Only `Test (ubuntu-latest / luajit)` ran again; every other job is
carried from attempt 1 under a new id with attempt 1's times, and `Docs consistency` was skipped
again.

Tally (pr408-ci-50005da-attempt2): 19 = 18 + 1.

Tally (pr408-ci-50005da-attempt2-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Changed paths | 121852854580 | success |
| Test (ubuntu-latest / luajit) | 121852854729 | success |
| Test (ubuntu-latest / lua54) | 121852855458 | success |
| Lint (lua54) | 121852855478 | success |
| M1 Acceptance Gates | 121852856257 | success |
| Docs consistency | 121852856553 | skipped |
| M4 Perf Gates | 121852856561 | success |
| Test (macos-latest / luajit) | 121852856612 | success |
| Perf budgets (debug) | 121852856661 | success |
| M10 Perf Gates (crdt) | 121852856800 | success |
| Test (ubuntu-latest / luajit, no crdt) | 121852857224 | success |
| Test (macos-latest / lua54) | 121852857636 | success |
| M6 Perf Gates | 121852857718 | success |
| Lint (luajit) | 121852899881 | success |
| Commit attribution (D9) | 121852905499 | success |
| Format | 121852907121 | success |
| GPU Render (headless) | 121852911603 | success |
| Test (crdt) | 121852919845 | success |
| M5 Perf Gates | 121852921857 | success |

Tally (pr408-ci-50005da-attempt2-success): 18 rows of the table above with `result` = `success`.

- `Test (ubuntu-latest / luajit)` 121852854729, 21:15:36Z to 21:49:00Z: its test step paired 217
  `running` lines with 217 `test result: ok`, 5,309 passed, 67 ignored, the rename row `ok`; its
  doc-test and `pmacs-protocol` steps ran, the same binaries and counts as attempt 1's `Test
  (ubuntu-latest / lua54)`; `WouldBlock` appears nowhere in its log. Under the rerun rule this
  establishes non-reproduction and nothing more: #282 stays at eight.

#### `main` at `4c57353`, E10d's squash merge of PR #408: green

`CI` 40505543789 at `4c57353`, attempt 1 of 1, `push`, created 2026-10-27T12:39:55Z and completed
13:23:36Z (the API's `updated_at`): `success`.

Tally (main-ci-4c57353): 19 = 18 + 1.

Tally (main-ci-4c57353-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 122193800951 | success |
| Changed paths | 122193801305 | success |
| Commit attribution (D9) | 122193801384 | success |
| Lint (lua54) | 122193801603 | success |
| Lint (luajit) | 122193801645 | success |
| Perf budgets (debug) | 122193934899 | success |
| M4 Perf Gates | 122193934930 | success |
| M5 Perf Gates | 122193934944 | success |
| M1 Acceptance Gates | 122193934946 | success |
| Test (macos-latest / luajit) | 122193934973 | success |
| Test (ubuntu-latest / luajit, no crdt) | 122193935000 | success |
| Test (crdt) | 122193935045 | success |
| M10 Perf Gates (crdt) | 122193935071 | success |
| Test (macos-latest / lua54) | 122193935110 | success |
| GPU Render (headless) | 122193935118 | success |
| Test (ubuntu-latest / lua54) | 122193935122 | success |
| Test (ubuntu-latest / luajit) | 122193935144 | success |
| M6 Perf Gates | 122193935366 | success |
| Docs consistency | 122193936803 | skipped |

Tally (main-ci-4c57353-success): 18 rows of the table above with `result` = `success`.

- The six test legs, each through its own log: `Test (crdt)` 122193935045 ran 12:40:16Z to
  13:22:39Z, 42m23s, pairing 216 `running` lines with 216 `test result: ok`, 5,256 passed, 67
  ignored; `Test (ubuntu-latest / luajit, no crdt)` 122193935000, 217 pairs, 4,731 passed, 53
  ignored; `Test (ubuntu-latest / lua54)` 122193935122 and `Test (ubuntu-latest / luajit)`
  122193935144, 217 pairs each, 5,309 passed, 67 ignored; `Test (macos-latest / luajit)`
  122193934973 and `Test (macos-latest / lua54)` 122193935110, 218 pairs each, 5,265 passed, 65
  ignored: PR #408's head figures on every leg. `test result: FAILED` and `WouldBlock` appear in no
  test leg's log.
- `Perf budgets (debug)` 122193934899: `every budget ran and held`,
  `opening_two_thousand_files_stays_within_budget ... ok`.

`Grammar fuzz` 40505543087 at `4c57353`, `success`, both arms deciding `run=false` (`no changed path
can change the grammar set`): `ubsan` 122193798256, `asan-strict` 122193798499.

So, at E10d's close: `main` is green at `4c57353`, and PR #408 merged green at `50005da` on its
attempt 2. Nothing moves: #282 stays at eight. Every other count above stands.

### E10h: PR #418 at `8ef24c6`, `main` green at `57cd5ac`, the gates' samples listed, #417 filed

Recorded 2026-10-28 at C10h. PR #418 (`e10h/worktree-builds`, two signed commits on `57cd5ac`)
changes the root build script's watch list and the build hash `InstanceIdentity` carries; it touches
no test row but its own suite. Every run was read from the jobs endpoint by conclusion, and every
test leg of each code-bearing run through its own log, fetched with escape sequences allowed and
stripped, each `running` line paired with its own result line. Every run id and job id below was
searched for in this file before it was written, and none was here. Nothing was rerun.

#### `main` at `57cd5ac`, E10d's close registry commit: green

`CI` 40512223437 at `57cd5ac`, attempt 1 of 1, `push`, created 2026-10-27T13:27:42Z and completed
13:31:55Z (the API's `updated_at`): `success`, the registry-push shape: the changed-paths job found
only this file, so every test leg and perf gate was skipped.

Tally (main-ci-57cd5ac): 15 = 6 + 9.

Tally (main-ci-57cd5ac-jobs): 15 rows in the table below.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 122216791398 | success |
| Lint (luajit) | 122216791760 | success |
| Changed paths | 122216791966 | success |
| Format | 122216792214 | success |
| Lint (lua54) | 122216792243 | success |
| Docs consistency | 122216893017 | success |
| Test (crdt) | 122217065501 | skipped |
| Test (matrix) | 122217077146 | skipped |
| GPU Render (headless) | 122217079007 | skipped |
| M10 Perf Gates (crdt) | 122217127281 | skipped |
| M6 Perf Gates | 122217173501 | skipped |
| Perf budgets (debug) | 122217300912 | skipped |
| M4 Perf Gates | 122217309207 | skipped |
| M5 Perf Gates | 122217362862 | skipped |
| M1 Acceptance Gates | 122217567472 | skipped |

Tally (main-ci-57cd5ac-success): 6 rows of the table above with `result` = `success`.

The skipped matrix row is the one job the API names with its unexpanded template, `Test (${{
matrix.os }} / …)`, written here as `Test (matrix)`. `Grammar fuzz` 40512223172 at `57cd5ac`,
`success`: `Grammar fuzz` 122216790134 and `Grammar fuzz (asan-strict)` 122216789606.

#### PR #418's `CI` 40610184158 at `8ef24c6`: red on #382's second occurrence

`pull_request`, the merge `121e640` into `57cd5ac`, attempt 1 of 1, created 2026-10-28T00:39:52Z and
completed 01:15:49Z (the API's `updated_at`): `failure`.

Tally (pr418-ci-8ef24c6): 19 = 17 + 1 + 1.

Tally (pr418-ci-8ef24c6-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 122559569689 | success |
| Commit attribution (D9) | 122559569909 | success |
| Changed paths | 122559569960 | success |
| Lint (lua54) | 122559570029 | success |
| Lint (luajit) | 122559570039 | success |
| Test (crdt) | 122559632513 | success |
| GPU Render (headless) | 122559632597 | success |
| Test (macos-latest / lua54) | 122559632630 | failure |
| Test (ubuntu-latest / luajit, no crdt) | 122559632642 | success |
| Test (ubuntu-latest / luajit) | 122559632644 | success |
| M6 Perf Gates | 122559632649 | success |
| M5 Perf Gates | 122559632672 | success |
| M10 Perf Gates (crdt) | 122559632699 | success |
| M4 Perf Gates | 122559632719 | success |
| M1 Acceptance Gates | 122559632720 | success |
| Test (ubuntu-latest / lua54) | 122559632770 | success |
| Test (macos-latest / luajit) | 122559632771 | success |
| Perf budgets (debug) | 122559632797 | success |
| Docs consistency | 122559634141 | skipped |

Tally (pr418-ci-8ef24c6-success): 17 rows of the table above with `result` = `success`.

- `Test (macos-latest / lua54)` 122559632630, a required check, 00:40:07Z to 01:01:37Z: its only
  failure, `e7c_indicator_acceptance::e7c_fix_3_a_slow_request_appears_as_its_method_and_vanishes_when_answered`
  at `tests/e7c_indicator_acceptance.rs:726:5`, `nothing on the indicator under the threshold:
  [(29, Some("⋯1 parse rust                   "))]`, the row's trace reading the parse at 29 ms,
  gone at 70, the rename on the indicator at 302 ms and gone at 1630, and `test result: FAILED. 4
  passed; 1 failed; 0 ignored` (7.08 s, log lines 4150 to 4178). **#382's second occurrence, read
  as the template**: the issue names the reading at `0` ms and this one is at 29 ms. The assertion is
  one template over the readings under the threshold, and the parse is again in the frames just
  after the request, not before it (#363's), so it is counted with the fragment read as the
  template, as #316's third was at `24593f1`; the issue is commented with that reading
  (issuecomment-6458193026), and splitting it out is the owner's call. 217 paired, 216 `ok`; the
  leg's doc-test and protocol steps were skipped after the red. The branch reaches no indicator,
  parse or LSP path, and a debug build's identity and build-script output are the base's: an
  argument from what the change reaches, not a measurement.
- The five other test legs pair each `running` line with a `test result: ok`: `Test (crdt)`
  122559632513, 00:40:06Z to 01:14:47Z, 34m41s, 217 pairs, 5,263 passed, 67 ignored; `Test
  (ubuntu-latest / luajit)` 122559632644 and `Test (ubuntu-latest / lua54)` 122559632770, 218 pairs
  each, 5,316 passed, 67 ignored; `Test (ubuntu-latest / luajit, no crdt)` 122559632642, 218, 4,738
  passed, 53 ignored; `Test (macos-latest / luajit)` 122559632771, 219, 5,272 passed, 65 ignored:
  `main`'s figures at `4c57353` plus the branch's suite's seven on every leg. `WouldBlock` appears in
  no test leg's log. The branch's `e10h_worktree_builds_acceptance` passed 7 of 7 on all six legs.
- `Perf budgets (debug)` 122559632797: `every budget ran and held`.

`Grammar fuzz` 40610184019 at `8ef24c6`, attempt 1 of 1, `success`, both arms deciding `run=false`
(`no changed path can change the grammar set`): `Grammar fuzz` 122559569310, `Grammar fuzz
(asan-strict)` 122559568881.

#### E10h's gates: listed, not tallied, #417 filed

All four are the default plan (`CARGO_BUILD_JOBS=6 nice -n 19 scripts/gate`) in `pmacs-e8d`, each
with `fmt`, `clippy`, `doc`, `build` and `diff-check` ok, no daemon surviving (identified by
environ), every `running` line paired with a result line and `WouldBlock` in none; their logs are
copied to `~/build/e10h/gate-logs/`.

- `20261027T210549Z-1973572` at `57cd5ac`: five of six, `05-sweep` 5,687 / 1 / 67 at 225 pairs;
  #311's fragments on `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (a store
  holding `Could not find module ‘Prelude’`, the trace reaching `LSP:ready·processi`, `E7E
  with-compiler Some("ghc-9.6.7")`; 125.97 s at result line 3697).
- `20261027T213212Z-2101016` at `57cd5ac`: the same, 125.65 s at result line 3664.
- `20261027T235518Z-2631791` at `8ef24c6`: five of six, 5,694 / 1 / 67 at 226 pairs; #311's
  fragments, 126.08 s at result line 3677.
- `20261028T001547Z-2763247` at `8ef24c6`, with the load average at 11.94 / 13.10 / 12.02 read just
  after it, from other work: five of six, 5,691 / 4 / 67 at 226 pairs: #311's fragments (134.45 s at
  result line 3688); #288's, `the check's diagnostic landed: ["pmacs-fake-lsp"]` and `test result:
  FAILED. 4 passed; 1 failed; 0 ignored` (result line 3618); #316's, `kill 1: nothing held the
  switch's follow-up parse, which started a worker at once and installed`, `tree=true`, `unit=pid_`,
  `death=killed`, `parse:killed` and `test result: FAILED. 10 passed; 1 failed; 0 ignored` (result
  line 4028); and `e8_review1_real_server_probes::fr1_c_c_h_opens_the_last_popup_after_a_motion_closed_it`,
  `no accepted popup for C-c h in 120 s; last None; status "LSP: no hover info"` and `test result:
  FAILED. 6 passed; 1 failed; 0 ignored` (result line 4125), which no row names (#317's result line
  is `3 passed; 6 failed`, every rust-analyzer row on macOS), filed as **#417**. The same tree's
  previous gate passed all four rows.

The samples are commented on #311 (issuecomment-6457482343, all four gates), #288
(issuecomment-6457485764) and #316 (issuecomment-6457486173).

So, at E10h's close: `main` is green at `57cd5ac`, and PR #418 is red at its head `8ef24c6` on
#382's second occurrence, read as the template, which takes #382 to two, on `Test (macos-latest /
lua54)`. #311, #288 and #316 met only local samples here and do not move; #417 is filed from a local
sample and has no CI occurrence. Every other count above stands.

### E10h fix round 1: PR #418 at `bca22b9`, `main` green at `44345b6`, #420 filed, the gates' samples listed

Recorded 2026-10-28 at E10h's fix round 1. PR #418 (`e10h/worktree-builds`) moved from `8ef24c6` to
`ccfb7e4` by four signed commits, pushed at 10:37Z, and to `bca22b9` by a fifth, pushed at 13:59Z
after `ccfb7e4`'s run had completed; the round changes the root build script's watch list in a
release build, three doc comments and its own suite, and touches no other test row. Every run was
read from the jobs endpoint by conclusion, and every test leg of each code-bearing run through its
own log, fetched with escape sequences allowed and stripped, each `running` line paired with its own
result line. Every run id and job id below was searched for in this file before it was written, and
none was here. Nothing was rerun.

#### `main` at `44345b6`, E10h's registry commit: green

`CI` 40615387746 at `44345b6`, attempt 1 of 1, `push`, created 2026-10-28T01:29:50Z and completed
01:31:40Z (the API's `updated_at`): `success`, the registry-push shape: the changed-paths job found
only this file, so every test leg and perf gate was skipped.

Tally (main-ci-44345b6): 15 = 6 + 9.

Tally (main-ci-44345b6-jobs): 15 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 122576562093 | success |
| Changed paths | 122576562160 | success |
| Commit attribution (D9) | 122576562180 | success |
| Lint (lua54) | 122576562189 | success |
| Lint (luajit) | 122576562263 | success |
| Docs consistency | 122576624142 | success |
| M6 Perf Gates | 122576625481 | skipped |
| GPU Render (headless) | 122576625519 | skipped |
| Perf budgets (debug) | 122576626254 | skipped |
| Test (crdt) | 122576626285 | skipped |
| M5 Perf Gates | 122576626358 | skipped |
| M4 Perf Gates | 122576626411 | skipped |
| Test (matrix) | 122576626778 | skipped |
| M1 Acceptance Gates | 122576627129 | skipped |
| M10 Perf Gates (crdt) | 122576627376 | skipped |

Tally (main-ci-44345b6-success): 6 rows of the table above with `result` = `success`.

`Docs consistency` 122576624142: `test result: ok. 18 passed`. The skipped matrix row is the one job
the API names with its unexpanded template, written here as `Test (matrix)`. `Grammar fuzz`
40615387751 at `44345b6`, `success`, both arms deciding `run=false` (`no changed path can change the
grammar set`): `Grammar fuzz` 122576562618 and `Grammar fuzz (asan-strict)` 122576561853.

#### PR #418's `CI` 40673606425 at `ccfb7e4`: red on #420's first occurrence

`pull_request`, the merge `baad342` into `44345b6`, attempt 1 of 1, created 2026-10-28T10:37:34Z and
completed 11:13:00Z (the API's `updated_at`): `failure`.

Tally (pr418-ci-ccfb7e4): 19 = 17 + 1 + 1.

Tally (pr418-ci-ccfb7e4-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Commit attribution (D9) | 122766288285 | success |
| Lint (luajit) | 122766288411 | success |
| Changed paths | 122766288591 | success |
| Lint (lua54) | 122766288625 | success |
| Format | 122766288722 | success |
| M4 Perf Gates | 122766368078 | success |
| Perf budgets (debug) | 122766368120 | success |
| M10 Perf Gates (crdt) | 122766368139 | success |
| Test (ubuntu-latest / luajit) | 122766368163 | success |
| Test (ubuntu-latest / luajit, no crdt) | 122766368169 | success |
| Test (macos-latest / lua54) | 122766368192 | success |
| GPU Render (headless) | 122766368195 | success |
| M5 Perf Gates | 122766368213 | success |
| M6 Perf Gates | 122766368219 | success |
| Test (crdt) | 122766368264 | failure |
| Test (macos-latest / luajit) | 122766368271 | success |
| Test (ubuntu-latest / lua54) | 122766368285 | success |
| M1 Acceptance Gates | 122766368435 | success |
| Docs consistency | 122766371368 | skipped |

Tally (pr418-ci-ccfb7e4-success): 17 rows of the table above with `result` = `success`.

- `Test (crdt)` 122766368264, 10:37:49Z to 11:12:59Z: its only failure,
  `e8b_switch_buffer_acceptance::e8b_fr1_the_gpu_reaches_a_dired_buffer_by_its_own_name_and_keeps_d18_s_four`
  at `tests/e8b_switch_buffer_acceptance.rs:628:5`, ``assertion `left == right` failed``, `left:
  "\"WHERE dired.lua\""`, `right: "\"WHERE editor.rs\""`, and `test result: FAILED. 9 passed; 1
  failed; 0 ignored` (74.34 s, log lines 4860 to 4890): the row's first band assertion, the window
  after `C-x b editor.rs RET` at its start. No row or open issue named the selector with these
  fragments (#361, its only issue, closed at E10c, carried the row's last switch, `no buffer: zzz`):
  **#420, filed, its first occurrence**, its candidate mechanism #326's, from the row and not a run
  (a bootstrap snapshot's viewport applied after the switch). The same row passed on the run's four
  other legs that compile it and on `Test (crdt)` at `8ef24c6`. 216 paired, 215 `ok`, `5,267
  passed, 1 failed, 67 ignored`; the leg's doc-test step was skipped after the red (at `8ef24c6` its
  217 pairs included that step's). The branch reaches no switch-buffer, GPU or bootstrap path, and a
  debug build's script output and identity are the base's: an argument from what the change
  reaches, not a measurement.
- The five other test legs pair each `running` line with a `test result: ok`: `Test
  (ubuntu-latest / luajit)` 122766368163 and `Test (ubuntu-latest / lua54)` 122766368285, 218 pairs
  each, 5,322 passed, 67 ignored; `Test (ubuntu-latest / luajit, no crdt)` 122766368169, 218, 4,744
  passed, 53 ignored; `Test (macos-latest / luajit)` 122766368271 and `Test (macos-latest / lua54)`
  122766368192, 219 each, 5,278 passed, 65 ignored: `8ef24c6`'s figures plus the round's six rows on
  every leg. #382's row passed on `Test (macos-latest / lua54)`. `WouldBlock` appears in no test
  leg's log. The branch's `e10h_worktree_builds_acceptance` passed 13 of 13 on all six legs.
- `Perf budgets (debug)` 122766368120: `every budget ran and held`.

`Grammar fuzz` 40673606402 at `ccfb7e4`, attempt 1 of 1, `success`, both arms deciding `run=false`
(`no changed path can change the grammar set`): `Grammar fuzz` 122766288120, `Grammar fuzz
(asan-strict)` 122766288658.

#### PR #418's `CI` 40701497907 at `bca22b9`: red on #373's third occurrence

`pull_request`, the merge `d9ecde4` into `44345b6`, attempt 1 of 1, created 2026-10-28T13:59:06Z and
completed 14:35:36Z (the API's `updated_at`): `failure`.

Tally (pr418-ci-bca22b9): 19 = 17 + 1 + 1.

Tally (pr418-ci-bca22b9-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 122862232190 | success |
| Lint (luajit) | 122862232249 | success |
| Commit attribution (D9) | 122862232412 | success |
| Changed paths | 122862232433 | success |
| Lint (lua54) | 122862232653 | success |
| Perf budgets (debug) | 122862328595 | success |
| M10 Perf Gates (crdt) | 122862328610 | success |
| Test (crdt) | 122862328664 | success |
| Test (macos-latest / lua54) | 122862328681 | success |
| Test (ubuntu-latest / lua54) | 122862328701 | failure |
| M1 Acceptance Gates | 122862328712 | success |
| GPU Render (headless) | 122862328761 | success |
| M5 Perf Gates | 122862328776 | success |
| Test (ubuntu-latest / luajit) | 122862328780 | success |
| Test (macos-latest / luajit) | 122862328807 | success |
| M4 Perf Gates | 122862328977 | success |
| Test (ubuntu-latest / luajit, no crdt) | 122862329040 | success |
| M6 Perf Gates | 122862329063 | success |
| Docs consistency | 122862330669 | skipped |

Tally (pr418-ci-bca22b9-success): 17 rows of the table above with `result` = `success`.

- `Test (ubuntu-latest / lua54)` 122862328701, a required check, 13:59:20Z to 14:33:29Z: its only
  failure, `process::tests::release_joins_every_polling_reader_whatever_the_deadline` at
  `src/process.rs:5525:9`, `only the blocking reader on the master is left; the parser is joined`,
  `left: 0`, `right: 1`, the lib's `test result: FAILED. 2310 passed; 1 failed; 13 ignored` (68.90
  s, the `FAILED` row at log line 2569, the result line at 3159): **#373's third occurrence**, every
  required fragment, on a leg its first two did not run (both were `Test (ubuntu-latest / luajit)`),
  so counted on the template reading as #382's second was at C10h, splitting it out the owner's
  call; commented there (issuecomment-6471920367). Its test step paired 216 `running` lines with 216
  results, 5,268 passed, 1 failed, 67 ignored; its doc-test and `pmacs-protocol` steps were skipped
  after it. The branch has no diff in `src/process.rs`, and the row passed on the run's five other
  test legs, on all six at `ccfb7e4` and in both of the round's local gates: an argument from
  untouched code, not a measurement.
- The five other test legs pair each `running` line with a `test result: ok`: `Test (crdt)`
  122862328664, 13:59:21Z to 14:34:06Z, 34m45s, 217 pairs, 5,270 passed, 67 ignored; `Test
  (ubuntu-latest / luajit)` 122862328780, 218, 5,323 passed, 67 ignored; `Test (ubuntu-latest /
  luajit, no crdt)` 122862329040, 218, 4,745 passed, 53 ignored; `Test (macos-latest / luajit)`
  122862328807 and `Test (macos-latest / lua54)` 122862328681, 219 each, 5,279 passed, 65 ignored:
  `ccfb7e4`'s figures plus the round's one row more on every leg. `WouldBlock` appears in no test
  leg's log. The branch's suite passed 14 of 14 on all six legs; #382's row passed on every leg and
  #420's on the five that compile it.
- `Perf budgets (debug)` 122862328595: `every budget ran and held`.

`Grammar fuzz` 40701497719 at `bca22b9`, attempt 1 of 1, `success`, both arms deciding `run=false`
(`no changed path can change the grammar set`): `Grammar fuzz` 122862231803, `Grammar fuzz
(asan-strict)` 122862231451.

#### E10h review 1's and fix round 1's gates: listed, not tallied

All three are the default plan (`CARGO_BUILD_JOBS=6 nice -n 19 scripts/gate`) in `pmacs-e8d`, each
with `fmt`, `clippy`, `doc`, `build` and `diff-check` ok, no daemon surviving (identified by
environ), every `running` line paired with a result line and `WouldBlock` in none.

- `20261028T075138Z-3170652` at `8ef24c6`, review 1's (logs copied to
  `~/build/e10h-r1/gate-20261028T075138Z-3170652/`, recounted from its own log): five of six,
  `05-sweep` 5,694 / 1 / 67 at 226 pairs; #311's fragments on
  `e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (a store holding `Could not
  find module ‘Prelude’`, the trace reaching `LSP:ready·processi`, `E7E with-compiler
  Some("ghc-9.6.7")`; 127.46 s at result line 3675); commented on #311 (issuecomment-6464060535).
- `20261028T102008Z-3478102` at `ccfb7e4` (logs copied to `~/build/e10h-f1/gate-logs/`): five of
  six, 5,700 / 1 / 67 at 226 pairs; #311's fragments, 128.91 s at result line 3683; commented on
  #311 (issuecomment-6466779610).
- `20261028T104504Z-3664877` at `bca22b9`: five of six, 5,700 / 2 / 67 at 226 pairs; #311's
  fragments, 128.94 s at result line 3684, commented on #311 (issuecomment-6467259604); and
  `pmacs-gpu`'s `tests::overwide_status_runs_never_wrap_and_keep_the_suffix_pinned`, `assertion
  failed: min_y >= band_top && max_y <= height` at `pmacs-gpu/src/main.rs:26652:13`, `test result:
  FAILED. 380 passed; 1 failed; 0 ignored` (result line 7282): the sibling recorded on #304 since
  E7h, this time alone, #304's own row passing beside it, the previous gate passing both; commented
  there (issuecomment-6467260634), not filed apart. `pmacs-gpu` has no diff on the branch.

So, at E10h's fix round 1: `main` is green at `44345b6`, and PR #418 is red at its head `bca22b9` on
#373's third occurrence, read as the template, which takes #373 to three; its previous head
`ccfb7e4` was red on #420, filed at one, first seen in CI. #382 stays at two, as the owner ruled,
its row passing on every leg of both runs. #311 and #304 met only local samples here and do not
move. Every other count above stands.

### E10g: PR #422 at `808c445` green, `main` green at `5ae5a27`, the gate's samples listed

Recorded 2026-10-28 at C10g. PR #422 (`e10g/theme-machinery`) opened at `808c445`, eight signed
commits on `5ae5a27`, pushed at 16:34Z; the branch adds the theme machinery (named themes, faces for
the buffer and the surfaces painted from constants, `ui.theme` and its picker, color depth) and its
suite, and changes two existing rows (`theme_faces_acceptance`'s count of faces the bare `ui`
catch-all reaches, and `pmacs-gpu`'s `own_wash_faces_color_local_rects_peers_keep_the_constant`,
whose expected face wash now goes sRGB to linear). Every run was read from the jobs endpoint by
conclusion, and every test leg of the code-bearing run through its own log, fetched with escape
sequences allowed and stripped, each `running` line paired with its own result line. Every run id
and job id below was searched for in this file before it was written, and none was here. Nothing
was rerun.

#### `main` at `5ae5a27`, E10h fix round 1's registry commit: green

`CI` 40708405959 at `5ae5a27`, attempt 1 of 1, `push`, created 2026-10-28T14:41:17Z and completed
14:43:36Z (the API's `updated_at`): `success`, the registry-push shape: the changed-paths job found
only this file, so every test leg and perf gate was skipped.

Tally (main-ci-5ae5a27): 15 = 6 + 9.

Tally (main-ci-5ae5a27-jobs): 15 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 122886677648 | success |
| Lint (lua54) | 122886677932 | success |
| Commit attribution (D9) | 122886677937 | success |
| Changed paths | 122886677995 | success |
| Lint (luajit) | 122886678066 | success |
| Docs consistency | 122886783059 | success |
| M10 Perf Gates (crdt) | 122886785379 | skipped |
| GPU Render (headless) | 122886785553 | skipped |
| Perf budgets (debug) | 122886785607 | skipped |
| M4 Perf Gates | 122886785984 | skipped |
| M1 Acceptance Gates | 122886785990 | skipped |
| Test (matrix) | 122886786419 | skipped |
| M5 Perf Gates | 122886786491 | skipped |
| M6 Perf Gates | 122886786613 | skipped |
| Test (crdt) | 122886786788 | skipped |

Tally (main-ci-5ae5a27-success): 6 rows of the table above with `result` = `success`.

`Docs consistency` 122886783059: `test result: ok. 18 passed`. The skipped matrix row is the one job
the API names with its unexpanded template, written here as `Test (matrix)`. `Grammar fuzz`
40708405943 at `5ae5a27`, `success`, both arms deciding `run=false` (`no changed path can change the
grammar set`): `Grammar fuzz` 122886677035 and `Grammar fuzz (asan-strict)` 122886677615.

#### PR #422's `CI` 40727254457 at `808c445`: green

`pull_request`, the merge `aa7e0e4` into `5ae5a27`, attempt 1 of 1, created 2026-10-28T16:34:14Z and
completed 17:10:40Z (the API's `updated_at`): `success`.

Tally (pr422-ci-808c445): 19 = 18 + 1.

Tally (pr422-ci-808c445-jobs): 19 rows in the table below.

| job | id | result |
|---|---|---|
| Format | 122953737613 | success |
| Lint (luajit) | 122953737923 | success |
| Lint (lua54) | 122953738035 | success |
| Changed paths | 122953738041 | success |
| Commit attribution (D9) | 122953738147 | success |
| M5 Perf Gates | 122953882864 | success |
| M10 Perf Gates (crdt) | 122953882883 | success |
| Test (macos-latest / luajit) | 122953882988 | success |
| M4 Perf Gates | 122953883020 | success |
| Test (ubuntu-latest / luajit) | 122953883021 | success |
| Test (crdt) | 122953883080 | success |
| Perf budgets (debug) | 122953883084 | success |
| GPU Render (headless) | 122953883085 | success |
| M6 Perf Gates | 122953883127 | success |
| M1 Acceptance Gates | 122953883160 | success |
| Test (macos-latest / lua54) | 122953883161 | success |
| Test (ubuntu-latest / lua54) | 122953883346 | success |
| Test (ubuntu-latest / luajit, no crdt) | 122953883447 | success |
| Docs consistency | 122953885984 | skipped |

Tally (pr422-ci-808c445-success): 18 rows of the table above with `result` = `success`.

- The six test legs pair each `running` line with a `test result: ok`: `Test (crdt)` 122953883080,
  16:34:33Z to 17:09:43Z, 35m10s, 217 pairs, 5,294 passed, 67 ignored; `Test (ubuntu-latest /
  luajit)` 122953883021 and `Test (ubuntu-latest / lua54)` 122953883346, 218 pairs each, 5,347
  passed, 67 ignored; `Test (ubuntu-latest / luajit, no crdt)` 122953883447, 218, 4,769 passed, 53
  ignored; `Test (macos-latest / luajit)` 122953882988 and `Test (macos-latest / lua54)`
  122953883161, 219 each, 5,303 passed, 65 ignored. `WouldBlock` appears in no test leg's log. The
  branch's `e10g_theme_acceptance` passed its 21 rows on all six legs (26 tests with `common::ready`'s
  five), and `theme_faces_acceptance` 29 of 29 (27 without `crdt`).
- `Perf budgets (debug)` 122953883084: `every budget ran and held`.

`Grammar fuzz` 40727254259 at `808c445`, attempt 1 of 1, `success`, created 16:34:14Z and completed
17:30:05Z. Both arms decided `run=true` (`src/syntax.rs changed`, by the branch's theme library on
the syntax registry; no grammar, `vendor/` or `builtin/queries/` change) and gave every grammar its
15 s smoke: `Grammar fuzz` 122953741009, 16:34:17Z to 17:24:42Z, and `Grammar fuzz (asan-strict)`
122953741539, 16:34:18Z to 17:30:04Z. Neither found a crash; each replayed 22 grammars' inputs
through the worker, every one answered or contained, 0 crashed. Both found the same bash memory
finding, `exceeded memory cap at 4 GB`, reproduced alone (minima 63,798 and 62,758 bytes), contained
by the worker's memory watch: no row or issue named it, so **#423 is filed**, its first occurrence.
Their slow findings (bash, cmake, cuda, lua and rust, each with its exponent) are commented on #293
(issuecomment-6476041125); a cmake hang under `ubsan`, and two haskell and one markdown_inline hang
under `asan-strict`, were not reproduced.

#### E10g's gate: listed, not tallied

`20261028T161458Z-441402` at `2a553f0`, the default plan (`scripts/gate`) in `pmacs-e10g` (logs
copied to `~/build/e10g/gate-logs/`): five of six, `fmt`, `clippy`, `doc`, `build` and
`diff-check` ok, no daemon surviving (identified by environ), 226 `running` lines each paired with a
result line, 5,728 passed, 2 failed, 67 ignored, `WouldBlock` in none. Its reds: #311's fragments on
`e7e_hls_attaches_in_a_cabal_project_and_reports_a_typed_type_error` (a store holding `Could not
find module ‘Prelude’`, the trace reaching `LSP:ready·processi`, `E7E with-compiler
Some("ghc-9.6.7")`; 129.43 s at result line 3709), commented on #311 (issuecomment-6474814762); and
the branch's own `theme_faces_acceptance::bare_ui_merge_ships_the_catch_all_without_touching_spans`,
its pinned count of faces the catch-all reaches, which the branch's `808c445` fixes. A hand run of
`pmacs-gpu`'s tests met `tests::overwide_status_runs_never_wrap_and_keep_the_suffix_pinned` (`assertion
failed: min_y >= band_top && max_y <= height`), the sibling recorded on #304, passing alone three
times after; not a gate, commented there (issuecomment-6474819461).

So, at C10g: `main` is green at `5ae5a27`, and PR #422 is green at its head `808c445`, its
`Grammar fuzz` green with #423 filed at one. #311 and #304 met only local samples here and do not
move. Every other count above stands.

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
| occurrences | at least thirty-six, 2026-08-07 to 2026-10-12, all local, all under sweep load; the panic line moves with `attach.rs` and is not part of the signature. Six later local samples, 2026-10-14 to 2026-10-16, are listed after the held list and not counted, under the owner's ruling of 2026-10-20. The first twelve are enumerated in this file's history before 2026-09-05; the twenty-four since are the list below this table, with the tallies (added at fix round 1, review 1's Low 3). The count is a floor: nobody has counted runs, so an occurrence is only ever recorded when someone reads the log |
| candidate mechanism | the test drives a scripted transient-then-success sequence over a real socket pair; unknown whether the broken pipe is the fixture's writer closing early or a retry-path defect. Unresolved |
| retirement | hardening that removes the named mechanism plus a discriminating witness, or a diagnosis showing the fixture, not the code, closes the pipe |

Tally (R7): 36 = 12 + 24.

Tally (R7-held): 24 items in the list below.

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
- thirty-fourth: gate log `20261010T130620Z-3845623`, step `05-sweep` of the six-stage gate, E8c review 1's third gate on `e8c/quit-must-exit` at `a35b76b` (#357), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`); `pmacs-gpu` has no diff on the branch, and the row passed in the review's two other gates and the fix round's (recorded with E8c's fix round 1, above)
- thirty-fifth: gate log `20261010T192148Z-332172`, step `05-sweep` of the six-stage gate, #360's pull request's gate on `fix/places-no-fsync` at `c33b55c` (#362), `attach.rs:1971`, `test result: FAILED. 373 passed; 2 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`), the other failure in the binary being #304's sibling; `pmacs-gpu` has no diff on the branch (recorded with that pull request, above)
- thirty-sixth: not a gate but the wall-clock classification pass's first measurement sweep at `824ca82`, each test binary run directly in the gate's environment with libtest's `--report-time`, the load 53.78 at its start (other sessions' fuzzing and builds), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`); the pass changes no code, and the row passed in its three later sweeps and in its gate (recorded with that pass, above)

The local samples since, listed and not counted under the owner's ruling of 2026-10-20:

- gate log `20261014T171638Z-1006348`, step `05-sweep` of the six-stage gate, the owner's pass gate over E8d on `e8d/drift-fixes` at `22de659` (#372), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (`transient sequence must attach`, `Handshake(Io(Os { code: 32, kind: BrokenPipe`); `pmacs-gpu` has no diff on the branch (recorded at E9's close, above)
- gate log `20261015T112921Z-1698319`, step `05-sweep`, E8e.1's gate on `e8e/common-editor` at `a7f915b` (#376), `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments; the branch changes tests alone (recorded at E9's close, above)
- gate log `20261015T122545Z-1811476`, step `05-sweep`, E8e.2's gate at `7ae872a`, `attach.rs:1971`, `test result: FAILED. 374 passed; 1 failed`, all three fragments (recorded at E9's close, above)
- gate log `20261016T100845Z-734980`, step `06-sweep`, E9.2's gate on `e9/session-identity` at `772b039` (#378), `attach.rs:2009` (the line moved with E9's own change to `attach.rs`), `test result: FAILED. 374 passed; 1 failed`, all three fragments (recorded at E9's close, above)
- the same gate's `07-sweep-luajit`, `attach.rs:2009`, `test result: FAILED. 374 passed; 1 failed`, all three fragments, the sweep's third failure beside #311's and #264's; E9's handoff names R7 in that gate's `sweep` alone (recorded at E9's close, above)
- gate log `20261016T154436Z-987086`, step `06-sweep`, E9's gate at `fd32170`, `attach.rs:2009`, `test result: FAILED. 374 passed; 1 failed`, all three fragments; the row passed in the same gate's `07-sweep-luajit` (recorded at E9's close, above)

E9 diagnosed a mechanism and changed the fixture, not this row: `d86d135`
(2026-10-16) makes the two `attach.rs` fixtures' server halves read the
client's bootstrap and its `Resume` before they drop their end, where
they had returned after the `AttachRequest` (one after the bootstrap)
with the client's writes still to come, so a loaded sweep could lose
them as `Handshake(Io(BrokenPipe))`. Its handoff records the row run
twenty times by hand after it. None of E9's gates on a tree carrying
`d86d135` met R7: E9.3's, the tip's and the eleven after them, thirteen
gates of two sweeps each, recorded at E9's close above. That is
non-reproduction and nothing more; whether the diagnosis retires the
row is the owner's ruling.

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
