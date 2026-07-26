# Active work — cross-machine resume ledger

**Snapshot: 2026-07-25.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

## Repository authority

- Canonical development URL:
  `https://github.com/levineuwirth/pmacs.git`. This ledger uses the
  normalized local alias `githubsucks` so its refs and recovery commands
  are identical on every machine. Remote names are otherwise
  machine-local: `origin` may name this canonical URL, a release mirror,
  or something else, and therefore has no authority by name alone.
- Canonical base at this snapshot:
  `githubsucks/main` @ `d400f30` (Lean 4 Stage 3b #170 atop Stage 3a
  #167, the bottom-panel landed-doc refresh #156, the inline-math slice
  #158, dired Stage 1 #165, the GPU terminal input fix #166, Lean 4
  Stage 2 #161, the dired framing #164, COHERENCE.md #163, find-file
  #162, Lean 4 Stage 1 #160, and the minimap blank-slab fix #159;
  protocol v20). The previous snapshot named `d152120`; the recovery
  check below accepts it or anything newer.
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

The `git log` command must expose `d152120` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Lean 4 lane (Arc 8) — Stages 1, 2, 3a, 3b MERGED; Stage 4 IN FRAMING

- Stage 1 **merged as #160** (`main` @ `0827dd1`, 2026-07-25, one review
  round, all twelve checks green). Branch `githubsucks/lean4-stage1`
  retained; it was worked in the shared checkout (no sibling worktree).
- Approved framing: `docs/lean4-mode-framing.md` revision 4, committed as
  the branch's first commit (`a382965`) after three review rounds. **Seven
  stages**, 19 decisions (Q#LN1–19), 64 acceptance criteria. North star:
  match or exceed VS Code's Lean support.
- **Stage 1 implemented; no wire change (protocol stays v20), no LSP, no
  frontend change.** Four commits: framing, grammar, theme captures,
  editing surface + acceptance.
  - `Cargo.toml` + `src/syntax.rs`: `arborium-lean` 2.18 and one
    `BUILTIN_LANGUAGES` entry named **`lean4`** (Q#LN2 — the name becomes
    the `didOpen` language_id), claiming `.lean` only.
  - `src/highlight.rs`: four capture entries — `constructor`, `character`,
    `keyword.conditional`, `warning`.
  - `builtin/runtime/{comment,pair,syntax}.lua`: `--` comments, the
    `⟨⟩ ⦃⦄ ⟮⟯` pair set, the `lean` → `lean4` modeline alias.
  - `tests/lean4_stage1_acceptance.rs` plus unit tests in `syntax.rs` /
    `highlight.rs`: 12 criteria, 17 tests.
- **Q#LN1's open obligation is discharged.** `tree-sitter-lean4` is
  unusable (depends on `tree-sitter ^0.25` directly against our 0.26,
  exports no `LANGUAGE` const despite its README, packages no queries);
  `arborium-lean` rides `tree-sitter-language 0.1` with a pre-generated
  ABI-15 parser. `cargo tree -d` shows no duplicate core. The parse smoke
  pins the failure mode that matters: `→`/`∀`/`≥` must produce
  `(arrow)`/`(forall)`/`(comparison)`, since a mismatched-core build
  degrades silently on exactly those characters rather than failing loudly.
- **Q#LN4 is a deliberate retro-paint of seven language entries**, not
  four: `tree_sitter_javascript::HIGHLIGHT_QUERY` is concatenated
  base-first into javascriptreact/typescript/typescriptreact. Its shape is
  "every capitalized identifier" (`#match? "^[A-Z]"`) plus every Lua table
  brace — not "constructors". Pinned in both directions per #146.
- Implementation findings not in the framing:
  - `warning` had to move from bold red to bold **bright** red: `number`
    is plain `fg(1)`, so `sorry` and an adjacent numeric literal were the
    same colour. Found by writing the test.
  - `Some(1)` is **not** `@constructor` — in call position a narrower
    `@function` pattern wins. Only bare or pattern-position capitalized
    identifiers reach it. Pinned so the blast-radius claim stays honest.
  - Lean node kinds nest: `module > declaration > def|theorem`.
  - `pmacs.parse.injection_aliases` is a documented **write-only** Lua
    proxy (canonical map is Rust-side), so fence tests must drive
    `_parse_now` and inspect layer languages, never read the table back.
- **Review round 1 addressed.** The finding: acc12's server-list assertion
  could not fail for the regression it named — the shared `editor()`
  helper wipes `pmacs.lsp.config` before any buffer opens, so
  `#pmacs.lsp.list() == 0` holds for every language regardless of what
  Stage 1 ships. It now asserts against a **pristine** `EditorState` that
  `pmacs.lsp.config.lean4` is nil, with a non-vacuity check that the same
  lookup finds `rust`; bite-verified by adding a `lean4` config to
  `lsp.lua` and watching it fail. Also fixed a stale column in a
  `highlight.rs` comment.
- Verification on this branch: `cargo fmt --check` clean; strict workspace
  Clippy clean; 1,826 default + 2,003 CRDT library tests; lean4 Stage 1
  9/9; comment toggle 14; auto-pair 45; injection 4; M4 121; required GPU
  152; **isolated-config workspace sweep 3,150 across 90 suites**;
  `git diff --check` clean. The sweep needs an isolated `XDG_CONFIG_HOME`
  for the reason recorded in the bottom-panel lane below.
### Stage 2 — multi-root LSP server affinity (Q#LN15)

- Portable branch: `githubsucks/lsp-multi-root-affinity`, shared checkout,
  based on `githubsucks/main` @ `0827dd1`. Named for the substrate, not
  for Lean: **the diff contains no Lean content**, because `ensure_server`
  is the one server-affinity function every LSP language shares and a
  cross-cutting change to it must not be reviewable only as a Lean
  feature.
- Three files, no protocol change: `src/lua_bindings/mod.rs` (the
  `lsp.list()` row builder gains `root_uri` + `cwd`),
  `builtin/runtime/lsp.lua` (`project_root_for` returns `root, source`;
  `ensure_server` hoists it above the reuse loop and matches on it),
  `tests/lsp_multi_root_acceptance.rs` (9 tests, acceptance 13–21).
- **The rule that keeps this from regressing every other language: the
  affinity key is the root only when a root was actually FOUND.**
  `project_root_for` never returns nil for a file with a path — its last
  resort is the file's own directory — so a naive `(language_id, root)`
  key gives every directory of loose scratch files its own server, for
  every language. `source` is `"config" | "detected" | "fallback"` and
  only the first two become a key.
- **Wire-identical for the fallback case, and that is provable rather
  than hoped.** Matching is on the spawned spec's `root_uri` (nil matching
  nil), so the fallback spawn passes `root_uri = nil`; `cwd` still carries
  the directory and `build_initialize` derives the identical `rootUri`
  from `cwd` when the field is None, using a percent-encoder with the same
  allowed set as Lua's `file_uri_for`. `build_initialize` (`src/lsp.rs`)
  is the **only** reader of `spec.root_uri` in the tree.
