# Framing — the reap ledger fails silently in three places

**Revision 1.** Status: awaiting review round 1. Proposed lane:
`reap-ledger-silent-failures`, worktree `../pmacs-reap-ledger`, based on
`githubsucks/main` @ `22df6ab` (a reading; re-measure at branch time).

**Parked by PR #200's framing §5 and unparked by its evidence.** #200
retired the premise that justified the ledger's leniency; it deliberately
changed no disposition, and said so. This lane owns what it refused.

**Diagnosis first. No disposition change is proposed in this revision.**
§2 asks whether one is warranted; §5 parks every candidate until the
lane's own evidence exists. That ordering is not caution for its own
sake — Stage A of the signal lane had three tolerance rules rejected
across three revisions, each because it concluded something about one
entity from something that was not about that entity, and this is the
same shape of problem on the same data structure.

## 0. Coherence impact (COHERENCE §20)

- **Journey step 8, "Open a terminal"**, teardown half, and every
  compile/grep run through `spec.group`. **No grade change proposed.**
- **Serves §9 (worker model), failure attribution.** The ledger is the
  one mechanism that can see a survivor nothing else can, and today it
  cannot report that it failed to.
- **Interaction islands: none. Config registry: not adopted.
  Background-work attribution: unchanged.**
- **No audited claim in COHERENCE.md changes**; under §25 no COHERENCE
  edit rides this PR.


## 1. Ground truth (verified at `22df6ab`)

### 1.1 What the ledger is for

