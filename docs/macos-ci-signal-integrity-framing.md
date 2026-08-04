# Framing — macOS CI signal integrity: a signature registry, then hardening

**Revision 1.** Status: framing only. No branch work beyond this
document. Scouted against `githubsucks/main` @ `bfb97c6`.

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

| # | test | job / flavor | exact signature | causal status |
|---|---|---|---|---|
| 1 | `async_runtime::tests::supersede_cancels_in_flight_job_within_50ms` | macOS / luajit | `supersede did not cancel within 50ms` | **measurement design** — see §1.2 |
| 2 | `process::tests::a_successful_signal_disposition_depends_on_whether_it_is_fatal` | macOS / lua54 | `leader=exited(signal SIGUSR1)` | **test race** — see §1.3 |
| 3 | *(same test)* | macOS / lua54 | `EPERM, measured_group=unobservable(ESRCH), leader=live` | **UNRESOLVED — possible product defect** — see §1.4 |
| 4 | `vterm_stage2::terminal_escape_gates_local_bindings_and_double_escape_sends_interrupt` | macOS / luajit | `left: [], right: [49]` | **test race** — see §1.5 |

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

`EPERM, measured_group=unobservable(ESRCH), leader=live` is **not** a
test race. It is the group-target behaviour the process-signal lanes
have circled three times:

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
the same shape as §1.3 and is fixable at the helper — every caller
inherits the fix.

### 1.6 The prose is duplicated and keyed by name

Flake claims currently live in at least six places: the handoff's
hazards list, `docs/active-work.md` (twice), and the reap-ledger,
process-signal, vterm and terminal-config framings. They disagree in
detail, none carries an exact signature or an evidence link, and the
handoff's list names three tests — **two of which are not among the
four incidents seen here**, while three of these four are absent from
it.

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

- **Q#MS1 — where does the registry live?** It must be one file, and
  every other mention becomes a pointer. `docs/agent-handoff.md` §5 is
  the natural home (it already holds the hazards list), but a dedicated
  `docs/ci-flake-registry.md` is easier to keep in one voice and to diff.
  **Leaning: a dedicated file, with the handoff pointing at it**, since
  the handoff is a briefing and this is a table that will grow.
- **Q#MS2 — what retires an entry?** Proposal: hardening that removes
  the mechanism, plus N consecutive green runs of that job on `main`.
  N needs a number, and the number is a judgement about how much
  evidence "gone" requires.
- **Q#MS3 — does signature 1 get reformulated or re-measured?** §1.2
  argues its budget measures the observer. Reformulating as an ordering
  assertion changes what the test proves; keeping a duration means
  finding a clock the pump loop does not participate in. **This is the
  one question this lane should not answer alone** — it is the
  async-runtime lane's design call.
- **Q#MS4 — does hardening ship before or with the registry?** The user
  has already answered: **registry now, hardening next.** Recorded here
  so the sequencing is visible in the document rather than only in the
  conversation.

---

## 3. Bets

- **Bet 1 — signatures 2 and 4 disappear under hardening**, because both
  have a named mechanism and a fix at the readiness predicate.
  *Falsified if either recurs after the predicate is strengthened.*
- **Bet 2 — signature 1 does not**, because widening a budget that
  measures the observer changes nothing about what it measures.
- **Bet 3 — signature 3 recurs and stays unexplained** until the
  process-signal lane resolves the group-target question. The registry's
  job is to keep it visible, not to fix it.

---

## 4. Acceptance

**Stage 1 — the registry (this lane's first PR):**

1. **One authoritative table**, with a row per **signature**: exact test
   path, job and Lua flavor, **exact signature text**, evidence link,
   causal status, and retirement condition.
2. **Every duplicate mention becomes a pointer.** The handoff hazards
   list, both `active-work.md` mentions, and the four framing docs cite
   the registry rather than restating a claim.
3. **The three tests named in the current handoff list are audited**:
   each is either carried into the registry with a signature and
   evidence, or removed with a note saying it was never substantiated.
   No entry survives on reputation.
4. **The rerun rule is replaced**, not softened:
   - one rerun that reproduces **the same signature** is evidence of
     **intermittence only**;
   - a **different signature**, or **the same signature twice
     consecutively**, requires investigation or a merge-base control
     before the red is attributed to the environment.
5. Signature 3 is recorded **UNRESOLVED — possible product defect**, and
   its retirement condition is a diagnosis, never a green rerun.

**Stage 2 — hardening (a separate PR):**

6. `wait_for_file` requires a **non-empty** result — or better, the
   expected content — so the predicate matches the assertion. Every
   caller inherits it.
7. The USR1 fixture proves the **trap is installed**, not merely that
   the process started. The child publishes readiness after installing
   the trap, and the test waits on that.
8. Signature 1 is **not** fixed by widening the budget (Q#MS3).
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
- **Linux and GPU flakes.** `a33_headless_terminal_frame_paints_...` and
  `m6_8_supervisor_reaps_...` are in the current handoff list and get
  audited under acceptance 3, but this lane's incidents are macOS.

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
