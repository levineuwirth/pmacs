# GPU probe SIGINT lane — run manifest

Every physical run behind
`docs/gpu-probe-sigint-framing.md` §4. Pushed so the evidence travels;
the log bodies stay machine-local under
`/home/jeans/build/pmacs-gate-targets/probe-sigint-evidence/` and are
identified here by SHA-256 prefix and byte count.

## Provenance honesty

These runs were made **before** this manifest existed, so their
provenance is **reconstructed, not captured**. Specifically:

- **Commands** are exact — they are the literal invocations issued.
- **Worktree** is exact.
- **HEAD** is given as a range where the run cannot be pinned to one
  commit, and marked `~`. It is never guessed at single-commit
  precision.
- **Cleanliness** was not recorded at the time and is therefore
  `UNKNOWN` for every pre-manifest run. It is not inferred.

**D0 (below) exists because of this.** No conclusion in §4 should rest
on a `UNKNOWN`-cleanliness row once D0 has replaced it.

## The artifact-identity column, and why it exists

Reduction and sweep runs did **not** always execute the same compiled
test executables. Cargo's target selection changes the fingerprint, so
`--test a --test b` and `--workspace` can produce byte-different
binaries for the same source. Verified:

| family | `gpu_initial_target_acceptance` | `gpu_invocation_acceptance` |
|---|---|---|
| reduction (R7–R9) | `-91f51d0b5303ff9f`, sha `36912fa25a72ffc7` | `-6b4b8223dea45247`, sha `858d71486d0b66f0` |
| workspace (R10, F1–F5) | `-5d9105cb7047aab8`, sha `1b3cc86cbb8d6092` | `-d4dae4f01bcdef62`, sha `ede0c07dd9abb456` |

They are byte-different. Any claim of the form "the same binaries pass
as a subset" is therefore **unsupported by these runs**.

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
| R7 | `test --features crdt --no-fail-fast --lib --bins` + `--test` ×14 (targets 6–19) + `--test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | reduction | green | `9e1ebc59ed9f0dd4` 187531 |
| R8 | `test --features crdt --no-fail-fast --test` ×18 (targets 20–37) + `--test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | reduction | green | `8b26ebfcf5f871b4` 28677 |
| R9 | `test --features crdt --no-fail-fast --lib --bins` + `--test` ×32 (targets 6–37) + `--test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | **reduction** | green | `b31d98ee2f427eca` 214566 |
| R10 | `test --workspace --features crdt --no-fail-fast --test gpu_initial_target_acceptance --test gpu_invocation_acceptance -- --skip basedpyright` | mg | ~`b72843a` | UNKNOWN | **workspace** | green | `81b48fd7a0e261dc` 3553 |
| F1 | `build --workspace --no-default-features --features luajit,crdt && test --workspace --features crdt --no-fail-fast -- --skip basedpyright` | **main** | `72da24a` | clean (verified `git status --porcelain` empty) | workspace | **red** | `10b55b8ba8741125` 334446 |
| F2 | `test --workspace --features crdt --no-fail-fast -- --skip basedpyright` | mg | ~`b72843a` | UNKNOWN | workspace | **red** | `474f88f0dad581fe` 338555 |
| F3 | same as F2, with resource sampler | mg | ~`b72843a` | UNKNOWN | workspace | **red** | `7b8519e7300e8bb3` 338555 |
| F4 | same as F2, with process sampler | mg | ~`b72843a` | UNKNOWN | workspace | **red** | `5ccdefc5d89eece3` 338555 |
| F5 | gate stage 15 of `./scripts/gate --protocol --acceptance ×6` | mg | `5174f73` + docs | UNKNOWN | workspace | **red** | gate log `20260817T172537Z-2375685/15-sweep-crdt.log` |

Supporting, not a reduction: `9b8a01076b44bb7c` 98838
(`proc-sample.log`) is the process-table sampler output behind the
retracted "mechanism located" claim.

**R2 and R6 are distinct runs.** Revision 2's §4 cited `gpu3.log` for
both; that log is R6's three-suite run only, and R2's log was never
preserved. R1 likewise has no log. Both are marked accordingly rather
than backfilled.

## D0 — re-run the matrix with captured provenance

Before any §4 row is relied on for a conclusion, re-run the reductions
under a harness that records, per run and at the time: exact argv and
environment, worktree, `git rev-parse HEAD`, `git status --porcelain`
emptiness, the artifact hashes actually executed, the result, and the
log digest. Two constraints learned the hard way:

- run them **at `main`**, not on a feature branch — R1–R10 ran in the
  `panel-mapping-generation` worktree, which carries §5b changes;
- record the **artifact hash per run**, since command shape changes it,
  which is the whole reason R9's result did not mean what it appeared
  to mean.
