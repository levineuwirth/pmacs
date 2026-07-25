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
  `githubsucks/main` @ `8c86d34` (the dired framing #164 atop find-file
  #162, COHERENCE.md #163, Lean 4 Stage 1 #160, the minimap blank-slab fix
  #159, bottom-panel Stage 1 #155, the inline-math re-scout #154, the vterm
  PTY-flake fix #153, and the GPU initial-target doc refresh #152; protocol
  v20).
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

The `git log` command must expose `8c86d34` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Inline-math slice lane — PR #158 OPEN, main integrated

- Portable branch: `githubsucks/inline-math-slice`; worktree
  `../pmacs-math-slice`. **PR #158**, base `main`.
- **Canonical `main` merged into the lane three times on 2026-07-25**,
  every time merged rather than rebased, per the #135/#137 precedent: the
  PR is awaiting review rounds and a rebase would break every review
  anchor.
  - First at `8c86d34` (28 commits behind). The conflict was
    **pre-existing**, not introduced by the dired (#164) or Lean 4
    ledger commits; it already conflicted against `main` @ `e745068`.
  - Then at `46a1b8f`, after Lean 4 Stage 2 (#161) landed while this
    branch's CI was still running. Same single conflict, same shape,
    same resolution.
  - Then at `b889873`, after the GPU terminal input fix (#166) landed.
    **No conflict at all this time** — and that is exactly why it still
    needed a real integration, see below.
- **The first two conflicts were this ledger and nothing else** — both
  sides' lanes kept verbatim each time. That is the standing cost of a
  long-lived PR here: every merge to `main` edits this file, so a branch
  awaiting review re-conflicts on it and only on it. It is a docs
  collision, never a code one, and it says nothing about integration
  risk — do not read a `CONFLICTING` badge on this PR as a code signal
  without checking which file `git merge-tree` names.
- **The inverse trap matters more, and #166 is the case in point: a
  CLEAN `git merge-tree` is not a reason to skip integrating.** #166
  landed 41 lines in `pmacs-gpu/src/main.rs`, the same heavily-rewritten
  file as the first integration, and git merged it without a murmur
  because the two edits sit in different regions (#166 is entirely in the
  headless probe — `PMACS_GPU_PROBE_OBSERVE_MS`, `PROBE_INPUT_CHAR`,
  `input_echo_observed` — while this lane rewrites the render path).
  Merging the PR on the strength of that clean auto-merge would have
  shipped a combination no gate had ever run. **Decide whether to
  integrate from the shared-FILE set, not from whether git complained.**
- **First integration's surface** (derived from `git diff
  <merge-base>..main`, not from another PR's file list):
  `pmacs-gpu/src/main.rs` gained 72 lines on main from `e547a90` — the
  minimap all-blank-slab divide-by-zero fix — and this lane rewrites
  large parts of the same file. Git auto-merged it **textually**; a
  clean auto-merge is not evidence the tree compiles (the folding-arc
  lesson), so the full gate suite below is what actually discharges it.
- **Second integration's surface is code-disjoint.** #161 touched
  `COHERENCE.md`, `builtin/runtime/lsp.lua`, `src/lua_bindings/mod.rs`,
  and a new `tests/lsp_multi_root_acceptance.rs`; intersecting that
  against this lane's own changed-file set leaves exactly one entry,
  `docs/active-work.md`. No source file is touched by both sides, so
  this one carries none of the first integration's semantic risk.
- **CI ran on this branch for the first time on 2026-07-25 and passed
  all twelve** (Format, both Lints, GPU Render headless, all four Test
  matrix jobs, M1/M4/M5/M6 gates) at `8b457de` — the first-integration
  tip. Before that there were zero workflow runs since the PR opened on
  2026-07-24, while every other open PR had a full run; not a fork and
  not a trigger-config issue (the workflow fires on all `pull_request`
  events), cause never identified. So the green run **validates the
  first integration, including the `pmacs-gpu/src/main.rs` auto-merge,
  on macOS and Linux both** — the platforms local gating could not
  cover. The second and third integrations get their own CI run on the
  push that carries them.
- Framing: `docs/inline-math-slice-framing.md` rev 3, approved after two
  review rounds; parent arc framing merged as #154.
- State: parser, font bundle (GUST licence), MATH-table layout with the
  measured height budget, currency-guarded detection, and the
  `ChunkSource::MathBox` spacer substrate are implemented and
  round-3-reviewed (review fixes at `cbf7782`: the exclusive-`end`
  mapping bug its own test had pinned, script-marker whitespace, fallible
  layout via `UncoverableGlyph`, real fraction gap-min constants —
  flagship scale 0.867, fallback depth 5).
- Caret-driven suppression (the Q#MS5 gate over the effective caret and
  Q#MS11 selection endpoints, chunk substitution before tab expansion,
  the line-reuse predicate's third input, and the CursorByte /
  optimistic-edit / Decorations refresh triggers), the draw pass
  (per-glyph mini-buffers positioned by each shaped line's real
  baseline, fraction-rule quads over the washes, the F8b family pin),
  the Q#MS11 whole-rectangle wash widening, and the pixel acceptance
  battery (criteria 5–11, 14–16; 17 discharged by the differential
  `cargo tree -e features` check — byte-identical with and without the
  dependency line) are implemented on the branch tip.
- Clippy is CLEAN on the whole workspace at `-D warnings` — the draw
  pass consumed every formerly-dead item.
- Verification **pre-integration** (at `14c1c01`, against the old base):
  199 `pmacs-gpu` tests under `PMACS_REQUIRE_GPU=1`; 1,815 default +
  1,992 CRDT library tests; M4 121; full workspace sweep green (isolated
  `XDG_CONFIG_HOME`). Superseded by the post-integration run below —
  those numbers describe a tree 28 commits behind.
- Verification after the **first** integration: `cargo fmt --check`
  clean; strict workspace Clippy clean; 1,826 default + 2,003 CRDT
  library tests; **202 `pmacs-gpu` tests under `PMACS_REQUIRE_GPU=1`**;
  M4 121; isolated-`XDG_CONFIG_HOME` `--no-fail-fast` workspace sweep
  3,208 across 91 suites, zero failures; `git diff --check` clean.
- Verification after the **third** integration (this is the set that
  describes what the PR now proposes): fmt clean; `git diff --check`
  clean; strict workspace Clippy clean; **1,829 default + 2,006 CRDT**
  library tests; **202 `pmacs-gpu`** under `PMACS_REQUIRE_GPU=1`; M4 121;
  **isolated-`XDG_CONFIG_HOME` `--no-fail-fast` sweep 3,224 across 92
  suites, zero failures**.
- **Test-count reconciliation is the integration proof, not the pass.**
  Run it against what the other side actually added, per merge:
  - First: GPU 199 → **202**, and `e547a90` added exactly **3**
    `pmacs-gpu` tests — the whole delta on main since the merge base.
    Structurally spot-checked too: main's fix survives as
    `(count > 0).then(|| MinimapLineShape {` (the deferred closure,
    **not** the eager `then_some`) with its regression test.
  - Third: #166 adds **3** library tests, **2** to
    `vterm_stage3_acceptance`, and **0** to `pmacs-gpu`. Predicted lib
    1,826 → 1,829, CRDT 2,003 → 2,006, GPU unchanged at 202 — and that
    is exactly what ran. Suite count 91 → **92** is #161's new
    `tests/lsp_multi_root_acceptance.rs` binary. All three sides'
    markers confirmed live in `pmacs-gpu/src/main.rs`: #166's probe
    symbols, this lane's `math_plan_for_line` / `math_gates_match` /
    `cached_math_subs_for_slice` / `widen_over_math_chunks` / 21
    `MathBox` references, and the first integration's minimap fix.
- **Ops trap, cost hours: `m4_5_basedpyright_initializes_and_negotiates_
  encoding` does not time out — it hangs forever.** A `--workspace`
  sweep parks on `m4_acceptance` with a live
  `basedpyright/langserver.index.js` child and never advances (observed
  stuck at 38 of 92 suites for 2h26m). The per-suite M4 gate already
  carries `-- --skip basedpyright`; **the workspace sweep needs the same
  flag** — `cargo test --workspace --no-fail-fast -- --skip
  basedpyright` (libtest filters apply to every binary; verify it bit by
  checking the run reports exactly 1 filtered out). Do not read a
  long-running sweep as "slow": check whether the suite count is
  advancing.
- Remaining: the user's review pass.
  Named v0 approximations: the peer-caret half of acceptance 14 is
  pinned at the mapping level (unit tests), not pixels; a soft-wrapped
  spacer draws its box whole at the first run's origin; the fit budget
  reads the bundled code face even under a custom `set_font` family
  (the draw anchors to the real shaped baseline either way).
## Lean 4 lane (Arc 8) — Stage 1 MERGED; Stage 2 IN REVIEW (PR #161)

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

## Bottom-panel lane (Arc 7) — Stage 1 MERGED; Stage 2 (GPU band) is next

Stage 1 is on `main`; nothing in this arc is in flight. Stage 2 has **no
branch and no framing yet** — the approved parent framing
`docs/bottom-panel-framing.md` (rev 4) is what it re-scouts against.

- Stage 1 merged as **#155** (`main` @ `e745068`, 2026-07-24, after two
  review rounds). No protocol change. Durable substrate facts live in
  `docs/agent-handoff.md` §1; the two round lessons are in §5.
- Retained, carrying nothing unmerged: branch `bottom-panel` and worktree
  `../pmacs-bottom-panel`.
- **Stage 2 obligations, already named by the framing** — the starting
  point for its own framing doc: `InstanceMessage::PanelFrame` plus
  `FrontendEvent::{FrontendCellGeometry, PanelResizeRows, PanelPointer}`
  at the next available protocol version, gated in both directions and
  each extended enum byte-pinned on its own previous final variant;
  extracting `paint_frame`'s per-window body *together with* the
  active-window auto-scroll preparation; routing every consumer in the
  framing's §1.3 census of 23 transitive active-context reads through
  `primary_document_window`; the focus-chrome surface matrix (Q#BP14b);
  and Q#BP17's fold-projection parameter plus the stale invariant comment
  at `src/window.rs`. Stage 3 is the adopter default flip.
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
