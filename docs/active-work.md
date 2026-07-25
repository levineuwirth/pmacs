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
  `githubsucks/main` @ `0dd16a5` (GPU initial-target #148 atop folding Stage 2
  landed-doc refresh #150, folding Stage 2 #149, the ledger refresh #147, web
  grammars HTML+CSS #146, and the LaTeX Stage 1 #144 / inline-math framing
  #145 pair; protocol v20).
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

The `git log` command must expose `0dd16a5` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Inline-math slice lane — PR #158 OPEN, main integrated

- Portable branch: `githubsucks/inline-math-slice`; worktree
  `../pmacs-math-slice`. **PR #158**, base `main`.
- **Canonical `main` merged into the lane twice on 2026-07-25**, both
  times merged rather than rebased, per the #135/#137 precedent: the PR
  is awaiting review rounds and a rebase would break every review anchor.
  - First at `8c86d34` (28 commits behind). The conflict was
    **pre-existing**, not introduced by the dired (#164) or Lean 4
    ledger commits; it already conflicted against `main` @ `e745068`.
  - Then at `46a1b8f`, after Lean 4 Stage 2 (#161) landed while this
    branch's CI was still running. Same single conflict, same shape,
    same resolution.
- **Both conflicts were this ledger and nothing else** — both sides'
  lanes kept verbatim each time. That is the standing cost of a
  long-lived PR here: every merge to `main` edits this file, so a branch
  awaiting review re-conflicts on it and only on it. It is a docs
  collision, never a code one, and it says nothing about integration
  risk — do not read a `CONFLICTING` badge on this PR as a code signal
  without checking which file `git merge-tree` names.
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
  cover. The second integration is not yet CI-covered, but its surface
  is the ledger alone.
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
- Verification **post-integration** (the merge commit; this is the set
  that describes what the PR now proposes): `cargo fmt --check` clean;
  strict workspace Clippy clean; 1,826 default + 2,003 CRDT library
  tests; **202 `pmacs-gpu` tests under `PMACS_REQUIRE_GPU=1`**; M4 121;
  **isolated-`XDG_CONFIG_HOME` `--no-fail-fast` workspace sweep 3,208
  across 91 suites, zero failures**; `git diff --check` clean.
- **The GPU count is the integration proof, not just a pass.** It went
  199 → **202**, and `e547a90` added exactly **3** tests to `pmacs-gpu`
  — the whole delta on main since the merge base. So both sides' tests
  are present and running; neither was dropped by the auto-merge.
  Spot-checked structurally too: main's fix survives as
  `(count > 0).then(|| MinimapLineShape {` (the deferred closure, **not**
  the eager `then_some`) with its regression test, alongside this lane's
  37 math references in the same file.
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

## Dired lane — framing APPROVED; Stage 0 MERGED, Stage 1 next

- Approved framing: `docs/dired-framing.md` (revision 5), landing as its
  own docs PR off `githubsucks/main` @ `2af1ab3`, branch
  `githubsucks/dired-framing`, worktree `../pmacs-dired-framing`. The
  repo's `-framing`-branch convention (`vterm-framing`,
  `gpu-initial-target-framing`, `tab-width-parity-framing`).
- **Stage 0 (`C-x C-f` find-file) MERGED as #162** (`main` @ `2af1ab3`,
  2026-07-25, one review round, 12/12 CI green). Durable facts moved to
  `docs/agent-handoff.md` §1 per rule 3 below.
- **Stage 1 (the dired view) is next and unstarted.** Branch `dired`
  (worktree `../pmacs-dired-arc`) carries the framing commits only and is
  based on the now-superseded `0827dd1`; **rebase it onto the `main`
  resulting from the framing PR before implementing**, or cut a fresh
  branch — its framing commits become redundant once the docs PR lands.
- Stage 1's scope, from the framing §10: `builtin/runtime/dired.lua`; the
  `dired` major mode + mode keymap; buffer-per-directory with lexical
  canonicalization and the ownership check; read-only intercept +
  `set_round_trip_input`; visit routing through `window.display_file`;
  parent/sort/revert/quit; `C-x d` (with the `display` opt) / `C-x C-j`;
  cursor preservation by basename; the `dired.kill-when-opening` config
  key; **and the tolerant `read_dir` opt** — the only Rust in the stage.
- The one Rust change is load-bearing and is why Stage 1 is not
  pure-Lua: `read_dir_blocking` (`src/fs.rs:201`) fails the **entire
  listing** on any of five per-entry conditions, and the tolerant wrapper
  its own module doc delegates to package authors **cannot be written in
  Lua** — the primitive returns one error and no partial vec.
- Coherence (framing §0.5, required since #163): serves `COHERENCE.md`
  §20 Priority 1, which names this work explicitly; journey steps 7 and
  (partially) 3; **adds no interaction island** — keys are a mode-scoped
  keymap, and wdired is a mode swap; adopts `pmacs.config` for
  `dired.kill-when-opening`; inherits §9's worker-attribution gap for its
  `read_dir` jobs without worsening it.
- **Boundary with the Journey Stage 1 arc** (`COHERENCE.md` §20 arc-cut
  1): CLI directory-argument handling (`pmacs .` exits 1) belongs there,
  not here. The two meet at `resolve_target_buffer`; dired supplies the
  buffer a directory should resolve *to*, and `pmacs .` should route into
  it rather than growing a second directory surface.

## Bottom-panel lane (window placement + side windows) — Stage 1 IN REVIEW

- Portable branch: `githubsucks/bottom-panel`, worktree
  `../pmacs-bottom-panel`, based on `githubsucks/main` @ `ddaa80d`.
- Approved framing: `docs/bottom-panel-framing.md` revision 4, committed
  as the branch's first commit (`c27f75a`).
- **Stage 1 implemented; no wire change (protocol stays v20).** What
  landed on the branch:
  - `src/window.rs`: `WindowParams` (`side` / `fixed_rows` / `dedicated`
    + implementation-owned `quit_action` and `origin_document`), `Side`,
    a depth-bounded `QuitAction`, `MIN_WINDOW_OUTER_ROWS = 2`,
    `Layout::compute(area, fixed)`, the `subtree_min_rows` /
    `interactive_min_rows` recursions, `boundary_below`, and the three
    new `FrontendView` fields (`panel_capable`, `frame_geometry`,
    `panel_hidden`).
  - `src/editor_core.rs`: `primary_document_window`, the non-side target
    rule, `display_buffer` + the Q#BP3 placement policy, `quit_window`,
    `reconcile_panel_layout_core`, `resize_boundary`, per-frontend
    `JumpEntry`s, and the shared `resolve_target_buffer` seam that the
    #148 initial-target bootstrap now routes through as well.
  - `src/editor.rs`: the reconciliation transaction, geometry
    declaration, the side-window `dispatch_idle_for` gate, the divider
    paint, and the divider drag.
  - `src/lua_bindings/window_panel.rs`: the whole `pmacs.window` panel
    surface plus the shared adopter-placement helpers;
    `builtin/runtime/window.lua` owns `window.panel-height` /
    `window.min-height` and the resize commands.
  - Adopters: `listview.open`, `compile.run`, `pmacs.terminal.open` all
    take `display = "current" | "panel"` (Stage 1 default `"current"`);
    LSP/compile visits route through `display_file`.
- **Review round 1 addressed.** The load-bearing finding: the Q#BP6
  side-window split guard (`try_split_active`) had **no production
  caller** — `pmacs.window.split_horizontal` / `split_vertical`, and so
  `C-x 2` / `C-x 3`, still went through plain `split_active`. Splitting a
  focused panel made the root wrapper's final child a split rather than
  `Leaf(side)`, which both `Layout::compute`'s fixed pass and
  `document_subtree` key on. It survived the first round because the
  acceptance test called the core method **directly**; it now goes
  through the real Lua binding. This is the folding-arc round-2 lesson
  repeating exactly: *after wiring a guard into a production hook, pin it
  through the real path — a direct-call test misses the wiring.*
  Also fixed: the armed divider drag was not scoped to its arming
  frontend (it could cancel and swallow a peer's mouse events); a
  recompile carries no `display` and duplicated a panel-placed
  `*compilation*` into the document window; and
  `paint_mode_line_graphemes` had lost its doc block to an insertion.
  Five bite-verified fixes (three via `scripts/bite`, two by manual
  revert since their tests share `src/daemon.rs` with the production
  code).
- Two Stage-2 hazard pins now exist in `src/daemon.rs`, closing the gap
  the review named: a fresh attach while `LOCAL` is focused in a panel
  inherits `LOCAL`'s **document** buffer, and an initial-target bootstrap
  whose `after-load` hook creates and selects a panel still reasserts
  into a document window.
- **Review round 2 addressed.** The load-bearing finding: **Q#BP7 item 1
  — "growth reaching the live tail re-arms follow" — was never
  implemented.** `at_bottom` is the instantaneous geometric readout
  `scroll_offset == 0`, which a still-anchored view satisfies whenever it
  is momentarily tall enough to reach the tail, so the round-1 assertion
  could not see the gap: the next rows the child printed pushed the
  anchored view back into history. `src/terminal/view.rs` now has
  `rearm_follow_on_growth`, reached by one shared `declare_view_size`
  helper from every size-declaring path (`snapshot_for_view`,
  `record_view_size`, `view_status_for_size`) so grid and semantic
  declarations cannot disagree.
  Also fixed: the PTY fixtures emitted LF-only output, which staircases
  until every row clips to blanks — so the anchor assertions compared
  `""` with `""` and could not fail (now CRLF, each guarded by
  `assert!(!top_before.is_empty())`); acc33's contrast case asserted
  nothing; `start_run` let `already_in_panel` override an **explicit**
  `display = "current"`, which is the documented opt-out from the Stage 3
  flip (now gated on omission); and `window_drag` was a daemon-global
  slot that a peer's mode-line press could clear.
- Durable test lessons from this round, both the same class:
  1. **A geometric readout is not a state predicate.** `at_bottom` says
     "the viewport currently reaches the tail", not "this view follows
     the tail". Pinning follow requires feeding MORE output and asserting
     the view moved (acc32b uses a filesystem gate between two bursts).
  2. **A PTY in the default mode does not translate LF to CRLF.** An
     `echo`-driven fixture staircases rightward and clips to blanks past
     the viewport width, so any text equality over it is vacuously true.
     Emit `\r\n`, and guard text comparisons with a non-empty assertion
     the way the daemon pin guards on `!panel_hidden`.
- **Round-2 self-review caught a regression the round-2 commit
  introduced**, in the change it labelled "minor": routing
  `pmacs.window.buffer()`'s **no-argument** arm through the fid-scoped
  `selected_window` validator made it **fallible**, and
  `acting_frontend` can name a frontend with **no registered view** (a
  bare `dispatch_key` from an unattached peer does exactly that). The
  runtime calls that function on ordinary edits from `killring`,
  `syntax`, `autosave`, `pair`, `indent` and `comment` **without
  `pcall`**, so the raise never surfaced as an error — it silently
  dropped the operation. `kill_ring_acceptance` went 30/30 → 25/5
  (`frontend_detached_drops_per_frontend_state`: "B has kill state").
  The no-arg arm is back on ambient `active_buffer_id()` and documented
  as deliberately infallible; the explicit-window arm keeps its Q#BP11
  validation. New **acc19c** pins it through the real path (a
  `buffer.after-edit` subscriber during a viewless peer's `dispatch_key`)
  and bites against the regressing commit.
  Generalizes: **a "uniformity" cleanup that changes a function's
  fallibility is not minor** — check every caller's error discipline
  first, and remember that an ambient resolver's fallback IS its
  contract.
- Verification on this branch: `cargo fmt --check` clean; strict
  workspace Clippy clean; 1,817 default + 1,994 CRDT library tests;
  `bottom_panel_stage1_acceptance` 46/46; kill ring 30 default + 30 CRDT;
  vterm Stage 1 9 default + 10 CRDT; M4 121; required GPU 152;
  compile 67; vterm Stage 2 4 / Stage 3 5 (7 CRDT); folding Stage 2 48;
  statusline 7; listview 6;
  **isolated-config workspace sweep 3,130 passed across 89 suites, zero
  failures**; `git diff --check` clean.
  - **Run the sweep with an isolated `XDG_CONFIG_HOME`.** The real
    `~/.config/pmacs/init.lua` on this desktop calls
    `pmacs.packages.install_local(...)`, so every editor the sweep builds
    races on one shared install root; a losing race sets a status message
    that leaks into the mode line and breaks
    `folding_stage2_acceptance::unfolded_frame_is_identical_to_the_pre_folding_baseline`,
    which compares whole painted frames. Standalone it is 48/48. This
    generalizes the known `compile_mode_acceptance` real-config trap:
    any suite that paints the status area inherits it.
  - **A latent pre-existing `main` bug surfaced while gating and is NOT
    this branch's**: `buffer::tests::proptests::rope_matches_crdt_projection_after_arbitrary_edits`
    fails on `main` @ `352bf0b` with `ops = [Insert(0,"a"),
    Insert(0,"aaa"), Replace(0,1,"a"), Undo]` — undo of a textually-null
    `Replace` returns a no-op edit result still carrying `crdt_op =
    Some`, violating the suite's own shape invariant. `src/buffer.rs` is
    byte-identical here, and the seed was deliberately **not** committed
    (it would make an unrelated failure deterministically red on this
    PR). Needs its own lane.
  - Durable test lesson from this round: `TerminalViewStatus.scroll_offset`
    is documented as the retained rows between **this viewport** and the
    live tail, so it necessarily tracks the viewport height. Asserting it
    constant across a panel height change is either vacuous or wrong —
    the invariant Q#BP7 actually states is that the **anchor** is frozen,
    which the acceptance now pins by comparing the first visible row's
    text, plus `at_bottom` for the follow re-arm.
  - `compile_mode_acceptance` needs `--test-threads=1` locally; it is
    67/67 there. Under default parallelism it fails roughly 1 run in 3,
    with a *different* test each time (acc14/acc25a, then acc24) —
    **verified pre-existing** by swapping in `githubsucks/main`'s
    `builtin/runtime/compile.lua` and reproducing the same rate. The
    `pmacs-gpu` bin tests have historically gone red under a loaded sweep
    (wgpu device contention). Rerun isolated before treating either as a
    regression.
- Stage 2 (the GPU panel band, next available protocol version) has its
  own re-framing obligation before implementation; Stage 3 is the default
  placement flip.

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
