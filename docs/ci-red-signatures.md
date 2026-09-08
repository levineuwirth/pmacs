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
occurrences as comments on the issue. The first three filed that way
are #248 (a compile-mode wait ending before the finalization pass),
#250 (`ETXTBSY` on a freshly written stub) and #251 (a bundled package
failing to load during a parallel library test run).

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

### R7 — managed-retry attach hits a broken pipe under sweep load

| field | value |
|---|---|
| selector | `-p pmacs-gpu attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream` |
| job | local (Linux), inside a workspace sweep; never seen in isolation or in CI |
| required fragments | `transient sequence must attach` + `Handshake(Io(` + `BrokenPipe` (or `code: 32`) |
| occurrences | fourteen, 2026-08-07 to 2026-09-05, all local, all under sweep load; the panic line moves with `attach.rs` and is not part of the signature. The thirteenth and fourteenth: gate logs `20260905T202734Z-1751532` (step `07-sweep`, load average 14.2) and `20260905T205642Z-2051072` (step `05-sweep` of the six-stage gate), both `attach.rs:1889` with all three fragments |
| candidate mechanism | the test drives a scripted transient-then-success sequence over a real socket pair; unknown whether the broken pipe is the fixture's writer closing early or a retry-path defect. Unresolved |
| retirement | hardening that removes the named mechanism plus a discriminating witness, or a diagnosis showing the fixture, not the code, closes the pipe |

What the occurrences establish: the tree is excluded twice over (two
consecutive gate runs on one worktree differing by one markdown file,
green then red; several occurrences on documentation-only commits), and
green reruns number in the dozens. What they do not establish: a
mechanism, or a rate, since nobody has counted runs and failures over a
fixed window. The remaining candidates have to be varied inside a gate
run, one per run.

### U16 — a git invocation finds its working directory deleted

| field | value |
|---|---|
| selector | `--lib packages::fetcher::tests::cache_survives_across_fetcher_instances` |
| job | local (Linux), the workspace sweep |
| required fragments | `Unable to read current working directory: No such file or directory` + `remote did not send all necessary objects` |
| occurrences | three on 2026-08-31 within eleven hours, the third on `main` after a documentation-only merge |
| candidate mechanism | `bare_filename_saves_in_cwd` (`src/file_io.rs`) calls `set_current_dir` on a `TempDir`; concurrently this test's `run_git` spawns `git` with no explicit cwd, so the child inherits the temp directory; the parent restores its cwd, which does nothing for the child; the `TempDir` drops under a live `git`. A candidate with a citation, not a demonstrated chain |
| retirement | run the two selectors concurrently in a tight loop until it reproduces, or remove the process-global mutation (`save_atomic` taking the directory, or that test in a subprocess). A serial guard around `set_current_dir` does not close it: the child outlives the guard |

### U17 — a supersede race lost the other way, on `main`

| field | value |
|---|---|
| selector | `--test m8_1_acceptance read_dir_supersede_cancels_in_flight_predecessor` |
| job | GitHub Actions, the serialized crdt sweep (`--test-threads=1`) |
| required fragments | `first read_dir must be superseded; got ok` |
| occurrences | one, `main` at `aae5b35`, run 33375945966 |
| candidate mechanism | the predecessor completed before the cancellation took effect. `--test-threads=1` serializes the test functions in one executable and so removes one source of contention the test's "in flight" depends on; nothing measured the predecessor's duration with the flag on versus off, and nothing rules out a real supersede defect |
| retirement | diagnosis; a witness that holds the predecessor in flight deterministically rather than by load |

## Closed rows

Each row's full evidence is in this file's history before 2026-09-05.
One line per row: what it was, when it closed, and what closed it.

| row | what it was | closed | closer |
|---|---|---|---|
| R1 | `supersede_cancels_in_flight_job_within_50ms` missed its 50 ms budget | 2026-09-05 | the assertion is a wall-clock budget; it is `#[ignore]` and runs in the perf jobs and `scripts/gate --perf` |
| R2 | `SIGUSR1` delivered before the trap was installed | 2026-08-05 | test race fixed with a readiness gate and an `exec` |
| R4 | readiness predicate satisfied by an empty file | 2026-08-05 | `wait_for_file` waits for the expected bytes, with three witness tests |
| R5 | `async pump deadline exceeded` in the supersede close path, macOS | 2026-09-05 | readiness waits migrated to `tests/common/ready.rs`, which reports elapsed and last-observed state; a recurrence is an issue |
| R6 | readiness file never published in the panel terminal fixture, macOS | 2026-09-05 | same migration; the wait now reports what the child last wrote |
| R8 | LSP listview row rendered relative to a stray ancestor marker | 2026-08-08 | test hermeticity fixed |
| A1, A2 | historical claims with no linked occurrence | 2026-09-05 | nothing was ever measured; a recurrence is an issue |
| U1 | an unclassifiable local red, fragments not captured | 2026-09-05 | unclassifiable; a recurrence is an issue |
| U2 | `m6_1_pty_raw_mode_disables_kernel_echo`, `stty -a` output empty | 2026-09-05 | readiness migration; the PTY read now waits for the record it asserts on |
| U3 | the R7 selector with fragments lost | 2026-09-05 | folded into R7's occurrence count |
| U4 | `a_pty_resize_blanks_the_host_before_repainting`, macOS, three occurrences | 2026-09-05 | not reproduced since 2026-08-15; a recurrence is an issue |
| U5 | `ctrl_c_during_reconnect_sleep_yields_clean_exit`, macOS lua54 | 2026-09-05 | one occurrence, not reproduced; a recurrence is an issue |
| U6 | `criterion_1` and `composition_overhead` red together | 2026-09-05 | both are wall-clock budgets, now `#[ignore]` |
| U7 | a different render-budget test red each sweep (dired, outline) | 2026-09-05 | render budgets, now `#[ignore]` |
| U8 | `acc28_child_input…`, macOS luajit, fragments destroyed | 2026-09-05 | readiness migration (the R6 family); a recurrence is an issue |
| U9 | a PTY test and a budget test red together in one sweep | 2026-09-05 | the budget half is `#[ignore]`; the PTY half is U2's mechanism |
| U10 | the budget red rotating between two runs of one commit | 2026-09-05 | budgets `#[ignore]` |
| U11 | `dispatch_parse_round_trips_a_rust_source_file` missed its parse budget, macOS | 2026-09-05 | the round trip stays in the default run; the 100 ms budget is a separate `#[ignore]` test |
| U12 | a budget test and a PTY test together in `lib-crdt` | 2026-09-05 | budgets `#[ignore]`; the gate no longer has a `lib-crdt` stage |
| U13 | `skipped_directories_are_reported_with_a_reason` received empty child stdout inside the sweep | 2026-09-05 | the gate runs each test once, in one sweep; a recurrence is an issue |
| U14 | four selectors red in one gate run across three stages | 2026-09-05 | three are budgets, now `#[ignore]`; the fourth (`state: initializing`) is a readiness wait, migrated |
| U15 | a rotated multi-red cluster with a load reading | 2026-09-05 | all budgets, now `#[ignore]` |
| U18 | a Go checksum-database fetch failed before anything was built | 2026-09-05 | not a test; filed as issue #249 |
| U19 | a terminal bell not observed within a 5 s poll | 2026-09-05 | readiness migration; the wait reports the last frame seen |
| U20 | `composition_overhead` red alone | 2026-09-05 | a budget, now `#[ignore]` |
| U21 | `m6_1_pty_canonical_mode_keeps_kernel_echo` red alone in `lib` | 2026-09-05 | U2's mechanism; the gate runs each test once |
