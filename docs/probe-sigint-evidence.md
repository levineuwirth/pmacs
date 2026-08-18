# GPU probe SIGINT lane — run manifest

Every physical run behind
`docs/gpu-probe-sigint-framing.md` §4. Pushed so the evidence travels;
the log bodies stay machine-local under
`/home/jeans/build/pmacs-gate-targets/probe-sigint-evidence/` and are
identified here by SHA-256 prefix and byte count.

## Provenance honesty

These runs were made **before** this manifest existed, so their
provenance is **reconstructed, not captured**. Specifically:

- **Commands** are exact and complete argv. Revision 3 abbreviated
  R7–R9 as "`--test` ×N (targets 6–37)" and F5 as "`--acceptance` ×6",
  which are descriptions, not reconstructable invocations. They are
  written out in full below.
- **Worktree** is exact.
- **HEAD** is given as a range where the run cannot be pinned to one
  commit, and marked `~`. It is never guessed at single-commit
  precision.
- **Cleanliness** was not recorded at the time and is therefore
  `UNKNOWN` for every pre-manifest run. It is not inferred.

**D0 (below) exists because of this.** No conclusion in §4 should rest
on a `UNKNOWN`-cleanliness row once D0 has replaced it.

## Artifact identity — suffix is NOT byte identity

Two distinct things were conflated in revision 3 and are separated here.

**Cargo suffix** is recorded in each log and is therefore *known* per
run. **Byte identity** is not: target directories have been overwritten
many times since, so a hash computed today is the hash of whatever
occupies that path now, not of what a given run executed.

That the two differ is demonstrated, not assumed. F1 ran in the `main`
worktree (`pmacs-fdccc423`) and executed suffixes `-5d9105cb7047aab8`
and `-d4dae4f01bcdef62` — **the same suffixes** as the panel-worktree
sweeps — yet the bytes at those paths differ by worktree:

| worktree | `gpu_initial_target…-5d9105cb` | `gpu_invocation…-d4dae4f0` |
|---|---|---|
| `pmacs-fdccc423` (main) | `e057803988c34cf7` | `00f06aeb089ce38d` |
| `pmacs-mapping-gen-…` (§5b) | `1b3cc86cbb8d6092` | `ede0c07dd9abb456` |

Same suffix, different source head, different bytes. So **"workspace
artifact family" was not an identity class** and is withdrawn as a
grouping. Each run below records the suffix its log shows, and byte
identity as `UNKNOWN` unless contemporaneously captured — which, for
every pre-manifest run, it was not. R1 and R2 have no preserved log at
all and so cannot claim even a suffix.

## Runs

All in worktree `pmacs-mapping-gen` unless stated. `WT=mg` is
`/home/jeans/Repos/personal/pmacs-mapping-gen`; `WT=main` is
`/home/jeans/Repos/personal/pmacs` at `72da24a`. All carry
`CARGO_TARGET_DIR=/home/jeans/build/pmacs-gate-targets/pmacs-mapping-gen-8cb089c8`
except `WT=main`, which uses `…/pmacs-fdccc423`.

