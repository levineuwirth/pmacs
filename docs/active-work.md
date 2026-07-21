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
  `githubsucks/main` @ `7bc0c61` (#125 merged; protocol v18).
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

The first command must expose `7bc0c61` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.


## Active lane: Vterm Stage 1 terminal core

- Portable branch: `githubsucks/vterm-core`
- Implementation commits: `bbc1f33` (Stage 1), `962944b` (Darwin signal),
  and review fixes `f0a235f`, `28f2e6c`, `bf972a7`; current branch head:
  `5d50558`.
- Pull request: #126,
  <https://github.com/levineuwirth/pmacs/pull/126> — open, non-draft, targeting
  canonical `main`; not merged.
- State: Arc 5 Stage 2's first of three PRs is implemented, review round 1 is
  addressed, and the branch is fully gated against protocol-v18 `main`. It is
  the headless terminal core only; the Stage 2 TUI/Lua surface and Stage 3
  protocol/GPU renderer are not implemented and must start only after their
  preceding PR lands.
- Framing: `docs/vterm-framing.md` Revision 4 maps Stage 1 Acceptance 1–14,
  records review round 1, and preserves the downstream TUI/GPU contracts.
- Final from-start sequence after review round 1: Clippy clean; 1,660 default +
  1,836 CRDT library tests (3 ignored each); 8 default + 9 CRDT vterm
  acceptance; M4 114 passed (3 ignored, 1 filtered); required GPU 109;
  workspace 2,767 passed across 79 suites (19 ignored, 1 filtered); diff check
  clean. The first review Clippy pass found only identical LF/IND match arms;
  after consolidation, the complete sequence restarted from gate 1.
- `scripts/bite main src/lib.rs --test vterm_stage1_acceptance` returned
  `bite: OK` because the old crate root could not compile the new terminal API;
  this is explicitly weaker compile-time API evidence, not a clean behavioral
  assertion failure.
- `scripts/bite HEAD^ src/ansi.rs --lib
  parser_split_points_produce_identical_screen` returned `bite: OK` with a
  clean behavioral cursor-position failure against the pre-dispatch parser.

Recovery worktree, only if no existing worktree owns the branch:

```sh
git worktree add --track \
  -b vterm-core \
  ../pmacs-vterm-core \
  githubsucks/vterm-core
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
