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

Same suffix, different source head, different bytes **today**. So
**"workspace artifact family" was not an identity class** and is
withdrawn as a grouping.

Three levels of knowledge, kept apart:

0. **Portability caveat.** Suffixes below are read from logs that are
   **machine-local**; this manifest is the portable record of them, and
   a reader elsewhere is trusting this transcription, not verifying it.
   Rows that were never logged say `UNKNOWN` and are not guessed.
1. **Suffix — known**, from each log. A differing suffix means Cargo
   computed a different metadata hash, i.e. it treated the two as
   distinct compilations.
2. **Today's bytes at a path — known**, and shown above.
3. **The bytes a historical run executed — UNKNOWN.** Target
   directories have been overwritten repeatedly since; a hash computed
   now is the hash of the current occupant.

So statements of the form "R9 and the sweeps ran byte-different
binaries" are **withdrawn everywhere**. What is established is that
they ran **different Cargo compilations** (different suffixes), which
is enough to void the comparison and is all that is claimed. R1 and R2
have no preserved log and cannot claim even a suffix.

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
| R7 | `test --features crdt --no-fail-fast --lib --bins --test acceptance --test ambient_isolation_acceptance --test auto_indent_acceptance --test auto_indent_crdt_acceptance --test auto_pair_acceptance --test auto_pair_crdt_acceptance --test autosave_acceptance --test bottom_panel_stage1_acceptance --test bottom_panel_stage2a_acceptance --test bottom_panel_stage2b_daemon_acceptance --test bottom_panel_stage2b_gpu_acceptance --test bottom_panel_stage2b_protocol_acceptance --test comment_toggle_acceptance --test compile_mode_acceptance --test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | `gpu_invocation…-6b4b8223` **only** — R7 does not select `gpu_initial_target` | green | `9e1ebc59ed9f0dd4` 187531 |
| R8 | `test --features crdt --no-fail-fast --test compile_mode_crdt_acceptance --test completion_popup_acceptance --test config_registry_acceptance --test cua_region_acceptance --test desktop_acceptance --test destination_capture_acceptance --test dired_acceptance --test discovery_acceptance --test discovery_stage2_acceptance --test editops_acceptance --test find_file_acceptance --test folding_acceptance --test folding_stage2_acceptance --test full_grid_resync_acceptance --test gate_script_acceptance --test git_status_stage1_acceptance --test gpu_font_acceptance --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | `-91f51d0b` (`half2.log:1`) / `-6b4b8223` | green | `8b26ebfcf5f871b4` 28677 |
| R9 | `test --features crdt --no-fail-fast --lib --bins --test acceptance --test ambient_isolation_acceptance --test auto_indent_acceptance --test auto_indent_crdt_acceptance --test auto_pair_acceptance --test auto_pair_crdt_acceptance --test autosave_acceptance --test bottom_panel_stage1_acceptance --test bottom_panel_stage2a_acceptance --test bottom_panel_stage2b_daemon_acceptance --test bottom_panel_stage2b_gpu_acceptance --test bottom_panel_stage2b_protocol_acceptance --test comment_toggle_acceptance --test compile_mode_acceptance --test compile_mode_crdt_acceptance --test completion_popup_acceptance --test config_registry_acceptance --test cua_region_acceptance --test desktop_acceptance --test destination_capture_acceptance --test dired_acceptance --test discovery_acceptance --test discovery_stage2_acceptance --test editops_acceptance --test find_file_acceptance --test folding_acceptance --test folding_stage2_acceptance --test full_grid_resync_acceptance --test gate_script_acceptance --test git_status_stage1_acceptance --test gpu_font_acceptance --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | **`-91f51d0b` / `-6b4b8223`** (log `:3066`, `:3087`) | green | `b31d98ee2f427eca` 214566 |
| R10 | `test --workspace --features crdt --no-fail-fast --test gpu_initial_target_acceptance --test gpu_invocation_acceptance -- --skip basedpyright` | mg | ~`b72843a` | UNKNOWN | **`-5d9105cb` AND `-d4dae4f0`** (log `:3`, `:24`) | green | `81b48fd7a0e261dc` 3553 |
| F1 | `build --workspace --no-default-features --features luajit,crdt && test --workspace --features crdt --no-fail-fast -- --skip basedpyright` | **main** | `72da24a` | clean (verified `git status --porcelain` empty) | `-5d9105cb` / `-d4dae4f0`; today's occupants `e0578039…` / `00f06aeb…` | **red** | `10b55b8ba8741125` 334446 |
| F2 | `test --workspace --features crdt --no-fail-fast -- --skip basedpyright` | mg | ~`b72843a` | UNKNOWN | `-5d9105cb` / `-d4dae4f0` (`postclean.log`) | **red** | `474f88f0dad581fe` 338555 |
| F3 | same argv as F2, with a resource sampler running | mg | ~`b72843a` | UNKNOWN | `-5d9105cb` / `-d4dae4f0` (`sweep-inst.log`) | **red** | `7b8519e7300e8bb3` 338555 |
| F4 | same argv as F2, with a process-table sampler running | mg | ~`b72843a` | UNKNOWN | `-5d9105cb` / `-d4dae4f0` (`sweep-diag.log`) | **red** | `5ccdefc5d89eece3` 338555 |
| F5 | stage 15 of `./scripts/gate --protocol --acceptance bottom_panel_stage1_acceptance --acceptance bottom_panel_stage2a_acceptance --acceptance bottom_panel_stage2b_daemon_acceptance --acceptance bottom_panel_stage2b_gpu_acceptance --acceptance bottom_panel_stage2b_protocol_acceptance --acceptance gui_stage1a_wire_acceptance` | mg | `5174f73` + uncommitted docs | UNKNOWN | `-5d9105cb` / `-d4dae4f0` | **red** (3 bins: both ctrl_c copies + `m6_1_pty_mode_lifecycle`) | `20260817T172537Z-2375685/15-sweep-crdt.log`, sha `e5bdc911e366` |
| F6 | stage 9 of `./scripts/gate --protocol` | mg | **`724b785`** (see below) | UNKNOWN | `-5d9105cb` / `-d4dae4f0` | **red** (2 bins, both ctrl_c copies) | `20260816T063330Z-1977433/09-sweep-crdt.log`, sha `7a75d999ac4f` |
| F7 | stage 9 of `./scripts/gate --protocol` | mg | `5174f73` (committed 08:45:41, run began 08:45:49) | UNKNOWN | `-5d9105cb` / `-d4dae4f0` | **red** (2 bins, both ctrl_c copies) | `20260816T064549Z-2144707/09-sweep-crdt.log`, sha `9d3c6ad1bfc9` |

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

`sweep-crdt` has 17 log files in this target directory. Counted **per
test copy**, not per stage:

| outcome | runs |
|---|---|
| both copies `... ok` | **13** |
| **neither copy executed** — the stage died compiling `pmacs` (`error[E0308]: mismatched types`), log `20260815T182846Z-708693` | **1** |
| both copies `FAILED` | **3** (`…-1977433`, `…-2144707`, `…-2375685`) |

Revision 4 said "14 runs, 11 green, 3 red on other tests" and that the
earlier reds "failed on unrelated tests". **Both were wrong**: the
count is 13 / 1 / 3, and one of the earlier non-passing runs is a
**compile failure that never reached either copy**, not a red on
another test. Of the genuinely red-on-other-tests sweeps, `…-2839374`
(08-15 09:55) failed protocol and version rows and `…-830195`
(08-15 18:37) failed `composition_overhead_under_ten_percent` and a
v21/v20 row — those two did execute `ctrl_c`, and it passed.

So the failure is **not long-standing**. Last observed green:
`20260815T185708Z`. First observed red: `20260816T063330Z`. The machine
was not rebooted across it — the current boot began 08-14 09:30.

**But this is not yet a source boundary.** Reflog and commit times put
HEAD at `7599661` during the last green (`3c06176` was committed 40 s
*after* that run finished) and at `724b785` during the first red
(`5174f73` was committed 08:45:41, after that run ended at 08:42:01 —
revision 4's manifest wrongly recorded F6 at `5174f73`). Cleanliness
was captured for **neither**, and the tree was being edited throughout.
So the window dates a **machine/worktree-state transition**, not two
clean source revisions.

One relationship is worth recording **for exactly what it shows**:
`72da24a` is an **ancestor** of `7599661` (`git merge-base
--is-ancestor`), yet fails today (F1) while `7599661` passed on 08-15.
The two observations differ in commit **and** environment **and**
time, so they are **non-comparable and support no causal conclusion of
any kind** — not even "outcome is not determined by commit alone",
since different commits can deterministically produce different
outcomes. The pair does not discriminate an environmental change, a
source/environment interaction, or a fix before `7599661` followed by a
regression before `724b785`. And an ancestor outside the interval is
irrelevant to whether the interval contains a regression.

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
- **first, reproduce the two candidate endpoints CLEANLY** —
  `7599661` (last observed green) and `724b785` (first observed red) —
  each checked out clean, each in its **own isolated target
  directory**. A decision procedure with **no predicted outcome**:
  endpoints differ → a regression lives in `7599661..724b785` and a
  bisect over that interval is justified; endpoints agree → the
  difference is not captured by those two commits under current
  conditions, and the question becomes what else changed across the
  window.
