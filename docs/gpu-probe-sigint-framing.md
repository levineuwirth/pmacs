# GPU launcher / probe SIGINT teardown — framing

Revision 7. Status: **awaiting approval. No implementation.**

Revisions 1 and 2 were each rejected on five findings. Every correction
is recorded in place rather than quietly rewritten, because three of
them were claims this document itself had advanced:

- r1 → r2: the ">6 s selector" and the "≥8 s lifetime" arithmetic
  (§5); "two processes with default disposition" (§3); "119 binaries
  green, one red" (§1); an unobtainable A2 (§8); "journey steps
  touched: none" (§9).
- r2 → r3: **"R9 ran the same binaries" — it did not** (§4); "the probe
  never blocks indefinitely" (§3); the launcher call-site count (§5);
  reduction provenance, now in `docs/probe-sigint-evidence.md`; and
  ledger corrections that had not been made portable (§11).
- r3 → r4: the interaction table overstated (§4); the red count was 7,
  not 5, and abbreviated argv were not argv (manifest); "workspace
  artifact family" conflated Cargo suffix with byte identity
  (manifest); ledgers still carried the falsified R9 conclusions (§11).
  **And a finding that reframes the lane: the failure has a datable
  onset (§4a) and is not long-standing.**
- r6 → r7: the ancestry supports **no** causal statement at all — the
  observations are non-comparable, and even "outcome is not determined
  by commit alone" is withdrawn (§4a); D0a was not yet a valid decision
  procedure
  (§7); "neither binary contains signal-handling code" is false — the
  `pmacs` binary registers SIGINT in daemon mode (§3); residual
  artifact wording (§4, manifest).
- r5 → r6: the ancestry argument overreached (§4a) — it shows outcome
  is not determined by commit alone, and nothing more; residual
  byte-identity and "artifact family" wording in both ledgers (§11);
  and three provenance slips (§4, manifest).
- r4 → r5: the section summaries still carried revision-3 counts and
  groupings (§4); the onset count was 13/1/3, not 14 (§4a); "byte-
  different" overstated what is knowable about historical artifacts
  (§4, manifest); and **the onset is not a source boundary** (§4a).

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

- **`run_gpu`'s own path installs no handler.** It
  (`src/main.rs:324`) blocks in `command.status()` (`:363`) — a plain
  `waitpid` — with nothing installed along the way. Grepping
  `SIGINT|signal_hook|sigaction|ctrlc|set_handler|pthread_sigmask|sigprocmask`
  across `pmacs-gpu/src` returns nothing.
  **Revision 6 said "neither binary contains signal-handling code";
  that is false.** The `pmacs` binary *does* — `install_signal_handlers`
  (`src/daemon.rs:628`) registers `SIGINT` and `SIGTERM` — it simply is
  not on `run_gpu`'s path. And a grep of project sources cannot exclude
  a runtime or dependency installing a disposition. So the established
  fact is narrow: **no explicit installation on `run_gpu`'s path**.
- **The probe's event loop wakes at least every 50 ms.**
  `run_headless_managed_probe` (`pmacs-gpu/src/main.rs:1065`) loops on
  `event_rx.recv_timeout(Duration::from_millis(50))`. **Revision 2 said
  "never blocks indefinitely", which is false**: the probe's stdin
  reader thread blocks in `read_to_end` (`:1109`) with no timeout, and
  once `ready` the loop has **no deadline of its own** — it leaves only
  when stdin closes (`:1212`). So the process is not bounded; only the
  event wakeup is.
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
candidate precisely because `run_gpu`'s path is silent. But **"whatever
disposition they hold was inherited" is a hypothesis, not a finding** —
revision 6 stated it as established, which it is not, since neither a
source grep nor an absent call proves what the runtime disposition is.
D2 measures it. Until then it is one candidate among the three D1/D2
are built to separate.

## 4. Reductions attempted

**Full provenance lives in `docs/probe-sigint-evidence.md`**, which is
pushed with this branch: exact command, worktree, HEAD, cleanliness,
the Cargo suffixes actually executed, result, and log digest for every
physical run. Log bodies stay machine-local under
`/home/jeans/build/pmacs-gate-targets/probe-sigint-evidence/` — `/tmp`
is a tmpfs and they were nearly lost to a cleanup mid-lane.

Three provenance caveats are recorded there rather than smoothed over:
**R1 and R2 have no preserved log** (revision 2 cited `gpu3.log` for
both R2 and R6; that log is R6's three-suite run alone, and counting
one run as two was wrong); **cleanliness is `UNKNOWN` for every
pre-manifest run**, because it was not recorded at the time and is not
inferrable; and **R1–R10 ran in the `panel-mapping-generation`
worktree**, not at `main`. `D0` re-runs the matrix under a harness that
captures all of it, at `main`, before any row here is relied on.

All rows carry `--features crdt`. Full argv, worktree, HEAD,
cleanliness and Cargo suffixes per run: `docs/probe-sigint-evidence.md`.

