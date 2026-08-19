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

| id | exact command (after `cargo`) | WT | HEAD | clean | Cargo suffixes executed | result | log (sha256/16, bytes) |
|---|---|---|---|---|---|---|---|
| R1 | `test --features crdt --test gpu_invocation_acceptance ctrl_c_on_launcher_group` ×3 | mg | ~`724b785`–`5174f73` | UNKNOWN | **UNKNOWN** (no log) | green, 0.15–0.17 s | **none preserved** |
| R2 | `test --features crdt --test gpu_invocation_acceptance` | mg | ~`724b785`–`5174f73` | UNKNOWN | **UNKNOWN** (no log) | green, 15 passed | **none preserved** |
| R3 | `test --workspace --features crdt --no-fail-fast -- --skip basedpyright ctrl_c_on_launcher_group` | mg | ~`724b785`–`5174f73` | UNKNOWN | `-5d9105cb` / `-d4dae4f0` | green | `e09a96512035284e` 33113 |
| R4 | `test --features crdt --lib --test gpu_invocation_acceptance --no-fail-fast` | mg | ~`724b785`–`5174f73` | UNKNOWN | `-6b4b8223` only | green, 2145 + 15 | `89050c702de22d57` 158812 |
| R5 | `test --features crdt --no-fail-fast --test gate_script_acceptance --test gpu_invocation_acceptance` | mg | ~`5174f73`–`b72843a` | UNKNOWN | `-6b4b8223` only | green | `31b3e5249b475479` 3706 |
| R6 | `test --features crdt --no-fail-fast --test gpu_font_acceptance --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | mg | ~`5174f73`–`b72843a` | UNKNOWN | `-91f51d0b` / `-6b4b8223` | green, 11+15+15 | `332693a39c73731a` 4569 |
| R7 | `test --features crdt --no-fail-fast --lib --bins --test acceptance --test ambient_isolation_acceptance --test auto_indent_acceptance --test auto_indent_crdt_acceptance --test auto_pair_acceptance --test auto_pair_crdt_acceptance --test autosave_acceptance --test bottom_panel_stage1_acceptance --test bottom_panel_stage2a_acceptance --test bottom_panel_stage2b_daemon_acceptance --test bottom_panel_stage2b_gpu_acceptance --test bottom_panel_stage2b_protocol_acceptance --test comment_toggle_acceptance --test compile_mode_acceptance --test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | `gpu_invocation…-6b4b8223` **only** — R7 does not select `gpu_initial_target` | green | `9e1ebc59ed9f0dd4` 187531 |
| R8 | `test --features crdt --no-fail-fast --test compile_mode_crdt_acceptance --test completion_popup_acceptance --test config_registry_acceptance --test cua_region_acceptance --test desktop_acceptance --test destination_capture_acceptance --test dired_acceptance --test discovery_acceptance --test discovery_stage2_acceptance --test editops_acceptance --test find_file_acceptance --test folding_acceptance --test folding_stage2_acceptance --test full_grid_resync_acceptance --test gate_script_acceptance --test git_status_stage1_acceptance --test gpu_font_acceptance --test gpu_initial_target_acceptance --test gpu_invocation_acceptance` | mg | ~`b72843a` | UNKNOWN | `-91f51d0b` / `-6b4b8223` (`half2.log:438`, `:459`) | green | `8b26ebfcf5f871b4` 28677 |
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

## D1/D2 — EXECUTED 2026-08-19. The outer invocation is the variable

**The causal variable is the OUTER invocation of the test runner**, so
it is recorded here as a first-class column. Earlier "exact commands"
in this file are incomplete for that reason: they gave the inner
`cargo` argv and omitted how the runner itself was started.

### Controlled arms, committed head, worktree-local target

Head `38f2af4`, `dirty=0`, worktree
`/home/jeans/Repos/personal/pmacs-probe-sigint`, target
`/home/jeans/build/pmacs-gate-targets/pmacs-probe-sigint-84ed0f9e`,
`TMPDIR=/home/jeans/build/pmacs-gate-targets/tmp/arms`.

Inner command, identical in both arms:

```
cargo test --features crdt --no-fail-fast \
  --test gpu_invocation_acceptance --test gpu_initial_target_acceptance \
  -- ctrl_c_on_launcher_group
```

