# Framing — integration tests read the developer's real config

**Revision 1.** Status: awaiting review round 1. Proposed lane:
`test-ambient-config-isolation`, worktree `../pmacs-test-isolation`,
based on `githubsucks/main` @ `4cd4a7b` (a reading; re-measure at branch
time).

**The suite is green in CI and red on a developer machine that has a
real `~/.config/pmacs/init.lua`.** Not flaky — deterministic, and
attributed to whatever branch happens to be checked out.

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

### 1.5 Scale

96 files in `tests/`. **18** reference `EditorState`, `Editor::new`,
`load_user_config`, or `TestDaemon` and are therefore candidates. Only a
handful set any isolating variable today.

**The 18 is a candidate count from a name-based grep, not a census.**
It is an upper bound on nothing and a lower bound on nothing; §3 Bet 1
replaces it with a real classification. Recorded this way deliberately —
a previous lane in this repo built a classification from a truncated
grep and misclassified five rows.

### 1.6 What is NOT established

- **Whether any test *writes* into the developer's real directories.**
  `XDG_DATA_HOME` and `XDG_STATE_HOME` back autosave
  (`src/autosave.rs`), minibuffer history (`src/minibuffer.rs:724`),
  builtin packages (`src/builtin_packages.rs:142`) and the package
  installer (`src/packages/installer.rs:96`) — all with `HOME`
  fallbacks. Read-only pollution is a failed gate; *write* pollution
  would touch real user data. **No such write has been observed**, and
  this lane does not claim one. Bet 2 goes looking, because the cost of
  being wrong is asymmetric.
- **Whether CI is genuinely unaffected**, as opposed to merely having no
  config today. A CI image that ever grows a `$HOME/.config/pmacs` would
  break the same way, silently.
- **Whether the 11 failures are the whole blast radius.** The run
  aborted at the first failing binary, so every suite ordered after
  `compile_mode_acceptance` never executed.


## 2. Questions

- **Q#TI1** — Should the `cfg(test)` guard be widened, or should
  isolation be the test harness's job? *Proposed: the harness's.
  Widening the guard means production code deciding it is under test,
  which is exactly the shape that lets a test pass against behaviour
  production never runs.*
- **Q#TI2** — For in-process tests, injection point or launched
  environment? *Proposed: an explicit constructor that does not load
  user config, following `Installer::root_override`'s precedent. A
  wrapper script that sets the variable fixes the symptom for whoever
  remembers to use it.*
- **Q#TI3** — Should CI arm a check that the isolation is real?
  *Proposed: yes — otherwise this recurs the moment a CI image grows a
  config file, and recurs invisibly.*
- **Q#TI4** — Does anything write outside its temp dir? *Unknown; Bet 2.*


## 3. Bets

- **Bet 1 — the population is classifiable.** Every file in `tests/`
  is classified as spawned, in-process, or neither, by reading each
  candidate's construction site rather than by grepping for a name.
  - *Falsified if* a file is both, or constructs the editor indirectly
    through a helper that hides which it is. Then the classification is
    reported with that ambiguity rather than forced.

- **Bet 2 — the exposure is read-only.** A test run under an
  instrumented `HOME`/`XDG_*` pointing at a fresh directory leaves no
  writes behind.
  - *Falsified if* anything appears there — which upgrades this lane's
    priority sharply, from "gates lie locally" to "tests touch real user
    data".
  - **A clean result is evidence about the suites that ran**, not a
    guarantee; the run must be recorded with which binaries executed.

- **Bet 3 — isolation is verifiable by a test that fails without it.**
  A positive control: a fixture writes an `init.lua` that would break a
  known assertion, and the suite stays green because the isolation holds.
  - *Falsified if* no such fixture can be built without the very env
    mutation §1.4 rules out. Then isolation is asserted structurally
    (no candidate constructs an editor without the non-loading path) and
    labelled as the weaker check it is.

- **Bet 4 — the fix does not change production behaviour.** The
  non-loading constructor is additive; the existing one is untouched.
  - *Falsified if* any production call site has to change.


## 4. Acceptance

1. A classification of every `tests/*.rs` file into spawned /
   in-process / neither, with counts stated and the method named
   (read, not grepped).
2. In-process tests construct the editor through a path that does not
   load user config, by explicit choice at the call site rather than by
   a `cfg` the caller cannot see.
3. Spawned tests set **both** `HOME` and `XDG_CONFIG_HOME`, following
   `m5_7_acceptance.rs:132`. Any that set only one are fixed.
4. `compile_mode_acceptance` passes with a real
   `~/.config/pmacs/init.lua` present that defines `find-file` — the
   exact condition that produced §1.1.
5. Bet 2's write-probe result recorded, listing which binaries ran.
6. `cfg(test)`-only guards are not widened into a
   production-decides-it-is-under-test shape (Q#TI1).
7. The `src/editor.rs:770` comment is corrected: it claims a protection
   it does not provide for integration tests.
8. README or the handoff records that a local full-suite run needs an
   isolated `XDG_CONFIG_HOME` until this lands, so the next person does
   not spend the afternoon I did attributing 11 failures to their branch.


## 5. Parked

- **Any change to how production resolves config paths.** Out of scope;
  the defect is in the tests.
- **The `crdt` half of the corpus being dark in CI** — a different
  coverage hole with its own lane in `docs/active-work.md`.
- **Whether CI should install a hostile `init.lua` deliberately** to
  keep this honest. Attractive, but it is a CI-policy decision and this
  lane is already load-bearing enough.


## 6. Gates

Standard suite, each its own step with a real exit status and nothing
after the command that could mask it. **Run twice: once with an isolated
`XDG_CONFIG_HOME`, and once with a deliberately hostile `init.lua` in
place.** A lane about ambient state that is only ever verified in a clean
environment has not been verified at all.


## 7. Branch plan

One branch, one PR. Bet 1's classification first and alone — it decides
how large the change is, and its answer belongs in review before any
mechanical edit rides on it.
