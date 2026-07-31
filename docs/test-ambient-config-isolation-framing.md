# Framing — integration tests use the developer's real ambient roots

**Revision 5.** Status: implemented on branch
`test-ambient-isolation-impl` (worktree `../pmacs-test-isolation-impl`,
based on `githubsucks/main` @ `54a092e`), PR #206. Revision 4 was
approved after three review rounds and merged as #201; revision 5 records
implementation findings and one **deliberate departure from §7's branch
plan**, not a new design round.

**The suite is green in CI and red on a developer machine that has a
real `~/.config/pmacs/init.lua`.** Not flaky — deterministic, and
attributed to whatever branch happens to be checked out.

**And it is not only reads.** Review round 1 falsified revision 1's
central assumption: integration tests **write into the real user data
directory**. The lane is therefore about *ambient roots*, not about
`init.lua`. See §1.6.

## Revision history

**Revision 4 → 5**, at implementation. No design changed; two things are
recorded that a later reader would otherwise have to reconstruct.

### The §7 branch plan was consciously exceeded

**§7 said: classification first and alone, "its answer belongs in review
before any mechanical edit rides on it."** The classification came back
at **342 in-process construction sites across 66 of 97 files** — large
enough that §7's shape would naturally suggest splitting the mechanical
migration into its own lane. **It was not split, and that is deliberate.**

The alternatives were both worse, and both worse in the way this lane
exists to prevent:

- **Split, migrating nothing now.** The seam would land while 65 files
  kept reading the developer's real `init.lua` and writing their real
  data root. The lane's own defect would survive its own PR.
- **Split, with a broad temporary allowlist.** Acceptance 12's ratchet
  would ship exempting ~65 files. A ratchet whose allowlist is most of
  the corpus does not ratchet; it records. And the exemption would have
  to be removed later by the same reviewer who granted it, with nothing
  failing in the meantime to remind anyone.

So the whole-corpus migration is the atomic remediation and rides this
PR. §7's *ordering* is honoured — the classification is the first commit,
before any mechanical edit — while its implied *scoping* is not. The
qualification this leaves on acceptance 12 (the allowlist is narrow
because everything else moved, not because the change was small) was
raised in the PR and accepted in review round 1.

### The exemption shape acceptance 12 needs

Review round 1 found an ambient `EditorState::new()` in an ordinary
parent test of the isolation suite itself — the exposure, committed by
the suite written to remove it. **A file-level allowlist did not catch
it**, because the file was already exempted for its re-exec'd positive
control, and a bare exemption licenses a named file to grow new ambient
sites indefinitely.

The fix is that **every exemption carries its exact site count**, so an
added site fails even inside an exempted file. That is the shape
acceptance 12 needs; "a narrow, named allowlist" is not sufficient on its
own, because narrowness constrains which files are exempt and says
nothing about how far each exemption stretches.

**Revision 3 → 4**, after review round 3 (one blocking, two major). All
three accepted.

- **The isolated gate still inherited `PMACS_STATE_HOME`** (§1.6a,
  §6). That override wins over `XDG_STATE_HOME`, so setting the four XDG
  storage variables did not isolate state on a machine that exports it.
  The contract now names the exact five variables rather than counting
  roots.
- **Q#TI5 still called self-spawn the durable regression guard** after
  revision 3 had accepted that it proves behaviour, not continued
  adoption. Q#TI5 now names both guards and their different jobs.
- **The active-work lane still said revision 2 and still prescribed
  isolated `XDG_CONFIG_HOME` alone.** Both are synchronized with this
  revision.

**Revision 2 → 3**, after review round 2 (three blocking, two major).
All five accepted; all five verified before acceptance, two of them by
running the thing rather than reading it.

- **Journey isolation had no executable mechanism** (§1.10). `cargo test
  --test journey_acceptance` launches with the caller's environment and
  the binary cannot isolate itself before its own tests run. Rev 2 said
  journey "is isolated by its launched environment" and nothing arranged
  that — which quietly assumed the external wrapper this lane exists to
  delete. Now named and pinned: per-test parent/child re-exec.
- **One self-spawning test is not a ratchet** (§4.7). It proves the seam
  works; it cannot notice a raw `EditorState::new()` added to a
  different binary next month. A checked source inventory is now an
  acceptance criterion.
