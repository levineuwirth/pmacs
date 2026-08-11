# LSP file watcher D3 — the polling cost — framing

**Status: revision 3 — DRAFT, awaiting review. No implementation may
begin from this document.**

Continues issue #233, which stays open until this lane closes it. D1
and D2 — matching correctness and the re-registration leak — merged as
**#234** (`ae84d58`); this frames the remainder the user ruled next on
2026-08-11: the watcher is now *correct* and still walks everything,
every tick, forever.

## Review round 1 — five findings, and what each changed

Revision 1 was reviewed 2026-08-11 and did not survive it. Recorded
here because three of the five are cases where the framing reasoned
from the wrong mechanism, which is exactly what a framing review
exists to catch before code does.

- **P1 — the promised idle state was impossible.** Revision 1
  promised "one brief `walk_tree` blip every four seconds"; but
  `pmacs.workers.sleep` allocates a **running job for its whole
  duration** (`dispatch_sleep`, `src/async_runtime.rs:1022-1037` —
  the job sleeps in 1 ms slices on a **pool thread**), and
  `activity_summary` counts every `Running` job
  (`src/async_runtime.rs:1570`). A 4 s backoff sleep would render as a
  *constant* `⋯1 sleep 4000ms`, and filtering sleeps from the
  indicator would touch the instrument this lane declares out of
  scope. **Revision 2 removed the sleep from the design entirely**
  (see "The cadence" below) — the fix is the codebase's own idiom,
  not a new mechanism.
- **P1 — the scan root must be the server's, not a freshly detected
  one.** Revision 1 proposed `pmacs.project.detect` for the
  string-form base. Server rooting honours **configured strings and
  custom resolvers first** (`project_root_for`,
  `builtin/runtime/lsp.lua:783`), and the bundled texlab entry
  documents why its root *cannot* be `project.detect` (Q#LX2,
  `lsp.lua:284` — texlab wants the document root, not the repository
  root). The server's own `root_uri` and `cwd` are already exposed
  (`pmacs.lsp.list`, `src/lua_bindings/mod.rs:10985-11002`;
  `root_uri` is the spec field verbatim, nil when the server never
  asked for a root). **Revision 2 rooted the scan at what the
  registering server actually serves**: `root_uri` → `cwd` →
  attached-file directory, in that order.
- **P1 — coalescing needs registration-epoch semantics.** Revision 1
  said "route the shared diff through every watcher" without defining
  which watcher set owns a diff. A watcher registered between two
  snapshots would receive a false CREATED for a file that predates
  its registration; membership changing during a walk recreates
  either #234's stale-watcher batch or the same pre-registration
  event. **Revision 2 defined shared snapshots with per-watcher
  baselines**, refined by round 2 below.
- **P1 — "VCS-only exclusion is safest" was wrong.** A hard skip
  silently ignores a server that legitimately registers `.git/HEAD`
  or `**/.git/**`, and glob semantics mean even `**/*.rs` *can* match
  under `.git/` — so any unconditional exclusion is a deviation from
  the registered contract, not a safe default. Revision 1 also
  overweighted the win: exclusion was a **job-count** lever when every
  directory was a separate job, and the walk primitive removes that
  economics. **Revision 2 defaults to no unconditional exclusion.**
- **P2 — the arithmetic used the issue's machine, not this one.**
  With D = 220 on this checkout it is 220 `read_dir` jobs **plus one
  sleep** per watcher per tick — 221, or **1,326 across
  rust-analyzer's six watchers** — and revision 1's proposed steady
  state was itself two jobs (sleep + walk), not one. Corrected
  throughout.

## Review round 2 — the scheduler was underspecified

Round 2 (2026-08-11) closed the round-1 findings and found the
cadence's own semantics missing: revision 2 said *when* scans become
due but not what happens when due-ness, in-flight walks, joins, and
retirement collide.

