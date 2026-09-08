# GPU launcher / probe SIGINT teardown — framing

Revision 13. Status: **APPROVED 2026-08-19 at `5dece3e` — the shipped
ABI is defective on macOS (§4d), and revision 13 replaces it.
Implementation may proceed under §8's A1–A8 contract.**

Revision 12 was approved at `1fc0df6` and implemented; CI then found its
status-only ABI unsound on a platform this session could not reach.
Mechanism in §4c; the ABI defect and its remedy in §4d and §7c. Still no
product change — this remains gate/test correctness only.

Revision 10 was approved 2026-08-19 at `4fba9f6`, authorising
diagnostic-only D1/D2. They ran, and found the mechanism on the first
reproducing sweep. It is not what this document was built around: bet 1
is **withdrawn by scope** and A5 **retired by scope** (D4 was never
executed, so no claim is made about a real session either way), and
nothing in the evidence implicates the probe's shutdown path.

Revision 9 was approved 2026-08-19 at `15c25ec`. **That approval did
not extend to revision 10**, because retiring D0b (§7) materially
changed the approved diagnostic sequence — the revision 9 text made
D0b mandatory before every other diagnostic. Revision 10's approval
covers that retirement and the A3 contingency that preserves its
obligation. **D1/D2 have since been executed under it and found the
mechanism (§4c).**

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
- r9 → r10: **D0b retired as a precondition** (§7), which changes the
  approved sequence and is why this revision needs its own approval;
  D0a executed and its causal conclusion narrowed twice — "source
  hypothesis eliminated" withdrawn in favour of "the commits do not
  discriminate under current conditions" (§4b, and the endpoint
  table's two uniform-same rows); portable provenance corrected after
  it corrupted every log digest and silently dropped `/tmp` and
  `MemAvailable`; and `uptime` recorded as **UNKNOWN**, since §7 names
  it but the harness kept only the load averages.
- r8 → r9: D0a's classifier was not total — it named only "clean
  split" and "mixed", leaving both-green, both-red, non-execution,
  copy-disagreement and unrelated-failure outcomes unprescribed, all of
  which occur in the historical logs (§7); and strict A/B/A/B does not
  equalise drift (§7).
- r7 → r8: the superseded one-run D0 rule survived in three places
  (§4a, §7, manifest, ledger); D0a still overstated its rates and left
  the bisect's own classifier unspecified (§7); residual artifact
  wording and four wrong suffix attributions (§4, manifest).
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
sufficient: R9 establishes **same target names and order**, not the
same compilations.

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
**irrelevant** to whether `7599661..724b785` contains a regression. The
endpoints, under the N = 5 contract in §7 D0a, settle only **whether a
bisect is currently justified** — not whether the interval contains a
regression. Those are different questions, and D0a's outcome
(both-uniform-red) answers the first and leaves the second open. (Revision 6 wrote "needs only that the
two clean endpoints differ *now*", which is the superseded one-run
rule; a bare difference decides nothing.)

**This still supersedes the reduction matrix as the lane's first
move**, as endpoint reproduction — which is a decision procedure, not a
prediction.

## 4b. D0a result — the commits do not discriminate today

Executed 2026-08-19 under the approved contract **with one departure,
stated up front**: the contract's captured-conditions list names
`uptime`, and the harness kept only the load averages from it. `uptime`
is therefore `UNKNOWN` for all ten runs. Everything else held — 10 runs,
counterbalanced `A B B A A B B A A B`, N = 5 per endpoint, clean
detached worktrees, isolated target directories, `dirty=0` verified per
run, **zero voids, zero splits** — and no classification depends on the
missing field, so the verdict stands. D1/D2's harness must capture the
full list.

**A (`7599661`) uniform-red. B (`724b785`) uniform-red.** By the
endpoint table this is *both endpoints uniform the same way*: the
difference is **not captured by those two commits**.

- **No bisect of `7599661..724b785` is justified UNDER CURRENT
  CONDITIONS**, and none will run. `7599661` passed inside `sweep-crdt`
  on 08-15 and fails 5/5 clean today, so the two commits **do not
  discriminate now**.
- **That is the entire causal claim.** Earlier wording here — "the
  source hypothesis is eliminated", "the interval cannot contain the
  transition", "not reachable by source" — is **withdrawn**.
  Uniform-red today says nothing about 08-15: a historical source
  regression could be **masked** by a later environmental effect, or by
  a source/environment interaction under which both commits now fail.
  **Failing to discriminate is not the same as not differing.**
- **The onset window is deprioritised, not excluded.**
- **A reliable reproduction now exists** — 10/10 today across two
  commits, ~4 minutes per run. This is D0a's most useful product:
  **D1/D2 no longer wait on a rare event.**
- One cheap negative on "what else changed": **no package activity in
  the window** (`/var/log/pacman.log`, 08-15 19:57 → 08-16 06:33;
  nearest 08-18). Not pursued further — with a reproduction in hand,
  direct measurement dominates archaeology.

Per-run provenance: `docs/probe-sigint-evidence.md` §D0a.

## 4c. D1/D2 RESULT — the mechanism, and it is my own artifact

**`SIGINT` was ignored by every process in the target group, because I
launched the test runner in the background.**

Captured at the moment of the test's own `kill`:

```
test parent  pid 8252                       SigIgn=0000000000001007
launcher     pid 8281 ppid=8252 pgid=8281   SigIgn=0000000000001007
                                            SigCgt=0000000000000440  wchan=do_wait
probe        pid 8284 ppid=8281 pgid=8281   SigIgn=0000000000001007
```

