# Framing — the CRDT half of the test corpus is dark in CI

**Revision 2.** Status: framing only. No branch, no implementation.
Scouted against `githubsucks/main` @ `4223dd3` (#208), with **zero open
PRs** on the board.

**Revision 1 → 2** records the user's decisions on Q#CC3, Q#CC4 and
Q#CC7, and **corrects revision 1's classification of `m10_10_perf`**,
which was wrong in a way that would have made the lane worse. See §1.3a.
The correction also completes the disposition accounting: all 279 dark
tests are now assigned, with 275 recovered and 4 excluded for named
reasons (§1.2a).

`.github/workflows/ci.yml` never enables the `crdt` feature anywhere.
Every `#[cfg(feature = "crdt")]` test is therefore **not compiled** in
CI — not skipped, not filtered, not reported. **279 tests have never
executed in CI, and 186 of them are in the library**, whose
`cargo test --lib --features crdt` invocation `CLAUDE.md` lists as a
**required** pre-PR gate. CI has never once run a required gate.

This lane was named in `docs/active-work.md` under "NEEDS A LANE" and
has had no branch and no owner since it was found while gating #166.

---

## 0. Coherence impact (COHERENCE §20)

- **Journey steps touched:** none directly. This lane adds no user-facing
  surface and no command.
- **Interaction islands added:** none.
- **Config registry adoption:** none.
- **Background-work attribution:** none.
- **Why it belongs on the board anyway:** it is the release-readiness
  prerequisite for **P8 (Distribution, §17)**. §17's grade is "missing —
  zero release machinery exists," and the first thing release machinery
  must do is produce an artifact from a tree whose tests actually ran.
  Shipping binaries over a corpus where half of the library's tests have
  never been compiled by CI converts an invisible gap into a shipped one.
  This lane does not advance a coherence concern; it protects the one
  that comes next.

---

## 1. Ground truth (measured at `4223dd3`, 2026-08-01)

Every number below was **re-measured on this tree**, not quoted forward.
The ledger is explicit that the figure moves with every merge, and it
did: the previous reading was 273 at `74301d1`.

### 1.1 The census

Under CI's exact flags versus the same flags plus `crdt`:

| | tests | test binaries |
|---|---:|---:|
| `--no-default-features --features luajit` | 3,467 | 93 |
| `--no-default-features --features luajit,crdt` | 3,746 | 101 |
| **dark** | **279** | **8** |

The binary column is a fact the earlier census did not carry: **eight
test binaries contain zero tests under CI's flags.** They are still
built, still run, and still report `ok` — with nothing in them. A green
result from those eight is not weak evidence, it is no evidence.

Per target, every row with a nonzero delta:

| dark | CI | full | target |
|---:|---:|---:|---|
| 186 | 1,899 | 2,085 | **the library itself** (`src/lib.rs`) |
| 21 | 15 | 36 | `m5_5_acceptance` |
| 14 | 1 | 15 | `gpu_invocation_acceptance` |
| 14 | 1 | 15 | `gpu_initial_target_acceptance` |
| 8 | 0 | 8 | `m10_11_acceptance` |
| 6 | 0 | 6 | `m10_2_perf` |
| 6 | 0 | 6 | `auto_pair_crdt_acceptance` |
| 4 | 5 | 9 | `vterm_stage3_acceptance` |
| 4 | 0 | 4 | `m10_10_perf` |
| 3 | 2 | 5 | `bottom_panel_stage2b_gpu_acceptance` |
| 3 | 0 | 3 | `compile_mode_crdt_acceptance` |
| 2 | 22 | 24 | `theme_faces_acceptance` |
| 2 | 0 | 2 | `m11_5_semantic_acceptance` |
| 1 | 18 | 19 | `terminal_copy_mode_acceptance` |
| 1 | 10 | 11 | `gpu_font_acceptance` |
| 1 | 9 | 10 | `vterm_stage1_acceptance` |
| 1 | 7 | 8 | `statusline_segments_acceptance` |
| 1 | 0 | 1 | `m10_11_perf` |
| 1 | 0 | 1 | `auto_indent_crdt_acceptance` |

The rows sum to 279. This is the whole census, not its head.
`bottom_panel_stage2b_gpu_acceptance` is new since the previous reading.

### 1.2 "279 dark" overstates what a plain `crdt` leg recovers

**Eleven of the 279 are `#[ignore]`d**, so adding `--features crdt` to
the `test` job does not run them — `--ignored` does, and the `test` job
does not pass it. Measured by listing the ignored set under the crdt
build and differencing against the CI build:

| dark | of which ignored | recovered by a plain leg | target |
|---:|---:|---:|---|
| 186 | 1 | 185 | the library |
| 8 | 3 | 5 | `m10_11_acceptance` |
| 6 | 6 | **0** | `m10_2_perf` |
| 1 | 1 | **0** | `m10_11_perf` |
| — | — | — | all other rows: nothing ignored |

**A plain `crdt` leg recovers 268 tests, not 279.** The remaining 11
need either an `--ignored` invocation or a deliberate exclusion.
Quoting 279 as the lane's deliverable would overstate it by the exact
set that is hardest to place.

### 1.2a Full disposition of all 279

Every dark test is assigned. Nothing is residue.

| count | disposition | mechanism |
|---:|---|---|
| 268 | **recovered by the plain `crdt` leg** — including `m10_10_perf`'s 4, which stay unignored (§1.3a) | new ubuntu/luajit `crdt` job |
| 7 | **recovered by a new `m10-perf-gates` job**: `m10_2_perf` (6) + `m10_11_perf` (1) | `--release --ignored --features crdt`, per the `m5`/`m6` precedent |
| 3 | **deliberately excluded**: `m10_11_acceptance`'s PTY-doubled tests, marked *"operator-invoked before tagging, not CI-default"* | unchanged; documented, not a gap |
| 1 | **deliberately excluded**: `buffer::tests::proptests::crdt_undo_of_an_identity_replace_reports_a_no_op_edit_carrying_an_op` — the **#157 CRDT undo repro**, an `#[ignore]`d marker for a known open defect | unchanged; arming it is that defect's lane, not this one |
| **279** | | |

**The lane recovers 275 of 279.** The other four are excluded with
stated reasons. This distinction matters for the PR description: a lane
that says "268 of 279" invites the question of what the 11 are, and two
of the four answers are "already correct."

### 1.3 The deliberate/accidental classification — the lane's stated first task

`docs/active-work.md` says this classification "is not finished and is
the lane's first task." It is finished here.

**Deliberate, and belonging in a perf job** — `#[ignore]`d because they
are release-mode benches or budget gates, exactly like
`m5_perf_acceptance` and `m6_perf_acceptance`:

- `m10_2_perf` — 6 dark, all 6 ignored: *"perf bench; release-mode-only
  via `--ignored --nocapture`"*.
- `m10_11_perf` — 1 dark, ignored: *"perf gate; requires release build"*.

**But their placement is accidental even so.** Grepping `ci.yml` for
`--test` yields exactly four named suites: `acceptance`,
`m4_acceptance`, `m5_perf_acceptance`, `m6_perf_acceptance`.
**`m10_2_perf` and `m10_11_perf` have no CI job at all**, with or
without `crdt`. Their being `#[ignore]`d is deliberate; their being
unreferenced by any workflow is not. Q#CC4 fixes the second.

**Deliberate, and belonging nowhere in CI** — a third disposition,
distinct from the above and easy to collapse into it:

- `m10_11_acceptance` — 3 of its 8 dark are ignored, marked *"PTY-doubled
  tests are operator-invoked before tagging, not CI-default"*. These are
  not benches awaiting a job; they are **deliberately manual**, run by an
  operator before tagging a release. Giving them a job would contradict
  the reason they are ignored. Its other 5 dark tests are ordinary
  coverage and are recovered by the plain leg.

**Accidental** — `m10_10_perf`: 4 dark, **zero `#[ignore]` markers**, and
no CI job naming it. Accidental in that *nothing runs it*. See §1.3a for
why its unignored state is nevertheless deliberate and must be
preserved.

Everything else in the table is accidental: ordinary correctness
coverage that has simply never been compiled.

### 1.3a `m10_10_perf` is not a perf gate — revision 1 got this wrong

Revision 1 classified `m10_10_perf` as a perf suite that "a plain `crdt`
leg would start running inside the general correctness job... a hazard
the fix must handle," and the natural remedy — add `#[ignore]`, give it
a perf job — was proposed and accepted on that basis. **The suite's own
header falsifies it:**

> The numbers are recorded to stdout via eprintln (visible under
> `cargo test -- --nocapture`) and asserted against **generous bounds
> that exist to catch catastrophic regressions, not to verify a tight
> perf claim.**

Its four tests are catastrophic-regression tripwires with deliberately
loose bounds. **The absence of `#[ignore]` is the design, not an
oversight**, and the contrast with its siblings is explicit in their
ignore reasons:

| suite | `#[ignore]` reason | what it is |
|---|---|---|
| `m10_2_perf` | *"perf bench; release-mode-only via `--ignored --nocapture`"* | benchmark |
| `m10_11_perf` | *"perf gate; requires release build"* | budget gate |
| `m10_10_perf` | **none** | regression tripwire |

Three consequences:

1. **Adding `#[ignore]` would demote a deliberate CI-default tripwire
   into an operator-invoked bench** — a coverage *reduction* shipped
   inside a coverage lane.
2. **The hazard revision 1 named does not apply to this suite.** Perf
   assertions are dangerous in a shared correctness job when their
   bounds are tight; generous bounds designed to catch only catastrophic
   regressions are precisely what is safe there.
3. **`m10_10_perf` therefore needs no job of its own.** It is recovered
   by the plain `crdt` leg, and its 4 tests are inside the 268.

The general lesson, which is the reusable part: *a suite's `#[ignore]`
state is a claim about how it should be invoked, and the file that makes
the claim is the authority.* Revision 1 classified three suites by their
filename suffix (`_perf`) and their marker counts, and got the one whose
name and markers disagreed with its purpose exactly backwards.

### 1.4 The clippy obstacle is real, and the ledger's inventory is stale

`cargo clippy --workspace --all-targets --features crdt -- -D warnings`
**fails on `main`.** The ledger recorded this and correctly warned that
its list was "a lower bound, not an inventory," because clippy abandons
remaining targets once one fails.

**`--keep-going` is what converts the lower bound into an inventory.**
That flag was not used before; with it, the complete set at `4223dd3` is
**eight findings across four files**:

| file:line | lint |
|---|---|
| `src/daemon.rs:4464` | `useless_conversion` to the same type: `u64` |
| `src/daemon.rs:4544` | item in documentation is missing backticks |
| `src/daemon.rs:4551` | `too_many_lines` (112/100) |
| `tests/auto_indent_crdt_acceptance.rs:42` | missing doc backticks |
| `tests/bottom_panel_stage2b_gpu_acceptance.rs:509` | `too_many_lines` (104/100) |
| `tests/vterm_stage3_acceptance.rs:637` | `too_many_lines` (122/100) |
| `tests/vterm_stage3_acceptance.rs:816` | `too_many_lines` (132/100) |
| `tests/vterm_stage3_acceptance.rs:866` | redundant `continue` |

**The ledger's list is wrong in both directions**, which is why it had to
be re-measured rather than carried forward: the `unneeded mut` at
`src/daemon.rs:4965` is **gone** (fixed incidentally by later work), a
finding in `bottom_panel_stage2b_gpu_acceptance.rs` is **new**, and every
`src/daemon.rs` line number has moved. A stale lint inventory is worse
than none, because it invites fixing lines that no longer exist.

None of the eight is a correctness defect. All are lint-policy findings,
and none requires a behavioral change — which is what makes them safe to
clear in a preparatory commit rather than a design round.

### 1.5 `PMACS_REQUIRE_GPU` does not cover the suites the fix wants to move

The proposed fix routes four GPU-requiring `crdt` suites onto the
existing `gpu-render` job, on the grounds that it already has lavapipe
and `PMACS_REQUIRE_GPU=1`. **That job runs `cargo test -p pmacs-gpu` — a
different package.** All four target suites live in the root `pmacs`
package's `tests/`, so moving them means adding a *new* root-package
invocation to that job, not extending an existing one.

And the guard is **not uniform across the four**. Grepping every
reference to `PMACS_REQUIRE_GPU`:

- `tests/vterm_stage3_acceptance.rs` — two sites, both binary-presence
  skips promoted to failures.
- `tests/bottom_panel_stage2b_gpu_acceptance.rs` — one site, same shape.
- `pmacs-gpu/src/main.rs` — adapter presence, a different condition.
- **`gpu_invocation_acceptance` and `gpu_initial_target_acceptance`
  reference it nowhere.** Setting the variable does not arm them.

So `PMACS_REQUIRE_GPU=1` is necessary for a37 and the panel suite and
**insufficient as a blanket guarantee** that all four ran. Whatever
proves these suites executed has to be per-suite, not one environment
variable assumed to cover the set.

### 1.6 The a37 vacuum, restated with its current mechanism

`a37_real_daemon_real_pty_and_headless_gpu_render_one_terminal_session`
derives the frontend binary path from `CARGO_BIN_EXE_pmacs`'s sibling and
**returns `ok` after an `eprintln!` when it is absent**. The ledger
measured 9/9 in 0.17 s having never run it, against ~4 s for a real run.
`PMACS_REQUIRE_GPU` is the only thing that promotes that skip to a
failure, and `CLAUDE.md` applies that flag to `cargo test -p pmacs-gpu`,
a different package — so **the required local gate does not cover a37
either.** This is unchanged and re-confirmed at the grep above.

### 1.7 The local sweep is green, and what that does and does not mean

A full serialized sweep under CI's flags plus `crdt` —
`cargo test --all-targets --no-default-features --features luajit,crdt -- --test-threads=1 --skip basedpyright`
— completed on this machine at `4223dd3`:

**104 suites reporting, 3,715 passed, 0 failed, 30 ignored.**

It **reconciles exactly** with §1.1's census, which is the check that
matters: 3,715 passed + 30 ignored + 1 `basedpyright` test filtered by
`--skip` = **3,746**, the crdt-flags census figure. No test binary was
silently absent from the sweep, and no suite reported `ok` for a
population smaller than the census predicted. A sweep that did not
reconcile would be the a37 vacuum at corpus scale.

Seven targets report `ok. 0 passed`. Three are helper binaries with no
tests (`pmacs_audit`, `pmacs_fake_lsp`, `pmacs_fake_mcp`) — expected.
**The other four make §1.3's asymmetry visible in the run itself:**
`m5_perf_acceptance` and `m6_perf_acceptance` report zero *and have
`--ignored` CI jobs*; `m10_2_perf` and `m10_11_perf` report identically
and **have none**. The same output line means "covered elsewhere" for two
of them and "covered nowhere" for the other two, which is exactly why
Q#CC4 cannot be answered by looking at a test run.

**What this establishes:** the 268 recoverable tests are not hiding
correctness defects. Bet 1 holds locally.

**What it does not establish, and the distinction is the lane's whole
risk:** the failures the ledger predicts are *hosted-runner timing and
concurrency* failures — real PTY behavior on CI runners, wgpu under
lavapipe, daemon sockets at unfamiliar concurrency. A green serialized
run on a 16-thread developer machine removes one class of explanation
(the tests are wrong) and leaves the class actually expected (the
environment differs) entirely untested. **This result must not be quoted
as evidence that the CI leg will be green.**

### 1.8 What is NOT established

- **No macOS reading exists.** Every measurement here is Linux. The
  ledger's expectation that these suites will be flakier on hosted
  runners than on a developer machine is an expectation, not a
  measurement.
- **This machine is a laptop with an integrated Radeon (Phoenix1), 16
  threads.** It is a real Vulkan device rather than lavapipe, but it is
  shared-memory and thermally constrained, which is precisely the
  condition under which the ledger records a37 producing
  `last_frame_text` all spaces with nonzero `rendered_nonuniform_frames`.
  **A red a37 here is ambiguous by construction.** Universum (7900 XTX,
  available remotely) is where that ambiguity gets settled — and settling
  it there proves the *test* is sound, not that the *CI job* will pass,
  because the CI job runs on lavapipe regardless.

---

## 2. Questions

- **Q#CC1 — does the `crdt` leg go on the existing `test` matrix or its
  own job?** The matrix is already 4 legs (2 OS × 2 Lua flavors) and is
  the CI critical path at an observed 17-minute max. A `crdt` leg on all
  four doubles the most expensive job. A separate ubuntu-only job costs
  one leg. **Leaning: separate job, ubuntu-only, per the ledger's "start
  ubuntu-only and decide about macOS from evidence."**
- **Q#CC2 — is `crdt` matrixed over Lua flavor?** The feature is
  orthogonal to the Lua VM. `m5-perf-gates` and `m6-perf-gates` already
  set the precedent of luajit-only with a written justification.
  **Leaning: luajit only, with the reason recorded in the workflow the
  way the existing perf jobs do.**
- **Q#CC3 — where does `m10_10_perf` go? DECIDED: it stays unignored and
  runs in the plain `crdt` leg.** The decision was first taken as "give
  it a perf job," on revision 1's classification; §1.3a then established
  that classification was wrong — the suite is a deliberate CI-default
  regression tripwire with generous bounds, not a bench, and adding
  `#[ignore]` would reduce coverage inside a coverage lane. **The
  decision was reversed on the evidence, not on preference**, and it is
  a one-line change to reinstate the original answer if the tripwire
  reading is rejected.
- **Q#CC4 — do `m10_2_perf` and `m10_11_perf` get a job in this lane, or
  a follow-on? DECIDED: in this lane.** They are dark for a second,
  independent reason (unreferenced by any workflow) that predates the
  `crdt` gap, so this PR does fix two causes at once. That is accepted
  deliberately: leaving them would ship a lane headlined "the dark tests
  now run" with 7 still dark, and the second cause is one job block, not
  a second investigation. They go in a new `m10-perf-gates` job shaped
  like the existing `m5-perf-gates` / `m6-perf-gates`.
- **Q#CC5 — what proves each GPU suite actually executed?** §1.5
  establishes `PMACS_REQUIRE_GPU` covers only two of the four. Does the
  lane add the guard to the other two, or assert execution some other
  way (a test-count floor per suite)?
- **Q#CC6 — do the clippy fixes ride this PR or precede it?** Eight
  lint-policy findings across four files, none behavioral. Riding along
  means the PR that adds CI coverage also touches `src/daemon.rs`.
  **Leaning: a separate preparatory commit on the same branch**, so the
  diff reads as two intents, but not a separate PR — the lints are
  unreachable-by-CI today and have no independent reason to be fixed.
- **Q#CC7 — is a red first run acceptable to merge behind? DECIDED:
  fix in-lane by default, with one bounded escape hatch (below).**

  First, one of the three options offered in revision 1 was not real.
  **"Land the leg non-required until it is green" assumed the first CI
  run happens after merge. It does not** — the new job runs on the pull
  request, so the entire first-run failure set is visible *during
  review*, before anything reaches `main`. There is no window in which
  `main` carries a red required check, and therefore nothing to protect
  against by landing non-required. The genuine choice is only
  fix-in-lane versus quarantine-with-follow-on, taken per failure once
  observed.

  **Default: fix in-lane.** The escape hatch is keyed on *cause class*,
  not on effort, because effort is what turns a scoped lane into an
  unbounded one:

  1. **The lane's own configuration** (wrong flags, missing build step,
     a suite that skipped vacuously) — **always in-lane.** It is this
     PR's defect.
  2. **Reproduces locally** under §1.7's sweep or on Universum —
     **in-lane.** A real test defect the lane surfaced is the lane
     working as intended.
  3. **Environment-only** — green serialized here, green on Universum,
     red only on a hosted runner — **named follow-on lane.** This is the
     class that historically consumed multiple review rounds on this
     project: the signal lane had three tolerance rules rejected across
     three revisions, each concluding something about a process from
     something that was not about that process. Committing in advance to
     resolve that class *inside a workflow-configuration PR* is
     committing to an unbounded investigation in a lane whose whole
     value is being small and mergeable.

  **§1.7 makes class 3 the most likely red and class 2 the least**, which
  is the uncomfortable direction: the corpus is already green serialized
  on a developer machine, so a CI red is by elimination an environment
  difference. The escape hatch exists precisely because the measurement
  points that way.

