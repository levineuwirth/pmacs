# Framing — make the signal diagnostic discriminating (evidence collection)

**Revision 2.** Status: awaiting review round 2. Lane:
`process-signal-diagnostic-completeness`, worktree
`../pmacs-signal-identity`, based on `githubsucks/main` @ `391d38a`.

**This is Stage B of the lane whose Stage A merged as PR #176**
(`docs/process-signal-tolerance-framing.md`, revision 4). Stage A made a
failing `kill` self-describing and parked every tolerance rule behind one
condition: evidence. Evidence has arrived (§1.2). It does not support any
of the parked rules, and — per review round 1 — it does not support an
identity claim either.

**Evidence collection only. No tolerance rule, no change to which
process gets signalled, no disposition change.** Everything behavioural
is parked in §5.

## Revision history

**Revision 1 → 2**, after review round 1 (two blocking, two major). All
four accepted; all four verified against the code before acceptance.

- **Rev 1 proposed measuring the pgid and then *targeting* it.** That is
  a behavioural change resting on an identity claim a numeric pgid cannot
  support (§1.5). Retargeting is parked (§5); this lane only *records*.
- **Rev 1 repeated the defect it was written to fix.** Its Bet 1 asserted
  `getpgid(child) == pid`, which an implementation that ignored `getpgid`
  and returned `pid` would satisfy — the same non-discrimination the
  review found in the landed acceptance (§1.4). Every measurement in this
  revision now requires a case where the two values **differ**.
- **Rev 1 scoped the PTY path out.** Review showed the PTY fallback is
  indistinguishable from a normal pipe child in the report (§1.6), which
  is a defect in the diagnostic this lane owns. Scope now includes it,
  and the rev 1 sentence "this lane does not touch the PTY path" is
  withdrawn.
- **Rev 1 never noticed the report omits the signal** (§1.7).

**Rev 1 also mis-stated where the lane lived.** It was written to a
session scratchpad, which is not portable and was not on `githubsucks`,
so review round 1 necessarily landed on Stage A's revision 4 instead.
That is why round 1's line references point at the merged document. The
findings apply regardless — three of the four are defects in code that is
on `main` right now — but the process error is recorded here because
"work is portable only after it is committed and pushed" is a standing
project rule and this lane broke it on its first step.


## 0. Coherence impact (COHERENCE §20)

- **Journey step 8, "Open a terminal"** (§2), teardown half, plus every
  compile/grep run through `spec.group`. **No grade change and no
  behavioural change.**
- **Serves §9 (worker model), failure attribution.** Stage A made the
  failure describe itself; this lane makes the description
  *discriminating* — today several distinct failures render identically.
- **Interaction islands: none. Config registry: not adopted.
  Background-work attribution: unchanged.**
- **No audited claim in COHERENCE.md changes**, so under §25 no
  COHERENCE edit rides this PR.


## 1. Ground truth (verified at `391d38a`)

### 1.1 Stage A landed and has now fired

`signal_failure_report` and `LeaderObservation` merged as **PR #176 on
2026-07-26** (`62316a9`). The occurrence below is the first failure
carrying the new format rather than a bare errno. Stage A is the reason
this document can exist.

### 1.2 The new occurrence, verbatim