- **P1 — a joining watcher must force an immediate baseline scan.**
  Revision 2's baseline was "the first snapshot completed after
  join" — but a backed-off group's next snapshot can be 4 s away, so
  a file created after registration and before that delayed snapshot
  would fold into the baseline and never be reported. Today,
  registration begins its initial scan immediately (`lsp.lua:2074`);
  the coalesced design must preserve that. **A join pulls the group's
  `next_scan_at` to now; if a walk is already in flight, exactly one
  immediate follow-up scan is queued.** The baseline is sharpened to
  match: a snapshot serves as a watcher's baseline only if its **walk
  started after the join** — an in-flight walk may have passed a
  directory before a pre-join file appeared there, and using its
  snapshot as a baseline would turn that file into a false CREATED on
  the next diff.
- **P1 — single-flight and retirement were undefined.** The document
  itself establishes that walks can outlive their interval, so
  "every due group starts a scan" permits overlapping walks —
  restoring multiple jobs, completing snapshots out of order, and
  making the epoch state ambiguous. **The group scheduler below is
  now a defined state machine**: one in-flight scan per group,
  deadlines advanced from completion, stale completions rejected,
  and retirement (last member gone, or server death) that
  cooperatively cancels the walk — which puts cancellation into
  `walk_tree`'s contract and tests.
- **P2 — `⋯1` was an overclaim.** Groups are keyed by
  (server, base), so several can be due on the same frame, and
  per-group single-flight still permits `⋯N`. **The bar is restated
  accurately**: absence at idle; while scans run, one attributable
  job per concurrently due group. Global serialization is offered as
  the alternative under Q#D3-1 if `⋯1` must be guaranteed.

## Verified against the tree at `add0ba1`

Every claim below was read or measured this session (revisions 2 and
3 re-verified their corrections against the code).

- Each registered watcher is its own coroutine looping
  `sleep(FILE_WATCH_INTERVAL_MS)` → `scan_tree` (`lsp.lua:1924`,
  `:2074-2083`); the interval is 250 ms. Registration's initial scan
  runs immediately (`:2074-2075`).
- `scan_tree` awaits `pmacs.fs.read_dir` once **per directory**
  (`:2038-2041`), one async job each. `walk` recurses
  unconditionally; `matches` gates only whether an entry is
  *recorded*.
- **A sleep is a pool-occupying running job.** `dispatch_sleep`
  dispatches `run_sleep` onto the worker pool
  (`async_runtime.rs:1022-1037`), so every sleeping watcher holds one
  of the pool's `available_parallelism - 1` threads for the full
  interval — the hazard `autosave.lua:133-139` documents in so many
  words. Twelve pre-#234 rust-analyzer watchers were twelve
  mostly-sleeping pool threads.
- **Jobs per tick per watcher = 1 sleep + D read_dirs.** On this
  checkout D = 220, so 221 per watcher and **1,326 per tick for
  rust-analyzer's six watchers**.
- **250 ms is a floor, not a period.** The awaits are sequential, so
  a tree whose walk takes longer than the interval makes the
  effective period scan-bound — the loop never idles. The issue
  measured ~270 ms on a two-directory tree; D round trips dominate on
  real trees.
- Measured on this checkout: **220 directories, of which `.git` is
  177 — over 80 % of the walk**. (The issue's machine carried an
  in-tree `target/`: 187 directories, 46 outside `.git`+`target`.
  This machine exports an external `CARGO_TARGET_DIR` and still pays
  `.git`.)
- **The string-form base is nondeterministic, found while framing:**
  `resolve_watcher`'s string arm takes the directory of the FIRST
  attachment `pairs()` happens to yield (`:2150-2173`) — table
  order, not a chosen root. #234 made matching correct *per base*;
  **which** base is still accidental.
- **The codebase already has a no-job cadence idiom with five
  adopters.** `process.after-tick` fires every frame, including idle
  frames (the run loops tick on a frame *timeout*), and
  `pmacs.editor.monotonic_ms` is the clock built for exactly such
  loops (`lua_bindings/mod.rs:13833`). `autosave.lua`'s Q#AS2 sweep
  is the model: one clock read and one compare per frame, no job, no
  pool thread.
