# LSP file watcher D3 — the polling cost — framing

**Status: revision 2 — DRAFT, awaiting review. No implementation may
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
  scope. **Revision 2 removes the sleep from the design entirely**
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
  asked for a root). **Revision 2 roots the scan at what the
  registering server actually serves**: `root_uri` → `cwd` →
  attached-file directory, in that order.
- **P1 — coalescing needs registration-epoch semantics.** Revision 1
  said "route the shared diff through every watcher" without defining
  which watcher set owns a diff. A watcher registered between two
  snapshots would receive a false CREATED for a file that predates
  its registration; membership changing during a walk recreates
  either #234's stale-watcher batch or the same pre-registration
  event. **Revision 2 defines shared snapshots with per-watcher
  baselines** (below), plus two witnesses the existing six tests do
  not cover.
- **P1 — "VCS-only exclusion is safest" was wrong.** A hard skip
  silently ignores a server that legitimately registers `.git/HEAD`
  or `**/.git/**`, and glob semantics mean even `**/*.rs` *can* match
  under `.git/` — so any unconditional exclusion is a deviation from
  the registered contract, not a safe default. Revision 1 also
  overweighted the win: exclusion was a **job-count** lever when every
  directory was a separate job, and the walk primitive removes that
  economics. **Revision 2 defaults to no unconditional exclusion**
  and reframes Q#D3-2 around the full option set.
- **P2 — the arithmetic used the issue's machine, not this one.**
  With D = 220 on this checkout it is 220 `read_dir` jobs **plus one
  sleep** per watcher per tick — 221, or **1,326 across
  rust-analyzer's six watchers** — and revision 1's proposed steady
  state was itself two jobs (sleep + walk), not one. Corrected
  throughout; the revised design's steady state is **zero jobs at
  idle** and one `walk_tree` job while a scan runs.

## Verified against the tree at `add0ba1`

Every claim below was read or measured this session (revision 2
re-verified the round-1 corrections against the code).

- Each registered watcher is its own coroutine looping
  `sleep(FILE_WATCH_INTERVAL_MS)` → `scan_tree` (`lsp.lua:1924`,
  `:2074-2083`); the interval is 250 ms.
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
- **No `notify`/inotify dependency in the tree** and **no
  ignore-list infrastructure to reuse** — both re-verified, both
  carried from the D1/D2 framing.

## What §9 asks of this lane

Not "quiet the modeline." The indicator is the instrument that found
this, and the churn it shows is real; quieting it is explicitly out
of bounds. The lane's job is to make the background work **small,
attributable, and honest**: at idle there should *be* no running
background work to report, and while a scan runs it should be one job
named for its root.

## The cadence — after-tick deadlines, not sleeps (review P1)

The per-watcher sleep loop is replaced by the Q#AS2 idiom: one
`process.after-tick` subscription owns every scan group's schedule.
Per frame it reads `pmacs.editor.monotonic_ms` once and compares each
group's `next_scan_at`; a due group gets its scan started (a
coroutine that runs the walk and diff). Waiting therefore allocates
**no job and no pool thread** and renders **no indicator segment** —
`activity_summary` returns `None` at zero by contract. While a scan
runs, the indicator honestly shows its one job.

This also retires a defect revision 1 did not name: today's sleeps
hold pool threads, so N watchers subtract N threads from a pool of
`available_parallelism - 1`. The after-tick cadence gives them all
back.

## The scan root (review P1)

