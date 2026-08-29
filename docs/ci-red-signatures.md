# CI red signatures — the triage registry

**This file is the single authority for judging a red CI run.** It is an
occurrence ledger, not a flake list: a row records what was seen, what is
known about why, and what would retire it. **A row is not a claim that
the failure is harmless.**

Deliberately not named "flakes". One of its rows is a possible product
defect, and a filename that called it a flake would confer immunity the
evidence does not support.

Read this before attributing any red run to the environment. Landed
framing documents keep their own historical evidence and reasoning —
that is not duplication, and it is not superseded by this file. What
lives here is **live triage policy**.

---

## How a row matches

**A test-name match is never sufficient.** A red run matches a row only
when *all three* hold:

1. the **exact test selector** matches;
2. the **job / flavor** matches;
3. **every required fragment** is present in the failure output.

Where a fragment lists alternatives (`ESRCH` / `No such process`), any
one satisfies that requirement — those are the same condition rendered
differently by platform or libc.

Fragments are **normalized**, never pasted verbatim. PIDs, elapsed
times, thread ids and rendered OS-error suffixes vary between runs; a
verbatim key would match nothing. The evidence link preserves the exact
occurrence.

**A failure in a listed test that does not carry that row's fragments is
a NEW incident**, judged on its own. The process test below is why this
rule exists: it produced two signatures with different mechanisms and
different causal status, and only one of them is a test bug.

---

## The rerun rule

This replaces "rerun before concluding", which conflated three different
outcomes.

- **A green rerun after a red establishes INTERMITTENCE ONLY.** It does
  not establish environmental cause, harmlessness, or retirement.
- **The same signature on the rerun is a SECOND OCCURRENCE.** It remains
  blocking pending investigation or a merge-base control.
- **A different signature is a NEW INCIDENT**, judged independently.

A merge-base control — running the same command on the merge base — is
what distinguishes "this branch caused it" from "this tree has it". It
is cheaper than argument and is the correct response to a second
occurrence.

---

## What retires a row

**Retirement is causal, never a count of green runs.** A row is retired
by removing or explaining its mechanism:

| causal status | retirement condition |
|---|---|
| **test race** | hardening that removes the named mechanism, plus a discriminating witness for the stronger predicate |
| **measurement design** | the owning lane replaces or justifies the measurement and pins the resulting claim |
| **unresolved** | diagnosis and an explicit disposition |

**Audit notes (`A`-numbers) have no retirement condition**, because they
have nothing to retire — see that section. A linked occurrence promotes
one into an `R` row; absence retires nothing, because nothing was ever
measured.

Main-branch greens are **occurrence evidence** and accumulate toward a
rate. They retire nothing by themselves. Retired rows stay in this file
with their disposition, so a recurrence is recognisable.

**A red matching a RETIRED row AND POSTDATING ITS RETIREMENT is a
recurrence, and it puts the retirement in question — it is not a known
flake.** The claim a retirement makes is that the mechanism is gone
*from the retirement forward*; the same signature afterwards falsifies
that claim, which is a stronger finding than a live row, not a weaker
one. Reopen the row rather than rerunning.

**A matching red that PREDATES the retirement corroborates the row
instead.** It is an additional occurrence of the mechanism the fix
removed, so it strengthens the evidence and challenges nothing. Add it
to the row's evidence; do not reopen. This is not a technicality — an
occurrence scan reaches backwards by construction, so most matches it
finds will be of this kind, and treating them as recurrences would
reopen every retired row the first time anyone looked.

**Both halves need the date, which means a row is unmatchable without
one.** R2's second occurrence (`main` run 30710662474, 2026-08-01, four
days before its 2026-08-05 retirement) is the worked example, and it
also shows the flavor field is not part of matching: R2's first
evidence is macOS / lua54 and that one is macOS / luajit.

---

## Live rows

**Four of the seven evidenced rows are live.** R2 and R4 were retired
on 2026-08-05 and **R8 on 2026-08-08**; all three are below, under
"Retired rows", with their dispositions. R5 and R6 were added on
2026-08-06 from an occurrence scan and a live red; neither is
diagnosed.

**Rate, as of 2026-08-06.** Of the last 25 `main` runs, **23 green and
2 red** — the two reds being R2's second occurrence (30710662474) and
R5 (30555667095). R6 has **no** occurrence in that window; it was first
seen on a PR branch. This is the first rate this file has carried, and
it is a floor rather than a measurement: it counts only `main`, only 25
runs, and only reds that were still readable.

### R1 — supersede cancellation budget