`SigIgn=0x1007` is signals 1, 2, 3 and 13 — and **signal 2 is
`SIGINT`**. The launcher's `SigCgt=0x440` is signals 7 and 11 only,
Rust's SIGBUS/SIGSEGV handlers; there is no `SIGINT` handler anywhere.
Every `SigPnd`/`ShdPnd` is zero and every per-thread `SigBlk` is zero,
so this is **ignored** delivery, not **blocked** delivery. Launcher and
probe share `pgid=8281`, so nothing escaped the group either. All three
candidates D1/D2 was built to separate are thereby separated.

`kill(-pgid, SIGINT)` is a **no-op for every member**. The launcher
waits in `do_wait` for a child that was never told to stop, and the 5 s
deadline fires.

### Where the ignore comes from — measured in both directions

| invocation | child's `SigIgn` | `SIGINT` |
|---|---|---|
| foreground | `0000000000001000` | bit 12 only (SIGPIPE) — **deliverable** |
| `setsid nohup … &` | `0000000000000007` | SIGHUP, SIGINT, SIGQUIT — **ignored** |

A shell running a command in the background without job control sets
`SIGINT`/`SIGQUIT` to `SIG_IGN` in the child; `nohup` adds `SIGHUP`.
**`SIG_IGN` is inherited across `fork` and survives `exec`**, so it
propagates shell → `cargo` → test binary → launcher → probe.

### The controlled experiment

Same command, same tree, same target directory, minutes apart — only
the invocation differs:

| arm | invocation | both target copies |
|---|---|---|
| 1 | foreground | **ok** |
| 2 | `setsid nohup … &` | **FAILED** |

### This invalidates most of this lane's investigation, and I caused it

I adopted `setsid nohup … &` on 08-16 to stop the Bash tool's
ten-minute cap truncating gate runs. **That is the "onset".**

- **The subset-vs-full distinction was never real.** Every reduction I
  ran was foreground; every full sweep was backgrounded. The two
  variables were perfectly confounded, so §4's matrix measured my
  invocation method rather than the code, and R9's "paradox" dissolves.
- **The 08-15 → 08-16 window** dates my method change, not the machine
  and not the source.
- **D0a's both-uniform-red is consistent and was right** — its harness
  backgrounded both arms, so both were red; the cause was invisible to
  a comparison in which it did not vary.
- **"Pre-existing on `main`" is true but trivial**: `main` fails the
  same way backgrounded and passes foreground.

### Consequences for the contract

- **Bet 1 is WITHDRAWN BY SCOPE; A5 is RETIRED BY SCOPE.** D4 was
  never executed, so **nothing here establishes that a real
  `pmacs --gpu` session behaves correctly** — only that no observed
  evidence of a user-facing defect survives, every red run being
  explained by the runner's invocation. Any user-facing claim needs its
  own lane and its own evidence.
- **The §7/§8 remedy no longer follows.** What remains is narrower and
  genuinely real:
  1. **The gate must not be invoked so that `SIGINT` is ignored** — a
     runner practice, and the direct cause of all seven red sweeps.
  2. **The test should not fail obscurely when its precondition is
     absent.** "child did not exit within 5s" sent this lane chasing a
     teardown defect for nine revisions. It should detect an ignored
     `SIGINT` and say so. Silently skipping is not acceptable —
     `scripts/gate`'s own comments record that self-skipping tests
     "void coverage silently".
- **A3's subset/full obligation is discharged by explanation**, not by
  D0b: the difference was invocation mode, demonstrated in both
  directions.

`#![forbid(unsafe_code)]` rules out `pre_exec` as one *mechanism*; it
does **not** select the remedy, and revision 11's leap from the first to
the second did not follow. §7b weighs the candidates and §7c records
the decision.

## 4d. The shipped ABI is defective on macOS — found by CI

`70f0bc9`, `Test (macos-latest / lua54)` and `… / luajit`, one row:

```
scripts/gate: line 543: .../check-sigint-deliverable: Permission denied
gate: no stage has run; this is not a test failure.
  left: Some(1)   right: Some(2)
```

**macOS `/bin/sh` returns 1 when it cannot execute a file; Linux
returns 126.** The gate took its `1 | 2)` branch — identifiable only
because that branch's message text differs, since it withholds the
number — so a helper that never ran was classified **`ignored`**. A
broken guard told the operator their environment ignores `SIGINT`.

That is the conflation §7c forbids, reached by a route §7c did not
anticipate. Five of six SIGINT rows pass on macOS; this is the sixth.

**A7 earned its keep here.** It was satisfied *by disclosure* precisely
because the POSIX-portability claim was argued rather than measured.
The first time it was measured, the claim was wrong in a specific,
narrow way — which is the outcome a disclosed-but-untested criterion
exists to make visible.

**A rejected repair, recorded so it is not retried.** Moving `ignored`
from 1 to 3 was proposed and refused: it relocates the collision
without closing it, because an execution failure can return **any**
nonzero status. The generalisation is the useful part —
**no exit status can prove the helper ran.**

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

1. ~~The failure is a real teardown defect a user meets.~~
   **WITHDRAWN BY SCOPE — not falsified.** Every observed red run is
   explained by inherited `SIG_IGN` from a background invocation
   (§4c), so **no observed evidence of a user-facing defect remains**.
   That is weaker than proving a real wgpu session is correct, and
   **D4 was never executed** (§7), so the correct statement is: this
   lane is now **gate/test correctness only**, and any user-facing
   claim is out of its scope and unevidenced in both directions.
2. **UPHELD.** It is **not** a timing margin — confirmed twice over: a
   green run finishes in ~0.19 s against a 5 s deadline, and the
   foreground arm passes while the background arm fails with the same
   binaries.
3. **UPHELD, and now load-bearing.** Raising the deadline is not a fix
   and remains out of scope: the signal is never delivered, so no
   deadline is long enough.

## 7. First step — diagnostics keyed on identity, not age

