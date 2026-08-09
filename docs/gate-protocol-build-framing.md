# `scripts/gate --protocol` — the build its sweep depends on

**Status: framing pass, revision 3. Pre-implementation. Awaiting
approval.**

**Revision 3 fixes a witness that could not fail.** Revision 2's
`--self-test` plan put the failing step **last**, so an aborting runner
and a continuing one produce identical output — the witness for
Q#GR-2's "the suite keeps going" policy would have passed on a runner
doing the opposite. A passing **sentinel after** the failure, asserted
to have written its log, is what separates them. §7 also now pins the
**exact** build command rather than only the step's name and position,
since a `build-crdt` running plain `cargo build` would leave the gate
just as unsound while looking repaired.

**Revision 2 takes three review findings.** The normative requirement
goes **entirely** into handoff §3 rather than being split across §3 and
§5 (§5, Q#GR-3). Q#GR-1's observation procedure is respecified on a
**disposable** target with the binary's absence asserted before each
run, rather than by deleting a file from a live worktree. And the
build-attribution criterion, which revision 1 stated with **no way to
observe it**, gets a witness — via a hardcoded synthetic plan, not the
plan-file injection that would reintroduce this script's own
`--acceptance` defect (Q#GR-5).

**A narrow lane, deliberately.** One missing step in one script, plus
the boundary question that let it go missing. No feature work, no
`src/`, no protocol.

---

## 1. The defect

`scripts/gate --protocol` adds the CRDT workspace sweep. That sweep has
a documented precondition, and **the script does not run it**.

`docs/agent-handoff.md` §5 (`:532-535`):

> **The crdt sweep needs `cargo build --workspace` first**, or twelve
> `gpu_invocation_acceptance` tests fail on a missing `pmacs-gpu`
> binary. `cargo build --workspace --no-default-features --features
> luajit,crdt` is the invocation that produces both binaries.

`scripts/gate`'s plan emitter (`:187-204`) goes
`… → gpu → sweep → sweep-crdt → diff-check`, with **no build step
anywhere**. Read from the source, not inferred from the failure.

**Observed, not theorised.** PR #228's first gate run failed step 09
with twelve `gpu_invocation_acceptance::crdt::*` failures, all
*"build pmacs-gpu before this acceptance suite"*, and `debug/pmacs-gpu`
was absent from that worktree's target directory. Running the
documented invocation and re-running the gate turned it green.

## 2. Why it was latent until now, which is the interesting part

**#225 caused this to become reachable, and #225 is also what makes it
matter.**

Before #225 every worktree on this machine resolved to **one shared**
`CARGO_TARGET_DIR`. That directory almost always already contained a
`pmacs-gpu` binary from some earlier build, so the precondition was
**satisfied by accident** on essentially every run. #225 gave each
worktree its own target directory; a fresh one starts empty, and the
omission becomes load-bearing on the very first `--protocol` run in a
new worktree.

So this is not "a bug #225 introduced". It is a **pre-existing gap in
the documented procedure that #225 stopped hiding** — and the reason it
is urgent rather than tidy is that the failure mode is not a red gate.
A red gate is fine; it stops you. The hazard is the *reverse*: a green
`--protocol` run whose crdt sweep was decided by **what happened to be
in the build directory** rather than by the diff. That is a gate that
reports coverage it does not have, which is precisely what #225 exists
to prevent.

## 3. The likely mechanism, marked as inference

The failing tests are namespaced `gpu_invocation_acceptance::crdt::*`,
which suggests they are **feature-gated to `crdt`** and therefore
compile and run only under the crdt sweep. That would explain why the
default sweep passes on a tree with no `pmacs-gpu` binary at all — it
never runs the tests that spawn it.

**This is inference from the test names and one observation, and it is
not yet verified.** Q#GR-1 makes establishing it part of the work
rather than an assumption the fix rests on.

## 4. The change

*My vote: **a named `build-crdt` gate step, emitted immediately before
`sweep-crdt` and only under `--protocol`***, running the invocation
handoff §5 names.

- **A named step, not a silent prelude.** It appears in
  `--print-plan`, gets its own numbered log alongside the others, and
  fails the suite with its own name if the build fails.
- **Not folded into the `sweep-crdt` command.** `cargo build … && cargo
  test …` would make a *build* failure appear under the name `sweep-crdt`
  in the failure list — a wrong attribution in the one place the script
  exists to be trustworthy about. **Q#GR-5 is how that is witnessed**,
  which revision 1 asserted without supplying.
- **Only under `--protocol`.** If §3's inference holds, the default
  sweep does not need it, and adding an unconditional workspace build
  to every gate run is a real cost paid for nothing.

## 5. The boundary question, which is the durable half

The script's own header says:

> `docs/agent-handoff.md` section 3 owns the REASONING for each of
> these … `--print-plan` renders this without running anything, which
> is what makes **drift from section 3** testable.

**The drift here is from §5, not §3** — and that is a coherent reason
for the omission rather than mere oversight. `scripts/gate` was written
against §3's gate policy; this precondition lives in §5's hazard
register, which the script never claimed to encode.

So the durable fix is not only the missing line. It is deciding where a
requirement like this belongs.

**The normative requirement moves ENTIRELY into §3.** It is gate policy
— it decides whether a gate's result means anything — and §3 is already
where such policy lives. §5 keeps the **incident and its signature**,
which is history, not contract.

**The script header keeps citing §3 and only §3.** Revision 1 also
proposed citing §5, which was wrong twice over: it splits one
executable contract across two sections, and it weakens the single
clean boundary the script has (*"§3 owns the reasoning"*) at the same
time as Q#GR-4 declines to build any automated check for prose drift.
A boundary that is neither enforced nor singular is not a boundary.
One normative home, one citation. Q#GR-3.

## 6. Open questions

### Q#GR-1 — what exactly must be built, and does the default sweep need it too?

§5 names `cargo build --workspace --no-default-features --features
luajit,crdt` and says it "produces both binaries". §3's inference says
the default sweep is unaffected. **Neither is verified by this
document.**

*Required before implementation, by observation rather than reading.*
Revision 1 said "delete `pmacs-gpu` from a target directory", which is
both unsafe and insufficient: it **mutates a durable worktree's build
directory**, and removing one binary does not establish that the other
artifacts and feature permutations are cold — a stale dependency graph
can satisfy the run for reasons the experiment never sees.

**The procedure:**

1. A **disposable** target directory (a scratch `CARGO_TARGET_DIR`, or
   a throwaway worktree), never a live lane's. Nothing under
   `$HOME/build/pmacs-gate-targets/` belonging to a real branch is
   touched.
2. **Assert `debug/pmacs-gpu` is ABSENT before each run**, as a
   recorded precondition rather than an assumption. A run whose
   starting state was not checked proves nothing about a cold tree.
3. Run the **default** sweep alone. Record pass/fail and, if it fails,
   the failing test names.
4. Reset to the same cold state, assert absence again, run the **crdt**
   sweep alone. Record the same.

Each sweep separately, so a result cannot be explained by the other
having built the binary first — which is the exact accident (§2) that
hid this defect for the entire life of the shared target directory.

If the default sweep also needs the binary, the step is unconditional
and §4's "only under `--protocol`" is wrong.

**This is the one thing in this lane I would not accept on reasoning.**
The whole defect is a precondition nobody checked; establishing its
replacement by reading would repeat the error at one remove.

### Q#GR-2 — does a build failure fail the suite, or abort it?

*My vote: **fail like any other gate***, and let the remaining steps
run. `--no-fail-fast` is the established posture of this suite, and a
sweep that then fails for the missing binary produces a second,
consistent signal rather than a mysterious absence.

The counter-argument is real: twelve downstream failures with a known
cause is noise. But the script already prints per-gate logs and a
`FAILED:` list, so the cause is named at the top, and suppressing
downstream output is how a tool starts deciding what its user is
allowed to see.

### Q#GR-3 — where does this requirement live?

See §5. **§3 gains it normatively; §5 keeps the incident; the script's
header keeps citing §3 alone.** Revision 1 proposed citing both, which
would have split one executable contract across two sections while
Q#GR-4 declines to build any check for prose drift.

### Q#GR-5 — how is the attribution criterion witnessed at all? **(new in rev 2)**

Revision 1 asserted that a build failure must be attributed to
`build-crdt` rather than `sweep-crdt`, and gave no way to observe it.
That criterion was unwitnessable as written: `tests/gate_script_acceptance.rs`
deliberately exercises only **no-gates** paths, so plan assertions can
prove a step's name and its order and **nothing about runtime
behaviour**.

**The obvious seam is a trap.** Making `PLAN_FILE` injectable — let a
test hand the runner its own plan — would work, and it would turn the
script into a general command executor via the `eval` at its runner
loop. That is the **same class of defect this script's own review
already caught in `--acceptance`**, which was fixed with a refusal at
parse time. Reintroducing it one lane later, in the tool whose purpose
is to be trustworthy, is not a trade worth making.

*My vote: **a `--self-test` mode running a HARDCODED synthetic plan***
— **three** lines: a passing step, a failing one named `build-crdt`,
and **a passing SENTINEL after it**.

**The third line is not padding, and revision 2's two-line plan was
broken without it.** With the failure last, a runner that **aborts** on
failure and one that **continues** produce identical output, so the
witness passes either way — and Q#GR-2's whole answer is that the suite
keeps going. A sentinel *after* the failing step, asserted to have run
and written its log, is the only thing that distinguishes them.

So it asserts: the runner names the failing gate, lists it under
`FAILED:`, writes its log where it says it does, exits non-zero, **and
the sentinel after the failure has its own log** — which is Q#GR-2's
policy made observable rather than declared.

- **No injection.** The synthetic plan is a literal inside the script;
  nothing external supplies a command.
- **Runs no real gate**, so it stays on the cheap no-gates side of the
  existing suite. `true`/`false` are the whole workload.
- **It tests the runner, which is the thing under test.** Whether
  `cargo build` really fails is `cargo`'s business; whether *this
  script names the right gate when a command fails* is the criterion,
  and it is orthogonal to which command failed.

The alternative is a **documented manual witness** — break the build by
hand, run the gate, record the output in the lane. Honest, and it rots:
nothing re-runs it, so it decays into a claim about a past machine.
Named as the fallback if review rejects a new mode.

### Q#GR-4 — should `--print-plan` be asserted against the handoff?

Tempting and out of scope. A test that parses prose out of
`agent-handoff.md` and compares it to the plan would be brittle in the
direction that produces false confidence. **Not in this lane**, and
named so it is not mistaken for an oversight.

## 7. Verification

- **`--print-plan --protocol` emits `build-crdt` immediately before
  `sweep-crdt`, carrying the EXACT command.** All three asserted —
  presence, position, and the literal
  `cargo build --workspace --no-default-features --features luajit,crdt`.
  Name and position alone would pass on a step that builds the wrong
  feature set, which is the failure this lane is fixing: the crdt sweep
  needs *those* features, and a `build-crdt` that ran plain
  `cargo build` would leave the gate exactly as unsound while looking
  repaired.
- **`--print-plan` WITHOUT `--protocol` does not emit it** (subject to
  Q#GR-1 — if the default sweep turns out to need the binary too, this
  assertion inverts and §4 changes with it).
- **A real fresh-target `--protocol` run goes green without a manual
  build**, which is the acceptance criterion and the thing that was
  false. Witnessed on a target directory with no `pmacs-gpu` in it.
- **A failing gate is attributed to its own name**, witnessed through
  `--self-test`'s synthetic plan (Q#GR-5): the run exits non-zero,
  prints `build-crdt` as the failing step, lists it under `FAILED:`,
  and writes the log path it claims. This is the criterion revision 1
  stated with no way to observe it.
- **The suite CONTINUES past a failed gate** (Q#GR-2) — the sentinel
  step after `build-crdt` in the synthetic plan has its own log.
  **Revision 2's two-line plan could not assert this**: with the
  failure last, an aborting runner and a continuing one are
  indistinguishable, so the witness would have passed on a runner that
  does the opposite of the stated policy.
- **The existing 15 `tests/gate_script_acceptance.rs` tests still
  pass**, and the new assertions join them on the **no-gates paths**
  (`--print-plan` runs nothing), keeping the suite cheap.

**What this will NOT prove:** that the plan matches the handoff in
general (Q#GR-4), or that any other §5 hazard is encoded in the script
— this lane fixes one and asks where such requirements belong, it does
not audit §5.

## 8. Not in scope

Any feature work. Any `src/` change. Auditing the rest of handoff §5
for further unencoded preconditions (worth doing; not here). A
plan-versus-handoff consistency test (Q#GR-4). Changing which gates the
suite runs, or the acceptance-suite selection policy — §3 remains
authoritative for both. **Rerunning PR #228's gate**, which is that
lane's unblocking step and happens after this lands, not inside it.
