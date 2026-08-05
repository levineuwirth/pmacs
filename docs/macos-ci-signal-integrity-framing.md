# Framing — macOS CI signal integrity: a signature registry, then hardening

**Revision 3.** Status: **Stage 1 implemented** on
`macos-ci-signal-integrity`, PR #215. Scouted against
`githubsucks/main` @ `bfb97c6`.

**Revision 2 → 3** exists because the implementation discovered a state
the contract did not allow, and the contract — not the implementation —
was what needed changing. Acceptance 3 offered a binary: carry an
incumbent in with a signature and evidence, or remove it as never
substantiated. **Both incumbents are neither**, and shipping a third
state while the governing criterion still said "two" would have made the
framing describe something the branch does not do.

Revision 3 also **retires the phrase "mechanism named"**, which
overstated what the audit found. The a33 audit proves an assertion
string and a historical claim exist; the m6_8 audit proves a test is
timing-based.
**Neither establishes a failure mechanism** — no occurrence was ever
**linked or captured**, so nothing is known about how either fails.
(Not "never observed": someone may well have seen one and not recorded
it. What is established is the absence of a *record*, which is the only
thing this audit can speak to.) They become
**audit notes A1/A2**, not registry rows, and `R`-numbers are reserved
for signatures with linked evidence.

Revision 2
separates a machine-matchable signature from verbatim, variable CI
output; preserves historical incident evidence while centralizing live
triage policy; gives retirement a causal rule rather than an arbitrary
green-run count; and corrects the rerun rule.

Four red CI incidents across #213 and #214 were each correctly judged
"not caused by this PR" — and #214's case is airtight, because it is
docs-only and its tree is byte-identical to a green `main`. **That
proves the PRs did not cause them. It does not prove they are harmless
environmental noise, and three of the four have a specific, findable
mechanism.**

This lane separates those two claims, which the current process
conflates.

---

## 0. Coherence impact (COHERENCE §20)

- **Journey steps touched:** none.
- **Interaction islands:** none.
- **Config registry adoption:** none.
- **Background-work attribution:** none.
- **Why it belongs on the board:** it protects §19's acceptance-test
  ratchet and every arc that reads a red run. A flake list that is
  wrong, incomplete, or keyed by test *name* rather than *signature*
  makes "rerun before concluding" a habit rather than a judgement — and
  a real regression arriving in that stream is indistinguishable from
  the noise it hides in.

---

## 1. Ground truth (measured at `bfb97c6`)

### 1.1 The accounting: FOUR incidents, THREE tests, FOUR signatures

The registry must count **signatures**, not test names. The process test
alone produced two, with different mechanisms and different causal
status — collapsing them under one name would have hidden a possible
product defect behind a known-flaky label.

The registry key is an exact **test selector + job/flavor + normalized
match rule**, not a pasted panic block. PIDs, elapsed times, and rendered
OS-error suffixes vary between runs; each row therefore names the
required invariant fragments and the evidence link preserves the
verbatim occurrence. A row matches only when every listed requirement
is satisfied; where a requirement lists alternative platform renderings,
one of those alternatives suffices. A test-name match by itself never
matches a registry entry.

| # | exact test selector | job / flavor | required signature fragments | causal status |
|---|---|---|---|---|
| 1 | `--lib async_runtime::tests::supersede_cancels_in_flight_job_within_50ms` | macOS / luajit | `supersede did not cancel within 50ms` | **measurement design** — see §1.2 |
| 2 | `--lib process::tests::a_successful_signal_disposition_depends_on_whether_it_is_fatal` | macOS / lua54 | `leader=exited(signal SIGUSR1)` | **test race** — see §1.3 |
| 3 | *(same test)* | macOS / lua54 | all of `EPERM`, `measured_group=unobservable(` + `ESRCH` / `No such process`, and `leader=live` | **UNRESOLVED — possible product defect** — see §1.4 |
| 4 | `--test vterm_stage2_acceptance terminal_escape_gates_local_bindings_and_double_escape_sends_interrupt` | macOS / luajit | both `left: []` and `right: [49]` | **test race** — see §1.5 |

