# `scripts/gate` — per-worktree build isolation, and one gate suite

**Status: revision 6 — AWAITING APPROVAL.** Revision 6 extends the
isolation contract to `TMPDIR` (§2a below) and is the only unapproved
part of this document; everything else is as approved. It is a
*widening of an existing responsibility*, not a new feature: §2 already
owns "what the gate isolates", and `TMPDIR` was simply missing from
that list — which is how a stray `/tmp/.git` came to redden a gate run
on an unrelated lane.

**Previously, revision 5. Approved at revision 4 and IMPLEMENTED; revision
5 records two safety defects review found in the implementation.**

**Neither was a design gap — both were the implementation failing to
honour this document, which is worth stating because it is the case a
framing doc cannot prevent by itself:**

- **`--prune` could delete every managed directory.** §2.6 requires
  liveness to be *established*; the code masked
  `git worktree list` failure with `|| true`, so running from outside
  any repository produced an **empty** live set — under which every
  marked directory is an orphan and `--prune --force` deletes all of
  them, live lanes included. Now: two refusals (not in a worktree; the
  enumeration failed), and an outside-repo survivor test.
- **`--acceptance` was shell-injectable.** §2.3 hands the name into a
  command the runner evaluates; nothing validated it, so
  `--acceptance 'x; rm -rf ~'` would have executed. Now: an allowlist
  of what a cargo target can be named, refused at parse time, with a
  metacharacter rejection test and a canary.

Also fixed: log directories now carry the PID as well as a
whole-second timestamp (two runs in one second shared a directory and
could overwrite each other's evidence — reintroducing U2/U3 through a
naming choice); the ownership marker is enforced as exactly one line
rather than read head-first; and the `prunable` test fails loudly
instead of returning green when `git worktree add` fails, with cleanup
through a `Drop` guard that survives a panicking assertion.

Developer tooling. **Not coherence-affecting** — it touches no journey
step, adds no interaction island, adopts no config-registry setting, and
creates no background work (`COHERENCE.md` §20). It is named here so the
absence is a statement rather than an omission.

**Revision 2** answered five review findings: the managed root and
ownership marker Q#GS1 needed, the narrowing of Q#GS2 (the script
cannot infer touched acceptance suites), Q#GS4's dry-run and
eligibility rules, the corrected — conditional — CRDT workspace sweep,
and the test-root override.

**Revision 4** makes the log runner `set -e`-safe (the earlier
`rc=$?` form never executes under `set -eu`, which `scripts/bite`
already uses), corrects a stale justification — ambient-root isolation
**merged as #206**, so the five variables are belt-and-braces for
external and integration paths rather than cover for a missing lane —
and extends the canonical-path rule to **directory derivation**, which
§2.5 had omitted while §4's symlink test depended on it.

**Revision 3 answered three findings, two of which were regressions I
introduced.** Rewriting §2 for revision 2 **silently dropped two of the
original core responsibilities**: ambient-root isolation and durable
sweep logs. The second of those *is* the U2/U3 remedy, so losing it
would have left this document proposing a fix for a problem it had
stopped solving. Both are restored as §2.2, with the exit-status
hazard in log capture specified rather than left to the implementation.
Prune liveness is made precise in §2.6 — `prunable` entries and one
canonical path representation — and §4's marker claim is reconciled
with the interface by `--init`.

---

## 1. Why this exists

### 1.1 The measured problem

Parallel worktrees are about to become the working method. They do not
work on this machine today, and the reason is one line of shell config:

```
set -gx CARGO_TARGET_DIR $HOME/build/cargo-target   # fish config.fish:55
```

Every worktree therefore builds into **one** directory, and **cargo
takes an exclusive lock on it**. Two lanes building concurrently do not
run in parallel — the second blocks — and they invalidate each other's
artifacts, so alternating between lanes recompiles from scratch each
time. Parallel worktree development under this arrangement is *slower
than serial*.

### 1.2 The measurement that corrected the first plan

The shared directory was **285G** (`debug/deps` 222G across 12,084
files; `debug/incremental` 52G). That number drove an initial proposal
to add `sccache` so per-worktree directories would not lose artifact
sharing.