- Deliberate behavior change, asserted not discovered: a server
  hand-spawned from `init.lua` with only `cwd` set also reads back nil, so
  a root-bearing attach will not adopt it.
- `config[language].root` may now be a `function(path) -> string|nil`,
  memoized per directory — needed because the hoist puts root resolution
  on every attach rather than every spawn. The memo is keyed **weakly by
  the resolver function itself**, so replacing `config[lang].root` cannot
  serve a root the previous resolver computed. This is Q#LN8's
  generalization landing early; the Lean resolver that uses it is Stage 3.
- Bite-verified three ways: 5/9 fail against the pre-change `lsp.lua`,
  8/9 against the pre-change `mod.rs`, and — the one that matters most —
  installing the naive always-key-on-root variant fails acceptance 20 and
  21 exactly as Q#LN15 part 2 predicts. The four that survive the first
  bite (13, 15, 16, 19) are the regression pins; passing on both sides is
  their job.
- Every fixture sets `pmacs.project.set_search_boundary` at its own
  tempdir root. Without it the marker walk climbs to the filesystem root
  and a stray `.git` above the temp directory turns the markerless cases
  into detected ones — the assertions would still pass while testing
  nothing.
- **Found but not fixed here (pre-existing, own lane):** `ensure_server`
  never forwards `cfg.restart` to `pmacs.lsp.spawn`, so a
  `restart = "never"` in `pmacs.lsp.config[lang]` is silently dropped on
  the auto-attach path. At least one existing test sets it believing it
  takes effect. Out of scope for a PR whose acceptance 16 pins existing
  attach behavior as unchanged.
- **Review round 1 addressed.** The blocker was process, not design: the
  test file was committed *before* `cargo fmt` ran, so the fix sat
  uncommitted in the working tree and the branch as pushed failed the
  first gate. The reported "fmt clean" described the worktree, not the
  branch — gate results are only meaningful when run against the pushed
  tree. Also added the two pins review asked for (a **string** `config
  .root` as an affinity key — acc17 only covered the function form; and
  `root = false` reading as unset), each bite-verified against exactly
  the mutation it targets and neither against the other. And documented
  the canonicalization obligation: the `"detected"` arm is canonicalized
  for free, a **configured** root is not, so on macOS a resolver
  returning `/var/…` and a detected `/private/var/…` are different keys
  for one directory. Stage 3's Lean resolver is the first real consumer,
  so the obligation is written at the point of use.
- Verification on this branch: `cargo fmt --check` clean; strict
  workspace Clippy clean; 1,826 default + 2,003 CRDT library tests;
  multi-root 11/11; M4 121; statusline 7; completion popup 9; auto-pair
  45; required GPU 155; **isolated-config workspace sweep 3,164 across 91
  suites**; `git diff --check` clean. The sweep needs an isolated
  `XDG_CONFIG_HOME` and `-- --skip basedpyright`.

### Stage 3a — dispatch seams + `pmacs.fs.canonicalize` — MERGED #167 (`main` @ `6f348c9`)

- Worktree `../pmacs-lean-stage3`, branched off `githubsucks/main` @
  `46a1b8f`. Carries framing **rev 5** (the Stage 3 split) as its first
  two commits, then the implementation, then a bite-driven correction.
- **Stage 2 merged as #161** (`main` @ `46a1b8f`, 2026-07-25, two review
  rounds). COHERENCE.md §7 records the slice; §1.2 records the dead
  `pmacs.error` channel found landing it.
