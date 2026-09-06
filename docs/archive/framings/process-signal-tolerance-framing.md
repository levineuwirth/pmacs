# Framing — make the PTY terminate failure self-describing (diagnostic only)

**Revision 4.** Status: awaiting review round 4. Lane:
`pty-terminate-eperm`, worktree `../pmacs-math-slice`, based on
`githubsucks/main` @ `ccf29e3`.

**Diagnostic only. No disposition changes, no tolerance rules, no
behavioural fix.** Every rule this document proposed across revisions 1
to 3 is parked (§5). The lane's entire deliverable is that the next
occurrence of the failure explains itself.

## Revision history

**Revision 3 → 4**, after review round 3 (two blocking, one major) and
its scope call. All accepted.

- **Group-directed ESRCH was also unsafe**, for the same reason EPERM
  was: it proves the selected *foreground group* vanished, not that the
  leader exited. A job-control race — foreground job exits after
  `tcgetpgrp` and before `kill`, shell alive and not yet reclaiming the
  terminal — would have been reported as success with the leader never
  signalled. Rev 3's acceptance 7 pinned that unsafe behaviour. **All
  tolerance is parked** (§5).
- **Rev 3's Stage A implemented Stage B.** It declared itself
  diagnostic-only, then listed tolerance and bookkeeping acceptances.
  Removed.
- **Q#PS6 (already-reaped `terminate` is `Ok`) is parked separately.**
  It is an independent behavioural fix answering a different failure;
  under one-feature/one-PR it does not ride with instrumentation.
