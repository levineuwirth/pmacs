# GPU launcher / probe SIGINT teardown — framing

Revision 2. Status: **awaiting approval. No implementation.**

Revision 1 was rejected on five findings. Each is answered below, and
the two that changed the technical picture — the lifetime arithmetic
(§5) and the disposition claim (§3) — are recorded as corrections
rather than quietly rewritten.

## 1. The problem, stated as what is observed

`ctrl_c_on_launcher_group_does_not_reach_spawned_daemon` fails with

```
child did not exit within 5s   (tests/gpu_invocation_acceptance.rs:180, called from :1115)
```

The test spawns `pmacs --gpu --socket <s>` in its own process group
(`process_group(0)`, `:1107`), waits for the probe to report
`phase=ready`, sends `SIGINT` to the **group** (`:1113`), and requires
the launcher to exit within five seconds.

**It fails in two binaries, not one.** `tests/gpu_initial_target_acceptance.rs`
includes the suite as a module, so a reproducing sweep reds twice.
Reference run `20260816T064549Z-2144707/09-sweep-crdt.log`:

| | line | result |
|---|---|---|
| `gpu_initial_target_acceptance` | 3097 | `FAILED. 14 passed; 1 failed … 5.19s` |
| `gpu_invocation_acceptance` | 3131 | `FAILED. 14 passed; 1 failed … 5.18s` |
| green result summaries | — | **119** |

So the correct statement is **119 green result summaries and two red
binaries**. Revision 1 said "119 binaries green, one red", which was
wrong on both halves.

**This is pre-existing on `main`.** The identical `build-crdt &&
sweep-crdt` pair at `72da24a`, clean worktree, own target directory,
fails the same test.

## 2. Why this blocks more than one lane

`sweep-crdt` is stage 15 of the sixteen-stage `--protocol` gate. While
it reds, **no branch can present a green gate**, `main` included.
`panel-mapping-generation` (§5b) is complete with its own fifteen
stages green and is held behind this lane by explicit instruction.

## 3. Ground truth (cited), and what it does *not* establish

- **Neither binary contains signal-handling code.** `run_gpu`
  (`src/main.rs:324`) blocks in `command.status()` (`:363`) — a plain
  `waitpid` — with no handler installed. Grepping
  `SIGINT|signal_hook|sigaction|ctrlc|set_handler|pthread_sigmask|sigprocmask`
  across `pmacs-gpu/src` returns nothing.
- **The probe never blocks indefinitely.**
  `run_headless_managed_probe` (`pmacs-gpu/src/main.rs:1065`) loops on
  `event_rx.recv_timeout(Duration::from_millis(50))`.
- **The daemon *does* handle signals, deliberately.**
  `src/daemon.rs:629-641` registers `SIGTERM`/`SIGINT` via
  `signal_hook::flag`. The daemon is the process the test asserts must
  **survive**, detached from the launcher's group.

**Correction to revision 1.** Revision 1 concluded from the first two
bullets that "two processes with default `SIGINT` disposition should
both die at once". **That does not follow, and it contradicted this
document's own leading hypothesis.** Absence of handler *code* says
nothing about runtime *disposition*: `SIG_IGN` is inherited across
`fork` **and** survives `exec`, so either process can hold a
non-default disposition it never installed — from the test harness,
from `cargo`, or from the invoking shell. Inherited ignore is a live
candidate precisely because the source is silent. What the source
establishes is narrower: **neither binary sets a disposition itself**,
so whatever disposition they hold at runtime was inherited, and that is
measurable rather than arguable.

## 4. Reductions attempted — each with command, count, and log

Preserved off the tmpfs at
`/home/jeans/build/pmacs-gate-targets/probe-sigint-evidence/`, because
`/tmp` is a tmpfs and these were nearly lost to a cleanup mid-lane.

