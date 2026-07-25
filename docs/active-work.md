# Active work — cross-machine resume ledger

**Snapshot: 2026-07-25.** This file records volatile work that has not
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
  `githubsucks/main` @ `ccf29e3` (the CRDT undo repro #157 atop the
  inline-math landed-doc refresh #172, the bottom-panel landed-doc
  refresh #156, the inline-math slice #158, dired Stage 1 #165, the GPU
  terminal input fix #166, Lean 4 Stage 2 #161, the dired framing #164,
  COHERENCE.md #163, find-file #162, Lean 4 Stage 1 #160, and the minimap
  blank-slab fix #159; protocol v20). **Lanes below that name an older
  base have not been re-based; derive their integration surface from
  `git diff <their base>..main`.**
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