**EXECUTED. The mechanism is known (§4c): inherited `SIG_IGN`.** This
section is kept as the record of what was run. D1/D2 discriminated the
three candidates — ignored rather than blocked delivery (`SigPnd` and
per-thread `SigBlk` all zero), and no escape from the group (shared
`pgid`). D3 is discharged by the controlled arms. **D4 was NOT
executed**, and bet 1 is withdrawn by scope rather than falsified.

As written, the step read: the first commit is diagnostic only, and it
must **discriminate** the three live candidates: blocked delivery,
inherited ignore, and an escaped or wrong process
group.

- **D0a — reproduce the onset endpoints CLEANLY** (§4a): `7599661`
  (last observed green) and `724b785` (first observed red), each
  checked out clean, each in its own isolated target directory. This is
  a **decision procedure with no predicted outcome**. One run per
  endpoint decides nothing: **so far** the failure has been observed
  only in the full sweep, which is a statement about what has been run,
  not a property established of the defect. The procedure is therefore
  specified rather than left to judgement:
  - **N = 5 full `sweep-crdt` runs per endpoint.** Five is a
    **predefined evidentiary threshold, chosen in advance so the
    outcome cannot be argued after the fact** — it does not
    "mathematically separate" anything. The historical 7/7 red and
    13/13 green are **not endpoint-specific rates** and must not be
    read as such: of the seven reds only F6 ran at `724b785`, and of
    the greens only the last ran at `7599661`, both with **unknown
    cleanliness**.
  - **Counterbalanced order**, not strict alternation. Runs go in
    `AB BA AB BA AB` pairs, so neither endpoint systematically follows
    the other. Revision 8 claimed strict `A/B/A/B…` makes session drift
    "hit both arms equally"; **it does not** — under strict
    alternation B always follows A and owns the final time point. What
    counterbalancing buys is the removal of *systematic order
    confounding*; with an even run count one arm still holds the last
    slot, and that residue is accepted and stated rather than papered
    over.
  - **Identical captured conditions per run** — and D0a satisfied this
    list only **partially**: it captured everything below except
    `uptime`, keeping the load averages and discarding elapsed time.
    D1/D2's harness must capture the whole list. Same harness as D0b —
    argv, worktree, `git rev-parse HEAD`, `git status --porcelain`
    emptiness, the Cargo suffixes executed, result, log digest — plus
    the machine facts that have already misled this lane once
    (`uptime`, `free`, `/tmp` usage, leaked-daemon count).

  **Classifying a single run.** The unit is *the two copies of the
  target test* — `crdt::ctrl_c_…` and
  `gpu_invocation_acceptance::crdt::ctrl_c_…` — and nothing else in the
  sweep:

  | run outcome | definition |
  |---|---|
  | **green** | both copies executed and both `... ok` |
  | **red** | both copies executed and both `FAILED` |
  | **split** | both executed, copies **disagree** |
  | **void** | either copy **did not execute** |

  Two of these are not hypothetical. `20260815T182846Z-708693` is a
  **void**: the stage died compiling `pmacs` (`error[E0308]`) and
  neither copy ran. And sweeps red on *unrelated* rows are ordinary —
  `…-2839374` and `…-830195` both failed other tests while both target
  copies passed. **A sweep red only on unrelated tests is a `green`
  run** under this classifier, because the classifier reads the two
  copies and nothing else. Unrelated failures are still recorded, as
  evidence about environment stability.

  **Handling each outcome:**

  - **void** — discard and re-run, up to **3 voids across the whole
    procedure**. Beyond that the environment is too unstable to
    classify anything and D0a **stops**; that is itself the finding.
  - **split** — **stop immediately.** Two copies of the same source in
    different binaries disagreeing within one run is a distinct defect,
    and characterising it takes priority over the endpoint question.

  **Endpoint verdicts**, from 5 valid (non-void) runs each: *uniform
  green* (5/5), *uniform red* (5/5), or **mixed** (anything else).

  | `7599661` | `724b785` | conclusion |
  |---|---|---|
  | uniform green | uniform red | **clean split → bisect `7599661..724b785` permitted** |
  | uniform red | uniform green | clean split, **direction inverted** — a real difference, but it falsifies which endpoint was believed good; record loudly and re-examine the onset reading before bisecting |
  | uniform green | uniform green | **the commits do not discriminate under current conditions** → no bisect now; ask what else changed across the window. This does **not** exclude a source difference that current conditions mask |
  | uniform red | uniform red | **the commits do not discriminate under current conditions** → same. A historical regression masked by a later environmental effect, or a source/environment interaction, remains possible |
  | mixed | any | **intermittent under fixed source → no bisect**; back to D1/D2 |
  | any | mixed | as above |

  - **Permitting a bisect requires the clean-split row.** Every other
    row forbids one.
  - **The bisect itself uses the same classifier.** Every intermediate
    commit is classified by the identical N = 5 protocol under the same
    captured conditions; a commit that classifies **mixed** — or
    produces a **split**, or exceeds the void budget — **aborts the
    bisect** rather than being guessed, skipped, or rerun until it
    agrees. A bisect whose steps are cheaper than its endpoints would
    inherit exactly the weakness this contract exists to remove.

- **D0b — RETIRED as a precondition on 2026-08-19, kept as a
  contingency.** It existed to make the §4 reduction matrix trustworthy
  so the subset-vs-full comparison could locate the mechanism
  *indirectly*. D0a has since produced a **reliable direct
  reproduction** (10/10 across two commits, ~4 min/run), and D1/D2
  measure the mechanism itself. Sharpening an indirect instrument while
  a direct one is in hand is the wrong order of work.

  **The obligation is now SATISFIED, by explanation rather than by
  running D0b.** A3 asked that the subset/full difference be accounted
  for: §4c accounts for it — every subset ran foreground and every full
  sweep backgrounded, and the controlled arms demonstrate the
  difference in both directions with byte-identical binaries. **D0b is
  therefore not owed and will not run.**

  As written, the retired step read: re-run the §4 matrix, at `main`,
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
- **D4 — NOT EXECUTED.** It would have established whether a real
  `pmacs --gpu` session, rather than the wrapper/headless probe,
  reproduces the hang. It is **not run and not needed**, because bet 1
  is withdrawn by scope: with every observed failure explained by the
  runner's invocation, there is no user-facing claim left for this lane
  to make. **A5 is retired by scope, not falsified** — nothing here
  demonstrates a real session behaves correctly, only that no evidence
  of the contrary survives.