---

## 3. Bets

- **Bet 1 — the 268 recoverable tests mostly pass. RESOLVED: they all
  do, locally.** The serialized sweep is 3,715/3,715 with an exact
  census reconciliation (§1.7). This bet is settled for the developer
  machine and **explicitly not settled for CI**, which is the
  environment the lane is actually changing.
- **Bet 2 — the failures that do appear concentrate in the process/PTY
  and GPU suites**, not in the library. The library's 185 are pure logic
  over a CRDT backend; the flake surface the ledger documents is
  uniformly real-process and real-device. *Unresolved: with zero local
  failures there is nothing yet to concentrate. This bet now resolves
  only on the first CI run, and is the reason §7 sequences the GPU
  commit last.*
- **Bet 3 — the eight clippy findings are the complete blocker.** With
  `--keep-going` there is no remaining truncation, so no further lint
  surprises appear once these are cleared. *Falsified if clearing them
  reveals findings in targets that failed to build for a non-lint
  reason.*
- **Bet 4 — `PMACS_REQUIRE_GPU` alone will not prove the GPU suites ran.**
  §1.5 already all but establishes this; the bet is that a per-suite
  execution assertion is needed and that adding it finds at least one
  suite silently skipping.

---

## 4. Acceptance

Written against §1's measurements, with the local sweep (§1.7) in hand.
Criteria 8 and 9 exist specifically because that sweep came back green:
a green pre-measurement is the condition under which a vacuous CI job is
easiest to ship unnoticed.

