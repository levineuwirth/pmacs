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
| **mechanism named, no occurrence** | a linked occurrence promotes it to one of the above; sustained absence is not retirement, because nothing was ever measured |

Main-branch greens are **occurrence evidence** and accumulate toward a
rate. They retire nothing by themselves. Retired rows stay in this file
with their disposition, so a recurrence is recognisable.

---

## Live rows

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

### R2 — USR1 delivered before the trap is installed

| field | value |
|---|---|
| **selector** | `--lib process::tests::a_successful_signal_disposition_depends_on_whether_it_is_fatal` |
| **job / flavor** | macOS / lua54 |
| **required fragments** | `leader=exited(` + `SIGUSR1` |
| **causal status** | **test race** |
| **evidence** | [#213 run 30927084982 attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30927084982/attempts/1) |
| **retirement** | the fixture proves the trap is installed, with a witness that fails without it |

Readiness is `ProcessEventKind::Started`, emitted at **spawn** — not when
`/bin/sh` has parsed `trap '' USR1`. SIGUSR1's default disposition is
terminate, so a signal inside that window kills the child. The fixture's
own comment states the requirement it does not enforce.

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

### R4 — readiness predicate satisfied by an empty file

| field | value |
|---|---|
| **selector** | `--test vterm_stage2_acceptance terminal_escape_gates_local_bindings_and_double_escape_sends_interrupt` |
| **job / flavor** | macOS / luajit |
| **required fragments** | `left: []` **and** `right: [49]` |
| **causal status** | **test race** |
| **evidence** | [#214 run 30932558752 attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30932558752/attempts/1) |
| **retirement** | `wait_for_file` requires the expected content, with a witness that fails against a zero-byte file |

`wait_for_file` returns as soon as `fs::read` succeeds — which succeeds
on a **zero-byte file**. The probe writes readiness with
`open(path,'wb').write(b'1')`, and `open()` creates the file before
`write()` fills it. The predicate is "readable"; the assertion is
"contains `1`" (`49` is ASCII `'1'`).

---

## Audited incumbents — mechanism named, no occurrence recorded

These were carried in the handoff's hazards list **without evidence**.
The audit found the tests real and the mechanisms plausible, so they are
neither confirmed nor deleted: they are recorded honestly as unmeasured.

**Nothing here confers "known flaky".** A red matching one of these is a
first recorded occurrence and should be investigated, not reruns-to-green.

### R5 — GPU terminal cell background did not paint

| field | value |
|---|---|
| **selector** | `-p pmacs-gpu a33_headless_terminal_frame_paints_cells_without_document_layers` |
| **job / flavor** | GPU Render (headless), under parallel load |
| **required fragments** | `the terminal cell background did not paint` + `blue pixels` |
| **causal status** | **mechanism named, no occurrence recorded** |
| **evidence** | **none linked.** The assertion string is real (`pmacs-gpu/src/main.rs:17973`); no run was ever cited |
| **retirement** | a linked occurrence promotes this row; absence retires nothing, because nothing was measured |

### R6 — supervisor reap across cycles

| field | value |
|---|---|
| **selector** | `--test m6_8_multi_repl_acceptance m6_8_supervisor_reaps_all_children_across_cycles` |
| **job / flavor** | not recorded |
| **required fragments** | **not recorded** — no signature was ever captured |
| **causal status** | **mechanism named, no occurrence recorded** ("timing-based", 10 cycles) |
| **evidence** | **none linked** |
| **retirement** | a linked occurrence with a signature promotes this row |

**R6 cannot currently be matched.** Without required fragments there is
no rule to apply, so a red in this test is a new incident by default.
That is the correct outcome for an entry that never carried evidence.

---

## Occurrence log

| date | run | row | outcome |
|---|---|---|---|
| 2026-08-04 | [30826884642](https://github.com/levineuwirth/pmacs/actions/runs/30826884642) | R1 | rerun green — intermittence only |
| 2026-08-04 | [30927084982 att.1](https://github.com/levineuwirth/pmacs/actions/runs/30927084982/attempts/1) | R2 | rerun green — intermittence only |
| 2026-08-04 | [30932558752 att.1](https://github.com/levineuwirth/pmacs/actions/runs/30932558752/attempts/1) | R3, R4 | rerun green — intermittence only; **R3 remains unresolved** |

Four incidents, three tests, **four signatures**. Count signatures: the
process test contributed two, and only one of them is a test bug.

The #214 occurrence is the strongest available evidence that these are
not caused by the PRs they appeared on — that PR is **docs-only and its
tree is byte-identical to a green `main`**. It is not evidence that any
of them is harmless.