**The number was misleading and the proposal was wrong.** 285G is years
of accumulation across *two* projects — pmacs and levcs share that
directory. Measured directly, on a cleaned tree:

| | wall | size |
|---|---|---|
| `cargo build --workspace`, cold | 55s | 3.4G |
| **`cargo test --workspace --no-run`, cold** | **80s** | **19G** |

And sccache, measured across two target directories:

| | hit rate |
|---|---|
| C/C++ | 50.00% |
| **Rust** | **0.00%** |

Rust rlibs embed their target-dir path in metadata, so dependency
artifacts are not bit-identical between directories; `--extern` content
hashes differ and misses cascade. sccache does not deliver
cross-worktree Rust reuse without `--remap-path-prefix`, which would
degrade backtrace paths for every project on the machine.

**So the sharing that per-worktree directories cost is worth 80 seconds
and 19G per lane.** Four lanes is 76G against 604G free. There is
nothing to buy back. sccache stays configured — it is cheap and earns
its keep on the C/C++ dependencies — but it is **not** what makes
parallel lanes work, and this document exists partly so that claim is
not repeated.

### 1.3 The second problem, which shares a solution

The gate suite in `docs/agent-handoff.md` §3 is retyped by hand every
time. It has been gotten wrong repeatedly, **including twice in the
session that motivated this script**:

- A remediation sweep used `--tests` instead of `--workspace`, silently
  dropping `pmacs_protocol` and `pmacs_gpu` — including protocol tests
  that same lane had just written. §3 now carries a warning about
  exactly this.
- A full sweep was piped through `grep`, discarding the failure output.
  An intermittent red was then unmatchable against
  `docs/ci-red-signatures.md`, because a row needs its fragments. This
  produced registry note **U2**, and then **U3** when it happened again
  a second time in the same session.

Both are the same failure: **a procedure that lives only in prose gets
executed differently each time.** A script that sets the target
directory must already know how to run the suite, so it should own the
part of it that is fixed.

---

## 2. Design

`scripts/gate` — POSIX `sh`, matching `scripts/bite`'s shape: heavy
header comment explaining the reasoning, distinct exit codes, failure
output that says what to do next.

### 2.1 The managed target root (Q#GS1, Q#GS4)

Everything the script creates lives under **one root it owns**:

```
$HOME/build/pmacs-gate-targets/<basename>-<8 hex of CANONICAL worktree path>/
```

Both parts derive from the **canonical physical path** of §2.5 — not
from `$PWD`, which preserves whatever symlinked spelling the caller
happened to use. Two spellings of one worktree must produce **one**
directory, or the same lane silently builds into two and the isolation
buys nothing while costing double.

Deliberately **not** `$HOME/build/` directly: that holds
`cargo-target`, `go`, and `cargo`, none of which this script may reason
about. A dedicated root means `--prune` never has to decide whether an
unfamiliar sibling is fair game.

Each managed directory carries an **ownership marker** at its top level:

```
.pmacs-gate-target        # one line: the absolute worktree path it serves
```

The marker is what makes deletion safe. It is written at creation, and
**a directory without a well-formed marker is never touched**, whatever
its name. Discovery is: *direct children of the managed root, that are
directories, that contain a readable `.pmacs-gate-target` whose single
line is an absolute path.* Nothing recursive, nothing outside the root,
no name-pattern matching.

`PMACS_GATE_TARGET_ROOT` overrides the root. It exists for the test
harness (§4) and is documented as test-only; the behavior tests must
never be able to reach the real root.

### 2.2 Ambient roots and durable logs

Two responsibilities that revision 2's rewrite dropped. They are core,
not incidental.

**All five ambient roots, fresh per invocation.** Every gate command
runs with

```
XDG_CONFIG_HOME  XDG_DATA_HOME  XDG_STATE_HOME  XDG_CACHE_HOME  PMACS_STATE_HOME
```

all pointed at one directory created fresh for that run. **The fifth is
not redundant**: `PMACS_STATE_HOME` outranks `XDG_STATE_HOME`
(`src/state.rs`), so redirecting only the four XDG variables leaves the
real state root live on a machine that exports it.