| # | reduction (after `cargo test`) | runs | result | log |
|---|---|---|---|---|
| R1 | `--test gpu_invocation_acceptance ctrl_c_on_launcher_group` | 3 | green, 0.15–0.17 s | **no log preserved** |
| R2 | `--test gpu_invocation_acceptance` (whole suite) | 1 | green, 15 passed | **no log preserved** |
| R3 | `--workspace --no-fail-fast -- --skip basedpyright ctrl_c_on_launcher_group` | 1 | green — every binary runs, only this test executes | `filtered.log` |
| R4 | `--lib --test gpu_invocation_acceptance --no-fail-fast` | 1 | green, 2145 + 15 | `two.log` |
| R5 | `--test gate_script_acceptance --test gpu_invocation_acceptance` | 1 | green | `suspect.log` |
| R6 | `--test gpu_font_acceptance --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | 1 | green | `gpu3.log` |
| R7 | `--lib --bins` + `--test`×14 (targets 6–19) + the suite | 1 | green | `half1.log` |
| R8 | `--test`×18 (targets 20–37) + the suite | 1 | green | `half2.log` |
| R9 | `--lib --bins` + `--test`×32 (targets 6–37) + the suite | 1 | green (`-91f51d0b`, `-6b4b8223`) | `prefix.log` |
| R10 | `--workspace ... --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | 1 | green (`-5d9105cb`, `-d4dae4f0`) | `wsonly.log` |
| F1–F7 | full `--workspace --no-fail-fast -- --skip basedpyright`, plus three gate `sweep-crdt` stages | **7** | **red, 7/7** | `base-sweep.log` (at `72da24a`), `postclean.log`, `sweep-inst.log`, `sweep-diag.log`, gates `…-1977433`, `…-2144707`, `…-2375685` |

**Correction to revision 2: R9 did not run the same compilations.** It
executed `gpu_invocation_acceptance-6b4b8223dea45247`; the failing
sweeps executed `-d4dae4f01bcdef62`. **Differing Cargo suffixes mean
Cargo computed different metadata hashes — different compilations.**
Revision 4 went further and called them "byte-different"; that is
**withdrawn**, because the bytes a historical run executed are not
knowable now — target directories have been overwritten, and a hash
computed today is the hash of the current occupant. The weaker claim is
sufficient: R9 establishes **same target names and order**, not same
binaries.

What the evidence is **consistent with** is an interaction. It does not
isolate one, because the rows differ in more than the two columns shown
— different source heads, different worktrees, unknown cleanliness, and
different Cargo compilations:

| prior targets execute | compilation set | result |
|---|---|---|
| yes | `-91f51d0b` + `-6b4b8223` (`prefix.log:3066`, `:3087`) | R9 green |
| no | `-5d9105cb` + `-d4dae4f0` (`wsonly.log:3`, `:24`) | R10 green |
| yes | `-5d9105cb` + `-d4dae4f0`, all seven | **F1–F7 red (7)** |

Neither factor alone reproduced it **in these runs**. That is the
whole of the claim. `--workspace` artifact selection is **not
sufficient by itself and not ruled out**; later-selected packages can
influence Cargo's build graph and fingerprints *before* their test
executables run, so "their targets execute after the failure at line
3066" does not exonerate them — that claim is withdrawn. And since
§4's own preamble says no historical row should be relied on until D0,
**this table is a description of what was observed, not a finding**.
Revision 3 asserted it as an interaction while simultaneously
disclaiming its inputs, which cannot both be true.

Also refuted, by measurement: machine load (red on a quiet box, load
2.77); tmpfs starving RAM (**tested by experiment** — `/tmp` 21 G →
1.2 G, available 27 G → 45 G, still red); leaked daemons (peak 58, +8
per sweep, green runs already at 46–60); inotify (47 of 1024).

## 4a. The onset is datable — and it reframes the lane

`sweep-crdt` appears **17 times** in this target directory's gate logs.
`ctrl_c` fails in **exactly the last three**, and passed — both copies,
`... ok` — in the runs before them.

Counted **per test copy** across the 17 `sweep-crdt` logs:

| outcome | runs |
|---|---|
| both copies `... ok` | **13** |
| **neither copy executed** — stage died compiling `pmacs` (`error[E0308]`), `…-708693` | **1** |
| both copies `FAILED` | **3** (`…-1977433`, `…-2144707`, `…-2375685`) |

Revision 4 said "14 runs, 11 green, 3 red on other tests" and that the
earlier reds failed on unrelated tests. **Both wrong.** The count is
13 / 1 / 3, and one earlier non-passing run is a **compile failure that
never reached either copy**. The two genuinely red-on-other-tests
sweeps did execute `ctrl_c`, and it passed.

**So the failure is not long-standing.** "Pre-existing on `main`"
remains true — F1 at `72da24a` reproduces it — but "always broken" was
never established and is now contradicted. Last green containing it:
`20260815T185708Z`. First red: `20260816T063330Z`. The machine was not
rebooted across that boundary.