- **"Strictly additive / cannot regress behaviour" was overstated** and
  is narrowed (Q#PD3).
- The injected-kill seam is restored as an explicit decision (Q#PD4).

**Rounds 1–3, for the record.** Rev 1 classified on errno alone and
claimed a live owned child cannot yield EPERM — false. Rev 2 gated on
`try_wait`, which observes the leader while a PTY signal targets the
foreground group — unsound whenever those diverge, and it could not be
shown to fix the observed failure at all. Rev 3 corrected EPERM but left
ESRCH unsafe and mixed the stages. **Three consecutive designs were
wrong in the same direction: each tried to conclude something about a
process from something that was not about that process.**


## 0. Coherence impact (COHERENCE §20)

- **Journey step 8, "Open a terminal"** (§2), teardown half. **No grade
  change and no behavioural change** — this lane only improves what a
  failure reports.
- **Serves §9 (worker model), failure attribution**, in its most literal
  sense: an error that names only an errno cannot be attributed.
- **Interaction islands: none. Config registry: not adopted.
  Background-work attribution: unchanged.**
- **No audited claim in COHERENCE.md changes**, so under §25 no
  COHERENCE edit rides this PR.


## 1. Ground truth (scouted @ `ccf29e3`, re-verified each revision)

### 1.1 The failure reports an errno and nothing else

`ProcessSupervisor::signal` (`src/process.rs:921`) maps the `kill`
failure to `format!("kill: {e}")` (`:931`). That string is everything a
reader gets.

### 1.2 The signal target is not the observation target

- **Signal target** — `signal_target` (`:687`) returns `-pgrp` for a
  PTY, where `pgrp = master.process_group_leader()`: the tty's
  **current foreground process group**, read at signal time.
- **Observation target** — `ChildHandle::try_wait` (`:668`) observes the
  **spawned leader**.

They coincide only while the leader owns the terminal. Job control is
precisely the mechanism that makes them diverge, and the PTY path is
**always group-directed by design** — spawn rejects `group = true` for
PTY mode with the rationale that "PTY children already lead their own
session and are signaled group-wide" (`:1428-1429`).

**This is why every tolerance rule across rev 1–3 failed review**, and
why the diagnostic must record the target and the leader state as
*separate* facts.

### 1.3 The reap ledger is disjoint from this path

`tick_reap_ledger` (`:1075`) treats any probe error as "nothing left we
can reach" for **bounded growth**, asserting EPERM "cannot happen for our
own children". It is armed only for `proc.spec.group`, which PTY mode
cannot set. Rev 1's "asymmetry" argument was a misreading; withdrawn.

### 1.4 The observed failure, and the limits of the evidence

macOS CI, PR #172 (**docs-only** diff), `Test (macos-latest / luajit)`,
`acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel`
([attempt 1](https://github.com/levineuwirth/pmacs/actions/runs/30177276839/attempts/1)):

```
in function 'terminate'
cause: ExternalError(Process("kill: EPERM: Operation not permitted"))
```

**Established:** the errno, and the call path
(`pmacs.terminal.terminate` → `session.rs:566` → `signal`).

**Not established:** that the child had exited (the probe's last source
statement is a file write at
`tests/bottom_panel_stage1_acceptance.rs:2239`; CPython teardown follows
and does not synchronise with it); that any pgid was recycled; or what
the signal target actually was.

**This is the whole reason the lane is diagnostic.** Every candidate fix
needs at least one of those three facts, and none is available.

### 1.5 Caller inventory

| Caller | Disposition |
|---|---|
| `src/lsp.rs:1364`, `:2427` | discards (`let _ =`) |
| `src/mcp.rs:1229`, `:1239`, `:1915` | discards (`let _ =`) |
| `src/terminal/session.rs:319`, `:607`, `:635` | discards (`let _ =`) |
| **`src/terminal/session.rs:566`** (propagating at `:577`) | **propagates** as `TerminalError::Process` |
| supervisor-internal `shutdown` path | discards |
| `src/lua_bindings/mod.rs:8150`, `:8164` | propagates to Lua |
| `src/lua_bindings/mod.rs:8717` | propagates (via `session.rs:566`) |
| `src/daemon.rs:4162` | **test-only** `.expect`, not production |

No test in the repository asserts either error string, so widening the
message breaks nothing.

### 1.6 `portable-pty` caches the exit status on Unix

Pinned `portable-pty 0.9.0`: `spawn_command` returns
`std::process::Child` (`unix.rs:228`), and `impl Child for
std::process::Child::try_wait` delegates to
`std::process::Child::try_wait` (`lib.rs:271-277`), which caches into
`self.status`. Both `ChildHandle` variants therefore cache.


## 2. Decisions

### Q#PD1 — what the widened error records

On a `kill` failure in `signal`, the error carries:

| Field | Why |
|---|---|
| **target source** — `tcgetpgrp` vs `group` vs `leader-pid` fallback | which branch of `signal_target` (`:687`) ran |
| **target kind and value** — `-pgid` or `pid`, with the number | the entity actually signalled |
| **spawn-time pgid / leader pid** | a divergence from the target is the job-control hypothesis, visible only by comparison |
| **errno** | as today |
| **leader `try_wait` state** — `exited(status)` / `live` / `unobservable(e)` | separates "the leader is gone" from "the group we signalled is gone" — the distinction all three failed designs collapsed |

Every candidate Stage B rule is decidable from these five together, and
none is decidable from the errno alone.

### Q#PD2 — the disposition is preserved exactly

The call still fails, with the same `Err`, in every case. No state
transition changes, no ledger arming changes, no tolerance. A reader
diffing behaviour should find none.

### Q#PD3 — the honest claim is "no disposition change", not "strictly additive"

Rev 3 said the diagnostic was only an error-string change and could not
regress behaviour. **That overstated it.** `try_wait` on an exited child
**reaps it and caches the status**, so consulting it in the failure path
is an internal state change: the child may be reaped earlier than it
otherwise would be.

Observably safe, because both variants cache (§1.6) and `poll_one`
(`:1133`) will still see `Ok(Some(_))` and emit its event. But safe by
argument is not safe by assertion, so the terminate-failure-then-tick
event pin is retained (acceptance 5).

### Q#PD4 — the injected-kill seam injects the KILL, never the observation

Acceptance 5 needs a forced `kill` failure while the **real**
`ChildHandle::try_wait` runs against the **real** child. A stubbed
observation would bypass exactly the code path in question.

So the seam is a test-only override of the *kill attempt's result*,
consumed once by the signal path; everything downstream — target
selection, the observation, the error construction — runs for real. This
also makes the diagnostic's own fields testable without racing the
kernel.

### Q#PD5 — nothing else lands here

No tolerance rule, no idempotence change, no `signal_target` change. See
§5.


## 3. Bets (falsifiable)

- **B1 — The five fields are sufficient to discriminate the §1.4
  hypotheses.** Falsified if a recurrence carries all five and still
  leaves the cause ambiguous — which would itself be a finding worth
  having.
- **B2 — Widening the message breaks no caller.** Evidence: §1.5, and no
  test asserts the string.

*Retracted across revisions and not reinstated:* rev 1's "a live owned
child cannot yield EPERM"; rev 2's "exit observation suffices"; rev 2's
"this removes the failure class"; rev 3's "group ESRCH is safe to
tolerate".


## 4. Acceptance

1. A group-directed `kill` failure produces an error carrying all five
   Q#PD1 fields, with the target rendered as `-pgid` and the leader
   state distinct from it.
2. A leader-directed `kill` failure does the same, with the target
   rendered as `pid` and the target source recorded as the fallback
   branch.
3. The leader state renders each of `exited(status)`, `live`, and
   `unobservable(e)` correctly.
4. **The disposition is unchanged**: every injected failure still
   returns `Err`, with no state transition and no ledger arming
   (Q#PD2). Falsified by revert — flipping any arm to `Ok` fails this.
5. **Forced injected kill failure against the real PTY child
   observation**, then tick: exactly one exit event, with the correct
   status (Q#PD3/Q#PD4). A fully stubbed observation does not satisfy
   this and is rejected as vacuous.
6. The existing suites stay green, pinning "no behavioural change" from
   the outside.


## 5. Parked (not deferred-and-forgotten — each needs its own evidence)

- **All tolerance rules.** Group-directed EPERM *and* ESRCH both fail on
  the §1.2 entity split; leader-directed tolerance is plausible but
  unmotivated until evidence shows the fallback branch is ever taken.
  Needs Stage A evidence first.
- **Q#PS6, `terminate` on an already-reaped process returning `Ok`.**
  Independent behavioural fix, different failure (§1.6 of rev 3), its
  own lane under one-feature/one-PR.
- **`signal_target`'s read-then-kill of `tcgetpgrp`** — still the most
  likely real fix site, still unframed.
- `terminate` cancelling pending restarts; PTYs in
  `pmacs.process.list`; any change to `C-c` delivery.


## 6. Gates

Full suite per `CLAUDE.md`. Touched suites:
`bottom_panel_stage1_acceptance`, the vterm stages, and
`compile_mode_acceptance`. Sweep with `-- --skip basedpyright`.


## 7. Branch plan

`pty-terminate-eperm`, one PR, diagnostic only. This framing is its first
commit; the instrumentation and its tests are the second.

**The lane then closes.** It does not wait for the flake to recur: the
next occurrence — whenever it happens, under whoever's PR — carries its
own evidence, and Stage B is framed then. Math work proceeds immediately
after this lands.
