# Active work — cross-machine resume ledger

**Snapshot: 2026-07-20.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

## Repository authority

- Canonical development remote: `githubsucks`
  (`https://github.com/levineuwirth/pmacs.git`).
- Canonical base at this snapshot:
  `githubsucks/main` @ `f8096ff` (#124 merged, protocol v17).
- `origin/main` @ `d3fa632` is the release mirror and was 400 commits
  behind `githubsucks/main` at the snapshot. Do not base new work on it.
- The shared desktop checkout contained unrelated uncommitted work. The
  branches below were prepared in isolated worktrees; never clean or
  overwrite the shared checkout to recover them.

Start on another machine with:

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
- Checkpoint head: `a8b7195`
- The checkpoint is a continuation branch for recovery, not a second
  feature and not a merge target. Finish there, then push its completed
  head to `githubsucks/json-yaml-grammar`.

The checkpoint carries the unpushed review-fix set that previously lived
only in `../pmacs-jsonyaml`:

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

Verification already completed before transfer:

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

Still required on the destination machine:

1. Add the PATH-gated real-YAML pmacs acceptance test, mirroring the
   JSON test. Disable SchemaStore and Kubernetes CRD catalog access in
   the test for deterministic, network-free operation.
2. Run it against the exact Red Hat provider and require:
   auto-attach, initialized state, a non-empty diagnostic, and no crash.
3. Update `docs/json-yaml-framing.md` if the live pmacs path reveals any
   difference from the standalone evidence.
4. Rebase the checkpoint onto current `githubsucks/main`.
5. Run the full repository gate suite from `AGENTS.md`.
6. Push the completed head to `githubsucks/json-yaml-grammar`; confirm
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
