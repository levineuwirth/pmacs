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

## Vterm Stage 3 implementation lane

- Portable branch: `githubsucks/vterm-gpu`
- Framing carried as the first commit; implementation follows it.
- Base: canonical `main` @ `1dd47fc` (modeline detection #132 atop Vterm
  Stage 2 #130). Cut from `main`, NOT stacked on `vterm-stage3-framing`,
  per the framing's §8.
- PR: #135, <https://github.com/levineuwirth/pmacs/pull/135>, open against
  canonical `main`. Never merge without explicit authorization.
- State: criteria 28-37 implemented. Protocol v19 (`SUPPORTED=[6..=19]`):
  `InstanceMessage::TerminalFrame` (discriminant 26, daemon-gated),
  `FrontendEvent::TerminalResize` (11) and `TerminalPointer` (12)
  (frontend-gated). `pmacs-protocol/src/terminal.rs` owns the shared
  bounds and `TerminalFrame::validate`; `pmacs-gpu/src/terminal.rs` is the
  pure cell-space paint planner; `pmacs-gpu --headless-probe` drives the
  real attach client without winit for criterion 37.
- Verification (clean tree, this machine):
  - `cargo fmt --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo test --lib`: 1,757 passed (3 ignored);
  - `cargo test --lib --features crdt`: 1,933 passed (3 ignored);
  - vterm Stage 1 acceptance: 9 default / 10 CRDT;
  - vterm Stage 2 acceptance: 4 default / 4 CRDT;
  - vterm Stage 3 acceptance: 4 default / 5 CRDT (the CRDT-only case is
    the real-daemon + real-PTY + headless-GPU path);
  - statusline acceptance: 7 default / 8 CRDT;
  - `cargo test --test m4_acceptance -- --skip basedpyright`: 120 passed
    (3 ignored, 1 filtered);
  - `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`: 127 passed;
  - `cargo test --workspace -- --skip basedpyright`: 2,919 passed across
    83 suites (19 ignored), one invocation;
  - `git diff --check`.
- Open caveat: the required-GPU suite failed ONCE mid-session and did not
  reproduce across eight subsequent runs or the full sweep. The failing
  test's identity was not captured. Re-run `PMACS_REQUIRE_GPU=1 cargo test
  -p pmacs-gpu` a few times on review; if it recurs, capture the name.
- Next: user review rounds on the PR.

Recovery worktree:

```sh
git worktree add --track \
  -b vterm-gpu \
  ../pmacs-vterm-gpu \
  githubsucks/vterm-gpu
```

## Vterm Stage 3 framing lane (superseded)

- Portable branch: `githubsucks/vterm-stage3-framing`
- Revision 8 framing, reviewed and approved. Its content is carried on
  `vterm-gpu`; this branch is kept only as the approval record and has no
  unmerged runtime work.

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
