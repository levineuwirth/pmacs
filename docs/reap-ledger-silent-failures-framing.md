# Framing — group cleanup fails silently at four sites

**Revision 2.** Status: awaiting review round 2. Proposed lane:
`reap-ledger-silent-failures`, worktree `../pmacs-reap-ledger`, based on
`githubsucks/main` @ `22df6ab` (a reading; re-measure at branch time).

**Parked by PR #200's framing §5 and unparked by its evidence.** #200
retired the premise that justified the ledger's leniency; it deliberately
changed no disposition, and said so. This lane owns what it refused.

## Revision history

**Revision 1 → 2**, after review round 1 (three blocking, two major).
All five accepted; all five verified in the code first.

- **§0 named the wrong journey step.** It claimed step 8, "Open a
  terminal", while Q#RL5 in the same document says the ledger is
  unreachable for PTY children. Both cannot be true. The only production
  `group = true` caller is compile mode
  (`builtin/runtime/compile.lua:820`), so this is **step 9, build/test**,
  plus general pipe-process cleanup.
- **A fourth site was missed.** `final_drain_runtime` (`:2331`) probes
  `kill(-pgid, None)`, treats every error as dead, discards its `SIGKILL`
  result and sets its own `group_killed` flag — and it enforces the
  ledger deadline *while no tick runs*. It is now in scope (§1.2a).
- **The staging contradicted itself.** "Bet 1 ships alone" against an
  acceptance requiring three failure-path tests, under one-branch /
  one-PR. A seam with no tests does not even prove it reaches the
  production calls. §7 now scopes the first PR as seam **plus**
  behaviour-preserving tests, which is still diagnosis-only.
- **A generic seam is the wrong shape.** `shutdown()` calls
  `self.signal(*id, SIGKILL)` *before* its ledger force-kill, so a single
  "next kill errno" would be eaten by the wrong call. The seam is now
  site-directed (§3 Bet 1).
- **§1.3 overstated the loop coupling.** Early exit needs the ledger
  empty **and** `any_running()` already false; a live managed record
  keeps the loop going regardless. The precondition is now stated.

**Diagnosis first. No disposition change is proposed in this revision.**
§2 asks whether one is warranted; §5 parks every candidate until the
lane's own evidence exists. That ordering is not caution for its own
sake — Stage A of the signal lane had three tolerance rules rejected
across three revisions, each because it concluded something about one
entity from something that was not about that entity, and this is the
same shape of problem on the same data structure.

## 0. Coherence impact (COHERENCE §20)