## 7b. Remedy options — revision 11 evaluation, superseded by §7c

Revision 11 jumped from "`pre_exec` is `unsafe`" to "therefore a
precondition assertion". That does not follow: ruling out one mechanism
does not select another. Four candidates, with the trade-off that
decides each:

| # | remedy | effect | cost / risk |
|---|---|---|---|
| R-a | **Runner normalisation** — never invoke the gate so that `SIGINT` is ignored; if backgrounding is needed, restore the disposition first | removes the cause for every test at once | a *practice*, not a mechanism: nothing enforces it, and this lane exists because I violated it silently |
| R-b | **Early gate guard** — `scripts/gate` refuses to start when `SIGINT` is `SIG_IGN`, naming the reason | enforces R-a mechanically, once, for all suites | refuses runs that would mostly have succeeded. *This row originally added "needs an explicit override for deliberate background use"; §7c rejects that — see there* |
| R-c | **Fixture isolation** — the test restores the default disposition in the spawned launcher | fixes the test wherever it runs, background included | `pre_exec` is `unsafe`, and `#![forbid(unsafe_code)]` binds the lib crate; an integration test could technically opt out, but doing so to dodge a project invariant needs its own argument |
| R-d | **Test-local precondition assertion** — detect `SIG_IGN` on `SIGINT` and fail with that reason instead of "child did not exit within 5s" | converts nine revisions of misdirection into one accurate line | does not make the test *pass* when backgrounded; it only stops it lying about why it failed |

**They are not exclusive**, and the likely answer is R-b + R-d: a guard
that stops the whole gate from running in a state where several suites
are meaningless, plus a test that explains itself if it is ever reached
that way. R-a alone is what already failed. R-c is the only one that
makes the test genuinely invocation-independent, and it is also the one
that touches an `unsafe` prohibition, so it needs a separate decision.

**Portability is a selection criterion, not an afterthought.** Reading
`SigIgn` from `/proc/self/status` is Linux-only. The suite is already
`#![cfg(unix)]`, so a `/proc`-based guard would narrow it further to
Linux; `sigaction`-based querying would be portable but is `unsafe`.
Whichever is chosen must state what it does on a non-`/proc` unix.

Revision 11 implemented no remedy and selected none. Section 7c records
the later selection that governs implementation.

## 7c. Remedy — SELECTED: R-b + R-d, via one portable probe

**Decided 2026-08-19.** R-b (early gate guard) and R-d (test-local
precondition diagnosis), implemented through a **single checked-in
helper** built on a behavioural probe rather than `/proc` or `unsafe`:

```sh
sh -c 'trap "exit 23" 2; kill -INT $$; exit 0'
```

It sends itself `SIGINT`. If the signal is deliverable the trap runs and
it exits **23**; if `SIGINT` was inherited as `SIG_IGN` the kill is a
no-op and it exits **0**. Verified in both contexts: 23 foreground, 0
under `setsid nohup … &`.

**The raw probe is not a total classifier, and must not be used as
one.** Its `exit 0` is reached both when the `kill` was a no-op *and*
when the `kill` itself failed — a failed `kill` overwrites its own
status and lands on the same exit. Read naively, a broken probe reports
"inherited `SIG_IGN`", which would fail the gate for the wrong reason
and send the next reader down this lane again.

So the **helper owns the classification**. The checked-in executable is
`scripts/check-sigint-deliverable`; this is its complete interface:

**Revision 13 replaces the status-only ABI with a validated
`(status, token)` pair.** The interface is:

| outcome | exit | token on **stdout** |
|---|---|---|
| `safe` | 0 | `pmacs-sigint-v1:safe` |
| `ignored` | 1 | `pmacs-sigint-v1:ignored` |
| `error` | 2 | `pmacs-sigint-v1:error` |

**Any other pair is a boundary error, mapped to 2** — including a
correct-looking status with no token, a token that does not match its
status, an unknown token, or a status outside 0–2. Diagnostics stay on
**stderr**; the token is the only thing on stdout, so parsing it cannot
be confused by human-readable text.

**Why the token, and why the earlier design was wrong.** Revision 12's
ABI carried the verdict in the exit status alone. CI proved that
insufficient: on macOS a shell that cannot execute the helper exits
**1**, which the ABI already reads as `ignored`, so a broken guard told
the operator their environment ignores `SIGINT`. The first proposed
repair — move `ignored` to 3 — was **rejected, correctly**: it only
relocates the collision, because an execution failure can return *any*
nonzero status. **No exit status can prove the helper ran.** A token it
must have printed can.

Consumers therefore validate the exact pair and treat every mismatch as
`error`. They still do not re-derive the classification: the helper
decides, and the pair is what makes the helper's decision
distinguishable from a shell's.

Its complete POSIX-shell shape. Each arm emits **exactly one token on
stdout** and its diagnostic on stderr, so a status is never the only
thing a consumer sees:

```sh
probe_status=0
sh -c 'trap "exit 23" 2 || exit 24; kill -INT "$$" || exit 24; exit 0' \
  || probe_status=$?
case "$probe_status" in
  23)
    echo 'pmacs-sigint-v1:safe'
    exit 0
    ;;
  0)
    echo 'pmacs-sigint-v1:ignored'
    echo 'pmacs: SIGINT is ignored; run this command with SIGINT deliverable' >&2
    exit 1
    ;;
  *)
    echo 'pmacs-sigint-v1:error'
    echo "pmacs: could not determine whether SIGINT is deliverable (probe status $probe_status)" >&2
    exit 2
    ;;
esac
```