Outer invocation, the only difference. `arms.sh` is machine-local, so
the commands are given **fully expanded** — a reader elsewhere needs no
access to it:

```
# fg arm
cd /home/jeans/Repos/personal/pmacs-probe-sigint && \
env TMPDIR=/home/jeans/build/pmacs-gate-targets/tmp/arms \
    CARGO_TARGET_DIR=/home/jeans/build/pmacs-gate-targets/pmacs-probe-sigint-84ed0f9e \
    cargo test --features crdt --no-fail-fast \
      --test gpu_invocation_acceptance --test gpu_initial_target_acceptance \
      -- ctrl_c_on_launcher_group

# bg arm — byte-identical inner command, wrapped:
setsid nohup sh -c '<the fg command above>' > <log> 2>&1 & disown
```

The wrapper additionally recorded `git rev-parse HEAD`,
`git status --porcelain | wc -l`, the exit status, both copies'
results, the executed suffixes, their hashes, and the log digest.

| arm | outer | exit | ok | failed | `SigIgn` | binary hashes | log sha256/16 |
|---|---|---|---|---|---|---|---|
| fg | foreground | 0 | 2 | 0 | not captured (no failure ⇒ no dump) | `aaec01673691479a…` (prefix) |
| bg | `setsid nohup … &` | 101 | 0 | 2 | `0000000000001007` | `c744d85a84cb8683…` (prefix) |

Both arms executed the same two binaries, whose **full** SHA-256 are:

```
gpu_initial_target_acceptance-91f51d0b5303ff9f
  0890b78cca22ac1e80b79845f85fb6e88def3330db15ae123a2a672d3084124c
gpu_invocation_acceptance-6b4b8223dea45247
  ef6ff1c15e11062ab53a075763814f32c1bbc9be1b146d068c60e91fa247c696
```

Same head, same target directory, `dirty=0`, and the binaries were not
rebuilt between arms — so nothing but the outer invocation varies. The
**log** digests above are 16-character **prefixes**, not full values,
and are identifiers only; no claim rests on them.

### Disposition — UNRECORDED CORROBORATION, not a controlled arm

This table was read ad hoc from `/proc/self/status` in the two shells
and **its runs were not captured**: no head, no cleanliness, no log,
no digest. It agrees with the arms above and with §4c's capture, and it
is labelled separately for that reason — it corroborates, it does not
evidence.

| context | child `SigIgn` | `SIGINT` |
|---|---|---|
| foreground | `0000000000001000` | bit 12 (SIGPIPE) only — deliverable |
| `setsid nohup … &` | `0000000000000007` | SIGHUP, SIGINT, SIGQUIT — ignored |

The portable probe adopted as the remedy (framing §7c) supersedes it as
the *recorded* mechanism check:
`sh -c 'trap "exit 23" 2; kill -INT $$; exit 0'` exits **23** when
`SIGINT` is deliverable and **0** when it is inherited as ignored.
Verified in both contexts.

### The first D1/D2 capture, and why it is superseded

The capture quoted in framing §4c came from `d12.log`, which finished
14:10 — **five minutes before `afe3631` committed the diagnostic
code** — and ran in the reused `d0a-B` target directory rather than
this worktree's. Its signal facts agree with the arms above, but it is
**not admissible provenance**: uncommitted tree, foreign target. The
arms table replaces it, and `d12.log` is retained only as the first
sighting.

### Historical foreground/background mapping — RECONSTRUCTED

The claim that "every reduction was foreground and every full sweep was
backgrounded" is **reconstructed from this session's transcript, not
captured at run time**. No run before today recorded its outer
invocation, because none of the harnesses knew it mattered. It is
consistent with every observation and with the two arms above, but it
is inference, and rows R1–R10 and F1–F7 carry **no outer-invocation
field**. That gap is the direct cause of nine revisions spent on a
confounded matrix.

## D0a — EXECUTED 2026-08-19. Verdict: difference NOT captured

Ten runs, counterbalanced `A B B A A B B A A B`, N = 5 per endpoint,
**zero voids, zero splits**. Endpoints checked out detached and clean
in dedicated worktrees (`pmacs-d0a-A`, `pmacs-d0a-B`), each with its own
target directory, each run performing the gate's `build-crdt`
precondition then the `sweep-crdt` command. `dirty=0` verified per run.

