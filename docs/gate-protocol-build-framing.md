# `scripts/gate --protocol` — the build its sweep depends on

**Status: framing pass, revision 1. Pre-implementation. Awaiting
approval.**

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
  exists to be trustworthy about.
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
requirement like this belongs, and making the script's stated contract
match what it actually has to guarantee. *My vote: **§3 gains the
precondition** (it is gate policy — it decides whether a gate's result
means anything), §5 keeps the incident and its signature, and the
script's header stops naming §3 as its only source.* Q#GR-3.

## 6. Open questions

### Q#GR-1 — what exactly must be built, and does the default sweep need it too?

§5 names `cargo build --workspace --no-default-features --features
luajit,crdt` and says it "produces both binaries". §3's inference says
the default sweep is unaffected. **Neither is verified by this
document.**

*Required before implementation, by observation rather than reading:*
delete `pmacs-gpu` from a target directory, run the **default** sweep,
and record whether it passes; then repeat for the crdt sweep. If the
default sweep also needs a binary, the step is unconditional and §4's
"only under `--protocol`" is wrong.

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

See §5. *My vote: §3 gains it, §5 keeps the incident, the script's
header cites both.* The alternative — leave §5 as the only home and
have the script silently encode it — reproduces exactly the condition
that made this gap invisible.

### Q#GR-4 — should `--print-plan` be asserted against the handoff?

Tempting and out of scope. A test that parses prose out of
`agent-handoff.md` and compares it to the plan would be brittle in the
direction that produces false confidence. **Not in this lane**, and
named so it is not mistaken for an oversight.

## 7. Verification

- **`--print-plan --protocol` emits `build-crdt` immediately before
  `sweep-crdt`.** Order asserted, not just presence: a build after the
  sweep it feeds is the same defect with an extra line.
- **`--print-plan` WITHOUT `--protocol` does not emit it** (subject to
  Q#GR-1 — if the default sweep turns out to need the binary too, this
  assertion inverts and §4 changes with it).
- **A real fresh-target `--protocol` run goes green without a manual
  build**, which is the acceptance criterion and the thing that was
  false. Witnessed on a target directory with no `pmacs-gpu` in it.
- **A failing build is attributed to `build-crdt`**, not to
  `sweep-crdt` (Q#GR-2) — the wrong-name case §4 rejects.
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