- **Framing rev 5 splits Stage 3 into 3a and 3b** because rev 4 broke its
  own §4 rule — the row read "two `lsp.lua` generalizations" under prose
  claiming Stage 3 was Lean-only. One generalization shipped as Stage 2;
  the other (Q#LN9's seams) is the shared event drain, so it is now its
  own substrate stage. 3a and 3b are **strictly sequential** — 3b's
  subscriber is written against 3a's seam and both touch `lsp.lua`.
- Ships: `pmacs.lsp.on_notification` / `on_response`, two arms in
  `handle_server_requests`, a pending-response purge, and
  `pmacs.fs.canonicalize` (Q#LN20). No protocol change, no Lean content.
- **Two framing claims were corrected during implementation**, both
  recorded in §0.1 finding 6 and in the round-2 commit:
  1. The reachable leak is **not** a killed buffer. The Rust core fires
     exactly five hooks (`buffer.after-edit`, `buffer.after-load`,
     `buffer.after-switch`, `frontend.detached`, `process.after-tick`) —
     **there is no buffer-kill hook**, so nothing tears an attachment
     down and the drain keeps reaching that server. The real path is
     `attach_buffer` dropping a dead sid from `attachments` and
     rebuilding against a fresh server, which makes `crashed`/`stopped`
     the event *least* likely to be drained. Hence the purge polls
     `pmacs.lsp.list()` rather than riding the drain.
  2. Acceptance 32 does **not** pin "removed before invocation" —
     `pcall` catches the raise either way, so before/after is
     unobservable without a re-entrant drain. It pins removal being
     **unconditional**; renamed accordingly.
- **`pmacs._fs` is installed from `install_async`, not `install_project`**,
  purely for load order: `make_workspace` runs *after* `fs.lua` is
  evaluated, so a canonicalizer placed there reads nil. This cost one
  failing run to discover and is the kind of thing to check first.
- Bites recorded (all against the committed tree): removal gated on a
  clean return → acc32 fails 2 != 1; an event-driven purge → the
  no-attachment case fails "never called" while the attached case still
  passes; a resolver without `canonicalize` → two servers (34b's own
  falsification, which ships as a test).
- **Known unpinned:** the purge's generation (`attempt`) check. Reaching
  it needs a crash *and* its restart to fall in a gap with no
  `_async.tick`; the backoff is 500ms, so any tick sees `crashed` first
  and the absent-or-terminal arm fires. Labelled as defensive in the
  code rather than left looking covered.
- Verification on this branch: `cargo fmt --check` clean; strict
  workspace Clippy clean; 1,826 default + 2,003 CRDT library tests;
  dispatch seams 15/15 on Linux (14 on macOS — see below); multi-root
  13/13; M4 121; required GPU 155; **isolated-config workspace sweep
  3,189 across 93 suites, zero failures**; `git diff --check` clean.
- **Two flakes/portability facts from CI round 1, both worth keeping:**
  1. `composition_overhead_under_ten_percent` tripped once in a local
     sweep at 18.8% against a 10% budget, then passed 3/3 in isolation
     here, passed in isolation on main, and passed a full sweep rerun.
     The tell is in its own output: the same run reported realistic-frame
     overhead as **-4.6%**, and a negative figure is measurement noise,
     not added work. Load-sensitive under a parallel `--workspace` run.
  2. **A non-UTF-8 filename fixture cannot be built on macOS.** APFS
     enforces valid UTF-8, so `std::fs::write` fails with EILSEQ
     ("Illegal byte sequence") before the code under test is reached.
     `#[cfg(unix)]` is NOT sufficient for such a fixture —
     `#[cfg(target_os = "linux")]` is. Cost one red CI round to learn.

### Stage 3b — the Lean language server — MERGED #170 (`main` @ `d400f30`)

- Same worktree `../pmacs-lean-stage3`, **branched off
  `lean4-stage3a-seams`, not off `main`** — 3b consumes 3a's response
  seam and `pmacs.fs.canonicalize`, so it is strictly sequential.
  **Retarget PR #170 to `main` BEFORE merging #167, not after** — the
  kill-ring lesson exactly. (Round 1 of this ledger entry stated the
  reverse in its first sentence and the correct rule in the next; the
  review caught it. A safety rule written twice with opposite senses is
  worse than not written.)
- Ships `builtin/runtime/lean.lua` (new), one `include_str!` line in
  `src/editor.rs`, `pmacs.lsp._attach_buffer` exported from `lsp.lua`,
  a `leanprogress` mode plus `waitForDiagnostics` validation on
  `pmacs_fake_lsp`, and `tests/lean4_server_acceptance.rs` (40 tests).
  No protocol change.
- **Stage 1's acceptance 12 is half superseded and was rewritten, not
  deleted.** It asserted `pmacs.lsp.config.lean4 == nil` to catch a
  Stage-3 front-run; 3b is that stage. What survives is the restraint
  half — constructing an editor spawns nothing though the config now
  names `lake`, and opening a Lean buffer with no server configured
  spawns nothing — which is what holds Q#LN7's "not at init" promise.
- **The marker test is wrong in two opposite directions if done naively**
  and both are pinned: `io.open` SUCCEEDS on a directory (so truthiness
  accepts a `lean-toolchain` dir), but requiring a non-nil read rejects
  an EMPTY `lean-toolchain` (a legitimate marker — existence semantics,
  not content). Discriminator is `read`'s SECOND return; decline only on
  a non-nil err. Probed on LuaJIT 2.1.
- **Fifteen bites recorded, each against the committed tree.** R1: bare
  `io.open` → 24a fails / 24b passes; require-non-nil → 24b fails / 24a
  passes; no canonicalization → symlinked open spawns two servers; no
  re-attach after the swap → three latch tests fail; hook keyed on the
  attachment → the missing-`lake` case fails; `waitForDiagnostics`
  without `version` → acc37 fails with InvalidParams. R2: skip retiring
  a terminal server → `attempt` reaches 3; no originating-buffer gate →
  the Lean buffer is left on the `lake` stub; retry-forever → the
  failing-fallback test fails; version-probe any command → the
  working-wrapper test fails; no disabled guard → the unconfigured test
  sees "`nil` could not be started". R3: verdict keyed on `watching` →
  the late-verdict test finds the buffer still on `lake`; `buf_key`
  rewritten per load → the second-buffer test fails; hardcoded
  `lake serve` → the wrapper-naming test fails.
- **Round-2 review: three more P1 lifecycle defects, suite 20/20 with
  all of them live.** (1) The crashed primary respawned forever —
  skipping the retire call avoided corrupting terminal servers but left
  `next_restart_at` armed. **`forget` is the call for a TERMINAL server**
  (it requires terminal state and removes the client, dropping the
  restart timer); `stop` is for a live one and corrupts a terminal one.
  (2) Re-attachment targeted whatever buffer was active when the async
  verdict landed; an unrelated Rust attachment satisfied "a different
  server id". (3) A failing fallback retried every tick forever, silent.
  Plus two P2s: the Lake version parser was applied to arbitrary wrapper
  output, and an UNCONFIGURED `config.lean4` was reported as failure and
  latched, poisoning the session.
- **Round-3 review: two more P1s, both asynchronous correlation, suite
  25/25.** (a) `probe.watching` is cleared when the server initializes,
  so a SLOW version verdict arrived with nil and retired nothing —
  `_attach_buffer` returned the still-live primary and the retry called
  it success, so status and config said "fell back" while the buffer
  stayed put. **That is the round-1 silent no-op reached through a third
  event ordering.** `probe.primary` is now separate from
  `probe.watching` and survives initialization. (b) `buf_key` was
  rewritten on every Lean `after-load`, so a second Lean buffer opened
  before the verdict became the rebuild target while the latch still
  watched the first buffer's server. Target buffer and primary server
  are one fact and are now armed together, once. Plus a P2: the failure
  message hardcoded `lake serve` after the latch became
  command-agnostic, sending wrapper users to debug the wrong binary.
- **Round-4 review: one P1, and it is the same defect a FOURTH time.**
  `pmacs.lsp.config.lean4` is a single global entry, so swapping its
  command invalidates **every** Lean buffer and **every** Lean server —
  Q#LN15 gives one per project root. Rounds 1–3 each fixed the repair
  for one buffer and one server; round 4 is "repair the armed target,
  strand the rest". The shape that finally holds: retire ALL `lean4`
  servers on latch, and repair each buffer **lazily and at most once**
  when it becomes active (`buffer.after-switch` + the tick), because
  `_attach_buffer` is active-buffer-only and cannot reach the others.
  The per-buffer once-only bound is what stops a failing fallback
  retrying forever — the round-2 defect a naive global repair loop would
  have reintroduced for every buffer instead of one. Plus a P2: the
  argument-inclusive attribution was implemented but pinned only by
  "contains the command name", so a mutation dropping every argument
  still passed.
- **Round-5 review: one P1 plus a frontend scope hole, and four more.**
  (1) A fallback that SPAWNS and then dies retried forever: the
  once-per-buffer guard bounds `_attach_buffer`, not the server it
  produced, and `ensure_server` never forwards `cfg.restart` so the
  fallback inherits `OnCrash` — respawned by the manager with no
  ceiling, silently, because `latched` had disabled the primary's poll.
  The fallback now gets its own one-shot die-before-initialize watch.
  (2) **Simultaneous frontends**: both repair triggers read the ambient
  `pmacs.window.buffer()`, and the daemon restores `active_frontend` to
  the last-dispatched one before `tick_processes`, so a Lean buffer
  active in ANOTHER frontend gets no `after-switch` and stays stale.
  Fixed at the right seam — **make CONSUMPTION safe**: both
  `attached_for_active` and `attachment_for_request` now refuse a record
  whose server is dead (the former rebuilds, the latter reports none,
  since it must not perturb LSP state). Healing at the point of use is
  frontend-agnostic, because whichever frontend runs a command is active
  while it runs. (3) The retirement sweep selected on `language_id`, so
  it stopped USER-spawned Lean servers too; it now keys on the
  `default-lean4` label `ensure_server` stamps, which is the derivation
  discriminator. (4) `probe.latched` gated repair even when NO swap
  occurred, so an already-fallback config was retried and misreported.
  Split out `probe.fallback_installed`. (5) The once-per-buffer
  assertion counted TABLE KEYS, which cannot distinguish "once per
  buffer" from "every tick for one buffer" — cardinality stays 1 either
  way. Now a numeric attempt counter; the bite shows **174 vs 1**.
- **Round-6 review: four P1s and one P2, suite 40/40.** (1) General
  point-of-use healing treated a crashed OnCrash server as absent and
  spawned beside it while its old id still had `next_restart_at` armed;
  `attach_buffer` now forgets a terminal record before replacement.
  `attachment_for_request` remains non-attaching and preserves the
  record, so a same-id restart can recover instead of being orphaned.
  (2) The fallback watch was scalar, while Q#LN15 permits simultaneous
  per-root servers and lsp.lua can create them without passing through
  Lean's repair function. Watches are now per-SID and discover every
  config-driven Lean server from a private origin table. (3) The shipped
  `lean.wait-for-diagnostics` command bypassed both safe resolvers and
  still consumed a stopped record; it now uses a command-safe resolver,
  waits asynchronously for a healed replacement to initialize, and the
  test requires the real request to finish. (4) When no config swap
  occurred, one failed root still swept a healthy root; that arm now
  retires only the SID whose verdict fired. (5) `label` is public and
  unreserved, therefore not ownership. lsp.lua records successful
  config-driven spawns privately, and every Lean lifecycle decision keys
  on that origin fact; the user-server pin deliberately collides on
  `default-lean4`. All five bites against `19f48d4` discriminate: the
  old files produce 2 same-root servers, a fallback attempt of 4, a
  shipped command still targeting `stopped`, retirement of the healthy
  root, and retirement of the colliding user server, respectively.
- **DURABLE LESSON — "the test that passes" vs "the test that
  discriminates."** Green tests across six rounds repeatedly pinned only
  a nearby helper or an absence, and only biting exposed it. **Carry this
  to `docs/agent-handoff.md` when the lane lands.** The concrete shapes,
  all from this branch:
  1. R1 acceptance 36 asserted "every server is terminal" — pinning the
     ABSENCE of the fallback it claimed to test.
  2. "No live non-fallback server" misses a respawn loop: a respawning
     server sits in `crashed` most of the time. `attempt` counts
     respawns; liveness does not.
  3. Returning to a buffer via `find_or_open` re-fires
     `buffer.after-load`, which repairs the attachment regardless of the
     code under test. Use `switch_buffer`.
  4. A MISSING executable fails synchronously inside `after-load`, where
     the rebuild happens inline — no async race can occur. Only the
     probe path exercises asynchronous ordering.
  5. A mutation that RAISES (indexing a nil config) is swallowed by the
     hook's pcall, so the bite "passes" for the wrong reason. A bite must
     reproduce the original shape, not merely break the code.
  6. A fixture whose `serve` sleeps can never let the primary initialize
     first, so it cannot reach the ordering where a late verdict must
     retire a LIVE server.
  7. Asserting on a field that no longer exists (`_probe.reattach_from`
     after a refactor) reads as nil and passes for nothing. Assert
     positive facts — a count, a command string — not absences.
  8. Counting DISTINCT KEYS cannot bound REPEATED WORK: a per-tick retry
     on one buffer keeps `#repaired == 1` forever. Count the attempts,
     not the things attempted against (bite: 174 vs 1).
  9. A NONEXISTENT executable only exercises synchronous ENOENT. To
     reach "spawned, then died", the fixture must actually spawn.
  10. Calling the two SAFE HELPERS directly does not pin a shipped
      command that bypasses both. Drive the command registry entry and
      require its terminal result — replacing a dead record with a
      `starting` server is still not success if the request is issued
      before initialize.
  Rule: **a test is not evidence until the mutation it targets has been
  shown to fail it.**
- **SECOND DURABLE LESSON — a scope error repeats until the scope is
  named.** The "fallback silently does not happen" defect came back four
  times: no re-attach; re-attach cleared by an unrelated buffer;
  re-attach satisfied by the server being replaced; re-attach of one
  buffer while the others stay stale. Every fix was locally correct and
  none asked *what does this config swap invalidate?* — the answer being
  every Lean buffer and every Lean server, because the config entry is
  global and servers are per-root. **When a change edits shared state,
  enumerate everything derived from it before repairing anything.**
- **SUBSTRATE BUG FOUND, not fixed here (framing §6).**
  `LspManager::stop` on an ALREADY-terminal server takes its
  not-initialized branch, terminates the dead process and sets
  `ShuttingDown { .. None }` on the premise that "the next exit
  observation cleans up" — but the exit already happened, which is what
  made it `Crashed`. No further event arrives, so the client is stuck in
  `ShuttingDown` **forever**: `server_is_live` reads it as LIVE, so
  `attach_buffer` never rebuilds, and `forget` refuses it for not being
  terminal. **Stopping a dead server is what makes it un-replaceable.**
  Lean works around it by dispatching on state: `forget` when
  terminal, `stop` when live. Merely SKIPPING the call is not
  enough — that leaves `next_restart_at` armed.
- Round-1 review found four P1s, all real: the latch swapped the config
  but never spawned or re-attached (and acc36 *asserted every server was
  terminal*, pinning the absence of the fallback); a missing `lake`
  bypassed probe and latch entirely because the hook keyed on an
  attachment that ENOENT prevents; `waitForDiagnostics` omitted the
  `version` Lean requires; and the ledger stated the dangerous stacking
  order.
- The probe's non-zero exit is deliberately NOT a fallback trigger —
  §2.9's elan shim makes `lake --version` fail where `lake serve` still
  works. Only a parseable version below 3.1.0 triggers it; the
  server-failure latch covers the rest.
- Verification on this branch: `cargo fmt --check` clean; strict
  workspace Clippy clean; 1,829 default + 2,003 CRDT library tests;
  lean4 server 40/40; lean4 stage 1 9/9; dispatch seams 15/15;
  multi-root 13/13; M4 121; required GPU 155; **isolated-config
  serial workspace sweep 3,229 across 94 suites, zero failures**;
  `git diff --check` clean. (Round 1 of
  this entry recorded 17/17 and 3,206 — the PRE-fix counts — after the
  fixes were pushed. The ledger's protocol is that verification
  describes the pushed tree; recording it late is the #161 fmt-blocker
  error in a slower form.)

### Stage 4 — framing rev 6, split into 4a/4b (branch `lean4-stage4a-typed-edit-chain`)

- Stages 3a and 3b **merged as #167** (`main` @ `6f348c9`) and **#170**
  (`main` @ `d400f30`), 2026-07-26. Both were integrated against a main
  that had advanced 50 commits mid-review; the only conflict either time
  was this ledger's own lane headings, resolved by keeping both sides.
- Worktree `../pmacs-lean-stage4`, branched off `main` @ `d400f30`.
  Framing-only so far: `docs/lean4-mode-framing.md` **revision 6**. No
  code. Awaiting user approval before implementation, per the workflow.
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
  carry `$CURSOR`, **93** are multi-codepoint.
- Citation sweep per COHERENCE §25: five live citations moved in the 50
  commits since rev 5 — `take_typed_edit` 12827→12990,
  `handle_server_requests` 1549→1815, `fs.stat` 93→133,
  `detect_buffer_language` 452→457, `send_request`/`send_notification`
  9342/9361→9507/9527.
- Verification: none yet — the branch carries no code. `git diff --check`
  clean.

## Dired lane — Stage 0 MERGED; Stage 1 IN REVIEW (PR #165)

- Approved framing: `docs/dired-framing.md` **revision 6** — rev 5 is the
  approved text (merged as its own docs PR #164), rev 6 adds §0's Stage 1
  implementation notes (S1-1…S1-9). Stages 2 (marks and operations) and 3
  (wdired) each get their own detailed framing after the prior stage lands.
- **Stage 0 (`C-x C-f` find-file) MERGED as #162** (`main` @ `2af1ab3`,
  2026-07-25, one review round, 12/12 CI green). Durable facts moved to
  `docs/agent-handoff.md` §1 per rule 3 below.
- **Stage 1 branch: `githubsucks/dired-stage1`**, worktree
  `../pmacs-dired-stage1`, based on `githubsucks/main` @ `8c86d34` (the
  framing merge #164). **A fresh cut, not a rebase:** the older `dired`
  branch (`ffdd642`, worktree `../pmacs-dired-arc`) was based on the
  superseded `0827dd1` and carried only the framing content #164 already
  put on `main`, so merging it would have reconciled two histories of one
  document. It is left untouched and carries nothing unmerged.
- **Stage 1 implemented; no wire change (protocol stays v20).** What
  landed on the branch:
  - `builtin/runtime/dired.lua`: one buffer per directory named
    `*dired:<canonical path>*` with the handle-table ownership check;
    read-only intercept + `set_round_trip_input`; the `dired` major mode
    and its mode-scoped keymap (`RET`/`f`, `^`, `n`/`p`, `g`, `q`, `s`);
    basename cursor re-seating across every wholesale repaint;
    `display_file` for file visits and same-window reuse for directory
    descent; `C-x d` / `C-x C-j`; the `dired.kill-when-opening` setting.
    Loaded after `window.lua`.
  - `src/fs.rs`: `ReadDirTolerance`, `FsDirEntryError`, `FsDirListing`,
    and one walk that either fails on a per-entry condition or records it
    (Q#DR6). `src/async_runtime.rs` carries the listing in
    `ReplyKind::ReadDir` / `JobResult::ReadDir`; `src/lua_bindings/mod.rs`
    keys the Lua result **shape** on `errors.is_some()`, so the bare array
    the frozen M8.2 fixture consumes with `ipairs` is untouched;
    `builtin/runtime/fs.lua` validates read-op opts and **rejects unknown
    keys** (a typo'd `tolerant` used to degrade silently to fatal).
  - `src/editor_core.rs` + `src/lua_bindings/mod.rs`:
    `normalize_buffer_path` is `pub` and exposed as
    `pmacs.path.canonicalize` — Q#DR2's preferred end state, so no Lua
    mirror exists and Stage 2 owes no mirror removal. This makes B2
    ("tolerant `read_dir` is the only Rust change") false by one small
    binding, deliberately.
  - `tests/dired_acceptance.rs`: 22 tests over framing items 1–16,
    dispatch-driven; item 17 is the m8_1/m8_2/m8_3 additivity gate.
- **The framing claim the substrate falsified (S1-2):** R2-3 expected a
  dedicated dired panel to carry its dedication across a descent.
  `display_buffer` never replaces the buffer in a slot dedicated to
  another one — it discards every side-specific parameter and falls back
  to the document window (Q#BP3 2.iii), and the exact-window arm errors.
  Dired does not unpin the user's panel; both arms are pinned.
- **The vacuity the bites found (S1-3):** acceptance 3c cannot pin the
  descent *routing*. Dired holds focus in its own panel, so a raw
  `switch_buffer` lands in the same window and every 3c assertion holds
  either way. Dedication is the only discriminator, so the
  dedicated-panel test is the real pin — and the vacuity is documented at
  the assertion rather than relabelled.
- **The pre-existing test dired's first mode-scoped binding broke
  (S1-4):** `describe_key_identifies_every_default_binding` asserted every
  binding in the stack resolves through `describe.key` context-free, which
  held only while the modes table was empty. It now sets the effective
  context per binding and explicitly *clears* the mode for global ones,
  because a leaked mode legitimately shadows a global chord of the same
  name (dired's `RET` shadows `edit.newline-and-indent`).
- Durable substrate facts, independent of this arc:
  - `pmacs.buffer.kill` (not `remove`) redirects windows off a doomed
    buffer before removal, so `kill-when-opening` kills **after** the
    replacement is displayed.
  - Interactive origin does **not** survive an await: work resumed in
    `tick_async` sees no `InteractiveCommandOrigin`, so `pmacs.window.*`
    acts for the *ambient* active frontend (S1-9).
  - Kinds are lstat-based in both `read_dir` and `stat`, so nothing in an
    entry says whether a symlink points at a directory; `RET` probes by
    trying to list it (S1-8).
  - A path-backed buffer's *name* is its full path, not its basename —
    worth knowing before writing any name assertion.
  - `C-x d` takes **no** completion source on purpose (S1-5): with one,
    RET on an empty field opens whatever sorts first, and
    RET-on-where-you-are is the gesture the binding exists for. The field
    is prefilled instead.
- **Bite verification:** 15 claims, each mutated in place and required to
  fail the test that names it. `dired.lua` is new, so `scripts/bite`'s
  file swap does not apply; every mutation was applied and reverted with
  `git checkout --`. One came back VACUOUS and is recorded above.
- **Review round 1 addressed** (framing rev 7, S1-10…S1-12). Three
  behavioral fixes, each bite-verified: `dired.revert`'s re-seat is
  guarded on the active buffer (an ambient `move_to_line` after an await
  moved an unrelated buffer's cursor — the buffer-level instance of
  S1-9); `fmt_size` keeps the column width past ten digits, because
  `_layout` is a contract Stage 3 is planned against; and the symlink
  descent dropped its probe, since `open_directory`'s
  changed-nothing-on-failure invariant *is* the probe (it was listing the
  target directory twice). Plus a consecutive-`readdir`-error cap, because
  **nothing cancels a dired listing** — it carries no supersede key, so
  cancellation was never the backstop the tolerant loop implicitly relied
  on. Naming/comment findings taken as-is.
  - Durable process lesson, hit twice now: a mutation-bite helper restores
    with `git checkout --`, which reverts to **HEAD** — so a fix must be
    committed *before* it is bitten. Round 1's fixes were briefly wiped by
    exactly that.
- **Canonical main integrated twice** — at `46a1b8f` (multi-root LSP
  affinity #161) and again at `b889873` (GPU terminal input #166), both
  merged rather than rebased per the #135/#137 precedent so the review
  anchors stay addressable. Each conflict was a single doc hunk resolved
  as the union: this lane owns COHERENCE's journey step 7 file half, #161
  owns the in-flight list, #166 owns step 8's GPU-terminal addendum.
  Three things worth carrying:
  - **A conflicting PR silently stops running CI.** GitHub builds
    `pull_request` runs against the merge ref, which does not exist while
    the PR conflicts, so no run is created and nothing reports a
    failure — the checks list simply stays as it was. Three pushes to
    this branch produced no CI at all before the cause was found. Watch
    `mergeable` on a long-lived lane, not just the check list.
  - #161's own COHERENCE finding **falsified a claim in this lane's
    module doc**: `pmacs.error` is never defined in production, so an
    uncaught raise inside a `pmacs.async` coroutine does not reach
    `*errors*` as the comment said. It reaches a bare `error()` inside
    `pmacs._async.tick()`, whose result `tick_async` discards with
    `let _ =` — i.e. nowhere. That makes dired's per-coroutine `pcall` +
    `set_status` load-bearing rather than tidy, and the comment now says
    so.
  - **A lane in review against a fast-moving `main` needs its gates rerun
    per integration, not per push.** Main advanced twice inside this
    review round, and the second time landed while the first
    integration's sweep was still running. The numbers below describe the
    twice-merged tree.
- Verification on the twice-merged tree (`main` @ `b889873`):
  `cargo fmt --check` clean; strict workspace Clippy clean; **1,832
  default + 2,009 CRDT** library tests; dired acceptance **25 default +
  25 CRDT**; m8_1 10 / m8_2 15 / m8_3 32 unchanged; multi-root 13 and
  vterm Stage 3 5 (both suites main added, green under this lane's
  `mod.rs` and `editor.rs` changes); M4 121; required GPU 155;
  **isolated-`XDG_CONFIG_HOME` workspace sweep 3,205 passed across 93
  suites, zero failures**; `git diff --check` clean. The sweep needs the
  isolated config for the reason recorded in the bottom-panel lane
  below.
- Coherence (framing §0.5, required since #163): serves `COHERENCE.md` §20
  Priority 1, which names this work explicitly; journey step 7's file half
  goes from no surface to a surface; **adds no interaction island** — keys
  are a mode-scoped keymap, and wdired will be a mode swap; adopts
  `pmacs.config` for `dired.kill-when-opening`; inherits §9's
  worker-attribution gap for its `read_dir` jobs without worsening it. The
  audited claims this changes are updated in `COHERENCE.md` itself, per its
  §25.
- **Boundary with the Journey Stage 1 arc** (`COHERENCE.md` §20 arc-cut
  1): CLI directory-argument handling (`pmacs .` exits 1) belongs there,
  not here — Stage 1 does **not** fix it. The two meet at
  `resolve_target_buffer`; dired supplies the buffer a directory should
  resolve *to*, and `pmacs .` should route into it rather than growing a
  second directory surface.

## GPU terminal input lane — IN REVIEW

- Portable branch: `githubsucks/gpu-terminal-input`, worktree
  `../pmacs-gui-term-input`, based on `githubsucks/main` @ `46a1b8f`.
- Approved framing: `docs/gpu-terminal-input-framing.md` revision 2,
  committed as the branch's first commit (`9a0df21`). Bug fix, not a
  feature; **no protocol change (stays v20)**.
- Reported as "text input within the terminal doesn't work on GUI, this is
  fine in TUI". Root cause: the dispatcher applied **both** terminal-layout
  syncs to **every** attached frontend, and a semantic session satisfies both
  conditions (a `term_sizes` entry from `AttachRequest` *and* a terminal
  declaration). Its PTY was resized twice per tick forever — grid arm installs
  the TUI placement size, semantic arm installs the declared content
  rectangle, each arm's idempotence guard seeing only what the other just
  wrote — so the child took a `SIGWINCH` storm at tick cadence.
- **The fix is a split, not a guard.** The grid arm is also the only per-tick
  controller-liveness release a semantic frontend gets, and
  `sync_semantic_terminal_layout` cannot take that over: the buffer-follow
  snapshot clears the viewport declaration (`on_buffer_snapshot_sent`), so
  that arm stops running in exactly the switch-away case that needs the
  release. `sync_terminal_layout` is therefore split into a
  frontend-kind-neutral half (panel reconcile + liveness) and a grid-only
  geometry half, with the loop body extracted to
  `sync_terminal_layouts_for_tick` so the exclusivity is structural and tests
  drive the real thing.
- **Trap for anyone touching this again:** the release at the "no
  `window_placements` entry" arm reads like liveness and is grid geometry. A
  semantic frontend has no placement entry at all, so moving it into the
  neutral half releases a GPU controller every tick.
- Bite-verified against **two** pre-images, because the naive guard fixes the
  storm and introduces the leak:

  | pin | `main` | naive guard | the split |
  |---|---|---|---|
  | settle (acc 2+3) | FAIL | pass | pass |
  | controller release (acc 6) | pass | FAIL | pass |
  | grid still resizes (acc 5) | pass | pass | pass |

- Real-path evidence: a quiet child trapping `SIGWINCH` reports **144 frames
  in 4 s and `WINCH 1..12` on screen** against the pre-fix tree, versus a
  settled screen with the fix.
- **Deliberately out of scope, named:** interactive-shell echo on a raw-mode
  PTY (Q#GT5 — reproduces in-process too, so it is not the GUI/TUI
  asymmetry), and a geometry change appearing to clear the visible screen
  (reproduces pre-fix; why acceptance 4 latches its observation across
  frames).
- Verification on this branch: `cargo fmt --check` clean; strict workspace
  Clippy clean; 1,829 default + 2,006 CRDT library tests; vterm Stage 1/2/3
  10 / 6 / 9 CRDT; bottom-panel Stage 1 46; M4 121; required GPU 155;
  **isolated-config workspace sweep 3,177 across 92 suites, zero failures**;
  `git diff --check` clean. Gates were run against the committed tree.

## Terminal config + copy mode arc — Stage 1 IN REVIEW

- Approved framing: `docs/terminal-config-and-copy-mode-framing.md`
  **revision 4** (four review rounds), committed as the first commit of
  Stage 1's branch. Two stages, two branches, two PRs; **no protocol
  change**.
- **Stage 1 = `githubsucks/terminal-config`**, worktree
  `../pmacs-terminal-config`, based on `githubsucks/main` @ `d152120`
  and merged up to `c93f9ee` during review round 1. Profiles,
  scrollback, escape key, and the `C-c t` opening binding.
- **Stage 2 = `terminal-copy-mode`, not started.** Branch it off `main`
  after Stage 1 merges: no dependency, but both edit
  `builtin/runtime/terminal.lua`.
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

## Bottom-panel lane (Arc 7) — Stage 1 MERGED; Stage 2 IN FRAMING

Stage 1 is on `main`. **Stage 2 is in framing**, no implementation in
flight.

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