| id | exact command (after `cargo`) | WT | HEAD | clean | artifacts | result | log (sha256/16, bytes) |
|---|---|---|---|---|---|---|---|
| R1 | `test --features crdt --test gpu_invocation_acceptance ctrl_c_on_launcher_group` ×3 | mg | ~`724b785`–`5174f73` | UNKNOWN | reduction | green, 0.15–0.17 s | **none preserved** |
| R2 | `test --features crdt --test gpu_invocation_acceptance` | mg | ~`724b785`–`5174f73` | UNKNOWN | reduction | green, 15 passed | **none preserved** |
| R3 | `test --workspace --features crdt --no-fail-fast -- --skip basedpyright ctrl_c_on_launcher_group` | mg | ~`724b785`–`5174f73` | UNKNOWN | workspace | green | `e09a96512035284e` 33113 |
| R4 | `test --features crdt --lib --test gpu_invocation_acceptance --no-fail-fast` | mg | ~`724b785`–`5174f73` | UNKNOWN | reduction | green, 2145 + 15 | `89050c702de22d57` 158812 |
| R5 | `test --features crdt --no-fail-fast --test gate_script_acceptance --test gpu_invocation_acceptance` | mg | ~`5174f73`–`b72843a` | UNKNOWN | reduction | green | `31b3e5249b475479` 3706 |
| R6 | `test --features crdt --no-fail-fast --test gpu_font_acceptance --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | mg | ~`5174f73`–`b72843a` | UNKNOWN | reduction | green, 11+15+15 | `332693a39c73731a` 4569 |
| R7 | `test --features crdt --no-fail-fast --lib --bins --test acceptance --test ambient_isolation_acceptance --test auto_indent_acceptance --test auto_indent_crdt_acceptance --test auto_pair_acceptance --test auto_pair_crdt_acceptance --test autosave_acceptance --test bottom_panel_stage1_acceptance --test bottom_panel_stage2a_acceptance --test bottom_panel_stage2b_daemon_acceptance --test bottom_panel_stage2b_gpu_acceptance --test bottom_panel_stage2b_protocol_acceptance --test comment_toggle_acceptance --test compile_mode_acceptance --test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | `-91f51d0b` / `-6b4b8223`; bytes UNKNOWN | green | `9e1ebc59ed9f0dd4` 187531 |
| R8 | `test --features crdt --no-fail-fast --test compile_mode_crdt_acceptance --test completion_popup_acceptance --test config_registry_acceptance --test cua_region_acceptance --test desktop_acceptance --test destination_capture_acceptance --test dired_acceptance --test discovery_acceptance --test discovery_stage2_acceptance --test editops_acceptance --test find_file_acceptance --test folding_acceptance --test folding_stage2_acceptance --test full_grid_resync_acceptance --test gate_script_acceptance --test git_status_stage1_acceptance --test gpu_font_acceptance --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | suffixes per log; bytes UNKNOWN | green | `8b26ebfcf5f871b4` 28677 |
| R9 | R7's argv with R8's eighteen `--test` names spliced in before `--test gpu_invocation_acceptance` — i.e. `test --features crdt --no-fail-fast --lib --bins` then `--test` for each of targets 6–37 in sweep order, then `--test gpu_invocation_acceptance`. Full list = R7's ∪ R8's, deduplicated, order preserved. | mg | ~`b72843a` | UNKNOWN | **`-91f51d0b` / `-6b4b8223`** (log `:3066`); bytes UNKNOWN | green | `b31d98ee2f427eca` 214566 |
| R10 | `test --workspace --features crdt --no-fail-fast --test gpu_initial_target_acceptance --test gpu_invocation_acceptance -- --skip basedpyright` | mg | ~`b72843a` | UNKNOWN | **`-5d9105cb`** (log `:3`); bytes UNKNOWN | green | `81b48fd7a0e261dc` 3553 |
| F1 | `build --workspace --no-default-features --features luajit,crdt && test --workspace --features crdt --no-fail-fast -- --skip basedpyright` | **main** | `72da24a` | clean (verified `git status --porcelain` empty) | `-5d9105cb` / `-d4dae4f0`; bytes at those paths **today** `e0578039…` / `00f06aeb…`, i.e. NOT the panel worktree's | **red** | `10b55b8ba8741125` 334446 |
| F2 | `test --workspace --features crdt --no-fail-fast -- --skip basedpyright` | mg | ~`b72843a` | UNKNOWN | `-5d9105cb` / `-d4dae4f0`; bytes UNKNOWN | **red** | `474f88f0dad581fe` 338555 |
| F3 | same argv as F2, with a resource sampler running | mg | ~`b72843a` | UNKNOWN | as F2; bytes UNKNOWN | **red** | `7b8519e7300e8bb3` 338555 |
| F4 | same argv as F2, with a process-table sampler running | mg | ~`b72843a` | UNKNOWN | as F2; bytes UNKNOWN | **red** | `5ccdefc5d89eece3` 338555 |
| F5 | stage 15 of `./scripts/gate --protocol --acceptance bottom_panel_stage1_acceptance --acceptance bottom_panel_stage2a_acceptance --acceptance bottom_panel_stage2b_daemon_acceptance --acceptance bottom_panel_stage2b_gpu_acceptance --acceptance bottom_panel_stage2b_protocol_acceptance --acceptance gui_stage1a_wire_acceptance` | mg | `5174f73` + uncommitted docs | UNKNOWN | suffixes per log; bytes UNKNOWN | **red** (3 bins: both ctrl_c copies + `m6_1_pty_mode_lifecycle`) | `20260817T172537Z-2375685/15-sweep-crdt.log`, sha `e5bdc911e366` |
| F6 | stage 9 of `./scripts/gate --protocol` | mg | `5174f73` | UNKNOWN | suffixes per log; bytes UNKNOWN | **red** (2 bins, both ctrl_c copies) | `20260816T063330Z-1977433/09-sweep-crdt.log`, sha `7a75d999ac4f` |
| F7 | stage 9 of `./scripts/gate --protocol` | mg | `5174f73` | UNKNOWN | suffixes per log; bytes UNKNOWN | **red** (2 bins, both ctrl_c copies) | `20260816T064549Z-2144707/09-sweep-crdt.log`, sha `9d3c6ad1bfc9` |

Supporting, not a reduction: `9b8a01076b44bb7c` 98838
(`proc-sample.log`) is the process-table sampler output behind the
retracted "mechanism located" claim.

**The red count is 7, not 5.** Revision 3 said "F1–F5, red 5/5" while
the framing separately cited gate `…-2144707`, a *different* physical
run the manifest never listed. Both exist, both are red, and there is a
third gate run too. F1–F7 above enumerate all seven, each with its own
log digest. F5 also has an extra failing binary
(`m6_1_pty_mode_lifecycle_started_then_exited`) that F6/F7 do not.

**R2 and R6 are distinct runs.** Revision 2's §4 cited `gpu3.log` for
both; that log is R6's three-suite run only, and R2's log was never
preserved. R1 likewise has no log. Both are marked accordingly rather
than backfilled.

## The onset is datable — and this supersedes the reduction matrix

`sweep-crdt` has run 17 times in this target directory's gate logs. The
`ctrl_c` failure appears in **exactly the last three**, and the test
**passed inside `sweep-crdt`** — both copies, `... ok` — in the runs
before them.

| date | gate run | sweep-crdt | ctrl_c |
|---|---|---|---|
| 08-14 → 08-15 19:57 | 14 runs | 11 green; 3 red on *other* tests | **passes** where the stage ran it |
| 08-16 06:33 | `…-1977433` | red, 2 bins | **fails, both copies** |
| 08-16 06:45 | `…-2144707` | red, 2 bins | **fails, both copies** |
| 08-17 17:25 | `…-2375685` | red, 3 bins | **fails, both copies** |

The three earlier red sweeps failed on unrelated tests — protocol and
version rows on 08-15 09:55, and `composition_overhead_under_ten_percent`
plus a v21/v20 row on 08-15 18:37. **None involved `ctrl_c`.**

So the failure is **not long-standing**. Last green containing it:
`20260815T185708Z`. First red: `20260816T063330Z`. Something changed in
that window, on a machine that was not rebooted (uptime spans it).
**Bisecting that boundary is a sharper lead than any reduction**, and
D0 is amended to do it first.

## D0 — re-run the matrix with captured provenance

Before any §4 row is relied on for a conclusion, re-run the reductions
under a harness that records, per run and at the time: exact argv and
environment, worktree, `git rev-parse HEAD`, `git status --porcelain`
emptiness, the artifact hashes actually executed, the result, and the
log digest. Two constraints learned the hard way:

- run them **at `main`**, not on a feature branch — R1–R10 ran in the
  `panel-mapping-generation` worktree, which carries §5b changes;
- record the **artifact hash per run at run time**, since command shape
  changes it — the reason R9's result did not mean what it appeared to
  mean — and since a hash computed later reflects only what occupies
  that path now;
- **first, bisect the 08-15 19:57 → 08-16 06:33 onset window.** A test
  that passed in this stage fourteen times and then failed three times
  in a row has a change behind it, and finding that change is worth
  more than any further reduction.
