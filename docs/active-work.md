# Active work — cross-machine resume ledger

**Snapshot: 2026-07-28.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

**No lane below is retained past its merge.** This snapshot removes the
resource-op delete guard (#186) and dired Stage 2 framing (#171) lanes
the moment their PRs merged, because the same commit put their
load-bearing decisions into `docs/agent-handoff.md` §1 — rule 4's
precondition, satisfied deliberately rather than deferred. The
bottom-panel lane is not removed: 2B-2 landing leaves 2B-3 and Stage 3
ahead of it, so the lane is rewritten to the remaining plan.

**#188's lane arrived with #188**, which is the point: with several PRs
open, a lane written on `main` for work that lands elsewhere
re-conflicts on every merge. Written on its own branch it costs one
conflict, at the merge that would have happened anyway.

The PTY terminate
diagnostic (#176) was the last lane retained past its merge — retained
because rule 4 removes a
merged lane only *after* its durable facts reach
`docs/agent-handoff.md`, and that absorption was unowned. The
2026-07-28 snapshot
owned it: #176's facts are now in the handoff (§1's arc bullet and §5's
two ops lessons about ticking observers and proving child exit), so its
lane is gone. The Lean 4, GPU-terminal-input, inline-math (#172), dired
(#169), and terminal config + copy mode lanes were removed the same way
— the last of these was #180's work, folded into #182 so two open PRs
would stop re-conflicting in this file.

**Trust the canonical-base line below over any lane header**: if a PR
number appears in `git log --first-parent githubsucks/main`, it has
landed regardless of what a lane says.

**Two open PRs had no lane here at all before the 2026-07-28
snapshot** — #174 and #171. An open PR is exactly the volatile work this
file exists to record, so its absence is a ledger defect rather than a
tidy omission: #171 drifted **153 commits** while invisible here, and
its still-green old CI run described a tree nobody had looked at since.
**When a PR is opened, give it a lane.** All three have since merged —
#174, #171 and #186 — so per rule 4 their lanes are gone again and
their durable facts are in `docs/agent-handoff.md` (§5 for #174's
lesson, §1 for the two framings).

## Repository authority

- Canonical development URL:
  `https://github.com/levineuwirth/pmacs.git`. This ledger uses the
  normalized local alias `githubsucks` so its refs and recovery commands
  are identical on every machine. Remote names are otherwise
  machine-local: `origin` may name this canonical URL, a release mirror,
  or something else, and therefore has no authority by name alone.
- Canonical base at this snapshot:
  `githubsucks/main` @ `7586905` (the docs-only coherence listview
  correction #189, atop the docs-only landed-state refresh #185, the M4
  config-sink race fix #174, bottom-panel Stage 2B-1 #184, the
  Journey/GPU directory-target ratchet #183, Journey Stage 1a #182 and
  the previously recorded landed work).
  **Protocol schema support is
  `v6..=v21`; the production server-first `Hello` still advertises
  v20** — two different facts, and #184 landed only the first. The
  previous snapshot named `0442d78`, and **the
  recovery floor advances with it**: the check below now requires
  `7586905` or newer, so a tree at `0442d78` no longer passes. That is
  deliberate — the floor moves with the base, because a check that
  accepts an older commit than the declared base passes on a tree the
  rest of this file does not describe.
  **Lanes below that name an older base have not been re-based; derive
  their integration surface from `git diff <their base>..main`.**
- On the transfer source, `origin/main` named a release mirror at
  `d3fa632` and lagged badly. On the current destination, `origin` names
  the canonical URL. This difference is why all recovery begins by
  verifying URLs and normalizing `githubsucks` rather than trusting
  `origin/main`.
- The shared desktop checkout contained unrelated uncommitted work. The
  branches below were prepared in isolated worktrees; never clean or
  overwrite the shared checkout to recover them.

Start on another machine by inspecting its remotes:

```sh
git remote -v
git remote get-url githubsucks
```

If the second command says the alias is absent, add it; if it prints a
different URL, stop and resolve that collision rather than overwriting an
unknown remote:

```sh
git remote add githubsucks https://github.com/levineuwirth/pmacs.git
```

Then recover current refs:

```sh
git fetch githubsucks --prune
git log -1 --oneline githubsucks/main
git worktree list
git status --short --branch
```

The `git log` command must expose `7586905` — the base named above — or a
newer intentional main. Keep this threshold and the canonical-base line in
step: a recovery check that accepts an older commit than the base it
declares canonical will pass on a tree the rest of this file does not
describe.
If it does not, stop and repair the remote/fetch configuration.

## The CRDT half of the test corpus is dark in CI — NEEDS A LANE

- **No branch, no framing yet.** Found while gating #166, then measured
  properly during the vterm as-framed audit. Deliberately kept out of #166 so
  a CI change would not arrive after review approval.
- **Root cause:** `.github/workflows/ci.yml` never enables the `crdt` feature
  anywhere — zero hits across the workflow directory. The `test` job runs
  `cargo test --all-targets --no-default-features --features luajit|lua54`.
  Every `#[cfg(feature = "crdt")]` test is therefore **not compiled** in CI,
  not merely skipped.
- **Measured, `--list` under CI's exact flags versus the same flags plus
  `crdt`: 3,176 vs 3,449 — 273 tests dark.** Re-measured at `74301d1`
  (2026-07-26; at `fe8b8ba` it read 3,170 vs 3,443, the same 273 dark —
  #176 added six tests, none of them `crdt`-gated). **The number moves
  with every merge and must be
  re-measured, not quoted.** #168 reported 3,024 vs 3,288 — 264 dark,
  177 in the library — at `1b6a084`; #178 then added CRDT-only
  generated-buffer coverage, and other lanes landed CRDT tests in
  between. Per target:

  | dark | CI | full | target |
  |---:|---:|---:|---|
  | 185 | 1,848 | 2,033 | **the library itself** (`src/lib.rs`) |
  | 21 | 15 | 36 | `m5_5_acceptance` |
  | 13 | 1 | 14 | `gpu_invocation_acceptance` |
  | 13 | 1 | 14 | `gpu_initial_target_acceptance` |
  | 8 | 0 | 8 | `m10_11_acceptance` |
  | 6 | 0 | 6 | `auto_pair_crdt_acceptance` |
  | 6 | 0 | 6 | `m10_2_perf` |
  | 4 | 5 | 9 | `vterm_stage3_acceptance` |
  | 4 | 0 | 4 | `m10_10_perf` |
  | 3 | 0 | 3 | `compile_mode_crdt_acceptance` |
  | 2 | 22 | 24 | `theme_faces_acceptance` |
  | 2 | 0 | 2 | `m11_5_semantic_acceptance` |
  | 1 | 14 | 15 | `terminal_copy_mode_acceptance` |
  | 1 | 9 | 10 | `vterm_stage1_acceptance` |
  | 1 | 7 | 8 | `statusline_segments_acceptance` |
  | 1 | 10 | 11 | `gpu_font_acceptance` |
  | 1 | 0 | 1 | `auto_indent_crdt_acceptance` |
  | 1 | 0 | 1 | `m10_11_perf` |

  The rows sum to 273; the table is the whole census, not its head.

- **The single worst line is the library.** `cargo test --lib --features crdt`
  is a REQUIRED local gate in `CLAUDE.md`, and CI has never run it. 185
  library tests — the whole CRDT half — are developer-machine-only, and
  that count grows with every merged branch that adds a `crdt`-gated
  unit test.
- **Ten suites run zero or one test in CI**, including `gpu_initial_target`
  (#148's entire acceptance, 1/14), `gpu_invocation` (#141's, 1/14), and
  `a37`, the Vterm Stage 3 real-daemon/real-PTY/real-wgpu path that #135
  built specifically because "a decoded-message fixture would prove none of
  the three fit together".
- **⚠ `a37` will report green in the new job without running, unless the
  job builds `pmacs-gpu` AND sets `PMACS_REQUIRE_GPU=1`.** Measured
  2026-07-26 while gating #173. `a37_real_daemon_real_pty_and_headless_gpu_
  render_one_terminal_session` derives its sibling binary path from
  `CARGO_BIN_EXE_pmacs`, and on a missing binary it `eprintln!`s a skip and
  **returns `ok`**. A fresh worktree running
  `cargo test --features crdt --test vterm_stage3_acceptance` reports **9/9
  in 0.17 s having never run it**; a real run takes ~4 s. Only
  `PMACS_REQUIRE_GPU=1` promotes that skip to a failure, and `CLAUDE.md`
  applies that flag to `cargo test -p pmacs-gpu` — a **different package**,
  so the required local gate does not cover a37 either. The `gpu-render`
  job already sets the flag, which is what makes fix-shape part 2 sound;
  state it as a **requirement** of that job rather than inheriting it by
  luck, because a `crdt` leg added to the plain `test` job would run a37
  vacuously.
- **`a37` is also load-sensitive, which changes how to read the expected
  first-run failures.** It passed at `d152120` and failed at that *same
  commit* twenty minutes later, with a second agent saturating the machine
  with `rustc` in between; it then failed identically on `d152120`,
  `04c5ad1`, and the #173 merge commit, which is how #173 established the
  failure was not its own. The signature is `last_frame_text` all spaces
  with `rendered_nonuniform_frames` nonzero — frames arrive, content does
  not. `pmacs-gpu`'s own suite flaked the same way under the same load
  (201/202, then 202/202 on immediate rerun). **So a red a37 on the first
  CI run is ambiguous by construction**: before treating it as a real
  failure, run the same command on the merge base, and prefer serialized
  execution for this suite over retry-until-green.
- **Sort deliberate from accidental before proposing a fix.** Some of the 264
  are perf suites that are `#[ignore]`d by default and belong to their own
  jobs (`m10_2_perf` 6, `m10_11_perf` 1). `m10_10_perf` has **no** `#[ignore]`
  and no CI job naming it, so it looks accidental. This classification is not
  finished and is the lane's first task.
- **Fix shape, two parts** (the flag combination is verified to work:
  `--no-default-features --features luajit,crdt` lists 10 vterm Stage 1 tests
  versus 9 without):
  1. a `crdt` leg on the `test` job for the non-GPU suites and the library;
  2. the GPU-requiring `crdt` suites onto the existing `gpu-render` job, which
     already has lavapipe and `PMACS_REQUIRE_GPU=1` —
     `vterm_stage3_acceptance`, `gpu_invocation_acceptance`,
     `gpu_initial_target_acceptance`, `gpu_font_acceptance`.
- **Expect first-run failures, and budget for them.** These would execute in
  CI for the first time ever: real PTY timing on CI runners, wgpu under
  lavapipe, and daemon-socket tests at unfamiliar concurrency. Start
  ubuntu-only and decide about macOS from evidence. A red first run is the
  lane working, not the lane failing.
- Mitigating fact, verified rather than assumed: #166's three unit pins are
  **not** `crdt`-gated and do run under CI's exact flags, including the
  controller-release pin whose only job is catching the plausible wrong fix.
- **This lane also owns a `--lib --features crdt` flake, observed and
  scoped without overclaiming its cause** (inherited from #178's gating,
  where the terminal lane recorded it). `cargo test --lib --features
  crdt` failed ~1 run in 5 on
  `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
  — `active_reader_probe` returning `None` at `process.rs:3179` ("live
  runtime probe"). **Pre-existing and unrelated to #178:** that branch
  did not touch `src/process.rs` at all, and the test passed 10/10
  standalone; the observed
  failures were during parallel full-suite runs. That localizes the
  trigger to suite load or interaction, but does **not** distinguish
  parallelism from another full-suite effect — no serial full-suite bite
  was run. The leading code-path explanation is the known `drain_until`
  trap: draining for `Started` also ticks, and a tick can reap the leader
  before the following `active_reader_probe`. That is an inference from
  the failure site and control flow, not yet a falsified root cause.
  Discriminating it belongs here. Two unnamed CRDT failures in #178's
  round-2 gating are a plausible match but remain **unattributed** — no
  test names were captured.
- **A second standing obstacle for this lane:** `cargo clippy --workspace
  --all-targets --features crdt -- -D warnings` **fails on `main`** —
  measured at `74301d1`: seven errors before the build aborts, four in
  `src/daemon.rs` (`useless_conversion` at 3996, missing doc backticks at
  4076, `too_many_lines` 112/100 at 4083, an unneeded `mut` at 4965) and
  three in `tests/vterm_stage3_acceptance.rs` (`too_many_lines` at 637
  and 793, a redundant `continue` at 843). **Treat that as a lower
  bound, not an inventory:** Clippy abandons the remaining targets once
  one fails, and a run on an older tree surfaced a further doc-backticks
  error in `tests/auto_indent_crdt_acceptance.rs:42` that this run never
  reached. The
  standing gate list runs Clippy without `crdt`, so these lints have
  never been enforced. Any CI job that compiles the `crdt` targets has to
  fix them first or it will be red on arrival.

## Bottom-panel lane (Arc 7) — 2B-2 MERGED; 2B-3 IS NEXT

Stage 1, the Stage 2 framing, Stage 2A, Stage 2B-1, and **Stage 2B-2 are
all on `main`**. Framing revision 5's three-way split of 2B was
explicitly approved on 2026-07-27; revision 6 records PR #184's review
correction. **2B-2 — the daemon panel projection and epoch machine —
landed as [PR #187](https://github.com/levineuwirth/pmacs/pull/187)**,
one review round of five findings on top of the implementation, 12/12
green, 22/22 mutations biting. Its durable lessons are in
`docs/agent-handoff.md` §1; what remains below is the 2B-3 plan.

**2B-2's boundaries, restated because they are easy to overrun:** the
production `Hello` stays at v20 and `panel_capable` stays `false`. The
slice is dark/test-only capability exactly as 2B-1 was. Compatible v21
activation, the GPU band, and the negotiated capability flip are all
2B-3's, and 2B-3 may **not** simply change the unsolicited `Hello` to
21.

- **What PR #187 shipped, dark by construction:** the semantic daemon's
  `FrontendCellGeometry` epoch machine; one reconciled panel grid
  derivation; `PanelFrame::{Present, Absent}` projection on both document
  and terminal semantic paths; stable presentation epochs; resize and
  pointer validation against the live window/buffer/epochs; the panel's
  own statusline context; and pre-drain semantic panel-terminal resize.
  It does not add the GPU consumer or enable the capability.
- **Review round 1 closed five findings plus one sweep result at
  `3ecb03d`.** The wire-area clamp became durable hide state; a stale
  same-buffer reopen can no longer retain input authority; semantic panel
  terminals resize before child drain; `NoMessage` retains a published
  band baseline while `Invalidated` clears it; wheel activation follows
  the terminal-only focus rule; and legally wide panels clamp their PTY
  content without disappearing.
- **Review round 2 closed two findings at `bfaaf2b` plus this ledger
  commit.** Side affinity can replace the buffer while preserving the
  `WindowId`, so retained panel statusline segments are now keyed by the
  full `(WindowId, BufferId)` presentation. Every authoritative `Absent`
  also clears that baseline, including duplicate-suppressed `Absent`, so
  a later `Present` under `NoMessage` cannot resurrect peer state that
  was already cleared. Two acceptance tests bite those exact transitions.
  This lane and `docs/agent-handoff.md` now name the open PR, current
  landed base, checkpoint, and 2B-3 ordering instead of calling 2B-2
  merely “next.”
- **Round-2 verification at code checkpoint `bfaaf2b`:** formatting and
  strict workspace Clippy; library **1,863 passed + 3 ignored** default
  and **2,048 passed + 4 ignored** CRDT; bottom-panel Stage 1 / 2A /
  2B-1 / 2B-2 **46 / 17 / 16 / 28**; statusline **8 CRDT**; semantic
  routing **2 CRDT**; M4 **121 passed + 3 ignored + 1 filtered**;
  required GPU **202/202**; isolated-config full workspace sweep; and
  `git diff --check`. The first workspace sweep had one GPU rendering
  failure in `failures_and_display_math_render_as_source`; that test had
  passed in the immediately preceding required-GPU run, passed alone,
  and the complete workspace rerun passed. Real-daemon and managed-attach
  cases were rerun outside the tool sandbox after its local-socket policy
  produced `Operation not permitted`; the authoritative reruns passed.
- **Cross-machine recovery (fresh clone):**

  ```sh
  git fetch githubsucks --prune
  git switch --track -c bottom-panel-stage2b2 githubsucks/bottom-panel-stage2b2
  git rev-parse HEAD
  ```

  #187 has landed, so `githubsucks/main` already contains this work and
  the branch is retained only for provenance. Start 2B-3 from `main`.
- **Stage 2B-1 MERGED as #184** (`main` @ `6bee09d`, 2026-07-28; all
  twelve checks green on the reviewed head `5539b6e`; two review rounds
  plus a gate-found follow-up). Branch
  `githubsucks/bottom-panel-stage2b` and worktree `../pmacs-bp-stage2b`
  are retained and carry nothing unmerged. Durable facts — the
  schema-support-versus-advertisement split, the shared `wire_grid`
  boundary, authoritative `Absent`, and the two epochs — are in
  `docs/agent-handoff.md` §1 per rule 3, not here.
- **What 2B-1 deliberately did not do**, because 2B-2 and 2B-3 must not
  re-litigate it: no producer, no consumer, no capability change.
  `panel_capable` is still `false` for every semantic session, so the
  journey grade is unchanged and every shipped v20 client remains
  attachable.
- **2B-3 inherits a hard constraint from 2B-1's review**: it owns a
  *compatibility-preserving* v21 activation mechanism and may **not**
  simply change the unsolicited `Hello` to 21. The handshake is
  server-first, so that one-line change locks out every shipped v20
  frontend before it can even send an `AttachRequest`.
- **Two review rounds, and what each cost.** Round 1: `PanelFrame`
  needed an explicit `buffer_id`, the transport ratchet had to drive the
  real attach path rather than a detached codec assertion, and shared
  grid bounds needed one validator. Round 2: the server-first `Hello`
  made the advertised v20↔v21 compatibility one-way; `COHERENCE.md` and
  the handoff still named only v20 schema support; framing §9 named a
  nonexistent aggregate 2B suite instead of the three exact slice
  suites; and the panel plus copied-terminal "one byte over" fixtures
  were actually two bytes over.
- **The full gate — not review — found two version-ladder omissions and
  one probe contradiction.** The statusline and Vterm Stage 3 ladders
  still pinned v20 and rejected v21. Separately, Vterm Stage 3's
  headless probe exited its loop as soon as resize plus two nonuniform
  composites were observed, while its acceptance later required the PTY
  child's `VTERMROW` output in the final frame; the v20-compatible
  handshake made that scheduling race deterministic, so the report
  sampled a blank frame. The probe now waits for the exact child-output
  observation its acceptance asserts.
- **The probe fix then leaked its own fixture, and that is the reusable
  lesson.** The generic runner hard-coded the *producer* fixture's
  `VTERMROW` breadcrumb, so the CAT input fixture could satisfy every
  assertion yet never satisfy the loop exit — it waited out the
  20-second safety deadline and passed on the deadline. Producer probes
  now name their own required frame text while input probes finish on
  the latched echo, and the report exposes `completion_observed` which
  **both** paths assert, so a deadline-driven pass cannot hide a stall
  again.
- **Final verification on the reviewed head:** `cargo fmt --check`;
  strict workspace Clippy; library **1,849 passed + 3 ignored** default
  and **2,034 passed + 4 ignored** CRDT; bottom-panel Stage 1 / 2A /
  2B-1 **46 / 17 / 16**; folding Stage 2 **48**; GPU font **11**;
  statusline **8 CRDT**; m11_5 semantic **2 CRDT**; GPU initial target
  and invocation **15 / 15 CRDT**; the handshake consumers m5_5 / m5_7 /
  mode-system wiring **36 / 7 / 1 CRDT**; Vterm Stages 1 / 2 / 3
  **10 / 6 / 9 CRDT**, including the required real daemon + real PTY +
  real wgpu probe; M4 **121 passed + 3 ignored + 1 filtered**; required
  GPU **202/202**; the isolated-config one-invocation full workspace
  sweep; and `git diff --check`.
  - Retained as classification rather than erased: the first
    required-GPU pass was **201/202** on
    `a_fraction_draws_rule_pixels_between_its_operand_rows`, a rendering
    test structurally outside a protocol-only diff, which passed
    immediately in single-threaded isolation and **202/202** on the
    mandatory complete rerun. Separately, library and Vterm attempts
    *inside the restricted tool sandbox* produced `Operation not
    permitted` failures in socket-based attach tests; the authoritative
    outside-sandbox reruns passed.
- **Ordering for the rest of the arc is fixed:** 2B-2 branches from
  landed `main`; 2B-3 branches only after 2B-2 lands; Stage 3 (the
  adopter default flip) last. Each slice starts fresh from landed main.

- **Stage 2A MERGED as #177** (`main` @ `0a3fcd1`, 2026-07-26, all twelve
  checks green at `8424172`, three review rounds). Branch
  `githubsucks/bottom-panel-stage2a` and worktree `../pmacs-bp-stage2a`
  are retained and carry nothing unmerged. Five commits: the classified
  census routing, the painter extraction + acceptance, the lane record,
  then the round-1, round-2 and round-3 review fixes. **No protocol
  change; no behavior change for any frontend today** — with
  `panel_capable = false` for semantic sessions,
  `primary_document_window` returns `view.active` in every existing
  configuration, so this is seam adoption that becomes load-bearing in
  2B.
- **Stage 2A verification on its merge result:** `cargo fmt --check` clean; strict
  workspace Clippy clean; **1,832 default + 2,015 CRDT** library tests;
  `bottom_panel_stage2a_acceptance` **17**; bottom-panel Stage 1 46;
  statusline segments 8 CRDT; m11_5 semantic 2 CRDT; GPU initial target
  14 CRDT; terminal config 12 CRDT; vterm Stage 1/2 10 / 6; folding
  Stage 2 48; M4 121; required GPU 202; `git diff --check` clean.
- **Every routed producer is now pinned at a seam its production caller
  uses, and each pin was falsified by revert**: #1 follow, #2 lazy CRDT
  upgrade, #3 `CursorByte`, #5 decorations, #7 `Viewport` (aligns
  without focusing), #8 `Pointer` (aligns and focuses), #9 the
  terminal-context gate, #12 statusline, #21 the publication filter,
  plus the focus-class negatives. #1/#3/#21 required extracting three
  named helpers, because their only production caller is
  `dispatcher_loop`, which no test can drive.
- **Three lessons about the TESTS, not the code, all from review:**
  (a) a *structural* test comparing the two authorities directly does
  **not** catch a misrouted consumer — only consumer-level assertions
  do; (b) a daemon-path test must `register_session` or the event is
  dropped at the uninstalled-session check before reaching the code
  under test; (c) a discriminating fixture must make the two routings
  DISAGREE — comparing two non-terminal buffers, or two windows with no
  selection, yields the same answer either way and proves nothing.
  Round 2 found four of my own pins vacuous by exactly these shapes, and
  round 3 found two more problems of the same family: a pin placed at a
  HELPER while production called it from a producer (reverting only the
  producer's call site left every test green), and a socket-pair
  assertion whose blocking read made a regression HANG instead of fail.
  Both now assert at the producer, with read timeouts on every read.
- **Review round 1 closed: 4 P1 + 2 P2, all real.** The P1s were a
  stale-`Pointer` focus steal (the failed-alignment arm returned the
  window, so #8's activation focused it before `dispatch_pointer`
  rejected the buffer), the missing A2A-2 two-context fan-out, a census
  suite that asserted the AUTHORITY rather than the CONSUMERS, and the
  missing main integration. **Two of the new pins were themselves
  vacuous on the first attempt** — the dispatcher test passed because an
  unregistered session is dropped at `daemon.rs:1962` before reaching
  the aligner, and the painter test was a fixed-point check that
  survived deleting `text_view.render`. Both now fail under their own
  bite.
- **`vterm_stage3_acceptance::a37` is a pre-existing flake here**, not a
  Stage 2A regression: measured **6/8 failures on the base commit** and
  **7/8 on the branch** in matched isolated samples. It needs a real
  daemon + real PTY + headless GPU and is documented load-sensitive.
  It also silently returns `ok` unless `pmacs-gpu` has been built, and
  is `crdt`-gated so CI never runs it at all.
- **Two suites are dark without `--features crdt`**:
  `m11_5_semantic_acceptance` reports **0 tests** and
  `gpu_initial_target_acceptance` reports **1** in the default config.
  Both are semantic-census suites, so Stage 2A must be gated with the
  feature on or its most relevant coverage never executes.

- Stage 1 merged as **#155** (`main` @ `e745068`, 2026-07-24, after two
  review rounds). No protocol change. Durable substrate facts live in
  `docs/agent-handoff.md` §1; the two round lessons are in §5.
- Landed-docs follow-up merged as **#156** (`main` @ `d152120`,
  2026-07-25).
- **Stage 2 framing: `docs/bottom-panel-stage2-framing.md` revision 6**
  is on branch `githubsucks/bottom-panel-stage2b` (revision 5 is commit
  `56301ed` there),
  worktree `../pmacs-bp-stage2b`. Revisions 1–4 remain on
  `githubsucks/bottom-panel-stage2-framing` (head `4fbd47f`, four
  framing commits, revision 4 at `49757e5`). Round 1 closed 2 blocking +
  3 high;
  round 2 closed 1 blocking + 2 high + 1 medium and decided both open
  items; round 3 closed 1 blocking + 1 high + 1 medium. No open items
  remain. Revision 5 adds no decision; it records the approved
  2B-1/2B-2/2B-3 implementation split. Revision 6 corrects the
  server-first compatibility contract, durable protocol claims, exact
  acceptance-suite names, and `limit + 1` fixture. The
  parent framing `docs/bottom-panel-framing.md` (rev 4) remains
  authoritative, **including its acceptance criteria 37–55**.
- Retained, carrying nothing unmerged: branch `bottom-panel` and worktree
  `../pmacs-bottom-panel`.
- **Stage 2 ships as four serial implementation slices**, each landing
  before the next branches:
  **2A** = classified §1.3 census routing + `paint_frame` per-window
  painter extraction (with the active-window auto-scroll preparation), no
  protocol change; **2B-1** = reserved protocol schema **v21**, with
  production advertisement held at v20,
  (`InstanceMessage::PanelFrame` plus
  `FrontendEvent::{FrontendCellGeometry, PanelResizeRows, PanelPointer}`,
  gated both directions, each extended enum byte-pinned on its own
  previous final variant); **2B-2** = daemon panel projection and epoch
  machine; **2B-3** = compatible v21 activation, the GPU band, and the
  negotiated `panel_capable` flip.
  Stage 3 is the adopter default flip.
- **Correction — this entry previously mis-stated the census contract.**
  It is **not** "route every consumer through `primary_document_window`".
  Q#BP14 classifies the 23 reads into four classes and routes only the
  **Projection** class that way; focus/input (#13–#15, #23), focus chrome
  and surface-routed (#16–#19), and focus/session (#20) keep their own
  authorities. Rerouting them would break remote-op validation and
  application, `DispatchIdle`, presence, focused search/menu/completion
  routing, and terminal bell ownership. The Stage 2 framing carries the
  full table.
- **The GPU document bottom is three boundaries, not one.**
  `text_area_bottom` (`pmacs-gpu/src/main.rs:8490`) is today
  `status_band_top`, `geometry_capacity_bottom`, and
  `document_text_bottom` at once. Once a band is installed they diverge:
  the status chrome must stay pixel-identical at the physical window
  bottom while document consumers move. A blanket rewrite of that helper
  moves both together and passes an "everything moved" assertion, so the
  Stage 2 criterion asserts **both directions in one scenario**. The
  census is 20 production sites (8 status-owned, 12 document-owned) + 1
  definition + 8 test sites = 29 matches; the framing carries the
  per-site table. The three easiest to misclassify are document
  completion `:6140`, minibuffer candidates `:7351`, and edge scrolling
  `:8561` — each with its own visible symptom.
- **Folding Stage 3 and this arc's Stage 2 both touch the semantic
  projection.** Whichever is framed second re-scouts the other's landed
  state.

## Folding lane (Arc 6) — Stages 1 and 2 MERGED; Stage 3 (GPU) is next

Both shipped stages are on `main`; nothing in this arc is in flight. Stage 3
has **no branch and no framing yet**.

- Stage 1 (headless fold engine) merged as **#142**, Stage 2 (grid/daemon
  collapse) as **#149** — both under "Closed since the last snapshot".
- Retained, carrying nothing unmerged: branches `folding` / `folding-tui`
  and worktrees `../pmacs-folding` / `../pmacs-folding-tui`. The framings
  `docs/folding-framing.md` (rev 5) and `docs/folding-stage2-framing.md`
  (rev 4) are the approved artifacts Stage 3 re-scouts against.
- **Stage 3 (GPU) obligations, already named by the framings** — the
  starting point for its own framing doc: GPU collapse at TUI parity;
  caret/hit-test fold-awareness; the `BufferSnapshot` **fold-mirror clear**
  (parent R2-4 — without it, empty-after-revert diff suppression leaves
  stale folds on the GPU, the same trap class as #120); CRDT-origin and
  GPU-optimistic interactive unfold (parent R2-3); and flipping
  `FrontendView.fold_projection` to `true` for semantic frontends, which
  Stage 2 deliberately left `false` (Q#FD21).

## Resource-op delete guard implementation — PR #190 OPEN, review round 2 closed

- Portable branch: `githubsucks/resource-op-delete-guard-impl`, worktree
  `../pmacs-rd-impl`. Implements the framing merged as #186
  (`docs/resource-op-delete-guard-framing.md`, revision 5 plus its new
  §§9-10). Position against `main`, as pasted command output rather than a
  remembered constant — **`main` moved while this lane was being
  written**:

  ```
  $ git merge-base HEAD githubsucks/main
  b7bf2c664475c36b60cf7c0361ea75cd3c3b2315
  ```

  That is the whole durable fact, and it is deliberately the ONLY
  number pasted here. **An ahead-count cannot be recorded in the file
  it counts**: writing it is a commit, so the value is stale by one the
  instant it is written, and the previous attempt at this entry proved
  it — a pasted `4 0` read `5 0` at the pushed head. Run
  `git rev-list --left-right --count HEAD...githubsucks/main` when you
  need it; the merge-base above is what tells you whether the answer is
  still meaningful.

  **Re-measure the merge-base too before relying on it.** `main` moved
  twice while this lane's round-1 fixes were being written (#192 and
  #193), then twice more during round 2 (#188 and #194). This branch
  integrates through #194. `main` has branch protection now, so a stale
  base is not merely untidy: all 12 checks must pass on the merging head,
  and a conflicting PR builds no merge ref at all, so a green run from
  before the move reads as current when it is not.
- **The framing's §8 branch plan is superseded and cannot be followed.**
  It says "one PR — #186, which becomes the implementation PR", written
  when #186 was still open. #186 merged as framing-only, so the
  implementation got its own branch and PR. Nothing about the decisions
  changes; only the branch plan. Both the framing's header and its §8 now
  say so on their own pages.
- **Layer 1 (the primitive) and Layer 2 (the applier + server-request
  boundary) are both complete and both pinned through their production
  paths.** The Layer 2 gap the first commit named — criteria 11, 11a-11d,
  12, 13, 15 having no production-path pin — is closed.
- **Review round 1 found four defects; all four are fixed and all four
  are recorded in the framing's new §9**, because two of them were
  corrections *to that document*, and a correction living only in a test
  comment is invisible to the next reader of the framing:
  - **P1 §9.3 — the preflight broke ordered resource ops.** Every delete
    was judged against the filesystem's *initial* state, so a valid
    `create X -> delete X` (or `rename A -> B -> delete B`) was refused
    with a fabricated `NotFound` about a path the batch was about to
    create. A regression this lane introduced. **Decision: defer, do not
    simulate** — a delete whose target is related by component-aware path
    containment to a path an *earlier* op creates, renames, or removes is
    left to the primitive. Q#RD3 already calls the check a filter, not a
    transaction. `edit` ops are deliberately excluded, so the
    buffer-and-filesystem half still fires early for untouched targets
    (criterion 11c depends on exactly that).
  - **P1 §9.5 — the required production-boundary acceptances were
    missing.** Landed: 11, 11a-11d, 12 (both directions), 13, 15.
  - **P1 §9.4 — mid-batch failures were misreported as complete aborts.**
    `apply_workspace_edit` now returns `nil, message, applied_op_count`,
    and ONE renderer serves both the status line and the server's
    `failureReason`. All three callers updated.
  - **P2 §9.2 — non-recursive deletes inspected descendants.** `recursive`
    is now a parameter of the shared query. The counterexample is an
    orphan: a modified buffer at `tree/gone.rs` whose file is already gone
    blocked a non-recursive delete of the now-*empty* `tree/`.
- **Review round 2 found two more defects; both are fixed and recorded in
  the framing's new §10:**
  - **P1 §10.1 — dependency filtering compared raw path spellings.**
    `create dir/./x -> delete dir/x` was wrongly preflight-refused even
    though both operations name the same lexical path. The comparison
    now runs both sides through the registry's existing lexical
    `pmacs.path.canonicalize` normalizer before component-aware
    containment. This is deliberately comparison-only: operation
    execution still receives the server's original path, and no
    filesystem/symlink canonicalization was added.
  - **P1 §10.2 — a failing first plan item could mutate while reporting
    “nothing was mutated.”** `apply_workspace_edit` now returns an
    `execution_started` fact in addition to the completed-item count.
    Only parse/plan/preflight failures claim that nothing changed. Once
    execution starts, the shared renderer conservatively says the
    failing operation may have changed state. Criteria 22a and 22b pin
    both forms: a multi-edit text item whose first edit lands before its
    second edit fails, and a resource rename that creates destination
    parents before the filesystem rename fails.
- **`delete_verdict` is narrowed, and #171 inherits the narrowed
  version.** Q#RD6's shared query is this lane's to own; descendant
  matching is now reserved for recursive deletes. Q#RD5's "inspect widely,
  mutate narrowly" is unchanged in substance — "widely" means the set the
  op can actually destroy.
- **Criterion 3's stated bite: fixed by fixing the SETUP, not the doc.**
  The framing says it fails against buffer-first ordering. Against the
  first shipped setup it did not (a directory target with no buffer bound
  to it), and §9.2's narrowing would then have left that setup with no
  bite at all. The buffer is now bound to the *exact* deleted path — a
  file opened, then replaced on disk by a non-empty directory, so a
  non-recursive `remove_dir` fails with `ENOTEMPTY` deterministically and
  under any uid. Both stated pre-images now bite, so the framing's wording
  needed no amendment after all.
- **The fake is one parameterized mode, not eight.**
  `PMACS_FAKE_LSP_MODE=applyeditplan` reads its whole `WorkspaceEdit` from
  `PMACS_FAKE_LSP_EDIT_PLAN` and publishes the client's response to
  `PMACS_FAKE_LSP_APPLYEDIT_SINK` (written `.part`-then-rename, so a
  polling reader never sees a partial record). Fail-closed: an unreadable
  plan sends no `applyEdit` and reports itself through the sink.
  `pmacs_fake_lsp` is a cargo BIN resolved through
  `env!("CARGO_BIN_EXE_...")`, so every CI leg builds it and a missing
  binary is a build failure — there is deliberately no
  skip-and-return-ok arm.
- **Criterion 15's stub is hosted in `m4_acceptance`, and the gate list
  moved with it.** `lsp_dispatch_seams_acceptance` is struck from the
  framing's §7 gate list AND its §8 touch table in the same edit, under
  §8's permitted simplification. It is still *run* as a gate, because
  `builtin/runtime/lsp.lua` changed.
- Acceptance: criteria 1-16, §9's 18, 19a-19c and 20, plus §10's 21 and
  22a-22b, all in `tests/m4_acceptance.rs` and prefixed `rd`. 28 tests.
- Bite verification uses `scripts/bite` **with the positive control** it
  gained in #192, merged into this lane. The pre-image for the round-1
  fixes is this lane's own first commit `1873be6`, not `main` — those
  defects were introduced by it. Per-criterion results are in the commit
  message. The round-2 criteria 21, 22a, and 22b each pass on the
  round-2 code checkpoint `cb7fe81` and produce a clean assertion
  failure against its pre-image `c804dd5`.
- Gates green at the round-2 tree: fmt; clippy `-D warnings`; `--lib`
  **1863**; `--lib --features crdt` **2048**; `m4_acceptance` **149**
  passed, **3** ignored, **1** filtered; `lsp_dispatch_seams_acceptance`
  **15**; `dired_acceptance`
  **25** and `autosave_acceptance` **29** (the framing's watch items);
  required GPU **202**; full isolated-config workspace sweep; `git diff
  --check` clean. The only warning in the non-Clippy CRDT build is the
  pre-existing `unused_mut` in `src/daemon.rs`; strict Clippy is clean.
- Recovery from a clean checkout:
  `git fetch githubsucks && git worktree add ../pmacs-rd-impl
  -b resource-op-delete-guard-impl githubsucks/resource-op-delete-guard-impl`.

## dired Stage 2a — rename/delete reconciliation — PR OPEN

- Portable branch: `githubsucks/dired-stage2-impl`, worktree
  `../pmacs-dired-s2`. Implements **Stage 2a only** of the framing merged
  as #171 (`docs/dired-stage2-framing.md` rev 9, §5/§6/§10 — the
  substrate transaction, no dired surface). Position against `main`, as
  pasted command output rather than a remembered constant:

  ```
  $ git merge-base HEAD githubsucks/main
  e003b81cdd577140fc77330bd4578d3090696877
  ```

  That base is the #190 merge, and #190 matters here specifically:
  Stage 2a **adopts** its `delete_verdict` refusal rather than
  reinventing one, and lifts its walk query out into
  `editor_core::buffers_bound_under` so the guard and both
  reconciliation seams cannot disagree about which buffers an operation
  touches. **Re-measure the merge-base before relying on it** —
  `main` has branch protection, all 12 checks must pass on the merging
  head, and a conflicting PR builds no merge ref at all, so a green run
  from before a move reads as current when it is not.
- **What 2b and 2c still owe, stated so the split boundary is auditable.**
  2a ships **no user-visible surface at all** and no dired code: the
  `dired_acceptance` count is deliberately unchanged at **25**, and a
  moved count there would mean it touched something it should not have.
  2b owes the mark and operation layer (`m u U t d x D R w M`),
  `pmacs.minibuffer.confirm` plus its `src/editor.rs` load-sequence line,
  `pmacs.killring.push`, dired's own `resource.renamed` subscriber, and
  acceptance 1–22, 33, 39–41. 2c owes `mkdir`/`copy`/`remove_dir_all`,
  `JobKind` 12 → 15, `dired.recursive-deletes`, and acceptance 42–47.
- **The split boundary has not moved since rev 9.** It was re-checked
  against this tree: #188 (generated-buffer immutability Stage 1) did not
  convert dired's `paint`, so §3.1's coordination note is still an
  obligation of that lane rather than a collision with this one, and
  nothing in this diff touches `builtin/runtime/dired.lua`.
- **Two m4 rows were re-pinned, and that is a behaviour change to a
  landed lane's assertions.** `rd9` and `rd14` pinned #190's deliberate
  restraint on the `apply_resource_op` delete arm — descendants stay
  orphaned, only the first of two duplicate path-bound buffers is
  reconciled — and both doc comments gave the same reason: widening
  would have routed N buffers through `remove_buffer_and_fire`, phase 2
  without phase 1, leaving up to N windows on removed ids.
  `EditorCore::reconcile_delete` composes both phases, so the constraint
  is discharged and the old assertions became the defect. Each row now
  asserts BOTH directions — reconciled away **and** no window holding a
  removed id — and each direction is bite-verified.
- **One framing claim is wrong and is corrected at the test, not
  silently worked around.** §5's G1 says a stale captured path
  "materializes a phantom" by reaching `resolve_target_buffer`'s
  `NotFound` arm. It does not: `pmacs.buffer.find_or_open` calls
  `crate::file_io::load_file` directly and maps the error, so a missing
  path **raises**, and the `NotFound` arm belongs to
  `resolve_target_buffer`, which serves `pmacs.window.display_file` and
  the startup/daemon target rather than that binding. The defect is real
  and smaller: the `pcall` swallows the raise, so the user is stranded
  wherever the last applied op left them. Acceptance 34 is restructured
  to bite on that (its plan edits another file first, which is what makes
  the restore observable at all) and the correction is recorded in the
  test's own doc comment.
- **Two bites were vacuous as the framing specified them, and both
  reasons are worth keeping.** Item 28's *rename* row cannot pin the
  walk's containment rule: `reconcile_rename` calls
  `Path::strip_prefix` to rebuild a descendant's tail, and that is
  component-aware too, so a string-prefix walk is silently corrected a
  second time. The row moved to the **delete** side, where the walk's
  verdict IS the kill list. Item 30's composition-order assertion was a
  tautology: the LSP attach leaves `diagnostic` **last** in the stack, and
  moving the last element to the end is a no-op, so a remove-and-re-push
  was indistinguishable from an in-place mutation; the row now pushes one
  more overlay after it and asserts that precondition explicitly.
- **23 acceptance criteria are bite-verified by executed mutation**, each
  labelled `OK (assertion)` — none merely `OK (COMPILE)`, and none
  vacuous. Items 25, 27, 28, 29 (both directions), 30 (both mutations),
  31, 31b (both gates), 31d (both halves), 34, 50 (both mutations), 51,
  52, 53b, 54, 55, plus the two re-pinned m4 rows in three
  configurations.
- Verification at this head, each gate run to its own file and its own
  exit code checked (never through a pipe): `cargo fmt --check` clean;
  `cargo clippy --workspace --all-targets -- -D warnings` clean;
  `cargo test --lib` **1,875** passed / 3 ignored; `--lib --features
  crdt` **2,060** / 4 ignored; the new
  `resource_reconciliation_acceptance` **24** default and **24** crdt;
  `dired_acceptance` **25** and **25** crdt, deliberately unmoved; the
  frozen additivity gate `m8_1` **10** / `m8_2` **15** / `m8_3` **32**,
  all unchanged; `m4_acceptance -- --skip basedpyright` **149** passed /
  3 ignored / 1 filtered; `lsp_multi_root_acceptance` **13**;
  `lsp_dispatch_seams_acceptance` **15**; `journey_acceptance` **24**
  (the ratchet floor, asserted as a count rather than a colour);
  `gpu_invocation_acceptance` **15** crdt — **and that number is only
  real with `pmacs` and `pmacs-gpu` built first**, which is the `a37`
  trap in §5: the same command reported 12 failures before the build and
  15 passes after, so a red run there is not evidence of a regression
  until the binaries exist; `PMACS_REQUIRE_GPU=1 cargo test -p
  pmacs-gpu` **202**; isolated-`XDG_CONFIG_HOME` workspace sweep with
  `--no-fail-fast` **3,557** passed across **104** suites, 19 ignored, 0
  failed; `git diff --check` clean.
- **Ownership, per the framing's own warning.** §16 says 2a must not run
  concurrently with **Journey Stage 1b**, because 1b's LSP
  spawn-failure reporting lands in `builtin/runtime/lsp.lua`'s
  attachment lifecycle and 1b's compile/binding half touches
  `src/editor_core.rs` — the same two files 2a rewrites, where the
  conflicts are semantic rather than textual so a clean `git merge`
  proves nothing. **1b must not be started while this PR is open.** No
  other lane in flight touches them: #188 is `dired.lua`/`buffer.rs`
  generated-buffer writes, and the bottom-panel and CI lanes are
  elsewhere.
- Recovery from a clean checkout:
  `git fetch githubsucks && git worktree add ../pmacs-dired-s2
  -b dired-stage2-impl githubsucks/dired-stage2-impl`.

## Generated-buffer immutability framing lane — PR #188 OPEN, PROPOSED

- Portable branch: `githubsucks/generated-buffer-immutability`; worktree
  `../pmacs-generated-immutability`. **PR #188**, base `main`, forked from
  `githubsucks/main` @ `ad41cf1`, **integrated through `5e186c7`** —
  #189 (clean), then #186 and #171 (`docs/active-work.md` conflict),
  then #187 (the same file again, after it removed the two landed
  framing lanes), #192 at merge commit `76cfaac`, and #193
  (`docs/active-work.md` conflict again) after revision 7's first push.
  Revision 6 was reviewed at head `55c3061`; revision 7 closes that
  round. Framing only —
  `docs/generated-buffer-immutability-framing.md`, revision 7, plus this
  lane. **No runtime code, no protocol change.**
- **PROPOSED — six review rounds closed (thirty-two findings,
  twenty-two P1, ten P2). Not approved. Do not implement, do not merge.**
- **Stage 1 implementation is PR #191, open. The boundary is explicit
  and has already been needed twice:** #188 owns the **acceptance
  contract**; #191 **adopts** criteria and may not restate, narrow, or
  reclassify them. Where an implementation finds a criterion impossible,
  the framing is revised and re-approved first. The **selection-anchor
  clamp** is Q#GB6's to specify and #191's to implement, and both must
  describe the same rule. Round 5 found #191 had locally restated Stage 1
  criteria 5 and 7 while #188 still carried the originals — a divergence
  neither lane's gates can catch, because each is green against its own
  description.
- **Round 5's two corrections that other lanes need:**
  - **`Window::Selection::anchor` is an unclamped byte position and it
    PANICS**, not merely dangles. `Window::region` (`src/window.rs:472-479`)
    clamps neither endpoint; `region_bytes` (`src/editor_core.rs:4184-4191`)
    hands the result to `Rope::slice`, which asserts at `src/rope.rs:145`.
    Reproduced by #191: select 0..30, shrink the buffer to two bytes,
    copy. The fix is **clamp-or-clear** in **both** `notify_buffer_edit`
    (`src/editor_core.rs:1836-1850`, clamps nothing today) and
    `rebuild_views_for` (`:1865-1882`, clamps cursor and view_top but not
    selection) — and the rule is already in the tree for the terminal's
    own selection type at `src/terminal/view.rs:715-721`. A stale anchor
    also reaches the presence broadcast (`src/presence.rs:122-123`).
  - **`Buffer`'s `on_edit` broadcast stops at the first error in FOUR
    places**, so a view later in attach order keeps pre-edit offsets:
    `src/buffer.rs:1288` (`apply_edit` / `apply_edit_skip_intercepts`),
    `:1250` (the no-op early-return arm), `:1033`
    (`apply_remote_crdt_op` — the replica import path), and `:1543`
    (`broadcast_on_edit`, i.e. **undo and redo**). Any lane relying on a
    buffer-attached view staying in step with the rope is affected.
- **What it frames.** The class-wide half of the `set_generated_contents`
  invariant that `docs/agent-handoff.md` §4 and `COHERENCE.md` §14 both
  record as unfinished: `Buffer::undo` gates on `ensure_writable()`
  (`src/buffer.rs:1302`) and never consults the intercept chain, so the
  `add_intercept`-plus-`bypass_intercept` idiom leaves the rope writable
  and every affected buffer emptiable. All five families were reproduced
  by execution at `ad41cf1`, not inferred; the transcripts are in the
  document's §0 and §2.
- **Recommended primitive:** `Buffer::apply_generated_edit(op)`, exposed
  as a `{ generated = true }` option on the existing Lua mutators, with
  `set_generated_contents` reimplemented as its whole-buffer wrapper. It
  is the only candidate in which the buffer is never observably unlocked.
  **Revision 3 pins the transaction** (framing §3.4): its **own**
  `run_buffer_edit` arm — **not** the bypass arm, which calls
  `begin_edit`, which calls `ensure_writable` first (`src/buffer.rs:725`)
  and would refuse every generated write to a locked buffer — one
  `&mut Buffer` method with every exit named.
  **Revision 4 replaces revision 3's cleanup predicate.** Cleanup is
  driven by an explicit five-variant `GeneratedOutcome` reported by the
  apply, **not** inferred from `revision`. Inferring it was wrong three
  ways: a successful no-op (`src/buffer.rs:1245-1253` returns `Ok`
  without bumping `revision`) kept history the contract forbids; a CRDT
  mid-transaction failure happens **upstream of `revision` entirely**
  (`:1140-1163`), so it was neither cleaned nor detected; and the
  unconditional relock **locked a fresh buffer that was never
  successfully written**. `NoOp` clears, `Rejected` restores the entry
  lock state, `Diverged` clears nothing and surfaces. **Revision 5 keeps
  the five outcomes but preserves the `Edit` in
  `AppliedThenFailed { edit, error }`: the borrow-free Lua finisher fans
  it out to window caches and replica mirrors before returning the
  error.** Collapsing to `Result` inside `Buffer` was too early.
  **Revision 6 replaces the delete→insert enumeration with a
  `crdt_mutated` flag**, because `export_updates_since` can fail after
  every successful CRDT op shape, and changes all four `Buffer`
  broadcast loops to continue-and-retain-first-error.
  **Revision 7 installs quarantine at common divergence detection
  before generated or ordinary callers map the outcome.** The monotonic
  poison blocks all three CRDT snapshot exporters, daemon-origin
  queueing and every later owner-generated write; criteria 16c and 16d
  pin the containment boundary. Repair remains deferred.
- **Two stages, two PRs.** Stage 1 — listview ownership fix **plus its
  identity-routing fix in the same PR**, dired and listview adopting the
  shipped primitive, the window-coordinate clamp, and the fold decision.
  Stage 2 — the new primitive, compile's nine write sites, the search
  panel's four, compile/search ownership + routing, the path-backed
  refusal plus `mark_clean`, and the terminal-only
  `identity_protected` guard. **No Lua unlock ships.**
- **Nine facts from this lane that other lanes need before it merges:**
  - **`bypass_intercept` is the wrong inventory key.** It misses
    `*buffer-list*`, `*help*` and `*workers*`, which are generated with
    plain writes and no intercept at all. `docs/agent-handoff.md` §4's
    four-row table inherits that blind spot — **and undercounts by one**:
    `src/help.rs:354` `replace_help_buffer` is a fifth writer mechanism
    (own find-or-create, `Buffer::apply_edit`, own `mark_clean`) writing
    the **same** `*help*` buffer as `default.lua:1239`, which does not
    mark clean. Two owners, one buffer, two copies of the name constant
    across the FFI boundary.
  - **`COHERENCE.md` §14's listview consumer list was wrong and is now
    FIXED** — PR #189 (`main` @ `7586905`) landed exactly the correction
    this lane measured. Nothing owed. Recorded so it is not re-asserted.
  - **Three writers adopt any buffer sharing their name** —
    `listview.lua:95`, `compile.lua:263`, `default.lua:861-868` — against
    a rule the tree already states at `terminal.lua:300-305` and
    implements at `dired.lua:476-504`. Measured: a foreign
    `*references*` is clobbered and left permanently un-editable, and a
    `pmacs.compile.run` that **raises on validation** still leaves a
    foreign `*compilation*` un-editable. Today `M-x buffer.undo` — this
    arc's bug — is the only recovery, so the arc must not lock these
    buffers before fixing ownership.
  - **Disambiguating a name breaks the sites that read one.** Census in
    framing §2.10: 19 units across 14 grep lines, two genuinely broken.
    `listview.lua:44`'s `panels[d.name]` (written under the *requested*
    name at `:97`, read under the *actual* name) has **four** consumers,
    and the fourth — `listview.open:118-123`'s never-capture-a-panel
    guard — fails **inverted and silently**, capturing a panel as its own
    `q` target. `compile.lua:216`'s `is_generated_buffer` has two.
    `compile.lua`'s `slots` is **not** affected: keyed by a module
    constant at both ends, with `slot_for_buffer` id-based.
  - **`read_only` is one boolean serving THREE policies** (framing
    §2.11): the generated lock; terminal identity
    (`src/terminal/session.rs:305`); and, as a *reader*,
    `src/lua_bindings/fold.rs:313`'s "is this a document buffer" test,
    pinned by `tests/folding_acceptance.rs:570`. Consequence for any
    lane: **locking a buffer silently disables `pmacs.fold.fold` on it**,
    with the status `fold rejected: not a document buffer`.
  - **The SHIPPED `set_generated_contents` can overwrite a live terminal
    identity buffer.** It does `self.read_only = false` unconditionally
    (`src/buffer.rs:546`), so it lifts a lock it did not install, writes,
    and re-locks. Present on `main`, untested, unframed anywhere before
    revision 4. Refused in Stage 2 by the `identity_protected` field —
    an **intrinsic** flag marked once by a crate-private monotonic
    `mark_identity_protected()` in `TerminalSession::open`, never written
    by `set_read_only`. Revision 3 tried to infer this from the lock's
    provenance instead; that broke the lift-and-restore idiom at
    `tests/terminal_copy_mode_acceptance.rs:578-584`, and the general
    lesson is that a **derived** fact must be maintained by every
    mutation of what it derives from — and `set_read_only` is `pub`.
  - **`acc16e` is `crdt`-gated and is the only shipped consumer of the
    lift-and-restore idiom.** `cargo test --test
    terminal_copy_mode_acceptance` **without** `--features crdt` never
    compiles it, so a green run of that suite proves nothing about the
    seam. Any lane touching `read_only` semantics must run it with the
    feature and confirm `acc16e` is in the count.
  - **`identity_protected` is not generated-lock provenance.** Revision
    4 tried to use “not a terminal identity buffer” as proof that the
    generated primitive installed the lock; it is not. Revision 5
    therefore removes `pmacs.buffer.unlock_generated` from the arc
    entirely. Wdired's future generated→editable transition remains
    dired Stage 3 work and must be owner-specific or use the eventual
    lock-policy enum.
  - **The CRDT `Replace` mid-transaction divergence is real and
    unowned.** `crdt.delete` then `crdt.insert` (`src/buffer.rs:1140-1163`);
    if the first succeeds and the second fails, the code's own comment
    says "the CRDT is mid-transaction ... This is an invariant
    violation." It reaches `apply_edit` and `apply_edit_skip_intercepts`
    today and is reported as an ordinary `CrdtRejected`, so nothing
    distinguishes it. This lane names and contains it; **repair is
    deferred and unowned.** Revision 6 makes classification total with a
    `crdt_mutated` flag: any later error, including
    `export_updates_since` after `Insert`, `Delete` or `Replace`, is
    `Diverged`. Revision 7 installs a common monotonic poison before
    either generated or ordinary API mapping, blocks
    `initial_target_snapshot`, `send_buffer_snapshots`,
    `export_buffer_snapshot` and `queue_daemon_origin_crdt_op`, and
    rejects every later owner-generated write with `CrdtQuarantined`.
    Criteria 16c and 16d fault-inject the complete boundary under
    `cargo test --lib --features crdt`; there is no public
    fault-injection API and no four-variant fallback.
- **Overlap warning.** Stage 2 touches `src/lua_bindings/mod.rs`'s buffer
  mutator bindings and `src/buffer.rs`. Do not run it concurrently with
  the `apply_resource_op` lane or the bottom-panel 2B work without
  assigning those files to one lane first. The framing itself touches
  neither.
- **Cross-lane, settled, not re-decided here.** #186 owns the urgent
  pre-filesystem refusal for synchronous `apply_resource_op`; #171 later
  owns full post-delete lifecycle reconciliation, including the async
  race where a buffer becomes modified after dired dispatch. **#171's
  Q#DR25 is deferred INTO this lane** — confirmed against #171 revision 7
  (`fd7ae37`), which states that dired's listing becoming immutable is
  "owned by the `generated-buffer-immutability` lane" and that "Stage 2
  does not implement it, does not gate on it, and carries no acceptance
  for it." This lane's Stage 1 claims that work. **Neither ordering
  conflicts**: #171 Stage 2b changes `paint`'s callers, this lane changes
  `paint` itself. Revisions 1 and 2 of this framing never mentioned
  Q#DR25 at all; revision 3 §9b records it.
- **Re-measured at `ad41cf1` while scouting: 276 CRDT-dark tests**
  (3,251 vs 3,527), by
  `cargo test --all-targets --no-default-features --features lua54[,crdt] -- --list | grep -c ': test$'`.
  Recorded here because the section above asks for exactly that and
  warns against quoting a stale figure; it does not replace that
  section's per-target census, which was not re-derived.

## Test-improvement arc, lane 2 — silent-skip arming

- Portable branch: `githubsucks/silent-skip-arming`, worktree
  `../pmacs-skiparm`. Implements `TEST_IMPROVEMENT.md` §1.2 and §5.4.
- **Base, measured at write time rather than quoted:**

  ```
  $ git log --oneline -1 githubsucks/main
  5e186c7 Merge pull request #193 from levineuwirth/test-improvement-audit
  ```

  The previous revision of this entry said "base measured at write
  time, pasted below" and then pasted nothing: the script meant to
  substitute it reported success and silently matched no text, and the
  claim was not re-read. Recorded because it is the same defect this
  ledger keeps catching one level up — **asserting a measurement is not
  making one, and a tool reporting success is not the measurement
  either.**
- Recovery from a clean checkout:
  `git fetch githubsucks && git worktree add ../pmacs-skiparm
  -b silent-skip-arming githubsucks/silent-skip-arming`.
- **The defect:** `let Ok(_) = which_binary(x) else { eprintln!(..);
  return; }` reports GREEN when the tool is absent, and CI installed
  none of the tools. A block of real-language-server and multi-shell
  tests had therefore **never once executed their bodies** in CI while
  reporting success. A suite that cannot distinguish "passed" from
  "never ran" is worse than a missing one, because it reads as
  coverage.
- **The fix is the project's own pattern.** `PMACS_REQUIRE_*` already
  makes a missing GPU fatal for `vterm_stage3_acceptance`; this adds
  `PMACS_REQUIRE_LSP`, `PMACS_REQUIRE_SHELLS` and `PMACS_REQUIRE_LUA`,
  plus the CI step that installs the tools. Per-tool variables, not one
  blanket flag, so a tool that must stay unarmed keeps its decision
  visible at the call site.
- **`basedpyright` is deliberately NOT installed and NOT armed.** Its
  test has no timeout and hangs forever — root cause is the
  non-interruptible reader-thread join in `RuntimeHandles::drop`,
  already a named deferral in `src/process.rs`. The `test` job has no
  `timeout-minutes` either. Arming it today would trade a vacuous green
  for a six-hour hang across four legs. `PMACS_REQUIRE_PYRIGHT` exists
  and is never set, so the flip is one line once lane 4 (the hang) and
  lane 3 (timeouts) land. **Do not arm it before both.**
- **A trap found while writing the workflow, not after:** the natural
  Actions idiom `${{ runner.os == 'Linux' && '1' || '' }}` sets the
  variable to the EMPTY STRING elsewhere, and `var_os().is_some()` is
  true for `Some("")`. That would have armed the guard on exactly the
  runners with no tools installed. The helper therefore treats empty as
  unset. `PMACS_REQUIRE_GPU` has the same latent shape and is safe only
  because it is set literally.
- **Verified by execution in all three states**, on a tool genuinely
  absent from this machine (`vscode-json-language-server`): unset ->
  skips green; armed -> hard failure naming the CI step; empty string
  -> skips green. The armed failure is the bite, and on `main` it
  cannot occur because no guard exists.
- **The tests pass when they actually run** — which was the open
  question, since none of them had. Armed locally: 11 `m6_5` + 8 `m6_8`
  REPL tests green, and all six real-LSP tests (clangd x2, gopls x2,
  rust-analyzer x2) green individually.
- **rust-analyzer is installed in the Linux-gated step, not via the
  toolchain action's `components:`.** The first revision put it there,
  which applies to *every* matrix leg — and **presence, not
  `PMACS_REQUIRE_LSP`, is what decides whether a gated test body
  runs**. That would have executed the two rust-analyzer tests on macOS
  for the first time ever, on the legs that are simultaneously the CI
  critical path and the documented flake surface, while this entry
  claimed Linux only. The variables not being set there would only have
  meant absence was tolerated; it would not have kept the tests
  skipped. Text and workflow now agree.
- **Tool versions are pinned** (`gopls@v0.16.2`,
  `vscode-langservers-extracted@4.10.0`,
  `yaml-language-server@1.15.0`). `@latest` and bare `npm install -g`
  make CI drift with upstream releases, so a bad publish breaks CI with
  no commit here to bisect against. Caching the built `gopls` on the
  pinned version is a follow-up, not done here.
- **§1.2 is NOT fully closed by this lane.** The guards arm the
  *entry* skip only. `tests/m4_acceptance.rs`'s mid-test rust-analyzer
  bail ("workspace likely still indexing; skipping") survives, so even
  armed, that test's only assertion can still vanish under load —
  precisely when a regression would show. Mid-test skips are their own
  shape and want their own pass.
- **Not this lane's to fix, recorded so it is not mistaken for
  oversight:** the generated-buffer immutability lane above still reads
  "PR #188 OPEN, PROPOSED" and #188 has merged. Rule 4 forbids
  relabelling it and permits removal only once its durable facts reach
  `docs/agent-handoff.md`, which #188 did not touch — it changed the
  framing and this ledger only. So the absorption is genuinely owed,
  and the natural carrier is the arc's own next PR (#191, Stage 1),
  not a testing lane reaching across into someone else's arc.
- **Follow-up owed after this merges:** delete
  `githubsucks/handoff-2026-07-20`. Removing the documentation lane
  removes the only pointer to that branch, so nothing will otherwise
  remind anyone it still exists on the remote.
- Linux only for now, deliberately: macOS needs the brew equivalents
  and roughly doubles install cost on the slowest matrix leg. The
  variables stay unset there, so those tests skip cleanly.
- Also removes the **documentation lane**, whose disposition the ledger
  left undecided pending confirmation that its branch carried nothing
  unique. Confirmed by measurement: `githubsucks/handoff-2026-07-20` is
  **1 ahead, 365 behind**, and its entire unique diff is four doc files
  at 42 insertions against 88 deletions — merging it would *revert*
  current documentation. The section said "whoever confirms the branch
  carries nothing unique removes the section"; this is that.
## Parked lane: kill-ring browser + persistence

- Portable branch: `githubsucks/kill-ring-browser`
- Parked framing head: `503c489`
- State: framing only, revision 2; no implementation and no PR.
- Status: explicitly parked by the user on 2026-07-20.
- Its original scout was based on `0efb5cd`. The preserved framing marks
  this ground truth stale and requires a complete re-scout against the
  then-current `githubsucks/main` before implementation.
- Compile-mode has merged since the original scout, so old
  “compile-mode in flight” keybinding/touch-set assumptions are not
  authoritative.

Recovery worktree, only when the user un-parks it:

```sh
git worktree add --track \
  -b kill-ring-browser \
  ../pmacs-kill-ring-browser \
  githubsucks/kill-ring-browser
```

## Closed since the last snapshot

- **Terminal configuration + copy mode arc — BOTH STAGES MERGED, lane
  removed.** Stage 1 **#173** (`main` @ `cf54270`, one review round) and
  Stage 2 **#178** (`main` @ `fe8b8ba`, **four review rounds**, twelve
  checks green on head `1b44c69` — verified by `head_sha`, not by the
  check summary), both 2026-07-26, both with no protocol change.
  Approved framing: `docs/terminal-config-and-copy-mode-framing.md` rev
  4, committed as the first commit of Stage 1's branch; its Q#TC6a
  carries a superseded-in-part box rather than a silent rewrite. Durable
  facts moved to `docs/agent-handoff.md` §1 (the arc bullet) and §4 (the
  `set_generated_contents` invariant) per rule 3 below, and to
  `COHERENCE.md` §14. **Stage 2 ships eight of nine criteria and the
  missing one is named** — criterion 17 needs a real GPU frontend, so it
  waits on the `a37` footing; the handoff records what it must assert.
  Branches `githubsucks/terminal-config` and
  `githubsucks/terminal-copy-mode` with worktrees
  `../pmacs-terminal-config` and `../pmacs-terminal-copy-mode` are
  retained. The gate-run flake found while gating #178 moved to the CI
  `crdt`-coverage lane above, which owns its discrimination.
- **Dired Stage 1 (the directory view) — MERGED as #165** (`main` @
  `c8ec8f3`, 2026-07-25, after one review round). pmacs has a directory
  surface: `C-x d` / `C-x C-j`, one read-only buffer per directory named
  `*dired:<canonical path>*`, a `dired` major mode carrying
  `RET`/`f`, `^`, `n`/`p`, `g`, `q`, `s`. No wire change (v20). The Rust is
  two things — a per-entry-tolerant `read_dir` (Q#DR6), which had to be
  Rust because `read_dir_blocking` fails a whole listing on any of five
  per-entry conditions and a tolerant wrapper cannot be written in Lua at
  all, and `normalize_buffer_path` going `pub` as
  `pmacs.path.canonicalize` (Q#DR2's preferred end state, so no Lua mirror
  exists and Stage 2 owes no mirror removal). The frozen m8_1/m8_2/m8_3
  counts are unchanged, which is the additivity gate. 15 claims
  bite-verified; one came back VACUOUS (acceptance 3c cannot pin descent
  routing — dired holds focus in its own panel, so dedication is the only
  discriminator) and is documented at the assertion rather than
  relabelled. Its branch (`dired-stage1`) and worktree
  (`../pmacs-dired-stage1`) are done; the abandoned `dired` branch
  (`ffdd642`, `../pmacs-dired-arc`) was superseded by a fresh cut and
  carries nothing unmerged. **Stage 2 (marks and operations) and Stage 3
  (wdired) each still need their own framing**, and the frozen fixture
  shrinks after Stage 3. Durable substrate facts and both new ops lessons
  live in `docs/agent-handoff.md` §§1/5; the implementation notes are
  `docs/dired-framing.md` §0, S1-1…S1-12. Two named forward items for
  Stage 2: `apply_resource_op`'s rename rebind is exact-PathBuf-equality,
  first-match-only, looked up with the raw path while stored paths are
  normalized — so a directory rename strands every buffer under it, and
  `pmacs.fs.rename` has zero production callers, so it can be fixed at
  the primitive; and Q#DR5's seam is the main-thread drain
  `AsyncRuntime::tick`, not `_take_result`, where rename settles as an
  undifferentiated `ReplyKind::FsUnit` and so must be keyed on
  `JobKind::FsRename`.

- **GPU terminal input (the double terminal-layout sync) — MERGED as #166**
  (`main` @ `b889873`, 2026-07-25, one review round, all twelve checks green
  after a macOS PTY-timing rerun). The dispatcher applied **both**
  terminal-layout syncs to **every** attached frontend; a semantic session
  satisfies both conditions, so its PTY was resized twice per tick forever and
  the child took a `SIGWINCH` storm that made a GPU terminal untypable while
  output still flowed. `sync_terminal_layout` is now split into a
  frontend-kind-neutral half (panel reconcile + controller liveness) and a
  grid-only geometry half, with the loop body extracted to
  `sync_terminal_layouts_for_tick` so the exclusivity is structural. No
  protocol change (v20). Durable lessons are in `docs/agent-handoff.md` §5;
  the framing (`docs/gpu-terminal-input-framing.md` rev 2) carries three
  falsified hypotheses, the two-pre-image bite matrix, and two named
  out-of-scope items (Q#GT5 interactive-shell echo on a raw PTY, which
  reproduces in-process and so is not the GUI/TUI asymmetry; and a geometry
  change appearing to clear the visible screen, which reproduces pre-fix).
  Branch `gpu-terminal-input` and worktree `../pmacs-gui-term-input` retained.
  **Its landed-doc pair MERGED as #168** (`main` @ `1b6a084`,
  2026-07-26): #166 recorded as landed, the CI `crdt`-coverage gap
  measured (**264 tests dark workspace-wide**, 177 in the library — a
  reading taken at `1b6a084` and kept here only as history. **The CI
  `crdt`-coverage lane above is the authority for the live figure**;
  do not quote this one forward), the
  vterm audit corrected — "only 3 of 9 acceptances drive a real daemon"
  was optimistic; without the frontend binary the honest number is
  **2** — and the a37 findings folded into the coverage lane.
- **Inline-math slice — MERGED as #158** (`main` @ `5aa9044`,
  2026-07-25). Detect → parse → layout → draw for `$…$`, entirely inside
  `pmacs-gpu`, no protocol change. Verified by the user's manual pass on
  a real paper after the landing. What is worth carrying forward:
  - **The v0 subset is 34 Greek symbols, sub/superscript, and `\frac`.**
    An unsupported command fails the **whole span** back to source, so on
    a real document most inline spans still show LaTeX. Widening the
    symbol map is the highest-value next increment — ahead of display
    math, which is also deferred.
  - **A stale frontend binary is invisible from the source tree.** The
    slice lives only in `pmacs-gpu`, so after it merged the feature was
    absent until `cargo build --release -p pmacs-gpu` and a client
    restart; the daemon needs neither. Diagnose with `strings` on the
    binary (`Latin Modern Math`, `MathBox`) rather than by re-reading the
    checkout, which was already current.
  - **Main was integrated three times in one day** (`8c86d34`,
    `46a1b8f`, `b889873`), merged not rebased to preserve review anchors.
    Two conflicts, both this ledger and nothing else. **The dangerous
    case was the one that did NOT conflict**: #166 auto-merged into
    `pmacs-gpu/src/main.rs`, the file this lane rewrites, because the two
    edits sat in different regions of it. Decide whether to integrate
    from the shared-**file** set, never from whether git complained.
  - **Integration was proved by test-count reconciliation**, not by a
    green run: predict what the other side adds, then check the deltas.
    GPU 199→202 matched `e547a90`'s 3; later lib 1,826→1,829 and CRDT
    2,003→2,006 matched #166's 3, with GPU unchanged because #166 adds
    none. Suites 91→92 was #161's new binary.
  - **Why the branch had no CI for a day**: a conflicting PR builds no
    merge ref, so no `pull_request` run is created. The ledger previously
    recorded this cause as unidentified; it is not. Check `mergeable` and
    confirm a run exists for the current head SHA.
  - **`m4_5_basedpyright` has no timeout and hangs forever**, parking a
    `--workspace` sweep (observed 2h26m at 38 of 92 suites). It is
    **intermittent**, so an earlier clean sweep proves nothing. Sweep with
    `cargo test --workspace --no-fail-fast -- --skip basedpyright` and
    judge progress by whether the suite count advances.
  - Named v0 approximations: the peer-caret half of acceptance 14 is
    pinned at the mapping level, not pixels; a soft-wrapped spacer draws
    its box whole at the first run's origin; the fit budget reads the
    bundled code face even under a custom `set_font` family.
- **Bottom panel Stage 1 — MERGED as #155** (`main` @ `e745068`,
  2026-07-24, after two review rounds). Window placement, window
  parameters, TUI side windows, the divider, and the adopter `display`
  opt-in, with no protocol change. Both rounds found the same class of
  defect and are worth keeping:
  - **Round 1**: the Q#BP6 side-window split guard had *no production
    caller* — `C-x 2` still reached plain `split_active` — and survived
    because the acceptance test called the core method directly.
  - **Round 2**: Q#BP7's terminal growth re-arm had *never been
    implemented*, and the assertion meant to pin it (`at_bottom`) is a
    geometric readout that a still-anchored view satisfies; the anchor
    assertions beside it compared `""` with `""` because the PTY fixture
    emitted LF-only output.
  - **Post-round-2 self-review**, caught by CI going red on all four Test
    jobs: resolving `pmacs.window.buffer()`'s no-argument arm through the
    acting frontend made a total function partial, and six runtime modules
    silently dropped operations (`kill_ring_acceptance` 30/30 → 25/5).
    Fixed in `9110f9f` before merge.
  - Gating fact found on the way: **the workspace sweep must run with an
    isolated `XDG_CONFIG_HOME`**, because the real user `init.lua`
    installs a local package and the losing race leaks a status message
    into painted-frame comparisons. There is also a latent pre-existing
    `main` bug in the buffer CRDT undo path, unrelated to this arc.
  - `compile_mode_acceptance` is load-sensitive under default
    parallelism (~1 run in 3, a different test each time); verified
    pre-existing by swapping in `main`'s `compile.lua`. It is 67/67 at
    `--test-threads=1`.

- **GPU initial target — MERGED as #148** (`main` @ `0dd16a5`, 2026-07-24,
  after two review rounds). `pmacs --gpu [--socket …] FILE` opens a target
  before the GPU window appears. Protocol bumped 19 → 20: a semantic-session
  `SessionBootstrapRequest` after `AttachRequest`, plus an appended
  `InstanceMessage::InitialTargetResult` pre-window readiness barrier; v6–v19
  wire encodings are unchanged. Root owns launcher tilde/cwd resolution and
  exact raw-byte path transport; the daemon resolves/dedups/loads the target
  and runs load/switch hooks inside one dispatcher transaction, then
  publishes CRDT-upgraded targets to existing grid replicas (gated on
  `upgraded_to_crdt`, independent of the load/create outcome, so a dedup onto
  a hidden not-yet-backed buffer still reaches pre-attached replicas — round
  2 finding). Semantic replicas receive a publication only when displaying
  that buffer, so a second target launch cannot switch an existing GPU
  window. Round 2 also closed a failure-containment gap: every dispatcher-side
  bootstrap failure now shuts down the socket (a dropped write-half clone
  does not close a shared FD), and the dispatcher drops any event from a
  session that was never installed, rather than reaching absent render/size
  state. Integrated cleanly with Folding Stage 2 (#149): fold projection at
  attach is selected from the same negotiated `semantic_render` bit the
  target bootstrap uses. Its lane, worktree (`../pmacs-gpu-initial-target`),
  and branch (`gpu-initial-target`) are done; the `-framing` branch is kept.
  Durable substrate facts and both review-round lessons live in
  `docs/agent-handoff.md` §§1/5 and `docs/gpu-initial-target-framing.md`
  rev 3.

- **Folding Stage 2 (grid/daemon collapse) — MERGED as #149** (`main` @
  `6ed4fe9`, 2026-07-24, after **five** review rounds). The grid TUI now
  renders collapses. Spine (Q#FD12): `src/fold_view.rs`'s `VisibleLineMap`,
  derived from the fold store plus a window's line offsets and **never
  stored**, threaded as `Option<&'a VisibleLineMap>` on a lifetime-bearing
  `Viewport<'a>` that stays `Copy`. No wire schema or protocol change; the
  GPU path is Stage 3. 48 acceptance tests on the real `paint_frame` grid,
  every behavioral claim bite-verified. Durable design points, each a trap
  Stage 3 inherits:
  - the map's unit is a **merged hidden component** (overlapping *or
    adjacent* intervals unioned, keeping the earliest visible head), not a
    fold — folds may cross, and a later fold's own head can be hidden;
  - instances are **per rendered window** and **per command/event
    operation**, never per frame; a command's map follows the operation's
    **target** window, since a wheel event names a pane without activating it;
  - fold projection is **per-frontend** (`FrontendView.fold_projection`) —
    shared `EditorCore` motion would otherwise make a simultaneous unfolded
    GPU session's cursor skip lines it still displays;
  - a hidden cursor normalizes by **position**, not row, and `set_view_top`
    clamps in the setter rather than being repaired at render time;
  - the interactive-Lua unfold keys on the **post-intercept** edit site — a
    managed buffer intercept may legally relocate the op.

  Process notes worth keeping: `main` moved under the arc, and the merge was
  textually clean but **not semantically clean** (#146 added `Viewport`
  literals the new `folds` field invalidated) — a clean `git merge-tree` does
  not mean the merged tree compiles. CI was red at review on the macOS/luajit
  `outline_5_level_100_entry_renders_within_100ms` budget flake and went
  green on rerun.

- **Documentation ledger refresh — MERGED as #147** (`main` @ `0a479ae`,
  2026-07-24). The #142 housekeeping, expanded after review found the ledger
  stale through four merges rather than one. Its own macOS/luajit red was the
  vterm `VTERM_ALT_READY` PTY timeout; green on rerun.

- **Web grammars HTML + CSS — MERGED as #146** (`main` @ `47581f4`,
  2026-07-23). `.html/.htm/.xhtml` and `.css` highlight off the official
  `tree-sitter-html` 0.23 / `tree-sitter-css` 0.25 crate query constants (no
  in-repo overlay), and HTML's `INJECTIONS_QUERY` lights up `<script>` → js
  and `<style>` → css. Durable lesson recorded in
  `docs/web-grammars-html-css-framing.md`: the `highlight.rs` capture table
  is **global**, so adding a capture name retro-paints every other language —
  check the reverse direction and pin it.

- **LaTeX Stage 1 — MERGED as #144**, with its parent inline-math framing
  committed as **#145** (`main` @ `f09b0a1`, 2026-07-23).
  `.tex/.latex/.sty/.cls` highlight via `codebook-tree-sitter-latex` 0.6 plus
  the first in-repo query overlay (`builtin/queries/latex/highlights.scm`,
  `include_str!`) — the reusable pattern for grammars whose crate ships no
  usable queries. The crates.io `tree-sitter-latex` is provably broken (no
  `scanner.c`). The math parser and Tiers 3–4 are deferred to the inline-math
  arc.

- **Folding Stage 1 (headless fold engine) — MERGED as #142** (`main` @
  `c49a8c7`, 2026-07-23, after three review rounds; round 3 clean). The
  instance-side fold store + translating/dropping `View`, the structural
  source (derived head line, closer-aware tail), the Lua data API +
  interactive `C-c @` commands, the command-path pre-edit unfold, and
  authoritative-empty `FoldState` production landed with no protocol bump.
  The `folding` branch and worktree (`../pmacs-folding`) are retained but
  carry nothing unmerged; the `folding-framing.md` framing is preserved.
  CI red at merge was an unrelated environmental perf flake
  (`outline_5_level_100_entry_renders_within_100ms`, macOS/luajit only),
  green on rerun. Stage 2 has since merged as **#149** (above); durable
  substrate seams live in `docs/agent-handoff.md` §1.

- **Vterm Stage 3 (protocol v19 + GPU terminal) — MERGED as #135** (`main`
  @ `cac4961`, 2026-07-22, after two review rounds). Arc 5's terminal stage
  is complete (compile mode #113, Stage 1 #126, Stage 2 #130, Stage 3 #135).
  Its lane, worktree (`../pmacs-vterm-gpu`), and branch are done; durable
  substrate facts live in `docs/agent-handoff.md` and `docs/vterm-framing.md`.

- **Branches deleted 2026-07-22 (authorized):** `vterm-stage3-framing`
  (Revision 8 framing; its content is carried on `vterm-gpu`, verified as a
  superset before deletion — the branch was NOT an ancestor of `vterm-gpu`
  because the framing was copied rather than merged, so it needed a forced
  local delete) and `tab-width-parity` (a clean ancestor of `main` via
  #137). Both removed as worktree + local ref + `githubsucks` ref; the
  `origin` tracking refs were pruned. The `-framing` branches for each are
  deliberately kept.

- **Tab-width rendering parity — MERGED as #137** (`main` @ `2625ec7`,
  2026-07-22). One fixed 8-column `TAB_STOP_COLUMNS` in `pmacs-protocol`
  now drives core/TUI columns, GPU code projection, and minimap width;
  source bytes and protocol ranges are unchanged. Its lane, worktree, and
  `tab-width-parity` branch (local + `githubsucks`) are deleted; the
  `tab-width-parity-framing` branch is kept. This closes the long-standing
  "tab width is a rendering-parity
  bug, NOT a config gap" deferral recorded in `docs/agent-handoff.md` §5.
- **Locals-query processing — MERGED as #134** (with handoff #136), and
  **modeline detection handoff #133**. Both landed between this lane's
  base and its canonical-main integration.

- **Config registry — MERGED as #127** (`main` @ `2e37c04`). Its lane
  (`config-registry`, worktree `../pmacs-config-registry`) is done; the
  branch is kept but carries nothing unmerged. Durable substrate facts
  moved to `docs/agent-handoff.md` §1 per rule 3 below.
- Both this and Vterm Stage 1 ran as **concurrent lanes in sibling
  worktrees off `main`**, with the shared files (`src/editor.rs`,
  `src/lua_bindings/mod.rs`, `src/lib.rs`) assigned to one lane each in
  advance. The rebase of the second lane onto the first had **zero
  conflicts** — worth repeating for future parallel work, along with its
  precondition: agree the file split before either lane starts, and keep
  each lane's footprint in the other's files to a single line.

## Update protocol

Whenever a listed lane changes materially:

1. update its public branch and head/state here;
2. record new verification and remove superseded caveats;
3. keep durable architecture in `docs/agent-handoff.md`, not here;
4. remove the lane after merge or abandonment;
5. verify every recovery command from a clean worktree before calling
   the transfer complete.