1. `cargo clippy --workspace --all-targets --features crdt -- -D warnings`
   exits zero on the branch, and the check is run **with `--keep-going`**
   so its success is an inventory rather than a first-failure abort.
2. The dark census is **re-measured on the branch** and the workflow runs
   a leg that compiles them. **275 of 279 recovered**, with the four
   exclusions named individually (§1.2a) — never one headline number
   with an unexplained remainder.
2a. `m10_10_perf` is **unmodified** by this lane: no `#[ignore]` added,
   no job of its own, its 4 tests recovered by the plain leg. A diff
   touching `tests/m10_10_perf.rs` fails this criterion (§1.3a).
3. Every GPU-requiring suite added to a job **proves it executed**, per
   Q#CC5 — a suite that skips its body reports failure, not `ok`.
   `PMACS_REQUIRE_GPU` alone does not satisfy this, because it is absent
   from two of the four suites (§1.5).
4. `a37` specifically: a run that does not build `pmacs-gpu` **fails**
   rather than reporting 9/9 in 0.17 s.
5. The perf suites' placement is explicit and each is justified in the
   workflow text: `m10_2_perf` and `m10_11_perf` in a new
   `m10-perf-gates` job matching the `m5`/`m6` precedent;
   `m10_10_perf` deliberately in the correctness leg, **with its
   generous-bounds rationale written into the workflow comment** so a
   later reader does not "fix" the inconsistency by ignoring it.