| run | endpoint | HEAD | class | ctrl_c ok/failed | red bins | log |
|---|---|---|---|---|---|---|
| A#1 | A | `7599661` | **red** | 0 / 2 | 3 | `d0a/A-1.log` |
| B#1 | B | `724b785` | **red** | 0 / 2 | 2 | `d0a/B-1.log` |
| B#2 | B | `724b785` | **red** | 0 / 2 | 2 | `d0a/B-2.log` |
| A#2 | A | `7599661` | **red** | 0 / 2 | 4 | `d0a/A-2.log` |
| A#3 | A | `7599661` | **red** | 0 / 2 | 3 | `d0a/A-3.log` |
| B#3 | B | `724b785` | **red** | 0 / 2 | 2 | `d0a/B-3.log` |
| B#4 | B | `724b785` | **red** | 0 / 2 | 2 | `d0a/B-4.log` |
| A#4 | A | `7599661` | **red** | 0 / 2 | 3 | `d0a/A-4.log` |
| A#5 | A | `7599661` | **red** | 0 / 2 | 3 | `d0a/A-5.log` |
| B#5 | B | `724b785` | **red** | 0 / 2 | 2 | `d0a/B-5.log` |

**Exact commands.** Every run, in full. `<WT>` is
`/home/jeans/Repos/personal/pmacs-d0a-A` (detached at `7599661`) or
`/home/jeans/Repos/personal/pmacs-d0a-B` (detached at `724b785`);
`<TD>` is `/home/jeans/build/pmacs-gate-targets/d0a-A` or
`/home/jeans/build/pmacs-gate-targets/d0a-B` correspondingly:

```
env TMPDIR=/home/jeans/build/pmacs-gate-targets/tmp/d0a \
    CARGO_TARGET_DIR=<TD> \
    sh -c 'cd <WT> \
      && cargo build --workspace --no-default-features --features luajit,crdt \
      && cargo test --workspace --features crdt --no-fail-fast -- --skip basedpyright'
```

**Per-run provenance, transcribed.** All runs: `exit=101`, `dirty=0`,
`ok=0 failed=2`, suffixes `-5d9105cb` / `-d4dae4f0`, `/tmp` 3 G of 30 G.
Times are local 2026-08-19. `load` is the 1/5/15 average at run start;
`MemFree`/`MemAvail` in MB; `daemons` counts live `pmacs --daemon`:

| start | run | class | red bins | load | MemFree | MemAvail | daemons | log sha256/16 |
|---|---|---|---|---|---|---|---|---|
| 12:21:20 | A#1 | red | 3 | 2.51 2.85 3.57 | 1549 | 42069 | 72 | `e1c0fe47d55d8f5e` |
| 12:26:55 | B#1 | red | 2 | 7.86 13.48 8.97 | 9405 | 42725 | 76 | `3105794d515e6ec3` |
| 12:32:12 | B#2 | red | 2 | 8.04 18.28 13.23 | 9761 | 43370 | 80 | `07a662fb5ca15687` |
| 12:35:39 | A#2 | red | 4 | 11.34 21.23 16.20 | 8933 | 43281 | 84 | `450aacb9d15244c9` |
| 12:39:35 | A#3 | red | 3 | 13.40 26.07 20.45 | 10628 | 42989 | 88 | `19156bbbc852e2d3` |
| 12:43:26 | B#3 | red | 2 | 8.66 22.18 20.82 | 8778 | 43588 | 92 | `6f0d768a76ed9cb0` |
| 12:47:20 | B#4 | red | 2 | 17.96 33.10 27.07 | 10403 | 43617 | 96 | `75a043a3d5568598` |
| 12:50:49 | A#4 | red | 3 | 10.99 27.07 26.52 | 10702 | 43335 | 100 | `c1b6d88c08764d9a` |
| 12:54:47 | A#5 | red | 3 | 9.75 28.50 28.70 | 8494 | 43053 | 104 | `2ae21a3f53fc1a1a` |
| 12:58:38 | B#5 | red | 2 | 11.43 26.69 28.97 | 8562 | 43122 | 108 | `82c5baa0c7e49e37` |

