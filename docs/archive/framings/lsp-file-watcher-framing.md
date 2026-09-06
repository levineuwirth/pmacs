# LSP file watcher — framing

**Status: revision 2 — approved design (2026-08-10), plus two
correctness findings from review OF THE IMPLEMENTATION.** The user ruled
that D1 and D2 proceed with the walking explicitly surviving this lane;
D3 gets its own framing. The acceptance bar for this lane is correctness
and the leak, not "the flipping stops".

**Revision 2 records a review round against the code, not the design.
Both findings are cases where the first fix was itself wrong**, and both
were confirmed against the tree before being acted on:

- **P1 — the form must be read from the PATTERN, not from the union
  arm.** `resolve_watcher` returned `"absolute"` for *every* string, so
  a bare `*.txt` — a valid relative pattern under LSP 3.17, and how VS
  Code treats string watchers across workspace folders — was matched
  against `<base>/foo.txt` and could never fire. **That case worked
  before this lane touched it**, so the repair for #233 silently broke a
  live path while fixing another. A leading `/` is what makes a pattern
  absolute; classification now reads the string.
- **P2 — a scan completing after cancellation still emitted.**
  `scan_tree` awaits `read_dir` once per directory, so the coroutine
  sits suspended for most of a tick with `_sleep` already cleared. A
  cancel arriving there sets `cancelled` and has no sleep to interrupt,
  and the resumed scan ran on to `did_change_watched_files` — one stale
  batch under the superseded pattern, which is a wrong-pattern
  notification the server acts on. Cancellation and liveness are now
  rechecked after the scan.

**F1's lesson repeated itself inside this lane.** The flat-pattern test
constrains the RelativePattern **object** arm, so it said nothing about
the **string** arm P1's regression lived in — the same
tested-path/exercised-path split this framing opened by naming. Both
findings now have tests, and both tests were mutation-checked: each
fails only its own defect.

**A test seam was added, and is recorded here rather than buried.**
`pmacs.lsp._after_scan_for_tests` is a production hook, nil in normal
operation, that P2's witness requires: the race is a cancel landing
during one of the scan's suspensions, which no arrangement of real
timing produces on demand. Same device and justification as `git.lua`'s
`_deliver_status`. It is handed the scan result deliberately — a test
that cancels on any *other* scan passes with the fix deleted, because
the loop would break at the post-sleep check and emit nothing anyway.

Answers issue #233. **Scope is D1 and D2 only** — the two bug-shaped
defects. D3 (the polling cost) is named here, deferred with reasons, and
gets its own framing.

## What is and is not a regression

**#232 is not at fault and nothing about it should be reverted.** The
statusline activity indicator it added is *correct*: it renders real
in-flight jobs from `AsyncRuntime::activity_summary`, and the jobs it
names (`sleep 250ms`, `read_dir <path>`) are real. What changed on
2026-08-09 is **visibility**, not behaviour.

The behaviour has been there since `1c25730` (2026-05-19). So the user-
facing report — "the modeline flips several times a second" — is a
three-month-old defect that became observable last week, and the fix
belongs to the watcher, not the indicator.

Recorded plainly because the tempting move is to quiet the indicator,
and that would delete the only instrument that found this.

## Verified against the tree at `0e4c58d`

Every claim below was read or executed this session, not carried from
the issue.

- `FILE_WATCH_INTERVAL_MS = 250` (`lsp.lua:1924`); each watcher is one
  `pmacs.async` coroutine looping sleep → `scan_tree`
  (`lsp.lua:2060-2097`).
- `scan_tree` builds `rel` from an empty prefix and calls
  `matches(rel)` — **relative** paths (`lsp.lua:2035-2056`).
- **`walk` recurses into every directory unconditionally.** `matches`
  gates only whether an entry is *recorded*. A watcher that can never
  match still walks the whole tree every tick.
- `resolve_watcher`'s string branch returns the pattern **unchanged**
  with the base guessed from an attached file's directory
  (`lsp.lua:2102-2116`).
- `register_file_watchers` ends `file_watchers[skey][reg.id] = recs`
  with no cancellation of the outgoing list (`lsp.lua:2132`).
- Job purposes are `format!("sleep {}ms", …)` (`async_runtime.rs:1027`)
  and `format!("read_dir {}", …)` (`:1178`).
- The fake LSP registers **one** watcher, a `RelativePattern`
  `{ baseUri, pattern: "**/*.txt" }`, id `watch-1`
  (`pmacs_fake_lsp.rs:312-331`).

### The glob table, reproduced

Ran the tree's own `expand_braces` / `glob_one_to_pattern` /
`glob_matcher` under LuaJIT. Output matches the issue exactly, compiled
patterns included:

| glob | compiled | `main.go` | `go.mod` | absolute |
|---|---|---|---|---|
| `**/*.{mod,work}` | `^.-[^/]*%.mod$` | false | **true** | true |
| `<abs>/goproj/**/*.{go,…}` | `^/tmp/goproj/.-[^/]*%.go$` | false | false | true |
| `<abs>/rsproj/**/*.rs` | `^/tmp/rsproj/.-[^/]*%.rs$` | false | false | true |

## Two findings the issue does not carry, both of which shape the fix

### F1 — the existing test cannot discriminate this fix, in either direction

`**/*.txt` compiles to `^.-[^/]*%.txt$`, and `.-` spans `/`. Measured:
it matches `a.txt`, `sub/a.txt`, `/base/a.txt` **and**
`/base/sub/a.txt`. So `m4_24_workspace_did_change_watched_files` passes
whether the matching subject is relative or absolute.