The helper maps inner 23 → `(0, safe)`, inner 0 → `(1, ignored)`, and
every other status → `(2, error)`. Consumers do not parse the inner
23/0/24 statuses and do not supply their own signal diagnosis.

**The consumer flow is pair-validation, not status inspection:**

1. Run the helper, capturing **status**, **stdout** and **stderr**
   separately. The two process boundaries differ and the contract keeps
   that difference explicit: the shell consumer always receives a shell
   status, including when `exec` fails; Rust's `Command` instead returns a
   spawn error with **no status to inspect at all**. Either route can
   produce boundary `error`, but they are not the same input.
2. **Compare stdout as BYTES against an exact grammar. There is no
   trimming.**

   ```
   stdout := TOKEN | TOKEN LF
   TOKEN  := "pmacs-sigint-v1:" ("safe" | "ignored" | "error")
   LF     := 0x0A
   ```

   Nothing else validates: not leading whitespace, not a second
   newline, not CR, not interior or trailing spaces, not empty.

   **Revision 13's first grammar was unimplementable identically.** It
   said "strip one trailing newline, then trim ASCII whitespace" — but
   trimming removes *further* newlines, so `TOKEN\n\n` would have
   validated while the same clause demanded single-line output. Worse,
   POSIX command substitution `$(cmd)` strips **all** trailing
   newlines while Rust's `Command` returns raw bytes, so the two
   consumers could not have agreed even on a correct rule.

   **Variable capture cannot implement this, for two independent
   reasons — both measured, not reasoned:**

   - **It destroys the status.** `out=$("$helper"; printf x)` returns
     `printf`'s status, not the helper's: a helper exiting 1 yields
     assignment status **0**. Revision 13 specified exactly this idiom.
   - **It is not byte-preserving, and differs by shell.** Command
     substitution drops NUL in POSIX `sh`/bash; **zsh keeps it**
     (verified). So `TOKEN NUL` validates in one shell and not another
     — a contract two consumers cannot implement identically.

   **The shell consumer therefore captures to files and compares bytes.**
   Rust already receives byte vectors from `Command::output()` and compares
   those directly; it does not need or create capture files.

   ```sh
   # The guard runs BEFORE the gate's own temporary roots exist, so it
   # creates and owns its capture directory --- and arms the cleanup
   # BEFORE the helper can be invoked, so no path can leave residue.
   capture=$(mktemp -d "${TMPDIR:-/tmp}/pmacs-sigint.XXXXXX") || {
       echo 'gate: could not create the SIGINT guard capture directory (status=unavailable token=missing)' >&2
       exit 2
   }
   cleanup_sigint_capture() { rm -rf "$capture"; }
   trap cleanup_sigint_capture EXIT HUP INT TERM

   # `|| status=$?` IS LOAD-BEARING under `set -eu`: a bare invocation
   # dies at the helper's non-zero exit and never reaches the
   # assignment. This is the original shipped bug, and an earlier draft
   # of THIS SECTION reintroduced it.
   status=0
   "$helper" >"$capture/out" 2>"$capture/err" || status=$?

   # Select an expected token only for public helper statuses. This case
   # MUST precede any use of expected_token: the gate runs under `set -u`,
   # and an out-of-range shell status has no expected token.
   expected_token=
   case "$status" in
       0) expected_token=pmacs-sigint-v1:safe ;;
       1) expected_token=pmacs-sigint-v1:ignored ;;
       2) expected_token=pmacs-sigint-v1:error ;;
   esac

   token_ok=0
   if [ -n "$expected_token" ]; then
       printf '%s'   "$expected_token" >"$capture/want"
       printf '%s\n' "$expected_token" >"$capture/want_lf"
       if cmp -s "$capture/out" "$capture/want" \
          || cmp -s "$capture/out" "$capture/want_lf"
       then token_ok=1; fi
   fi

   if [ ! -s "$capture/out" ]; then
       token_state=missing
   elif [ "$token_ok" -eq 1 ]; then
       token_state=valid
   else
       token_state=unexpected
   fi

   case "$status:$token_ok" in
       0:1)
           # SAFE is the sole continuing path. Remove the guard-local
           # directory and disarm its trap BEFORE the gate installs its
           # later, unrelated cleanup trap.
           cleanup_sigint_capture
           trap - EXIT HUP INT TERM
           ;;
       1:1)
           cat "$capture/err" >&2
           printf 'gate: SIGINT guard status=1 token=valid\n' >&2
           exit 1
           ;;
       2:1)
           cat "$capture/err" >&2
           printf 'gate: SIGINT guard status=2 token=valid\n' >&2
           exit 2
           ;;
       *)
           # The captured stderr is untrusted here and is not surfaced as
           # the diagnosis. EXIT runs cleanup_sigint_capture.
           printf 'gate: SIGINT guard boundary error (status=%s token=%s)\n' \
               "$status" "$token_state" >&2
           exit 2
           ;;
   esac
   ```

   **The capture directory is guard-local by necessity.** The guard sits
   immediately after the worktree resolves and deliberately *precedes*
   the gate's log directory, ambient root and `GATE_TMPDIR`, so none of
   those exist yet. It must therefore create its own, and it inherits
   the same **no-residue invariant** the guard was placed early to
   honour: a refused run leaves nothing behind.

   Files preserve every byte including NUL, `cmp` compares bytes, and
   `status` is the helper's own. Rust performs the same comparison on
   `out.stdout` against `TOKEN` and `TOKEN + b"\n"`. Neither consumer
   trims, and neither routes stdout through a shell variable.

   If a future consumer *must* use variable capture, the status has to
   be carried out explicitly —
   `out=$("$helper"; st=$?; printf x; exit "$st")` — and the NUL
   divergence still bars it from claiming byte equality.

