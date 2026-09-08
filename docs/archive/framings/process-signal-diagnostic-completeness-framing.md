# Framing — make the signal diagnostic discriminating (evidence collection)

**Revision 6.** Status: implemented; PR open, review round 4 closed. Lane:
`process-signal-diagnostic-completeness`, worktree
`../pmacs-signal-identity`, based on `githubsucks/main` @ `4cd4a7b`
(re-measure at branch time; this is a reading, not a constant).

**This is Stage B of the lane whose Stage A merged as PR #176**
(`docs/archive/framings/process-signal-tolerance-framing.md`, revision 4). Stage A made a
failing `kill` self-describing and parked every tolerance rule behind
evidence. Evidence arrived (§1.2). It supports none of the parked rules,
no identity claim, and — per review round 2 — no claim that today's
escalation path is safe.

**Evidence collection only. No tolerance rule, no change to which process
gets signalled, no disposition change.** Everything behavioural is in §5.

## Revision history

**Revision 5 → 6**, after review round 4 (three blocking, one major).
All four accepted.

- **`measured_group` was sampled AFTER the failed `kill`** and after
  `observe_leader`, while both the framing and the function's own doc
  said before. A concurrent group change would have made the diagnostic
  report post-failure state as evidence about the attempted target. It
  is now sampled in `signal` before the kill and passed into the report.
- **The Linux corroboration did not exercise the production lookup.**
  Its helper read `portable_pty::process_group_leader` — the accessor
  this lane stopped using — so `pty_foreground_group` could have fallen
  back on every call with every test still green. Demonstrated: forcing
  it to always fall back leaves the injected pin **passing** and only
  the corroboration failing. The helper now calls the production lookup,
  and the corroboration forces *only* the kill so the report is built
  from a real terminal read.
- **This document did not update its own acceptance contract** (§4.1).
  Revision 5 recorded the falsification in the revision history and Bet
  1 but left the normative criterion demanding the real-shell rewrite —
  the exact "implementation quietly diverges from the contract" shape
  this project recorded as a lesson on #191/#188.
- **`TargetSource`'s doc had the wrong classification.** Two of four
  variants now target the leader pid, not one, and the pid-versus-group
  split does not line up with PTY-versus-pipe — which is *why*
  `PtyForegroundFallback` needed its own variant.

**Revision 4 → 5**, after implementation. **Bet 1 was falsified by CI**,
and the framing's own fallback is what shipped.

- **`bash -m` does not diverge on macOS.** Both macOS legs reported
  `job control never moved the terminal off the leader (leader=8542,
  foreground groups observed: [8542])` — the terminal stayed with the
  leader for the entire 10s bounded wait. It diverges reliably on Linux
  (20/20 locally, green on both ubuntu legs), so this is a platform
  difference, not a flake, and rerunning would have been wrong.
- **The stated fallback was taken**: the divergent case is now pinned by
  **injecting** the foreground group at the `signal_target` seam, and
  §3 Bet 1 records it as *weaker* than a real shell rather than
  quietly equivalent. Verified still discriminating — the
  `leader_pid`-substitution mutation fails it (`target=-1707909` against
  an expected `-1707910`).
- **The real fixture is retained as corroboration**, Linux-only, under
  `job_control_really_diverges_the_foreground_group`. It is skipped on
  macOS by platform check rather than by arming, because the
  precondition genuinely does not hold there; running it would assert a
  false claim about macOS instead of finding a bug.
- **`PMACS_REQUIRE_BASH` moves to Linux-only.** Rev 4's reasoning for
  arming it on both platforms — "macOS is where the failures happen, so
  arming it Linux-only leaves it dark where it matters" — was correct
  about the *diagnostic* and wrong about *this test*, which cannot
  produce its precondition on macOS at all. The diagnostic's macOS
  coverage comes from the injected pin, which runs everywhere.