| field | value |
|---|---|
| **selector** | `--lib async_runtime::tests::supersede_cancels_in_flight_job_within_50ms` |
| **job / flavor** | macOS / luajit |
| **required fragments** | `supersede did not cancel within 50ms` |
| **causal status** | **measurement design** |
| **evidence** | [#213 run 30826884642](https://github.com/levineuwirth/pmacs/actions/runs/30826884642) |
| **retirement** | the async-runtime lane replaces or justifies the measurement (Q#MCI3) |

The test's premise is `thread::sleep(15ms)`, asserted by comment to mean
"the worker picked the job up"; under load it may not have, in which case
the test measures the *queued* path while claiming the running one. And
its 50ms clock starts before the second dispatch and is consumed by the
test's own `tick()` + `sleep(1ms)` pump, so the interval is dominated by
when *the test* was scheduled. **Widening the budget would make it pass
and measure nothing more.**

### R3 — live-leader EPERM with an unobservable group

| field | value |
|---|---|
| **selector** | `--lib process::tests::a_successful_signal_disposition_depends_on_whether_it_is_fatal` |
| **job / flavor** | macOS / lua54 |
| **required fragments** | `EPERM` **and** `measured_group=unobservable(` **and** (`ESRCH` / `No such process`) **and** `leader=live` |
| **causal status** | **UNRESOLVED — possible product defect** |
| **evidence** | [#214 run 30932558752 attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30932558752/attempts/1) |
| **retirement** | **diagnosis and disposition by the process-signal / reap-ledger lanes. Never a green rerun.** |

**Same test as R2, different mechanism, different status.** This is the
group-target behaviour #176 and #200 circled and the reap-ledger lane
parked every disposition change pending: a group-directed `kill` returned
EPERM while the leader was observed live, and `measured_group` — the one
field able to disagree — could not be read at all.

**Do not treat a red matching this row as environmental.** A green rerun
changes nothing about it.

**R2's retirement does not touch this row, and must not be read as
touching it.** The 2026-08-05 hardening changed that test's *fixture* —
a readiness gate and an `exec` — and changed **no product code at all**;
the same group-directed `kill` runs against the same supervisor. What
the fixture change does do is alter the shape of the group being
signalled (one process now, where a forked `sleep` could make two), so
**a change in how often this row appears would be evidence about
frequency, not about cause**. A red carrying these fragments after that
date is this same unresolved row, and its retirement is still a
diagnosis by the process-signal / reap-ledger lanes.

---

### R5 — async pump deadline exceeded in the supersede close path

| field | value |
|---|---|
| **selector** | `--lib editor::tests::stream_supersede_delivers_cancelled_to_on_close` |
| **job / flavor** | macOS / lua54 |
| **required fragments** | `async pump deadline exceeded` |
| **causal status** | **UNRESOLVED — no diagnosis** |
| **evidence** | `main` [run 30555667095](https://github.com/levineuwirth/pmacs/actions/runs/30555667095), 2026-07-30 |
| **retirement** | diagnosis by the async-runtime lane, alongside R1's measurement question (Q#MCI3). **Never a green rerun.** |

**Not R1, though it is the nearest thing to it.** R1 is
`async_runtime::tests::supersede_cancels_in_flight_job_within_50ms`
failing `supersede did not cancel within 50ms`; this is a different
test in a different module failing a different assertion. They share a
subject — supersede, under a deadline, on macOS — and sharing a subject
is not sharing a signature. Filed separately so that a fix for one is
not read as a disposition for the other.

What the row does **not** claim: that the pump is slow, that the
deadline is wrong, or that this is the same measurement-design problem
R1 has. Nothing here has been diagnosed. It is recorded because it
happened and had a signature, which is the entire bar for a row.

### R6 — readiness file never published in the panel terminal fixture

| field | value |
|---|---|
| **selector** | `--test bottom_panel_stage1_acceptance acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel` |
| **job / flavor** | macOS / lua54 |
| **required fragments** | `timed out waiting for` **and** `/ready` |
| **causal status** | **UNRESOLVED — no diagnosis** |
| **evidence** | #217 [run 31023651701](https://github.com/levineuwirth/pmacs/actions/runs/31023651701), 2026-08-05 |
| **retirement** | the readiness helpers are audited and reconciled, with a witness. **Never a green rerun** — the next push was green and that retires nothing. |

**A THIRD copy of the readiness helper.** R4's disposition already
recorded that the empty-file predicate lived in a second helper
(`wait_for_published_file`) and warned that leaving it would let the
mechanism recur under a different selector as a new incident.
`tests/bottom_panel_stage1_acceptance.rs:2446` is a third,
independently written `wait_for_file` carrying only *half* the
hardening: it rejects a zero-byte file (`!bytes.is_empty()`) but never
waits for the expected content.

So the honest statement is narrow. This red is a **timeout**, not R4's
`left: [] / right: [49]`, and a timeout is what a *correct* predicate
does when the content never arrives — it is not evidence of the R4 bug.
What is established is only that a third copy exists and diverges from
the other two. **Whether this occurrence is a slow runner, a child that
never published, or something in the panel path is not known**, and the
suite passed 5/5 locally on Linux, which reproduces nothing about a
macOS runner and is not evidence about this occurrence.

The scope this row implies is the audit, not the test: how many
readiness helpers exist, whether they can be one, and what each
promises. Patching this call site alone would leave the same question
open under a fourth selector.

## Retired rows

**These stay here on purpose.** A retirement is a claim that a mechanism
is gone; keeping the signature is what makes a recurrence recognisable
as a falsification of that claim rather than as a fresh mystery. Both
were retired **causally** — the mechanism removed, plus a discriminating
witness that fails without the fix — never by a count of green runs.
**R8 joined them on 2026-08-08**, and unlike the other two it was never
intermittent — it was deterministic, and the "flake" reading was never
available to it.

### R8 — LSP listview row rendered relative to a stray ancestor marker — RETIRED 2026-08-08

| field | value |
|---|---|
| **selector** | `--test m4_acceptance flat_listview_consumers_render_byte_identically_after_the_tree_extension` |
| **job / flavor** | local (Linux), any invocation — isolated single-test runs included. Not load-sensitive |
| **required fragments** | `the flat references row renders verbatim` **and** a `left` value that is the `right` value with a **leading directory removed** |
| **causal status** | **DIAGNOSED and FIXED — test hermeticity** |
| **evidence** | reproduced deterministically on one Linux workstation; merge-base control confirmed it on `main` |
| **disposition** | `tests/m4_acceptance.rs::open_against_fake` now sets `pmacs.project.set_search_boundary` to the fixture directory. `docs/r8-fixture-boundary-framing.md` |

**Mechanism, established rather than guessed:**

1. `builtin/runtime/lsp.lua:2397` `display_path` shortens a location
   against the **detected project root** before rendering it.
2. `pmacs.project.detect` walks **upward** for a marker. From
   `/tmp/.tmpXXXXXX/r.rs` it reached `/tmp`.
3. That machine had a stray **`/tmp/.git`** — an *empty directory*, not
   a repository. The `.git` marker is directory-only, so an empty
   directory still matched.
4. Root resolved to `/tmp`, the prefix was stripped, and the row
   rendered as observed.

Control at diagnosis time: the same test with `TMPDIR` outside `/tmp`
passed.

**The product behaviour was never wrong and was not changed.**
Shortening a location against its project root is the feature. The
defect was that the fixture did not bound its own project detection, so
its assertion depended on what the developer's `/tmp` contained.
`src/project.rs:208` had documented this exact hazard — *"a developer's
`/tmp/.git`"*, in those words — and provided
`detect_project_within`; `open_against_fake` was one helper that missed
the pattern the same file already used five times.

**Retired causally, and the witness is portable.** A new test,
`a_planted_ancestor_marker_does_not_reach_the_rendered_row`, **plants
an empty `.git` in a temporary ancestor** and asserts the rendered row
stays absolute. Removing the boundary fails it *deterministically on
every machine* — including CI, where no `/tmp/.git` exists — because
the planted marker is nearer than any real one. **The original
machine's stray directory was corroboration, never the proof**, and it
was deliberately left in place: deleting it would have hidden the
hermeticity defect.

**Provenance of that `/tmp/.git` remains unresolved and is not needed.**
Observations of its timestamps disagreed, and `/tmp` is a tmpfs whose
entries are touched by inspection, so no timestamp is authoritative
here. It may have been created by the session that found the row. The
fix does not depend on the answer.

**What this retirement does NOT claim:** that the rest of the suite is
hermetic. `EditorState::new_with_roots` is called **113 times** in
`m4_acceptance` alone, an unknown number of them equally unbounded —
harmless only while their assertions do not render a path. That census
is a named follow-on in `docs/agent-handoff.md` §6, not a completed
audit.

### R2 — USR1 delivered before the trap is installed — RETIRED 2026-08-05

| field | value |
|---|---|
| **selector** | `--lib process::tests::a_successful_signal_disposition_depends_on_whether_it_is_fatal` |
| **job / flavor** | macOS / lua54 |
| **required fragments** | `leader=exited(signal SIGUSR1)` — **one exact fragment, not two loose ones**. Split into `leader=exited(` and `SIGUSR1` it would match a child that exited by some *other* disposition while `SIGUSR1` appeared elsewhere in the output |
| **causal status** | **test race** |
| **evidence** | [#213 run 30927084982 attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30927084982/attempts/1); **and `main` [run 30710662474](https://github.com/levineuwirth/pmacs/actions/runs/30710662474), 2026-08-01, macOS / *luajit*** — same exact fragment on the **other flavor**, found by an occurrence scan on 2026-08-06 |
| **retirement condition** | the fixture proves the trap is installed, with a witness that fails without it |
| **disposition** | **met.** The child publishes a readiness marker *after* `trap '' USR1`, the test waits for that marker's **content**, and `process::tests::usr1_readiness_waits_for_the_trap_not_for_the_spawn` is the witness |

Readiness was `ProcessEventKind::Started`, emitted at **spawn** — not when
`/bin/sh` has parsed `trap '' USR1`. SIGUSR1's default disposition is
terminate, so a signal inside that window kills the child. The fixture's
own comment stated the requirement it did not enforce.

Three things the fix and the witness settled that the row did not say:

- **Which call failed.** The fragment is rendered only on a *failed*
  `kill`, and the USR1 that killed the child cannot itself have failed —
  it is what did the killing. The failing call is therefore the SIGTERM
  that follows, whose diagnostic reports the leader's earlier death.
  **Why a group-directed TERM found no group is not established here**,
  and the fix does not depend on the answer. **This says nothing about
  R3**, whose leader was observed **live**.
- **The witness proves the predicate, not the platform.** The old fixture
  passes on Linux; the window is real everywhere but only macOS ever
  reported it. So the witness widens the pre-trap window *deliberately*
  (the fixture sleeps before `trap`) instead of hoping a loaded runner
  supplies one, and it proves survival by the child's **exit
  disposition** — a child that took the USR1 reports
  `Signaled { signal: "SIGUSR1" }` — rather than by an absence observed
  within a window.
- **The second occurrence is pre-retirement, and that is the whole
  question.** `main` run 30710662474 predates the 2026-08-05 retirement
  by four days, so it corroborates the row rather than falsifying its
  disposition — a red matching a retired row is a recurrence **only if
  it postdates the retirement**, and the rule above is about the claim
  a retirement makes going forward. It does add something: this row's
  evidence was macOS / lua54 and this one is macOS / **luajit**, so the
  mechanism was never flavor-specific, which is what the fix already
  assumed when it widened the window deliberately rather than hoping a
  loaded runner supplied one.

  *It was also very nearly filed as R3.* Same test, same `EPERM`, same
  `measured_group=unobservable(ESRCH…)` — and R3 explicitly requires
  `leader=live`, where this reads `leader=exited(signal SIGUSR1)`.
  Matching on the shared fragments and the shared test name would have
  attached a live, unresolved possible-product-defect row to an
  occurrence of a retired test race. **This is why the rows key on
  exact fragments and why R2's says "one exact fragment, not two loose
  ones."**
- **The fixture had a second, unnamed dependency.** These signals are
  group-directed, so a forked `sleep` would be an *untrapped* member of
  the same group. That it survived at all depended on the shell
  suppressing the fork for the last command of a `-c` script — a bash
  and dash optimization, not a guarantee. The fixture now says `exec`,
  and an ignored disposition survives `exec` by POSIX.

### R4 — readiness predicate satisfied by an empty file — RETIRED 2026-08-05

| field | value |
|---|---|
| **selector** | `--test vterm_stage2_acceptance terminal_escape_gates_local_bindings_and_double_escape_sends_interrupt` |
| **job / flavor** | macOS / luajit |
| **required fragments** | `left: []` **and** `right: [49]` |
| **causal status** | **test race** |
| **evidence** | [#214 run 30932558752 attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30932558752/attempts/1) |
| **retirement condition** | `wait_for_file` requires the expected content, with a witness that fails against a zero-byte file |
| **disposition** | **met.** The helper takes the expected bytes and waits while the file holds a **strict prefix** of them; `wait_for_file_does_not_return_a_zero_byte_readiness_file` is the witness, and with the old predicate restored it fails with `left: []`, `right: [49]` — **this row's two fragments, verbatim** |

`wait_for_file` returned as soon as `fs::read` succeeded — which succeeds
on a **zero-byte file**. The probe writes readiness with
`open(path,'wb').write(b'1')`, and `open()` creates the file before
`write()` fills it. The predicate was "readable"; the assertion is
"contains `1`" (`49` is ASCII `'1'`).

Two notes for anyone who reads a future red here:

- **The same mechanism lived in a second helper.** `wait_for_published_file`,
  one function away in the same suite, gated the real-TUI smoke's
  `assert_eq!(…, b"1")` on the identical "readable" predicate. It was
  fixed with the same predicate; leaving it would have let this row
  recur under a different selector, which the registry would have had to
  judge a new incident.
- **The helper returns divergent content instead of waiting for a
  match**, so a child that publishes the *wrong* thing is reported as a
  diff by the caller that owns the expectation, rather than as a timeout
  in a helper that does not. That behaviour has its own witness.

---

## Audit notes — historical claims with no linked occurrence

**These are NOT registry rows.** They carry `A`-numbers, not `R`-numbers,
because nothing here can be matched against a red run and nothing here
confers any status.

They were named in the handoff's hazards list without evidence. The audit
found the tests real and the claims recorded in good faith — but **an
assertion string existing is not a mechanism, and "timing-based" is not
an observation.** No occurrence of either was ever linked, so nothing is
known about how either fails, or whether either has failed.

Deleting them would discard a real recorded belief. Listing them beside
the evidenced rows would grant the reputation this file exists to deny.
So they are stated as what they are: **claims awaiting a first
occurrence.** A red in either test is a first recorded occurrence, to be
investigated and then promoted to an `R` row — not matched against
anything here.

### A1 — GPU terminal cell background did not paint

| field | value |
|---|---|
| **selector** | `-p pmacs-gpu a33_headless_terminal_frame_paints_cells_without_document_layers` |
| **job / flavor** | GPU Render (headless), under parallel load |
| **required fragments** | `the terminal cell background did not paint` + `blue pixels` |
| **status** | **historical claim, no linked occurrence** |
| **what IS established** | the test exists and the assertion string is real (`pmacs-gpu/src/main.rs:17973`). That is all |
| **what is NOT** | any mechanism, and any occurrence. No run was ever cited |
| **promotion** | a linked occurrence makes this an `R` row with a signature. Absence retires nothing, because nothing was measured |

### A2 — supervisor reap across cycles

| field | value |
|---|---|
| **selector** | `--test m6_8_multi_repl_acceptance m6_8_supervisor_reaps_all_children_across_cycles` |
| **job / flavor** | not recorded |
| **required fragments** | **not recorded** — no signature was ever captured |
| **status** | **historical claim, no linked occurrence** |
| **what IS established** | the test exists and runs 10 cycles; the handoff called it "timing-based" |
| **what is NOT** | any mechanism, any signature, any occurrence |
| **promotion** | a linked occurrence *with a captured signature* makes this an `R` row |

**A2 cannot be matched, and neither can A1** — that is what makes them
notes rather than rows. A red in either test is a new incident by
default. That is the correct outcome for an entry that never carried
evidence, and it means this file is **stricter** than the list it
replaces: nothing is pre-excused.

---

## Occurrence log

| date | run | row | outcome |
|---|---|---|---|
| 2026-08-04 | [30826884642](https://github.com/levineuwirth/pmacs/actions/runs/30826884642) | R1 | rerun green — intermittence only |
| 2026-08-04 | [30927084982 att.1](https://github.com/levineuwirth/pmacs/actions/runs/30927084982/attempts/1) | R2 | rerun green — intermittence only |
| 2026-08-04 | [30932558752 att.1](https://github.com/levineuwirth/pmacs/actions/runs/30932558752/attempts/1) | R3, R4 | rerun green — intermittence only; **R3 remains unresolved** |

Four incidents, three tests, **four signatures**. Count signatures: the
process test contributed two, and only one of them is a test bug.

| date | row | event |
|---|---|---|
| 2026-08-05 | R2, R4 | **retired** — mechanism removed, discriminating witness added; see "Retired rows" |

### U1 — an unclassifiable local red (long-lines lane, 2026-08-07)

Recorded because the alternative is to not record it. It is **not** a
row, cannot be matched, and excuses nothing.

| field | value |
|---|---|
| **selector** | `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu` — **test name not captured** |
| **job / flavor** | local (Linux), immediately after a 60s `m4_acceptance` run |
| **required fragments** | **none captured** |
| **status** | **unclassifiable — evidence destroyed at capture time** |
| **what IS established** | `test result: FAILED. 227 passed; 1 failed` was emitted once |
| **what is NOT** | which test, why, and whether the lane caused it |
| **cause of the gap** | the command piped through `tail -3`, which kept the summary line and discarded the failure block above it |

**Not matched against A1** despite A1 also being GPU-headless-under-load.
Matching needs an exact selector and every required fragment; this has
neither, and treating a shapeless red as "probably the known one" is
precisely the reputation-by-adjacency this file exists to deny.

Follow-up: 36 subsequent full runs clean, 6 of them under deliberate
concurrent load (`m4_acceptance` in parallel). Per the rerun rule that
establishes **intermittence only** — and here not even that, since
without a name there is nothing to call intermittent.

**The lesson is mechanical, not analytical: never pipe a gate through
`tail`/`head` on the run whose result you intend to report.** Filter
with `grep -E "FAILED|panicked|test result"`, which keeps failure
context, or capture the full log to a file and summarize from it.

### R7 — managed-retry attach hits a broken pipe under full-sweep load

The first incident this session with a **complete** signature, so it is
a matchable row rather than a `U` note. Recorded during long-lines
Stage 4; the lane touches no `pmacs-gpu` code at all.

| field | value |
|---|---|
| **selector** | `-p pmacs-gpu attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream` |
| **job / flavor** | local (Linux), `cargo test --workspace --features crdt --no-fail-fast`, i.e. under full-sweep load |
| **required fragments** | `transient sequence must attach` + `Handshake(Io(` + `BrokenPipe` (or `code: 32`) |
| **status** | **SEVENTH OCCURRENCE 2026-08-29 — causal status still UNRESOLVED.** The sixth and seventh came back to back on one lane and are written up together below; the fifth carries the strongest tree exclusion this row has had, a **documentation-only diff** |
| **what IS established** | **three** occurrences at `pmacs-gpu/src/attach.rs:1680`, the second and third with all three fragments **verified** rather than inferred; the test drives a scripted transient-then-success sequence over a real socket pair. **The added GPU test is not the mechanism** — see the third-occurrence control below |
| **what is NOT** | whether the broken pipe is the *fixture's* writer closing early or a real retry-path defect. **This row is not a claim that it is harmless** |
| **rerun evidence** | occurrence 1: 6 isolated runs green, plus a full `--workspace --features crdt` sweep green (113 targets). Occurrence 2: **30 green on the observing branch** (15 isolated selector, 15 full `-p pmacs-gpu`) **plus a 15-run merge-base control, also green**. Occurrence 3: 5 isolated selector runs green, 10 full `-p pmacs-gpu` runs green **with** the added test, and **1 failure in 10 with the added test `#[ignore]`d** — the first rerun in this row's history that reproduced anything. Per the rerun rule the green runs establish intermittence only; the red control run is what carries the exclusion |
| **retirement** | hardening that removes the named mechanism plus a discriminating witness — or a diagnosis showing the fixture, not the code, closes the pipe |

**Sixth occurrence — the parse-budget diagnosability lane, 2026-08-29,
local (Linux), `gpu` step.** All three required fragments present in the
durable log
(`pmacs-parse-budget-9c27ecfe/gate-logs/20260829T144541Z-350549/06-gpu.log`):

```
transient sequence must attach: Attach(Handshake(Io(Os { code: 32,
kind: BrokenPipe, message: "Broken pipe" })))
```

at `pmacs-gpu/src/attach.rs:1889`, `283 passed; 1 failed`.

* **The tree exclusion is as strong as the fifth's.** The observing
  lane's entire diff is `src/async_runtime.rs`,
  `tests/m4_acceptance.rs` and three docs — **no `pmacs-gpu` file, and no
  file `pmacs-gpu` links against beyond the workspace it always did.**
  The change is two `assert!` message strings.
* **Rerun: isolated selector green three times** (`1 passed`, 0.01 s
  each), which is this row's established control shape.
* **THE SAME RUN'S `sweep` STAGE WAS TRUNCATED, and is not evidence.**
  The gate was running in a background task that was killed at 314s;
  `07-sweep.log` ends in `Terminated`. That stage's absence says
  nothing, and the run as a whole is **not** a gate result. Only the
  `gpu` stage's failure is, because it completed and reported.
**A SEVENTH OCCURRENCE FOLLOWED IMMEDIATELY**, on the next gate run of
the same worktree at head `45d438c`
(`20260829T150011Z-429115/06-gpu.log`), same selector, same three
fragments, `283 passed; 1 failed`. **That run's other seven stages were
green**, `sweep` included and complete — 121 result lines, none with a
failure — so this pair is not confounded by a truncation the way the
sixth was.

**Two consecutive in-gate failures is new for this row**, whose prior
five were spread across lanes and months. It prompted a narrowing, and
the narrowing is the useful part.

**A THIRD IN-GATE RUN WAS GREEN** (head `68a16f9`, log
`20260829T152024Z-563254`, all eight stages, zero failures anywhere).
So "in-gate always fails" is **false**, and the paragraph below was
written before that run and is corrected rather than deleted.

**Seventeen green runs outside the gate, in four configurations, all at
head `45d438c` on the failing worktree:**

| condition reproduced outside the gate | runs | result |
|---|---|---|
| isolated selector | 3 | green |
| full `-p pmacs-gpu` binary | 6 | green |
| full binary under a gate-shaped 61-character `TMPDIR` (tested because this project already knows socket-path length matters) | 6 | green |
| `m4` then `gpu` back to back, as the gate orders them | 2 pairs | green |

**THESE ARE NOT EXCLUSIONS, and an earlier version of this entry called
them that.** The reasoning was wrong: **nothing outside the gate has
ever reproduced this failure**, in 17 runs across four configurations —
so matching one gate condition at a time *outside* the gate cannot
isolate an in-gate cause. All these runs establish is that none of the
four conditions **by itself** reproduces the failure. They do not show
that any of them is uninvolved when the gate supplies the rest.

**THE OBSERVATION WINDOW IS BOUNDED, deliberately.** It is the four
in-gate runs of 2026-08-29 up to and including the first green —
`20260829T144541Z`, `T150011Z`, `T152024Z`, `T152824Z` — plus the 17
out-of-gate runs taken between them. **Later head-exact verification
gates on this lane are NOT part of it and do not move these numbers.**
Without that boundary the tally re-counts itself every time a review
round adds a docs commit and the gate is re-run, which is a ratio that
drifts with review activity rather than with the phenomenon.

**What the window supports**, at the strength it carries: in-gate
**2 failures in 4**; out-of-gate **0 failures in 17**. That asymmetry
is suggestive and it is not a clean split,
because the third in-gate run passed.

**The method for the next occurrence follows from that.** Varying
conditions outside the gate cannot answer this question. It has to be
varied INSIDE — the gate's ambient root, its exported environment, and
process state carried across stage boundaries are the uneliminated
candidates, and each would need a gate run with that one thing changed.

**Causal status: still UNRESOLVED.** What these occurrences add is a
sharper question and a method, not a cause: previous entries compared
lanes and trees, and these locate the asymmetry in the *runner* while
showing that the obvious way to probe it — reproducing gate conditions
outside the gate — cannot work.

**Fifth occurrence — panel cell-mapping generation (§5b) framing,
2026-08-15, local (Linux).** The `scripts/gate` **`gpu` step** again,
the same flavor as occurrence 2, inside a `--protocol` run
(log `20260815T072601Z-2230169`).

* **All three fragments verified** from the durable log, not a filtered
  stream: `transient sequence must attach: Attach(Handshake(Io(Os {
  code: 32, kind: BrokenPipe, message: "Broken pipe" })))`.
* **The line moved and that is not a fragment.** It is
  `pmacs-gpu/src/attach.rs:1728` here against `:1680` in the earlier
  occurrences — `attach.rs` has changed since, and this row's
  convention already treats a `:LINE` suffix as occurrence-specific.
* **The tree exclusion is the strongest available in this row's
  history: the branch's entire diff is DOCUMENTATION.** No Rust, no
  wire surface, no `pmacs-gpu` file. Occurrences 1 and 4 argued
  "unrelated lane"; this one cannot be related at all.
* **Rerun: isolated selector green** (`1 passed`, 0.01 s). Per this
  file's rerun rule that establishes **intermittence only** and does
  not exonerate the tree — though here there is no tree change to
  exonerate.

**What five occurrences now support, stated carefully:** the failure is
**not lane-correlated**. It has appeared under three flavors across
five unrelated lanes, once on a diff that touches no code whatsoever.
That is evidence about *where the cause is not*, and still says nothing
about what it is. **The retirement condition is unchanged.**

**Not attributed to the observing lane**, and in neither case is the
reasoning merely "my diff looks unrelated": long-lines Stage 4 added no
wire surface, no protocol version change, and touched no file in
`pmacs-gpu`.

**Second occurrence — worker identity Stage 1, 2026-08-09, local
(Linux).** Recorded at the `scripts/gate` **`gpu` step**
(`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`), which is a **third
flavor**: not the `--features crdt` sweep of occurrence 1, and not U3's
default-features workspace sweep. Two things make it a match rather than
a `U` note:

* **The fragments were captured this time.** `transient sequence must
  attach: Attach(Handshake(Io(Os { code: 32, kind: BrokenPipe, message:
  "Broken pipe" })))` — all three of the row's required fragments,
  verified against the durable gate log rather than a filtered live
  stream. **That is what U2 and U3 both lost**, and it is why U3 could
  not be judged a recurrence. Reading the gate's own `NN-gpu.log` is the
  mechanical fix U3 prescribed, and it worked.
* **The merge-base control R7 asked for was run** — 15 runs at `4bc55e8`,
  green. It is **non-discriminating**, not exculpatory: the observing
  branch was equally green over 30 runs, so neither side reproduced and
  the control separates nothing. Recorded as a null result rather than
  as evidence.

**One causal path is NOT excluded and is named here rather than
dismissed.** The observing lane added a test to `pmacs-gpu`'s test module
(`main.rs`) — a GPU-heavy `render_offscreen` case. It touches no
`attach.rs`, no protocol, and no wire, but it does add a concurrent test
to the same binary, and the failing test is a socket handshake with a
one-second deadline. Contention is a plausible mechanism for a
`BrokenPipe`, and 30 green runs do not rule it out. If a third occurrence
lands, **run the control with the added test removed** rather than at the
merge base — that is the discriminating comparison this one was not.

**Fourth occurrence — the `scripts/gate` TMPDIR isolation lane,
2026-08-13, local (Linux). Same selector, same `gpu`-step flavor
(`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`), all three fragments
verified** against the durable gate log
(`20260813T143421Z-708100/07-gpu.log`): `transient sequence must
attach: Attach(Handshake(Io(Os { code: 32, kind: BrokenPipe, message:
"Broken pipe" })))`.

**One thing this occurrence ESTABLISHES.** The observing lane touches
**no `pmacs-gpu` file at all** (`git diff ca92796..HEAD -- pmacs-gpu/`
is empty) and adds **no test to that binary**. Occurrence 3 excluded
"the added GPU test is the mechanism" by a control; this occurrence
reproduces the signature with *nothing added to the binary*, which is
independent corroboration rather than a repeat of the same argument.

**The observing lane is environmentally non-neutral, and that is
recorded as a CHANGE rather than as a mechanism.** It moves the gate's
`TMPDIR` off `/tmp`, which on this machine puts every
`tempfile::tempdir()` in the run on btrfs instead of tmpfs. Noted so a
later occurrence can compare like with like.

**A causal claim built on that was advanced here and is WITHDRAWN.**
The draft argued the test's one-second deadline plus a slower
filesystem was a plausible new mechanism. It does not hold on
inspection: **the deadline bounds the connection RETRY loop, not the
socketpair handshake that returned `BrokenPipe`**, and the filesystem
work happens before that deadline is armed. The tempdir is created and
never bound — the failing I/O is on a `UnixStream::pair`. Recording a
mechanism that the code does not support is worse than recording none,
because the next occurrence gets measured against a story instead of
against the evidence.

**So the causal status is unchanged by this occurrence: UNRESOLVED,
with no new mechanism.** What it adds is the corroboration above. Three
isolated re-runs on the current tree were green, which by this file's
own rule establishes intermittence only.

**The discriminating comparison for a fifth occurrence** remains the
one the third occurrence prescribed. One occurrence, with no supported
mechanism, is not grounds to reverse a fix that closes two observed
hazards.

**Third occurrence — worker identity Stage 1 review round 2,
2026-08-09, local (Linux). Same selector, same `gpu`-step flavor, all
three fragments verified** against the durable gate log
(`20260809T172606Z-1387979/11-gpu.log`): `transient sequence must
attach: Attach(Handshake(Io(Os { code: 32, kind: BrokenPipe, message:
"Broken pipe" })))`. A match on this file's own rule, not a `U` note.

**The control the second-occurrence note prescribed was run, and this
time it discriminated — against the hypothesis.** Ten full
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu` runs with the added
`render_offscreen` test present: **10/10 green**. Ten more with that
test `#[ignore]`d, changing nothing else: **1 failure in 10**, carrying
all three required fragments
(`without/run-6.log`, `pmacs-gpu/src/attach.rs:1680`).

So the concurrent-GPU-test path named above is **excluded**: removing
the suspect made the failure *more* frequent, not less, which no
contention story from that test survives. What the run does establish is
that **the failure reproduces on demand at roughly 1-in-10 under
ordinary `-p pmacs-gpu` load** — the first time any rerun in this row's
history has reproduced it at all. That is a materially better starting
point than three isolated sightings, and it is the fact a diagnosis
should be built on: the rate makes a bisect of `attach.rs`'s handshake
path affordable, where before it was not.

**It is still not attributed to the observing lane**, and now for a
measured reason rather than an argument from diff shape: the arm without
the lane's only `pmacs-gpu` addition is the arm that went red.

**What would retire it is unchanged** — the mechanism, not the rate.
The next agent to touch this row should reproduce at 1-in-10 and
instrument which side closes the pipe, rather than re-running for green.

**Fourth occurrence — D3 file-watch scheduler (PR #235), 2026-08-11,
local (Linux), at the gate's SWEEP step** (`cargo test --workspace
--no-fail-fast`, default features — U3's flavor, this time with the
fragments captured). All three required fragments verified against the
durable gate log
(`pmacs-fdccc423/gate-logs/20260811T150651Z-1481359/08-sweep.log`):
`transient sequence must attach: Attach(Handshake(Io(Os { code: 32,
kind: BrokenPipe, message: "Broken pipe" })))`, `attach.rs:1680`.
242/243 in the target; the same sweep had passed twice earlier the same
day on materially the same tree (the diff between runs was a test file
and docs — **no `pmacs-gpu` code, no wire, no protocol**, the
strongest non-attribution shape this row has had). Ambient context,
recorded not asserted: load average ~5.2 and four leaked
`pmacs --daemon` processes resident. Consistent with the established
~1-in-10-under-load rate; adds no new mechanism evidence. The
retirement bar is unchanged.

### U2 — `m6_1_pty_raw_mode_disables_kernel_echo`, THIRD known occurrence

**Corrected 2026-08-09 after review.** A previous edit of this row
called the 2026-08-09 failure the *second* occurrence and claimed it
captured the fragment for the first time. **Both were wrong**, and the
evidence was already in this repository:
`docs/active-work.md` records a **2026-08-06** loaded `--features crdt`
run failing this selector *and* `m6_1_pty_canonical_mode_keeps_kernel_echo`
with the same `stty -a output was: ""`, and it already proposed a
mechanism family — **read-before-write on the child's output**, the
shape of **R4** (readiness predicate satisfied by an empty file) and
**R6** (readiness file never published).

So the fragment was captured before, under another feature flavor, and
this row's earlier "no mechanism has been proposed" was false of the
tree it was written in.

| field | value |
|---|---|
| **selector** | `--lib process::tests::m6_1_pty_raw_mode_disables_kernel_echo` |
| **job / flavor** | local (Linux), during `cargo test --tests --no-fail-fast` — the lib target alongside a full PTY-heavy corpus |
| **required fragments** | `panicked at src/process.rs:3953` · `raw mode should disable echo; stty -a output was: ""` |
| **status** | **at least three occurrences, load-correlated; the diff is EXCLUDED on the 2026-08-09 one** |
| **what IS established** | **Three occurrences.** **(1)** the original: failed once (`1916 passed; 1 failed`) under a full-corpus `--tests --no-fail-fast` run, fragments not captured. **(2) 2026-08-06**, loaded `--features crdt`: this selector **and** `m6_1_pty_canonical_mode_keeps_kernel_echo` both failed with the same `stty -a output was: ""` — the first capture, and the occurrence that proposed the read-before-write family. **(3) 2026-08-09**, worker-identity tip: `1919 passed; 1 failed` in `scripts/gate` step `03-lib` at load ~21, and **the tree contained ZERO code change since a 13/13 green run on the same lane** — the only delta was three lines of `docs/active-work.md`. A markdown edit cannot break a PTY test, so the change under test is ruled out as a cause rather than merely doubted. Passes isolated (`1 passed`, 0.01s). **Occurrence 2 is the one that matters most**: it shows the failure is not confined to one feature flavor and can take both selectors at once |
| **what the fragment ACTUALLY shows** | **The supervisor collected empty stdout** — `drain_until` then `collect_stdout(&evs)` (`src/process.rs:3948-3951`); the assertion inspects that string. It does **NOT** establish that `stty` emitted nothing: the bytes could have been lost in PTY delivery or in event collection. An earlier edit of this row said "`stty` produced no output at all", which asserts a mechanism the test cannot see. What is true is narrower and still useful: this is not a *termios* failure — nothing shows echo being configured wrongly — but which of {child never wrote, PTY dropped it, collection missed it} is open. The assertion's message invites the wrong reading, since it prints an empty string as though it were `stty`'s answer |
| **what is NOT** | **No mechanism is ESTABLISHED** — one is *proposed*: read-before-write on the child's output, the R4/R6 readiness family (occurrence 2). Proposed is not confirmed, and nothing here discriminates it from PTY delivery or event-collection loss. Not reproduced in a later full sweep (108 targets, exit 0), nor in 3 isolated `--lib` runs (1917/0 each), nor in the isolated rerun after occurrence 3. **Three occurrences establish intermittence and a load correlation; none establishes cause** |
| **discriminating control for the next occurrence** | capture the **full process event stream and the child's exit disposition**, not only the collected string — that is what separates "child never wrote" from "delivery or collection lost it", and the collected string cannot distinguish them however many times it is sampled. Cross-check against R4/R6's readiness family, which `docs/active-work.md`'s 2026-08-06 entry already implicates |
| **cross-reference** | `docs/active-work.md` — 2026-08-06 occurrence, `--features crdt`, **both** the raw and canonical selectors, same fragment, read-before-write hypothesis |
| **rival explanation not excluded** | leaked `pmacs --daemon` processes, which the handoff names as a standing confound for any load-sensitive local red |

### U3 — the R7 selector again, fragments lost the same way U2's were

`attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream`
failed once during long-lines Stage 5's default-features sweep and
passed on the recaptured rerun.

**This is not recorded as an R7 match, and the distinction is the
point.** R7's job/flavor is `--features crdt`; this was
default-features. More importantly its three required fragments
(`transient sequence must attach`, `Handshake(Io(`, `BrokenPipe` /
`code: 32`) are **unverified**, because the run's output was filtered to
the failing test names before it was read. By this file's own matching
rule that makes it a new incident, not a recurrence.

| field | value |
|---|---|
| **selector** | `-p pmacs-gpu attach::tests::managed_retry_survives_transients_and_uses_the_successful_stream` |
| **job / flavor** | local (Linux), `cargo test --workspace --no-fail-fast` — **default features**, unlike R7 |
| **required fragments** | **none captured** |
| **status** | **new incident, not reproduced** |
| **what IS established** | one failure; a recaptured rerun of the same command was green for this test, and the `--features crdt` sweep was green for it too. Per the rerun rule: **intermittence only** |
| **what is NOT** | whether it is R7's mechanism. It may well be. Nothing in hand shows it |

**The recurring mistake is mine, and it is now twice.** U2 records the
identical loss — "output was filtered to the `FAILED` line" — and I did
it again here by piping a sweep through `grep`. The fix is mechanical:
**redirect a full sweep to a file and grep the file**, never the live
stream. A signature that is cheap to capture and impossible to
reconstruct should never be traded for terminal brevity.

*(Renumbered from U4/U5 to **U6/U7** on the rebase onto `0857bf4`: `gate-protocol-build` landed its own U4/U5 in #229, and git merged both files **without a conflict**, producing duplicate ids across four sites. The pre-rebase warning is retired here because it has been carried out.)*

### U6 — two wall-clock budget tests fail together in one `lib-crdt` step

Recorded during worker identity Stage 1 review round 2, 2026-08-09, in
the same gate run that produced R7's third occurrence. **Fragments were
captured**, so unlike U1–U3 this one is matchable — it is a `U` row
because it has one occurrence and no mechanism, not because the evidence
was lost.

| field | value |
|---|---|
| **selector** | `--lib --features crdt optimistic::tests::criterion_1_end_of_line_typing_completes_sub_frame_per_keystroke` **and** `editor::tests::composition_overhead_under_ten_percent`, failing in the same run |
| **job / flavor** | local (Linux), `scripts/gate` step `04-lib-crdt`, with sibling worktrees building concurrently |
| **required fragments** | `criterion 1: per-keystroke orchestrator time` + `exceeds 1ms`; and `composition machinery added more than 10% overhead` |
| **status** | **new incident, one occurrence, not reproduced** |
| **what IS established** | both are **wall-clock budget assertions** — 1.264ms against a 1ms budget, and 1.297× against a 1.10× budget — so both are load-sensitive by construction. Both green in an isolated rerun of exactly those two selectors, and both green in the next full gate run of the same command (2105 passed) |
| **what is NOT** | whether the machine's concurrent load caused it. The confound is real (this machine runs one shared `CARGO_TARGET_DIR` and several worktrees) but **was not measured**, so it is a rival explanation, not a finding |
| **rival explanation not excluded** | a genuine regression in either path. Nothing in the observing diff touches the optimistic-echo orchestrator or the composition pipeline, but "my diff looks unrelated" is not evidence, and this row does not treat it as such |

**Two budget tests failing in one run and neither in the next is the
signature worth matching**, more than either name alone: a real
regression in two unrelated subsystems at once is far less likely than
one loaded machine. If a future run reds **one** of these without the
other, that is a different incident and should be judged as one.

### U7 — a *different* wall-clock render-budget test reds each sweep

Recorded during worker identity Stage 1 review round 3, 2026-08-09.
**Two consecutive `scripts/gate` runs of the same command, on the same
tree, red on step `12-sweep` with a different test each time** — which
is the signature, and it is a stronger one than any single selector.

| field | value |
|---|---|
| **selector** | run 1: `--test m8_2_acceptance dired_open_renders_10k_entries_under_200ms` **and** `--test m8_9_acceptance outline_5_level_100_entry_renders_within_100ms`; run 2: `--test dired_acceptance dired_renders_10k_entries_within_200ms` |
| **job / flavor** | local (Linux), `scripts/gate` step `12-sweep` (`cargo test --workspace --no-fail-fast`), **load average 12.9 / 23.9** with sibling worktrees building concurrently |
| **required fragments** | `must render within 200ms; took ` / `open() (parse + render) took ` + `spec budget is 100ms` |
| **status** | **new incident, three selectors, none reproduced** |
| **what IS established** | all three are **wall-clock render-budget assertions** (224ms and 258ms against a 200ms budget; 114ms against a 100ms budget), so all three are load-sensitive by construction. Each was green in an isolated rerun of its own selector, no selector reds twice, and **the third run of the same command on the same tree was green on all 13 steps** (log `20260809T200907Z-2672209`). The observing diff is **two string literals, their doc comments and one test** — it touches no render path at all, and cannot |
| **what is NOT** | that load caused it. The one-shared-`CARGO_TARGET_DIR` confound is real and again **unmeasured**, so it stays a rival explanation rather than a finding |
| **relation to U6** | same shape, different step and different tests: U6 is two budget tests in `04-lib-crdt` failing **together**; this is three render-budget tests in `12-sweep` failing **one per run**. Kept separate rather than merged, because merging would assert a shared mechanism nothing here shows |

**The rotating selector is the thing to match.** A regression that
moved between three unrelated render paths on an unchanged tree is far
less likely than one loaded machine; a future run that reds the *same*
one of these twice is a different incident and should be judged as one.

**The retirements are not occurrences and do not close the log.** R1 and
R3 stay live, and each retired row keeps its signature so a later red
matching one reopens it.

**All four *evidenced* rows (R1–R4) are macOS.** That is a property of
these occurrences, not of the file: **A1's job is `GPU Render
(headless)`, which runs on Ubuntu**, and **A2's job was never
recorded**. Nothing here is macOS-only by construction, and a future
row from any job belongs in the same table.

The #214 occurrence is the strongest available evidence that these are
not caused by the PRs they appeared on — that PR is **docs-only and its
tree is byte-identical to a green `main`**. It is not evidence that any
of them is harmless.

### U4 — `a_pty_resize_blanks_the_host_before_repainting`, macOS **both flavours**, three occurrences

Surfaced on PR #229's CI; twice more on PR #231's.

**The `lua54` in this row's original title was wrong as a signature
component, and matching on it would have missed two occurrences.** The
row was filed from #229's single `lua54` red and recorded the flavour in
the matching key. #231 then reddened the identical selector with the
identical three fragments **twice on `luajit`** — so flavour is not part
of this signature, and the row's own caution that "a deterministic
defect *can* be Lua-flavour-specific" is now settled in the other
direction: this one is not. Occurrence-keyed by suffix length, the three
are `25 362` (#229, `lua54`), `25 222` (#231 attempt 1, `luajit`) and
`25 054` (#231 attempt 2, `luajit`).

**A fourth sighting of these fragments was NOT an occurrence and must
not be counted as one.** It came from a deliberate bite during this
test's own development — the defect reintroduced on purpose (`consumer
ignores full_grid`), 34 831 bytes, failing in 20.09 s. It earns its
place here for what it proves instead: **the genuine defect and these
CI reds are signature-indistinguishable**, same message class and same
full-timeout duration, so the fragments alone can never tell a real
resync failure from whatever this is.

| field | value |
|---|---|
| **selector** | `--test full_grid_resync_acceptance a_pty_resize_blanks_the_host_before_repainting` |
| **job / flavor** | GitHub Actions, `Test (macos-latest / lua54)` **and** `Test (macos-latest / luajit)`, `macos-26-arm64`. **Flavour is not a matching key for this row** |
| **required fragments** | `FG-INV: the post-resize resync must blank the host` · `no CSI 2 J appeared in the` · `bytes emitted after the first painted frame` |
| **NOT fragments** | the byte count and the `:LINE` suffix are **occurrence-specific** and must not be matched on — the count is the collected suffix length, which varies per run, and the line moves with the file |
| **status** | **three occurrences on two branches; INTERMITTENT on #229 (passed on rerun), NOT observed to pass on #231 (0/2)** |
| **the #231 control experiment, and what it does and does not license** | Five valid observations at #231's exact base `0190102` — `run_attempt` 1, 2, 3, 4 and 6 — **all green on both macOS flavours**, against #231's 0/2. Under an equal-rate model the chance both failures land on the two branch runs is 1/C(7,2) = **4.8%**. Two things bound that number. First, **attempt 5 was discarded** because it reddened a *different* selector (U8) — so the base leg is 5/5 green *for this signature* and 5/6 overall, and "the base never fails" is not what was observed. Second, three unrelated macOS selectors reddening in one session is **a background platform failure rate**, and the equal-rate model the 4.8% assumes is exactly what such a rate violates. **The branch side was never resampled**: 5-vs-2 is an asymmetric experiment, and rerunning #231's failing job three more times at `4654b94` was the outstanding discriminator when it merged |
| **why #231's diff is excluded** | grepping its **entire** `src/` diff for `full_grid\|resize\|resync\|Geometry\|reconcile_panel_layout` matches **one import line** and nothing else; all 721 changed lines are placement, dedication and commit-contract logic. From the other side, `full_grid_resync_acceptance` (191 lines) contains no panel, side-window, dedication, display or directory surface — grep for those matches only a comment about CSI 2 J. #231 merged on this reading **over** the statistical signal above, which is a judgement recorded here so that a fourth occurrence can revisit it rather than re-derive it |
| **why #229's diff is excluded** | #229 changes only `scripts/gate`, `tests/gate_script_acceptance.rs` and documentation — **no `src/`, and the workflow never invokes `scripts/gate`**. Decisively, `full_grid_resync_acceptance` runs **before** the changed gate suite, so even a cross-suite leaked-state path is not available. The `luajit` leg passing on the same commit is **corroboration only** — a deterministic defect *can* be Lua-flavour-specific, so that observation must not be used as a structural exclusion |
| **what IS established** | **no blank was OBSERVED after the mark** within the test's fixed 20-second deadline. The collected suffix was the **entire** post-mark output (`suffix.len()`, 25 362 bytes on this occurrence — not a capped window; only the *displayed* head is truncated to 400 bytes), and that head shows ordinary repaint traffic (`ZQXMARKERQZ` rows with SGR + CUP), so the host was painting |
| **what is NOT** | any mechanism. Whether the blank was never emitted, emitted after the deadline, or lost in transport is **open** — and "it never emitted the blank" is a claim this evidence does not support. **The failing run's ~20 s duration is the fixed `Duration::from_secs(20)` timeout**, so the spread against a fast passing run is mechanically determined and is **not** independent timing evidence |
| **discriminating control — ASYMMETRIC, and only one direction concludes** | the suffix is already complete, so "capture more bytes" is not the gap — arrival time is. Extending the deadline and recording whether `CLEAR_ALL` arrives, and at what offset: **if it arrives, "emitted late" is established.** **If it does not, that establishes only "not observed by the longer deadline"** — *not* "never emitted", because transport loss produces the same absence. Separating non-emission from transport loss needs **producer-side emission evidence** (did pmacs write the clear?) cross-checked against the collected stream; no deadline, however long, can do it alone |

### U5 — `ctrl_c_during_reconnect_sleep_yields_clean_exit`, macOS `lua54`, one occurrence

Surfaced on the **rerun** of PR #229's failed job — a *different*
selector from U4, so by this file's matching rule it is a **new
incident, not U4 occurring twice**.

| field | value |
|---|---|
| **selector** | `--test m5_8_acceptance ctrl_c_during_reconnect_sleep_yields_clean_exit` |
| **job / flavor** | GitHub Actions, `Test (macos-latest / lua54)`, rerun attempt 2 |
| **required fragments** | `Ctrl-C during reconnect sleep should produce a clean exit` · `ExitStatus { code: 1, signal: Some("Interrupt: 2") }` |
| **NOT a fragment** | the `:LINE` suffix — occurrence-specific, moves with the file |
| **status** | **one occurrence, unresolved** |
| **what IS established** | Ctrl-C reached the process **as `SIGINT`** rather than as the raw-mode key event the test drives. That is all the exit status shows |
| **what is NOT** | whether injection preceded raw mode, raw mode was lost, or something else. Three mechanisms remain open and this fragment separates none of them |
| **exclusion strength — WEAKER than U4's, deliberately** | the changed `gate_script_acceptance` ran **earlier in the same job**, and it creates worktrees and directories. No leaked child or persistent signal-state mutation was observed, but "the diff touches no `src/`" is **not** the argument here that it is for U4, because cross-suite leaked state is a path reachability reasoning does not close |
| **control 1 — CROSS-SUITE ATTRIBUTION, and asymmetric** | run `m5_8_acceptance` alone on macOS `lua54`, without the gate suite ahead of it. **A matching isolated RED proves the gate suite is not necessary** for the failure. **An isolated GREEN proves nothing beyond that run** — the failure is intermittent, so absence under one run is not evidence of dependence. It also does **not** discriminate among the three mechanisms in either direction |
| **control 2 — mechanism** | observe **readiness and raw-mode state at the moment of injection**. Another isolated pass, however many times repeated, cannot separate "injected before raw mode" from "raw mode lost" from a third cause |

### U8 — `acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel`, macOS `luajit`, one occurrence, **fragments destroyed**

**Numbered U8 deliberately: U6 and U7 are reserved** for the two
wall-clock rows on `worker-identity-stage1` (PR #232), which renumbered
into that range when #229 took U4/U5. Taking U6 here would recreate the
duplicate-id collision that rebase already produced once.

**This row exists mostly as an admission.** It surfaced on attempt 5 of
a merge-base control at `0190102`, and **I reran the job before reading
its log**, which discarded it. GitHub keeps only the latest attempt's
logs for a rerun job. So this is U2's original condition exactly — a
selector with no fragments, unmatchable — and it was produced by the
very mistake U3 is named for.

| field | value |
|---|---|
| **selector** | `--test bottom_panel_stage1_acceptance acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel` |
| **job / flavor** | GitHub Actions, `Test (macos-latest / luajit)`, at base `0190102`, control attempt 5 |
| **required fragments** | **NONE CAPTURED — destroyed by rerunning the job before reading its log.** Recovery attempted via the jobs API and the attempt-scoped jobs endpoint; the log is gone |
| **what IS established** | it failed once (`46 passed; 1 failed`), panicking at `tests/bottom_panel_stage1_acceptance.rs:2454`, on the **exact merge base** — so it is not attributable to any open branch |
| **what is NOT** | everything else. Without the assertion text this cannot be matched against a future occurrence, which is the whole purpose of a row here |
| **why it matters anyway** | it is the **third distinct macOS selector** to red in one session, after U4 (`full_grid_resync`) and U5 (`ctrl_c_during_reconnect`). Three unrelated selectors failing on the macOS legs suggests a **background failure rate on that platform** rather than three independent test bugs — and that materially affects any equal-rate reasoning about which branch a failure "landed on" |
| **next occurrence** | **read the log BEFORE rerunning anything.** That is U3's stated lesson and this row is its fourth violation |

### U9 — a PTY test and a budget test red **together** in one `11-sweep`, with an in-run control

Recorded on the `destination-capture` merge tree, 2026-08-10, in the
gate run that was meant to clear PR #231.

**This row's value is its control, not its selectors.** U6 and U7 could
only compare a red run against a *different* run. Here both selectors
ran green **inside the same gate invocation**, minutes earlier, on the
same tree and machine — `03-lib` (1928 passed, 0 failed) and
`04-lib-crdt` (2113 passed, 0 failed) — and then failed in `11-sweep`.
So this is **not deterministic on this tree; causation and any rate
effect are unresolved.**

*The original wording here was "whatever this is, it is not the tree",
which this file's own rerun rule forbids: a same-tree green establishes
**intermittence only**, and a tree can raise an intermittent failure
**rate** without making it deterministic. Same-tree greens cannot
exonerate the tree. Corrected rather than deleted, because the wrong
claim is the one a later reader would otherwise reach for.*

| field | value |
|---|---|
| **selector** | `--lib process::tests::m6_1_pty_canonical_mode_keeps_kernel_echo` **and** `editor::tests::composition_overhead_under_ten_percent`, failing in the same `11-sweep` step |
| **job / flavor** | local (Linux), `scripts/gate` step `11-sweep` (`cargo test --workspace --no-fail-fast -- --skip basedpyright`), fresh per-lane target dir, no sibling worktrees building |
| **required fragments** | ``canonical mode should leave echo enabled (no `-echo` flag); stty -a output was: ""`` **and** `composition machinery added more than 10% overhead` |
| **NOT fragments** | the measured numbers (`1.613`, `single=191935 ns`, `dispatch=309602 ns`) and every `:LINE` suffix — occurrence-specific |
| **status** | **one occurrence; INTERMITTENT — the identical sweep command on the same tree was green (118 targets, 1928 passed, exit 0)** |
| **what IS established** | intermittence, with the strongest available exclusion of the tree: green in two earlier steps of the **same run**, green isolated afterwards (`2 passed`, 1.70 s), green on a full sweep rerun. Both assertions are **timing-sensitive by construction** — one reads collected child output within a deadline, the other measures wall-clock composition overhead (observed 1.613× against a 1.10× budget; 61.3% dispatch and 124.6% realistic overhead) |
| **what is NOT** | cause, and the load confound is **partially measured but NOT controlled**. The failing sweep ran inside a full gate; the green rerun started at load average 1.98 with the 5-minute figure still at 8.03 from that gate. Different conditions is not a measurement of the mechanism, and this row does not treat it as one |
| **the structural difference worth testing next** | `cargo test --workspace` runs **many test binaries concurrently**; `--lib` runs **one**. That is a difference in kind between the passing steps and the failing one, not merely a difference in load average — and it is the first candidate this family has had that is checkable rather than atmospheric. **Discriminating control:** rerun the sweep with test-binary concurrency pinned to 1, and separately run the `--lib` binary alone under synthetic load. A red under synthetic load at low sweep concurrency implicates load; a red at high concurrency and low load implicates the concurrency itself |
| **relation to U2 — a NEAR MISS, do not match it there** | the PTY fragment is U2's exact family (`stty -a output was: ""`), but U2's selector field names only `m6_1_pty_raw_mode_disables_kernel_echo`. U2's occurrence 2 saw raw **and** canonical fail together; here **canonical redded alone and raw passed**, which U2's evidence has never shown. It is recorded here rather than folded into U2 so that the "canonical alone" case stays visible |
| **relation to U6 — its own instruction, honoured** | `composition_overhead_under_ten_percent` is one of U6's two selectors, and U6 says plainly: "If a future run reds **one** of these without the other, that is a different incident and should be judged as one." It redded without `criterion_1_end_of_line_typing…`, in a different step, at a far larger margin (1.613× here against U6's 1.297×). Judged as a different incident, as instructed |
| **what this row does NOT assert** | that the two selectors share a mechanism. They failed together once; they belong to different subsystems; and U7 already refused this exact merge for U6. The **co-failure inside one step with an in-run green control** is the signature — not either name, and not a shared cause |

### U10 — the budget red ROTATES between two consecutive runs of one commit

Recorded during §5b review round 4, 2026-08-20. **Two consecutive
`scripts/gate` runs at the same commit with a clean worktree verified
at both ends of each run** — `70b334d`, `git status --porcelain` empty
before and after, both times. Each run was 15/16 green. Each red is a
**wall-clock budget assertion in a different step**, and **each is
green in the other run**.

| field | value |
| --- | --- |
| **run A** | log `20260820T155616Z-359755`, step `13-sweep` red: `dired_open_renders_10k_entries_under_200ms`, **263.961465ms against 200ms** (32% over). Step `15-sweep-crdt` **green** |
| **run B** | log `20260820T160806Z-578046`, step `15-sweep-crdt` red: `optimistic::tests::criterion_1_end_of_line_typing_completes_sub_frame_per_keystroke`, **1.044609ms against 1ms** (4.5% over), 2148 passed. Step `13-sweep` **green** |
| **required fragments** | `M8.2 spec: 10K entries must render within 200ms; took ` / `criterion 1: per-keystroke orchestrator time` + `exceeds 1ms` |
| **status** | **two occurrences, neither reproduced; every selector green on rerun** |
| **isolated controls** | both green in an isolated rerun of their own selector at load average 9.34, through the gate's target directory — the same control shape U6 and U7 each used |
| **what IS established** | **the tree is excluded, as strongly as this repository can exclude it.** Not "the diff touches no render path" — the **same commit** produced a pass and a fail of each row, with the worktree verified clean at both ends of both runs. Neither failing path is touched by the branch under test (`src/optimistic.rs`, `tests/m8_2_acceptance.rs` and the dired paths are all absent from `git diff --name-only githubsucks/main...HEAD`) |
| **what is NOT** | **that load caused it.** Load was **not sampled during either failing step**. A 76.63 reading exists for run A but was taken later in the same run, while step 15 was compiling; run B began at 23.46. Neither figure measures the failing moment, and this row does not pretend otherwise |
| **rival CLOSED since U7** | the shared `CARGO_TARGET_DIR` confound U7 left "real and again unmeasured". Each worktree now gets its own gate target directory (`pmacs-mapping-gen-8cb089c8`); no sibling shared it. Excluded **for these occurrences only** — it says nothing about U7's |
| **machine context, NOT a cause** | 150 leaked `pmacs` daemons were live throughout, 1.9 GB resident, oldest ~6.3 days — the standing "Leaked daemons — NEEDS A LANE" item. Their instantaneous CPU sampled at ~0%. Recorded because it is true of the machine, **not** because anything here shows it mattered |
| **relation to U7 — its escalation rule, honoured and CUT BOTH WAYS** | U7 says "a future run that reds the **same** one of these twice is a different incident and should be judged as one." Run A redded `dired_open_renders_10k_entries_under_200ms`, which was U7's run-1 selector, so that selector has now redded twice, 11 days apart — filed here rather than appended to U7, as instructed. **But within this pair the selector ROTATED**, which is U7's own core signature, and run B's selector is U6's. The repeat and the rotation are both true, and this row asserts neither as the finding |
| **relation to U6** | run B's selector is one of U6's two, redding **without** `composition_overhead_under_ten_percent`. U6 instructs that one-without-the-other is a different incident; honoured here |
| **what this row does NOT assert** | a shared mechanism between the two rows, or any mechanism at all. **The signature is the rotation across an identical commit** — not either name |

**Why this family keeps recurring, stated plainly.** Every row in it is
a wall-clock budget asserted **inside a workspace-wide parallel test
run**. `cargo test --workspace` starts many test binaries at once, so
each budget competes with the rest of the sweep in **every** run,
including the ones that pass. A 4.5% overshoot on a 1ms budget is not a
signal about the code. **U9 already named the discriminating control**
— pin test-binary concurrency to 1 and separately load a lone `--lib`
binary — and it remains unrun. Until it runs, this family should not
consume another review round.

**Widening a budget is not the fix**, and R1 already rejected it.

### U11 — `dispatch_parse_round_trips_a_rust_source_file`, macOS `lua54`, one occurrence, green on rerun

Recorded from PR #242, 2026-08-20, on head `61f0faf` — the head that
merged as `47b5463`. **Deferred to post-merge absorption by decision**,
so that no docs commit would invalidate that PR's head-exact gate
evidence; the fragments lived in
[PR #242's comment](https://github.com/levineuwirth/pmacs/pull/242#issuecomment-5359360750)
in the interim, because the raw job log is machine-local and **not
portable evidence**.

| field | value |
| --- | --- |
| **selector** | `--lib async_runtime::tests::dispatch_parse_round_trips_a_rust_source_file` |
| **job / flavor** | `Test (macos-latest / lua54)`, run `32393462318` |
| **required fragment** | `trivial parse should be fast`, panicking at `src/async_runtime.rs:3328` |
| **counts** | `1960 passed; 1 failed; 3 ignored`, finished in 64.49s |
| **attempt 1** | job `96504773333` — **failure** |
| **attempt 2, rerun** | job `96511228345` — **success** |
| **status** | **one occurrence, not reproduced** |
| **THE MARGIN IS UNRECOVERABLE** | the assertion is `assert!(duration_ms < 100, "trivial parse should be fast")` — a 100ms wall-clock budget on a 30-byte source, **with `duration_ms` omitted from the message**. The red cannot say whether it missed by 1ms or by 900ms, and **no future occurrence can be compared against this one**. This is a property of the assertion, not of the observation |
| **what IS established** | **non-determinism across these observations, and nothing more.** Four passes of the same selector on the exact head in local gate `20260820T163107Z-879828` — one each in `03-lib`, `04-lib-crdt`, `13-sweep`, `15-sweep-crdt`. Two `Test (macos-latest / luajit)` greens on the same commit and OS, jobs `96504773331` and `96511275402`. `src/async_runtime.rs` **byte-identical to `main`**, blob `9310ce3fca8c5fd8ebd39a68c29ad6985e256049` on both sides — not merely an empty diff |
| **what is NOT** | environmental cause, or harmlessness. The luajit greens narrow the red below "macOS at this commit"; they do not explain it. **A green rerun establishes intermittence only** |
| **relation to U10** | **not U10.** Different selector, different fragment, different subsystem, and remote rather than local. Filed separately for the same reason U10 was filed apart from U6 and U7 |

**A recurrence is not another instance of this row.** Because the margin
was never captured, a second red cannot be compared with the first, so
it owes a **merge-base control** before any claim is made about the
tree.

**The one-line fix that would make this family diagnosable** — include
the measured `duration_ms` in the assertion message — is **owed its own
small lane**. It was deliberately kept out of #242, whose diff does not
touch `src/async_runtime.rs`.

### U12 — U9's shape in `04-lib-crdt`: a budget test and a PTY test, together

Recorded during the panel-replay lane's gate, 2026-08-21, on head
`6142acc`. **Filed rather than folded into U6 or U9**, because both of
those tell it to be: U6 says one of its selectors redding without the
other is a different incident, and this is `composition_overhead_under_ten_percent`
alone again; U9 is that same shape but in `11-sweep` with a different
PTY selector.

| field | value |
| --- | --- |
| **selectors** | `--lib --features crdt editor::tests::composition_overhead_under_ten_percent` **and** `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`, failing in the same step |
| **job / step** | local (Linux), `scripts/gate` step `04-lib-crdt`, log `20260821T122852Z-2922631` |
| **required fragments** | `composition machinery added more than 10% overhead` / `live runtime probe` |
| **observed** | **1.247×** against the 1.10× budget (single 192493 ns, dispatch 240130 ns); the PTY row failed at `src/process.rs:5155`, where `active_reader_probe` found no live reader within the 2s `Started` window |
| **status** | **one occurrence, both selectors green on isolated rerun** |
| **the rest of the run** | **15 of 16 stages green**, including `sweep`, `m4`, `gpu`, `diff-check` and all eight touched acceptance suites |
| **what IS established** | both are timing-dependent by construction — one a wall-clock ratio, the other a 2-second liveness window — and each passed alone immediately afterwards. **`src/process.rs` is NOT touched by the observing branch at all**; `src/editor.rs` is, but only in the panel-replay paths, not in composition |
| **what is NOT** | that load caused it. Load was **11.04 at the gate's start and 27.79 (5-minute) at its end**, with two foreign `python` processes at ~2 cores throughout and an `apt`/`dpkg` install shortly before. Those are conditions, not a measurement of the mechanism, and the run was **knowingly taken on a machine that was quieter but not quiet** |
| **relation to U6** | its selector, alone again, in U6's own step. U6's instruction to judge that separately is honoured for the second time — see U9, which did the same |
| **relation to U9** | the same budget-plus-PTY co-failure, in `04-lib-crdt` rather than `11-sweep`, with `setsid_escapee…` where U9 had `m6_1_pty_raw_mode…` |

**This family has now produced U6, U9, U10 and U12, and the
discriminating control U9 named is STILL UNRUN**: pin test-binary
concurrency to 1, and separately load a lone `--lib` binary. Four
incidents is enough evidence that the family will keep costing review
rounds until someone runs it.
