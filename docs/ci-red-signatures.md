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

Tally (259-red-polls): 53–58 over 53, 54, 58, 58.

Tally (259-red-elapsed): 5.01–5.04 s over 5.04, 5.03, 5.03, 5.01.

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

### The `read Hello` family is at FOURTEEN

Fourteen occurrences across **five suites** and **six selectors**,
counted by fetching the failing job of every run on PR #257, PR #262
and PR #265 and `main`'s post-merge runs, grepping `WouldBlock`, and
extracting the panicking thread and site of each hit. Per-job counts:
1, 1, 4, 1, 2, 2, 1, 1, 1. (Twelve when this section was first written,
2026-09-09; thirteen and fourteen added 2026-09-10 from E3's fix-round
head and from `main` after E3's merge.)

Tally (read-hello-per-job): 14 = 1 + 1 + 4 + 1 + 2 + 2 + 1 + 1 + 1.

Tally (read-hello-family): 14 rows in the table below.

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

The six selectors are `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join`
(four), `daemon_reships_the_summary_after_a_real_buffer_round_trip`
(**five**, the family's most frequent on its own since the fourteenth),
`v15_peer_never_receives_theme_facts_and_v16_does` (two), and
`v16_peer_never_receives_font_facts_and_v17_does`,
`m10_10_non_replica_frontend_does_not_receive_cursor_byte` and
`daemon_routes_semantic_family_to_semantic_session_only` (one each).

Tally (read-hello-suites): 5 distinct values of `suite` in the table above.

Tally (read-hello-selectors): 6 distinct values of `selector` in the table above.

Tally (read-hello-a16_26): 4 rows of the table above with `selector` = `a16_26_real_daemon_v17_gate_v18_first_frame_and_late_join`.

Tally (read-hello-daemon_reships): 5 rows of the table above with `selector` = `daemon_reships_the_summary_after_a_real_buffer_round_trip`.

Tally (read-hello-v15_peer): 2 rows of the table above with `selector` = `v15_peer_never_receives_theme_facts_and_v16_does`.

Tally (read-hello-v16_peer): 1 row of the table above with `selector` = `v16_peer_never_receives_font_facts_and_v17_does`.

Tally (read-hello-m10_10): 1 row of the table above with `selector` = `m10_10_non_replica_frontend_does_not_receive_cursor_byte`.

Tally (read-hello-daemon_routes): 1 row of the table above with `selector` = `daemon_routes_semantic_family_to_semantic_session_only`.

Zero `WouldBlock` in all three failing macOS **lua54** jobs
(102040771394, 102047236847, 102154957233) and **zero at the merge
base**. The count is a floor: nobody has counted runs, so an
occurrence is recorded only when someone reads a log.

### #258 is at FOUR occurrences

Its own selector, job and all three required fragments, four times.

Tally (258): 4 items in the list below.

- run 34220035122 at `8f6784f`
- run 34272480226 at `d7fd465` (the occurrence D30 dispositioned)
- run 34369540895 at `e5417f6`
- run 34373256548 at `7b6c519`

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
| retirement | diagnosis. The deadline now reports its subject, its elapsed time and its poll count, so the next occurrence says whether it missed by a millisecond or by two seconds --- that is a step toward the diagnosis and is not itself a closer. Never a green rerun. **THE DISCRIMINATOR, and it is one number that already exists.** R5 shares its waiting shape, its `--lib` binary and the very ancestry of its reporting with #263 (`68a4a14` carried R5's reporting inward to `pump_until`), and the one measurement that separates *the pump was starved* from *the reply never came* is the poll count --- which **R5 has never been observed with**, both occurrences above predating that commit. So the next occurrence decides it, and nothing else has to be built: **~2000 polls in 2 s** means the pump ran at full rate and nothing settled, which excludes starvation, is the same window #263 saw, and merges the two rows under a second selector; **a small count** means the pump itself was starved, which is a different mechanism, and #263 is not R5. Recorded at C2's close so the next reader of this row does not re-derive it |

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
| occurrences | at least sixteen, 2026-08-07 to 2026-09-07, all local, all under sweep load; the panic line moves with `attach.rs` and is not part of the signature. The thirteenth and fourteenth: gate logs `20260905T202734Z-1751532` (step `07-sweep`, load average 14.2) and `20260905T205642Z-2051072` (step `05-sweep` of the six-stage gate). The fifteenth and sixteenth: gate logs `20260907T170429Z-45241` and `20260907T185321Z-604527`, both in step `06-sweep-luajit`, the LuaJIT-only sweep `--protocol` adds, each ending `test result: FAILED. 325 passed; 1 failed` on `-p pmacs-gpu --bin pmacs-gpu`. All four at `attach.rs:1889` with all three fragments. The first twelve are enumerated in this file's history before 2026-09-05; the four above are the enumeration held here. The count is a floor: nobody has counted runs, so an occurrence is only ever recorded when someone reads the log |
| candidate mechanism | the test drives a scripted transient-then-success sequence over a real socket pair; unknown whether the broken pipe is the fixture's writer closing early or a retry-path defect. Unresolved |
| retirement | hardening that removes the named mechanism plus a discriminating witness, or a diagnosis showing the fixture, not the code, closes the pipe |

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
| occurrences | three, all before the void closure: PR #229 (lua54) and PR #231 attempts 1 and 2 (luajit), 2026-08-15 and earlier. Full evidence in this file's history before 2026-09-05, including #231's five-observation base control and why neither branch diff excludes itself. The Linux observation of 2026-09-07 is #255 and is NOT an occurrence of this row --- the job does not match; see the section below |
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

### U17 — a supersede race lost the other way, on `main`

| field | value |
|---|---|
| selector | `--test m8_1_acceptance read_dir_supersede_cancels_in_flight_predecessor` |
| job | GitHub Actions: first the serialized crdt sweep (`--test-threads=1`), since then `Test (macos-latest / luajit)` twice and `Test (ubuntu-latest / luajit)` once, all three under cargo's default parallelism. The job is not a discriminator for this row and a match needs the selector and the fragment on any CI test leg |
| required fragments | `first read_dir must be superseded; got ok` |
| occurrences | four: `main` at `aae5b35`, run 33375945966 (the serialized crdt sweep); `main` at `d97e137`, run 34205653191, job `Test (macos-latest / luajit)`; PR #257 at `8f6784f`, run 34220035122, job `Test (ubuntu-latest / luajit)`; and `main` at `dbe40a1`, run 34349759554 (E1's post-merge run), job `Test (macos-latest / luajit)` (102459915513), `tests/m8_1_acceptance.rs:278:5`, `assertion left == right failed: first read_dir must be superseded; got ok` with `left: "ok"` and `right: "cancelled"`, `test result: FAILED. 9 passed; 1 failed`, the job's only failure against 119 `test result: ok`. The second, third and fourth run at cargo's DEFAULT parallelism under D23, and the third is on LINUX, so neither serialization nor macOS is required to produce it. The second is the merge-base control for the third: the same signature is on `d97e137` itself, so the branch did not introduce it. That control is ONE run at the base, which is one sample: it establishes that the signature exists on `d97e137`, not its rate. PR #257's second run, 34222042303 at `e78d184`, is GREEN on `Test (ubuntu-latest / luajit)`, and so are its third, fourth and fifth. Green runs are samples: non-reproduction and nothing more. The fourth occurrence is on `main` after two merges that did not touch `src/dispatch` or `tests/m8_1_acceptance.rs`, which is an argument from untouched files and not a measurement; it is the row's third occurrence on `main` and the second on this leg |
| candidate mechanism | the predecessor completed before the cancellation took effect. `--test-threads=1` was the first occurrence's candidate: it serializes the test functions in one executable and so removes one source of contention the test's "in flight" depends on. The second occurrence has no such flag, which does not refute the mechanism --- a fast predecessor is a fast predecessor however the runner got there --- but it does mean serialization is not required to produce it, and the remaining common factor is a macOS or Linux CI runner rather than a scheduling flag. Nothing has measured the predecessor's duration under either, and nothing rules out a real supersede defect |
| retirement | diagnosis; a witness that holds the predecessor in flight deterministically rather than by load |

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

Twenty-eight rows are listed on twenty-seven lines (A1 and A2 share
one). Seventeen lines are causal closures and stand: a wall-clock
assertion made `#[ignore]`, a duplicated test execution removed by the
one-sweep gate, a fixture race fixed with a readiness gate, a
hermeticity fault fixed, and U16's process-global cwd mutation deleted.
The ten that are not now say which word they are. **Recount these
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