- **The root list and the gates disagreed with the audit** (§1.6a, §6).
  `BootstrapRoots` covered four roots while §1.6a named six, and the
  revised gate still isolated only `XDG_CONFIG_HOME` — so the
  "isolated" gate could still write through the real data root. **Every
  local gate run in this repo today had exactly that hole.**
- **The count was one high and the ledger overstated it further**
  (§1.5). 65 files, not 66; and "all 96 test files load the real config"
  was false.
- **The recovery command did not work.** Verified by running it:
  `git worktree add <path> <remote-only-branch>` fails with `fatal:
  invalid reference`.

**Revision 1 → 2**, after review round 1 (four blocking, two major). All
six accepted; all six verified in the code before acceptance.

- **Rev 1's "the exposure is read-only" was already false** (§1.6).
  `EditorState::new` materializes bundled packages *before* config
  loading and unconditionally, and `materialize_all` creates
  directories. Confirmed on the development machine:
  `~/.local/share/pmacs/builtin-packages/v1.0.0` exists. A
  non-config-loading constructor does not fix this.
- **Rev 1's population count was wrong, and its stated method did not
  match the command that produced it** (§1.5). It reported 18 candidate
  files from a grep for `Editor::new`, while its prose described a
  broader pattern — and the real constructor is `EditorState::new`,
  which `Editor::new` does not match. **66 of 96** test files construct
  an editor directly.
- **File-level classification cannot work** (§1.5). At least **5** files
  are both in-process and spawned.
- **The seam must cover `EditorState::open`, not only `new`** (§1.7).
  `open` calls `Self::new()` directly, and `journey_acceptance` requires
  that exact public entry point on purpose.
- **Isolated construction must still finish initialization** (§1.8).
  Config loading and `set_init_complete()` share one conditional block,
  and `m8_2_acceptance` *documents* its dependence on integration-test
  construction being init-complete.
