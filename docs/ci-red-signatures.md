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

**Four of the six evidenced rows are live.** R2 and R4 were retired on
2026-08-05 and are below, under "Retired rows", with their dispositions.
R5 and R6 were added on 2026-08-06 from an occurrence scan and a live
red; neither is diagnosed.

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