3. Accept **only** these three pairs; every other combination is
   `error`:

   | status | stdout bytes | outcome |
   |---|---|---|
   | 0 | `pmacs-sigint-v1:safe` | `safe` |
   | 1 | `pmacs-sigint-v1:ignored` | `ignored` |
   | 2 | `pmacs-sigint-v1:error` | `error` |

4. Proceed only on `safe`. Otherwise stop — and **which stderr is
   authoritative depends on whether the pair validated**:

   - **Validated pair** (`ignored` or `error`): the helper's stderr
     *is* the diagnosis. Surface it unchanged.
   - **Boundary failure** (any invalid pair, or no pair at all): the
     helper's stderr is **untrusted and must not be presented as the
     diagnosis.** The consumer emits its own boundary wording, and
     either omits the child's stderr or reproduces it under an explicit
     untrusted label.

   **This closes a hole revision 13 left open.** A helper exiting
   **1 with no token but the canonical `SIGINT is ignored` text on
   stderr** would classify as boundary `error` — correctly — and then
   tell the operator their environment ignores `SIGINT`, which is A6's
   prohibition arriving through the diagnostic instead of the
   classification. A verdict that cannot be trusted cannot supply
   trusted wording either.

**`safe` is validated like the others.** Revision 12 let a consumer
proceed on exit 0 alone; under revision 13, `0` with a missing or wrong
token is `error` and the consumer stops. That is deliberate — a status
that arrives without the token did not come from this helper.

That produces one of three total outcomes:

| outcome | pair required | reached when |
|---|---|---|
| `safe` | `(0, pmacs-sigint-v1:safe)` | inner probe exits 23 |
| `ignored` | `(1, pmacs-sigint-v1:ignored)` | inner probe exits 0 after a successful `kill` |
| `error` | `(2, pmacs-sigint-v1:error)` | `kill` failed, `sh` unavailable, unexpected exit, another signal |
| `error` (boundary) | **anything else**, including *no* pair | helper missing or unexecutable; a status with a missing, mismatched, unknown or malformed token; **macOS's status 1 with no token** |

`error` is **not** treated as `ignored`. It fails the gate too, but with
a different diagnosis, because "your environment ignores SIGINT" and
"the guard could not run" are different problems and conflating them is
what a naive `exit 0` would do.

This is **POSIX shell only** — `trap`, `kill`, `$$` — so the mechanism
does not depend on `/proc` or `sigaction`: it is not Linux-only and adds
no `unsafe`. That is a contract-level portability argument, not a claim
that every supported Unix has already exercised it; A7 keeps the
implementation record explicit about which platforms were actually
tried.

**Both consumers use the same helper — but that alone no longer makes
them agree.** Under revision 12 the helper's exit status *was* the
verdict, so a shared helper guaranteed a shared answer. Under
revision 13 each consumer **independently validates the pair**, in a
different language, so they can now disagree by validating differently.
Revision 12's claim that they "can never disagree" is withdrawn.

What replaces it is a **shared conformance matrix plus one
consumer-specific boundary case**. Both validators exercise the shared
set and must agree on it; Rust alone exercises the no-status spawn-error
case that the shell boundary cannot represent.

**Two distinct failing outcomes**, which an earlier draft collapsed:

- **`error (validated)`** — the pair `(2, …:error)`. The helper ran and
  reported that it could not decide.
- **`error (boundary)`** — anything else. Nothing trustworthy was
  returned, so the consumer owns the wording (see step 4).

**The matrix is a generated cross-product over token class, encoding
and status, plus four out-of-band cases — enumerated below and counted
honestly.** An earlier draft applied the
malformed classes only at status 0, so a validator that checked tokens
strictly for `0` and accepted arbitrary output at `1` passed every row.

**Encodings matter, and one of them is what production actually
emits.** The helper prints with `echo`, so the real output is
**`TOKEN` + LF**. Both encodings validate:

```
E1 := TOKEN          (bare)
E2 := TOKEN LF       (what the shipped helper emits)
```

Per status, the cases are:

| class | stdout | count | expected |
|---|---|---|---|
| **V** | the **correct** token for this status, in `E1` and `E2` | 2 | **validates** |
| **M** | each of the **two other valid tokens**, in `E1` and `E2` | 4 | boundary |
| **E** | empty | 1 | boundary |
| **U** | `pmacs-sigint-v2:…` | 1 | boundary |
| **L** | LF + token | 1 | boundary |
| **X** | token + LF + LF | 1 | boundary |
| **S** | `␠` token `␠` | 1 | boundary |
| **C** | token + CR + LF | 1 | boundary |
| **D** | token token (one line) | 1 | boundary |
| **N** | token + NUL | 1 | boundary |

That is **14 per status × 3 statuses = 42**.

The **six mismatched valid-token pairs** are enumerated rather than
sampled, because choosing one per status would leave half of them
untested:

| status | wrong tokens (each in `E1` and `E2`) |
|---|---|
| 0 | `…:ignored`, `…:error` |
| 1 | `…:safe`, `…:error` |
| 2 | `…:safe`, `…:ignored` |

Only **V validates** — `(0,safe)` → `safe`, `(1,ignored)` → `ignored`,
`(2,error)` → `error (validated)`. The other **36** are
`error (boundary)`.

Out-of-band cases and their applicable consumers:

| # | consumers | case | expected |
|---|---|---|---|
| X1 | shell + Rust | status 126 with a correct token | `error (boundary)` — status outside 0–2 |
| X2 | **Rust only** | spawn failure (missing / unexecutable helper) | `error (boundary)`, **no status inspected** |
| X3 | shell + Rust | status 1, empty stdout, stderr = canonical ignored text | `error (boundary)`, and the output **must not** present "SIGINT is ignored" as the diagnosis |
| X4 | shell + Rust | status 0, correct token, plus extra bytes on **stderr** | `safe` — stderr is not consulted for classification |

