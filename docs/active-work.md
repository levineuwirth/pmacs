# Active work — cross-machine resume ledger

**Snapshot: 2026-07-21.** This file records volatile work that has not
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
  `githubsucks/main` @ `bb17ec9` (#123 merged atop #124, protocol v17).
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

The first command must expose `bb17ec9` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Active lane: Arc 4 stage 3 — statusline segments

- Portable branch: `githubsucks/statusline-segments`
- Feature head:
  `1be7a30468f1fab73432b363dd8e44c8d2234d93`
- Pull request: #125,
  **feat(statusline): composable modeline segments at protocol v18** —
  <https://github.com/levineuwirth/pmacs/pull/125>
- State: two review rounds addressed; Revision 3's Q#SL1-Q#SL11 and
  Acceptance 1-27 remain implemented and awaiting review. The feature
  history is based on canonical `main`
  `bb17ec955e083a56aa937b596906fce84a00533a`. The branch is **not
  merged**. Review resolutions:
  <https://github.com/levineuwirth/pmacs/pull/125#issuecomment-5036372634>
  and
  <https://github.com/levineuwirth/pmacs/pull/125#issuecomment-5036628171>.
- Scope delivered: strict composable per-window `pmacs.statusline`
  providers; borrow-released evaluation and per-context failure latches;
  legacy-preserving TUI composition; a pure built-in LSP provider;
  dynamic modeline-face inventory; protocol-v18 authoritative
  `StatuslineSegments`; snapshot/version symmetry; and atomic GPU
  validation, face resolution, shaping, clipping, and caching.
- Real TUI smoke: hermetic XDG roots under `/tmp`, live tmux PTYs at
  100x24 and verified 46x24, faced left/right custom runs with CJK,
  combining text, and an injected ESC. The ESC rendered as a space;
  ordering, clipping, protected `L1:C1 All`, and the separate echo row
  remained legible without overlap. Both sessions exited 0; focused TUI
  statusline/mode-line tests passed 6 + 1.
- Final verification after review round 2, sequential:
  `cargo fmt --check`; workspace/all-target Clippy with `-D warnings`;
  1,619 default library tests (3 ignored); 1,793 CRDT library tests (3
  ignored); 7 default and 8 CRDT statusline acceptance tests; 114 M4
  acceptance tests (3 ignored, `basedpyright` filtered); 109 required
  GPU tests; the exact one-invocation workspace sweep (2,718 passed
  across 78 suites, 19 ignored, `basedpyright` filtered); and
  `git diff --check`. No flaky or environment rerun was needed.
- Recovery state at handoff: `/home/jeans/Repos/active/pmacs-statusline`
  is clean, its local branch and `githubsucks/statusline-segments` both
  point at the feature head above, PR #125 targets canonical `main`,
  and GitHub reports 22 changed files (+5,707/-302), `MERGEABLE`, with
  merge-state checks currently `UNSTABLE`.

Recovery worktree on a machine that does not already have the local
branch:

```sh
git fetch githubsucks statusline-segments
git worktree add --track \
  -b statusline-segments \
  ../pmacs-statusline \
  githubsucks/statusline-segments
gh pr view 125 --repo levineuwirth/pmacs
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
  folded into PR #123 or either feature framing branch.

## Update protocol

Whenever a listed lane changes materially:

1. update its public branch and head/state here;
2. record new verification and remove superseded caveats;
3. keep durable architecture in `docs/agent-handoff.md`, not here;
4. remove the lane after merge or abandonment;
5. verify every recovery command from a clean worktree before calling
   the transfer complete.