Evidence:
[#213 run 30826884642](https://github.com/levineuwirth/pmacs/actions/runs/30826884642),
[#213 run 30927084982 attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30927084982/attempts/1),
[#214 run 30932558752 attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30932558752/attempts/1).

### 1.2 Signature 1 — the 50ms budget measures more than it claims

```rust
let first = rt.dispatch_sleep(2_000, Some("search"));
// Let the worker pick the job up so cancel hits a running job.
thread::sleep(Duration::from_millis(15));
let started = Instant::now();
let _second = rt.dispatch_sleep(2_000, Some("search"));
while !rt.is_complete(first) {
    assert!(started.elapsed() < Duration::from_millis(50), "supersede did not cancel within 50ms");
    let _ = rt.tick();
    thread::sleep(Duration::from_millis(1));
}
```

**Two design problems, and neither is fixed by a bigger number.**

- **The premise is a sleep.** `thread::sleep(15ms)` is asserted-by-comment
  to mean "the worker picked the job up". On a loaded runner it may not
  have, in which case the test measures cancellation of a *queued* job —
  a different code path, already covered by the sibling test — while
  claiming to measure a running one.
- **The clock includes the observer.** `started` begins before the
  second dispatch, and the budget is consumed by the test's own
  `tick()` + `sleep(1ms)` pump loop. Under scheduling pressure the
  measured interval is dominated by when *the test* got scheduled, not
  by when the worker observed the cancel flag.

**Widening 50ms to 200ms would make it pass and measure nothing more.**
The open question is what this test should assert: a latency bound needs
a clock the observer does not participate in, or the assertion should be
reformulated as ordering ("the first settles Cancelled before the second
completes") rather than duration.

### 1.3 Signature 2 — `Started` does not prove the trap is installed

```rust
spec.args = vec!["-c".into(), "trap '' USR1; sleep 30".into()];
let id = sup.spawn(spec).expect("spawn");
let pid = spawn_started_pid(&mut sup, id);   // waits for ProcessEventKind::Started
sup.signal(id, Signal::SIGUSR1).expect("USR1 delivers");
```

`spawn_started_pid` waits for `ProcessEventKind::Started { pid }`, which
is emitted when the process is **spawned** — not when `/bin/sh` has
parsed and installed `trap '' USR1`. **SIGUSR1's default disposition is
terminate**, so a signal delivered inside that window kills the child,
and the record is `exited(signal SIGUSR1)` instead of `Running`.

The fixture's own comment states the requirement it does not enforce:
*"Ignore USR1 so the successful non-fatal signal cannot end the child
and confuse the state assertion with a real exit."* That is exactly the
confusion observed.

This is the handoff's **"wait predicate weaker than the assertion"**
race: the predicate is "process exists", the assertion needs "trap
installed".

### 1.4 Signature 3 — the live-leader EPERM, deliberately unresolved

`EPERM, measured_group=unobservable(ESRCH), leader=live` is **not
established as a test race**. It is the group-target behaviour the
process-signal lanes have circled three times:

- #176 established that a group-directed `kill` returned EPERM while the
  leader was observed alive by a real `try_wait`, retiring "EPERM cannot
  happen for our own children" as a reason to discard the errno.
- #200 added `measured_group` from `getpgid`, the only field able to
  disagree — and it is reported here as `unobservable(ESRCH)`, meaning
  the group could not be measured at all.
- The reap-ledger lane parked every disposition change pending exactly
  this evidence.

**This lane does not resolve it and must not appear to.** Its registry
entry carries causal status **UNRESOLVED — possible product defect**,
and its retirement condition is a diagnosis, not a green rerun. Folding
it under the same "macOS signal timing" label as signature 2 is how a
real defect acquires a flake's immunity.

### 1.5 Signature 4 — a readiness predicate satisfied by an empty file

```rust
fn wait_for_file(path: &Path, timeout: Duration) -> Vec<u8> {
    loop {
        if let Ok(bytes) = fs::read(path) { return bytes; }   // succeeds on 0 bytes
        ...
    }
}
```

The probe writes readiness with `open(path,'wb').write(b'1')`. **`open()`
creates the file before `write()` fills it**, so `fs::read` can succeed
on a zero-byte file and `wait_for_file` returns `[]`. The caller then
asserts `== b"1"` and fails `left: [], right: [49]`.

The predicate is "readable"; the assertion is "contains `1`". This is
the same shape as §1.3 and is fixable at the helper. All four callers of
this helper require concrete non-empty content; the similar bottom-panel
helper already rejects empty reads.

### 1.6 Live triage policy and historical evidence are mixed together

Flake language currently appears in the handoff's two operational
rules, `docs/active-work.md`, and several landed framing documents. It
is not all duplication. The process-signal and reap-ledger framings, for
example, preserve exact historical occurrences and the reasoning those
lanes built from them; replacing that evidence with a pointer would
make a durable framing depend on a mutable registry.

The real duplication is **live classification and triage policy**. The
handoff's hazards list names three tests — two of which are not among
the four incidents seen here — while three of these four are absent.
Elsewhere, historical occurrence notes, forward-looking risk warnings,
and current "known flaky" claims are written in the same voice even
though they require different treatment.

A list that is both stale and incomplete is worse than none: it confers
"known flaky" on whatever happens to be named, and withholds it from
everything else.

### 1.7 What is NOT established

- **No incident has been reproduced locally.** All four are macOS-only
  and this machine is Linux. The mechanisms in §§1.2–1.5 are read from
  source and from CI signatures, not from a local repro.
- **Frequency is unmeasured.** Four incidents across two PRs is not a
  rate. The registry records occurrences so a rate can accumulate;
  it does not claim one now.
- **Signature 3's cause remains unknown**, by design (§1.4).

---

## 2. Questions

- **Q#MCI1 — where does the registry live? DECIDED:** one dedicated
  `docs/ci-red-signatures.md`, with the handoff's operational rule
  pointing at it. The handoff is a briefing; the registry is an
  occurrence ledger whose rows remain available after retirement.
- **Q#MCI2 — what retires an entry? DECIDED:** a mechanism-specific
  causal result, not N green runs. A known-race entry requires hardening
  that removes the named mechanism plus a discriminating acceptance
  witness for the stronger predicate. A measurement-design entry
  requires its owning lane to replace or justify the measurement and
  pin the resulting claim. An unresolved entry requires diagnosis and
  an explicit disposition. Main-job greens remain occurrence evidence,
  but cannot retire any row by themselves; retired rows remain in the
  history with their disposition.
- **Q#MCI3 — does signature 1 get reformulated or re-measured?** §1.2
  argues its budget measures the observer. Reformulating as an ordering
  assertion changes what the test proves; keeping a duration means
  finding a clock the pump loop does not participate in. **This is the
  one question this lane should not answer alone** — it is the
  async-runtime lane's design call.
- **Q#MCI4 — does hardening ship before or with the registry? DECIDED:**
  the user has already answered: **registry now, hardening next.**
  Recorded here so the sequencing is visible in the document rather
  than only in the conversation.

---

## 3. Bets

- **Bet 1 — signatures 2 and 4 disappear under hardening**, because both
  have a named mechanism and a fix at the readiness predicate.
  *Falsified if either recurs after the predicate is strengthened.*
- **Bet 2 — a signature-keyed registry refuses name-based immunity.** A
  future failure in the process test which does not contain all of row
  2's or row 3's required fragments is a new incident, not a known
  flake. *Falsified if the operational rule permits a test-name-only
  match.*
- **Bet 3 — green reruns do not erase unresolved evidence.** Signature
  3 remains unresolved until the process-signal lane diagnoses and
  disposes it, whether or not later runs pass. *Falsified if a green run
  changes that row's status or retirement condition.*

---

## 4. Acceptance

**Stage 1 — the registry (this lane's first PR):**

1. **One authoritative table**, with a row per **signature**: exact test
   selector, job and Lua flavor, normalized machine-match rule, evidence
   link to the verbatim occurrence, causal status, and mechanism-specific
   retirement condition. Matching requires the selector, job/flavor,
   and every required fragment; variable values are explicitly
   normalized rather than silently abbreviated.
2. **One live triage policy.** Operational duplicate classifications and
   rerun rules become pointers to the registry. Historical occurrence
   evidence stays where it supports a landed framing; a relevant note
   may gain a registry status link, but its evidence and reasoning are
   not replaced. Forward-looking risk statements are audited as risks,
   not silently promoted to known flakes.
3. **The three tests named in the current handoff list are audited**,
   into one of **three** states — the third was found by doing the audit
   and is why this is revision 3:
   - **carried as a registry row** (`R`-numbered) when a signature and a
     linked occurrence both exist;
   - **removed** when nothing substantiates it at all;
   - **recorded as an audit note** (`A`-numbered) when a *historical
     claim* exists but **no occurrence was ever linked**. An audit note
     is not a registry row, cannot be matched against a red run, and
     confers nothing.

   **No entry survives on reputation, and an audit note is not a weaker
   row — it is a different kind of statement.** A row says "this was
   seen, here is the evidence"; a note says "someone recorded a belief
   and no occurrence backs it."
4. **The rerun rule is replaced**, not softened:
   - a **green rerun after a red** establishes intermittence only; it
     does not establish environmental cause, harmlessness, or retirement;
   - the **same signature on the rerun** is a second occurrence and
     remains blocking pending investigation or a merge-base control;
   - a **different signature** is a new incident and is judged
     independently. A known test name confers no immunity.
5. Signature 3 is recorded **UNRESOLVED — possible product defect**, and
   its retirement condition is a diagnosis, never a green rerun.

**Stage 2 — hardening (a separate PR):**

6. The vterm Stage 2 `wait_for_file` requires the **expected content**
   (preferred) or at minimum a non-empty result, so the predicate
   matches the assertion. Its four callers all require concrete
   non-empty content; the bottom-panel helper already rejects empty
   reads and is not evidence for widening this change further.
7. The USR1 fixture proves the **trap is installed**, not merely that
   the process started. The child publishes readiness after installing
   the trap, and the test waits on that.
8. Signature 1 is **not** fixed by widening the budget (Q#MCI3).
9. Both hardened tests run **repeatedly** (a repetition set, as the
   reap-ledger lane did) rather than once, because a single green run of
   a formerly intermittent test proves nothing.

**Quarantine — only if hardening fails:**

10. A quarantined test moves to a **separate, still-blocking CI step**.
    Never `#[ignore]`, never `continue-on-error`, never a silent
    retry-to-green. A quarantine that stops failing the build is a
    deletion with extra steps.

---

## 5. Parked

- **Resolving signature 3.** It belongs to the process-signal /
  reap-ledger lanes, which have already parked three tolerance rules
  pending this class of evidence.
- **The `--no-fail-fast` gap and the crdt job's PTY deadlines**, both
  recorded in the CI CRDT lane. Related in spirit, separate in scope.
- **A general flake-rate dashboard.** The registry accumulates
  occurrences; turning that into a rate with alerting is its own thing.
- **Linux and GPU flakes.** `a33_headless_terminal_frame_paints_...`
  and `m6_8_supervisor_reaps_...` were audited under acceptance 3 and
  became **audit notes A1/A2** — claims with no linked occurrence.
  **A1's job runs on Ubuntu**, so the registry is not macOS-only even
  now; A2's job was never recorded. This lane's four *evidenced*
  incidents are macOS, which is a fact about them and not about the
  file.

---

## 6. Gates

The standing `CLAUDE.md` suite. Stage 1 is documentation-only and adds
no test; **its verification is acceptance 3** — the audit that no entry
survives on reputation. Stage 2 adds the repetition sets of acceptance 9.

---

## 7. Branch plan

Two PRs, in this order:

1. **`macos-ci-signal-integrity`** — the registry, the pointer
   rewrites, the audit, and the rerun rule. No code.
2. **A hardening PR** — the `wait_for_file` predicate and the USR1 trap
   readiness, each with a repetition set. Signature 1 is referred to the
   async-runtime design question rather than patched.

Quarantine, if it happens, is a third and is scoped by what hardening
fails to fix.