**Two defects in the previous transcription of this table, recorded
rather than silently fixed.** Every digest had lost its leading hex
character — A#1 read `1c0fe47d55d8f5e…` where the value is
`e1c0fe47d55d8f5e` — because the extraction started one byte late in
`logsha=<value>`. And `/tmp` and `MemAvailable` were captured by the
harness but dropped from the table. A transcription that silently
corrupts its own digests is worse than a pointer to the raw file, since
it looks verifiable and is not.

**`uptime` was NOT captured, and is `UNKNOWN` for all ten runs.** §7's
condition list names `uptime`; the harness recorded only the load
averages from it and discarded the elapsed time. The classifications
stand — none of them depends on it — but the condition list was **not
fully satisfied**, and D1/D2's harness must capture it. Recorded rather
than quietly treated as met.

Note the leaked-daemon count climbing 72 → 108, four per run. Recorded,
not implicated: it rises monotonically while every run classifies the
same. Raw logs stay machine-local at
`/home/jeans/build/pmacs-gate-targets/d0a/`; the table above is the
portable record.

**Endpoint verdicts: A uniform-red, B uniform-red.** By the approved
table this is the *both endpoints uniform the same way* row:

> **the difference is not captured by those two commits** under current
> conditions, and the question becomes what else changed across the
> window.

### What this settles

- **The two commits do not discriminate under current conditions.**
  `7599661` passed inside `sweep-crdt` on 08-15 and now fails **5/5**
  clean. **No bisect of `7599661..724b785` is justified under current
  conditions**, and none will be run.
- **That is the whole of the causal claim.** An earlier wording here
  said "the source hypothesis is eliminated" and that "the interval
  cannot contain the transition"; both are **withdrawn**. Uniform-red
  today is silent about what was true on 08-15 — a historical source
  regression could be **masked** by a later environmental effect, or by
  a source/environment interaction that makes both commits fail now.
  Not discriminating is not the same as not differing.
- **The onset window is deprioritised, not excluded.** It remains a
  true observation, and it remains *possible* that source matters
  within it; what is established is only that source cannot be
  probed **by this comparison, now**.
- **A reliable reproduction now exists.** 10/10 today, on two different
  commits, at ~4 minutes per run. **This is the most useful thing D0a
  produced**: the mechanism diagnostics D1/D2 no longer depend on a
  rare event, and can proceed immediately.
- **A's three extra failures are recorded, not swept up**:
  `a54_real_daemon_real_pty_and_headless_gpu_render_one_panel_hosted_terminal`,
  `one_daemon_serves_a_v21_panel_session_and_a_shipped_v20_client`, and
  `m6_1_pty_mode_lifecycle_started_then_exited`. The v21/v20 row is
  expected to differ at that older commit; the other two are
  process/PTY-spawn rows, the same family as the target. They do not
  affect classification, which reads only the two target copies.

### What it does not settle

Nothing about the mechanism. "What else changed across the window" has
one cheap negative result so far: **no package activity in the window**
(`/var/log/pacman.log` shows nothing between 08-15 19:57 and 08-16
06:33; nearest is 08-18). The `1.88` rust toolchain directory has mtime
08-15 22:39, inside the window, but the gate builds with `1.95.0`.
Neither is pursued further, because with a reliable reproduction in
hand **direct measurement (D1/D2) dominates archaeology**.

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
  directory**. A decision procedure with **no predicted outcome**, and
  **not decided by one run per endpoint**. The framing's §7 D0a holds
  the governing contract — the total run classifier
  (green / red / split / void), the void budget, the endpoint table and
  the bisect-step policy. In summary, keeping the two conclusions with
  the verdicts they actually belong to:

  - **expected-direction clean split** (`7599661` uniform green,
    `724b785` uniform red) → a bisect of `7599661..724b785` is
    permitted;
  - **inverted clean split** (`7599661` uniform red, `724b785` uniform
    green) → the commits differ, but the observed direction contradicts
    the onset reading; record it and re-examine that reading before any
    bisect;
  - **mixed at either endpoint** → the failure is **intermittent under
    fixed source**; no bisect;
  - **both endpoints uniform the same way**, green or red → **the
    difference is not captured by those two commits** under current
    conditions, and the question becomes what else changed across the
    window.

  Revision 8 attached "the difference is not captured" to the *mixed*
  clause. That was wrong — mixed means intermittency, not absence of a
  difference — and revision 9 corrected it in the framing but **left
  this file untouched**, because the edit's anchor silently missed.
