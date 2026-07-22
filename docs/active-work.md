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
  `githubsucks/main` @ `2625ec7` (tab-width parity #137 merged atop
  locals-query #134 and modeline detection #132; protocol v18 on `main`).
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

The first command must expose `2625ec7` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Vterm Stage 3 implementation lane

- Portable branch: `githubsucks/vterm-gpu`
- Framing carried as the first commit; implementation follows it.
- Base: cut from canonical `main` @ `1dd47fc`, NOT stacked on
  `vterm-stage3-framing`, per the framing's §8. Canonical `main` @
  `2625ec7` (tab-width parity #137, locals-query #134, modeline handoff
  #133/#136) is MERGED IN — see the integration entry below.
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
- Review round 1 addressed (framing §0.10): hover no longer claims durable
  terminal control (real defect, bite-verified); terminal motion dedupes by
  cell; declarations record only once sent; unchanged frames skip
  revalidation; the terminal-mode presence-sweep skip is removed. The
  review's predicted presence FREEZE did not reproduce — the buffer-follow
  clears the declaration before `render_frame`, so a truthful sweep always
  precedes terminal mode; that test is a labelled regression guard, not
  fix evidence.
- Review round 2 addressed (framing §0.11): a daemon disconnect now leaves
  terminal mode so the notice is visible (real defect, hand-verified
  because fix and test share a file); the per-tick full-grid clone is
  gone; inbound terminal events require a negotiated v19 session; a
  grid-missing press no longer arms a drag; roadmap/handoff Arc 5 lines
  corrected. Named deferral: terminal wheel gestures discard scroll
  magnitude.
- Post-round-2 gates (pre-integration): 1,758 default + 1,934 CRDT library
  tests; required GPU 129; workspace sweep 2,923 across 83 suites; Stage 3
  acceptance 5 default / 7 CRDT; M4 120; fmt, clippy, diff check clean.
- **Post-integration gates (canonical `main` @ `2625ec7` merged in):**
  `cargo fmt --check`; strict workspace Clippy; `pmacs-protocol` 17;
  `cargo test --lib` 1,768; `--features crdt` 1,944 (3 ignored each);
  vterm Stage 1 9/10, Stage 2 4/4, Stage 3 5/7, statusline 7/8, tab-width
  2/2 (default/CRDT); M4 121 passed (3 ignored, 1 filtered); required GPU
  139; workspace sweep 2,946 passed across 84 suites (19 ignored), one
  invocation; `git diff --check` clean.
- Closed caveat: the once-seen required-GPU failure did not reproduce in
  eight author runs plus five reviewer runs. Treated as environmental.
- Canonical-main integration after #137 landed: the agreed order was
  #137 first (it was approved and FROZEN at `5b23e11`, so it could not
  absorb a rebase without breaking its freeze), then this lane second.
  Integrated by MERGING canonical main into the branch, matching repo
  precedent (`Merge canonical main into vterm-tui`, `… into modeline
  detection`) rather than a rebase, which would have force-pushed away
  the review anchors on #135. Main had also moved past this lane's base
  by #133/#134/#136, so the integration surface was wider than the
  #135/#137 overlap: `src/semantic_render.rs` was a fourth overlapping
  code file and auto-merged, as did `pmacs-protocol/src/lib.rs`. The one
  code conflict was the `pmacs_protocol` import list in
  `pmacs-gpu/src/main.rs` (`TAB_STOP_COLUMNS` against the terminal
  types) — resolved as a union.
- Next: further user review rounds on the PR.

Recovery worktree:

```sh
git worktree add --track \
  -b vterm-gpu \
  ../pmacs-vterm-gpu \
  githubsucks/vterm-gpu
```


## Cross-PR coordination: #135 and #137 (resolved)

**Resolved 2026-07-22: #137 merged first, #135 integrated second.** Kept
as the worked example, because the deciding argument is reusable.

#137 was APPROVED and FROZEN at `5b23e11`. "Frozen" and "rebase onto the
resulting main" are mutually exclusive, so the frozen PR had to land
first — the alternative would have broken its freeze and voided its
approval. Three arguments pointed the same way: the approved PR should
not wait on an unapproved one; the PR carrying the protocol byte pins
should be the one that integrates, because its own suite is what detects
a disturbed discriminant; and the larger, more invasive change should
pay the integration cost, since its author has the context to verify the
merged result.

The named overlap (`pmacs-gpu/src/main.rs`, `pmacs-protocol/src/lib.rs`,
`Cargo.lock`, both ledger docs) was accurate but incomplete — main had
also moved by #133/#134/#136, making `src/semantic_render.rs` a fourth
overlapping code file. **Lesson: derive the integration surface from
`git diff <base>..main`, not from the other PR's file list.**

The feared semantic collision did not occur. Terminal cell geometry
still uses the monospace advance and is not routed through
`TAB_STOP_COLUMNS`: terminal columns come from the child, and tab
expansion is a DOCUMENT projection concern. That is verified rather than
assumed — see the integration gates on the lane above.

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
