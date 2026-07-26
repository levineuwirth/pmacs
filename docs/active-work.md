# Active work — cross-machine resume ledger

**Snapshot: 2026-07-26.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

**Two lane headers below are stale on purpose**, pending the docs updates
their own lanes owe: multi-root LSP affinity **#161 has merged** (the
Lean 4 lane still says IN REVIEW; its continuation is PR #167) and GPU
terminal input **#166 has merged** (its lane still says IN REVIEW; PR
#168 records it). Trust the canonical-base line below over a lane header:
if a PR number appears in `git log --first-parent githubsucks/main`, it
has landed regardless of what its lane says. (The inline-math lane was
here too until #172 removed it — that is the update those two owe.)

## Repository authority

- Canonical development URL:
  `https://github.com/levineuwirth/pmacs.git`. This ledger uses the
  normalized local alias `githubsucks` so its refs and recovery commands
  are identical on every machine. Remote names are otherwise
  machine-local: `origin` may name this canonical URL, a release mirror,
  or something else, and therefore has no authority by name alone.
- Canonical base at this snapshot:
  `githubsucks/main` @ `a27f646` (Lean 4 Stage 4a #179 atop bottom-panel
  Stage 2A #177, the bottom-panel Stage 2 framing #175, terminal
  configuration Stage 1 #173, Lean 4 Stage 3b #170, Stage 3a #167, the
  CRDT undo repro #157, the inline-math landed-doc refresh #172, the
  bottom-panel landed-doc refresh #156, the inline-math slice #158,
  dired Stage 1 #165, the GPU terminal input fix #166, Lean 4 Stage 2
  #161, the dired framing #164, COHERENCE.md #163, find-file #162, Lean 4
  Stage 1 #160, and the minimap blank-slab fix #159; protocol v20). The
  previous snapshot named `d152120`; the recovery check below accepts it
  or anything newer.
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

The `git log` command must expose `c93f9ee` — the base named above — or a
newer intentional main. Keep this threshold and the canonical-base line in
step: a recovery check that accepts an older commit than the base it
declares canonical will pass on a tree the rest of this file does not
describe.
If it does not, stop and repair the remote/fetch configuration.

## PTY terminate diagnostic lane — IN REVIEW (PR #176)

- Portable branch: `githubsucks/pty-terminate-eperm`; worktree
  `../pmacs-math-slice`. **PR #176**, base `main`, based on `ccf29e3`
  with `c93f9ee` (#175) merged in.
- Approved framing: `docs/process-signal-tolerance-framing.md`
  **revision 4**, after three review rounds.
- **Diagnostic only. No disposition change.** Every call that failed
  before still fails, with no state transition and no reap-ledger
  arming. `src/process.rs` is the only source file touched.
- **Why nothing is fixed:** revisions 1–3 each proposed a *tolerance*
  rule and all three were rejected as unsound in the same way — each
  concluded something about a process from something that was not about
  that process. Rev 1 from an errno alone (EPERM means the caller lacks
  permission, not that the id was recycled); rev 2 from `try_wait`,
  which observes the spawned **leader** while a PTY signal targets
  `-tcgetpgrp(...)`, entities that diverge exactly when job control has
  moved the terminal; rev 3 from group-directed **ESRCH**, which proves
  only that the selected foreground group vanished.
- **Two facts that killed the original argument.** `group = true` is
  *rejected* for PTY mode at spawn (`src/process.rs:1428-1429`), so the
  reap ledger never applies to the PTY path at all; and the ledger
  comment (`:1075`) says EPERM "cannot happen for our own children" and
  drops the entry for **bounded growth** — not a ruling that EPERM means
  dead.
- **The CI evidence never established the child had exited.** The probe's
  last source statement is a file write and CPython teardown does not
  synchronise with it, so no tolerance rule could even be shown to fix
  the symptom. That is the whole reason the lane is diagnostic.
- What ships: a failing `kill` now reports five separate facts — target
  source, target kind/value, spawn-time group, errno, and the leader's
  real `try_wait` state. The test seam injects the **kill result only**,
  never the observation, so the real `ChildHandle::try_wait` runs against
  the real child.
- **Not "strictly additive".** `try_wait` reaps and caches, so an exited
  child may be reaped earlier than otherwise. Safe because
  `portable-pty` 0.9.0 returns a `std::process::Child` on Unix and
  delegates `try_wait` to it, so `poll_one` still sees the cached
  status — pinned by an exactly-one-terminal-event test rather than
  assumed.
- Round-1 review fixes: the exited-child tests no longer use a fixed
  sleep as proof of exit (nix's `waitid` is unavailable on macOS and
  `libc::waitid` needs `unsafe`, which the crate forbids), instead
  driving the production diagnostic in a bounded loop until it observes
  the exit; and every assertion is now exact message equality built from
  the kernel-assigned pid, since the substring forms would have accepted
  a hardcoded target or a wrong exit code.
- Bites, all verified rather than assumed: tolerating the failure fails
  the disposition test; stubbing the leader observation fails three
  tests including the one-event pin; a hardcoded target fails four; a
  wrong exit code fails two.
- **The sweep found a real defect in these tests, not a flake.**
  `observing_the_leader_does_not_consume_the_exit_event` failed with
  "process ProcessId(26) is not running": the pid helper drained for
  `Started`, and **`drain_until` ticks**. A tick can observe an
  immediately-exiting child and move the record out of `Running`, after
  which `signal` never reaches the diagnostic at all, so the bounded
  loop spun to its limit. It passed standalone because the drain
  returned on `Started` before `poll_one` saw the exit; only load lost
  the race. Fast-exiting children now read the pid straight from the
  supervisor record (no tick), and the loop fails fast if the record
  left `Running`. **Verified under matched load: 0/15 with all 16 cores
  saturated, while the old ticking helper fails 1/10 — the fix is
  load-bearing.**
- Verification: fmt, `git diff --check`, strict workspace clippy clean;
  lib 1,838 + CRDT 2,015 (both +6, exactly the new tests); GPU 202; M4
  121; bottom-panel 46; compile-mode 67; vterm 9/6/5; **isolated-config
  `--no-fail-fast` sweep 3,258 across 93 suites, zero failures**.
  Earlier sweeps on this branch showed two failures and then one; the
  totals reconcile (3,256/2 → 3,257/1 → 3,258/0, same test count). The
  two that were genuinely unrelated —
  `read_dir_supersede_cancels_in_flight_predecessor` (known
  pre-existing) and
  `headless_snapshot_round_trip_summary_restores_the_minimap` — are
  load-contention flakes; the second is structurally unreachable from
  this diff, since `pmacs-gpu` depends on `pmacs-protocol` and never on
  `pmacs`.
- **Parked, each with its reason:** all tolerance rules (need the
  evidence this PR produces); `terminate` idempotence for an
  already-reaped process (independent fix, different failure, one
  feature per PR); and `signal_target`'s read-then-kill of `tcgetpgrp`
  — still the most likely real fix site.
- **The lane closes when this merges.** It does not wait for the flake
  to recur; the next occurrence carries its own evidence under whoever's
  PR, and a Stage B framing follows then.

## Lean 4 lane (Arc 8) — Stages 1, 2, 3a, 3b, 4a MERGED; 4b is next

- **Stages 1, 2, 3a and 3b are MERGED** — #160 (`main` @ `0827dd1`),
  #161 (`46a1b8f`), #167 (`6f348c9`), #170 (`d400f30`). Their full
  histories were pruned from this ledger in round 6, per this file's own
  instruction to remove entries when their PR merges; the durable facts
  now live in `docs/agent-handoff.md` §1's Lean 4 bullet, which is where
  a fresh machine should read them. `docs/lean4-mode-framing.md` rev 8
  carries the decisions.

### Stage 4 — framing rev 8, split into 4a/4b (branch `lean4-stage4a-typed-edit-chain`)

- Stages 3a and 3b **merged as #167** (`main` @ `6f348c9`) and **#170**
  (`main` @ `d400f30`), 2026-07-26. Both were integrated against a main
  that had advanced 50 commits mid-review; the only conflict either time
  was this ledger's own lane headings, resolved by keeping both sides.
- **Stage 4a merged as #179** (`main` @ `a27f646`, 2026-07-26) — the
  typed-edit consumer chain. Worktree `../pmacs-lean-stage4`, branched
  off `main` @ `d400f30`; retained, carrying nothing unmerged.
  `docs/lean4-mode-framing.md` **revision 8** remains the approved
  framing. **Stage 4b (the Lean-specific half) is framed and not
  started.**
- **Round 6 review found five P1s, four of them internal to rev 6** —
  facts about pmacs the revision asserted without checking, while its
  external (upstream) facts held. Fixed in rev 7: Stage 4a's footprint
  omitted the test file its own acceptance requires; pending
  abbreviation state was keyed by buffer when pmacs is **multi-frontend**
  (`EditorCore.views` is per-`FrontendId`, `take_typed_edit` is already
  frontend-keyed, and `buffer.after-switch` fires with NO arguments, so
  a buffer-keyed clear lets any frontend discard another's pending
  state); the shortest-match rule was missing its **tie-break by source
  declaration order**, which 101 prefixes depend on and a `pairs`-
  iterated Lua map cannot express; and the generator's "abort on keys
  needing escaping" rule **rejects the real table** (`\` is a key, `"`
  begins eleven).
- **A 404 on a guessed path is not evidence of absence.** Rev 6 declared
  the upstream package ships no README after fetching the package root,
  with the directory listing showing `src/README.md` already in hand.
  The README states the tie rule in one sentence.
- **Round 7 review found one remaining P1 in acceptance 45i.** Rev 7
  required A's pending abbreviation to survive B editing the same
  buffer, while Q#LN22 also required an exact buffer-revision advance.
  Those cannot both hold: revisions are buffer-global and every edit
  bumps them. Rev 8 keeps the conservative guard and separates
  ownership from survival — B cannot consume A's record, but B editing
  the shared buffer invalidates A lazily; B switching buffers or
  detaching remains frontend-scoped when no shared-buffer edit
  intervenes.
- **Round 5 re-scout split Stage 4 into 4a (substrate) and 4b (Lean).**
  4a is the typed-edit consumer chain — `builtin/runtime/typed_edit.lua`
  plus `pair.lua` re-expressed as one registered consumer, no behavior
  change. 4b is the input method. The split is forced by §4's own rule,
  which Stage 4's risk column ("refactors `pair.lua`'s provenance read")
  broke while the prose called the stage Lean-only.
- **This is the SECOND consecutive re-scout to find that rule broken**
  (round 4 found it for Stage 3). Rev 5 had even noticed the shape and
  answered it with a commit boundary. **A commit boundary is not a review
  boundary.** Re-check every remaining stage against §4 at scout time;
  the rule is not self-enforcing.
- **Rev 5's expansion semantics were wrong in three ways**, found by
  reading `leanprover/vscode-lean4` @ `17d1d08` rather than inferring
  from behavior. Resolution is *shortest key having the input as a
  prefix* (`\al` → `∀` from `all`, not `alpha`); there is **no
  terminator list** (`'+ '` is a key, so space extends after `\+`; `'\'`
  is a key, so `\\` → `\`); and an unmatchable tail is **appended**,
  not dropped (`\alp7` → `α7`).
- **There is no cursor-motion hook**, so rev 5's acceptance 43 ("moving
  the cursor out abandons it") was not buildable. Abandonment is lazy —
  validated at the next typed edit — and the criterion now asserts what
  pmacs can actually detect. Upstream drives this off `changeSelections`;
  that seam does not exist here.
- **`dispatch_key` is only half the production path for 4b.** The
  auto-pair suite gets away with dispatch-only because Q#AP1 removed the
  pair chars from the optimistic classifiers; `\` and the letters are
  NOT excluded, so on a CRDT frontend the optimistic producer is the real
  path. That producer is `#[cfg(feature = "crdt")]` and CI never enables
  `crdt`, and the gate list runs `--features crdt` only for `--lib` — a
  crdt-gated integration test is **dark twice over**.
- The whole expansion has cross-peer-degraded undo (Q#LN21): six
  source-peer optimistic inserts replaced by one daemon-peer op.
  `set_round_trip_input` would fix it and is rejected — it also disables
  `dispatch_idle`, so RET stops inserting a newline.
- Table facts re-derived at `17d1d08`: 1,855 entries, 36,861 bytes, all
  keys ASCII, **64** keys carry a `lean4` pair-set char, **305** keys are
  proper prefixes of another (so 1,550 expand eagerly), **26** values
  carry `$CURSOR`, and **119** are multi-codepoint — the 26
  `$CURSOR`-bearing values plus 93 others.
- Citation sweep per COHERENCE §25: five live citations moved in the 50
  commits since rev 5 — `take_typed_edit` 12827→12990,
  `handle_server_requests` 1549→1815, `fs.stat` 93→133,
  `detect_buffer_language` 452→457, `send_request`/`send_notification`
  9342/9361→9507/9527.
### Stage 4a — the typed-edit consumer chain (IMPLEMENTED, same branch)

- Footprint exactly as Q#LN10 declares it: `builtin/runtime/typed_edit.lua`
  (new), `pair.lua` re-expressed as one consumer,
  `src/editor.rs` +15 (the `include_str!` and its ordering comment), and
  `tests/typed_edit_chain_acceptance.rs` (new, 13 tests).
  **`tests/auto_pair_acceptance.rs` is UNCHANGED — `git diff --stat
  main...HEAD -- tests/auto_pair_acceptance.rs` is empty.** That is
  criterion 46 checked at the diff, which is the only way it means
  anything.
- **The chain calls consumers even when the record is nil.** This is a
  decision, not an implementation detail: three existing auto-pairing
  tests assert `pmacs.pair._last_record == nil` after a record-less
  fan-out (paste, programmatic insert, nested manual `hook.run`), so
  skipping consumers on nil fails them. Stage 4b needs the same
  delivery to abandon a pending abbreviation an unrelated edit
  invalidated.
- **Ordered insertion, not `table.sort`** — Lua's sort is not stable, and
  "ties broken by registration order" is a stated contract.
- **The chain `pcall`s each consumer** and reports through
  `set_status`. Rev 7 justified this by claiming an uncontained throw
  would fail the fan-out for every other subscriber including lsp.lua's
  didChange flush; **that is wrong** — `run_all_must_succeed`
  (`src/hook.rs:332`) collects errors and continues, so the other
  subscribers still run. The real consequence is narrower and still
  worth containing: the throw skips every LATER consumer in the chain.
  The rendering is protected too, because a Lua error may be a table
  whose `__tostring` throws.
- **Round 8 (review) findings, all fixed on this branch:** each consumer
  now gets its **own shallow copy** of the record (the same table let a
  declining consumer rewrite `rec.char`, which pairing reads — typing
  `x` could produce `x)`); the fan-out iterates a **snapshot** (a
  consumer registering a lower-priority one shifted itself forward under
  `ipairs` and ran twice, unbounded if repeated); `tostring` moved
  inside the containment; **non-finite and non-integer priorities are
  rejected** (NaN is a number and every ordered comparison with it is
  false, so it landed wherever the insertion scan gave up and silently
  voided the ordering contract); and `add_consumer` now returns a handle
  with `remove_consumer` beside it, so re-evaluating a config no longer
  leaks callbacks the way `pmacs.hook.add` does (COHERENCE §13).
- **Every acceptance test is bite-verified by mutation**, per the
  standing rule that a test is not evidence until the mutation it
  targets has been shown to fail it:

  | Mutation | Tests it fails |
  |---|---|
  | append instead of ordered insert | 5 chain |
  | `>=` instead of `>` in the insert scan | 1 chain (tiebreak) |
  | re-take the record per consumer | 4 chain |
  | ignore the claim return value | 1 chain |
  | drop the `pcall` | 1 chain |
  | skip consumers when `rec == nil` | 1 chain + **3 auto-pair** |
  | load `typed_edit.lua` after `lsp.lua` | 1 chain + **2 auto-pair** (Q#AP7) |
  | hand every consumer the same record table | 1 chain (46f) |
  | iterate the live array instead of a snapshot | 1 chain (46g) |
  | render the error outside the `pcall` | 1 chain (46d) |
  | accept any Lua number as a priority | 1 chain (46h) |
  | make `remove_consumer` a no-op | 2 chain (46g, 46h) |

  The first attempt at the last bite was WORTHLESS as written: moving
  only `typed_edit.lua` past `lsp.lua` left `pair.lua` calling a nil
  `add_consumer`, so the runtime failed to load and all 9 tests died —
  loud, but not a test of the flush-ordering property. Moving
  `typed_edit.lua` AND `pair.lua` past `lsp.lua` is the faithful
  falsification: registration succeeds, the hook lands late, and exactly
  the three ordering tests fail. **A bite that kills everything has not
  isolated anything.**
- Verification on this branch (commit-then-gate, so this describes the
  pushed tree): `cargo fmt --check` clean; strict workspace Clippy
  clean; 1,832 default + 2,009 CRDT library tests; auto-pair 45/45;
  typed-edit chain 13/13 (and 13/13 again under `--no-default-features
  --features lua54`, since the fixes touch `math.huge`, `%`, and
  `__tostring` behavior that differs between the backends); M4 121;
  required GPU 202; **isolated-config workspace sweep 3,332 across 97
  suites, zero failures** with `grep -c basedpyright` = 0; `git diff
  --check` clean.
- Stage 4b (the input method) is NOT in this PR and not started.

## Journey Stage 1a — IMPLEMENTED on branch, gates run, PR pending

- Framing `docs/journey-stage1a-framing.md` **rev 7** (four review
  rounds, then two correction revisions found during implementation).
  Branch `journey-stage1a-directory-open`, rebased onto `githubsucks/main`
  @ `74301d1`.
- Recovery: `git fetch githubsucks && git checkout
  journey-stage1a-directory-open`. Everything below is committed and
  pushed; nothing depends on a worktree or `/tmp`.
- **Ships:** the directory arm on `resolve_target_buffer`,
  `EditorState::open` rewritten as a caller of it (the unification), the
  `path.open-directory` chain + `pmacs.path.directory_handler` fallback
  slot, `pmacs.window.commit_to` with its scoped frontend and preflight,
  the nonconstructible destination userdata, the daemon bootstrap arm,
  and `tests/journey_acceptance.rs` (23 pins). No protocol change —
  still v20.
- **Doc updates ride the PR** per COHERENCE §25: §2 grade + step-3
  verdict row, §20 Priority 1 + the arc list, the GPU initial-target
  framing's Q#GT6 / acceptance 10 supersession, handoff §1.
- **Bite results** (each mutation run against the full suite): scope
  stops swapping `core.active_frontend` → N6a + P3 fail, nothing else;
  preflight moved after the callback → P1 + P2 fail, nothing else; drop
  the `ScopedFrontend` arm from `acting_frontend` → N4b fails, nothing
  else. That last mutation is why N4b exists — it left N4 green.
- Ordering: PR #177 MERGED (2026-07-26), so 1a was unblocked. 1a lands
  before dired Stage 2. When 1a lands, Stage 2 must re-scout and revise
  its framing around the scoped `pmacs.window.commit_to` boundary before
  its implementation branch is cut. That revision is a prerequisite, not
  a review-time discovery.
- **Named deferrals carried out of this stage:** dired's *interactive*
  paths (`C-x d`, tree descent, refresh) still rely on the ambient
  frontend a tick later and are not migrated onto captured destinations;
  the stale startup scratch buffer is still not removed (only the false
  doc comment is corrected); `resolve_target_buffer`'s directory arm has
  no picker, only the chain that leaves room for one.

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
  `crdt`: 3,024 vs 3,288 — 264 tests dark.** Per target:

  | dark | CI | full | target |
  |---:|---:|---:|---|
  | 177 | 1,832 | 2,009 | **the library itself** (`src/lib.rs`) |
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
  | 1 | 9 | 10 | `vterm_stage1_acceptance` |
  | 1 | 7 | 8 | `statusline_segments_acceptance` |
  | 1 | 10 | 11 | `gpu_font_acceptance` |
  | 1 | 0 | 1 | `auto_indent_crdt_acceptance` |
  | 1 | 0 | 1 | `m10_11_perf` |

- **The single worst line is the library.** `cargo test --lib --features crdt`
  is a REQUIRED local gate in `CLAUDE.md`, and CI has never run it. 177
  library tests — the whole CRDT half — are developer-machine-only.
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

## Terminal config + copy mode arc — Stage 1 MERGED; Stage 2 IN REVIEW

- Approved framing: `docs/terminal-config-and-copy-mode-framing.md`
  **revision 4** (four review rounds), committed as the first commit of
  Stage 1's branch. Two stages, two branches, two PRs; **no protocol
  change**.
- **Stage 1 MERGED as #173** (`main` @ `cf54270`, 2026-07-26, one review
  round, all twelve checks green). Branch `githubsucks/terminal-config`
  and worktree `../pmacs-terminal-config` retained. Profiles, scrollback,
  a per-terminal configurable escape key, and the `C-c t` opening
  binding; no protocol change. Main was integrated **twice** during the
  single review round (`ccf29e3`, then `c93f9ee` after the first merge
  left the PR conflicting) — see the no-CI-while-conflicting fact below.
- **Stage 2 = `githubsucks/terminal-copy-mode`**, worktree
  `../pmacs-terminal-copy-mode`, based on `githubsucks/main` @
  `cf54270`. Copy mode: `M-x terminal.copy-mode` / `C-c C-t`.
- **Stage 2 ships eight of nine criteria, and the missing one is named.**
  Criterion 17 (a real semantic frontend proving neither daemon buffer
  nor mirror mutates) is **not pinned**: the optimistic apply exists only
  in `pmacs-gpu/src/main.rs`, and the headless `SemanticClient` every
  other semantic test uses has no optimistic path, so a faithful test
  must drive the real GPU binary — the `a37` foundation, which CI never
  compiles, silently skips without the binary, and is load-sensitive. A
  second test on that footing buys the appearance of coverage. Both
  halves of the mechanism are pinned **ungated** instead: acceptance 16
  (the guard is armed — `dispatch_idle` false while the snapshot is
  focused) and 16b (the daemon holds — `is_read_only()` is **true** at
  the rope, so an op that did arrive is refused by `ensure_writable()`).
  **Rounds 2-3 changed what 17 must show.** 16b asserted `false` through
  round 1, documenting the hazard; round 2 closed it. So the eventual
  real-GPU test must look for **mirror mutation plus daemon refusal —
  divergence** — not the "mutates both sides, silently" the criterion
  originally specified, which after the fix cannot happen and would pass
  for the wrong reason. The wire-level half stays an explicit obligation
  of the CI `crdt`-coverage lane.
- Load-bearing Stage 2 decisions:
  - **The snapshot MATERIALIZES into an ordinary buffer**, so isearch,
    motion, selection and the kill ring work with no new substrate, and
    "keys must not reach the child" dissolves structurally — the
    transport arm keys on `is_terminal(buffer_id)` and a snapshot is not
    a terminal. **The dispatch-shadow count stays at six.**
  - **One serializer, not two** (Q#TC7): `copy_retained` builds a
    whole-range *selection* and hands it to `copy_selection_bytes`.
  - **`prune` reacts to removal rather than causing it** — it filters on
    `!registry.contains(buffer_id)`, so a child exiting does NOT remove
    the terminal buffer. That is why `on_removed` is a sound teardown
    hook, and why a finished command's output stays readable.
- **Five bites, five different wrong implementations.** Removing
  `set_round_trip_input` fails acceptance 16 **in the default
  configuration** (the whole reason that pin is ungated); a naive
  independently-written serializer fails all four unit pins, with the
  diffs naming each drift mode (broken soft wrap, untrimmed blanks,
  trailing newline); making re-invoke create a fresh buffer fails 18;
  dropping the kill-with-terminal teardown fails 18; removing the
  intercept fails 16b. Each failed exactly one test.
- **Review round 1 — four findings, all real, and they rhyme in pairs.**
  Two P1 implementation defects and two P2 vacuous pins, all four tracing
  to one root: **a name is not an identity, and a context-free readout is
  not a state observation.**
  - *P1 — a foreign same-named buffer was adopted and clobbered.* Snapshot
    writes use `bypass_intercept`, so found-by-name adoption overwrote a
    user's buffer; the reviewer reproduced "do not clobber" becoming 23
    newlines. Fixed by dired's F7 rule: **ownership means "in our own
    handle table"**, and a taken name yields a `<2>` variant.
  - *P1 — snapshot identity was keyed by terminal NAME.*
    `TerminalManager::open` uniquifies only the *derived* name, so an
    explicit `name = "*same*"` lets two valid terminals share one; they
    then shared a snapshot, `q` returned to the wrong terminal, and
    killing either removed it. Now keyed by comparing buffer handles in an
    array — `BufferIdLua` implements `__eq` but each wrapper is a distinct
    table key, so **comparison works and hashing does not**.
  - *P2 — the refresh pins were vacuous.* 19 compared a quiet terminal's
    snapshot against itself and 18 counted buffers, so both passed with
    `render_snapshot` replaced by a no-op. Now the test types a marker
    into the `cat` child, requires it **absent** first, then refreshes.
  - *P2 — the tail-follow pin could not observe view state.*
    `manager.snapshot(buffer_id)` is context-free and always reads the
    live screen, so it reported "at the tail" for a view forced to the
    oldest retained row. Now read through `snapshot_for_view`'s
    `at_bottom` and projected cells.
- **Four more bites, all discriminating.** Restoring adopt-by-name fails
  18a *and* 18b; restoring name-keyed identity fails 18b; making
  `render_snapshot` a no-op fails **both** 18 and 19 (the vacuity,
  demonstrated); and forcing the view off the tail fails 20.
- **Review round 2 — one P1, and its fix retires half a named deferral.**
  **Undo emptied the "read-only" snapshot.** `render_snapshot` wrote with
  `bypass_intercept`, leaving ordinary undo history, and **`Buffer::undo`
  reaches the rope through `ensure_writable` without ever consulting the
  intercept chain** — so `C-/` *or* `M-x buffer.undo` replaced a freshly
  rendered snapshot with an empty buffer. `set_round_trip_input` does not
  help: it routes the key into the daemon command path, which is where
  undo runs.
  - **Rebinding the undo chords would NOT have fixed it**, and
    `compile.lua` already says so in a comment — "command/menu undo stays
    dispatchable". `*compilation*` and listview panels therefore carry the
    same latent defect today.
  - Fixed with `Buffer::set_generated_contents` (Lua
    `pmacs.buffer.set_generated_contents`): lift `read_only`, replace
    skipping intercepts, **discard history**, re-assert `read_only`. This
    ships the deferred lane's two halves *as one primitive* — a bare
    `set_read_only` would let a caller lock a buffer it can no longer
    refresh, which is exactly why that lane was deferred. Clearing history
    also stops a periodically refreshed buffer accumulating rope clones
    nothing can ever pop.
  - New pins: **acc16c** drives the real M-x path
    (`command.invoke_interactive`), the chord, and redo, and asserts the
    owner's refresh still works; **acc16b** flipped from asserting
    `is_read_only()` is *false* to *true*, because the property it
    described is the one that was fixed; plus three `buffer.rs` unit tests.
  - Bite: restoring the `delete`+`insert` render reproduces the report
    exactly — `left: Some("")` against the full snapshot — failing acc16c
    and acc16b.
  - **Still open:** `*compilation*` and listview remain emptiable by
    `M-x buffer.undo`; the primitive they need now exists and is proven,
    so the remainder is adoption plus a streaming-friendly variant.
- **Review round 3 — one P1 and two P2s, all on the round-2 primitive.**
  The lesson: **a rope write is only half of an edit, and "discard
  history" means whichever history the buffer actually has.**
  - **P1 — the binding swallowed the edit.** `set_generated_contents`
    returned `()`, so nothing called `notify_buffer_edit_to_windows`.
    Two consequences, both reproduced by the reviewer: in the default
    build a window showing the buffer kept a `TextView` line index
    describing the *previous* contents, and the next paint indexed the
    new rope with stale ranges — `assertion failed: end <= self.len()`
    in `src/rope.rs`; in the CRDT build `pending_crdt_ops` stayed empty,
    so replica mirrors never received the owner's write. The prior
    `buf:delete`/`buf:insert` pair had done this fan-out for free.
    Fixed by applying **one whole-buffer `Replace`**, returning its
    `Edit`, and notifying from the binding.
  - **P2 — "discard history" was false in CRDT mode.** The v0.1 stacks
    are bypassed entirely there; the history lives in loro's
    `UndoManager`. `read_only` stops the replay but not the retention,
    which is the memory cost the contract claims to eliminate.
    `UndoManager` has no `clear`, but needs none — it records only what
    happens after construction, the property `CrdtState::from_bytes`
    already uses to keep the seed insert out of undo. New
    `CrdtState::clear_undo_history` rebinds a fresh manager to the
    same doc.
  - **P2 — the docs described the pre-fix architecture.** Q#TC6a said no
    Lua binding sets `read_only` and round-trip input is the only guard;
    the acceptance text still said `is_read_only() == false` while 16b
    had been flipped to `true`; `terminal.lua`'s comment repeated the
    obsolete claim. The architecture is **layered** and now says so:
    rope-level read-only protects the daemon copy, round-trip input
    protects the replica's optimistic mirror, and neither substitutes
    for the other. Q#TC6a carries a superseded-in-part box rather than
    being silently rewritten.
  - New pins: **acc16d** paints the window after a *shrinking* generated
    write (the stale offsets then point past the end, which is the
    reported crash rather than stale pixels); **acc16e** asserts the
    refresh is queued for mirrors through the real copy-mode path
    (`crdt`-gated, therefore dark in CI — 16d is the half that runs);
    plus a CRDT `buffer.rs` unit test that ten renders leave the
    `UndoManager` with nothing recorded.
  - Bites: dropping the notify panics acc16d at `rope.rs:145` and fails
    acc16e with `queued: []`; dropping the `UndoManager` rebind fails
    the new unit test on `can_undo`.
  - **Still open:** the fan-out obligation makes `*compilation*`/listview
    adoption more than a one-line swap — recorded in `COHERENCE.md` §14
    alongside the undo half.
- **Review round 4 — one P2, docs only, and it is the interesting kind.**
  **A fix can invalidate a test that was never written.** Criterion 17's
  *bite* still described the pre-round-2 world: remove
  `set_round_trip_input` and the op "mutates both sides, silently, with
  no divergence to notice". True while nothing set `read_only` from Lua;
  false once `set_generated_contents` did. A real-GPU test written to
  that spec would hunt for a daemon-side edit that can no longer occur
  and pass for the wrong reason — the specification would have leaked
  the round-2 regression back in, through a test not yet built.
  - Restated around **unauthorized mirror mutation plus daemon refusal =
    divergence**, in all four places that carried the old claim: the
    criterion, the Q#TC6a heading, the acceptance-16 doc comment, and the
    bite roster. The heading's "ONLY thing" now says what it is the only
    thing *for* — the replica's own mirror.
  - Why round-trip input is still load-bearing rather than redundant: a
    daemon refusal arrives after the frontend has already applied
    optimistically and painted. It buys divergence instead of silent
    agreement; it does not prevent the mutation the user sees.
  - **Gate-run flake observed and scoped without overclaiming its cause.**
    `cargo test --lib --features crdt` failed ~1 run in 5 on
    `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
    — `active_reader_probe` returning `None` at `process.rs:3179`
    ("live runtime probe"). **Pre-existing and unrelated:** this branch
    does not touch `src/process.rs` (last changed by the Darwin PTY
    signal-name fix), and the test passed 10/10 standalone; the observed
    failures were during parallel full-suite runs. That localizes the
    trigger to suite load or interaction, but does **not** distinguish
    parallelism from another full-suite effect — no serial full-suite bite
    was run. The leading code-path explanation is the known `drain_until`
    trap: draining for `Started` also ticks, and a tick can reap the leader
    before the following `active_reader_probe`. That is an inference from
    the failure site and control flow, not yet a falsified root cause.
    It belongs to the CI `crdt`-coverage lane for discrimination. The two
    round-2 CRDT failures had no captured test names; this flake is a
    plausible candidate for them, but they remain **unattributed**.
- Load-bearing decisions, each forced by scouted ground truth:
  - profiles are a **raw Lua table** — `ConfigValue` is four scalars with
    no table kind, so they join `pmacs.lsp.config` / `pmacs.pair.sets`;
  - the **two open-time settings resolve through the global chain**,
    because they are read before the identity buffer exists; only
    `terminal.escape-key` resolves per buffer;
  - the escape cache lives on **`TerminalSession`** so its lifetime is
    the terminal's. `value_epoch` alone is not a sufficient key: it does
    not advance when focus moves between terminals with different
    buffer-local values;
  - repeating the escape sends **that chord**, not a hardcoded `0x03`.
- **Four bites, each against a different plausible wrong
  implementation** — hardcoded ETX fails acc6/9; epoch-only cache key
  fails acc7; single last-entry cache fails acc8's parse count; removing
  the invalid-value fallback fails acc10. The first version of acc7
  passed against the epoch-only bite because it asserted only that
  terminal A still worked; the discriminating assertion is that **each**
  terminal honors its own chord and not the other's.
- Test instruments worth reusing: `cat -v` is the echo probe, because the
  screen rejects C0 controls before they reach cells so a raw echoed
  `Ctrl-X` is invisible; and the probe **counts occurrences** rather than
  testing presence, because a single-character probe collides with the
  child's own banner text.
- **Review round 1 (2026-07-25) — five findings, all real, all fixed.**
  One blocker and two majors were the same failure in three places: a
  claim asserted somewhere cheaper than where it lives.
  - *Blocker — `COHERENCE.md` was stale in four places, not the three
    reported.* Step 8 still read "no keybinding"; §11 still read "five
    settings"; and §6's dispatch table still cited
    `is_terminal_escape_chord`, **a symbol this PR deletes**. §25 makes
    that update ride the PR. A PR that changes audited ground truth has
    to re-grep the audit for its own symbols, not only for its topic.
  - *Major — acceptance 5 was vacuous.* It asserted a registry
    round-trip, so it stayed green with the setting's **only** consumer
    deleted. It now opens a real terminal whose child overflows the
    24-row screen, scrolls the view to its oldest retained row, and
    asserts `LINE001` is present at 10,000 and absent at 0. **Asserting
    a value was stored is not asserting anything reads it.**
  - *Major — acceptance 8a asserted the session count, not the cache.*
    An editor-side map with no purge hook — the exact rejected design —
    leaks *while* sessions drain, so it passed. Fixed with a
    `TerminalManager::escape_caches()` seam. **A lifecycle claim needs a
    lifecycle observable.**
  - *Moderate — `table.sort` over user-controlled profile keys.* A
    table holding both a string and a numeric key raised `attempt to
    compare number with string` **on the unknown-profile path**,
    replacing the diagnostic being asked for; `%q` raised likewise on a
    non-string `profile` argument. Both are partial functions applied to
    user input **on a diagnostic path** — the error reporter was the
    thing that failed.
  - *Minor — the committed framing still said "not yet approved".*
- **Three new bites, each falsified by revert**: deleting the scrollback
  consumer fails acc5 (and only acc5); restoring the raw-key sort
  reproduces `attempt to compare string with number` verbatim; and
  implementing the rejected editor-side map fails the new acc8a at
  `left: 2, right: 1` **while passing the old session-count version** —
  which is the review finding demonstrated rather than argued.
- Verification after the round-1 fixes, on the tree merged with
  `githubsucks/main` @ `c93f9ee`: `cargo fmt --check` clean; strict
  workspace Clippy clean; 1,832 default + 2,009 CRDT library tests;
  `terminal_config_acceptance` **12/12 in both configurations**; vterm
  Stage 1/2 9+10 / 6+6; config registry 16+16; bottom-panel Stage 1
  46+46; M4 121; required GPU 202; `git diff --check` clean.
  - `compile_mode_acceptance` fails 11/67 against the **real** user
    config and passes 67/67 with an isolated `XDG_CONFIG_HOME` — the
    known pre-existing trap, not this branch.
  - **`vterm_stage3_acceptance::a37` fails on this machine — and fails
    identically on the PR's own base `d152120`**, so it is not this
    branch's regression. It is load-sensitive: it passed at `d152120`
    once and failed at that same commit twenty minutes later, with a
    second agent saturating the machine with `rustc` in between. Two
    ways it lies, both worth knowing: it **silently returns `ok` when
    `pmacs-gpu` is not built** in the same target dir (only
    `PMACS_REQUIRE_GPU=1` promotes that skip to a failure, and the gate
    list applies that flag to `-p pmacs-gpu`, a *different* package), and
    it is **crdt-gated, so CI has never run it at all**. A green a37 in
    a gate log means nothing unless the binary was built and the flag
    was set. Needs its own lane; see the CI `crdt`-coverage lane on #168.
  - `pmacs-gpu` itself failed 201/202 once under the same load and passed
    202/202 on immediate rerun.

## Bottom-panel lane (Arc 7) — Stages 1, 2A + framing MERGED; 2B is next

Stage 1, the Stage 2 framing, and Stage 2A are all on `main`. **Stage 2B
has not started.**

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
- **Stage 2B is approved and unstarted.** It branches from `main`, **not
  stacked on 2A**, per the framing §9. Scope: protocol v21, the daemon
  panel projection, the GPU band, and the negotiated `panel_capable`
  flip. `docs/bottom-panel-stage2-framing.md` §7.2 carries its five
  acceptance criteria (A2B-1..5) plus the reassertion of parent
  criterion 52, and §8 records **no open items**, so 2B needs no further
  framing round. Its sharpest trap is §5.3's three-boundary split: the
  GPU `text_area_bottom` is `status_band_top`,
  `geometry_capacity_bottom` and `document_text_bottom` at once, and a
  blanket rewrite moves the status chrome along with the document while
  still satisfying an "everything moved" assertion — hence A2B-4's
  contrast form.
- Verification on the merge result: `cargo fmt --check` clean; strict
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
- **Stage 2 framing: `docs/bottom-panel-stage2-framing.md` revision 4**,
  on branch `githubsucks/bottom-panel-stage2-framing` (three commits,
  one per revision), worktree `../pmacs-bp-stage2`, based on
  `githubsucks/main` @ `ccf29e3`. Round 1 closed 2 blocking + 3 high;
  round 2 closed 1 blocking + 2 high + 1 medium and decided both open
  items; round 3 closed 1 blocking + 1 high + 1 medium. No open items
  remain. The approved
  parent framing `docs/bottom-panel-framing.md` (rev 4) remains
  authoritative, **including its acceptance criteria 37–55**.
- Retained, carrying nothing unmerged: branch `bottom-panel` and worktree
  `../pmacs-bottom-panel`.
- **Stage 2 ships as two serial slices**, 2A landing before 2B branches:
  **2A** = classified §1.3 census routing + `paint_frame` per-window
  painter extraction (with the active-window auto-scroll preparation), no
  protocol change; **2B** = protocol **v21**
  (`InstanceMessage::PanelFrame` plus
  `FrontendEvent::{FrontendCellGeometry, PanelResizeRows, PanelPointer}`,
  gated both directions, each extended enum byte-pinned on its own
  previous final variant), daemon panel projection, the GPU band, and the
  negotiated `panel_capable` flip. Stage 3 is the adopter default flip.
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

## Documentation lane

- Portable branch: `githubsucks/handoff-2026-07-20`
- Carries synchronized `AGENTS.md` / `CLAUDE.md`, this ledger, the
  durable handoff refresh, and the keybinding reference correction.
- It changes no runtime code.
- Review and merge this documentation branch separately; it must not be
  folded into a feature framing branch.
- Now also absorbs both landed arcs: Vterm Stage 1 (#126) and the config
  registry (#127). Canonical `main` is merged into it up to `2e37c04`,
  so its diff against `main` is documentation only.

## Closed since the last snapshot

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
