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
| retirement | diagnosis. The deadline now reports its subject, its elapsed time and its poll count, so the next occurrence says whether it missed by a millisecond or by two seconds --- that is a step toward the diagnosis and is not itself a closer. Never a green rerun |

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
| job | GitHub Actions, the serialized crdt sweep (`--test-threads=1`) |
| required fragments | `first read_dir must be superseded; got ok` |
| occurrences | three: `main` at `aae5b35`, run 33375945966 (the serialized crdt sweep); `main` at `d97e137`, run 34205653191, job `Test (macos-latest / luajit)`; and PR #257 at `8f6784f`, run 34220035122, job `Test (ubuntu-latest / luajit)`. The last two run at cargo's DEFAULT parallelism under D23, and the third is on LINUX, so neither serialization nor macOS is required to produce it. The second is the merge-base control for the third: the same signature is on `d97e137` itself, so the branch did not introduce it. That control is ONE run at the base, which is one sample: it establishes that the signature exists on `d97e137`, not its rate. PR #257's second run, 34222042303 at `e78d184`, is GREEN on `Test (ubuntu-latest / luajit)`, and so is its third, 34253949749 at the head `b2094ac`. Two green runs are two samples: non-reproduction and nothing more |
| candidate mechanism | the predecessor completed before the cancellation took effect. `--test-threads=1` was the first occurrence's candidate: it serializes the test functions in one executable and so removes one source of contention the test's "in flight" depends on. The second occurrence has no such flag, which does not refute the mechanism --- a fast predecessor is a fast predecessor however the runner got there --- but it does mean serialization is not required to produce it, and the remaining common factor is a macOS or Linux CI runner rather than a scheduling flag. Nothing has measured the predecessor's duration under either, and nothing rules out a real supersede defect |
| retirement | diagnosis; a witness that holds the predecessor in flight deterministically rather than by load |

## Closed rows

Each row's full evidence is in this file's history before 2026-09-05.
One line per row: what it was, when it left this section's live half,
and on what grounds.

**The grounds are not all the same kind, and the word says which.** A
row with a bare date is a causal closure: its mechanism was removed or
explained, which is the only retirement the rerun rule allows.

- **VOID** --- the closure was not a closure when it was written. Either
  it named no mechanism at all and rested on a count of green runs
  (U4, U5), or it named one that never reached the failing site (R5).
  A void row was never retired; it was only stopped being looked at,
  and it is live again above.
- **INCOMPLETE** --- the closure named a real mechanism that did reach
  the failing site, but that mechanism did not remove the cause. U8 is
  the one, and it is falsified rather than suspected.
- **DISCARDED** --- there was never evidence to retire. The fragments
  were lost or never captured, so the row could not match anything.
  This is an admission about the record, not a finding about the code,
  and it must never be counted as a causal closure.
- **MERGED** --- the question moved to another row and is live there.
- **REFERRED** --- not a test, so outside this file; the question lives
  in a GitHub issue.

Twenty-eight rows are listed on twenty-seven lines (A1 and A2 share
one). Nineteen lines are causal closures and stand: a wall-clock
assertion made `#[ignore]`, a duplicated test execution removed by the
one-sweep gate, a fixture race fixed with a readiness gate, a
hermeticity fault fixed, and U16's process-global cwd mutation deleted.
The eight that are not now say which word they are. **Recount these
against the table below rather than trusting them**: they were written
as twenty-seven, twenty-six, eighteen and eight one commit before U16
closed and moved into it, and were wrong until this line was rewritten
by counting again.

| row | what it was | disposition | grounds |
|---|---|---|---|
| R1 | `supersede_cancels_in_flight_job_within_50ms` missed its 50 ms budget | 2026-09-05 | the assertion is a wall-clock budget; it is `#[ignore]` and runs in the perf jobs and `scripts/gate --perf` |
| R2 | `SIGUSR1` delivered before the trap was installed | 2026-08-05 | test race fixed with a readiness gate and an `exec` |
| R4 | readiness predicate satisfied by an empty file | 2026-08-05 | `wait_for_file` waits for the expected bytes, with three witness tests |
| R5 | `async pump deadline exceeded` in the supersede close path, macOS | 2026-09-05, **VOID** | the closure named the `tests/common/ready.rs` migration, which cannot reach an in-crate unit test and so never applied to the failing site. R5 is live again above, as never-closed |
| R6 | readiness file never published in the panel terminal fixture, macOS | 2026-09-05 | same migration; the wait now reports what the child last wrote |
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
| U19 | a terminal bell not observed within a 5 s poll | 2026-09-05 | readiness migration; the wait reports the last frame seen |
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
shape and nothing has falsified either, so they stand as written --- on
notice, not reclassified. U2 (`the PTY read now waits for the record it
asserts on`) and R4 (`wait_for_file waits for the expected bytes`) are
the causal form and are not in question.

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
| 34220035122 | `8f6784f` | 2026-09-08T11:18:55Z | 11:38:36Z | 12 green, 3 red |
| 34222042303 | `e78d184` | 11:41:44Z | 12:01:39Z | 13 green, 2 red |
| 34253949749 | `b2094ac` | 16:54:59Z | 17:09:27Z | 16 green, 1 red |

All three are `pull_request` events with conclusion **failure**, and
`Docs consistency` is skipped in all three, correctly: the PR's changed
paths include code. The first two ran trees differing by one markdown
file; the third ran fix round 1's seven further commits.

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