The issue says the tested path and the exercised path are disjoint. The
sharper statement is that the existing test is **insensitive**: it
cannot fail for D1 and it cannot confirm D1's fix. New coverage must use
a pattern whose two readings disagree, or it will inherit the same
blindness.

### F2 — the fix cannot simply "match absolute"; the form must be carried

Per LSP, a plain-string glob matches the **absolute** path while a
`RelativePattern`'s pattern is relative to **its base**. Matching
everything absolutely breaks the second. Measured on `*.txt`:

| subject | matches |
|---|---|
| `a.txt` (relative, correct for RelativePattern) | **true** |
| `/base/a.txt` (absolute) | **false** |

`resolve_watcher` returns `(base, pattern)` and **discards which form it
came from**, so both callers below it are already unable to tell. The
fix therefore changes that function's contract — a third return value or
an explicit record field — rather than only changing the subject string
at the match site. A fix that ignores this trades rust-analyzer's six
broken globs for every `RelativePattern` whose pattern does not begin
`**/`.

## D1 — plain-string globs never match

**Consequences, as measured in the issue and confirmed by the table
above:** rust-analyzer is never told about any file change (all six
globs absolute); gopls is told about `go.mod`/`go.work` but never `.go`
sources (only its relative glob matches).

**Fix:** match a plain-string glob against `base .. "/" .. rel`; keep a
`RelativePattern` matched against `rel`. `resolve_watcher` gains the
form in its return, and the record carries it.

The leading `**/` in gopls' relative glob compiles to `.-`, which spans
`/`, so that glob keeps matching under the absolute subject — which is
why one server's working case does not regress.

## D2 — re-registration leaks the previous coroutines

`file_watchers[skey][reg.id] = recs` replaces the record list without
setting `cancelled` or cancelling the in-flight `_sleep`. The old
coroutines poll until the server dies and are unreachable by
`unregister_file_watchers`, which can only see what the table now holds.

**Reachable today**: rust-analyzer registers
`workspace/didChangeWatchedFiles` **twice under the same id**, six
watchers each, with no intervening unregister — 12 concurrent
coroutines, six permanently uncancellable. The issue's 44.1/s dir-open
rate against a ~270 ms period implies 12 watchers, so the leak is
measured from outside the process, not only read from the source.

**Fix:** cancel the outgoing list before replacing it, with the same
treatment `unregister_file_watchers` already applies.

## What this lane does NOT fix, stated so the report is not mistaken for closed

**The poll cost survives both fixes.** D1 makes matching correct and D2
halves rust-analyzer's watcher count; neither stops the walk. After this
lane, rust-analyzer still walks the entire tree every 250 ms — six times
per tick instead of twelve — including `.git`, `target` and
`node_modules`, at one async job per directory.

So the modeline will still show activity, at roughly half the rate. **If
the acceptance bar for this lane is "the flipping stops", this lane does
not meet it** and should not be started until D3 is framed. That is a
ruling for the user, not an assumption to make quietly.

**Answered 2026-08-10: the user accepted this scope.** D1 and D2
proceed; the walking is D3's problem, framed separately.

## D3 — deferred, with what was checked

Options named in the issue: coalesce a server's watchers into one scan;
root the scan at the workspace rather than an attached file's directory;
an ignore list; back off when nothing changes; or a real
filesystem-notification primitive.

Checked while framing: **there is no `notify`/inotify dependency in the
tree**, so the last option is a new crate *and* a new Rust primitive
plus its Lua binding — not a small change. There is also **no existing
ignore-list infrastructure** to reuse; `src/project.rs` knows `.git` as
a *marker* name, not as something to skip.

D3 is a `COHERENCE.md` §9 concern — background work with no ownership
model — and §9's own Stage 1 is the indicator that surfaced it.

## Verification

The suite must fail without each fix, which the existing suite cannot
(F1). Planned:

- **A fake-LSP mode registering a plain-string ABSOLUTE glob**, with a
  pattern whose relative and absolute readings **disagree** — so the
  test fails today and passes after D1.
- **A fake-LSP mode registering a `RelativePattern` whose pattern does
  not begin `**/`** (e.g. `*.txt` at the base). This is F2's guard: it
  passes today, and fails against a fix that matches everything
  absolutely. Without it, the obvious wrong fix is green.
- **A re-registration mode**: the same id twice, no unregister. The
  witness is that the superseded watchers **stop**, asserted on
  observable polling rather than on internal table shape, since the
  defect is precisely that the old records are unreachable.
- Existing `m4_24` kept and expected **unchanged** — it covers the
  working branch and its insensitivity is now recorded rather than
  mistaken for coverage.

Each new test is mutation-tested against the fix it names.

## Coherence impact (§20)

- **Journey steps**: none added; step 5's editing surface is affected
  only in that a correct watcher makes servers see edits they currently
  miss.
- **Interaction islands**: none.
- **Config registry**: no new setting. The interval stays a module
  constant; making it configurable would offer the user a knob for a
  defect rather than a preference, and D3 may remove the poll entirely.
- **Background-work attribution (§9)**: this lane *reduces* unattributed
  background work but does not model it. D3 owns that, and the honest
  statement is that the indicator worked — it made three months of
  invisible churn visible on its first week.

## Gates

`./scripts/gate --acceptance m4_acceptance` plus the touched LSP
acceptance suites; no `--protocol` (no wire change, no
`PROTOCOL_VERSION` bump).