- **`pmacs.hook.remove` does not exist** (the P3 prerequisite gap,
  `docs/agent-handoff.md` §1a) — an after-tick subscription is
  permanent, so the scheduler installs **once** and early-returns
  when it owns no groups, exactly as autosave's does when disabled.
- **Cooperative cancellation is the established job shape**: every
  job body in `async_runtime.rs` polls `cancel.is_cancelled()` at
  its work boundaries (`run_sleep` per slice, the others per unit);
  `walk_tree` polling between directory reads inherits the pattern.
- **No `notify`/inotify dependency in the tree** and **no
  ignore-list infrastructure to reuse** — both re-verified, both
  carried from the D1/D2 framing.

## What §9 asks of this lane

Not "quiet the modeline." The indicator is the instrument that found
this, and the churn it shows is real; quieting it is explicitly out
of bounds. The lane's job is to make the background work **small,
attributable, and honest**: at idle there should *be* no running
background work to report, and while scans run each should be one job
named for its root.

## The cadence — after-tick deadlines, not sleeps (round 1)

The per-watcher sleep loop is replaced by the Q#AS2 idiom: one
`process.after-tick` subscription owns every scan group's schedule.
Per frame it reads `pmacs.editor.monotonic_ms` once and compares each
group's `next_scan_at`. Waiting allocates **no job and no pool
thread** and renders **no indicator segment** — `activity_summary`
returns `None` at zero by contract. While a scan runs, the indicator
honestly shows its job.

The subscription installs once at module load and early-returns when
no groups exist — it cannot be removed, because `pmacs.hook.remove`
does not exist, and a guard is the house answer (autosave's
`enabled` check).

### The group state machine (round 2)

Per group — keyed (server, base) — the scheduler holds
`next_scan_at`, the backoff interval, an **in-flight flag**, a
**scan generation counter**, and a `rescan_queued` bit.

- **Single-flight.** The after-tick check skips a group whose walk is
  in flight; a group cannot become due against itself. Concurrent
  walks, out-of-order snapshots, and ambiguous epochs are therefore
  unrepresentable, not merely avoided.
- **Deadlines advance from completion.** On scan completion,
  `next_scan_at = completion time + current interval`. A walk that
  outlives its interval degrades to back-to-back scans with a full
  interval between them — never to overlap.
- **Stale completions are rejected.** Each scan carries its group's
  generation at start; a completion whose group is retired, or whose
  generation is not the group's current one, is dropped before any
  state write or emit — #234's P2 recheck, applied at group scope.
- **Joins wake the group.** A watcher joining sets
  `next_scan_at = now`. If a walk is in flight, `rescan_queued` is
  set instead, and completion of the current walk starts **exactly
  one** immediate follow-up scan. The joiner's baseline is the first
  snapshot whose **walk started after its join** (see below), so the
  baseline is at most one walk-duration away — never a backoff cap
  away. The join-triggered scan does **not** reset the backoff curve;
  only observed changes do.
- **Retirement.** When the last member leaves (unregistration, or
  supersession with no successor) or the server dies, the group
  retires: the in-flight walk's job is **cancelled cooperatively**,
  its completion is rejected by the generation rule, and the group's
  schedule entry and snapshots are dropped. A re-registration that
  replaces members keeps the group alive — the superseded members
  are cancelled per #234's D2 and the new members join as above.

## The scan root (round 1)

