# Active work — cross-machine resume ledger

**Snapshot: 2026-07-23.** This file records volatile work that has not
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
  `githubsucks/main` @ `47581f4` (web grammars #146 atop folding Stage 1
  #142, inline-math framing #145, and one-command GPU invocation #141;
  protocol v19).
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

The `git log` command must expose `47581f4` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Active lane: GPU initial target

- Portable implementation branch: `githubsucks/gpu-initial-target` @
  `bef1c08` (PR review fixes plus complete post-review verification); worktree
  `../pmacs-gpu-initial-target`.
- Approved framing branch: `githubsucks/gpu-initial-target-framing`;
  Revision 2 checkpoint `71039d1`.
- Original implementation base: canonical `githubsucks/main` @ `c49a8c7`
  (folding Stage 1 #142); current canonical `main` @ `47581f4` is integrated
  conflict-free by merge `d6d4be6`. Protocol was v19 before this work.
- State: implementation checkpoint `2dd30ec`; review-fix checkpoint `bef1c08`.
  Integrated, smoke-tested, fully gated, and published on 2026-07-23 at
  protocol v20. PR #148 remains open for user review:
  `https://github.com/levineuwirth/pmacs/pull/148`.
- Scope delivered: one session-scoped `pmacs --gpu [--socket …] FILE` target,
  protocol-v20 semantic bootstrap, launcher-owned tilde/cwd resolution, exact
  Unix path transport, pre-window target readiness, replica coherence, and the
  approved behavioral acceptance matrix.
- Post-review verification: formatting and strict Clippy; 1,800 default + 1,977
  CRDT library tests; target gate 1 default + 13 CRDT; M4 121; required GPU 152;
  Vterm Stage 3 5 default + 7 CRDT; isolated-config workspace sweep 3,269 across
  87 suites. Two concurrent real Wayland/Vulkan GPU windows remained on distinct
  target buffers after the second attach.
- Deferred unchanged: automatic GUI selection, multiple files, general
  live-open commands, packaging, and remote GPU paths.

Recovery worktree after the first push:

```sh
git worktree add --track \
  -b gpu-initial-target \
  ../pmacs-gpu-initial-target \
  githubsucks/gpu-initial-target
```

## Folding framing lane (Arc 6)

- Portable branch: `githubsucks/folding`; worktree `../pmacs-folding`.
- Base: **rebased onto canonical `main` @ `96d0bae`** at implementation
  start (was `cac4961`; the earlier base fell behind the docs + tab-width
  housekeeping).
- Framing head: revision 5 of `docs/folding-framing.md` (rev 1 → … → rev 4
  absorbed three review rounds; rev 5 records approval + the Q#FD4 binding
  decision).
- State: **Stage 1 (fold engine, headless) implemented; PR #142 OPEN**,
  two review rounds landed on the branch. Bindings decided (Q#FD4 → Emacs
  hideshow `C-c @` set); Bet B1 accepted as framed.
  Load-bearing decision (Q#FD1): the bundled grammars ship no fold query and
  no `folds.scm`, so the roadmap's "tree-sitter fold ranges" is not free; v1
  is structural node folding (block-like node ≥2 source lines, derived head
  line, closer-aware tail), with indentation fallback and curated queries
  deferred. `FoldState` already exists in the protocol; Stage 1 starts
  *producing* it (authoritative-empty), no protocol bump; gutter markers are
  frontend-derived like the diagnostic sign bars, so no new wire type. Staged
  like vterm: Stage 1 engine (headless), Stage 2 TUI, Stage 3 GPU.
- PR: **#142** (`Arc 6 folding — Stage 1: instance fold engine`), open
  against `main`. Round 1 (tail-boundary delete bug + buffer-kill cleanup +
  close-all point-move) and round 2 (pin the kill-path + close-all through
  the real command surface) are landed as fix commits on the branch.
- Next: land Stage 1; Stages 2/3 are separate branches/PRs, each re-framed
  in detail after the prior stage lands.

Recovery worktree:

```sh
git worktree add --track \
  -b folding \
  ../pmacs-folding \
  githubsucks/folding
```

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
