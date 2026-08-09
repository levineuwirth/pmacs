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
| **status** | **THIRD OCCURRENCE 2026-08-09 — causal status still UNRESOLVED, but one candidate mechanism is now EXCLUDED** |
| **what IS established** | **three** occurrences at `pmacs-gpu/src/attach.rs:1680`, the second and third with all three fragments **verified** rather than inferred; the test drives a scripted transient-then-success sequence over a real socket pair. **The added GPU test is not the mechanism** — see the third-occurrence control below |
| **what is NOT** | whether the broken pipe is the *fixture's* writer closing early or a real retry-path defect. **This row is not a claim that it is harmless** |
| **rerun evidence** | occurrence 1: 6 isolated runs green, plus a full `--workspace --features crdt` sweep green (113 targets). Occurrence 2: **30 green on the observing branch** (15 isolated selector, 15 full `-p pmacs-gpu`) **plus a 15-run merge-base control, also green**. Occurrence 3: 5 isolated selector runs green, 10 full `-p pmacs-gpu` runs green **with** the added test, and **1 failure in 10 with the added test `#[ignore]`d** — the first rerun in this row's history that reproduced anything. Per the rerun rule the green runs establish intermittence only; the red control run is what carries the exclusion |
| **retirement** | hardening that removes the named mechanism plus a discriminating witness — or a diagnosis showing the fixture, not the code, closes the pipe |

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

### U2 — `m6_1_pty_raw_mode_disables_kernel_echo`, one local occurrence

Has a selector, which U1 lacks — but still no fragments, so it cannot
be matched either. Recorded so a recurrence is recognisable.

| field | value |
|---|---|
| **selector** | `--lib process::tests::m6_1_pty_raw_mode_disables_kernel_echo` |
| **job / flavor** | local (Linux), during `cargo test --tests --no-fail-fast` — the lib target alongside a full PTY-heavy corpus |
| **required fragments** | **none captured** — output was filtered to the `FAILED` line |
| **status** | **new incident, unreproduced** |
| **what IS established** | it failed once (`1916 passed; 1 failed`), in no registry row, under a full-corpus run |
| **what is NOT** | any mechanism. Not reproduced in a later full `--tests --no-fail-fast` sweep (108 targets, exit 0) nor in 3 isolated `--lib` runs (1917/0 each) |
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

### U4 — two wall-clock budget tests fail together in one `lib-crdt` step

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

### U5 — a *different* wall-clock render-budget test reds each sweep

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
| **what IS established** | all three are **wall-clock render-budget assertions** (224ms and 258ms against a 200ms budget; 114ms against a 100ms budget), so all three are load-sensitive by construction. Each was green in an isolated rerun of its own selector, and no selector reds twice. The observing diff is **two string literals, their doc comments and one test** — it touches no render path at all, and cannot |
| **what is NOT** | that load caused it. The one-shared-`CARGO_TARGET_DIR` confound is real and again **unmeasured**, so it stays a rival explanation rather than a finding |
| **relation to U4** | same shape, different step and different tests: U4 is two budget tests in `04-lib-crdt` failing **together**; this is three render-budget tests in `12-sweep` failing **one per run**. Kept separate rather than merged, because merging would assert a shared mechanism nothing here shows |

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

### U4 — `a_pty_resize_blanks_the_host_before_repainting`, macOS `lua54`, one occurrence

Surfaced on PR #229's CI.

| field | value |
|---|---|
| **selector** | `--test full_grid_resync_acceptance a_pty_resize_blanks_the_host_before_repainting` |
| **job / flavor** | GitHub Actions, `Test (macos-latest / lua54)`, `macos-26-arm64` |
| **required fragments** | `FG-INV: the post-resize resync must blank the host` · `no CSI 2 J appeared in the` · `bytes emitted after the first painted frame` |
| **NOT fragments** | the byte count and the `:LINE` suffix are **occurrence-specific** and must not be matched on — the count is the collected suffix length, which varies per run, and the line moves with the file |
| **status** | **one occurrence; INTERMITTENT — passed on rerun** |
| **why the diff is excluded** | #229 changes only `scripts/gate`, `tests/gate_script_acceptance.rs` and documentation — **no `src/`, and the workflow never invokes `scripts/gate`**. Decisively, `full_grid_resync_acceptance` runs **before** the changed gate suite, so even a cross-suite leaked-state path is not available. The `luajit` leg passing on the same commit is **corroboration only** — a deterministic defect *can* be Lua-flavour-specific, so that observation must not be used as a structural exclusion |
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