- **Journey step 9, "Build and test"** — every compile and grep run,
  plus general pipe-process cleanup. **Not step 8:** the ledger arms
  only for `proc.spec.group`, which spawn *rejects* for PTY mode, so no
  terminal ever reaches it (Q#RL5). The only production `group = true`
  caller is compile mode (`builtin/runtime/compile.lua:820`).
  **No grade change proposed.**
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

### 1.2 Three silent failures in the persistent ledger

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

### 1.2a A fourth site: the in-drain twin

`final_drain_runtime` (`:2331`, called once from `:1573`) runs the same
pattern on the same pgid, while **no tick is running**:

```rust
let group_alive = nix::sys::signal::kill(Pid::from_raw(-ctx.pgid), None).is_ok();
if group_alive && now >= ctx.deadline && !group_killed {
    let _ = nix::sys::signal::kill(Pid::from_raw(-ctx.pgid), Some(Signal::SIGKILL));
    group_killed = true;
}
```

`is_ok()` collapses every errno into "dead", exactly as the persistent
ledger's `is_err()` does — and the consequence differs. A false "dead"
here makes `quiesced` true, which sets `rt.cancel` and **cancels the
readers**, so the failure mode is truncated output rather than a leaked
process.

**It is not identical to the persistent ledger and the framing does not
claim it is.** A later `tick` can retry the ledger entry; this decision
is terminal for that drain. It is in scope because it is the same
collapse on the same data with its own consequence, not because it is
the same bug.

**It constrains the seam.** `final_drain_runtime` is a free function
taking `&RuntimeHandles` and `Option<GroupDrainCtx>` — there is no
`&mut self` to hang a supervisor field on, so the injected outcome has
to arrive through `GroupDrainCtx`, populated at the `:1573` call site.

### 1.3 The shutdown loop's exit condition depends on the silent drop

`shutdown()`'s final loop (`:1773-1775`) runs while
`self.any_running() || !self.reap_ledger.is_empty()`, bounded at 2s, and
calls `tick()` — which calls `tick_reap_ledger`.

So the loop terminates when the ledger empties, and **the ledger empties
via (a)**. On `ESRCH` that is correct: the group is gone. On any other
errno the loop exits *early*, having concluded cleanup finished because
the probe failed.

**The precondition matters and revision 1 omitted it.** The condition is
a disjunction: `any_running() || !reap_ledger.is_empty()`. An early exit
therefore needs the ledger empty **and** `any_running()` already false —
a live managed record keeps the loop running whatever the ledger does.
The coupling is real for the case the ledger exists to serve, the
leader-exited survivor, and Bet 3 must build exactly that fixture rather
than any group.

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

- **Bet 1 — the four sites can be made injectable, site by site,
  without changing behaviour.** Not a generic "next kill errno":
  `shutdown()` calls `self.signal(*id, SIGKILL)` *before* its ledger
  force-kill, so a single shared one-shot would be consumed by the wrong
  call and the test would pass while proving nothing.

  The seam is **directed and one-shot per site**, with four independently
  addressable outcomes:

  | Site | What is injected |
  |---|---|
  | `tick_reap_ledger` probe | the `kill(-pgid, None)` result |
  | `tick_reap_ledger` escalation | the `SIGKILL` result |
  | `shutdown()` force-kill | the `SIGKILL` result |
  | `final_drain_runtime` probe / kill | both, via `GroupDrainCtx` (§1.2a) |

  Bet 3's coupling test needs **two at once** — a failed shutdown
  force-kill *and* a failed subsequent probe — so the seam must express
  more than one pending outcome. A typed queue per site, or a per-site
  slot, either is acceptable; a single global slot is not.

  **Fixture cleanup is part of the seam, not an afterthought.** An
  uninjected outcome left armed leaks into the next test in the same
  binary, and these tests run single-threaded in CI. Each seam is
  consumed on use and asserted empty at fixture teardown.
  - *Falsified if* a site cannot take a directed outcome without
    restructuring the code under test — which would make the test a test
    of the restructuring. `final_drain_runtime` is the one at risk,
    being a free function.

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
  - **The fixture must have `any_running()` already false** (§1.3): a
    leader that has exited leaving a group survivor. Any other shape
    tests the disjunction's other arm and proves nothing about the
    coupling.
  - *Falsified if* the loop still runs to its bound with the ledger
    emptied and no managed record live — in which case §1.3 overstates
    the coupling and the two changes can be separated after all.

- **Bet 4 — a failure here is reportable at all.** Q#RL3 has no answer
  yet. This bet is a scouting obligation, not a design: find every
  channel a background tick could use, and say plainly if none exists.
  - *Falsified if* the only available channel is one already known dead
    (`pmacs.error`) — in which case reporting becomes its own lane and
    this one ships instrumentation plus tests without it.


## 4. Acceptance

1. A **directed, multi-outcome** injection seam covering all four sites
   of §3 Bet 1, on Q#PD4's terms: result only, everything else
   production code. Consumed on use, asserted empty at teardown.
2. A test per silent failure, each asserting the observable consequence
   — entry dropped while the group lives; `killed` set after a failed
   kill; the same at shutdown; readers cancelled after a false "dead" in
   the drain — rather than that a function was called.
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

One branch, one PR: **the seam together with the tests that exercise
it.** Revision 1 said "Bet 1 ships alone", which contradicted an
acceptance requiring failure-path tests under one-branch/one-PR — and
was wrong on its own terms, because a seam with no tests does not even
demonstrate that it reaches the intended production calls.

This is still diagnosis-only: every test pins **current** behaviour,
including the behaviour that is wrong. Nothing in this PR changes what
the supervisor does.

If Bet 2 finds a path unreachable, the lane shrinks and says so rather
than manufacturing a failure to justify itself. If Q#RL3 finds no
reporting channel, reporting becomes its own lane and this PR ships
instrumentation plus tests without it.