PR #191, `Test (macos-latest / lua54)`,
[run 30553376486](https://github.com/levineuwirth/pmacs/actions/runs/30553376486/job/90907461258),
`process::tests::repeated_terminate_does_not_extend_ledger_deadline`.
1873 passed, 1 failed. **The rerun of the identical head passed 12/12**,
so the failure is intermittent, not deterministic:

```
re-terminate: "kill: EPERM: Operation not permitted
  (target=-8619 via group, leader_pid=8619, expected_group=-8619, leader=live)"
```

Established: the target source is `group` (the `spec.group` pipe path,
`signal_target` `:774-780`; `sh_group_spec` `:3402` sets it), **not** the
PTY path; and `leader=live`, from a real `try_wait` against the real
child, so the leader had not exited and had not been reaped.

### 1.3 Two written premises are falsified

- **`src/process.rs:1246-1247`** — `tick_reap_ledger` justifies treating
  any probe error as "nothing left we can reach" with the comment
  "**EPERM cannot happen for our own children**". §1.2 is a
  counterexample: our own child, alive, EPERM.
- **Stage A §1.3** — "the reap ledger is disjoint from this path", on the
  grounds that the ledger arms only for `proc.spec.group` and PTY mode
  cannot set it. §1.2's process **is** a `spec.group` process. Not
  disjoint.

Stage A's §1.2 entity-split analysis concerns the PTY path, where the
target is read from `tcgetpgrp`. **It does not apply to §1.2's
occurrence**, where the target is computed as `-leader_pid` with no read.

### 1.4 The landed acceptance cannot discriminate (round 1, P1)

`a_group_directed_kill_failure_reports_target_and_leader_separately`
(`:2400`) spawns a PTY child and asserts the exact string

```
target=-{pid} via tcgetpgrp, leader_pid={pid}, expected_group=-{pid}, leader=live
```

— the same `pid` three times. **An implementation that ignored
`tcgetpgrp` entirely and substituted `leader_pid` would pass.** The test's
own doc comment concedes it: "here they are asserted to agree only
because nothing has moved the terminal."

The premise of the whole diagnostic is that these two entities can
diverge, and no test exercises a case where they do. This is the
vacuous-assertion family already recorded in the handoff.

### 1.5 A numeric pgid cannot establish identity (round 1, P1)

Rev 1 proposed reading the real pgid and targeting it. Review is right
that this does not establish identity: the value is read before `kill`,
the read-then-kill window remains, and a *number* cannot distinguish the
original group from a recycled one.

**There is no portable mechanism that closes this.** `pidfd_open` +
`pidfd_send_signal` close pid reuse for a single *process* on Linux;
there is no process-*group* equivalent, and macOS has no pidfd at all.
Since the failures are macOS-only so far, no available mechanism makes
group signalling identity-safe.

**Therefore this lane records and does not retarget.** Acceptance
criteria state what was observed, never that the observation is
sufficient.

One narrowing fact, stated because it constrains the hypothesis space
and *not* as an identity claim: POSIX does not free a child's pid until
the parent reaps it, and §1.2 observed `leader=live` from a `try_wait`
that had not reaped. While that pid is held, no other process can be
assigned it, so no *new* group can be created bearing that pgid value
during the window. This narrows recycling as a candidate **for that one
occurrence**; it says nothing about whether the group still contained a
signallable member, which is the question EPERM actually turns on.

### 1.6 The PTY fallback is invisible in the report (round 1, P2)

`signal_target` (`:757-785`): when the PTY branch's
`master.process_group_leader()` returns `None`, control falls through —
`spec.group` is rejected at spawn for PTY mode — and returns
`TargetSource::LeaderPid`, rendered "leader-pid" (`:738`). **A normal
pipe child renders identically.** Two different situations, one string.

`portable-pty`'s implementation
(`portable-pty-0.9.0/src/unix.rs:374`) is:

```rust
fn process_group_leader(&self) -> Option<libc::pid_t> {
    match unsafe { libc::tcgetpgrp(self.fd.0.as_raw_fd()) } {
        pid if pid > 0 => Some(pid),
        _ => None,
    }
}
```

The errno is discarded, so "the tty has no foreground group" and
"`tcgetpgrp` failed" are already merged before pmacs sees the result.

**`nix::unistd::tcgetpgrp` returns `Result<Pid>`** (`nix-0.29.0/src/unistd.rs:368`,
ungated, and `pub mod unistd` at `lib.rs:183` is unconditional), so pmacs
can make this call itself and keep the errno **without `unsafe`** — which
matters because the crate is `#![forbid(unsafe_code)]`.

### 1.7 The report omits which signal failed (round 1, P2)

`signal_failure_report` (`:830-850`) takes target, leader pid, errno and
leader observation. **Not the signal.** `signal` (`:1074`) has it.

This is not cosmetic: `signal` transitions state and arms the reap ledger
only for `SIGTERM | SIGKILL | SIGHUP` (`:1099-1116`), and the public Lua
surface accepts INT, USR1, USR2 and QUIT as well (`src/lua_bindings/mod.rs:8627-8629`).
A failed `SIGUSR1` and a failed `SIGTERM` have different consequences and
currently produce indistinguishable text.

### 1.8 The disposition consequence, worse for the first call

`signal` returns `Err` **before** the state transition and **before**
arming the ledger:

| Which `terminate` hits EPERM | Consequence |
|---|---|
| A **later** one (§1.2's case) | Caller sees `Err`; state already `Exiting`, ledger already armed, so SIGKILL escalation still happens. |
| The **first** one | State stays `Running`, ledger never armed. **No escalation is ever scheduled** — the child is abandoned. |

Only the recoverable variant has been observed. This lane **pins** the
first-call variant and changes nothing about it (§5).

### 1.9 Limits of the evidence

- **Not reproduced locally.** Development is Linux; the failures are
  macOS-only. No claim in this lane rests on a local repro of the EPERM.
- **Frequency: two occurrences, in different paths** — PR #172 was the
  PTY path (`acc28`, luajit), §1.2 the group path (lua54). This is not
  one flaky test.
- **The mechanism is not established**, and this lane does not propose
  one. That is the point of the split.


## 2. Questions

- **Q#DC1** — Can the two entities be made to diverge in a test? *Yes:
  under a PTY with job control enabled, a shell places a background job
  in its own process group and hands it the terminal, so `tcgetpgrp` !=
  leader pid. §3 Bet 1 builds exactly that.*
- **Q#DC2** — Should the PTY fallback get its own `TargetSource`?
  *Proposed: yes, and pmacs should call `nix::unistd::tcgetpgrp` itself so
  the fallback can report the errno rather than inheriting portable-pty's
  discarded `None`.*
- **Q#DC3** — Should the report name the signal? *Proposed: yes, with a
  contrasting non-fatal signal tested.*
- **Q#DC4** — Should the measured pgid be reported for `spec.group`
  children? *Proposed: yes, as an observation clearly distinct from the
  assumed value, and with no claim of sufficiency (§1.5).*
- **Q#DC5** — Should anything be retargeted or tolerated? **No. Parked.**


## 3. Bets

Each bet names what falsifies it and what falsification teaches.

- **Bet 1 — the divergence is constructible.** A PTY fixture where the
  foreground group is *not* the leader: job control on, a background job
  given the terminal. The rewritten acceptance asserts both exact values
  and that they **differ**.
  - *Falsified if* the fixture cannot be made deterministic in CI (shell
    job-control timing). Then the lane says so and falls back to pinning
    divergence at the `signal_target` unit level with an injected
    foreground group, which is weaker and must be labelled as weaker.
  - This is the finding that matters most: without it, the entire
    diagnostic remains unverified in the only case it exists for.

- **Bet 2 — the PTY fallback is reachable and distinguishable.** A test
  drives the branch where the foreground-group lookup fails and asserts a
  source string distinct from a pipe child's.
  - *Falsified if* the branch cannot be reached without faking the
    lookup. Then the seam is made injectable exactly as Stage A made the
    kill injectable (Q#PD4), and that is stated rather than hidden.

- **Bet 3 — naming the signal is free.** Thread `signal` into the report.
  - *Falsified if* any existing exact-string test cannot be updated
    mechanically. Those four sites (`:2408`, `:2435`, `:2485`, `:2517`)
    are the highest-risk part of the diff: **a wholesale rewrite of
    expected strings is how a format regression hides**, so each is
    updated individually and listed in the PR body with before and after.

- **Bet 4 — the measured pgid can disagree with the assumed one, and the
  test proves the measurement is real.** A child that calls `setsid`, so
  its pgid is genuinely not its parent-assumed value, is measured and the
  two values asserted **different**.
  - *Falsified if* no such case can be built — in which case the
    measurement is unfalsifiable and should not ship, exactly per §1.4's
    lesson.

- **Bet 5 — §1.8's first-call abandonment is real.** Inject EPERM on the
  *first* terminate; assert no ledger entry and state still `Running`.
  - *Falsified if* the ledger is armed anyway, meaning §1.8 misreads
    `signal`.
  - **Pins current behaviour; does not fix it.**


## 4. Acceptance

1. A PTY job-control fixture in which `tcgetpgrp` != leader pid, with
   both exact values asserted and asserted to differ. The landed test at
   `:2400` is **rewritten**, not supplemented, since it currently pins a
   substitution as acceptable.
2. The PTY foreground-lookup fallback reports a source distinct from a
   pipe child's leader-pid, with a test driving the real branch.
3. The report names the signal; at least one non-fatal signal
   (`SIGUSR1`) is tested alongside `SIGTERM`, including that its
   disposition differs.
4. For `spec.group` children the report carries the measured pgid as a
   field distinct from the assumed one, renderable as unobservable, and
   a test asserts a case where they **differ**.
5. Every exact-string test updated individually, each listed in the PR
   body with before and after. No blanket rewrite.
6. §1.8's first-call abandonment pinned, labelled as pinning a known gap.
7. `docs/agent-handoff.md` records that "EPERM cannot happen for our own
   children" is false, with the run link; the comment at `:1246` is
   corrected in the same PR.
8. **No acceptance claims the telemetry establishes group identity.**
   §1.5 governs; the PR body repeats it.


## 5. Parked

- **Retargeting to the measured pgid.** Behavioural, and unsupported by
  §1.5. Needs this lane's evidence first.
- **Any tolerance rule for EPERM or ESRCH.** Unmotivated across Stage A's
  three revisions and still unmotivated.
- **§1.8's first-call abandonment.** Its own lane; disposition change.
- **Q#PS6** — `terminate` on an already-reaped process returning `Ok`.
- **`signal_target`'s read-then-kill of `tcgetpgrp`** — Stage A called it
  "the most likely real fix site, still unframed". Still is. This lane
  makes it *observable*, not fixed.
- **`compile_mode_acceptance` reading the developer's real
  `~/.config/pmacs/init.lua`** — separate defect (11 local failures,
  invisible in CI), unrelated to signals.


## 6. Gates

Standard suite, each its own step with a real exit status, nothing after
the command that could mask it: `cargo fmt --check`; `cargo clippy
--workspace --all-targets -- -D warnings`; `cargo test --lib`; `cargo
test --lib --features crdt`; `compile_mode_acceptance`;
`terminal_copy_mode_acceptance` (both feature configurations);
`bottom_panel_stage1_acceptance`; `cargo test --test m4_acceptance --
--skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`;
`git diff --check`.

**All local runs use an isolated `XDG_CONFIG_HOME`** — without it
`compile_mode_acceptance` fails 11 tests for reasons unrelated to this
lane.

**PTY job-control tests are the load-sensitive kind.** They are run
repeatedly, and the PR body records the repetition count rather than a
single green.


## 7. Branch plan

One branch, one PR. Order: Bet 1 first and alone, because it is the
finding that decides whether the diagnostic is worth extending at all. If
the divergence fixture cannot be made deterministic, the rest of the lane
is re-scoped rather than pushed through.
