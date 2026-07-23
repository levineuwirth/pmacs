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
  `githubsucks/main` @ `c49a8c7` (folding Stage 1 #142 atop one-command GPU
  invocation #141 and documentation refresh #140; protocol v19).
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

The first command must expose `c49a8c7` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Folding Stage 2 framing lane (Arc 6)

Stage 1 (the headless fold engine) is MERGED as **#142** (see "Closed since
the last snapshot"); this lane carries the Stage 2 (grid/daemon collapse)
work off the resulting main.

- Portable branch: `githubsucks/folding-tui`; worktree `../pmacs-folding-tui`.
- Base: canonical `main` @ `c49a8c7` (folding Stage 1 / #142 merged).
- Framing head: `docs/folding-stage2-framing.md` (rev 1 → rev 2; rev 2
  absorbs review round 1's five findings + two rulings). Numbering continues
  the parent `Q#FD` scheme from `Q#FD12`.
- State: **framing only, rev 2, under review; no implementation, no PR.**
  Load-bearing reframe (Q#FD12): the TUI has no non-identity
  source-line↔display-row map today (`view_top + row` is baked into ~13
  sites), so Stage 2's spine is one shared visible-line-map primitive that
  every consumer routes through. Correction to the parent premise (Q#FD19):
  yank + query-replace are `apply_active_edit` callers (local), not
  Lua-mutator callers; only comment-toggle/yank-pop take the Lua path
  (shared with the remote/optimistic-CRDT apply deferred to Stage 3). No
  wire schema or protocol change — `FoldState` production (Stage 1) is
  untouched; the GPU render path is Stage 3.
- Next: land the remaining review rounds → implement Stage 2 on this branch
  → PR. Stage 3 (GPU) is a separate branch/PR off the resulting main,
  re-framed in detail after Stage 2 lands.

Recovery worktree:

```sh
git worktree add --track \
  -b folding-tui \
  ../pmacs-folding-tui \
  githubsucks/folding-tui
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
  green on rerun. Stage 2 continues on `folding-tui` (see the lane above);
  durable substrate seams live in `docs/agent-handoff.md` §1.

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
