# GPU launcher / probe SIGINT teardown — framing

Revision 1. Status: **awaiting approval. No implementation.**

## 1. The problem, stated as what is observed

`gpu_invocation_acceptance::crdt::ctrl_c_on_launcher_group_does_not_reach_spawned_daemon`
fails inside the `sweep-crdt` gate stage with

```
child did not exit within 5s   (tests/gpu_invocation_acceptance.rs:180, called from :1115)
```

The test spawns `pmacs --gpu --socket <s>` in its own process group
(`process_group(0)`, `:1107`), waits for the probe to report
`phase=ready`, sends `SIGINT` to the **group**, and requires the
launcher to exit within five seconds.

This is **pre-existing on `main`**. It is not caused by any feature
branch: the identical `build-crdt && sweep-crdt` pair run at `72da24a`
in a clean worktree with its own target directory fails the same test —
119 test binaries green, one red.

## 2. Why this blocks more than one lane

`sweep-crdt` is stage 15 of the sixteen-stage `--protocol` gate. While it
reds, **no branch can present a green gate**, `main` included. The
`panel-mapping-generation` (§5b) lane is complete with its own fifteen
stages green and is held behind this lane by explicit instruction.

## 3. Ground truth (cited, not recalled)

- **The launcher does not handle signals.** `run_gpu`
  (`src/main.rs:324`) builds a `Command` for the GPU binary and blocks
  in `command.status()` (`:363`) — a plain `waitpid`. It installs no
  handler, so `SIGINT`'s default action should terminate it outright.
- **`pmacs-gpu` does not handle signals either.** Grepping
  `SIGINT|signal_hook|sigaction|ctrlc|set_handler|pthread_sigmask|sigprocmask`
  across `pmacs-gpu/src` returns **nothing**. The probe should also die
  on the default action.
- **The probe never blocks indefinitely.** `run_headless_managed_probe`
  (`pmacs-gpu/src/main.rs:1065`) loops on
  `event_rx.recv_timeout(Duration::from_millis(50))`, so it wakes twenty
  times a second regardless of traffic.
- **The daemon *does* handle signals**, and deliberately:
  `src/daemon.rs:629-641` registers `SIGTERM`/`SIGINT` through
  `signal_hook::flag`. The daemon is the process the test asserts must
  **survive**; it is detached from the launcher's group, which is the
  property under test.
- **Only one test in the suite uses `process_group(0)`** — this one.

Taken together the ground truth **deepens** the puzzle rather than
explaining it: two processes with default `SIGINT` disposition, one of
them polling at 50 ms, should both die immediately.

## 4. What has been ruled out, each by measurement

Recorded so this lane does not re-run them.

| hypothesis | how it was refuted |
|---|---|
| machine load | red on a quiet box, load 2.77 at launch |
| tmpfs starving RAM | `/tmp` 21 G → 1.2 G, available 27 G → 45 G; still red |
| leaked test daemons | peak 58, only +8 per sweep; green runs already sat at 46–60 |
| inotify exhaustion | 47 instances in use of 1024 |
| `--workspace` feature unification | same two targets under `--workspace` are green |
| a specific preceding test | **all 37 preceding targets + the suite run green** |
| later targets / other packages | they run at log lines 4848+, after the failure at 3066 |

**The last row is the strange one and is the real shape of this
problem**: the same binaries, in the same order, with the same tests
before it, pass as a subset and fail as part of the whole 119-target
sweep. Reproduction is 5/5 in the full sweep across two trees, and 0/N
in every reduction attempted.

## 5. One retracted claim, kept as a warning

An earlier attempt reported the mechanism as "the launcher blocks in
`do_wait` on a probe child stuck in `futex_do_wait`". **That was
retracted on its own evidence.** The sampler behind it caught 394
launchers and the longest-lived was **5 s total**; the failing instance
must outlive its `SIGINT` by 5 s, so its lifetime would be 8 s or more.
What was described is a *healthy* teardown from another test in the
suite. The pair is worth keeping only as the normal shape.

The lesson binds this lane's first step: **an instrument that samples
every launcher and hopes to catch the failing one is not good enough.**

## 6. Bets

1. The failure is a real teardown defect reachable by a user pressing
   Ctrl-C, not a test artifact — because the test asserts an ordinary
   product property and the two binaries involved carry no signal
   handling at all.
2. It is **not** a timing margin. A green run finishes in 0.15 s against
   a 5 s deadline (33×). A margin that large does not erode; something
   different happens.
3. Therefore **raising the deadline is not a fix** and is explicitly out
   of scope. If the conclusion turns out to be that the deadline is
   wrong, that requires its own argument and its own approval.

## 7. First step — an instrument that keys on the failing instance

No fix is proposed yet, because the mechanism is unknown. The first
commit is diagnostic only:

- **D1.** Sample only launchers whose lifetime exceeds ~6 s, so the
  failing instance is the *only* thing recorded, and capture for it:
  `/proc/<pid>/status` (`SigBlk`, `SigIgn`, `SigCgt`, `State`), the
  per-thread `wchan` under `/proc/<pid>/task/*/wchan`, and the same for
  every child. `SigIgn`/`SigBlk` answers directly whether `SIGINT` was
  ignored or blocked — including whether it was **inherited**, since
  `SIG_IGN` survives both `fork` and `exec` while handlers do not.
- **D2.** Record whether the launcher had already reaped its child at
  the moment the deadline expired, which separates "the child will not
  die" from "the launcher will not notice".
- **D3.** Run the full sweep under D1/D2 until the failure is captured
  **with** its diagnostics, and only then propose a fix.

## 8. Acceptance criteria for the eventual fix

Deliberately written now, so the fix cannot quietly become "make the
test pass".

- **A1.** The named mechanism is stated and demonstrated, not inferred:
  a witness that fails before the change and passes after, plus a
  mutation showing the witness bites its own clause.
- **A2.** `sweep-crdt` green on `main` for **three consecutive full
  runs** — 1/1 is not evidence for a defect that hid from every
  reduction.
- **A3.** The reduction paradox is explained or explicitly recorded as
  unexplained. If the fix makes the sweep green without accounting for
  why subsets always passed, that gap is stated in the record rather
  than left for the next reader to rediscover.
- **A4.** No deadline is raised, and no test is skipped, retried, or
  serialised to obtain green.
- **A5.** If the mechanism proves to be in `pmacs-gpu`'s shutdown path,
  a Ctrl-C on a real `pmacs --gpu` session tears down the frontend and
  leaves the daemon running — the product property the test encodes.

## 9. Coherence impact (`COHERENCE.md` §20)

- **Journey steps touched: none.** This is a teardown-correctness and
  gate-trustworthiness lane; no journey step changes grade.
- **Interaction islands: none added.**
- **Config registry: not touched.**
- **Background-work attribution: not touched.**

Naming these explicitly matters: a reader auditing §20 by grade movement
would otherwise conclude this lane touches nothing, when what it
restores is the ability of every other lane to prove itself.

## 10. Out of scope

- Raising or removing the 5 s deadline (bet 3).
- The ~10 daemons each sweep leaks. Real, separately recorded, and not
  implicated here — green runs already ran at 46–60 leaked daemons.
- The `gpu_initial_target_acceptance` binary including this suite as a
  module, which makes a reproducing sweep report the failure twice. A
  tidiness question, not a correctness one.