**Truthful totals:** the shared set is **45 cases** — the 42-case
cross-product plus X1, X3 and X4 — and both validators must agree on all
45. Rust additionally exercises X2, for **46 distinct cases overall**;
the shell exercises 45 because an `exec` failure there necessarily
becomes a shell status. Earlier drafts said twelve, then twenty-three,
then thirty-four; each was a count of a set that had not actually been
enumerated.

The two consumers:

- **`scripts/gate` fails immediately**, before any stage, with an
  explicit ignored-`SIGINT` diagnosis.
- **The target test invokes it** and reports the same precondition
  failure if run directly, instead of "child did not exit within 5s".

**Every refusing branch must print the observed helper status AND the
token state** — valid, missing, or unexpected. This is **diagnostic
context, never the classifier**: the classification is the validated
pair, and the printout exists so a failure is legible without another
CI round-trip. Revision 12 printed the raw status only in its catch-all
branch, so when macOS failed, the path had to be identified indirectly
by which message text appeared — the log could not simply say.

**One practical finding, measured after the guard was written:
backgrounding is not the problem — one *way* of backgrounding is.** This
session's tool-level background mode leaves `SIGINT` deliverable (helper
exits 0); `setsid nohup … &` does not (helper exits 1). The construct
that caused this lane was never necessary, which makes the guard cheap:
it forbids only what was already avoidable.

**No override.** A full gate run under ignored `SIGINT` cannot produce
valid evidence, so there is no flag to proceed anyway — a switch that
lets the gate run in a state where several suites are meaningless would
recreate exactly the failure this lane spent nine revisions on.

**R-c is rejected**: restoring the child's disposition needs
`pre_exec`, which is `unsafe`, and dodging a project invariant to make
one test invocation-independent is not a trade this lane will make.

**The Linux-only D1/D2 instrumentation is removed** once its evidence is
portable — it read `/proc`, it has produced its finding, and leaving it
in place would carry a platform dependency for no further return.

## 8. Acceptance criteria — REPLACED for the selected remedy

The A1–A5 written for a teardown fix no longer describe this work; they
are superseded wholesale. What the guard-and-diagnosis change must
show:

- **A1 — the guard bites.** `scripts/gate` invoked with `SIGINT`
  ignored exits immediately, before any stage runs, naming the ignored
  signal as the reason.
- **A2 — the direct-test diagnosis bites.** The target test run
  directly with `SIGINT` ignored fails with the precondition message,
  **not** with "child did not exit within 5s".
- **A3 — foreground success is unaffected.** Both target copies pass
  foreground, and the guard does not fire, so the remedy costs nothing
  in the normal case.
- **A4 — mutation.** Removing the probe's `trap` makes A3 fail: a
  normal foreground signal terminates the inner shell and is classified
  as `error`, not `safe`. Treating inner exit 0 as `safe` makes A1 and
  A2 fail by allowing inherited ignore through. Collapsing `error` into
  `ignored` makes A6 fail. Each mutation is named against the distinct
  row it must bite.

  **Measured 2026-08-19; every prediction holds:**

  | mutation | helper fg | helper bg | forced error | bites |
  |---|---|---|---|---|
  | baseline | 0 | 1 | 2 | — |
  | remove the probe's `trap` | **2** | 1 | — | **A3** — foreground degrades to `error`; backgrounded classification unchanged |
  | treat inner exit 0 as `safe` | 0 | **0** | — | **A1 and A2** — inherited ignore passes through both consumers |
  | collapse `error` into `ignored` | — | — | **1** | **A6** — a forced failure reports the ignored wording |

  **Revision 13 adds token mutations**, each of which must bite:

  | mutation | must fail |
  |---|---|
  | consumers accept a **missing** token (status only) | A6 and the macOS row — this is exactly the shipped defect |
  | consumers accept a **wrong** token for the status (e.g. `…:safe` with exit 1) | A6 |
  | consumers accept an **unknown** token (`pmacs-sigint-v2:safe`) | A6 |
  | helper prints the token to **stderr** instead of stdout | **A1 and A3** — see below |
  | consumer surfaces child stderr as the diagnosis on a **boundary** failure | **A6b** |
  | consumer accepts any status 2 regardless of token | **A6c** |
  | consumer trims whitespace before comparing | the **S/C classes** in the shared 42-case cross-product |

  **Why that last one maps to A1/A3 and not A2.** With the token on
  stderr, stdout is empty, so *every* outcome becomes boundary `error`.
  A1 (gate refuses under ignored `SIGINT`) still refuses but with the
  wrong diagnosis, and A3 (foreground success unaffected) breaks
  outright because `safe` no longer validates — both bite. **A2 does
  not**, because A2 only requires the direct test to report *a*
  precondition failure rather than the 5 s deadline, and a boundary
  `error` satisfies that as written. Revision 13 listed A2 here
  incorrectly. Either mapping is defensible; this framing keeps A2
  broad — the property it protects is "never the misleading deadline
  message" — and relies on A6 to pin *which* diagnosis appears.
- **A5 — the gate is otherwise unchanged**: a normal foreground run
  reaches and passes every stage it did before, with no stage added,
  skipped, reordered, or made conditional.
- **A6 — the `error` outcome is distinct, and cannot be counterfeited
  by a status alone.** With the probe forced to fail, **both the gate
  and the direct target test** report `error`, not `ignored`, and
  neither claims the environment ignores `SIGINT`. Extended by
  revision 13 to cover the pair: a **missing**, **mismatched** or
  **unknown** token is `error` in both consumers, whatever the status
  accompanying it.
