# Active work — cross-machine resume ledger

**Snapshot: 2026-07-20.** This file records volatile work that has not
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
  `githubsucks/main` @ `f8096ff` (#124 merged, protocol v17).
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

The first command must expose `f8096ff` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Active lane: PR #123 — JSON + YAML

- PR: <https://github.com/levineuwirth/pmacs/pull/123>
- Public PR branch: `githubsucks/json-yaml-grammar`
- Public PR head before the transfer checkpoint: `4be2a65`
- Original merge base: `56eb67e`; current main is nine commits ahead.
- Portable checkpoint branch:
  `githubsucks/json-yaml-handoff-2026-07-20`
- Checkpoint head: `5c202c5`
- The checkpoint is a continuation branch for recovery, not a second
  feature and not a merge target. Finish there, then push its completed
  head to `githubsucks/json-yaml-grammar`.

The checkpoint carries the transferred review-fix set plus the completed
destination-machine continuation:

- push configured settings via
  `workspace/didChangeConfiguration` immediately after `initialized`;
- fake-LSP configuration sink and a deterministic delivery test;
- explicit `json.validate.enable = true`;
- JSON provider pin
  `@t1ckbase/vscode-langservers-extracted@2.0.2`;
- corrected YAML configuration sections:
  `yaml`, `http`, `[yaml]`, `editor`, `files`;
- corrected telemetry, schema-network, and schema-association claims;
- PATH-gated real JSON provider integration test.
- PATH-gated real YAML 1.24.0 integration test with SchemaStore and the
  Kubernetes CRD catalog disabled for network-free determinism. It pins
  YAML-specific auto-attach/initialization, a real syntax diagnostic,
  no crash, and continued initialized state after the diagnostic.

Verification completed across the source and destination machines:

- JSON provider standalone protocol smoke passed.
- JSON provider through pmacs passed
  `m4_real_json_provider_receives_config_and_reports_diagnostics`.
- `m4_json_yaml_lsp_configs_pin_command_and_sections` passed.
- `m4_5_initial_config_pushed_via_did_change_configuration` passed.
- `cargo fmt --check` and `git diff --check` passed.
- `yaml-language-server@1.24.0` standalone protocol smoke passed:
  initialization reported version 1.24.0; the initial
  `workspace/configuration` request was exactly
  `yaml`, `http`, `[yaml]`, `editor`, `files`; opening the document
  caused a second scoped `[yaml]` request; invalid YAML produced a real
  parser diagnostic; shutdown exited 0 with empty stderr.
- `m4_real_yaml_provider_pulls_config_and_reports_diagnostics` passed
  against `/tmp/pmacs-yamlls` on the destination machine (0.36s).
- Bite verification passed: with `builtin/runtime/lsp.lua` swapped to
  pre-JSON/YAML `56eb67e`, the real YAML test failed at the absent YAML
  config rather than skipping.
- The four-commit checkpoint rebased cleanly onto canonical
  `githubsucks/main` @ `f8096ff`; the rewritten portable head was pushed
  with an exact force-with-lease.
- The first post-rebase Clippy pass found one `doc_markdown` warning in
  the new YAML test comment. The backtick-only correction was amended
  into the test commit and pushed at the checkpoint head above; the gate
  is being rerun.

Still required:

1. Run the full repository gate suite from `AGENTS.md`, putting both
   pinned temporary provider prefixes on PATH so neither live smoke can
   skip.
2. Push the completed head to `githubsucks/json-yaml-grammar`; confirm
   new PR checks belong to that head. Never merge without the user's
   instruction.

Provider setup is intentionally machine-local:

```sh
npm install --prefix /tmp/pmacs-jsonls \
  @t1ckbase/vscode-langservers-extracted@2.0.2
npm install --prefix /tmp/pmacs-yamlls \
  yaml-language-server@1.24.0
```

The Node language servers had to run outside the prior machine's
restrictive execution sandbox. The `/tmp` prefixes and smoke harnesses
do not travel.

Recovery worktree:

```sh
git worktree add --track \
  -b json-yaml-handoff-2026-07-20 \
  ../pmacs-jsonyaml-home \
  githubsucks/json-yaml-handoff-2026-07-20
```

If that local branch name already exists, omit `--track -b ...` and
give the existing local branch as the final argument.

## Active lane: Arc 4 stage 3 — statusline segments

- Portable branch: `githubsucks/statusline-segments`
- Framing head: `432e4d4`
- State: framing only, revision 1, based on `f8096ff` / protocol v17.
- Status: awaiting user review. No implementation and no PR.
- Scope: composable per-window Lua modeline providers, dynamic
  modeline-face inventory, and protocol v18
  `StatuslineSegments`; the built-in LSP segment is the first consumer.
- Do not implement until the user approves the framing. When approved,
  continue on this branch so the framing remains the first commit.

Recovery worktree:

```sh
git worktree add --track \
  -b statusline-segments \
  ../pmacs-statusline \
  githubsucks/statusline-segments
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