| # | reduction | runs | result | log |
|---|---|---|---|---|
| R1 | `cargo test --features crdt --test gpu_invocation_acceptance ctrl_c_on_launcher_group` | 3 | green, 0.15–0.17 s | *(console; superseded by R2)* |
| R2 | `cargo test --features crdt --test gpu_invocation_acceptance` (whole suite) | 1 | green, 15 passed | `gpu3.log` |
| R3 | `cargo test --workspace --features crdt --no-fail-fast -- --skip basedpyright ctrl_c_on_launcher_group` | 1 | green — every binary runs, only this test executes | `filtered.log` |
| R4 | `cargo test --features crdt --lib --test gpu_invocation_acceptance --no-fail-fast` | 1 | green, 2145 + 15 | `two.log` |
| R5 | `--test gate_script_acceptance --test gpu_invocation_acceptance` | 1 | green | `suspect.log` |
| R6 | three GPU suites in sweep order (`gpu_font`, `gpu_initial_target`, `gpu_invocation`) | 1 | green | `gpu3.log` |
| R7 | targets 1–19 (incl. `--lib --bins`) + the suite | 1 | green | `half1.log` |
| R8 | targets 20–37 + the suite | 1 | green | `half2.log` |
| R9 | **all 37 preceding targets** + the suite | 1 | green | `prefix.log` |
| R10 | `--workspace` with only `gpu_initial_target` + `gpu_invocation` | 1 | green | `wsonly.log` |
| F1–F5 | full `cargo test --workspace --features crdt --no-fail-fast -- --skip basedpyright` | 5 | **red, 5/5** | `base-sweep.log` (at `72da24a`), `postclean.log`, `sweep-inst.log`, `sweep-diag.log`, gate `…-2144707` |

**R9 is the shape of the problem.** The same binaries, in the same
order, with the same tests before it, pass as a subset and fail as part
of the whole. R10 rules out `--workspace` feature unification; other
packages' targets run at log lines 4848+, after the failure at 3066, so
they cannot be implicated either.

Also refuted, by measurement: machine load (red on a quiet box, load
2.77); tmpfs starving RAM (**tested by experiment** — `/tmp` 21 G →
1.2 G, available 27 G → 45 G, still red); leaked daemons (peak 58, +8
per sweep, green runs already at 46–60); inotify (47 of 1024).

## 5. Two retracted claims, both mine, kept as warnings

**Claim A — "mechanism located".** Reported the launcher blocked in
`do_wait` on a probe child in `futex_do_wait`.

**Claim B — the retraction of A.** Argued A was unsupported because the
failing launcher "must live ≥ 8 s" while the sampler's longest-lived
was 5 s.

**Claim B's arithmetic is false.** Both reproducing binaries finish in
**~5.19 s including the five-second timeout** (`:3097`, `:3131`), so
`phase=ready` is reached in roughly a tenth of a second and the failing
launcher lives about **5.1 s total** — squarely inside what the sampler
observed. A ">6 s" selector would therefore have captured **nothing**,
repeating the very sampling error it was written to correct.

So A is **not** refuted by B. A remains **unproven for a different
reason**: the suite spawns launchers from **five** call sites
(`:38, :65, :509, :534, :544, :574, :725, :1097` — eight `--gpu`
arguments across the file), so a launcher captured by command line
alone cannot be attributed to *this* test. The `do_wait` /
`futex_do_wait` pair is consistent with the failing instance and
consistent with a healthy sibling, and nothing recorded distinguishes
them.

The standing lesson is now the opposite of revision 1's: **do not key
on process age at all.** Key on identity.

## 6. Bets

1. **The failure is a real teardown defect** — a user pressing Ctrl-C
   on `pmacs --gpu` sees the same hang. **This is a bet, not a
   finding**, and the current witness does not reach the real GUI
   path: it goes through a wrapper script and `--headless-managed-probe`
   (`:1090-1093`), not a live wgpu frontend. Confirming or dropping this
   bet is D4 below.
2. It is **not** a timing margin. A green run finishes in 0.15 s against
   a 5 s deadline — 33×. Margins that large do not erode.