- **Rev 1 contradicted itself on the regression guard** (§2 Q#TI3 vs §5)
  and **never added its lane to `docs/active-work.md`**, which the
  repository's volatile-work protocol requires.

## 0. Coherence impact (COHERENCE §20)

- **No journey step, no user-facing behaviour.** This is test
  infrastructure.
- **Serves §9 (worker model) indirectly**: a gate that fails for reasons
  unrelated to the change under test destroys the signal the gate exists
  to give.
- **Interaction islands: none. Config registry: not adopted.
  Background-work attribution: unchanged.**
- **No audited claim in COHERENCE.md changes**; under §25 no COHERENCE
  edit rides this PR.


## 1. Ground truth (verified at `4cd4a7b`)

### 1.1 The observation

On 2026-07-30, `cargo test --all-targets --no-default-features --features
lua54` on a developer machine failed **11 of 67** in
`compile_mode_acceptance`, every one with:

```
[@/home/jeans/.config/pmacs/init.lua] command "find-file" is already
defined (refusing to overwrite)
stack traceback:
	[C]: in field 'define'
	/home/jeans/.config/pmacs/init.lua:3: in main chunk
```

Failing tests: `acc14`, `acc24`, `acc25a`, `acc27`, `acc29`, `r1f2`,
`r1f5`, `r2f1`, `r3f1`, `r4f1` (×2). With `XDG_CONFIG_HOME` pointed at an
empty directory the same suite is **67/67**.

### 1.2 The mechanism, and why the existing guard misses

`src/editor.rs:770` guards user-config loading:

```rust
#[cfg(not(test))]
{
    crate::config::load_user_config(&mut lua_host);
    lua_host.set_init_complete();
```

with a comment stating the intent exactly: *"Skipped under `cfg(test)` so
the lib's own test suite doesn't pick up the developer's real
`~/.config/pmacs/init.lua` and turn into a flaky environment-dependent
run."*

**`cfg(test)` is set only when compiling the crate's own unit tests.**
An integration test in `tests/` links `pmacs` as an ordinary dependency,
compiled *without* `cfg(test)` — so the guard is inactive for every one
of them. `cargo test --lib` is protected; `cargo test --test <name>` is
not.

The hazard was identified, a mitigation was written, and its scope does
not match the threat. That is the finding — not that nobody thought
about it.

### 1.3 There are two populations, and they need different fixes

- **Spawned.** The test launches `pmacs --daemon` as a child process.
  Isolation works by setting the child's environment.
- **In-process.** The test constructs `pmacs::editor::EditorState`
  directly in the test binary. `compile_mode_acceptance` is this kind
  (`tests/compile_mode_acceptance.rs:14`).

**The spawned case is already solved, once.** `tests/m5_7_acceptance.rs:132`:

```rust
fn spawn_pmacs_daemon(socket_path: &Path) -> Child {
    // Isolate user config: HOME and XDG_CONFIG_HOME both point at the
    // (currently empty) socket parent directory, so the daemon won't
    // try to read the developer's real `init.lua`.
    ...
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
```

Both variables, not just one — `config_dir` falls back from
`XDG_CONFIG_HOME` to `$HOME/.config`, so setting one alone leaves the
other path live.

### 1.4 In-process tests cannot isolate themselves

The obvious fix — set the variable in test setup — **is unavailable**.
`std::env::set_var` is `unsafe` in the 2024 edition and the crate is
`#![forbid(unsafe_code)]`.

This is already established in the codebase rather than inferred:
`src/packages/installer.rs:368` carries a `root_override` field
explicitly *because* of it — *"the project forbids `unsafe_code`, so
mutating `XDG_DATA_HOME` directly is not an option"*.

So an in-process test can only be isolated by (a) the environment it is
launched with, or (b) an injection point in the code under test. There
is precedent for (b) in the same repository.

### 1.5 Scale, measured

96 files in `tests/`. **65** contain an actual `EditorState::new()` or
`EditorState::open(` **call**. **5** are *both* in-process and spawned —
`vterm_stage3_acceptance` constructs an editor at `:159` and spawns a
daemon at `:665`.

**66 files match the bare name; 65 call it.** The 66th is
`tests/m5_6_acceptance.rs`, which mentions `EditorState::new` only to
say it deliberately does *not* use it (`:94`): *"Tests can't go through
`EditorState::new` here because the integration-test build doesn't have
`cfg(test)` set on the lib."* That makes it the **third** place in the
suite documenting the §1.2 gap — after `m8_2_acceptance` and the
`src/editor.rs` comment itself. A grep for a name counts mentions; only
reading counts calls.

**Revision 1 said 18, and its stated method did not match the command
that produced it.** The prose named a broad pattern; the command grepped
`Editor::new`, which does not match `EditorState::new` — the actual
constructor. So the number was ~4x low *and* described a different
search than the one run. Rev 1 hedged the number as "a candidate count,
not a census" while leaving both the figure and the description wrong;
a disclaimer on a bad measurement does not make it a good one.

**Consequence for the design:** classification must be per *construction
site* or per *test case*, never per file. A mutually exclusive file
partition cannot represent the 5 mixed files, and acceptance 1 of
revision 1 required exactly that partition.

### 1.6 The exposure is NOT read-only — this is established

`EditorState::new` materializes bundled packages **before** config
loading and **unconditionally** — outside any `cfg` guard
(`src/editor.rs:730`):

```rust
let bundled_root = crate::builtin_packages::bundled_runtime_dir();
let bundled_packages = crate::builtin_packages::materialize_all(&bundled_root)
    .expect("materialize bundled packages");
```

`bundled_runtime_dir()` resolves `XDG_DATA_HOME`, else `$HOME/.local/share`
(`src/builtin_packages.rs:142,148`), and `materialize_all` **creates
directories and writes package files** (`:174`).

**Confirmed on the development machine:**
`~/.local/share/pmacs/builtin-packages/` exists containing `v0.1.0` and
`v1.0.0`. Whatever produced those, the write path is live and reachable
from every in-process test, none of which override `HOME` or
`XDG_DATA_HOME` at all.

So revision 1's Bet 2 was not a question to investigate — it was already
answered, in the direction that matters. This upgrades the lane from
"local gates lie" to **"tests write into real user data"**.

### 1.6a Ambient variables, and the harness's incomplete storage coverage

`src/` reads: `HOME` (9 sites), `XDG_DATA_HOME`, `XDG_CONFIG_HOME`,
`XDG_STATE_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR`, and
`PMACS_STATE_HOME`.

The four bootstrap storage roots resolve through **five variables**:
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`,
`XDG_CACHE_HOME`, and `PMACS_STATE_HOME`. The fifth is not redundant:
`PMACS_STATE_HOME` wins over `XDG_STATE_HOME`
(`src/state.rs:47-68`), so redirecting the latter while inheriting the
former leaves the real state root live.

The shared harness `spawn_daemon_process_with_env`
(`tests/common/daemon.rs:154`) sets **`HOME` and `XDG_CONFIG_HOME`
only**. A spawned daemon therefore inherits the developer's real
`XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` and
`PMACS_STATE_HOME`.

**Setting `HOME` isolates a root only when the corresponding `XDG_*`
variable is unset**, because `HOME` is the *fallback*. On this machine
`XDG_DATA_HOME` happens to be unset, so `HOME` does cover it — which
means the harness's apparent adequacy is a property of one developer's
environment, not of the harness. On a machine that exports
`XDG_DATA_HOME`, spawned daemons write to the real one.

### 1.6b Scope: bootstrap STORAGE roots only

`BootstrapRoots` covers **config, data, state and cache** — the roots
that decide *where pmacs stores things at startup*. It does **not**
cover:

- **`HOME`'s non-storage semantics.** `expand_tilde`
  (`src/editor_core.rs:5457`) resolves a leading `~` for ordinary path
  entry, and `find_file_acceptance.rs:344` consumes `HOME` on purpose to
  pin that expansion (skipping when unset). Blanket-overriding `HOME`
  would silently retarget a user-facing path feature and its test.
- **`XDG_RUNTIME_DIR`**, which addresses sockets rather than stored
  data.

**Rev 2 listed six roots and proposed covering four without saying so.**
The two exclusions are deliberate and named here so the gap is a
decision rather than an oversight. A later lane may take `HOME`
semantics; this one must not, because the fix for a storage root
(redirect it) is the wrong fix for a path-expansion root (leave it and
isolate the *file* instead).

### 1.10 Journey needs a mechanism, not an intention

Rev 2 said `journey_acceptance` keeps the ambient `EditorState::open`
and "is isolated by the environment its binary is launched with". That
is not a mechanism. **Cargo launches each integration-test binary with
the caller's environment**, and a binary cannot re-point its own roots
before its ordinary tests run — §1.4's `set_var` prohibition applies to
itself. So under a plain `cargo test --test journey_acceptance` the
suite is ambient, and the only thing that made rev 2's sentence true was
an external environment wrapper — the very workaround this lane exists
to delete.

**The mechanism, named:** each journey test becomes a thin parent that
re-execs `std::env::current_exe()` with `--exact <test name>`, a marker
variable, and controlled roots; the child, seeing the marker, runs the
real body and calls the **ambient** `EditorState::open`.

That is the only shape found that satisfies both constraints at once:
the child drives the true production entry point, so
`journey_acceptance`'s ratchet discipline is untouched, while the
child's roots are controlled, so nothing reaches the developer's. The
same shape serves Bet 3's hostile-environment check, so it is one
helper, not two.

### 1.7 The seam must cover `open`, not only `new`

`EditorState::open` calls `Self::new()` directly (`src/editor.rs:944`),
so a non-loading *constructor* leaves every open-path test unisolated.

It cannot simply be bypassed. `tests/journey_acceptance.rs:16` requires
that exact public entry point, and says why: *"A directory arm with no
production caller passes every direct-call test, so step 3 goes through
`EditorState::open` — the same function `pmacs FILE` calls."* The
golden-journey ratchet and the isolation seam pull in opposite
directions, and the framing must resolve it rather than pick one.

### 1.8 Isolated construction must still finish initialization

Config loading and `set_init_complete()` share **one** conditional block
(`src/editor.rs:769-773`). Factoring by skipping the block would leave
every integration test permanently in the init phase, changing package
APIs and startup-only config behaviour.

**This is not hypothetical — it is already depended upon.**
`tests/m8_2_acceptance.rs:75` documents it:

> `EditorState::new()` sets the init-complete flag during startup (the
> integration-test build doesn't get the `cfg(test)` guard that lib
> tests do), so we reopen the init phase before `install_local`.

So the `cfg(test)` gap §1.2 calls a defect is, in this one respect,
load-bearing behaviour another suite was written against. Any fix must
skip **ambient reads** while still returning `is_init_complete() == true`.

### 1.9 What is still NOT established

- **Whether CI is genuinely unaffected**, as opposed to merely having no
  config and no prior data dir today. A CI image that grows either would
  break the same way, silently.
- **Whether the 11 failures are the whole blast radius.** The run
  aborted at the first failing binary, so every suite ordered after
  `compile_mode_acceptance` never executed.
- **What wrote `v0.1.0` and `v1.0.0`** on the development machine — a
  test run, or ordinary use of pmacs. The write *path* is proven; the
  provenance of those two directories is not, and this lane does not
  claim it.


## 2. Questions

- **Q#TI1** — Widen the `cfg(test)` guard, or make isolation the
  harness's job? *Proposed: neither alone. The guard must not widen
  (production deciding it is under test is how a test passes against
  behaviour production never runs), and §1.4 shows the harness cannot
  set env in-process. The answer is an explicit **bootstrap-roots
  parameter** threaded through construction.*
- **Q#TI2** — What is the seam, exactly? *Proposed: a
  `BootstrapRoots` value naming the config root and the data/state/cache
  roots, with a `BootstrapRoots::ambient()` used by production and a
  test constructor taking an explicit one. It must cover **both**
  `EditorState::new` and `EditorState::open` (§1.7), following
  `Installer::root_override`'s precedent (§1.4).*
- **Q#TI3** — How does `open`-path isolation coexist with the
  golden-journey ratchet? *Proposed: `open` gains a roots-taking sibling
  and `journey_acceptance` keeps calling the ambient `open`, because its
  purpose is to prove the production entry point is wired. Journey is
  then isolated by the **environment its binary is launched with**, not
  by a different call — which is the only option that preserves what the
  ratchet exists to prove.*
- **Q#TI4** — Must isolated construction remain init-complete? **Yes**
  (§1.8), and it is a criterion rather than an assumption.
- **Q#TI5** — What are the durable regression guards? *Proposed: two
  guards with different jobs. A **test-binary self-spawn under a
  controlled environment** is the behavioural proof: the isolated seam
  ignores hostile ambient roots and leaves them unmodified. A **checked
  source inventory** is the adoption proof: a raw ambient constructor
  added to another test binary fails the suite. Revision 1 proposed a
  hostile-config CI leg in Q#TI3 and parked that same mechanism in §5 —
  a contradiction; self-spawn replaces that workflow leg, while the
  inventory prevents migration drift.*


## 3. Bets

- **Bet 1 — the population is classifiable by construction site.**
  Every `EditorState::new`/`open` call site in `tests/`, and every
  daemon spawn, is classified. Files are not the unit: 5 are both
  (§1.5).
  - *Falsified if* sites are reached through helpers that obscure which
    kind they are. Then the helper is the unit and that is stated.

- **Bet 2 — every bootstrap storage root can be redirected without
  `unsafe`.**
  A `BootstrapRoots` parameter covers config, data, state and cache; the
  spawned harness controls all five storage variables of §1.6a.
  - *Falsified if* any root is resolved somewhere that cannot accept the
    parameter — e.g. behind a `OnceLock` initialised before construction.
    **That is a real risk and is checked first**, because it decides
    whether this design is possible at all.

- **Bet 3 — isolation is provable by a hostile-environment test.** A
  test spawns the test binary itself with `HOME`/`XDG_*` pointing at a
  directory containing an `init.lua` that would break a known assertion,
  and a pre-seeded data dir. The suite stays green, and the hostile
  directory is **unmodified afterwards**.
  - *Falsified if* the child cannot be given a controlled environment
    without the in-process `set_var` §1.4 rules out. (It can: `Command`
    takes `.env`. This bet is cheap and its failure would be
    informative.)
  - **The unmodified-afterwards half is the write half of the
    check** — a green suite that still wrote into the hostile root has
    not demonstrated isolation.

- **Bet 4 — the fix does not change production behaviour.** Ambient
  resolution stays the default; isolated construction still returns
  `is_init_complete() == true` (§1.8).
  - *Falsified if* any production call site changes, or if
    `m8_2_acceptance`'s `reopen_init_phase_for_testing` dance stops
    working.


## 4. Acceptance

1. A classification of every editor-construction site and daemon spawn
   in `tests/`, by **site**, with mixed files represented explicitly and
   counts stated. Method named (read, not grepped) — §1.5 is what a
   grep-shaped answer costs.
2. A `BootstrapRoots`-style parameter covering config, data, state and
   cache roots, reachable from **both** `EditorState::new` and
   `EditorState::open` (§1.7).
3. Isolated construction returns `is_init_complete() == true`, pinned by
   a test that fails if the init flip is skipped along with the ambient
   reads (§1.8).
4. `journey_acceptance` still drives the ambient `EditorState::open`;
   its isolation comes from the launched environment. The ratchet's
   production-entry-point discipline is unweakened, and this is asserted
   rather than asserted-to-be-obvious.
5. The shared harness `spawn_daemon_process_with_env` controls all
   **five storage variables** of §1.6a:
   `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`,
   `XDG_CACHE_HOME`, and `PMACS_STATE_HOME`.
6. `compile_mode_acceptance` passes with a real
   `~/.config/pmacs/init.lua` defining `find-file` present — the exact
   condition of §1.1.
7. A hostile-environment self-spawn test (Bet 3) that asserts both
   green **and** an unmodified hostile root.
8. `cfg(test)`-only guards are not widened (Q#TI1).
9. The `src/editor.rs:770` comment is corrected: it claims a protection
   it does not provide for integration tests, and says nothing about the
   unconditional package materialization above it.
10. This lane is recorded in `docs/active-work.md` with its branch and
    worktree, per the volatile-work protocol — revision 1 omitted it.
11. README or handoff records that a local full-suite run needs all
    **five storage variables** controlled until this lands:
    `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`,
    `XDG_CACHE_HOME`, and `PMACS_STATE_HOME`. Naming only the four XDG
    variables leaves a higher-precedence state override live.
12. **A durable adoption ratchet**, not a one-time census: a checked
    source inventory that fails when a new ambient
    `EditorState::new()`/`open(` appears in `tests/` outside a narrow,
    named allowlist. Acceptance 1 proves today's state; this keeps it.
    Falsified by adding an ambient constructor to any suite and
    observing the inventory test fail.
13. The journey mechanism of §1.10 is implemented and pinned: a plain
    `cargo test --test journey_acceptance`, with a hostile `init.lua`
    present and no external wrapper, is green and leaves the hostile
    root unmodified.


## 5. Parked

- **Any change to how production resolves ambient roots.** The default
  stays ambient; only construction gains a parameter.
- **A hostile-config CI leg.** Superseded by Q#TI5's self-spawn, which
  is strictly stronger — it travels with the test rather than with the
  workflow file. Recorded because revision 1 both proposed and parked
  it.
- **Cleaning up whatever already wrote into
  `~/.local/share/pmacs/builtin-packages/`.** Out of scope; this lane
  stops the writes, it does not audit the past.
- **The `crdt` half of the corpus being dark in CI** — separate lane.


## 6. Gates

Standard suite, each its own step with a real exit status and nothing
after the command that could mask it.

**Isolate every storage root, not just the config one.** Rev 2's gate
set only `XDG_CONFIG_HOME`, which stops the `init.lua` reads and leaves
the data root live — so an "isolated" run could still write to
`~/.local/share/pmacs`. **Every local gate run in this repository today
had that hole.** The correct invocation sets, to a fresh directory:

```
XDG_CONFIG_HOME  XDG_DATA_HOME  XDG_STATE_HOME  XDG_CACHE_HOME
PMACS_STATE_HOME
```

`PMACS_STATE_HOME` must be redirected or explicitly removed; it takes
precedence over `XDG_STATE_HOME`. `HOME` is deliberately left alone
(§1.6b).

**Run the suite twice: once isolated, once with a hostile environment**
— an `init.lua` that would break a known assertion, plus a pre-seeded
data root — and assert the hostile root is **byte-identical
afterwards**. A lane about ambient state verified only in a clean
environment has not been verified.

## 7. Branch plan

One branch, one PR. Bet 1's classification first and alone — it decides
how large the change is, and its answer belongs in review before any
mechanical edit rides on it.
