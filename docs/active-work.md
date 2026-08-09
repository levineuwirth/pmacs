# Active work — cross-machine resume ledger

**Snapshot: 2026-08-01.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

**Updated later the same day, on a new machine.** Development moved to
the laptop; the recovery path in "Repository authority" below was
exercised from this checkout and the `githubsucks` alias was absent and
had to be added, exactly as that section anticipates. **One lane opened:
CI CRDT coverage**, which had been sitting under "NEEDS A LANE" with no
branch and no owner since #166. It is implemented on
`ci-crdt-coverage` and its block replaces the old one below.

**Updated 2026-08-05.** One lane opened: **macOS CI signal integrity**
(#215, in review), which this file required a lane for and did not have
until review caught it — the #171 defect recurring. Its block is below.

**Updated later the same day.** #215 **merged** (`main` @ `12f2970`) and
that lane is **rewritten, not removed** — rule 4 removes a lane when its
ARC is done, and Stage 2 is the arc. Stage 2 (**hardening**) ran on
`ci-signal-hardening`, with its own lane block, its own checkpoint
table, and a lane written **before** the PR was opened rather than
after review asked for it. **It merged as #216 on 2026-08-05**, which
completes the arc: R2 and R4 are retired with discriminating
witnesses.

**The lane is kept, not yet removed, and that is a deferral rather than
a judgement.** Rule 4 would remove it now — but its residue must be
re-homed first, or removing it loses the state: **R1** is referred to
the async-runtime lane (Q#MCI3) and **R3** is an unresolved possible
product defect owned by the process-signal / PTY lane, neither of which
has a block here yet. Retiring this arc and opening those two is an
**absorption pass**, and doing it inside an unrelated feature PR is how
a ledger acquires edits nobody reviewed.

**Updated 2026-08-04.** Four PRs landed since: the CI CRDT coverage
lane #209, Distribution Stage 1 #211 (released as **v1.1.0**), the
post-release accuracy pass #212, and **bottom-panel Stage 3 #213 —
which completes Arc 7**. Its lane is **removed** per rule 4: the arc is
done *and* its durable facts are in `docs/agent-handoff.md` §1. The
distribution and CI-CRDT lanes are rewritten rather than removed,
because each keeps named follow-ons.

**This snapshot is an absorption pass, taken with ZERO open PRs** —
the one window in which a ledger refresh has nothing to re-conflict
with, and taken deliberately before a machine move. Nine PRs landed
since the previous anchor (#199–#207). Two arcs completed and their
lanes are **removed**, their durable facts now in
`docs/agent-handoff.md` §1: **Journey Stage 1** (1a plus the whole 1b
split) and **test ambient-root isolation**. Discovery and reap-ledger
merged a stage each and keep lanes rewritten to what remains.
`docs/agent-handoff.md` §1a now carries the whole board — every arc,
open lane, deferred item and standing hazard in one view.

*(Earlier note, retained because its reasoning still governs:)*
**This snapshot is an absorption pass.** Eight PRs merged on 2026-07-29
and 2026-07-30 (#188, #190, #191, #194, #195, #196, #197, #198) and the
ledger had drifted to 1,854 lines carrying six lanes whose work was
already on `main`. Those six are removed and their load-bearing
decisions are in `docs/agent-handoff.md` §1 — rule 4's precondition,
satisfied rather than deferred. The file is now 609 lines.

**A lane is removed when its ARC is done, not when a PR merges.** Two
lanes survive their sub-stage merges and are rewritten to the remaining
plan rather than deleted: generated-buffer immutability (Stage 1 merged,
Stage 2 not started) and bottom-panel (Stage 2 complete, Stage 3 ahead).
The bottom-panel block said so in its own text — *"this lane is not
removed at 2B-3's merge"* — and a wholesale removal keyed on "the PR
merged" would have discarded live planning. Read each block before
cutting it.

**#188's lane arrived with #188**, which is the point: with several PRs
open, a lane written on `main` for work that lands elsewhere
re-conflicts on every merge. Written on its own branch it costs one
conflict, at the merge that would have happened anyway.

The PTY terminate
diagnostic (#176) was the last lane retained past its merge — retained
because rule 4 removes a
merged lane only *after* its durable facts reach
`docs/agent-handoff.md`, and that absorption was unowned. The
2026-07-28 snapshot
owned it: #176's facts are now in the handoff (§1's arc bullet and §5's
two ops lessons about ticking observers and proving child exit), so its
lane is gone. The Lean 4, GPU-terminal-input, inline-math (#172), dired
(#169), and terminal config + copy mode lanes were removed the same way
— the last of these was #180's work, folded into #182 so two open PRs
would stop re-conflicting in this file.

**Trust the canonical-base line below over any lane header**: if a PR
number appears in `git log --first-parent githubsucks/main`, it has
landed regardless of what a lane says.

**Two open PRs had no lane here at all before the 2026-07-28
snapshot** — #174 and #171. An open PR is exactly the volatile work this
file exists to record, so its absence is a ledger defect rather than a
tidy omission: #171 drifted **153 commits** while invisible here, and
its still-green old CI run described a tree nobody had looked at since.
**When a PR is opened, give it a lane.** All three have since merged —
#174, #171 and #186 — so per rule 4 their lanes are gone again and
their durable facts are in `docs/agent-handoff.md` (§5 for #174's
lesson, §1 for the two framings).

## Repository authority

- Canonical development URL:
  `https://github.com/levineuwirth/pmacs.git`. This ledger uses the
  normalized local alias `githubsucks` so its refs and recovery commands
  are identical on every machine. Remote names are otherwise
  machine-local: `origin` may name this canonical URL, a release mirror,
  or something else, and therefore has no authority by name alone.
- Canonical base at this snapshot: **`githubsucks/main` @ `9a26ac8`** —
  GPU horizontal scroll **#223**, which **closes the QoL arc**, atop
  `2b56d16` TUI horizontal scroll **#222**, `02f3ec3` `ui.line-wrap`
  **#221** (protocol v22), `218d2e7` GUI zoom **#220** and `da56bec`
  `full_grid` **#219**. Beneath those, `db1bbe9`:
  the tree primitive **#217**, atop `2657568` the macOS CI
  signal-integrity **Stage 2 #216** (which retired R2 and R4), atop
  `12f2970` its Stage 1 registry **#215**, atop `f186253`:
  bottom-panel Stage 3 **#213**, which completes Arc 7, atop the
  post-release accuracy pass **#212**, Distribution Stage 1 **#211**
  (released as **v1.1.0**, the first release with prebuilt binaries),
  and the CI CRDT coverage lane **#209**. Beneath those, `cfc1710`:
  discovery Stage 1 #207, the ambient-root isolation implementation
  #206, Journey Stage 1b-3 #205, 1b-2 #204 and 1b-1 #203, the
  reap-ledger diagnostic #202, the isolation framing #201, the
  process-signal diagnostic #200 and the ledger absorption #199.
  **Protocol schema support is `v6..=v22`; the production server-first
  `Hello` still advertises v20** — two different facts, and #184 landed
  only the first. The upper bound moved to **v22 at #221**, which added
  `InstanceMessage::LineWrapFacts`; `ADVERTISED_PROTOCOL_VERSION` did
  not move and must not be edited to chase it. Verified against
  `pmacs-protocol/src/message.rs`, not carried forward: this line said
  `v6..=v21` for two merges after that stopped being true.
  **Historical `v21` statements elsewhere in this file are correct
  where they describe a stage as it landed** — only this
  current-state paragraph tracks the live range.
  **The recovery floor advances with the base**, so the check below
  now requires `9a26ac8` or newer; a tree at `db1bbe9` no longer
  passes — it would lack the entire QoL arc, which this file and the
  handoff both describe as complete. That is deliberate — a check accepting an older commit than
  the declared base passes on a tree the rest of this file does not
  describe.
  **Lanes below that name an older base have not been re-based; derive
  their integration surface from `git diff <their base>..main`.**
- On the transfer source, `origin/main` named a release mirror at
  `d3fa632` and lagged badly. On the current destination, `origin` names
  the canonical URL. This difference is why all recovery begins by
  verifying URLs and normalizing `githubsucks` rather than trusting
  `origin/main`.
- The shared desktop checkout contained unrelated uncommitted work. The
  branches below were prepared in isolated worktrees; never clean or
  overwrite the shared checkout to recover them.

Start on another machine by inspecting its remotes:

```sh
git remote -v
git remote get-url githubsucks
```

If the second command says the alias is absent, add it; if it prints a
different URL, stop and resolve that collision rather than overwriting an
unknown remote:

```sh
git remote add githubsucks https://github.com/levineuwirth/pmacs.git
```

Then recover current refs:

```sh
git fetch githubsucks --prune
git log -1 --oneline githubsucks/main
git worktree list
git status --short --branch
```

The `git log` command must expose `9a26ac8` — the base named above — or a
newer intentional main. Keep this threshold and the canonical-base line in
step: a recovery check that accepts an older commit than the base it
declares canonical will pass on a tree the rest of this file does not
describe.
If it does not, stop and repair the remote/fetch configuration.

**This path was exercised, not asserted, at this snapshot** — re-run
from an empty directory on 2026-08-08 when the base advanced to
`9a26ac8`, rather than having its SHA swapped. That distinction is the
whole point of this paragraph: advancing the base is exactly when the
recovery commands are most likely to have rotted, and a swapped SHA
reads identically to a verified one. `git clone` the canonical URL, add
the `githubsucks` alias, `git fetch githubsucks --prune`, confirm
`9a26ac8` is an ancestor of `githubsucks/main`, and recover with the
three-argument `git worktree add <path> -b <local> githubsucks/<branch>`
form. All four steps ran clean.

**Correction, found by re-running it.** This file claimed the
two-argument form fails for a remote-only branch with `fatal: invalid
reference`. **It does not fail.** On git 2.55.0 it *succeeds* and
leaves a **detached HEAD** — no branch, no upstream, `git status`
reporting `## HEAD (no branch)`.

Still use `-b`, but for the opposite reason to the one recorded: the
danger is not an error that stops you, it is that nothing stops you.
Work committed in that worktree sits on no branch, is not pushed by a
bare `git push`, and is exactly the "uncommitted work does not travel"
hazard in a shape that looks committed. **A documented error message
that never appears is worse than no documentation**, because the reader
waits for a signal that is not coming.

## `scripts/gate` — PR #225 OPEN (build tooling)

**PR #225** — https://github.com/levineuwirth/pmacs/pull/225. Written
**after** the PR existed, again, and again because review asked. Two
lanes in a row have now been added late; the correction from #171 and
#215 is not sticking, and recording that is more useful than a
back-dated block that pretends it did.

- **Branch `gate-script`**, base `githubsucks/main` @ `b833b13` (the
  #224 merge). **`githubsucks/gate-script` is the authoritative tip** —
  the ref, not a SHA. Recover with
  `git fetch githubsucks && git checkout gate-script`.
- **Framing `docs/gate-script-framing.md`**, approved at revision 4
  after four review rounds; revision 5 records two safety defects found
  against the implementation, not the design.
- **Scope:** `scripts/gate` (per-worktree `CARGO_TARGET_DIR`, five
  ambient roots, durable per-gate logs, the fixed gate suite),
  `tests/gate_script_acceptance.rs`, and a handoff §3 rewrite pointing
  at the script while §3 keeps policy and acceptance-suite selection.
  No `src/`, no crate, no manifest, no protocol.
- **Verification:** 15 acceptance tests over the no-gates paths, each
  isolated by `PMACS_GATE_TARGET_ROOT`. Mutation-tested; the two prune
  guards are redundant by design and only fail the test when **both**
  are removed, which is recorded in the test itself. Observed real runs
  confirm failed-gate naming, log paths, ambient creation and reaping,
  and distinct log directories per run.

**GATE STATUS — R8 RESOLVED.** This lane was blocked because
`scripts/gate` exited 1 on a clean tree: **R8** failed `m4_acceptance`
and therefore the sweep. That was never a footnote — #225 is the lane
that makes the gate suite authoritative, and a tool shipping with its
own gate red teaches the opposite of what it exists to teach.

**R8 was fixed and retired in #226** (`dcb852e`), which bounded the LSP
fixture's project detection. This branch is rebased onto it.

**RE-GATED 2026-08-09: `scripts/gate` exits 0.** All nine gates green in
one command — fmt, clippy, `--lib`, `--lib --features crdt`, both named
acceptance suites, `m4_acceptance`, `-p pmacs-gpu`, and the full
workspace sweep. That is #225's own acceptance criterion, and it is the
first time the tool has passed the suite it exists to run.

**Rebase resolution, per the standing rule that #226's R8 documentation
wins.** Three conflicts, all in R8 text this branch had written while
the row was still an open investigation: two in
`docs/ci-red-signatures.md` (both resolved to #226's retired row, this
branch's pre-fix copy dropped), and the framing-doc pair
(`e71e1bd` added it, `7cfba73` removed it — both **skipped**, since they
are net-zero here and `main` owns the file authoritatively; replaying
the second would have deleted `main`'s copy). Two now-stale lanes were
also removed: this branch's "R8 NEEDS A LANE" investigation block, and
#226's own lane, which Rule 4 retires now that it has merged — its
durable facts are in the retired registry row and the handoff §6
census.

## Git integration Stage 1 — PR #227 OPEN

**PR #227** — https://github.com/levineuwirth/pmacs/pull/227. Opened
2026-08-09 at `4002734`, after the framing was approved at revision 5
and the full gate suite went green. **Now at `6c1631e`, MERGE-BLOCKED.**

**Review round 1 found three blockers. Two are fixed; the third is why
this lane is blocked.**

- **P1 fixed (`ffe5ae2`) — concurrent `status` opens were ordered by
  `rev-parse` completion, not by invocation.** The generation was
  minted on arrival, inside `start_status`, so the **slowest root
  lookup won** rather than the newest invocation. It is now reserved at
  the command and carried through the root-resolution callback, which
  drops a superseded result **before any effect** — no spawn, no
  `state.root` write, and no status message either, since a message
  from a replaced invocation is as wrong as a panel from one.
- **P2 fixed (`6c1631e`) — a refused `keys` bind left a live, unowned
  buffer.** The preflight compared **raw tokens** while `parse_key_code`
  folds `RET`/`RETURN`/`ENTER` (and `SPC`/`SPACE`, `ESC`/`ESCAPE`,
  `BS`/`BACKSPACE`, `DEL`/`DELETE`, case-insensitively) onto one chord,
  so an alias spelling passed preflight and failed at bind time —
  *after* buffer creation and intercept install. The partial rollback
  removed only new keys, so later opens silently got `<2>`.

  **Fixed by full teardown, not by canonicalizing the preflight**, and
  the reasoning is worth keeping: a Lua canonicalizer would be a second
  copy of the Rust alias table and would go stale the day that table
  gains a name, reintroducing this exact bug for the new alias.
  `Keymap::bind` is the authority because it *is* what decides. There
  is also **no Lua-reachable canonicalization** to build on —
  `display_sequence` escapes only through `describe.key` and
  `keymap.list`, both of which require the sequence to be bound
  already. Verified; no binding was added for this.
- **P1a NOT fixed, and deliberately — it blocks this PR.** The async
  completions display UI without capturing the initiating frontend
  (`builtin/runtime/git.lua:609`, `:854`), so a result surfaces in
  whichever frontend is active when git exits. `commit_to` is the right
  mechanism and is **not Lua-reachable** outside a directory open, so
  the fix lives in the **`destination-capture`** lane. This lane adopts
  it after that lands. Verified untouched: no diff line in `ffe5ae2` or
  `6c1631e` reaches `open_status_panel`, `show_diff_buffer`,
  `pmacs.window.display` or `commit_to`.

**Re-gated at `6c1631e`:** all 11 steps green, acceptance now 27 tests.
Both fixes mutation-verified.

**Written with the lane's first commit, before the PR exists** — the
standing correction from #171 and #215. This session it was missed on
#224 and again on #225, both caught by review; writing it now is the
only thing that stops a third.

**Branch `git-status-stage1`**, base `githubsucks/main` @ `4bc55e8`
(the #225 merge). **`githubsucks/git-status-stage1` is the
authoritative tip** — the ref, not a SHA. Recover with
`git fetch githubsucks && git checkout git-status-stage1`.

- **Framing `docs/git-integration-framing.md`, revision 5, APPROVED
  2026-08-09** after four review rounds. Every round found something
  the previous one had asserted without reading; the doc records which.
- **Scope:** `*git-status*` (a `listview` panel over
  `git --no-optional-locks -C <dir> status --porcelain=v2 --branch -z`)
  and `*git-diff*` (plain generated text, file-level, no hunk model).
  Plus **one additive `listview` change**: an optional `keys` table,
  install-once with match-on-reopen, because `Keymap::bind` refuses
  duplicates and the refresh path re-opens.
- **NO WIRE CHANGE**, and that is load-bearing for scheduling:
  `PROTOCOL_VERSION` is a strict serialization point, so this lane can
  run concurrently with other work. **Stage 2 (gutter markers) needs
  new `DecorationKind` variants and must be scheduled alone.**
- **Known negative coherence impact (§9):** git runs as a spawned
  process, and spawned processes do not appear in `*workers*` — that is
  `async.lua`'s job list. This adds a fifth unattributable background
  thing. Labelled honestly; a label is not attribution.
- **Verification plan** in framing §6. The load-bearing cases: an `AM`
  unborn fixture (two labelled patches), untracked diff rendering on
  **exit 1** (`--no-index` implies `--exit-code`), two successive
  refreshes not raising `DuplicateBinding`, and a non-UTF-8 path that
  parses and displays but **refuses** its gestures at the
  `String`-typed binding boundary.
- **Gates:** `scripts/gate --acceptance git_status_stage1_acceptance`.

**Implemented.** `builtin/runtime/git.lua` (new, loaded after
`linewrap.lua`), the `keys` extension in `builtin/runtime/listview.lua`,
one chunk-load line in `src/editor.rs`, and
`tests/git_status_stage1_acceptance.rs` (25 tests, one per §6 bullet).
No `pmacs-protocol` change, no `PROTOCOL_VERSION` change, no
`DecorationKind` change — the no-wire property held.

Five things worth carrying, all found by biting the suite rather than by
reading:

- **`listview.open`'s `seat_cursor` walks DOWN from wherever the cursor
  is**, on the premise that a fresh `switch_active_buffer` zeroed it.
  Re-opening an already-displayed panel — which is exactly what the
  async completion model does — zeroes nothing, so the walk lands one
  row *below* the previous cursor. The completion handler seats
  unconditionally from line 0 instead of trusting `open`.
- **A selection test that inserts ONE row above the selection is
  vacuous**, because that accidental off-by-one lands on the right row.
  The fixture inserts two.
- **`{:?}` on a Rust string containing NUL cannot build a `-z` fixture.**
  Debug renders NUL as `\0`, and Lua's decimal escape swallows the
  digits after it — so `\0` before a `1` record becomes
  `string.char(1)` and the record merges into its predecessor. One test
  passed while parsing nothing: the merged text landed in
  `# branch.head`, the panel header rendered it, and a `contains`
  assertion on the panel text was satisfied by the header. Payloads are
  joined in Lua with `string.char(0)`.
- **A path may contain a newline, so a panel row must escape it.**
  Parsing the bytes correctly and then writing them raw into a
  one-row-per-line buffer desynchronizes every line-to-row map — and the
  rope is UTF-8 by project invariant, so non-UTF-8 path bytes cannot go
  in at all. Rows render `\xNN` escapes; the raw bytes stay on the
  record, where the refusal check reads them.
- **Untracked rows sort AFTER every tracked row** in porcelain v2, so an
  untracked file cannot be used to reorder a list above a selection.

Two deliberate deviations from the framing's letter, both narrow:
`--no-color` on every diff invocation (a user with `color.ui = always`
would otherwise get escape sequences in a buffer with no ANSI parser
behind it), and `pmacs.git._program`, a module-local that the
missing-binary witness points at a name not on `PATH`. There is no other
in-process route to that branch: Rust's `Command` resolves the program
against the **parent** process's `PATH`, so a child `env` cannot hide
git, and `std::env::set_var` is `unsafe` in edition 2024.

## QoL arc retirement — PR #224 OPEN (docs only)

**PR #224** — https://github.com/levineuwirth/pmacs/pull/224. Written
**after** the PR existed rather than with the lane's first commit —
which is the standing correction from #171 and #215 being missed again,
and it took review asking. Recorded that way rather than quietly
back-dated: this file requires a lane for **every open PR**, including
the PR that retires other lanes.

- **Branch `retire-long-lines-lane`**, base `githubsucks/main` @
  `9a26ac8` (the #223 merge). **`githubsucks/retire-long-lines-lane` is
  the authoritative tip** — the ref, not a SHA, since any edit to this
  block advances past whatever SHA it records. Recover with
  `git fetch githubsucks && git checkout retire-long-lines-lane`.
- **Docs only.** Two files, `docs/active-work.md` and
  `docs/agent-handoff.md`. No `src/`, no crate, no manifest, no test
  changes.
- **Scope:** Rule 4 for the QoL arc, closed at #223. Remove the three
  merged lane blocks (long lines, Stage 1 #219, Stage 2 #220) **after**
  re-homing their durable residue to the handoff; advance the handoff's
  date and `main` anchor and this file's canonical-base record and
  recovery floor; add capability-aware keymap resolution as a named
  handoff §6 backlog item — cross-cutting, **not started**, needs its
  own framing.
- **Verification:** `git diff --check` clean. **The recovery path was
  re-exercised, not SHA-swapped** — fresh clone into an empty
  directory, `githubsucks` alias, `--prune` fetch, `9a26ac8` confirmed
  an ancestor of `githubsucks/main`, worktree recovered with the
  three-argument form; all four steps clean. Swept for dangling
  references to the removed lanes and their branches. The full gate
  suite is **not** re-run for a change that cannot reach it, and that
  is stated rather than left as a gap.
- **Retire this block in the next absorption after #224 merges.** It
  describes a docs PR; once merged there is nothing volatile left.

## Docs absorption after #217 — MERGED as #218 (2026-08-06 09:59Z)

**PR #218** — https://github.com/levineuwirth/pmacs/pull/218. **This
block was written with the lane's first commit, before the PR
existed**, so the row above was filled in rather than invented. That is the standing correction from #171 (153 commits of
drift while invisible here) and #215 (no lane until review caught it):
this file requires a lane for every open PR, and the way that stops
recurring is writing it now rather than after someone asks.

- **Branch `docs-absorption-217`**, base `githubsucks/main` @ `db1bbe9`
  (the #217 merge). `githubsucks/docs-absorption-217` is the
  authoritative tip — any edit to this block advances past whatever SHA
  it records, so the ref is the thing to trust. Recover with `git fetch
  githubsucks && git checkout docs-absorption-217`.
- **Docs only.** No `src/`, no crate, no manifest, no test changes.
  Gates run are fmt, `git diff --check`, `--lib`, and
  `listview_acceptance`; the full suite is not re-run for a change that
  cannot reach it, and that is stated rather than left as a gap.
- **Scope:** retire the tree and macOS-CI arcs per rule 4, re-home
  their residue first, file R5 and R6, and carry four durable lessons
  into the handoff.

### Not in scope

Diagnosing R5 or R6, or auditing the three readiness helpers — those
are the lanes this one creates, not work it does. Retiring the
CI-CRDT, Distribution, or reap-ledger lanes: each still owns undone
work and rule 4 does not apply to them.

## Empty-content readiness, a fourth and fifth instance — FOR THE R6 AUDIT

Found 2026-08-06 while gating this lane, recorded here because it
widens an existing lane's scope rather than starting one.

A loaded `--features crdt` run failed `m6_1_pty_raw_mode_disables_kernel_echo`
and `m6_1_pty_canonical_mode_keeps_kernel_echo` with
**`stty -a output was: ""`** — read-before-write on the child's output.
That is the **same family as R4** (readiness predicate satisfied by an
empty file) and **R6** (readiness file never published), and it means
the readiness-helper audit's scope is not just three `wait_for_file`
copies under `tests/`: `src/process.rs`'s own tests carry the shape
too.

Both passed isolated and the full suite was green on a quiet machine,
so this is load-sensitive and **undiagnosed** — recorded as a scope
note for the audit, not as a registry row: these were local, and the
registry judges red **CI** runs.

## Pre-checkout CI reds — a class the registry has no row shape for

Seen 2026-08-06 on #220, three times across two runs (`M4`+`M5`, then
`M5` again on the rerun):

```
Prepare all required actions
Getting action download info
Failed to resolve action download info. Error: Internal Server Error
##[error]Failed to resolve action download info.
```

**This is not a flake and not a test failure.** The job dies inside
`Set up job`, before `actions/checkout` — a `grep` for
`checkout|cargo|test result:` over the full job log returns **0**. No
repo code is fetched, so the red carries *zero* information about the
commit, in either direction.

Two things follow, and both matter for signal integrity:

- **Re-running is a first execution, not a retry-to-green.** The rerun
  rule governs a test that ran and failed; nothing ran here. The
  discriminator is objective and cheap — did the job reach checkout?
- **It cannot be a registry row as the registry is written.** Matching
  requires an *exact test selector* plus fragments, and there is no
  test. Recorded here rather than forced into a shape it does not fit.

The second `M5` failure took **4m52s**, which read like a real run;
the duration was entirely retry backoff. Duration is not evidence that
a job executed — **the log is**.

Whether `docs/ci-red-signatures.md` should grow a short non-row section
for this class is an open question for its owner, not something this
lane decided.

## Tree primitive (P5) — MERGED as #217; adoption is the open work

**The lane is gone, not the work.** Rule 4 removes a lane after merge,
and the primitive is merged: `listview` rows take optional `depth` and
`id`, collapse is primitive-owned, selection re-seats by id, and the
LSP outline is the one adopter. `COHERENCE.md` §14 is ◐ and §20 says
adoption rather than construction, which is where the remaining work is
recorded — **not here**, because none of it is in flight.

What review found is worth carrying forward, since all four were
invisible to a passing suite:

- **A selection test that toggled the root proved nothing.** The root
  sits on line 1 before and after collapsing, so it passed unchanged
  under the line-keyed re-seating that id-keyed re-seating replaced.
  A moving-node witness (`tr_4`) was the fix.
- **TAB was bound on every listview**, so flat panels lost their
  fall-through to the global binding and the Q#P3 read-only intercept.
  Delegation restored it.
- **`item` was effectively required.** `line_to_item` is sparse when a
  row omits the optional `item`, and `seat_cursor` took `#` of it — a
  display-only tree stranded the cursor on the header. Every existing
  test supplied `item`, so none could reach it.
- **"Opaque, compared by equality" was two contracts.** Selection uses
  `==`; collapse keys a table, which consults no `__eq`. Narrowed to
  string-or-number, and with it uniqueness and not-NaN, all enforced
  where rows enter.

The last two are the durable lesson and it is in the handoff: **an
optional field that a data structure's shape depends on is not
optional**, and a contract that two mechanisms must honour is only as
strong as the weaker mechanism.

## Leaked daemons from `gpu_invocation_acceptance` — NEEDS A LANE

**Found 2026-08-05 while cleaning up after the tree-primitive work. No
branch, no framing.**

- **42 orphaned `pmacs --daemon` processes** were resident on the
  development machine, **the oldest 3 days 23 hours old**. All had been
  **reparented to systemd** (`ppid=1`) and all had **deleted sockets**,
  so nothing could ever reach or reap them.
- **Source: `tests/gpu_invocation_acceptance.rs`** — the one-command
  tests, whose daemons carry `--socket <tempdir>/one-command.sock`. The
  tempdir is cleaned up; the daemon is not.
- **Rate measured, not estimated: 3 per sweep.** A single isolated
  `--features luajit,crdt` sweep leaked exactly three. 42 is what
  several days of sweeps accumulate to.
- **This predates the tree work** — the oldest is four days old — so it
  is a standing leak, not something a current lane introduced.

**Why it belongs to the reap-ledger family.** This is precisely the
shape that lane exists for: a process that outlives its supervisor with
nothing left watching it. The ledger arms only for `spec.group`, and
these are daemons spawned by a test harness rather than by compile mode,
so **nothing in the existing ledger covers them**.

**Why it matters beyond tidiness.** Dozens of resident daemons were
present during every local sweep run this week, including the one that
produced the unclassified failure recorded in the **tree-primitive lane
above** (and, in full, in that lane's framing §6a). That
makes them a **rival explanation** to the shared-target-dir mechanism
for that occurrence, and neither can be tested against it now — the
signatures were not captured. A leak that quietly changes the
environment of every subsequent test run is a measurement problem as
well as a resource one.

**First questions for whoever takes it:** does the test harness fail to
reap, or does the daemon fail to exit when its socket disappears? Those
have different fixes, and the second would be a product defect rather
than a test one.

## macOS CI signal integrity — ARC RETIRED (#215, #216); residue re-homed

**Both stages merged and the arc is done**, so rule 4 removes the lane.
Stage 1 built `docs/ci-red-signatures.md` and audited the incumbents;
Stage 2 retired **R2** and **R4** with discriminating witnesses. The
framing `docs/macos-ci-signal-integrity-framing.md` and the registry
both survive the lane — the registry is the durable artifact this arc
existed to produce.

**Retiring it required re-homing the residue first**, which is why this
did not happen at merge. A lane removed while it still owns undone work
does not close that work, it hides it. What it owned:

- **R1** (supersede cancellation budget, *measurement design*) → the
  **async-runtime lane**, below. Its retirement condition is Q#MCI3:
  replace or justify the measurement. Widening the budget would make it
  pass and measure nothing more.
- **R3** (live-leader EPERM, **UNRESOLVED — possible product defect**)
  → the **reap-ledger lane**, below, which already parks every
  disposition change pending exactly this question.
- **R5** and **R6**, added 2026-08-06 and neither diagnosed → the
  async-runtime lane and a **readiness-helper audit** respectively.

### The rows are the state now, not this block

An occurrence scan on 2026-08-06 (last 25 `main` runs: 23 green, 2 red)
turned up two things worth recording as method rather than as trivia.

**A signature very nearly got misfiled by theme.** `main` run
30710662474 is the same test as R3, with `EPERM` and
`measured_group=unobservable(ESRCH…)` — and R3 requires `leader=live`
where that run reads `leader=exited(signal SIGUSR1)`, which is R2's
exact fragment. Read by test name and shared fragments it looks like
R3; read by required fragment it is R2, four days before R2's
retirement, on the *other* macOS flavor. **Attaching a live unresolved
product-defect row to an occurrence of a retired test race is precisely
the error the exact-fragment rule prevents**, and it was caught by
checking the fragment rather than the resemblance.

**A second red matched nothing at all** and became R5, rather than
being folded into R1 because both involve supersede under a deadline on
macOS. Sharing a subject is not sharing a signature.

### Async-runtime lane — NOT STARTED, owns R1 and R5

No branch, no framing. Owns **R1** (Q#MCI3: the 50ms budget starts
before the second dispatch and is consumed by the test's own pump, so
it measures when the test was scheduled) and **R5**
(`stream_supersede_delivers_cancelled_to_on_close`, `async pump
deadline exceeded`, undiagnosed). Filed together because both are the
async runtime under a deadline; they are **not** assumed to share a
cause.

### Readiness-helper audit — NOT STARTED, owns R6

No branch, no framing. **Three independently written readiness helpers
now exist**, and they disagree: `vterm_stage2_acceptance`'s waits for
expected content (hardened by #216), `wait_for_published_file` was
fixed with it, and
`tests/bottom_panel_stage1_acceptance.rs:2446` carries only the
zero-byte half. R4's disposition predicted this recurrence under a new
selector, and R6 is it.

**Scope is the audit, not the call site**: how many helpers exist,
whether they can be one, what each promises. Patching `acc28` alone
leaves the question open under a fourth selector — which is the same
mistake as fixing `wait_for_file` and leaving
`wait_for_published_file`, already made once in this arc.

## CI CRDT coverage — MERGED (#209); kept for its three follow-ons

**Rewritten, not removed.** Rule 4 removes a lane when its ARC is done;
the coverage itself has landed but three named follow-ons have not.
Durable facts — the corrections, the traps, the census tool — are in
`docs/agent-handoff.md` §§1/5 per rule 3, so they are not repeated here.

- **MERGED as #209** (`main` @ `c5f7501`, 2026-08-01, two review rounds),
  framing `docs/ci-crdt-coverage-framing.md` revision 5. The lane had sat
  under "NEEDS A LANE" with no branch and no owner since #166.
- **CI compiles and runs the CRDT corpus for the first time.**
  `Test (crdt)` runs **3,766** tests where none ran before;
  `M10 Perf Gates (crdt)` covers two suites no workflow had ever named;
  `cargo clippy --features crdt` is enforced going forward rather than
  being a required local gate that rots in CI. **275 of 279 dark tests
  recovered**, the other four excluded with stated reasons.
- Branch `ci-crdt-coverage` retained; it carries nothing unmerged.

### Two CI weaknesses Stage 3 exposed — decisions, not defects

Both surfaced while gating #213 and belong to this lane because it owns
the crdt job.

- **CI does not pass `--no-fail-fast`, so a multi-suite break reports as
  a single-suite one.** `cargo test` halts after a failing binary, so
  #213's first crdt failure showed **one** suite when a local
  `--no-fail-fast` sweep of the same tree showed **thirteen**. The
  Stage 3 census hit the identical trap and recorded it; CI has it too.
  The cost of the flag is running the remaining suites on a red build,
  which is usually what you want when diagnosing.
- **The crdt job pairs the heaviest build with real-PTY deadlines.** It
  installs lavapipe, builds the full workspace with `crdt`, runs the
  largest test count, and includes real-PTY smokes with **5-second**
  waits. #213 saw two different such suites fail on two runs of the
  same commit. One of those was a real regression the flip caused; the
  other was load. **That ambiguity is the problem** — a job where noise
  and signal look alike trains people to rerun rather than read.
  **The triage half of this is now owned by
  `docs/ci-red-signatures.md`** — signature-keyed rows and a rerun rule
  that refuses to treat a green rerun as an all-clear. What remains here
  is the job-cost question: whether the crdt job should carry real-PTY
  deadlines at all.
  Candidate: longer deadlines for real-PTY assertions in this job
  specifically, or serialize the PTY suites.

### Still owned by this lane, not yet done

- **The macOS `crdt` leg.** Deliberately ubuntu-only at first, "decide
  about macOS from evidence." **That evidence now exists** and is
  favourable: the non-crdt macOS legs pass at 3,474 (thirteen fewer than
  ubuntu's 3,487 — `cfg`-compilation of the Linux-gated process tests,
  not lost coverage), and no crdt-specific failure appeared anywhere.
  This is now a small, decidable addition rather than an open question.
- **The `--lib --features crdt` flake.**
  `process::tests::setsid_escapee_is_not_reaped_and_teardown_reclaims_readers`
  failed ~1 run in 5 with `active_reader_probe` returning `None`.
  **It did not reproduce in #209's runs** — `--lib --features crdt` was
  2,081/2,081 and the full serialized sweep clean — but that is *not*
  evidence against the hypothesis: every run was `--test-threads=1` and
  the trigger was observed under **parallel** full-suite load. The
  `drain_until` explanation (draining for `Started` also ticks, and a
  tick can reap the leader before the following probe) remains an
  inference from control flow, not a falsified root cause. **Its own
  PR** — a product-defect hypothesis, where everything in #209 was
  workflow configuration.
- **`InstanceCapabilities::crdt_replica`'s serde default.**
  `#[serde(default = "default_true")]` is a *third* default mechanism,
  unconditional, and therefore disagrees with the `Default` impl in a
  non-CRDT build. Exercising it needs a self-describing format and
  `pmacs-protocol`'s only serde dependency is postcard, which is not
  one. Adding `serde_json` as a dev-dependency to test a divergence
  #209 did not introduce was refused as scope creep. Parked, not
  forgotten.
- **The two unattributed CRDT failures from #178's round-2 gating.** No
  test names were captured, so there is nothing to reproduce.

Recovery, only if a follow-on needs the branch:

```sh
git worktree add ../pmacs-ci-crdt \
  -b ci-crdt-coverage-followup \
  githubsucks/ci-crdt-coverage
```

## Distribution (P8) — STAGE 1 SHIPPED as v1.1.0 (#211)

**Rewritten as a lane rather than removed**: the arc is not done — Stage
1 was scoped to binaries-on-tag and everything else in §17 is untouched.
Framing `docs/distribution-stage1-framing.md` revision 3. Durable facts
are in `docs/agent-handoff.md` §1.

- **Released 2026-08-01.** `v1.1.0-rc.1` (prerelease) then `v1.1.0`,
  **both cut from the same commit `000b6cd`** — the #211 merge SHA — so
  the final release was built from byte-identical source to the one whose
  artifacts were verified.
- **Verified against the DOWNLOADED artifacts, both tags, not the build
  logs:** archive member lists (exactly `pmacs`, `pmacs-gpu`, README, two
  licenses), executable bits, absence of `pmacs-audit` /
  `pmacs_fake_lsp` / `pmacs_fake_mcp`, `pmacs --version` = `1.1.0` and
  `pmacs-gpu --version` = `1.1.0 (protocol v21)`, `SHA256SUMS`, the
  glibc floor, and CRDT presence **against a non-CRDT negative control**
  (1,576 `loro` strings shipped versus **0** in a control build — the
  control is what makes the number mean anything).
- **`pmacs` alone needs only glibc 2.34; `pmacs-gpu` needs 2.35.** The
  stated floor is the pair's, 2.35, not the more flattering single-binary
  number. That is why RHEL 9 (2.34) is excluded even though the editor
  binary would run there.

### Still owned by this lane, not yet done

Each is a stated non-goal of Stage 1 (framing §5), not an oversight.
**The next increment is a decision about which of these the project
wants, not a continuation of a plan.**

- **Channels** (stable/nightly), **in-place update**, **rollback**.
- **Signing and notarization.** macOS binaries are Gatekeeper-quarantined
  today; the release notes say so.
- **RHEL 9 and older glibc** — needs a container or cross-build, not a
  runner change.
- **Intel macOS**, **Windows**, **reproducible builds**,
  **package-manager distribution**.
- **Runtime dependency checking.** §17 asks first launch to identify
  optional external tools; `/bin/sh`, `stty`, git and tar are documented
  and never checked. That is §18 onboarding work.

## Discovery lane (P4) — STAGE 1 MERGED (#207); STAGE 2 IS NEXT

**Rewritten, not removed.** Rule 4 removes a lane when its ARC is done;
this arc is not — Stage 1 built the command surface and everything that
needs a Rust change is still ahead. Stage 1's durable facts are in
`docs/agent-handoff.md` §1.

- **Landed:** eleven `help.*` commands over the existing registries,
  indexed by `M-x help`, with `editor.describe-command` /
  `editor.describe-setting` kept as forwarders. No Rust, no protocol
  change. §5 moved substrate-without-surface → **Partial**.
- **Stage 2 candidates, in rough dependency order:**
  1. **Richer M-x rows** — a **protocol change**:
     `MinibufferPrompt.candidates` is `Vec<String>`, while
     `CompletionPopupRow` already carries `kind`/`detail`, so the wire
     pattern is solved and the bump is the work.
  2. **`Command` gains title / category / aliases / flags /
     arg-schema** — a Rust type change across ~147 definition sites.
     MCP currently works around the missing schema by stuffing rendered
     JSON into the description string.
  3. **Predicate evaluation.** `Command.predicate` is read at
     `src/help.rs:76` and one test, and **evaluated nowhere**. Starting
     to evaluate it makes commands stop being invocable, so it needs its
     own decision about what "unavailable" means at each call site —
     M-x, dispatch, menu. `discovery_acceptance`'s `d9` pins today's
     behaviour with a *raising* predicate, so that stage must change the
     pin knowingly.
  4. **Help-layer unification.** `src/help.rs` is still orphaned. Stage 1
     funnels every command through `pmacs.editor._show_help` and renders
     via named per-subject functions, so the work is enumerated:
     replace the four subjects `src/help.rs` covers (key, mode, hook,
     buffer) and **write three new Rust renderers** for settings, lists
     and apropos.
  5. **The help prefix.** Deliberately untouched by Stage 1 — the
     decision is one for the whole family, and `C-h` is **not** free
     (non-kitty terminals cannot disambiguate Ctrl+Backspace from
     Ctrl+H; both produce byte 0x08). `F1` / `C-c ?` / a rebind are the
     candidates.
- **Two Stage-1 facts a Stage-2 author needs.** Completion is
  **assistance, not validation** — `resolve_accepted_value` returns the
  literal typed text when no candidate is selected, so closed-set
  acceptance is unbuilt Rust work. And **`invoke_interactive` is not the
  M-x path**; the forwarders learned that the hard way in CI, since
  `pmacs.command.invoke` is a real caller of the old names.

## Generated-buffer immutability lane (Arc: workbench primitives) — STAGE 1 MERGED; STAGE 2 IS NEXT

**Framing #188 (revision 7) and Stage 1 #191 are both on `main` @
`4cd4a7b`.** Their durable facts are in `docs/agent-handoff.md` §1 —
including the contract-ownership rule (the framing owns the acceptance
criteria; an implementation adopts them and may not restate or narrow
them) and why `dired`/`listview` were the correct first two families.

- **Stage 2 is not started and has no branch.** It owns everything with
  new Rust in it: `Buffer::apply_generated_edit` + `GeneratedOutcome` +
  the `{ generated = true }` option and its `run_buffer_edit` arm;
  `set_generated_contents` reimplemented over it; Q#GB10's path-backed
  refusal and `mark_clean`; Q#GB15's `identity_protected`; Q#GB13/GB18
  for `compile.lua` and the search panel; Q#GB5's `ensure_slot` lock;
  the remaining 13 write sites; and the three `compile_mode_acceptance`
  intruder tests converted per Q#GB12.
- **It collides with dired Stage 2b**, which changes `paint`'s callers.
  Whichever starts second integrates first.


## Reap-ledger silent failures — MERGED (#202); kept for its parked follow-ons

- **Branch `reap-ledger-silent-failures`**, worktree
  `../pmacs-reap-ledger`, based on `githubsucks/main` @ `22df6ab`.
  `docs/reap-ledger-silent-failures-framing.md`, **revision 4**;
  approved at revision 3 after two review rounds (round 1: three
  blocking, two major; round 2: two blocking, two major; all accepted).
  Revision 4 records implementation findings, not a new design round.
- **All four bets resolved.** Bet 1 (every site takes a directed
  outcome) and Bet 2 (every consequence is reachable) hold. **Bet 3
  resolves the shutdown coupling as real and measured** — under 500ms
  with a failed force-kill plus an errored probe, versus the full 2s
  bound with only the force-kill failing. **Bet 4 is falsified: no
  reporting channel exists**, so reporting becomes its own lane.
- **The in-drain pin's first fixture was vacuous, and the bite caught
  it.** `poll_one` TERMs the group on leader exit, so an untrapped
  descendant died before writing its late marker — absent on *both*
  paths. With the seam reverted the pin failed only the consumed-plan
  check, never the content assertion. Fixed with `trap '' TERM` behind
  the readiness gate.
- **Gates: 10/10 green** on the pushed tree, all five bootstrap-storage
  variables controlled — fmt, diff-check, clippy, `--lib` (1888),
  `--lib --features crdt` (2073), compile-mode (67), copy-mode in both
  feature configurations (18/19), M4 with the basedpyright skip (149),
  required GPU (221). The five new process pins ran **15/15** as a
  repetition set, since supervisor tests are load-sensitive.
- **Unparked from PR #200's §5.** #200 retired the premise that
  justified the ledger's leniency and deliberately changed no
  disposition; this lane owns what it refused.
- **Now also owns R3**, re-homed 2026-08-06 when the macOS CI
  signal-integrity arc retired. `docs/ci-red-signatures.md` R3 is a
  group-directed `kill` returning **EPERM while the leader was observed
  live**, with `measured_group` — the one field able to disagree —
  unreadable. It is the **same group-target question** #176 and #200
  circled and this lane parks every disposition change pending, which
  is why it lands here rather than staying with a retired CI lane. Its
  registry entry is explicit that it is an **unresolved possible
  product defect** and that **a green rerun never retires it**; that
  constraint travels with the row, not with whoever inherits it.

  Note what it is *not*: R2 is the same test with `leader=exited(signal
  SIGUSR1)`, a test race, retired. An occurrence scan on 2026-08-06
  nearly filed a second R2 occurrence here as R3 on the strength of the
  shared test name and shared `EPERM` fragment. **`leader=live` is the
  fragment that separates a product-defect candidate from a fixed
  fixture bug**, and it is the whole reason the row lists it.
- **Four sites, not the two #200 named.** In the persistent ledger: a
  probe error of any errno drops the entry and cancels escalation; a
  failed escalating `SIGKILL` is marked as succeeded, so **no later tick
  retries it**; and `shutdown()` discards its own force-kill result the
  same way — on the path that exists specifically to stop a leak at
  editor exit. Those last two are **distinct, not cumulative**:
  `shutdown()` force-kills every entry with no `!entry.killed` guard, so
  a failed escalation still gets one attempt at exit, while a failed
  force-kill leaks the group past exit with nothing left to try.
  **Plus the in-drain
  twin** `final_drain_runtime`, which collapses every errno to "dead"
  while no tick runs; a false "dead" there cancels the readers, so its
  failure mode is truncated output rather than a leaked process.
- **The blast radius is exactly what the ledger exists for:** a
  TERM-ignoring descendant that outlived its leader with output
  redirected. Neither leader state nor reader state can see it; only
  group liveness can. A silent drop leaks the one process nothing else
  is watching. **Journey step 9 (build/test), not step 8** — the ledger
  arms only for `spec.group`, which spawn rejects for PTY mode, so no
  terminal reaches it; compile mode is the only production caller.
- **`shutdown()`'s final loop terminates when the ledger empties**,
  which happens via the same silent drop — so the probe error that hides
  a leak can also end the cleanup loop early. That coupling is why the
  probe cannot be made strict on its own. **Its precondition is
  `any_running()` already false**, so the fixture must be a leader that
  exited leaving a survivor; any other shape tests the other arm of the
  disjunction.
- **None of the three has been observed.** #200 saw an explicit
  `SIGTERM` fail in `signal()`, not a ledger call. The premise is
  falsified and the path exposed; the occurrence is not evidence these
  fire.
- **They are also untestable today**: `tick_reap_ledger` and
  `shutdown()` call `nix` directly and consult no injection seam, unlike
  `signal()`'s `forced_kill_errno`. All five existing ledger tests
  exercise the success path only. The seam must be **site-directed and
  multi-outcome**: `shutdown()` calls `self.signal()` before its ledger
  force-kill, so a single global slot would be consumed by the wrong
  call, and the coupling test needs two pending outcomes at once. The
  in-drain probe repeats every 1 ms but only cancels readers after 50 ms
  of false "dead", so it needs a directed full-drain override rather
  than a one-shot error. Test state is per-supervisor and shared into
  the drain context, never global; teardown proves its intended site was
  reached. The in-drain SIGKILL's local flag has no independently
  observable outer-path consequence, so it is named but not given a
  dead injection seam. The first PR is the seam **plus** the tests that
  exercise it — a seam without tests does not show it reaches the
  intended calls.
- **Diagnosis first, no disposition change proposed.** Stage A of the
  signal lane had three tolerance rules rejected across three revisions
  for the same shape of error on the same data structure.
- Recovery from a clean checkout:

  ```sh
  git fetch githubsucks
  git worktree add ../pmacs-reap-ledger \
    -b reap-ledger-silent-failures \
    githubsucks/reap-ledger-silent-failures
  ```

## Folding lane (Arc 6) — Stages 1 and 2 MERGED; Stage 3 (GPU) is next

Both shipped stages are on `main`; nothing in this arc is in flight. Stage 3
has **no branch and no framing yet**.

- Stage 1 (headless fold engine) merged as **#142**, Stage 2 (grid/daemon
  collapse) as **#149** — both under "Closed since the last snapshot".
- Retained, carrying nothing unmerged: branches `folding` / `folding-tui`
  and worktrees `../pmacs-folding` / `../pmacs-folding-tui`. The framings
  `docs/folding-framing.md` (rev 5) and `docs/folding-stage2-framing.md`
  (rev 4) are the approved artifacts Stage 3 re-scouts against.
- **Stage 3 (GPU) obligations, already named by the framings** — the
  starting point for its own framing doc: GPU collapse at TUI parity;
  caret/hit-test fold-awareness; the `BufferSnapshot` **fold-mirror clear**
  (parent R2-4 — without it, empty-after-revert diff suppression leaves
  stale folds on the GPU, the same trap class as #120); CRDT-origin and
  GPU-optimistic interactive unfold (parent R2-3); and flipping
  `FrontendView.fold_projection` to `true` for semantic frontends, which
  Stage 2 deliberately left `false` (Q#FD21).

## Parked lane: kill-ring browser + persistence

- Portable branch: `githubsucks/kill-ring-browser`
- Parked framing head: `503c489`
- State: framing only, revision 2; no implementation and no PR.
- Status: explicitly parked by the user on 2026-07-20.
- Its original scout was based on `0efb5cd`. The preserved framing marks
  this ground truth stale and requires a complete re-scout against the
  then-current `githubsucks/main` before implementation.
- Compile-mode has merged since the original scout, so old
  “compile-mode in flight” keybinding/touch-set assumptions are not
  authoritative.

Recovery worktree, only when the user un-parks it:

```sh
git worktree add --track \
  -b kill-ring-browser \
  ../pmacs-kill-ring-browser \
  githubsucks/kill-ring-browser
```

## Closed since the last snapshot

- **Process-signal diagnostic completeness — MERGED as #200**
  (`main` @ `a2a92bb`), atop Stage A #176. `docs/process-signal-diagnostic-completeness-framing.md`
  revision 6; framing approved at revision 4 after three rounds, then
  five review rounds on the implementation. Durable facts are in
  `docs/agent-handoff.md` §1. **Evidence collection only** — no
  tolerance rule, no retargeting, no disposition change.
  - **Bet 1 was falsified by CI and the framing's own fallback shipped.**
    `bash -m` diverges the terminal's foreground group on Linux and
    never on macOS. The divergent case is pinned by injection
    everywhere; a Linux-only corroboration drives a real shell and is
    the **only** test exercising `pty_foreground_group` end-to-end, so
    **on macOS that lookup has no end-to-end coverage**.
  - **Group identity remains unprovable**, and the pre-kill sample does
    not change that: moving `getpgid` before the `kill` removed a
    post-hoc reading, it did not make the reading contemporaneous.
  - **Still parked, each needing its own lane:** the reap ledger's
    silent cancellation (an EPERM probe drops the entry; a failed
    `SIGKILL` is marked killed) — **being scoped next**; retargeting to
    the measured pgid; any EPERM/ESRCH tolerance rule; Q#PS6; and
    `signal_target`'s read-then-kill of `tcgetpgrp` on the PTY path.

- **Six lanes removed by the 2026-07-30 absorption pass**, all merged,
  all with their durable facts in `docs/agent-handoff.md` §1:
  **#190** resource-op delete-guard implementation; **#196** dired
  Stage 2a (rename/delete reconciliation — Stage 2b remains, unstarted
  and without a lane); **#188** generated-buffer immutability framing
  (revision 7, the governing contract); **#194** silent-skip arming;
  **#195** CI timeouts and concurrency; **#197** the process teardown
  stdin deadlock.
  - **#194 and #195 kept their lessons in §3 and §5 rather than §1**,
    which is why a PR-number search of the handoff finds them only once
    each. That is sufficient under rule 3 — durable knowledge has a
    home, not a required section.
  - **A census by PR number is a proxy, not a measurement.** Counting
    `#NNN` in the handoff said five of these lanes had no record at all;
    counting by *content* found most already documented, with the real
    gap being the implementation PRs specifically (#190, #191, #196)
    while their framings were recorded. The absorption written from the
    first count would have duplicated existing entries.

- **Terminal configuration + copy mode arc — BOTH STAGES MERGED, lane
  removed.** Stage 1 **#173** (`main` @ `cf54270`, one review round) and
  Stage 2 **#178** (`main` @ `fe8b8ba`, **four review rounds**, twelve
  checks green on head `1b44c69` — verified by `head_sha`, not by the
  check summary), both 2026-07-26, both with no protocol change.
  Approved framing: `docs/terminal-config-and-copy-mode-framing.md` rev
  4, committed as the first commit of Stage 1's branch; its Q#TC6a
  carries a superseded-in-part box rather than a silent rewrite. Durable
  facts moved to `docs/agent-handoff.md` §1 (the arc bullet) and §4 (the
  `set_generated_contents` invariant) per rule 3 below, and to
  `COHERENCE.md` §14. **Stage 2 ships eight of nine criteria and the
  missing one is named** — criterion 17 needs a real GPU frontend, so it
  waits on the `a37` footing; the handoff records what it must assert.
  Branches `githubsucks/terminal-config` and
  `githubsucks/terminal-copy-mode` with worktrees
  `../pmacs-terminal-config` and `../pmacs-terminal-copy-mode` are
  retained. The gate-run flake found while gating #178 moved to the CI
  `crdt`-coverage lane above, which owns its discrimination.
- **Dired Stage 1 (the directory view) — MERGED as #165** (`main` @
  `c8ec8f3`, 2026-07-25, after one review round). pmacs has a directory
  surface: `C-x d` / `C-x C-j`, one read-only buffer per directory named
  `*dired:<canonical path>*`, a `dired` major mode carrying
  `RET`/`f`, `^`, `n`/`p`, `g`, `q`, `s`. No wire change (v20). The Rust is
  two things — a per-entry-tolerant `read_dir` (Q#DR6), which had to be
  Rust because `read_dir_blocking` fails a whole listing on any of five
  per-entry conditions and a tolerant wrapper cannot be written in Lua at
  all, and `normalize_buffer_path` going `pub` as
  `pmacs.path.canonicalize` (Q#DR2's preferred end state, so no Lua mirror
  exists and Stage 2 owes no mirror removal). The frozen m8_1/m8_2/m8_3
  counts are unchanged, which is the additivity gate. 15 claims
  bite-verified; one came back VACUOUS (acceptance 3c cannot pin descent
  routing — dired holds focus in its own panel, so dedication is the only
  discriminator) and is documented at the assertion rather than
  relabelled. Its branch (`dired-stage1`) and worktree
  (`../pmacs-dired-stage1`) are done; the abandoned `dired` branch
  (`ffdd642`, `../pmacs-dired-arc`) was superseded by a fresh cut and
  carries nothing unmerged. **Stage 2 (marks and operations) and Stage 3
  (wdired) each still need their own framing**, and the frozen fixture
  shrinks after Stage 3. Durable substrate facts and both new ops lessons
  live in `docs/agent-handoff.md` §§1/5; the implementation notes are
  `docs/dired-framing.md` §0, S1-1…S1-12. Two named forward items for
  Stage 2: `apply_resource_op`'s rename rebind is exact-PathBuf-equality,
  first-match-only, looked up with the raw path while stored paths are
  normalized — so a directory rename strands every buffer under it, and
  `pmacs.fs.rename` has zero production callers, so it can be fixed at
  the primitive; and Q#DR5's seam is the main-thread drain
  `AsyncRuntime::tick`, not `_take_result`, where rename settles as an
  undifferentiated `ReplyKind::FsUnit` and so must be keyed on
  `JobKind::FsRename`.

- **GPU terminal input (the double terminal-layout sync) — MERGED as #166**
  (`main` @ `b889873`, 2026-07-25, one review round, all twelve checks green
  after a macOS PTY-timing rerun). The dispatcher applied **both**
  terminal-layout syncs to **every** attached frontend; a semantic session
  satisfies both conditions, so its PTY was resized twice per tick forever and
  the child took a `SIGWINCH` storm that made a GPU terminal untypable while
  output still flowed. `sync_terminal_layout` is now split into a
  frontend-kind-neutral half (panel reconcile + controller liveness) and a
  grid-only geometry half, with the loop body extracted to
  `sync_terminal_layouts_for_tick` so the exclusivity is structural. No
  protocol change (v20). Durable lessons are in `docs/agent-handoff.md` §5;
  the framing (`docs/gpu-terminal-input-framing.md` rev 2) carries three
  falsified hypotheses, the two-pre-image bite matrix, and two named
  out-of-scope items (Q#GT5 interactive-shell echo on a raw PTY, which
  reproduces in-process and so is not the GUI/TUI asymmetry; and a geometry
  change appearing to clear the visible screen, which reproduces pre-fix).
  Branch `gpu-terminal-input` and worktree `../pmacs-gui-term-input` retained.
  **Its landed-doc pair MERGED as #168** (`main` @ `1b6a084`,
  2026-07-26): #166 recorded as landed, the CI `crdt`-coverage gap
  measured (**264 tests dark workspace-wide**, 177 in the library — a
  reading taken at `1b6a084` and kept here only as history. **The CI
  `crdt`-coverage lane above is the authority for the live figure**;
  do not quote this one forward), the
  vterm audit corrected — "only 3 of 9 acceptances drive a real daemon"
  was optimistic; without the frontend binary the honest number is
  **2** — and the a37 findings folded into the coverage lane.
- **Inline-math slice — MERGED as #158** (`main` @ `5aa9044`,
  2026-07-25). Detect → parse → layout → draw for `$…$`, entirely inside
  `pmacs-gpu`, no protocol change. Verified by the user's manual pass on
  a real paper after the landing. What is worth carrying forward:
  - **The v0 subset is 34 Greek symbols, sub/superscript, and `\frac`.**
    An unsupported command fails the **whole span** back to source, so on
    a real document most inline spans still show LaTeX. Widening the
    symbol map is the highest-value next increment — ahead of display
    math, which is also deferred.
  - **A stale frontend binary is invisible from the source tree.** The
    slice lives only in `pmacs-gpu`, so after it merged the feature was
    absent until `cargo build --release -p pmacs-gpu` and a client
    restart; the daemon needs neither. Diagnose with `strings` on the
    binary (`Latin Modern Math`, `MathBox`) rather than by re-reading the
    checkout, which was already current.
  - **Main was integrated three times in one day** (`8c86d34`,
    `46a1b8f`, `b889873`), merged not rebased to preserve review anchors.
    Two conflicts, both this ledger and nothing else. **The dangerous
    case was the one that did NOT conflict**: #166 auto-merged into
    `pmacs-gpu/src/main.rs`, the file this lane rewrites, because the two
    edits sat in different regions of it. Decide whether to integrate
    from the shared-**file** set, never from whether git complained.
  - **Integration was proved by test-count reconciliation**, not by a
    green run: predict what the other side adds, then check the deltas.
    GPU 199→202 matched `e547a90`'s 3; later lib 1,826→1,829 and CRDT
    2,003→2,006 matched #166's 3, with GPU unchanged because #166 adds
    none. Suites 91→92 was #161's new binary.
  - **Why the branch had no CI for a day**: a conflicting PR builds no
    merge ref, so no `pull_request` run is created. The ledger previously
    recorded this cause as unidentified; it is not. Check `mergeable` and
    confirm a run exists for the current head SHA.
  - **`m4_5_basedpyright` has no timeout and hangs forever**, parking a
    `--workspace` sweep (observed 2h26m at 38 of 92 suites). It is
    **intermittent**, so an earlier clean sweep proves nothing. Sweep with
    `cargo test --workspace --no-fail-fast -- --skip basedpyright` and
    judge progress by whether the suite count advances.
  - Named v0 approximations: the peer-caret half of acceptance 14 is
    pinned at the mapping level, not pixels; a soft-wrapped spacer draws
    its box whole at the first run's origin; the fit budget reads the
    bundled code face even under a custom `set_font` family.
- **Bottom panel Stage 1 — MERGED as #155** (`main` @ `e745068`,
  2026-07-24, after two review rounds). Window placement, window
  parameters, TUI side windows, the divider, and the adopter `display`
  opt-in, with no protocol change. Both rounds found the same class of
  defect and are worth keeping:
  - **Round 1**: the Q#BP6 side-window split guard had *no production
    caller* — `C-x 2` still reached plain `split_active` — and survived
    because the acceptance test called the core method directly.
  - **Round 2**: Q#BP7's terminal growth re-arm had *never been
    implemented*, and the assertion meant to pin it (`at_bottom`) is a
    geometric readout that a still-anchored view satisfies; the anchor
    assertions beside it compared `""` with `""` because the PTY fixture
    emitted LF-only output.
  - **Post-round-2 self-review**, caught by CI going red on all four Test
    jobs: resolving `pmacs.window.buffer()`'s no-argument arm through the
    acting frontend made a total function partial, and six runtime modules
    silently dropped operations (`kill_ring_acceptance` 30/30 → 25/5).
    Fixed in `9110f9f` before merge.
  - Gating fact found on the way: an isolated `XDG_CONFIG_HOME` prevents
    the real user `init.lua` from installing a local package and leaking
    a status message into painted-frame comparisons. **That isolates the
    observed config symptom only, not the gate:** the ambient-root lane
    above establishes that data/state/cache must be controlled too.
    There is also a latent pre-existing `main` bug in the buffer CRDT
    undo path, unrelated to this arc.
  - `compile_mode_acceptance` is load-sensitive under default
    parallelism (~1 run in 3, a different test each time); verified
    pre-existing by swapping in `main`'s `compile.lua`. It is 67/67 at
    `--test-threads=1`.

- **GPU initial target — MERGED as #148** (`main` @ `0dd16a5`, 2026-07-24,
  after two review rounds). `pmacs --gpu [--socket …] FILE` opens a target
  before the GPU window appears. Protocol bumped 19 → 20: a semantic-session
  `SessionBootstrapRequest` after `AttachRequest`, plus an appended
  `InstanceMessage::InitialTargetResult` pre-window readiness barrier; v6–v19
  wire encodings are unchanged. Root owns launcher tilde/cwd resolution and
  exact raw-byte path transport; the daemon resolves/dedups/loads the target
  and runs load/switch hooks inside one dispatcher transaction, then
  publishes CRDT-upgraded targets to existing grid replicas (gated on
  `upgraded_to_crdt`, independent of the load/create outcome, so a dedup onto
  a hidden not-yet-backed buffer still reaches pre-attached replicas — round
  2 finding). Semantic replicas receive a publication only when displaying
  that buffer, so a second target launch cannot switch an existing GPU
  window. Round 2 also closed a failure-containment gap: every dispatcher-side
  bootstrap failure now shuts down the socket (a dropped write-half clone
  does not close a shared FD), and the dispatcher drops any event from a
  session that was never installed, rather than reaching absent render/size
  state. Integrated cleanly with Folding Stage 2 (#149): fold projection at
  attach is selected from the same negotiated `semantic_render` bit the
  target bootstrap uses. Its lane, worktree (`../pmacs-gpu-initial-target`),
  and branch (`gpu-initial-target`) are done; the `-framing` branch is kept.
  Durable substrate facts and both review-round lessons live in
  `docs/agent-handoff.md` §§1/5 and `docs/gpu-initial-target-framing.md`
  rev 3.

- **Folding Stage 2 (grid/daemon collapse) — MERGED as #149** (`main` @
  `6ed4fe9`, 2026-07-24, after **five** review rounds). The grid TUI now
  renders collapses. Spine (Q#FD12): `src/fold_view.rs`'s `VisibleLineMap`,
  derived from the fold store plus a window's line offsets and **never
  stored**, threaded as `Option<&'a VisibleLineMap>` on a lifetime-bearing
  `Viewport<'a>` that stays `Copy`. No wire schema or protocol change; the
  GPU path is Stage 3. 48 acceptance tests on the real `paint_frame` grid,
  every behavioral claim bite-verified. Durable design points, each a trap
  Stage 3 inherits:
  - the map's unit is a **merged hidden component** (overlapping *or
    adjacent* intervals unioned, keeping the earliest visible head), not a
    fold — folds may cross, and a later fold's own head can be hidden;
  - instances are **per rendered window** and **per command/event
    operation**, never per frame; a command's map follows the operation's
    **target** window, since a wheel event names a pane without activating it;
  - fold projection is **per-frontend** (`FrontendView.fold_projection`) —
    shared `EditorCore` motion would otherwise make a simultaneous unfolded
    GPU session's cursor skip lines it still displays;
  - a hidden cursor normalizes by **position**, not row, and `set_view_top`
    clamps in the setter rather than being repaired at render time;
  - the interactive-Lua unfold keys on the **post-intercept** edit site — a
    managed buffer intercept may legally relocate the op.

  Process notes worth keeping: `main` moved under the arc, and the merge was
  textually clean but **not semantically clean** (#146 added `Viewport`
  literals the new `folds` field invalidated) — a clean `git merge-tree` does
  not mean the merged tree compiles. CI was red at review on the macOS/luajit
  `outline_5_level_100_entry_renders_within_100ms` budget flake and went
  green on rerun.

- **Documentation ledger refresh — MERGED as #147** (`main` @ `0a479ae`,
  2026-07-24). The #142 housekeeping, expanded after review found the ledger
  stale through four merges rather than one. Its own macOS/luajit red was the
  vterm `VTERM_ALT_READY` PTY timeout; green on rerun.

- **Web grammars HTML + CSS — MERGED as #146** (`main` @ `47581f4`,
  2026-07-23). `.html/.htm/.xhtml` and `.css` highlight off the official
  `tree-sitter-html` 0.23 / `tree-sitter-css` 0.25 crate query constants (no
  in-repo overlay), and HTML's `INJECTIONS_QUERY` lights up `<script>` → js
  and `<style>` → css. Durable lesson recorded in
  `docs/web-grammars-html-css-framing.md`: the `highlight.rs` capture table
  is **global**, so adding a capture name retro-paints every other language —
  check the reverse direction and pin it.

- **LaTeX Stage 1 — MERGED as #144**, with its parent inline-math framing
  committed as **#145** (`main` @ `f09b0a1`, 2026-07-23).
  `.tex/.latex/.sty/.cls` highlight via `codebook-tree-sitter-latex` 0.6 plus
  the first in-repo query overlay (`builtin/queries/latex/highlights.scm`,
  `include_str!`) — the reusable pattern for grammars whose crate ships no
  usable queries. The crates.io `tree-sitter-latex` is provably broken (no
  `scanner.c`). The math parser and Tiers 3–4 are deferred to the inline-math
  arc.

- **Folding Stage 1 (headless fold engine) — MERGED as #142** (`main` @
  `c49a8c7`, 2026-07-23, after three review rounds; round 3 clean). The
  instance-side fold store + translating/dropping `View`, the structural
  source (derived head line, closer-aware tail), the Lua data API +
  interactive `C-c @` commands, the command-path pre-edit unfold, and
  authoritative-empty `FoldState` production landed with no protocol bump.
  The `folding` branch and worktree (`../pmacs-folding`) are retained but
  carry nothing unmerged; the `folding-framing.md` framing is preserved.
  CI red at merge was an unrelated environmental perf flake
  (`outline_5_level_100_entry_renders_within_100ms`, macOS/luajit only),
  green on rerun. Stage 2 has since merged as **#149** (above); durable
  substrate seams live in `docs/agent-handoff.md` §1.

- **Vterm Stage 3 (protocol v19 + GPU terminal) — MERGED as #135** (`main`
  @ `cac4961`, 2026-07-22, after two review rounds). Arc 5's terminal stage
  is complete (compile mode #113, Stage 1 #126, Stage 2 #130, Stage 3 #135).
  Its lane, worktree (`../pmacs-vterm-gpu`), and branch are done; durable
  substrate facts live in `docs/agent-handoff.md` and `docs/vterm-framing.md`.

- **Branches deleted 2026-07-22 (authorized):** `vterm-stage3-framing`
  (Revision 8 framing; its content is carried on `vterm-gpu`, verified as a
  superset before deletion — the branch was NOT an ancestor of `vterm-gpu`
  because the framing was copied rather than merged, so it needed a forced
  local delete) and `tab-width-parity` (a clean ancestor of `main` via
  #137). Both removed as worktree + local ref + `githubsucks` ref; the
  `origin` tracking refs were pruned. The `-framing` branches for each are
  deliberately kept.

- **Tab-width rendering parity — MERGED as #137** (`main` @ `2625ec7`,
  2026-07-22). One fixed 8-column `TAB_STOP_COLUMNS` in `pmacs-protocol`
  now drives core/TUI columns, GPU code projection, and minimap width;
  source bytes and protocol ranges are unchanged. Its lane, worktree, and
  `tab-width-parity` branch (local + `githubsucks`) are deleted; the
  `tab-width-parity-framing` branch is kept. This closes the long-standing
  "tab width is a rendering-parity
  bug, NOT a config gap" deferral recorded in `docs/agent-handoff.md` §5.
- **Locals-query processing — MERGED as #134** (with handoff #136), and
  **modeline detection handoff #133**. Both landed between this lane's
  base and its canonical-main integration.

- **Config registry — MERGED as #127** (`main` @ `2e37c04`). Its lane
  (`config-registry`, worktree `../pmacs-config-registry`) is done; the
  branch is kept but carries nothing unmerged. Durable substrate facts
  moved to `docs/agent-handoff.md` §1 per rule 3 below.
- Both this and Vterm Stage 1 ran as **concurrent lanes in sibling
  worktrees off `main`**, with the shared files (`src/editor.rs`,
  `src/lua_bindings/mod.rs`, `src/lib.rs`) assigned to one lane each in
  advance. The rebase of the second lane onto the first had **zero
  conflicts** — worth repeating for future parallel work, along with its
  precondition: agree the file split before either lane starts, and keep
  each lane's footprint in the other's files to a single line.

## Update protocol

Whenever a listed lane changes materially:

1. update its public branch and head/state here;
2. record new verification and remove superseded caveats;
3. keep durable architecture in `docs/agent-handoff.md`, not here;
4. remove the lane after merge or abandonment;
5. verify every recovery command from a clean worktree before calling
   the transfer complete.