`tick_reap_ledger` (`src/process.rs:1472`) exists for one case its own
doc names: **a TERM-ignoring descendant that survived its leader's clean
exit with its output redirected.** Neither leader state nor reader state
can see that survivor — `try_wait` reports the leader, and the readers
see a closed pipe. Only group liveness can (Q#CM3, round-3 finding 1).

That is the blast radius. A silent failure here leaks *precisely* the
process the mechanism exists to catch, and nothing else in the
supervisor is looking.

### 1.2 Three silent failures, not two

```rust
// (a) any probe error drops the entry
if nix::sys::signal::kill(Pid::from_raw(-*pgid), None).is_err() {
    return false;
}
// (b) the escalating SIGKILL's result is discarded, and the entry is
//     marked killed regardless
if now >= entry.deadline && !entry.killed {
    let _ = nix::sys::signal::kill(Pid::from_raw(-*pgid), Some(Signal::SIGKILL));
    entry.killed = true;
}
```

**(a)** `retain` returning `false` deletes the entry, so escalation is
cancelled. The comment is honest that this is a *bounded-growth policy*
rather than a claim the group is gone — #200 corrected it — but the
behaviour is unchanged: an `EPERM` probe is indistinguishable from
`ESRCH`.

**(b)** A failed `SIGKILL` is recorded as a successful one. The entry
then satisfies `!entry.killed == false` forever and is never retried.

**(c) `shutdown()` has the same discard** (`:1763-1766`), on the path
that exists specifically to stop a leak at editor exit:

```rust
for (pgid, entry) in &mut self.reap_ledger {
    let _ = nix::sys::signal::kill(Pid::from_raw(-*pgid), Some(Signal::SIGKILL));
    entry.killed = true;
}
```

Its own comment says it is there because "a pre-deadline ledger ... would
be silently discarded at Drop and leak the member". The fix for one
silent leak was written with a discarded result of its own.

### 1.3 The shutdown loop's exit condition depends on the silent drop

`shutdown()`'s final loop (`:1773-1775`) runs while
`self.any_running() || !self.reap_ledger.is_empty()`, bounded at 2s, and
calls `tick()` — which calls `tick_reap_ledger`.

So the loop terminates when the ledger empties, and **the ledger empties
via (a)**. On `ESRCH` that is correct: the group is gone. On any other
errno the loop exits *early*, having concluded cleanup finished because
the probe failed. The mechanism added to prevent a leak at exit can be
ended by the same error that hides one.

This coupling is the reason (a) cannot be changed casually: making the
probe strict without touching the loop converts a silent early exit into
a guaranteed 2-second stall at every editor exit that hits it.

### 1.4 None of the three has been observed

**#200 observed an explicit `SIGTERM` failing in `signal()`, not any
ledger call.** What it established is narrower and still sufficient to
open this lane: a group-directed `kill` computed from the spawn-time
`pgid == pid` assumption returned `EPERM` while the leader was alive, on
macOS, intermittently
([run 30553376486](https://github.com/levineuwirth/pmacs/actions/runs/30553376486/job/90907461258)).

That retires "EPERM cannot happen for our own children" as a *reason* to
discard an arbitrary group error. It does **not** show that any of (a),
(b) or (c) has fired. This lane must not claim otherwise, and §3's bets
are built to find out rather than to assume.

### 1.5 The ledger's kills have no injection seam

`signal()` consults `forced_kill_errno` (Q#PD4), which is how #176 and
#200 tested failure paths without provoking real errnos.
**`tick_reap_ledger` calls `nix` directly and consults nothing.** Same
for `shutdown()`'s loop.

So all three failure paths are, today, **untestable**. That is the
first thing this lane has to fix, and it is a prerequisite for any
disposition change rather than a nicety: a disposition change whose
failure path cannot be exercised is a rule nobody can falsify.

### 1.6 What is tested today

`liveness_probe_reaps_term_ignoring_survivor_after_leader_exit`
(`:4274`), `repeated_terminate_does_not_extend_ledger_deadline`
(`:4314`), `shutdown_force_kills_outstanding_ledger_groups` (`:4350`),
`leader_exit_reap_bounds_drain_with_pipe_holding_descendant` (`:4415`),
and `setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
(`:4450`).

Every one exercises the **success** path. None injects a probe or
`SIGKILL` failure, because §1.5 makes that impossible.

*`repeated_terminate_does_not_extend_ledger_deadline` is also the test
that flaked on macOS in #191 — the occurrence that produced §1.4's
evidence. It is load-sensitive and unrelated to what this lane changes;
noted so a red run on it is not mistaken for this lane's doing.*

### 1.7 Limits of the evidence

- **Not reproduced.** No ledger probe or `SIGKILL` has been seen to
  fail, on any platform.
- **The mechanism is not established.** Why a group-directed `kill` can
  return `EPERM` against a live owned child is still unknown — #200
  narrowed it and explicitly did not solve it.
- **Group identity remains unprovable** (#200 §1.5). Nothing this lane
  measures can distinguish the original group from a recycled one; a
  `pidfd` covers a process, not a group, and macOS has neither.


## 2. Questions

- **Q#RL1** — Should the probe distinguish `ESRCH` from other errnos?
  *Unknown, and deliberately not proposed yet. It is the obvious change
  and it directly trades bounded growth (the stated original reason) for
  correctness, while §1.3 shows it also changes editor-exit timing. It
  needs evidence and its own review.*
- **Q#RL2** — Should a failed escalation be retried, and how many times?
  *Unknown. `killed = true` on a failed `SIGKILL` is clearly wrong as
  bookkeeping; what should replace it is a policy question, not an
  obvious fix.*
- **Q#RL3** — How does a background tick report anything? *The ledger
  emits no events and has no status channel. `pmacs.error` is defined
  only by a test stub and is dead at 15 call sites, so it is not the
  answer. This is the lane's real design question.*
- **Q#RL4** — Does the `shutdown()` loop need its own termination
  condition if (a) becomes strict? *Almost certainly (§1.3), and that
  coupling is why the two cannot be changed independently.*
- **Q#RL5** — Is the ledger reachable for PTY children? **No** —
  `spec.group` is rejected at spawn for PTY mode, so the ledger is a
  pipe-path mechanism only. Stated because #200 revision 2 got this
  exact relationship wrong in the opposite direction.


## 3. Bets

- **Bet 1 — the three failure paths can be made injectable without
  changing behaviour.** A seam on the same terms as `forced_kill_errno`:
  it injects the *result* only, leaving the probe target, the deadline
  arithmetic, the `retain` decision, and the real ledger state as
  production code.
  - *Falsified if* injecting cannot reach all three sites — (a) and (b)
    are in one closure, (c) is in `shutdown()` — without restructuring
    the code under test, which would make the test a test of the
    restructuring.
  - **This bet ships alone and first.** It is worth landing even if
    every later bet is abandoned, because §1.6's coverage is
    success-only and will stay that way otherwise.

- **Bet 2 — each silent failure is demonstrable once injectable.** With
  the seam: an `EPERM` probe drops an entry whose group is still alive;
  a failed `SIGKILL` leaves `killed = true` and is never retried; and
  `shutdown()`'s discard does the same at exit.
  - *Falsified if* any of the three turns out to be unreachable in
    practice — for instance if some earlier guard makes the entry
    already absent. **That would be a genuinely good outcome** and would
    shrink this lane rather than embarrass it.

- **Bet 3 — the shutdown coupling is real and measurable.** A test shows
  the final loop exiting early when the probe errors, rather than
  running to its 2s bound.
  - *Falsified if* the loop's other condition (`any_running()`) holds it
    anyway, in which case §1.3 overstates the coupling and the two
    changes can be separated after all.

- **Bet 4 — a failure here is reportable at all.** Q#RL3 has no answer
  yet. This bet is a scouting obligation, not a design: find every
  channel a background tick could use, and say plainly if none exists.
  - *Falsified if* the only available channel is one already known dead
    (`pmacs.error`) — in which case reporting becomes its own lane and
    this one ships instrumentation plus tests without it.


## 4. Acceptance

1. An injection seam covering all three sites, on Q#PD4's terms: result
   only, everything else production code.
2. A test per silent failure, each asserting the observable consequence
   (entry dropped while the group lives; `killed` set after a failed
   kill; the same at shutdown) rather than that a function was called.
3. Each new test falsified by an actual revert, both directions
   recorded in the PR body.
4. The `shutdown()` coupling of §1.3 pinned by a test, whichever way
   Bet 3 resolves.
5. **No disposition change.** `retain` still drops on any error;
   `killed` is still set unconditionally. This lane makes the failures
   visible and testable; changing them is §5's.
6. Q#RL3 answered in the PR body with the channels actually found, or
   an explicit statement that none exists.
7. `docs/agent-handoff.md` records the three silent paths and that none
   has been observed — the distinction #200 had to make twice.

## 5. Parked

- **Q#RL1's strict-`ESRCH` probe**, **Q#RL2's retry policy**, and any
  other disposition change. Each needs this lane's evidence.
- **Reporting**, if Bet 4 finds no channel.
- **Retargeting to the measured pgid**, and any EPERM/ESRCH tolerance
  rule in `signal()` — still parked from #200, unchanged.
- **`signal_target`'s read-then-kill of `tcgetpgrp`** on the PTY path —
  Stage A's "most likely real fix site", still unframed, still not this.
- **Why a group-directed `kill` can return `EPERM` against a live owned
  child.** The mechanism is unknown; this lane instruments the
  consequences, not the cause.

## 6. Gates

Standard suite, each its own step with a real exit status and nothing
after the command that could mask it: `cargo fmt --check`; `cargo clippy
--workspace --all-targets -- -D warnings`; `cargo test --lib`; `cargo
test --lib --features crdt`; `compile_mode_acceptance`;
`terminal_copy_mode_acceptance` (both feature configurations); `cargo
test --test m4_acceptance -- --skip basedpyright`;
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`; `git diff --check`.

**All five bootstrap-storage variables controlled locally** —
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`,
`PMACS_STATE_HOME` — per the ambient-root framing merged as #201.
Isolating only the config root leaves the data-root write path open.

**Process-supervisor tests are load-sensitive**; the PR body records
repetition counts rather than a single green.

## 7. Branch plan

One branch, one PR. **Bet 1 first and alone**: the seam is the
prerequisite for everything else and is worth landing on its own. If
Bet 2 then finds a path unreachable, the lane shrinks and says so rather
than manufacturing a failure to justify itself.