3. Therefore **raising the deadline is not a fix** and is out of scope.
   If the conclusion turns out to be that the deadline is wrong, that
   needs its own argument and its own approval.

## 7. First step — diagnostics keyed on identity, not age

No fix is proposed; the mechanism is unknown. The first commit is
diagnostic only, and it must **discriminate** the three live candidates:
blocked delivery, inherited ignore, and an escaped or wrong process
group.

- **D1 — key on the PID this test records.** The test already owns
  `launcher.id()`. Capture around its own `kill`, not by scanning for
  age or command line.
- **D2 — snapshot before *and* after the signal**, for the test parent,
  the launcher, and the probe:
  - `SigIgn`, `SigCgt`, `SigBlk` — **per thread**, from
    `/proc/<pid>/task/*/status`, since `SigBlk` is thread-specific and
    a process-wide reading would hide a blocked delivery on the one
    thread that matters;
  - `SigPnd` and `ShdPnd` — a pending-but-undelivered `SIGINT` is
    exactly what distinguishes blocked delivery from ignore;
  - `PID`, `PPID`, `PGID`, `SID` for each — which settles whether the
    signal was even addressed to the right group, and whether anything
    escaped it.
  A post-failure snapshot alone cannot prove inheritance; the
  before/after pair is what makes the claim provable.
- **D3 — run the full sweep under D1/D2 until the failure is captured
  *with* its diagnostics.** Only then propose a fix.
- **D4 — settle bet 1 separately.** Establish whether a real
  `pmacs --gpu` session, not the wrapper/headless probe, reproduces the
  hang. The answer decides whether A5 is an obligation or is dropped.

## 8. Acceptance criteria for the eventual fix

Written now so the fix cannot quietly become "make the test pass".

- **A1.** The mechanism is stated and demonstrated, not inferred: a
  witness failing before the change and passing after, plus a mutation
  showing the witness bites its own clause.
- **A2.** `sweep-crdt` green for **three consecutive full runs on the
  reviewed fixed head of this branch**. Not "on main" — that is
  unobtainable before this lane is approved, gated and merged, and
  revision 1 stated an impossible precondition. Post-merge
  confirmation on `main` is a follow-up, not a gate on the fix.
- **A3.** The R9 paradox is explained, or explicitly recorded as
  unexplained. A fix that greens the sweep without accounting for why
  every subset passed leaves a gap, and the gap is stated rather than
  left for the next reader.
- **A4.** No deadline raised, no test skipped, retried, or serialised
  to obtain green.
- **A5.** **Conditional on D4.** If bet 1 holds, this is unconditional:
  Ctrl-C on a real `pmacs --gpu` session tears down the frontend and
  leaves the daemon running. If D4 shows the hang is reachable only
  through the wrapper/headless path, bet 1 is dropped, A5 is struck,
  and the lane is recorded as gate-correctness only.

## 9. Coherence impact (`COHERENCE.md` §20)

- **Journey step touched: 12(a), "closing is clean."** Ctrl-C teardown
  of a GPU session is exactly that step, whether or not its grade
  moves. **Revision 1 said "journey steps touched: none", which was
  false** — it reasoned from grade movement, which §20 explicitly warns
  against.
- **Grade movement: none expected.** This restores a property that is
  supposed to hold, rather than opening a new one.
- **Interaction islands: none added.**
- **Config registry: not touched. Background-work attribution: not
  touched.**
- Beyond step 12(a), what this lane restores is every *other* lane's
  ability to prove itself, since no branch can show a green gate while
  stage 15 reds.

## 10. Out of scope

- Raising or removing the 5 s deadline (bet 3).
- The ~10 daemons each sweep leaks — real, separately recorded, and not
  implicated: green runs already ran at 46–60 leaked daemons.
- `gpu_initial_target_acceptance` including the suite as a module. It
  is why the failure reds twice, and it is a tidiness question, not a
  correctness one.