**But the onset is NOT a source boundary, and a Git bisect is not yet
justified.** Reflog and commit times put HEAD at `7599661` during the
last green — `3c06176` was committed 40 s after that run finished — and
at `724b785` during the first red, since `5174f73` landed at 08:45:41,
after that run ended at 08:42:01. **Cleanliness was captured for
neither**, and the tree was under active edit throughout. So the window
dates a **machine/worktree-state transition**, not two clean revisions.

One further relationship is worth recording **only to say what it
cannot support**: `72da24a` is an **ancestor** of `7599661` (verified
by `git merge-base --is-ancestor`), yet `72da24a` fails today while
`7599661` passed on 08-15. **These two observations are
non-comparable** — they differ in commit *and* in environment *and* in
time — so **no causal conclusion of any kind may be drawn from the
pair**.

Revision 6 read it as "outcome is not determined by commit alone".
**That is withdrawn too**: different commits can deterministically
produce different outcomes, and this document's own fix-then-regression
scenario is an example. The pair supports nothing about determinism
either way.

**Revision 5 drew still more from it.** It said a source cause was
"positively discouraged", that the ancestry "says to expect" equal
endpoints, and that "whatever changed is environmental, cached, or
uncommitted". None of that follows either. Nothing in the pair
distinguishes:

- an environmental change;
- a source/environment interaction; or
- a source fix landing before `7599661` and a regression landing before
  `724b785`.

And an older ancestor outside the interval behaving badly is simply
**irrelevant** to whether `7599661..724b785` contains a regression: a
bisect over that interval needs only that the two clean endpoints
differ *now*.

**This still supersedes the reduction matrix as the lane's first
move**, as endpoint reproduction — which is a decision procedure, not a
prediction.

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
reason**: under `--features crdt` the suite spawns root launchers from
**six** call sites — `:509, :534, :544, :574, :725, :1097`, all inside
`#[cfg(feature = "crdt")] mod crdt` (`:88`). Eight `--gpu` arguments
appear in the file, but `:38` and `:65` sit under
`#[cfg(not(feature = "crdt"))]` (`:26`) and are compiled out of the
failing configuration. Revision 2 said "five" while citing eight, which
was wrong twice over. Six is the number; what matters is that it is
more than one, so a launcher captured by command line alone cannot be
attributed to *this* test. The `do_wait` /
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

- **D0a — reproduce the onset endpoints CLEANLY** (§4a): `7599661`
  (last observed green) and `724b785` (first observed red), each
  checked out clean, each in its own isolated target directory. This is
  a **decision procedure with no predicted outcome**. One run per
  endpoint decides nothing — this failure is context-sensitive by
  construction, appearing only in the full sweep — so the procedure is
  specified rather than left to judgement:
  - **N = 5 full `sweep-crdt` runs per endpoint**, since the observed
    failure rate in the reproducing configuration is 7/7 and the
    passing configuration 13/13; anything less cannot separate a real
    difference from the intermittency that has not yet been excluded.
  - **Interleaved**, alternating endpoints A/B/A/B…, so any drift in
    machine state across the session hits both arms equally instead of
    landing entirely on whichever ran second.
  - **Identical captured conditions per run**: same harness as D0b —
    argv, worktree, `git rev-parse HEAD`, `git status --porcelain`
    emptiness, the Cargo suffixes executed, result, log digest — plus
    the machine facts that have already misled this lane once
    (`uptime`, `free`, `/tmp` usage, leaked-daemon count).
  - **Permitting a bisect requires a clean split**: all N of one
    endpoint red and all N of the other green. A mixed result means the
    failure is intermittent under fixed source, and **no bisect is
    justified at all** — that outcome sends the lane back to D1/D2.
- **D0b — re-run the §4 matrix with captured provenance**, at `main`,
  recording the artifact hashes actually executed **at run time**.
  Revision 2's strongest claim collapsed because command shape silently
  changed the binary; no further reduction should be trusted until each
  row names the executable it ran.
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
- **A3.** There is no established "R9 paradox" to explain — R9 ran
  different Cargo compilations, so the comparison it appeared to make
  was never made. What A3 requires instead: **D0 recreates the subset/full
  comparison under captured provenance**, and whatever it then shows is
  either explained by the fix or explicitly recorded as unexplained. A
  fix that greens the sweep without that comparison having been made
  properly leaves the gap stated, not hidden.
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

## 11. Record corrections owed to other ledgers

A correction is not made until it is portable. Two were outstanding
when revision 2 was reviewed, and both are closed by this revision:

- **This branch's ledger** asserted that two default-disposition
  processes "should both die at once" and then withdrew that same claim
  further down. The assertion is removed; only the withdrawal and its
  reasoning remain.
- **`panel-mapping-generation`** carried "119 binaries green, one
  red", the ≥8 s arithmetic, the "default action" claim and the ">6 s
  selector"; `779a6bd` corrected those. **It still carried more**,
  found on re-review: `--workspace` unification "refuted", R9 running
  the "same binaries", later packages that "cannot be implicated", and
  a cause "cumulative across the preceding 37 binaries". Revision 3's
  claim here that the held lane no longer transports falsified claims
  was **premature**; those are corrected now, and this section should
  be read as a checklist that has been re-verified rather than an
  assurance.