**Revision 3 → 4**, after review round 3 (three blocking, one major).
All four accepted and checked against the exact APIs, process model, and
branch ancestry before revision.

- **Rev 3's "no safe fd bridge" conclusion was still too absolute**
  (§1.6). `filedescriptor::OwnedHandle::dup` accepts an `AsRawFd` through
  its safe `AsRawFileDescriptor` blanket implementation and returns an
  owned value implementing `AsFd`. A lifetime-tied wrapper around
  `MasterPty::as_raw_fd` therefore bridges to
  `nix::unistd::tcgetpgrp` with no `unsafe` in pmacs. The crate is
  already resolved through `portable-pty`; this lane declares it
  directly and restores errno capture.
- **The occurrence did not prove "our own child, alive, EPERM"** (§1.3).
  The failed target was a *group* and `try_wait` observed the leader
  process. No measurement established that the leader still belonged to
  that group. What is invalidated is using ownership of the spawned
  child to dismiss an arbitrary group-target error.
- **Bet 1 called a terminal-owning job "background"** (§3). A background
  group is, by definition, not the terminal's foreground group. The
  fixture now names `/bin/bash`, launches a foreground job in its own
  group, and waits for the actual terminal handoff before measuring.
- **The branch-base line described the scout, not the ancestry.** Rev 3's
  merge-base with `4cd4a7b` was still `391d38a`. Canonical main is now
  integrated, and the lane is recorded in `docs/active-work.md`.

**Revision 2 → 3**, after review round 2 (three blocking, three major).
All six accepted; all six verified in the code before acceptance.

- **Rev 3 concluded that the PTY errno proposal had no safe fd bridge.**
  It therefore withdrew and reduced rev 2's central new proposal (§1.6).
  Revision 4 supersedes that conclusion after checking
  `filedescriptor`'s safe duplication API.
- **Rev 2 said `getpgid` was "ungated". It is not** (§1.5a). The claim
  came from reading the four lines above the function; the gate is a
  block-level `feature!` opened 168 lines earlier. Same error shape as
  the truncated-output trap already in the handoff, committed inside a
  document about non-discriminating evidence.
- **Bet 4's `setsid` fixture was impossible** (§3, Bet 4). A `spec.group`
  child is already a process-group leader, and a group leader's `setsid`
  fails with EPERM.
- **"Recoverable" was unsupported** (§1.8). The ledger drops its entry on
  *any* probe error — including an EPERM that ownership of the recorded
  child cannot rule out for a group target — and discards the `SIGKILL`
  result while marking the entry killed.
- **Rev 2 falsified the wrong Stage A sentence** (§1.3). Stage A's
  disjointness claim was about the **PTY** path and remains true.
- **Rev 2's signal-disposition argument was wrong** (§1.7). Failed
  signals are all disposition-identical.
- **Bet 5 proposed a test that already exists** (§1.9). Cited as ground
  truth now, not invented.

**Revision 1 → 2**, after review round 1 (two blocking, two major); all
accepted. Rev 1 proposed *retargeting* to a measured pgid — a behavioural
change resting on an identity claim a number cannot support; it asserted
`getpgid(child) == pid`, which an implementation ignoring `getpgid` would
satisfy; it scoped the PTY path out; and it never noticed the report
omits the signal.

**Rev 1 was written to a session scratchpad rather than a branch**, so it
was never on `githubsucks` and review round 1 necessarily landed on
Stage A's merged document. Recorded because "work is portable only after
it is committed and pushed" is a standing rule this lane broke on its
first step.


## 0. Coherence impact (COHERENCE §20)

- **Journey step 8, "Open a terminal"** (§2), teardown half, plus every
  compile/grep run through `spec.group`. **No grade change, no
  behavioural change.**
- **Serves §9 (worker model), failure attribution.** Stage A made the
  failure describe itself; this lane makes the description
  *discriminating*, because several distinct failures render identically
  today.
- **Interaction islands: none. Config registry: not adopted.
  Background-work attribution: unchanged.**