**This is belt-and-braces, not a workaround for a missing lane.** The
ambient-root isolation implementation merged as **#206**; the in-crate
paths resolve roots explicitly and no longer depend on the caller's
environment. What the five variables cover is everything *outside* that
guarantee — integration suites that spawn the real binary, PTY and
daemon fixtures, and anything reaching a production resolution path —
where the process under test reads the environment it was handed. A
gate runner that scribbles in the developer's real config or data root
is a bad failure mode whether or not the crate promises not to, and
setting five variables is cheap insurance against it.

`HOME` is deliberately left alone, matching the existing guidance.

The directory is created under the managed target dir and **removed on
exit**, including on failure. Diagnosis comes from the logs below, not
from a retained config tree, and un-reaped ambient directories would
accumulate silently.

**Every gate's full output goes to a durable log.** Per invocation:

```
<target-dir>/gate-logs/<timestamp>/<NN>-<gate-name>.log
```

retained after the run, and the **workspace sweep log paths are printed
prominently** whether the run passes or fails. This is the direct U2/U3
remedy: both notes exist because a sweep's output was filtered through
`grep` before anyone read it, so an intermittent red could not be
matched against `docs/ci-red-signatures.md` — a row needs its exact
fragments, and they were gone. A log on disk cannot be filtered away by
the person reading it.

**The capture must preserve the gate's own exit status, and the obvious
way does not.** In POSIX `sh`, `cmd | tee log` reports **tee's** status,
not `cmd`'s — so a failing gate whose output was teed exits 0 and the
suite reports green. `set -o pipefail` is not POSIX (`dash` lacks it)
and this script targets `sh`.

The specified form is therefore **redirection, not a pipeline** — and
it must also survive `set -e`, which `scripts/bite` and
`scripts/feature-census` both use (`set -eu`) and this script will too.
**The naive form is doubly wrong**: `cmd > "$log" 2>&1; rc=$?` never
reaches the `rc=` assignment under `set -e`, because the shell exits on
the failing command — so the runner dies without printing which gate
failed or where its log is, which is the entire point of capturing it.

A failing command is only exempt from `set -e` when it is the condition
of an `if`, so the runner is written as one:

```sh
if cargo test --workspace --no-fail-fast -- --skip basedpyright > "$log" 2>&1
then rc=0
else rc=$?
fi
```

On a non-zero `rc` the runner prints **the failed gate's name and its
log path** and then exits non-zero itself. No pipeline exists, so no
status is lost to `tee`; no bare failing command exists, so no status is
lost to `set -e`.

The cost is that output is not live; that is the right trade for a gate
suite whose failures are read afterwards, and it is exactly how the
sweeps were run successfully in the session that motivated this
script.

### 2.3 Interface

```
scripts/gate [--acceptance SUITE]... [--protocol] [--print-plan]
scripts/gate --print-target-dir
scripts/gate --init
scripts/gate --prune [--force]
```

- **`--acceptance SUITE`** (repeatable) — the touched acceptance
  suites. **This is the Q#GS2 seam**: a script cannot infer from a
  working tree which suites a change touches, and guessing would be
  worse than asking, because a wrong guess reads as coverage. §3 stays
  authoritative for *choosing* them; the script only runs what it is
  handed, and **prints the list it ran** so a PR can quote it.
- **`--protocol`** — the change touches `PROTOCOL_VERSION`. Adds the
  CRDT *workspace* sweep (§2.4).
- **`--print-plan`** — print the exact commands and exit without
  running them. This is what makes drift testable (§4).
- **`--print-target-dir`** — print the derived directory and exit.
  **Pure**: it creates nothing, so it can be called freely.
- **`--init`** — create the managed directory and write its ownership
  marker, print the path, exit. **Runs no gates.** The gate path calls
  the same internal routine, so this is not a second implementation.

  It exists because §4's verification needs a *mutating* path it can
  drive safely: "the marker is written" cannot be tested through a
  pure printer, and testing it through a real gate run would execute
  the whole suite inside the suite. It is also genuinely useful — a
  worktree can be prepared before any work starts.
- **`--prune [--force]`** — see §2.6.