For a string-form (bare `*.txt` / absolute) registration the base
becomes, in order: the server's **`root_uri`** (spec verbatim — nil
when the server never asked for a root), the server's **`cwd`**, and
only then the attached-file directory. These describe the workspace
the registering server actually serves — including configured roots
and custom resolvers like texlab's, which `pmacs.project.detect` can
never reproduce (Q#LX2). This replaces the `pairs()`-order accident
with a deterministic, server-owned answer. `RelativePattern`s keep
their own `baseUri`, unchanged.

## Coalescing, with registration epochs (rounds 1 and 2)

One scan group per (server, base). The group's scanner records
**all** files (the matcher moves from scan time to diff time); each
completed scan increments the group's **snapshot epoch**.

Delivery semantics:

- Each watcher's **baseline is the first snapshot whose walk started
  after it joined** — an in-flight walk may have passed a directory
  before a pre-join file appeared there, so its snapshot cannot serve
  as a baseline (round 2). A watcher receives diffs only between
  snapshots at or after its baseline. A file created after the
  group's previous snapshot but before a watcher joined therefore
  produces **no event for that watcher** — folded into its baseline,
  exactly as the initial scan folds pre-existing files today.
- **Membership for delivery is captured at scan start**; a watcher
  joining mid-walk waits for its queued baseline scan.
- **Cancellation is rechecked per watcher at emit time** — #234's P2
  rule per member: a watcher superseded or unregistered during the
  walk emits nothing, and its replacement has no baseline yet, so it
  emits nothing either.
- Changes passing a watcher's matcher and kind mask are deduped by
  `(uri, type)` into the server's single
  `workspace/didChangeWatchedFiles` notification, as today.

## The walk primitive

`pmacs.fs.walk_tree(base)` — the whole recursive walk as **one job**
instead of one per directory: 220 `read_dir` jobs per scan on this
repo become 1. The indicator shows one purpose (`walk_tree <root>`).
An additive fs binding plus its async-runtime job; **no wire change**
(fs bindings are not the frontend protocol) and no new crate. Two
contract clauses, each with its own Rust tests:

- **Symlinks are recorded, not traversed** — `scan_tree`'s
  loop-safety, preserved.
- **Cancellation is cooperative and prompt**: the job polls its
  cancel token between directory reads (the established
  `async_runtime.rs` job shape), so group retirement mid-walk stops
  the walk instead of orphaning it.

## Exclusions (round 1) — none by default

Glob semantics make any unconditional skip a contract deviation:
`**/*.rs` compiles with a separator-spanning prefix, so it *can*
match under `.git/`, and a server may register `.git/HEAD` outright
(branch-watching tools do). The only semantics-preserving default is
**no unconditional exclusion**, and with the walk primitive the
economics support it: exclusion was worth 80 % of the *job count*
when every directory was a job; inside one `walk_tree` job it is only
readdir syscalls, and the whole 220-directory walk is a few
milliseconds of one pool thread per scan.

The option space, for Q#D3-2: (a) no unconditional exclusion — the
proposed default; (b) **opt-in** exclusion through configuration, for
users with pathological trees, framed explicitly as a
watcher-contract trade; (c) matcher-aware pruning — skip a subtree
only when *no* active watcher's pattern can match under it — sound
but almost never fires against real registrations, because
`**/`-leading globs can match anywhere; (d) a hard built-in VCS
skip, which revision 1 called "safest" and is not: it is (b) without
the opt-in.

## Idle backoff

The interval doubles while consecutive scans observe no change,
capped at 4 s; any change batch resets it to 250 ms. Under the
after-tick cadence a longer interval costs *nothing* while waiting —
backoff bounds **scan frequency**, not sleep-job length. Worst-case
latency for an external change at idle equals the cap **except at
registration, where the join rule forces an immediate baseline**
(round 2). LSP imposes no latency bound, and edits made through
pmacs never depended on the watcher (the server sees `didChange`).
The watcher exists for git checkouts, generated files, and other
editors.

## Deliberately staged separately — kernel notification

`notify` (inotify / FSEvents / kqueue) eliminates polling. Also: a
new dependency, a new Rust subsystem, a Lua binding, a platform
matrix, and an interaction with §9's ownership model. Staged as its
own framing — not because it is wrong but because everything above is
a pure win it does not obsolete (a kernel watcher still needs the
initial scan and a polling fallback), and a new-crate decision
deserves its own review.

## Proposed shape — Stage 1

After-tick cadence with the group state machine + walk primitive +
coalescing-with-epochs + backoff; no exclusions by default;
server-owned scan root.

At rest on this repo with rust-analyzer attached: **from 1,326 jobs
per scan-bound tick (six of them pool-thread-holding sleeps) to zero
jobs at idle**, with one `walk_tree` job per group for the few
milliseconds each scan actually runs — at most every 250 ms under
activity and every 4 s at rest, immediately once at registration.

## Open rulings — each blocks implementation

- **Q#D3-1 — the acceptance bar, stated accurately (round 2).** At
  idle the indicator is **absent** (no running job exists —
  `activity_summary`'s `None`-at-zero contract). While scans run it
  shows **one attributable job per concurrently due group** — `⋯N`
  when N (server, base) groups are due on the same frame, each named
  for its root; a typical single-project session has one group.
  Alternative if `⋯1` must be guaranteed: a global scan queue
  serializing walks across groups, at the cost of coupling one
  server's scan latency to another's tree size. Which bar?
- **Q#D3-2 — exclusions.** Proposed: none by default, with opt-in
  exclusion as a documented contract trade (option b) if a user
  asks. Confirm, or rule for one of (b)/(c)/(d) above.
- **Q#D3-3 — the scan root.** Proposed: server `root_uri` → server
  `cwd` → attached-file directory. This widens the watched tree for
  servers with a real root (today it is one attached file's
  directory, chosen by hash order) — a behavioural change to a path
  real servers exercise. Confirm the order, or rule otherwise.
- **Q#D3-4 — interval, cap, and backoff curve: constants or config
  keys.** The D1/D2 framing refused a knob for a defect; with D3 the
  cadence becomes a designed mechanism, so keys are defensible — but
  more registry surface is coherence cost. Proposed: constants until
  someone asks.

## Verification sketch

- **Idle witness:** with a server attached, watchers registered, and
  no file activity, `activity_summary` settles to `None` (the absent
  segment) between scans — the strongest form of the job-count
  claim, and unwritable under the sleep design.
- **Scan-cost witness:** one scan allocates O(1) jobs, not
  O(directories), on a tree with enough directories to discriminate.
- **Join-wakes witness (round 2):** with a group backed off at the
  cap, register a new watcher — a scan starts immediately
  (timestamps through the group seam), and a file created after the
  join is reported to the joiner from its baseline onward.
- **No-overlap witness (round 2):** with a walk deliberately held
  in flight past its interval (through the group seam or by
  withholding the completion pump), the group allocates **no second
  walk job**; deadlines resume from completion.
- **Retirement witness (round 2):** the last member unregisters
  mid-walk — the walk's job is cancelled, its completion is
  rejected (no emit, no state write), and the group's schedule entry
  is gone; a server death takes the same path.
- **Queued-baseline witness (round 2):** a watcher joining mid-walk
  gets exactly one immediate follow-up scan, and its baseline is
  that scan, not the walk that was in flight at join.
- **Epoch witness (registration between snapshots):** create a file
  after the group's snapshot, then register a second watcher, then
  let a scan complete — the old watcher receives CREATED, the new
  one receives **nothing** for that file, and does receive events
  for files created after its baseline.
- **Backoff witness:** quiet scans lengthen the gap between scans
  and one change resets it — observed through scan timestamps at the
  seam, not through sleep purposes (there are none).
- **Root witness:** a server with a configured root watches that
  root, not the attached file's directory; texlab's resolver shape
  is the fixture model.
- **Contract preservation:** all six existing `m4_24` watcher tests
  stay **byte-unchanged** and green.
- `walk_tree` Rust unit tests: symlinks recorded-not-traversed,
  cooperative cancellation observed mid-walk, signature parity with
  the Lua walk it replaces.
- Each new behaviour is mutation-tested against the defect it
  guards.

## Coherence impact (§20)

- **Journey steps:** none added; step 5 is unchanged.
- **Interaction islands:** none.
- **Config registry:** none by default; Q#D3-2/Q#D3-4 could add keys
  and are flagged as such.
- **Background-work attribution (§9):** at idle there is genuinely no
  running background work, and the indicator's absence is then a true
  statement rather than a filtered one; each scan is one job named
  for its root. The ownership *model* remains §9 Stage 2's work. As a
  side effect the watcher stops holding pool threads while waiting.

## Gates

`./scripts/gate --acceptance m4_acceptance`. No `--protocol` — no
wire change, no `PROTOCOL_VERSION` bump; `walk_tree` is an fs
binding, not a protocol message.