For a string-form (bare `*.txt` / absolute) registration the base
becomes, in order: the server's **`root_uri`** (spec verbatim — nil
when the server never asked for a root), the server's **`cwd`**, and
only then the attached-file directory. These describe the workspace
the registering server actually serves — including configured roots
and custom resolvers like texlab's, which `pmacs.project.detect` can
never reproduce (Q#LX2). This replaces the `pairs()`-order accident
with a deterministic, server-owned answer. `RelativePattern`s keep
their own `baseUri`, unchanged.

## Coalescing, with registration epochs (review P1)

One scan group per (server, base). The group's scanner records
**all** files (the matcher moves from scan time to diff time); each
completed scan increments the group's **snapshot epoch**.

Delivery semantics, stated precisely because revision 1 did not:

- Each watcher records the epoch current when it **joined** the
  group. Its **baseline is the first snapshot completed after it
  joined**; it receives diffs only between snapshots it has a
  baseline for. A file created after the group's previous snapshot
  but before a new watcher registered therefore produces **no event
  for that watcher** — it is folded into the watcher's baseline,
  exactly as the initial scan folds pre-existing files today.
- **Membership is captured at scan start**; a watcher joining
  mid-walk waits for the next snapshot.
- **Cancellation is rechecked per watcher at emit time** — #234's P2
  rule, now applied per member: a watcher superseded or unregistered
  during the walk emits nothing, and its replacement (a fresh join)
  has no baseline yet, so it emits nothing either. Both halves of the
  round-1 hazard close on the same two rules.
- Changes passing a watcher's matcher and kind mask are deduped by
  `(uri, type)` into the server's single
  `workspace/didChangeWatchedFiles` notification, as today.

## The walk primitive

`pmacs.fs.walk_tree(base)` — the whole recursive walk as **one job**
instead of one per directory: 220 `read_dir` jobs per scan on this
repo become 1. The indicator shows one purpose (`walk_tree <root>`).
An additive fs binding plus its async-runtime job; **no wire change**
(fs bindings are not the frontend protocol) and no new crate. Symlink
non-traversal (`scan_tree`'s loop-safety) moves into the primitive's
contract, witnessed by its own Rust tests.

## Exclusions (review P1) — none by default

Glob semantics make any unconditional skip a contract deviation:
`**/*.rs` compiles with a separator-spanning prefix, so it *can*
match under `.git/`, and a server may register `.git/HEAD` outright
(branch-watching tools do). The only semantics-preserving default is
**no unconditional exclusion**, and with the walk primitive the
economics support it: exclusion was worth 80 % of the *job count*
when every directory was a job; inside one `walk_tree` job it is only
readdir syscalls, and the whole 220-directory walk is a few
milliseconds of one pool thread every backoff interval.

The option space, for Q#D3-2: (a) no unconditional exclusion — the
proposed default; (b) **opt-in** exclusion through configuration, for
users with pathological trees, framed explicitly as a watcher-contract
trade; (c) matcher-aware pruning — skip a subtree only when *no*
active watcher's pattern can match under it — which is sound but
almost never fires against real registrations, because
`**/`-leading globs can match anywhere; (d) a hard built-in VCS skip,
which revision 1 called "safest" and is not: it is (b) without the
opt-in.

## Idle backoff

The interval doubles while consecutive scans observe no change,
capped at 4 s; any change batch resets it to 250 ms. Under the
after-tick cadence a longer interval costs *nothing* while waiting —
backoff now bounds **scan frequency**, not sleep-job length.
Worst-case latency for an external change at idle equals the cap;
LSP imposes no latency bound, and edits made through pmacs never
depended on the watcher (the server sees `didChange`). The watcher
exists for git checkouts, generated files, and other editors.

## Deliberately staged separately — kernel notification

`notify` (inotify / FSEvents / kqueue) eliminates polling. Also: a
new dependency, a new Rust subsystem, a Lua binding, a platform
matrix, and an interaction with §9's ownership model. Staged as its
own framing — not because it is wrong but because everything above is
a pure win it does not obsolete (a kernel watcher still needs the
initial scan and a polling fallback), and a new-crate decision
deserves its own review.

## Proposed shape — Stage 1

After-tick cadence + walk primitive + coalescing-with-epochs +
backoff; no exclusions by default; server-owned scan root.

At rest on this repo with rust-analyzer attached: **from 1,326 jobs
per scan-bound tick (six of them pool-thread-holding sleeps) to zero
jobs at idle**, with one `walk_tree` job for the few milliseconds a
scan actually runs, at most every 250 ms under activity and every 4 s
at rest.

## Open rulings — each blocks implementation

- **Q#D3-1 — the acceptance bar.** With Stage 1 the indicator is
  **absent at idle** (no running job exists — `activity_summary`'s
  `None`-at-zero contract) and shows `⋯1 walk_tree <root>` for the
  duration of each scan. Is that the bar — an honest blip per scan,
  absence otherwise — with true event-driven silence deferred to the
  kernel-notification framing?
- **Q#D3-2 — exclusions.** Proposed: none by default, with opt-in
  exclusion as a documented contract trade (option b) if a user asks.
  Confirm, or rule for one of (b)/(c)/(d) above.
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
  segment) between scans — the strongest form of the job-count claim,
  and unwritable under the sleep design.
- **Scan-cost witness:** one scan allocates O(1) jobs, not
  O(directories), on a tree with enough directories to discriminate.
- **Epoch witness (registration between snapshots):** create a file
  after the group's snapshot, then register a second watcher, then
  let a scan complete — the old watcher receives CREATED, the new one
  receives **nothing** for that file, and does receive events for
  files created after its baseline.
- **Epoch witness (replacement during a scan):** through the group's
  scan seam (the `_after_scan_for_tests` device, lifted to the
  group), re-register mid-scan — the superseded watcher emits
  nothing (P2's rule, per member) and the replacement emits nothing
  until its own baseline exists.
- **Backoff witness:** quiet scans lengthen the gap between scans
  and one change resets it — observed through scan timestamps at the
  seam, not through sleep purposes (there are none).
- **Root witness:** a server with a configured root watches that
  root, not the attached file's directory; texlab's resolver shape is
  the fixture model.
- **Contract preservation:** all six existing `m4_24` watcher tests
  stay **byte-unchanged** and green.
- `walk_tree` Rust unit tests: symlinks recorded-not-traversed,
  signature parity with the Lua walk it replaces.
- Each new behaviour is mutation-tested against the defect it guards.

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
