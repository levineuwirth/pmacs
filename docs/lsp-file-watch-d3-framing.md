# LSP file watcher D3 — the polling cost — framing

**Status: revision 1 — DRAFT, awaiting review. No implementation may
begin from this document.**

Continues issue #233, which stays open until this lane closes it. D1
and D2 — matching correctness and the re-registration leak — merged as
**#234** (`ae84d58`); this frames the remainder the user ruled next on
2026-08-11: the watcher is now *correct* and still walks everything,
every tick, forever.

## Verified against the tree at `add0ba1`

Every claim below was read or measured this session.

- Each registered watcher is its own coroutine looping
  `sleep(FILE_WATCH_INTERVAL_MS)` → `scan_tree` (`lsp.lua:1924`,
  `:2074-2083`); the interval is 250 ms.
- `scan_tree` awaits `pmacs.fs.read_dir` once **per directory**
  (`:2038-2041`), one async job each (`async_runtime.rs`'s `read_dir
  <path>` purpose). `walk` recurses unconditionally; `matches` gates
  only whether an entry is *recorded*.
- **Jobs per tick per watcher = 1 sleep + D read_dirs**, D the
  directory count under the base. rust-analyzer registers six watchers
  (post-D2; twelve before), so 6·(1+D) jobs per tick against one
  server.
- **250 ms is a floor, not a period.** The awaits are sequential, so a
  tree whose walk takes longer than the interval makes the effective
  period scan-bound — the loop never idles. The issue measured ~270 ms
  on a two-directory tree (250 ms plus one async round trip); D round
  trips dominate on real trees.
- Measured on this checkout: **220 directories, of which `.git` is 177
  — over 80 % of the walk**. (The issue's machine carried an in-tree
  `target/`: 187 directories, 46 outside `.git`+`target`. This machine
  exports an external `CARGO_TARGET_DIR` and still pays `.git`.)
- **The string-form base is nondeterministic, found while framing:**
  `resolve_watcher`'s string arm takes the directory of the FIRST
  attachment `pairs()` happens to yield (`:2150-2173`) — table order,
  not a chosen root. With one server attached to files in two
  directories, which tree a bare-string watcher can ever see depends
  on hash order. #234 made matching correct *per base*; **which** base
  is still accidental.
- **No `notify`/inotify dependency in the tree** and **no ignore-list
  infrastructure to reuse** (`src/project.rs` knows `.git` as a
  *marker*, not as something to skip) — both re-verified, both carried
  from the D1/D2 framing.

## What §9 asks of this lane

Not "quiet the modeline." The indicator is the instrument that found
this, and the churn it shows is real; quieting it is explicitly out of
bounds. The lane's job is to make the background work **small,
attributable, and honest**: fewer jobs doing the same watching, each
with a purpose naming its root.

## Design space

**A — Coalesce per (server, base).** One scan per tick serves every
watcher sharing a base: the scan records **all** files (today it
records only matches, so a shared scan moves the matcher from scan
time to diff time), the diff runs once, and each change is routed
through every watcher's matcher and kind mask, deduped by
`(uri, type)` into the server's single
`workspace/didChangeWatchedFiles` notification. For rust-analyzer this
is 6× → 1×. Pure Lua.

**B — A Rust walk primitive.** `pmacs.fs.walk_tree(base, opts)` —
the whole recursive walk as **one job** instead of one per directory:
188 jobs per tick per watcher on this repo becomes 1. The indicator
then shows one purpose (`walk_tree <root>`) instead of a stream of
`read_dir` lines. An additive fs binding plus its async-runtime job;
**no wire change** (fs bindings are not the frontend protocol) and no
new crate. Symlink non-traversal (`scan_tree`'s loop-safety) moves
into the primitive's contract.

**C — An ignore list.** Skip named directories at walk time. On this
checkout `.git` alone is >80 % of the walk. **Stated hazard:**
excluding `target/` can suppress legitimate events — rust-analyzer's
`**/*.rs` glob covers build-script `OUT_DIR` outputs under `target/`,
so an aggressive default trades churn for staleness in exactly the
server this issue is about. The default must be conservative
(Q#D3-2).

**D — Idle backoff.** The interval doubles while consecutive scans
observe no change, capped; any change batch resets it to 250 ms.
Worst-case latency for an *external* change at idle equals the cap.
LSP imposes no latency bound, and edits made through pmacs itself
never depended on the watcher (the server sees `didChange`); the
watcher exists for git checkouts, generated files, and other editors.

**E — Kernel notification** (`notify` crate: inotify / FSEvents /
kqueue). Eliminates polling. Also: a new dependency, a new Rust
subsystem, a Lua binding, a platform matrix, and an interaction with
§9's ownership model. **Deliberately staged separately** — not because
it is wrong but because A–D are pure wins E does not obsolete (a
kernel watcher still needs the initial scan, and a poll remains the
fallback path), and a new-crate decision deserves its own framing.

## Proposed shape — Stage 1 is B + A + D, with C at a conservative default

- One `walk_tree` job per (server, base) per tick; Lua keeps the
  retained map, diffs, and routes per watcher (A + B).
- Backoff 250 ms ×2 per quiet scan → 4 s cap, reset on any change (D).
- Skip-list default: **VCS metadata only** (`.git`, `.hg`, `.svn`) —
  `target/` and `node_modules` stay walked unless Q#D3-2 rules
  otherwise, because correctness beats quiet.
- The string-form base prefers the **project root**
  (`src/project.rs::detect_project` — already Lua-reachable via
  `pmacs.project`) and falls back to the attached file's directory,
  which fixes the `pairs()`-order nondeterminism (Q#D3-3).

At rest on this repo with rust-analyzer attached: from ~1,100 jobs per
scan-bound tick to **one job every four seconds**, named for its root.

## Open rulings — each blocks implementation

- **Q#D3-1 — the acceptance bar.** With Stage 1 the modeline shows one
  brief `walk_tree` blip per backoff interval at idle (up to every
  4 s), **not silence**. Silence requires Stage 2 (E) or touching the
  indicator, which is out of bounds. Is "one attributable blip at
  idle, correct events, bounded latency" the bar this lane must meet?
- **Q#D3-2 — the skip-list default and its surface.** VCS-only
  (safest), or the broader VS Code-style exclude set
  (`node_modules` etc.), with the `target/` staleness hazard above?
  And where it lives: a module constant, or a config-registry key —
  noting `ConfigValue` is four scalars, so a **list-valued setting is
  not expressible today**; a key now means a delimited string, or the
  skip list waits for table-valued settings (the §6 prerequisite).
- **Q#D3-3 — the string-form base.** Project-root preference (fixes
  the nondeterminism; widens the watched tree when attachments span
  directories) or attached-file directory (narrower, hash-order
  dependent)? The proposed shape says project root; it is still a
  behavioural change to a path real servers exercise.
- **Q#D3-4 — interval and cap: constants or config keys.** The D1/D2
  framing refused a knob for a defect. With D3 the poll becomes a
  designed mechanism, so keys are defensible — but two more registry
  keys is coherence surface. Proposed: constants until someone asks.

## Verification sketch

- **Job-count witness:** a coalesced server's per-tick allocation is
  O(1), not O(directories) — observed through the activity summary or
  a counter seam, asserted on a tree with enough directories to
  discriminate.
- **Backoff witness:** quiet scans lengthen the interval and one
  change resets it — observable through the sleep purpose or a seam.
- **Skip witness:** a change under `.git/` never emits; the same
  change outside it still does.
- **Contract preservation:** all six existing `m4_24` watcher tests
  stay **byte-unchanged** and green — they are D1/D2's contract, and
  this lane must not weaken it.
- `walk_tree` gets Rust unit tests of its own: skip list honored,
  symlinks recorded-not-traversed, signature parity with the Lua walk
  it replaces.
- Each new behaviour is mutation-tested against the defect it guards.

## Coherence impact (§20)

- **Journey steps:** none added; step 5 is unchanged.
- **Interaction islands:** none.
- **Config registry:** none by default; Q#D3-2/Q#D3-4 could add keys
  and are flagged as such.
- **Background-work attribution (§9):** unattributed churn drops from
  O(directories × watchers) jobs per tick to one attributable job per
  server at rest. The ownership *model* remains §9 Stage 2's work,
  not this lane's.

## Gates

`./scripts/gate --acceptance m4_acceptance`. No `--protocol` — no wire
change, no `PROTOCOL_VERSION` bump; `walk_tree` is an fs binding, not
a protocol message.