### 2.4 The gate policy the script encodes

Revision 1 said "both feature configurations", which **misstated §3**.
The corrected encoding, fixed portion first:

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings      # its own step
cargo test --lib
cargo test --lib --features crdt
cargo test --test <each --acceptance suite>
cargo test --test m4_acceptance -- --skip basedpyright
PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu
cargo test --workspace --no-fail-fast -- --skip basedpyright
git diff --check
```

and **only with `--protocol`**, per §3's "touching `PROTOCOL_VERSION`
STRENGTHENS the sweep line; it does not replace it":

```
cargo test --workspace --features crdt --no-fail-fast -- --skip basedpyright
```

**One strengthening is proposed rather than assumed.** §3 writes the
normal sweep without `--no-fail-fast` and adds it only in the
protocol-bump form. I propose `--no-fail-fast` **always**, and the
justification is that §3's own stated reason is not specific to
bumps — *"cargo stops at the first failing target: without it a bump
that breaks eight assertions reports one."* A lane that breaks three
unrelated targets has the same problem.

**The cost is bounded and paid only on red.** A green sweep is
byte-for-byte the same work; a red one continues instead of stopping,
so it costs the remaining targets' runtime exactly when the complete
picture is most wanted. That is the same diagnosability argument that
produced U2 and U3, and it is cheap here. If the added red-run time is
judged not worth it, dropping this reverts one flag and nothing else.

The CRDT **library** tests (`--lib --features crdt`) stay
unconditional — they always were.

### 2.5 One canonical path representation

Marker creation and prune comparison **must** use the same spelling of
a path, or a live worktree can be pruned. `/home/jeans/Repos/...` and a
symlinked route to the same directory are the same worktree and
different strings; git reports the path as it was registered, which
need not match how the marker was written.

The canonical form is the **physical absolute path** — symlinks
resolved, `pwd -P` semantics:

```sh
canon() { ( cd "$1" 2>/dev/null && pwd -P ) }
```

Applied at **all three** points, which is the correction revision 3
needed: **directory derivation** (§2.1) hashes `canon` of the worktree,
the marker stores `canon` of the worktree at creation, and every path
from `git worktree list` is passed through `canon` before comparison.

Naming only the last two, as an earlier draft did, leaves the hash
computed from an uncanonicalized `$PWD` — and then a symlinked
invocation derives a *different* directory whose marker records the
*canonical* path. The result is a second target directory for a live
worktree, indistinguishable from an orphan. The symlink test in §4
exists for exactly this and would fail against that draft. A path that cannot be `cd`-ed into yields
empty, which never matches a marker — correct, because a directory that
is gone is not a live worktree.

This matters concretely here: `~/.config/fish/config.fish` and the
cargo config are already symlinks into a dotfiles repo on this machine,
so symlinked spellings are the norm, not a hypothetical.

### 2.6 Pruning (Q#GS4)

**`--prune` is dry-run by default and prints what it would delete.**
Deleting requires `--force` as a second, explicit step. There is no
automatic pruning, and pruning never happens on the gate path.

A managed directory is **eligible** when all of:

1. it is a direct child of the managed root;
2. it contains a readable `.pmacs-gate-target` whose content is a
   single absolute path;
3. that path, canonicalized per §2.5, is **not a live worktree**.

**"Live" is narrower than "listed", and the difference is the whole
correctness of this rule.** `git worktree list --porcelain` keeps
reporting a worktree that was **administratively registered but whose
directory was manually deleted** — it simply adds a `prunable <reason>`
line to that entry. Treating every listed path as live would therefore
make exactly the directories most worth reclaiming permanently
ineligible.

So: an entry counts as live **only when its record carries no
`prunable` line**. Records are the blank-line-separated blocks of
`--porcelain` output, read from the primary checkout.

Anything failing any condition is **listed as skipped, with the
reason** rather than passed over silently — a prune that quietly
ignores things is how one learns too late that the marker was never
written.

### 2.7 What it cannot do, stated plainly

**This is a convention, not an enforcement.** An agent that runs bare
`cargo test` still gets the shared directory and still takes the lock.
Nothing in-repo can prevent that while `CARGO_TARGET_DIR` is exported
globally, because **the environment variable overrides
`build.target-dir` in `.cargo/config.toml`** — a per-worktree config
file is silently ineffective, which is worse than absent, because it
looks like it worked.

Real enforcement would need one of: dropping the global export (affects
levcs and whatever else uses it), or `direnv` per worktree. Both are
machine-config changes outside this repo. **The proposal is the script
plus a handoff §3 rewrite that points at it**; if the convention proves leaky
under real parallel load, direnv is the escalation.

---

## 2a. `TMPDIR` isolation (revision 6, AWAITING APPROVAL)

**The gap.** §2 lists what a gate run isolates: the target directory and
five ambient roots. `TMPDIR` was not on that list, so
`tempfile::tempdir()` fixtures landed wherever the operator's `/tmp`
pointed. That is not a hygiene preference — **project detection walks
UPWARD**, so a marker anywhere above the temp directory re-roots every
markerless fixture beneath it.

**Observed, not hypothesised.** An empty `/tmp/.git` reddened
`m4_24_bare_string_glob_stays_relative` and
`m4_24_d3_fallback_base_is_the_smallest_attachment_dir` *inside a gate
run*, on a lane whose entire executable diff lived in `pmacs-gpu` — a
crate the failing test binary does not link. Diagnosing it cost a review
round, and the workaround was a manual `TMPDIR=` on every invocation.

**The contract.** Each invocation gets a directory created fresh by
`mktemp -d` under `<gate-root>/tmp/`, exported once so every stage and
every process they spawn inherits it, and reaped by the exit trap that
already removes the ambient root.

Four decisions inside that, each of which had a cheaper wrong answer:

1. **Not a subdirectory of `/tmp`.** It inherits `/tmp`'s ancestors and
   therefore the marker. The directory has to sit somewhere with no
   marker above it.
2. **Off the GATE ROOT, not the per-worktree target.** A Unix socket
   path cannot exceed `SUN_LEN` (108 bytes) and the suites bind sockets
   *inside* `TMPDIR`. The per-worktree target is 60 bytes and the gate
   root 36; the first implementation used the former and produced
   114-byte socket paths, failing six daemon and attach tests. **The
   parent is consequently SHARED between worktrees and is not covered
   by `--prune`**, which only considers directories carrying an
   ownership marker; each run removes its own leaf.
3. **Created by `mktemp -d`, not `mkdir -p` on a pid.** PIDs are reused,
   so after a SIGKILL a `mkdir -p` silently *adopts* a leftover
   directory and the run inherits another run's fixtures.
4. **Two guards, and both fail loudly at startup** rather than letting
   the symptom appear deep in a suite as a limit with no cause:
   - a **byte-counted** length check reserving the measured maximum
     suffix (`/.tmpXXXXXX/directory-target.sock`, 33 bytes) plus
     headroom — byte-counted because `${#var}` counts *characters*
     under UTF-8 while `sun_path` is byte-limited;
   - an **ancestor-marker check**, because **a managed root is not
     inherently marker-free**: a `.git` in `$HOME`, a marker above
     `$HOME/build`, or a contaminated `PMACS_GATE_TARGET_ROOT` rebuilds
     the original defect one directory up. Placement under a directory
     the gate owns is *necessary, not sufficient*, so the precondition
     is verified rather than assumed.

     **MIRRORING THE NAMES IS NOT ENOUGH — the TYPES are part of the
     contract.** `match_marker` (`src/project.rs`) requires `.git` to be
     a **directory** and the seven language markers to be **files**, so
     an existence-only test rejects ancestors detection itself ignores.
     The case that matters is not exotic: **a git WORKTREE has a `.git`
     FILE**, so every worktree in this repository would have tripped an
     `[ -e ]` check while project detection walked straight past it.
     The guard tests `[ -d ]` for `.git` and `[ -f ]` for the rest.

**The budget is the SUPPORTED-PLATFORM FLOOR, not Linux's.** `sun_path`
is 108 bytes on Linux but **104 on Darwin** (xnu `bsd/sys/un.h`), and
pmacs supports macOS — CI runs a `macos-latest` leg. A Linux-derived
limit would pass on the machine that wrote it and bind-fail on the
other, which is the worst place to find out. **The usable PATH length is
one less than the array**, because the stored value is NUL-terminated:
103 on Darwin, 107 on Linux. The script takes **103**.

**RULING — a synthetic nested gate does not pay a reserve it never
uses.** The reserve exists for fixtures that bind sockets under
`TMPDIR`. This script's own behaviour suite runs *nested* gates whose
plans are synthetic (`true`, `false`, one `echo`) and which bind no
socket at all, so applying the fixture reserve to them would reject a
configuration that cannot suffer the failure it guards against — and
the suite would fail on a setup it created rather than on the behaviour
under test. That is not hypothetical: at a 45-byte reserve the nested
path measured ~71 bytes and was rejected.

Two ways to resolve it were available, and **the layout was changed
rather than the guard weakened**:

- *Rejected — exempt nested runs from the guard.* It would make the
  guard untestable in the configuration the tests exercise, and "this
  run is nested" is not something the script can know reliably.
- **Adopted — the behaviour suite roots its gates at a SHORT base
  (`/tmp`) instead of inheriting the ambient `TMPDIR`.** A nested gate
  then sits at ~24 bytes rather than ~71 and clears the real reserve
  with room to spare. The suite is explicit that it does this for the
  socket budget, and it is free to use `/tmp` precisely because its
  plans create no markerless fixture — the same reason it may set the
  ancestor escape.

**The guard therefore keeps the true maximum for real runs**, and the
tests stop paying for a hazard they cannot encounter. If a future
behaviour row *does* bind a socket, it must move off the short base and
take the reserve with it.

**Escape hatch, documented test-only.**
`PMACS_GATE_ALLOW_ANCESTOR_MARKER` exists for this script's own
behaviour tests, which run the gate under a `tempfile::tempdir()` whose
ancestors they do not control — on a machine whose `/tmp` carries the
very marker in question — and whose plans are synthetic, so no
markerless fixture exists for a marker to re-root. It sits beside
`PMACS_GATE_TARGET_ROOT` in kind and in risk. **The check is witnessed
by a row that deliberately does not set it.**

**Verification.** Two witnesses beyond the refusal row: propagation
observed in a *spawned child* (the self-test's first step reports its
own `$TMPDIR` into its log — asserting the variable inside the script
would only prove the script can set a variable), and cleanup after a
run that **failed on purpose**, which is the path a leak would actually
take.

**Residual, stated rather than covered.** A custom project marker
registered at runtime is invisible to a shell script and is not
checked. The built-in list mirrors `default_markers()` in
`src/project.rs` and will drift if that list grows.

## 3. Resolved questions

### Q#GS1 — directory naming — **RESOLVED**

`<basename>-<8 hex of absolute path>` under the managed root of §2.1.
Branch-name derivation was rejected: the name would change on every
branch switch inside one worktree, discarding artifacts precisely when
they are most reusable. Basename alone collides — the recovery
convention in `docs/active-work.md` actively produces sibling
worktrees with predictable names.

### Q#GS2 — gate authority — **RESOLVED, NARROWED**

The script is authoritative for the **fixed executable gates**. §3
remains authoritative for **policy** (why `--workspace` not `--tests`,
why `--skip basedpyright`, when `--protocol` applies) and for
**selecting the touched acceptance suites**, which reach the script
only through `--acceptance`.

Revision 1 claimed the script should own the suite outright. That was
wrong for a concrete reason: **no script can infer which acceptance
suites a change touches**, and one that guessed would report coverage
it did not have.

### Q#GS3 — does the primary checkout get an isolated directory — **RESOLVED**

Yes. Uniform, no special case. The primary abandons the warm shared
directory and pays 80s once. A rule with an exception gets applied
wrongly.

### Q#GS4 — pruning — **RESOLVED**

Explicit only, dry-run by default, `--force` to delete, eligibility and
skip-reporting per §2.6, liveness narrowed to non-`prunable` records
and paths canonicalized per §2.5.

### Q#GS5 — CI — **RESOLVED, UNCHANGED**

CI runs in fresh containers with their own target directory and no
shared lock, so the isolation is a no-op there. CI's matrix splits the
suite across jobs deliberately (`Test (crdt)`, `M4 Perf Gates`, …);
collapsing that into one script would serialize the matrix and lose
per-job signal. Worth revisiting; not free, and not this change.

---

## 4. Verification

Revision 1 proposed behavior tests with no mechanism to run them.
`PMACS_GATE_TARGET_ROOT` (§2.1) is that mechanism: **every behavior
test points it at a `tempfile::tempdir()`, and the real managed root is
unreachable from the suite.**

The tests live in `tests/gate_script_acceptance.rs`, so they ride the
existing workspace sweep — a new test, not a CI change (Q#GS5).

**The recursion constraint shapes what is testable.** A test that ran
`scripts/gate` for real would run the full suite *inside* the suite. So
the tests exercise only paths that **run no gates** — which is a
stronger constraint than "non-mutating", and revision 2 conflated the
two. Asserting that the marker is written requires something that
*writes* it; a pure printer cannot, and a real gate run must not. That
is what `--init` is for (§2.3): it mutates, it runs no gates, and the
gate path calls the same routine, so the test is not exercising a
second implementation.

The paths under test:

- **`--print-plan` matches §3.** Asserts the plan contains
  `--workspace` and **never** `--tests`; carries `--skip basedpyright`
  on both suite-wide lines; includes both `--lib` configurations; and
  that `--protocol` adds the CRDT workspace sweep while the default
  does **not**. This is the direct test of Q#GS2's named drift risk,
  and it is why `--print-plan` exists.
- **`--acceptance` is reflected in the plan**, once per suite, in
  order — the seam §3 keeps authority over.
- **Derivation is stable across a branch switch** and **differs
  between two worktrees** — the two properties Q#GS1 turns on.
  `--print-target-dir`, which mutates nothing.
- **`--init` writes the marker**, and its content is the worktree's
  path in the §2.5 canonical form. Also that `--init` is idempotent:
  running it twice leaves one directory and one marker.
- **A symlinked spelling of the same worktree derives the same
  directory and does not prune.** The fixture reaches one worktree
  through a symlink; without §2.5's canonicalization at both ends this
  is precisely the case that deletes a live lane's artifacts.
- **`--prune` is dry-run by default**: against a fixture root holding
  one eligible directory, one unmarked look-alike, and one marker
  pointing at a live worktree, it *names* the eligible one and
  **deletes nothing**. Re-run with `--force`, it deletes exactly that
  one; the look-alike and the live one survive. This is the test that
  matters — a prune bug is unrecoverable.
- **A `prunable` worktree record counts as dead.** The fixture
  registers a worktree and deletes its directory without
  `git worktree remove`, so `git worktree list --porcelain` still
  reports it, carrying `prunable`. Its target directory must be
  eligible. Treating "listed" as "live" would make exactly the
  directories most worth reclaiming permanently un-prunable, and
  nothing else in the suite would notice.
- **Skip reasons are reported**, not silent.

**Verified by observation, not automated** — and named so the gap is
explicit: that a failing gate exits non-zero and says which gate. A
deliberately broken tree is a real run of the full suite, so it is a
one-off recorded in the lane's PR rather than a test.

**Also verified by observation:** that the ambient roots are actually
redirected and the sweep logs actually appear. Both are properties of a
real gate run, so they are confirmed in the same one-off as the exit
status above, and named here so the confirmation is not skipped. **If
the log file is absent, the U2/U3 fix is not real** — that is the one
outcome of the observed run that would block the lane.

**What none of this proves:** that agents will use the script. That is
§2.7's admission, and no test in this repo can close it.

Gates for the lane itself: the standard suite (via the script, once it
exists — the lane gates itself by construction), plus `shellcheck` if
available and `git diff --check`.

---

## 5. Not in scope

Dropping the global `CARGO_TARGET_DIR` export. direnv. `sccache`
tuning or `--remap-path-prefix`. CI restructuring (Q#GS5). Pruning the
233G shared directory beyond the 51G of `debug/incremental` already
reclaimed — it has a co-tenant, and `cargo clean` there would destroy
levcs's artifacts.