- **A8 — the guard leaves no residue, on every path.** The guard
  creates its own capture directory because it runs before the gate's
  temporary roots exist, and arms its cleanup **before** invoking the
  helper. After `safe`, `ignored`, validated `error`, boundary `error`,
  and a failure to create the directory at all, **no capture directory
  survives**. This is the same no-residue invariant that put the guard
  early in the first place; adding a capture directory must not weaken
  it.
- **A6b — a boundary failure never speaks with the helper's voice.**
  A helper exiting **1 with no token but the canonical
  `SIGINT is ignored` text on stderr** classifies as boundary `error`
  in both consumers, **and neither presents "SIGINT is ignored" as the
  diagnosis** — the child's stderr is omitted or explicitly labelled
  untrusted. Shared case X3. Without this, A6 is satisfiable in the
  classification while being violated in the message the operator
  actually reads.
- **A6c — exact-pair validation at status 2.** `(2, …:error)` is
  `error (validated)`; `(2, missing)`, `(2, …:safe)`, `(2, …:ignored)`
  and `(2, unknown-version)` are each **boundary** errors. The status-2
  slice of the shared cross-product exercises both permitted encodings,
  every mismatched valid token and every malformed class. A validator
  that accepts any status 2 regardless of token must fail this row.
- **A6a — the macOS row, SCOPED TO THE GATE.** An unexecutable helper
  on macOS makes `/bin/sh` exit **1 with no token**; the gate must
  classify that as **boundary error → 2**, never `ignored`. Not
  hypothetical: it is the observed CI failure on `70f0bc9`
  (`Test (macos-latest / lua54)` and `… / luajit`), and the row is
  satisfied only when that platform is green.

  **It does not apply to R-d, for two independent reasons**, and
  revision 13 was wrong to state it for "both consumers":
  - **R-d never sees that status.** The gate invokes the helper through
    `/bin/sh`, which converts an exec failure into a shell exit status.
    R-d uses Rust's `Command`, which returns a **spawn error with no
    exit status at all** — a different code path reaching `error` by a
    different route (conformance row 12, not row 5).
  - **macOS CI does not compile R-d's test.** It lives inside
    `#[cfg(feature = "crdt")] mod crdt`, and the macOS jobs run
    `--no-default-features --features <lua>` with no `crdt`;
    `Test (crdt)` is `runs-on: ubuntu-latest`.

  So R-d's macOS behaviour is **unexercised**, and this framing does not
  pretend otherwise. Closing that would need either a non-crdt-gated
  R-d row or a macOS crdt job — **neither is proposed here**, and A7
  records the gap instead of hiding it.
- **A7 — PARTIALLY EXERCISED ON macOS, one defect found, R-d still
  Linux-only.** Revision 12 closed this by disclosure because no
  non-Linux unix was reachable. **That is now stale: macOS CI reached
  it and measured it red**, so the disclosure fallback no longer
  applies and the criterion is restated against evidence.
  - **Exercised on macOS (`Test (macos-latest / lua54)` and
    `… / luajit`, head `70f0bc9`):** five of the six helper/gate rows
    pass — all three helper outcomes, gate refusal on `ignored`, and
    gate refusal on a helper-reported `error`.
  - **One known defect on macOS:**
    `gate_maps_an_unexecutable_helper_to_error_not_ignored` fails,
    status 1 with no token classified as `ignored`. This is the whole
    reason for revision 13 (§4d), and A6a is the row that closes it.
  - **R-d: Linux-only, unexercised on macOS**, because its test is
    crdt-gated and the macOS jobs build without `crdt`. Stated as a
    gap, not argued away.
  - **Everything else remains a contract argument**: the helper is
    POSIX shell only, reads no `/proc` and calls no `sigaction`. BSD
    and other unixes are still untried.

## 8b. Superseded criteria, kept for the record

These were written for a teardown fix that is no longer the work. They
are retained so the change of target is visible rather than silent;
**none of them binds.**

Written when this lane still expected a teardown repair. **Superseded
by §8**; kept verbatim below.

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
  was never made. What A3 requires instead: **the demonstrated D1/D2
  mechanism accounts for the subset/full difference, or D0b recreates
  that comparison under captured provenance before this lane closes.**
  In the first case, record the mechanism's explanation. In the second,
  whatever D0b shows is either explained by the fix or explicitly
  recorded as unexplained. A fix that greens the sweep without either
  path leaves the gap stated, not hidden.
- **A4.** No deadline raised, no test skipped, retried, or serialised
  to obtain green.
- **A5.** **Conditional on D4.** If bet 1 holds, this is unconditional:
  Ctrl-C on a real `pmacs --gpu` session tears down the frontend and
  leaves the daemon running. **RETIRED BY SCOPE**: D4 was not executed,
  bet 1 is withdrawn, and this lane is recorded as **gate/test
  correctness only**. A5 is not claimed satisfied and not claimed
  falsified — it is out of scope, and a user-facing teardown claim would
  need its own lane and its own evidence.

## 9. Coherence impact (`COHERENCE.md` §20)

- **Journey steps touched: NONE, as finally established.** Earlier
  revisions claimed 12(a) "closing is clean", on the premise that this
  lane repairs Ctrl-C teardown. §4c withdraws that premise: no product
  behaviour changes, because the failure is an artifact of how the test
  runner is invoked. Revision 1's "none" reached the right answer by
  the wrong route (grade movement, which §20 warns against); this is
  the right answer for the stated reason.
- What the lane does touch is **gate trustworthiness**: seven red
  sweeps that named a product defect and had none.
- **Grade movement: none expected.** This restores a property that is
  supposed to hold, rather than opening a new one.
- **Interaction islands: none added.**
- **Config registry: not touched. Background-work attribution: not
  touched.**
- What the lane restores is every *other* lane's ability to prove
  itself: while the gate can be run in a state where several suites are
  meaningless, a red stage 15 tells you nothing about the branch.

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
