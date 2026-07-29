# Testing audit — gaps and shortcomings

Audited 2026-07-29 at `7a3a55d` (branch `journey-stage1a-directory-open`,
even with `main` for everything cited here). Scope: testing coverage and
test quality only, plus CI-pipeline improvements (added mid-audit at the
user's request). This document is the raw material for a future testing
arc; each numbered lane in §8 still needs its own framing doc per the
workflow, and coherence-affecting lanes must state their `COHERENCE.md`
§20 impact there, not here.

**Every count in this file is a reading, not a constant.** The dark-test
census in `docs/active-work.md` ("The CRDT half of the test corpus is
dark in CI — NEEDS A LANE") moved three times in three weeks. Re-measure
before acting; the appendix gives the commands.

Method: five parallel read-only audits (dark/gated tests; unit-coverage
map; Lua-layer coverage; test-quality/flake patterns; CI-vs-local-gate
delta), findings cross-checked against `docs/active-work.md`,
`docs/agent-handoff.md` §3/§5, and the memory of prior review rounds.
Claims that accuse an existing doc of being wrong were re-verified by
hand before inclusion.

---

## 0. Baseline: what exists

- Root `pmacs` package: ~2,030 unit tests in `src/`, 92 integration
  targets in `tests/` (~93k lines, ~1,426 tests), shared harness in
  `tests/common/` (daemon + PTY, both high quality).
- `pmacs-gpu`: 202 unit tests, including a real headless wgpu render
  harness with ~20 pixel-asserting tests. No `tests/` dir; bin-only, so
  doctests are structurally impossible.
- `pmacs-protocol`: 17 unit tests (12 transport, 5 terminal). No
  `tests/` dir.
- `builtin/`: 16,445 lines of Lua across 31 files. No Lua test
  framework of any kind; all coverage is indirect via Rust `eval()`.
- Property tests: 5 proptest sites, all in the root crate (CRDT
  convergence, rope≡CRDT projection, text_view inverse, keymap stack,
  manifest round-trip). One checked-in regression seed (`crdt.txt`).
  No fuzzing anywhere.
- CI: one workflow, 8 jobs / 12 legs, ~14.5 min wall per run, triggers
  on `push:main` + `pull_request` only. No coverage measurement, no
  scheduled runs, no branch protection on `main` (verified via API:
  404, so every job is advisory).

The suite is unusually thoughtful in places — the daemon harness's
connect-based readiness probe, the `PMACS_REQUIRE_GPU` hard-fail
pattern, `dual_mode_test!`, the encoding-stability pins, `scripts/bite`
itself. Most defects below are **inconsistent application of patterns
the project already invented**: the correct helper exists in one file
and a degraded copy exists in another.

---

## 1. Dark tests — things that never run

### 1.1 CRDT (known; already a scoped lane — do not re-scope here)

`docs/active-work.md` §"The CRDT half of the test corpus is dark in CI"
carries the full census (273 dark at `74301d1`: 185 library + 88
integration), the two-part fix shape, and the two standing obstacles
(`clippy --features crdt` is red on `main`; the `setsid_escapee` ~1-in-5
load flake). Defer to it. What this audit **adds** to that lane:

- **Nothing anywhere compiles the 8 `#![cfg(feature = "crdt")]`
  integration targets** — not CI, and not the local gates either:
  `cargo test --lib --features crdt` builds the library only. The
  `*_crdt_acceptance.rs` / `m10_*` files are type-checked exclusively
  on developer machines that happen to run them by name. CI builds
  them as empty green binaries, which reads as "passed" in the log.
- `tests/m10_11_perf.rs` puts its `#![cfg(feature = "crdt")]` at line
  60, after the module doc — any future head-of-file scan for the gate
  will misclassify it (the "never classify from truncated output" trap,
  now in file-layout form).
- `src/buffer.rs:2984` — the parked CRDT no-op-replace undo bug is
  doubly dark (`#[ignore]` + crdt) and its own comment says un-ignoring
  it is step one of the fix. It needs a named owner or it will sit
  forever.

### 1.2 Vacuous-green skips: tests that pass without executing

The `let Ok(_) = which_binary(x) else { eprintln!(...); return; }` shape
passes green when the tool is absent. **21 external-tool-gated tests; 15
have never executed their bodies in CI** because `ci.yml` installs none
of the tools (the only `apt-get install` in the workflow is lavapipe):

- All 9 real-language-server tests in `tests/m4_acceptance.rs`
  (rust-analyzer ×2, basedpyright, clangd ×2, gopls ×2, json-ls,
  yaml-ls) — the only real-LSP coverage in the suite.
- The entire M6.8 multi-REPL acceptance (6/6 tests, gated on a
  lua/luajit binary) plus the zsh/fish/lua shell-spawn tests in
  `tests/m6_5_repl_acceptance.rs`.

Same pathology, already ledger-documented: `a37` in
`tests/vterm_stage3_acceptance.rs` is triple-dark (crdt-gated; skips
when `pmacs-gpu` isn't built; `PMACS_REQUIRE_GPU` is only set on a job
that tests a *different package*). It has never executed in CI.

**The project already owns the fix pattern**: `PMACS_REQUIRE_GPU`
promotes a silent skip to a hard failure, and CI arms it on the
`gpu-render` job. There is no `PMACS_REQUIRE_LSP` / `PMACS_REQUIRE_SHELLS`
analogue, and no CI step installs the cheap tools (gopls, clangd, zsh,
fish, lua are all one `apt-get`/`brew` line; basedpyright is an npm
install).

Related mid-test variant: `tests/m4_acceptance.rs:5783-5789` skips its
only assertion when rust-analyzer is "likely still indexing" — the
assertion vanishes exactly when the system is slow, which is when a
regression would show.

### 1.3 Ignored tests owned by nobody

27 `#[ignore]` tests; 11 run in the four `--ignored` CI jobs; **16 run
nowhere**:

- `src/rope.rs:1101,1112,1130` (3 perf smokes) and `src/buffer.rs:2984`
  — `cargo test --lib` never passes `--ignored`; these are dead code
  that still compiles.
- `tests/m3_acceptance.rs:218` (`m3_6_grep_kernel_under_2s_on_8_cores`)
  — `#[ignore]` + needs `PMACS_KERNEL_PATH` + no job names
  `m3_acceptance --ignored`.
- The m10_2/m10_11 perf and doubled-PTY operator tests (crdt-gated on
  top of `#[ignore]`; the doubled-PTY trio is deliberately
  operator-invoked — fine, but nothing records when it last ran).

### 1.4 CI runs weakened versions of the gates it does run

CI green currently certifies less than the spec text implies:

- `async_runtime_soak_lifecycle_stable` runs at **10 s vs its 3600 s
  spec gate** (`PMACS_SOAK_SECS`); the hour-long soak has never run.
- M6 ingest gate: CI floor 64 MB/s vs the 100 MB/s default; cancel
  gate: 30 trials/500 ms vs 100/5000. Documented in `ci.yml` comments
  as a runner-budget tradeoff, but nothing anywhere runs the full
  profile, ever.
- The soak's FD and RSS assertions silently no-op on macOS
  (`/proc`-dependent) while the test reports green.

---

## 2. Coverage gaps by layer

### 2.1 pmacs-protocol: the wire format is tested from the wrong crate

`pmacs-protocol/src/message.rs` — 2,100 lines: `InstanceMessage`,
`FrontendEvent`, `Hello`, capability negotiation, version constants —
has **zero tests in its own crate**. The real coverage (117 tests:
postcard round-trips, v16–v19 encoding-stability pins, negotiation
matrix) lives in the root package's `src/protocol.rs`. Consequences:

- `cargo test -p pmacs-protocol` certifies almost nothing (17 tests).
- A future consumer that depends on `pmacs-protocol` alone inherits an
  effectively untested crate.
- CI's `-p pmacs-protocol --all-targets` step cannot run doctests
  (`--all-targets` excludes them) — zero exist today, so this is a trap
  armed for the first doctest someone writes.

Also: local `cargo fmt --check` (no `--all`) formats the root package
only, and local clippy via `--workspace` covers `pmacs-protocol` while
CI's clippy never does — each side has a hole the other covers.

### 2.2 Root crate: zero-test production files

Largest files with **no `#[cfg(test)]` at all**:
`src/terminal/session.rs` (854 — the process/session registry),
`src/lua_bindings/window_panel.rs` (796), `src/lua_bindings/mcp.rs`
(602), `src/lua_bindings/index.rs` (395), `src/lua_bindings/fold.rs`
(390), `src/lua_bindings/diag.rs` (237), `src/audit/rules.rs` (154).
Thinnest large files: `src/mcp.rs` (2.5 tests/kloc),
`src/lua_bindings/mod.rs` (13,955 prod lines, 10.6/kloc — also the
F-016 split candidate), `src/daemon.rs` (9.7/kloc). The lua_bindings
submodules are integration-covered via acceptance suites but have no
unit-level pins; per the project's own review history ("pin where
production CALLS"), that leaves the binding seam itself unpinned.

The LSP client (`src/lsp.rs`) unit-tests only wire mechanics (framing,
URIs, position encoding); server lifecycle, capability handling, and
the diagnostics pipeline are covered solely through the fake-LSP
acceptance path — plus the 9 real-server tests that never run (§1.2).

### 2.3 pmacs-gpu: the live half is untested

Well covered: headless render, math layout/parse, attach negotiation,
coalescing, minimap, tab projection. Untested: the winit
`ApplicationHandler` event loop (one test), the live surface/swapchain
`render()` path (only the offscreen twin is exercised), clipboard
(`arboard` — zero tests reference cut/copy/paste), font-database
sanitization beyond the four bundled fixture TTFs, edge-scroll/context
menu/minibuffer interactive drivers. No proptests in the crate.

### 2.4 The Lua layer: 16.4k lines, no test framework, one universal
guarantee

Every `EditorState::new()` evals all 30 runtime chunks, so every Rust
test is a *load/syntax* smoke for all Lua. Behavior coverage is via
per-file `eval()` helpers **copy-pasted into 67 of 92 test files** (823
call sites) poking module namespaces directly. Findings:

- **`pmacs.error` is a test-stub-only API — confirmed and worse than
  the memory of it.** ~35 references across 9 builtin files, every one
  guarded (`if pmacs.error then` / `pcall`). Production never assigns
  it; the only assignments are 10 stubs inside test code. The tests
  that assert into those stubs prove the message is *formatted*, not
  that anyone receives it — all would stay green if the channel were
  deleted. `dired.lua:56`, `lsp.lua:1659`, `lean.lua:171` acknowledge
  this in comments and route around it via `set_status`. The
  `COHERENCE.md` tally ("fifteen call sites") is stale.
- **56 statement-position `pcall`s discard errors entirely**, 25 of
  them in `lsp.lua`'s hot path (`did_change`, `did_open`, inlay/token
  pulls). A server going silent after a failed `did_change` is
  unobservable — including to the test suite.
- **Interaction islands with zero test contact**: `menus/default.lua`
  (6 of 11 commands never referenced anywhere in the tree),
  `keymaps/default.lua` (no suite pins the default binding table — a
  silent rebinding regression only surfaces if an unrelated suite
  happens to type that key), `window.lua` and `fold.lua` (10/10
  registered commands unnamed in tests; their frontend/point-resolution
  wrapper bodies are bypassed by the Rust-level tests underneath),
  `workers.cancel-at-point` and `recent-files` (never mentioned).
- **Command-name coverage is the honest metric and it is low**:
  `lsp.lua` registers 15 commands, 12 never named by any test;
  `commands/default.lua` registers 80, 35 never named.
- `builtin/api/packages.lua` is never loaded or parsed by anything; its
  single "test" greps it as a string for three substrings.
- `lean_abbrev.lua` (1,883 lines, 11% of the Lua tree) has ~5
  assertions; generated data, low risk, but zero drift detection
  against its generator.

### 2.5 No fuzzing, and two ideal targets

No cargo-fuzz/AFL/libfuzzer anywhere. The two highest-value targets are
exactly the shapes fuzzing is best at: `pmacs_protocol::transport::
read_message` (length-prefixed decoder with a `MAX_FRAME_BYTES` guard;
12 hand tests) and `src/ansi.rs` (VT escape parser; 44 hand tests).
Both parse attacker-adjacent input (a compromised frontend socket; PTY
output from arbitrary child processes). The existing 30 s
`fuzz_no_crashes` in `tests/acceptance.rs` fuzzes buffer edits only.

---

## 3. Test-quality defects (races, hangs, silent lies)

Ranked; each is a known-bitten shape recurring in a new place.

1. **`RuntimeHandles::drop` joins a non-interruptible reader with no
   timeout** (`src/process.rs:631-642` + `:1874`). This is the real
   basedpyright hang: a PATH'd server leaving a descendant holding the
   stdout write end blocks `read()` forever, below every test-level
   deadline. It is a **product** defect the skip merely hides; the fix
   (give non-group readers the `O_NONBLOCK`+poll loop that group
   readers already have) is named as a deferral at
   `src/process.rs:1937-1940`. Fixing it retires the `--skip
   basedpyright` clause from the gates and the latent CI stall (§4).
2. **`wait_for_file` without a non-empty guard**
   (`tests/vterm_stage2_acceptance.rs:723`) — returns on the first
   successful read, including the zero-byte window of the non-atomic
   writers it waits on; four call sites then assert exact bytes. The
   fixed variant (`&& !bytes.is_empty()`) already exists at
   `tests/bottom_panel_stage1_acceptance.rs:2307` — someone was bitten
   and patched only the local copy. The vterm file's own panic message
   ("published but malformed") describes the race it causes. Cheapest
   high-value fix in this audit.
3. **Fixed-iteration `settle()` as the sole synchronization before
   strict assertions about a real forked LSP child** — 16–20 ms total,
   no predicate, no deadline: `tests/lsp_multi_root_acceptance.rs:125`,
   `tests/lsp_dispatch_seams_acceptance.rs:105`,
   `tests/lean4_server_acceptance.rs:112`; ~58 `settle()` calls gating
   ~68 tests. The correct predicate-plus-deadline pump exists at
   `tests/journey_acceptance.rs:63`, `tests/dired_acceptance.rs:107`,
   and (exemplary, with diagnostic dumps) `tests/m10_11_acceptance.rs:316`.
4. **`drain_until`/`drain_lsp_until` return partial results on timeout
   instead of panicking, and 15 call sites discard the result** (`let _
   = drain_lsp_until(...)`). A test whose awaited event never arrives
   proceeds unsynchronized and fails elsewhere — or passes for the
   wrong reason. `tests/m9_4_acceptance.rs:121` already does it right
   (panic with the job id). Also note these helpers are effectful
   (ticking reaps children, `take_events` consumes) — the known
   `drain_until` trap — and at least one assertion
   (`tests/m4_acceptance.rs:1070-1084`) is load-bearing on the side
   effect of a *discarded* call.
5. **Git fixtures inherit the ambient global gitconfig** (9 files, e.g.
   `tests/m8_7_acceptance.rs:87`, `tests/m7_review_acceptance.rs:30`).
   A developer with `commit.gpgsign = true` gets an unbounded pinentry
   hang inside `git commit`. One `GIT_CONFIG_GLOBAL=/dev/null` +
   `GIT_CONFIG_NOSYSTEM=1` pair per fixture closes it.
6. **Bare sleeps with strict assertions after**: 20 sleep-then-assert
   sites outside poll loops, notably `tests/m5_5_acceptance.rs:184`
   (300 ms for daemon slot-clear), `tests/m4_acceptance.rs:7093`
   (120 ms against a 75 ms coalescing window — 45 ms of margin), and
   negative assertions after fixed waits
   (`tests/lean4_server_acceptance.rs:493` acc28,
   `tests/m8_7_acceptance.rs:786`) that pass if the thing simply
   hasn't happened *yet* — each needs a positive-control sentinel.
7. **Unit tests that require serial execution without declaring it**:
   `src/file_io.rs:427` mutates process-wide cwd (`set_current_dir`)
   while concurrent tests resolve relative paths through
   `current_dir()` fallbacks; `/proc`-global probes in
   `tests/worker_shutdown_acceptance.rs` and the M6.6 RSS gate.
8. **Smaller but real**: no `set_read_timeout` in
   `tests/gpu_invocation_acceptance.rs:165`'s hand-rolled
   `wait_for_daemon` (the shared harness's version at
   `tests/common/daemon.rs:200` has one, with a comment saying why); a
   2 s 100%-CPU busy-spin at
   `tests/statusline_segments_acceptance.rs:1016` that manufactures
   load for its siblings; leaked orphan daemons on timeout in
   `tests/m5_7_acceptance.rs:211` plus a PID-reuse-unsafe `kill(pid,0)`
   probe; `PmacsPty::output()` clones an unbounded buffer on every
   20 ms poll (`tests/common/pty.rs:55`); `sleep(1100ms)` in a unit
   test (`src/file_io.rs:420`).

**The serialization contract is CI-only and it cuts both ways.**
`--test-threads=1` appears at five points in `ci.yml` and nowhere in
the documented local gates, `.cargo/`, or any hook. So the process-
lifecycle races CI's comment admits to dodging are structurally never
caught (masked in CI, dismissed as local flakes), while
serial-dependent tests (the `settle()` family, `/proc` probes, the cwd
test) pass in CI and flake for whoever runs the documented local gate.

---

## 4. Tooling and infrastructure gaps

- **`scripts/bite` has no positive control.** It never runs the tests
  against the *current* tree, so a test that fails everywhere — e.g.
  broken, or environmentally skipped-then-failing — reports `bite: OK`.
  Given §1.2's silent skips, a bite claim on a PATH-gated test is
  meaningless. ~5-line fix: require `cargo test "$@"` to pass on the
  unmodified tree first.
- **`docs/agent-handoff.md` (§5, "A fix must be COMMITTED before it is
  bitten") misdescribes bite's mechanism** — verified: it claims
  restore-by-`git checkout --` (revert to HEAD, destroying uncommitted
  work), but the script has restored from a `mktemp` copy via `cp` +
  `trap` since its only commit. The commit-first rule stays sound for
  two *different* reasons (SIGKILL bypasses the trap; biting an old
  version of your own uncommitted claim is circular), but the stated
  mechanism will mislead an agent reasoning about what is safe. Fix
  the doc, keep the rule.
- **Bite-verification is honor-system.** It's step 4 of the working
  method and appears in eight framing docs as prose claims, but
  nothing re-checks a bite after the code moves. (A CI mode is
  probably overkill; recording the bitten ref+command in the framing
  doc would at least make claims re-runnable.)
- **No coverage measurement, Rust or Lua.** No tarpaulin/llvm-cov/
  grcov/luacov anywhere. For Lua it's worse than absent: Rust line
  coverage of `include_str!` chunks would report the *eval* line, not
  which Lua branches ran. §2.4's command-name coverage is currently the
  only measurable proxy.
- **No enforcement layer between "documented gates" and "what runs"**:
  the gate list exists as prose in three places (CLAUDE.md, AGENTS.md,
  handoff §3 — the handoff adds a full-workspace sweep the other two
  omit). No script runs the suite, so drift among the three copies is
  invisible. A `scripts/gates` that executes the canonical list would
  make the prose testable.

---

## 5. CI: correctness hardening

(Findings that change what CI *certifies*; speedups are §6.)

1. **Branch protection is off** — every job is advisory; a red run
   merges as easily as a green one. Turn on required checks for the
   cheap deterministic jobs at minimum (fmt, clippy, ubuntu test legs).
2. **No job timeouts except m6** (15 min). Everything else inherits
   360 min. The day a runner image ships any of the PATH-gated tools
   (§1.2), the basedpyright-class hang burns 6 h × 4 matrix legs with
   no signal. `timeout-minutes: 25` on every job is free insurance.
3. **The crdt lane** (already scoped in `docs/active-work.md`): CI leg
   for `--features luajit,crdt`, GPU-requiring crdt suites onto
   `gpu-render` with `PMACS_REQUIRE_GPU=1` stated as a requirement (or
   a37 runs vacuously), clippy-crdt fixed first. Not re-scoped here.
4. **Arm the skips it already runs**: install gopls/clangd/zsh/fish/
   lua on the ubuntu test leg (one apt line + one `go install`/npm
   line) and add `PMACS_REQUIRE_LSP`/`PMACS_REQUIRE_SHELLS` env
   guards mirroring `PMACS_REQUIRE_GPU`, set only on legs where the
   tools are installed. This converts 15 permanently-vacuous tests
   into real CI coverage.
5. **A scheduled (cron) job** for what per-PR CI can't afford: the
   3600 s soak, full M6 cancel/ingest profiles, the `--ignored` perf
   smokes nothing runs (§1.3), and N repetitions of the known
   load-sensitive suites as a flake canary. Nightly, ubuntu-only,
   `workflow_dispatch` for manual runs. This also gives intermittent
   failures a place to burn in instead of surfacing mid-review.
6. **One deliberately-parallel leg** (default test threads, ubuntu,
   allowed-to-fail at first) so the races serialization currently
   masks become visible on purpose instead of as local flakes.
7. **`pmacs-protocol` clippy** — add `-p pmacs-protocol` to the CI
   clippy job (local `--workspace` covers it; CI never has).

## 6. CI: speedups and efficiency

Measured baseline (run 30454133846, warm cache): wall ~14.5 min;
critical path is `Test (macos-latest/luajit)` at ~14.6 min, macos/lua54
~10 min, ubuntu test legs ~6.7 min, perf gates 2–3.5 min, everything
else ≤1.5 min.

1. **No `concurrency` group** — force-pushes and the ledger-contention
   rebase treadmill leave superseded runs burning to completion. Add
   `concurrency: {group: ci-${{ github.ref }}, cancel-in-progress:
   true}` guarded to PRs. Likely the single largest minutes saving
   given the project's rebase-heavy flow, and macOS minutes are the
   expensive ones.
2. **Split serial from parallel on the test job.** `--test-threads=1`
   serializes ~3,200 tests for the sake of a handful of
   process-lifecycle suites. Splitting into `cargo test --lib` (parallel)
   + integration targets (serial) would cut the critical-path macOS leg
   substantially; nextest (below) is the cleaner version of the same
   move.
3. **Consider cargo-nextest** for the test job: per-test process
   isolation retires the cwd/`/proc`/env hazards of §3.7 *and* enables
   safe parallelism; per-test timeouts retire the hang class at the
   runner level; automatic retries with failure reporting make the
   known load-flakes visible instead of rerun-by-hand; JUnit output
   for free. This is the one tool that converts several §3 classes
   from "fix each site" to "fixed by construction".
4. **Trim the macOS matrix if minutes matter**: the lua-flavor cfg
   audit found zero flavor-gated tests — the 2×2 matrix only varies
   mlua's backend. macOS × lua54 has caught real macOS bugs
   (`m4_5` config-sink race was lua54/macOS-shaped), so don't drop
   macOS — but macOS × one flavor + ubuntu × both retains the platform
   signal at ¾ the macOS cost. Decide from flake history, not a priori.
5. **Redundant work, minor**: `cargo build --all-targets` before
   `cargo test --all-targets` is a near-no-op (incremental) but adds
   log noise; `-p pmacs-protocol` runs identically on all 4 legs
   (comment acknowledges; it's seconds); the gpu-render apt-get runs
   every job (~20 s; cacheable via `actions/cache` on the deb pool if
   it ever grows).
6. **Perf gates on shared runners** already run loosened profiles
   (§1.4) and are the most variance-exposed jobs. If they start
   flaking, prefer trend-tracking (store the measured numbers as
   artifacts, gate on regression-vs-window) over further threshold
   loosening — a threshold loose enough to never flake certifies
   nothing.

---

## 7. What is deliberately out of scope

- Windows: no `cfg(windows)` code exists; unix-only sockets. Not a
  testing gap until the product targets it.
- `src/bin/pmacs_fake_lsp.rs` / `pmacs_fake_mcp.rs` (3,145 untested
  lines): test fixtures; testing the fakes is circular. Their fidelity
  is instead covered by §1.2's real-server tests — which is an argument
  for §5.4, not for unit-testing the fakes.
- The lean4/latex suites' stub-based design (`lake_stub`,
  vendored grammar): verified deliberate and CI-real; not a gap.

## 8. Candidate lanes for the arc (priority order)

1. **CRDT CI lane** — already scoped in `docs/active-work.md`; first
   task there is deliberate-vs-accidental classification. Blockers:
   clippy-crdt red, a37 vacuity, first-run flake budget.
2. **Silent-skip arming** (§1.2, §5.4) — install tools + `PMACS_REQUIRE_*`
   guards. Small, mechanical, converts 15 vacuous tests to real ones.
   Independent of lane 1.
3. **CI hardening/speedup** (§5.1–2, §6.1–3) — timeouts, concurrency,
   branch protection, serial/parallel split or nextest. No product code.
4. **The reader-join hang** (§3.1) — the one product fix in this list;
   retires the basedpyright skip everywhere. Already a named deferral
   in `src/process.rs`.
5. **Race-shape cleanup sweep** (§3.2–6) — port the already-invented
   fixes (non-empty guard, predicate pumps, panic-on-timeout, git env
   pinning) to the degraded copies. Mostly mechanical; each fix should
   be bite-verified, which motivates doing lane 6 first or alongside.
6. **`scripts/bite` positive control + handoff correction** (§4) —
   tiny, but raises the trust ceiling on every other lane's
   verification claims.
7. **Lua error-channel decision** (§2.4) — either define `pmacs.error`
   in production or delete the 35 dead references and standardize on
   `set_status`; then a sweep of the 56 discarded pcalls. This is a
   product-coherence question (COHERENCE.md owns the framing) as much
   as a testing one.
8. **Protocol-crate self-sufficiency** (§2.1) — move/duplicate the
   round-trip + negotiation pins into `pmacs-protocol`; add its clippy
   to CI. Mechanical but wide.
9. **Fuzz targets** (§2.5) — `read_message` + ANSI parser, run on the
   cron job (lane 3's infrastructure).
10. **Coverage floor for the untested-file list** (§2.2, §2.3) and
    **Lua interaction-island pins** (§2.4: menus, default keymap,
    window/fold wrappers) — largest and least mechanical; needs its
    own framing to avoid writing biteless tests, per the project's own
    "new tests need their own bite" history.

---

## Appendix: re-measurement commands

```sh
# Dark-test census (re-measure, never quote):
cargo test --all-targets --no-default-features --features luajit -- --list | grep -c ': test'
cargo test --all-targets --no-default-features --features luajit,crdt -- --list | grep -c ': test'

# Ignored tests that no job runs:
grep -rn '#\[ignore' src/ tests/ --include='*.rs'

# Silent-skip sites:
grep -rn 'which_binary\|not on PATH; skipping' tests/

# pmacs.error references (production never assigns it):
grep -rn 'pmacs\.error\b' builtin/ --include='*.lua'

# Discarded statement-position pcalls:
grep -rn '^\s*pcall(' builtin/ --include='*.lua'

# Per-job CI timing for a run:
gh run view <run-id> --json jobs --jq '.jobs[] | "\(.name): \(.startedAt) -> \(.completedAt)"'
```