6. The new job carries `timeout-minutes`, per the workflow's own standing
   rule that every job does.
7. `docs/active-work.md`'s "NEEDS A LANE" block is replaced by this
   lane's state, and its stale figures (273 dark, the seven-item clippy
   list) are corrected rather than left beside the new ones.
8. **The new job's test count reconciles.** Its reported passed +
   ignored + filtered must equal the branch's re-measured crdt census
   for the suites it runs, the way §1.7 reconciles to 3,746. A job that
   runs fewer tests than the census predicts has found a silently-absent
   binary, and that is the defect class this whole lane exists to end.
9. **A deliberately-broken bite proves the leg is not vacuous.** Before
   merge, break one `crdt`-gated library test on the branch and confirm
   the new CI job goes red. A leg added to a corpus that is already
   green locally cannot otherwise be distinguished from a leg that
   compiles nothing — which is precisely the failure `.github` has today
   and the reason criterion 8 is not sufficient on its own.

---

## 5. Parked

- **macOS.** Ubuntu-only first, by the ledger's own instruction. A macOS
  `crdt` leg is a follow-on decided from the first run's evidence.
- **The `--lib --features crdt` flake.** `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
  failing ~1 in 5 under parallel full-suite load, with
  `active_reader_probe` returning `None`. The ledger's leading
  explanation — `drain_until`'s tick reaping the leader before the probe
  — is an inference from control flow, **not a falsified root cause**, and
  no serial full-suite bite has been run to separate parallelism from
  another whole-suite effect. Discriminating it is named as belonging to
  this lane; it should be its own PR, because it is a product defect
  hypothesis and everything else here is workflow configuration.
- **The two unattributed CRDT failures from #178's round-2 gating.** No
  test names were captured, so there is nothing to reproduce.
- **`basedpyright`.** Still hangs forever, still `--skip`ped, still
  deliberately not installed in CI. Unchanged by this lane.

---

## 6. Gates

The standing suite from `CLAUDE.md`, plus the two this lane exists to
make meaningful:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- **`cargo clippy --workspace --all-targets --features crdt --keep-going -- -D warnings`** (new, and the lane's own subject)
- `cargo test --lib`
- `cargo test --lib --features crdt`
- the touched acceptance suites
- `cargo test --test m4_acceptance -- --skip basedpyright`
- `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`
- `git diff --check`

Verified green on this machine at `4223dd3` **before any change**: fmt,
diff-check, `--lib` (1,896 passed), `--lib --features crdt` (2,081
passed, 4 ignored), required GPU (221 passed), and the full serialized
crdt sweep (3,715 passed, 0 failed, 30 ignored, reconciling exactly to
the 3,746 census — §1.7).

Recording the pre-change baseline matters here more than usual: this
lane's subject *is* the test corpus, so without a baseline any red on
the branch is unattributable between the lane's changes and the tests it
newly compiles.

---

## 7. Branch plan

One branch, `ci-crdt-coverage`. Commits in this order, because each
earlier one is a precondition for the next being observable:

1. **Clear the eight clippy findings** (§1.4). Nothing can compile the
   `crdt` targets under `-D warnings` until this lands, so no CI change
   is testable before it.
2. **Add the `m10-perf-gates` job** for `m10_2_perf` and `m10_11_perf`
   (Q#CC4), shaped like `m5-perf-gates` / `m6-perf-gates`:
   `--release --features crdt -- --ignored --nocapture`, with
   `timeout-minutes` and a written luajit-only justification.
   **No source file changes** — per Q#CC3 and §1.3a, `m10_10_perf` is
   not touched and gets no `#[ignore]`.
3. **Add the non-GPU `crdt` leg**, ubuntu-only, luajit-only, with its
   justification written in the workflow. This is the commit that
   recovers 268 tests, `m10_10_perf`'s 4 among them.
4. **Prove the leg is not vacuous** (acceptance 9): break one
   `crdt`-gated library test, confirm the new job goes red, revert.
   Do this *before* step 5, so the proof is against the simpler job.
5. **Add the GPU-requiring `crdt` suites to `gpu-render`**, with the
   per-suite execution proof from Q#CC5 — this is where §1.5's
   package mismatch has to be handled explicitly.
6. **Update `docs/active-work.md` and `docs/agent-handoff.md`** per
   acceptance 7.

**The GPU-suite commit is the one that needs Universum.** Steps 1–3 are
fully verifiable on this laptop; step 4's failures cannot be
distinguished from thermal and load noise here (§1.8), and settling them
on the 7900 XTX proves the tests are sound without proving the lavapipe
CI job will pass. Budget for that gap rather than assuming it away.