- **No audited claim in COHERENCE.md changes**; under §25 no COHERENCE
  edit rides this PR.


## 1. Ground truth (verified at `4cd4a7b`)

### 1.1 Stage A landed and has now fired

`signal_failure_report` and `LeaderObservation` merged as **PR #176 on
2026-07-26** (`62316a9`). §1.2 is the first failure carrying the new
format rather than a bare errno. Stage A is why this document can exist.

### 1.2 The new occurrence, verbatim

PR #191, `Test (macos-latest / lua54)`,
[run 30553376486](https://github.com/levineuwirth/pmacs/actions/runs/30553376486/job/90907461258),
`process::tests::repeated_terminate_does_not_extend_ledger_deadline`.
1873 passed, 1 failed. **A rerun of the identical head passed 12/12**, so
the failure is intermittent, not deterministic:

```
re-terminate: "kill: EPERM: Operation not permitted
  (target=-8619 via group, leader_pid=8619, expected_group=-8619, leader=live)"
```

Established: the target source is `group` — the `spec.group` pipe path
(`signal_target` `:774-780`; `sh_group_spec` `:3402`) — **not** the PTY
path; and `leader=live`, from a real `try_wait` against the real child,
so the leader had neither exited nor been reaped.

### 1.3 What the occurrence actually invalidates

- **`src/process.rs:1246-1247` uses an invalid premise.**
  `tick_reap_ledger` justifies treating any probe error as "nothing left
  we can reach" with the comment "**EPERM cannot happen for our own
  children**". But the operation is group-directed: ownership of the
  spawned child says nothing unless that child is still a member of the
  targeted group.
- **§1.2 does not prove EPERM was "for our own child".** The failed
  target was group `-8619`; `leader=live` observed process `8619`.
  Nothing measured `getpgid(8619)`, so the occurrence establishes only
  that a group target computed from the spawn-time assumption returned
  EPERM while the leader process was alive. That is enough to invalidate
  the comment as a reason to discard arbitrary group errors, but not to
  attribute the errno to the child.
- **Stage A §1.3 is *not* falsified.** It said the ledger is disjoint
  from **the PTY path**, because the ledger arms only for
  `proc.spec.group` and PTY mode cannot set it. That remains true. §1.2
  is the separate `spec.group` path. Rev 2 conflated "this path" with
  "the signal path generally" and claimed a falsification it had not
  made.

Stage A's entity-split analysis concerns the PTY path, where the target
is read from `tcgetpgrp`. It does not apply to §1.2, where the target is
computed as `-leader_pid` with no read at all.

### 1.4 The landed acceptance cannot discriminate

`a_group_directed_kill_failure_reports_target_and_leader_separately`
(`:2400`) spawns a PTY child and asserts the exact string

```
target=-{pid} via tcgetpgrp, leader_pid={pid}, expected_group=-{pid}, leader=live
```

— the same `pid` three times. **An implementation that ignored
`tcgetpgrp` and substituted `leader_pid` would pass.** The test's own doc
comment concedes it: "here they are asserted to agree only because
nothing has moved the terminal." The premise of the diagnostic is that
these entities can diverge, and nothing exercises a case where they do.

### 1.5 A numeric pgid cannot establish identity

The value is read before `kill`, the window remains open, and a *number*
cannot distinguish the original group from a recycled one.

**No portable mechanism closes this.** `pidfd_open` + `pidfd_send_signal`
close pid reuse for a single *process* on Linux; there is no
process-*group* equivalent, and macOS has no pidfd. The failures are
macOS-only, so nothing available makes group signalling identity-safe.

**This lane therefore records and does not retarget.** No acceptance
claims the telemetry is sufficient.

One narrowing fact, stated as narrowing and **not** as identity: POSIX
does not free a child's pid until the parent reaps it, and §1.2 observed
`leader=live` from a `try_wait` that had not reaped. While that pid is
held, no *new* group can be created bearing that pgid value. This makes
recycling an unlikely explanation **for that one occurrence**, and says
nothing about whether the group still held a signallable member — which
is what EPERM actually turns on.

### 1.5a The nix surface is gated, and available for a non-obvious reason

Both calls this lane would use live inside block-level gates:

- `getpgid` (`nix-0.29.0/src/unistd.rs:335`) is inside
  `feature! { #![feature = "process"] }` opened at `:167`.
- `tcgetpgrp` (`:368`) is inside
  `feature! { #![all(feature = "process", feature = "term")] }` at `:360`.

pmacs declares `nix` with `features = ["signal", "user", "fs", "term",
"socket", "poll"]` — **`process` is not listed**. It is enabled anyway
because **nix's own `signal` feature depends on `process`**, verified
with `cargo tree -e features -i nix:0.29.0`:

```
├── nix feature "process"
│   └── nix feature "signal"
│       └── pmacs v1.0.0
```

Confirmed by compiling both calls against the real dependency graph.

**This is stable but implicit.** The lane adds `process` to pmacs' own
feature list so the dependency is declared rather than inherited — a
one-line change that makes a real requirement visible.

*Rev 2 asserted `getpgid` was "ungated", from reading the four lines
above it. The gate was 168 lines up. Recorded because it is the same
defect class this document exists to fix.*

### 1.6 The PTY fallback is invisible; a safe owned-dup bridge preserves errno

`signal_target` (`:757-785`): when the PTY branch's
`master.process_group_leader()` returns `None`, control falls through —
`spec.group` is rejected at spawn for PTY mode — and returns
`TargetSource::LeaderPid`, rendered "leader-pid" (`:738`). **A normal
pipe child renders identically.** Two situations, one string.

`portable-pty` (`portable-pty-0.9.0/src/unix.rs:374`) discards the errno:

```rust
fn process_group_leader(&self) -> Option<libc::pid_t> {
    match unsafe { libc::tcgetpgrp(self.fd.0.as_raw_fd()) } {
        pid if pid > 0 => Some(pid),
        _ => None,
    }
}
```

`nix::unistd::tcgetpgrp` requires `F: AsFd`, while
`MasterPty` exposes only `fn as_raw_fd(&self) -> Option<RawFd>`
(`portable-pty-0.9.0/src/lib.rs:114`). Rev 3 inspected only the standard
library's raw-to-owned constructors and concluded every bridge required
`unsafe`. That missed the safe duplication abstraction already in the
dependency graph:

- `filedescriptor::OwnedHandle::dup<F: AsRawFileDescriptor>(&F)` is safe
  (`filedescriptor-0.8.3/src/lib.rs:230`);
- on Unix, `filedescriptor` implements `AsRawFileDescriptor` for every
  `T: AsRawFd` (`src/unix.rs:20`);
- `OwnedHandle` implements `AsFd` (`src/unix.rs:64`).

A small wrapper holds a borrow of `MasterPty` for its lifetime and
implements the safe `AsRawFd` trait by returning the master's reported
fd. `OwnedHandle::dup` consumes that borrowed view immediately and
returns an independently owned duplicate; `tcgetpgrp(&owned)` then
preserves the `Errno`. **No raw-to-owned constructor and no `unsafe`
appears in pmacs.** `filedescriptor 0.8.3` is already in `Cargo.lock`
through `portable-pty`; this lane adds it as a direct dependency because
pmacs now calls its API.

`OwnedHandle::dup` is itself fallible and preserves its Unix
`std::io::Error` source. That failure must not be collapsed into the
terminal query. The PTY result is therefore four-way and discriminating:

- a positive pgid selects the foreground group, as today;
- a duplicate failure falls back to the leader and reports the
  `duplicate-master-fd` stage plus its OS errno;
- a `tcgetpgrp` error falls back to the leader and reports the errno;
- absence of a master fd is a distinct unavailable source, not forged
  into an errno.

### 1.7 The report omits which signal failed

`signal_failure_report` (`:830-850`) takes target, leader pid, errno and
leader observation — **not the signal**. `signal` (`:1074`) has it, and
the public Lua surface accepts INT, USR1, USR2 and QUIT besides the fatal
three (`src/lua_bindings/mod.rs:8627-8629`). A failed `SIGUSR1` and a
failed `SIGTERM` are today textually indistinguishable.

**Rev 2 justified this by claiming their dispositions differ. That was
wrong.** `signal` returns `Err` at `:1092-1098`, *before* the fatal-signal
branch at `:1099`, so **every failed kill is disposition-identical**
regardless of signal. The disposition difference is real only for
**successful** calls. The reporting gap stands on its own: you cannot
tell which signal failed. Acceptance 3 separates the two.

### 1.8 The disposition consequence, and why "recoverable" was wrong

`signal` returns `Err` **before** the state transition and **before**
arming the ledger:

| Which `terminate` hits EPERM | Consequence |
|---|---|
| A **later** one (§1.2's case) | Caller sees `Err`. State is already `Exiting` and a ledger entry **remains scheduled**. |
| The **first** one | State stays `Running`, ledger never armed. **No escalation is ever scheduled** — the child is abandoned. |

**Rev 2 called the first row "recoverable" and claimed "SIGKILL
escalation still happens". Unsupported.** `tick_reap_ledger`
(`:1249-1254`):

```rust
if nix::sys::signal::kill(Pid::from_raw(-*pgid), None).is_err() {
    return false;                       // drops on ANY error, incl. EPERM
}
if now >= entry.deadline && !entry.killed {
    let _ = nix::sys::signal::kill(..., Some(Signal::SIGKILL));  // result discarded
    entry.killed = true;                // marked killed regardless
}
```

If a ledger probe returns EPERM, it **drops the entry and cancels
escalation silently**; if its `SIGKILL` fails, the result is recorded as
if it succeeded. §1.2 did not observe either ledger call — it observed a
later explicit `SIGTERM` to the same assumed group number — so the
ledger failure is an exposed, still-unmeasured hazard rather than an
observed occurrence. The honest statement remains that escalation is
*scheduled*, not that it happens.

**This still-silent path is parked, explicitly** (§5) rather than
absorbed: it is a second site with its own disposition questions, and
folding it in would repeat Stage A rev 3's error of implementing Stage B
inside Stage A.

### 1.9 The first-call variant is already pinned

`an_injected_failure_changes_no_state_and_arms_no_ledger` (`:2501`)
already spawns a `spec.group` child, injects EPERM on the **first**
`terminate`, and asserts `Running` plus an empty ledger. **Rev 2's Bet 5
proposed inventing it.** It is ground truth, and its exact-string
assertion (`:2517`) is one of the four sites acceptance 5 must update.

### 1.10 Limits of the evidence

- **Not reproduced locally.** Development is Linux; failures are
  macOS-only. No claim rests on a local repro of the EPERM.
- **Two occurrences, in different paths** — PR #172 was the PTY path
  (`acc28`, luajit), §1.2 the group path (lua54). Not one flaky test.
- **The mechanism is not established**, and this lane does not propose
  one.


## 2. Questions

- **Q#DC1** — Can the two entities be made to diverge in a test? *Yes:
  under a PTY, `/bin/bash` with job control enabled launches a
  **foreground** job in its own process group and hands it the terminal,
  so `tcgetpgrp` != leader pid.*
- **Q#DC2** — Should the PTY fallback get its own `TargetSource`?
  *Proposed: yes. A failed duplicate or terminal lookup reports its stage
  and errno; a missing master fd reports unavailable (§1.6).*
- **Q#DC3** — Should the report name the signal? *Proposed: yes, on the
  reporting argument alone (§1.7).*
- **Q#DC4** — Should the measured pgid be reported for `spec.group`
  children? *Proposed: yes, as an observation distinct from the assumed
  value, with no sufficiency claim (§1.5).*
- **Q#DC5** — Retarget or tolerate anything? **No. Parked.**


## 3. Bets

- **Bet 1 — the divergence is constructible.** A PTY fixture where the
  foreground group is not the leader: `/bin/bash --noprofile --norc -m`
  launches a foreground child in a fresh process group. The fixture
  performs a bounded wait until `tcgetpgrp` itself reports the non-leader
  group, asserts that group still has a live member as the positive
  control, and only then injects the failing `kill`. The rewritten
  acceptance asserts both exact values **and that they differ**.
  - *Falsified if* the fixture cannot be made deterministic in CI. Then
    the lane falls back to pinning divergence at the `signal_target` unit
    level with an injected foreground group, and labels that as weaker.
  - **OUTCOME: falsified on macOS.** Both macOS legs observed the
    terminal stay with the leader for the full bounded wait; Linux
    diverges reliably. The fallback shipped: the divergent case is
    pinned by injection everywhere, and the real shell corroborates it
    on Linux only.
  - **The injected pin is weaker, and here is exactly how.** It proves
    the target is read from the *lookup* rather than substituted from
    the leader — the substitution mutation still fails it. It does
    **not**, by itself, prove any real shell produces that divergence;
    `job_control_really_diverges_the_foreground_group` carries that, on
    one platform.

- **Bet 2 — the PTY fallback is reachable and distinguishable.** A test
  drives all three non-success arms: a duplicate errno, a `tcgetpgrp`
  errno, and a missing master fd. Each source is distinct from a pipe
  child's and from the others.
  - *Falsified if* the branch cannot be reached without faking the
    lookup — in which case the seam is made injectable exactly as Stage A
    made the kill injectable (Q#PD4), stated rather than hidden.

- **Bet 3 — naming the signal is free.** Thread `signal` into the report.
  - *Falsified if* any exact-string test cannot be updated mechanically.

- **Bet 4 — a `spec.group` child's measured pgid can be made to differ
  from its pid.** **Not via `setsid`:** `spec.group` sets
  `process_group(0)` before exec, so the recorded child is already a
  process-group leader, and a group leader's `setsid` fails with EPERM.
  Forking a `setsid` helper does not help either — `getpgid(recorded_pid)`
  still observes the wrapper.
  The fixture instead has the recorded child **`setpgid` into another
  existing group in the same session**, with a readiness handshake before
  the measurement and explicit cleanup of the anchor group afterwards.
  - *Falsified if* no such fixture is deterministic — in which case the
    measurement is unfalsifiable and **does not ship**, per §1.4's lesson.


## 4. Acceptance

1. The divergent case is pinned on **every** platform: a group-directed
   failure whose target differs from the leader pid, with both exact
   values asserted and asserted to differ. The pre-Stage-B test is
   **rewritten**, not supplemented — it pinned a substitution as
   acceptable.

   **The divergence is injected at the `signal_target` seam**, per Bet
   1's falsification: a real `bash -m` fixture diverges on Linux and
   never on macOS. The injected form is weaker and §3 Bet 1 says how.

1a. **A Linux-only corroboration** drives a real job-control shell,
   forces *only* the kill failure, and asserts the production report
   names the real foreground group. This is the sole test that exercises
   `pty_foreground_group` end-to-end — every other test supplies the
   group itself and therefore cannot detect a lookup that always falls
   back. **Residual limitation, stated rather than buried: on macOS the
   production lookup has no end-to-end coverage**, because the platform
   cannot produce the precondition.
2. The PTY foreground-lookup fallback reports a source distinct from a
   pipe child's. Separate tests drive the duplicate-error and
   `tcgetpgrp`-error arms and assert the exact stage and errno; a third
   drives the unavailable-fd arm. If one cannot be produced reliably
   through a real PTY, the lookup result is injected while the branch,
   target choice, real child observation, and report construction remain
   production code (§3 Bet 2).
3. The report names the signal, split into two independent checks:
   (a) a **failure-format** comparison showing `SIGUSR1` and `SIGTERM`
   failures differ *in text only*, both leaving state and ledger
   unchanged; and (b) a **successful-call disposition control** showing
   a successful `SIGUSR1` does not transition state or arm the ledger
   while a successful `SIGTERM` does.
4. For `spec.group` children the report carries the measured pgid as a
   field distinct from the assumed one, renderable as unobservable, with
   a test asserting a case where they **differ** (§3 Bet 4). **It is
   sampled before the `kill`**, not during report construction, so it
   records pre-kill evidence about the target that was attempted rather
   than state left behind by the failure.

   **It does not describe the group at the moment the `kill` executed.**
   `getpgid` and `kill` remain separated by the read-then-act window
   §1.5 describes, so the sample can be stale by the time the signal is
   delivered. Moving it earlier removes a *post-hoc* reading; it does
   not make the reading contemporaneous, and no acceptance may claim it
   does.
5. All four exact-string sites — `:2408`, `:2435`, `:2485`, `:2517` —
   updated **individually**, each listed in the PR body with before and
   after. No blanket rewrite: that is how a format regression hides.
6. `:2501`'s existing first-call pin is **retained and cited**, updated
   only for the new format.
7. `process` added to pmacs' declared `nix` features, and
   `filedescriptor 0.8` declared directly for the safe PTY-fd duplicate
   (§1.5a, §1.6).
8. `docs/agent-handoff.md` records that ownership of the recorded child
   cannot justify dismissing an error from a group target, with the run
   link and §1.2's measurement limit; the comment at `:1246` is corrected
   in the same PR without claiming that the child itself received EPERM.
9. **No acceptance claims the telemetry establishes group identity**
   (§1.5), and none claims escalation is guaranteed (§1.8). The PR body
   repeats both.


## 5. Parked

- **The reap ledger's silent cancellation** (§1.8): if a probe returns
  EPERM the entry is dropped, and a failed `SIGKILL` is marked as killed.
  The explicit-signal occurrence exposes the premise but did not observe
  either ledger call. **Its own lane** — disposition questions, second
  site.
- **Retargeting to the measured pgid.** Behavioural; unsupported by §1.5.
- **Any tolerance rule for EPERM or ESRCH.** Unmotivated across Stage A's
  three revisions and still unmotivated.
- **§1.8's first-call abandonment.** Pinned at `:2501`, not fixed here.
- **Q#PS6** — `terminate` on an already-reaped process returning `Ok`.
- **`signal_target`'s read-then-kill of `tcgetpgrp`** — Stage A's "most
  likely real fix site, still unframed". This lane makes it *observable*,
  not fixed.
- **`compile_mode_acceptance` reading the developer's real
  `~/.config/pmacs/init.lua`** — separate defect, 11 local failures,
  invisible in CI.


## 6. Gates

Standard suite, each its own step with a real exit status and nothing
after the command that could mask it: `cargo fmt --check`; `cargo clippy
--workspace --all-targets -- -D warnings`; `cargo test --lib`; `cargo
test --lib --features crdt`; `compile_mode_acceptance`;
`terminal_copy_mode_acceptance` (both feature configurations);
`bottom_panel_stage1_acceptance`; `cargo test --test m4_acceptance --
--skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`;
`git diff --check`.

**All local runs use an isolated `XDG_CONFIG_HOME`** — without it
`compile_mode_acceptance` fails 11 tests for unrelated reasons.

**PTY job-control tests are load-sensitive.** Run repeatedly; the PR body
records the repetition count, not a single green.


## 7. Branch plan

One branch, one PR. Bet 1 first and alone: it decides whether the
diagnostic is worth extending at all. If the divergence fixture cannot be
made deterministic, the lane is re-scoped rather than pushed through.
