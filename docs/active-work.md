# Active work — cross-machine resume ledger

**Snapshot: 2026-07-22.** This file records volatile work that has not
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
  `githubsucks/main` @ `1dd47fc` (modeline detection #132 merged atop Vterm
  Stage 2 #130; protocol v18).
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

The first command must expose `1dd47fc` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Vterm Stage 3 framing lane

- Portable branch: `githubsucks/vterm-stage3-framing`
- Framing contract commit: `d7bb831`; Revision 8 review fixes: `c72dfea`;
  both follow canonical-main integration.
- Base: canonical `main` @ `1dd47fc` (modeline detection #132 atop Vterm
  Stage 2 #130), protocol v18.
- PR: none. This is framing only; no Stage 3 implementation branch exists.
- State: `docs/vterm-framing.md` Revision 8 maps criteria 28–37, has passed
  one external review, and awaits explicit user approval. It locks additive
  protocol v19 `TerminalFrame`, `TerminalResize`, and `TerminalPointer`; an
  8 MiB aggregate glyph-byte bound under the unchanged 16 MiB transport cap;
  dual viewport declaration for the first terminal frame; authenticated
  per-view semantic routing; and a fixed-cell native GPU renderer/input/cache
  contract.
- Stage 2 is landed as PR #130 at merge `86fc1bc`. Stage 3 starts from that
  integrated substrate and does not reopen its TUI/Lua/controller contracts.
- Review: no architectural defect. `c72dfea` makes the measured-size fixture
  maximize style and cluster-prefix overhead, names the GPU clipboard signal
  path without implying child OSC 52 support, and aligns Arc 5/internal-stage
  naming.
- Verification: documentation-only `git diff --check`; framing consistency
  search. No runtime gates apply before implementation.
- Next: explicit user approval. After approval, create `vterm-gpu` from the
  then-current canonical main and implement criteria 28–37; do not stack the
  feature on this documentation branch.

Recovery worktree on a machine that does not already own the branch:

```sh
git worktree add --track \
  -b vterm-stage3-framing \
  ../pmacs-vterm-stage3-framing \
  githubsucks/vterm-stage3-framing
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
