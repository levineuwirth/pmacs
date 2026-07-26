# Active work — cross-machine resume ledger

**Snapshot: 2026-07-26.** This file records volatile work that has not
landed on `main`. Read it after `docs/agent-handoff.md`. Remove completed
entries when their PR merges; do not let this become a second permanent
backlog.

## Repository authority

- Canonical development URL:
  `https://github.com/levineuwirth/pmacs.git`. This ledger uses the
  normalized local alias `githubsucks` so its refs and recovery commands
  are identical on every machine. Remote names are otherwise
  machine-local: `origin` may name this canonical URL, a release mirror,
  or something else, and therefore has no authority by name alone.
- Canonical base at this snapshot:
  `githubsucks/main` @ `d400f30` (Lean 4 Stage 3b #170 atop Stage 3a
  #167, the bottom-panel landed-doc refresh #156, the inline-math slice
  #158, dired Stage 1 #165, the GPU terminal input fix #166, Lean 4
  Stage 2 #161, the dired framing #164, COHERENCE.md #163, find-file
  #162, Lean 4 Stage 1 #160, and the minimap blank-slab fix #159;
  protocol v20). The previous snapshot named `d152120`; the recovery
  check below accepts it or anything newer.
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

The `git log` command must expose `d152120` or a newer intentional main.
If it does not, stop and repair the remote/fetch configuration.

## Lean 4 lane (Arc 8) — Stages 1, 2, 3a, 3b MERGED; Stage 4a IN REVIEW

- **Stages 1, 2, 3a and 3b are MERGED** — #160 (`main` @ `0827dd1`),
  #161 (`46a1b8f`), #167 (`6f348c9`), #170 (`d400f30`). Their full
  histories were pruned from this ledger in round 6, per this file's own
  instruction to remove entries when their PR merges; the durable facts
  now live in `docs/agent-handoff.md` §1's Lean 4 bullet, which is where
  a fresh machine should read them. `docs/lean4-mode-framing.md` rev 8
  carries the decisions.

### Stage 4 — framing rev 8, split into 4a/4b (branch `lean4-stage4a-typed-edit-chain`)

- Stages 3a and 3b **merged as #167** (`main` @ `6f348c9`) and **#170**
  (`main` @ `d400f30`), 2026-07-26. Both were integrated against a main
  that had advanced 50 commits mid-review; the only conflict either time
  was this ledger's own lane headings, resolved by keeping both sides.
- Worktree `../pmacs-lean-stage4`, branched off `main` @ `d400f30`.
  Framing-only so far: `docs/lean4-mode-framing.md` **revision 8**. No
  code. Awaiting user approval before implementation, per the workflow.
- **Round 6 review found five P1s, four of them internal to rev 6** —
  facts about pmacs the revision asserted without checking, while its
  external (upstream) facts held. Fixed in rev 7: Stage 4a's footprint
  omitted the test file its own acceptance requires; pending
  abbreviation state was keyed by buffer when pmacs is **multi-frontend**
  (`EditorCore.views` is per-`FrontendId`, `take_typed_edit` is already
  frontend-keyed, and `buffer.after-switch` fires with NO arguments, so
  a buffer-keyed clear lets any frontend discard another's pending
  state); the shortest-match rule was missing its **tie-break by source
  declaration order**, which 101 prefixes depend on and a `pairs`-
  iterated Lua map cannot express; and the generator's "abort on keys
  needing escaping" rule **rejects the real table** (`\` is a key, `"`
  begins eleven).
- **A 404 on a guessed path is not evidence of absence.** Rev 6 declared
  the upstream package ships no README after fetching the package root,
  with the directory listing showing `src/README.md` already in hand.
  The README states the tie rule in one sentence.
- **Round 7 review found one remaining P1 in acceptance 45i.** Rev 7
  required A's pending abbreviation to survive B editing the same
  buffer, while Q#LN22 also required an exact buffer-revision advance.
  Those cannot both hold: revisions are buffer-global and every edit
  bumps them. Rev 8 keeps the conservative guard and separates
  ownership from survival — B cannot consume A's record, but B editing
  the shared buffer invalidates A lazily; B switching buffers or
  detaching remains frontend-scoped when no shared-buffer edit
  intervenes.
- **Round 5 re-scout split Stage 4 into 4a (substrate) and 4b (Lean).**
  4a is the typed-edit consumer chain — `builtin/runtime/typed_edit.lua`
  plus `pair.lua` re-expressed as one registered consumer, no behavior
  change. 4b is the input method. The split is forced by §4's own rule,
  which Stage 4's risk column ("refactors `pair.lua`'s provenance read")
  broke while the prose called the stage Lean-only.
- **This is the SECOND consecutive re-scout to find that rule broken**
  (round 4 found it for Stage 3). Rev 5 had even noticed the shape and
  answered it with a commit boundary. **A commit boundary is not a review
  boundary.** Re-check every remaining stage against §4 at scout time;
  the rule is not self-enforcing.
- **Rev 5's expansion semantics were wrong in three ways**, found by
  reading `leanprover/vscode-lean4` @ `17d1d08` rather than inferring
  from behavior. Resolution is *shortest key having the input as a
  prefix* (`\al` → `∀` from `all`, not `alpha`); there is **no
  terminator list** (`'+ '` is a key, so space extends after `\+`; `'\'`
  is a key, so `\\` → `\`); and an unmatchable tail is **appended**,
  not dropped (`\alp7` → `α7`).
- **There is no cursor-motion hook**, so rev 5's acceptance 43 ("moving
  the cursor out abandons it") was not buildable. Abandonment is lazy —
  validated at the next typed edit — and the criterion now asserts what
  pmacs can actually detect. Upstream drives this off `changeSelections`;
  that seam does not exist here.
- **`dispatch_key` is only half the production path for 4b.** The
  auto-pair suite gets away with dispatch-only because Q#AP1 removed the
  pair chars from the optimistic classifiers; `\` and the letters are
  NOT excluded, so on a CRDT frontend the optimistic producer is the real
  path. That producer is `#[cfg(feature = "crdt")]` and CI never enables
  `crdt`, and the gate list runs `--features crdt` only for `--lib` — a
  crdt-gated integration test is **dark twice over**.
- The whole expansion has cross-peer-degraded undo (Q#LN21): six
  source-peer optimistic inserts replaced by one daemon-peer op.
  `set_round_trip_input` would fix it and is rejected — it also disables
  `dispatch_idle`, so RET stops inserting a newline.
- Table facts re-derived at `17d1d08`: 1,855 entries, 36,861 bytes, all
  keys ASCII, **64** keys carry a `lean4` pair-set char, **305** keys are
  proper prefixes of another (so 1,550 expand eagerly), **26** values
  carry `$CURSOR`, and **119** are multi-codepoint — the 26
  `$CURSOR`-bearing values plus 93 others.
- Citation sweep per COHERENCE §25: five live citations moved in the 50
  commits since rev 5 — `take_typed_edit` 12827→12990,
  `handle_server_requests` 1549→1815, `fs.stat` 93→133,
  `detect_buffer_language` 452→457, `send_request`/`send_notification`
  9342/9361→9507/9527.
### Stage 4a — the typed-edit consumer chain (IMPLEMENTED, same branch)

- Footprint exactly as Q#LN10 declares it: `builtin/runtime/typed_edit.lua`
  (new), `pair.lua` re-expressed as one consumer,
  `src/editor.rs` +15 (the `include_str!` and its ordering comment), and
  `tests/typed_edit_chain_acceptance.rs` (new, 13 tests).
  **`tests/auto_pair_acceptance.rs` is UNCHANGED — `git diff --stat
  main...HEAD -- tests/auto_pair_acceptance.rs` is empty.** That is
  criterion 46 checked at the diff, which is the only way it means
  anything.
- **The chain calls consumers even when the record is nil.** This is a
  decision, not an implementation detail: three existing auto-pairing
  tests assert `pmacs.pair._last_record == nil` after a record-less
  fan-out (paste, programmatic insert, nested manual `hook.run`), so
  skipping consumers on nil fails them. Stage 4b needs the same
  delivery to abandon a pending abbreviation an unrelated edit
  invalidated.
- **Ordered insertion, not `table.sort`** — Lua's sort is not stable, and
  "ties broken by registration order" is a stated contract.
- **The chain `pcall`s each consumer** and reports through
  `set_status`. Rev 7 justified this by claiming an uncontained throw
  would fail the fan-out for every other subscriber including lsp.lua's
  didChange flush; **that is wrong** — `run_all_must_succeed`
  (`src/hook.rs:332`) collects errors and continues, so the other
  subscribers still run. The real consequence is narrower and still
  worth containing: the throw skips every LATER consumer in the chain.
  The rendering is protected too, because a Lua error may be a table
  whose `__tostring` throws.
- **Round 8 (review) findings, all fixed on this branch:** each consumer
  now gets its **own shallow copy** of the record (the same table let a
  declining consumer rewrite `rec.char`, which pairing reads — typing
  `x` could produce `x)`); the fan-out iterates a **snapshot** (a
  consumer registering a lower-priority one shifted itself forward under
  `ipairs` and ran twice, unbounded if repeated); `tostring` moved
  inside the containment; **non-finite and non-integer priorities are
  rejected** (NaN is a number and every ordered comparison with it is
  false, so it landed wherever the insertion scan gave up and silently
  voided the ordering contract); and `add_consumer` now returns a handle
  with `remove_consumer` beside it, so re-evaluating a config no longer
  leaks callbacks the way `pmacs.hook.add` does (COHERENCE §13).
- **Every acceptance test is bite-verified by mutation**, per the
  standing rule that a test is not evidence until the mutation it
  targets has been shown to fail it:

  | Mutation | Tests it fails |
  |---|---|
  | append instead of ordered insert | 5 chain |
  | `>=` instead of `>` in the insert scan | 1 chain (tiebreak) |
  | re-take the record per consumer | 4 chain |
  | ignore the claim return value | 1 chain |
  | drop the `pcall` | 1 chain |
  | skip consumers when `rec == nil` | 1 chain + **3 auto-pair** |
  | load `typed_edit.lua` after `lsp.lua` | 1 chain + **2 auto-pair** (Q#AP7) |
  | hand every consumer the same record table | 1 chain (46f) |
  | iterate the live array instead of a snapshot | 1 chain (46g) |
  | render the error outside the `pcall` | 1 chain (46d) |
  | accept any Lua number as a priority | 1 chain (46h) |
  | make `remove_consumer` a no-op | 2 chain (46g, 46h) |

  The first attempt at the last bite was WORTHLESS as written: moving
  only `typed_edit.lua` past `lsp.lua` left `pair.lua` calling a nil
  `add_consumer`, so the runtime failed to load and all 9 tests died —
  loud, but not a test of the flush-ordering property. Moving
  `typed_edit.lua` AND `pair.lua` past `lsp.lua` is the faithful
  falsification: registration succeeds, the hook lands late, and exactly
  the three ordering tests fail. **A bite that kills everything has not
  isolated anything.**
- Verification on this branch (commit-then-gate, so this describes the
  pushed tree): `cargo fmt --check` clean; strict workspace Clippy
  clean; 1,832 default + 2,009 CRDT library tests; auto-pair 45/45;
  typed-edit chain 13/13 (and 13/13 again under `--no-default-features
  --features lua54`, since the fixes touch `math.huge`, `%`, and
  `__tostring` behavior that differs between the backends); M4 121;
  required GPU 202; **isolated-config workspace sweep 3,332 across 97
  suites, zero failures** with `grep -c basedpyright` = 0; `git diff
  --check` clean.
- Stage 4b (the input method) is NOT in this PR and not started.

## Dired lane — Stage 0 MERGED; Stage 1 IN REVIEW (PR #165)

- Approved framing: `docs/dired-framing.md` **revision 6** — rev 5 is the
  approved text (merged as its own docs PR #164), rev 6 adds §0's Stage 1
  implementation notes (S1-1…S1-9). Stages 2 (marks and operations) and 3
  (wdired) each get their own detailed framing after the prior stage lands.
- **Stage 0 (`C-x C-f` find-file) MERGED as #162** (`main` @ `2af1ab3`,
  2026-07-25, one review round, 12/12 CI green). Durable facts moved to
  `docs/agent-handoff.md` §1 per rule 3 below.
- **Stage 1 branch: `githubsucks/dired-stage1`**, worktree
  `../pmacs-dired-stage1`, based on `githubsucks/main` @ `8c86d34` (the
  framing merge #164). **A fresh cut, not a rebase:** the older `dired`
  branch (`ffdd642`, worktree `../pmacs-dired-arc`) was based on the
  superseded `0827dd1` and carried only the framing content #164 already
  put on `main`, so merging it would have reconciled two histories of one
  document. It is left untouched and carries nothing unmerged.
- **Stage 1 implemented; no wire change (protocol stays v20).** What
  landed on the branch:
  - `builtin/runtime/dired.lua`: one buffer per directory named
    `*dired:<canonical path>*` with the handle-table ownership check;
    read-only intercept + `set_round_trip_input`; the `dired` major mode
    and its mode-scoped keymap (`RET`/`f`, `^`, `n`/`p`, `g`, `q`, `s`);
    basename cursor re-seating across every wholesale repaint;
    `display_file` for file visits and same-window reuse for directory
    descent; `C-x d` / `C-x C-j`; the `dired.kill-when-opening` setting.
    Loaded after `window.lua`.
  - `src/fs.rs`: `ReadDirTolerance`, `FsDirEntryError`, `FsDirListing`,
    and one walk that either fails on a per-entry condition or records it
    (Q#DR6). `src/async_runtime.rs` carries the listing in
    `ReplyKind::ReadDir` / `JobResult::ReadDir`; `src/lua_bindings/mod.rs`
    keys the Lua result **shape** on `errors.is_some()`, so the bare array
    the frozen M8.2 fixture consumes with `ipairs` is untouched;
    `builtin/runtime/fs.lua` validates read-op opts and **rejects unknown
    keys** (a typo'd `tolerant` used to degrade silently to fatal).
  - `src/editor_core.rs` + `src/lua_bindings/mod.rs`:
    `normalize_buffer_path` is `pub` and exposed as
    `pmacs.path.canonicalize` — Q#DR2's preferred end state, so no Lua
    mirror exists and Stage 2 owes no mirror removal. This makes B2
    ("tolerant `read_dir` is the only Rust change") false by one small
    binding, deliberately.
  - `tests/dired_acceptance.rs`: 22 tests over framing items 1–16,
    dispatch-driven; item 17 is the m8_1/m8_2/m8_3 additivity gate.
- **The framing claim the substrate falsified (S1-2):** R2-3 expected a
  dedicated dired panel to carry its dedication across a descent.
  `display_buffer` never replaces the buffer in a slot dedicated to
  another one — it discards every side-specific parameter and falls back
  to the document window (Q#BP3 2.iii), and the exact-window arm errors.
  Dired does not unpin the user's panel; both arms are pinned.
- **The vacuity the bites found (S1-3):** acceptance 3c cannot pin the
  descent *routing*. Dired holds focus in its own panel, so a raw
  `switch_buffer` lands in the same window and every 3c assertion holds
  either way. Dedication is the only discriminator, so the
  dedicated-panel test is the real pin — and the vacuity is documented at
  the assertion rather than relabelled.
- **The pre-existing test dired's first mode-scoped binding broke
  (S1-4):** `describe_key_identifies_every_default_binding` asserted every
  binding in the stack resolves through `describe.key` context-free, which
  held only while the modes table was empty. It now sets the effective
  context per binding and explicitly *clears* the mode for global ones,
  because a leaked mode legitimately shadows a global chord of the same
  name (dired's `RET` shadows `edit.newline-and-indent`).
- Durable substrate facts, independent of this arc:
  - `pmacs.buffer.kill` (not `remove`) redirects windows off a doomed
    buffer before removal, so `kill-when-opening` kills **after** the
    replacement is displayed.
  - Interactive origin does **not** survive an await: work resumed in
    `tick_async` sees no `InteractiveCommandOrigin`, so `pmacs.window.*`
    acts for the *ambient* active frontend (S1-9).
  - Kinds are lstat-based in both `read_dir` and `stat`, so nothing in an
    entry says whether a symlink points at a directory; `RET` probes by
    trying to list it (S1-8).
  - A path-backed buffer's *name* is its full path, not its basename —
    worth knowing before writing any name assertion.
  - `C-x d` takes **no** completion source on purpose (S1-5): with one,
    RET on an empty field opens whatever sorts first, and
    RET-on-where-you-are is the gesture the binding exists for. The field
    is prefilled instead.
- **Bite verification:** 15 claims, each mutated in place and required to
  fail the test that names it. `dired.lua` is new, so `scripts/bite`'s
  file swap does not apply; every mutation was applied and reverted with
  `git checkout --`. One came back VACUOUS and is recorded above.
- **Review round 1 addressed** (framing rev 7, S1-10…S1-12). Three
  behavioral fixes, each bite-verified: `dired.revert`'s re-seat is
  guarded on the active buffer (an ambient `move_to_line` after an await
  moved an unrelated buffer's cursor — the buffer-level instance of
  S1-9); `fmt_size` keeps the column width past ten digits, because
  `_layout` is a contract Stage 3 is planned against; and the symlink
  descent dropped its probe, since `open_directory`'s
  changed-nothing-on-failure invariant *is* the probe (it was listing the
  target directory twice). Plus a consecutive-`readdir`-error cap, because
  **nothing cancels a dired listing** — it carries no supersede key, so
  cancellation was never the backstop the tolerant loop implicitly relied
  on. Naming/comment findings taken as-is.
  - Durable process lesson, hit twice now: a mutation-bite helper restores
    with `git checkout --`, which reverts to **HEAD** — so a fix must be
    committed *before* it is bitten. Round 1's fixes were briefly wiped by
    exactly that.
- **Canonical main integrated twice** — at `46a1b8f` (multi-root LSP
  affinity #161) and again at `b889873` (GPU terminal input #166), both
  merged rather than rebased per the #135/#137 precedent so the review
  anchors stay addressable. Each conflict was a single doc hunk resolved
  as the union: this lane owns COHERENCE's journey step 7 file half, #161
  owns the in-flight list, #166 owns step 8's GPU-terminal addendum.
  Three things worth carrying:
  - **A conflicting PR silently stops running CI.** GitHub builds
    `pull_request` runs against the merge ref, which does not exist while
    the PR conflicts, so no run is created and nothing reports a
    failure — the checks list simply stays as it was. Three pushes to
    this branch produced no CI at all before the cause was found. Watch
    `mergeable` on a long-lived lane, not just the check list.
  - #161's own COHERENCE finding **falsified a claim in this lane's
    module doc**: `pmacs.error` is never defined in production, so an
    uncaught raise inside a `pmacs.async` coroutine does not reach
    `*errors*` as the comment said. It reaches a bare `error()` inside
    `pmacs._async.tick()`, whose result `tick_async` discards with
    `let _ =` — i.e. nowhere. That makes dired's per-coroutine `pcall` +
    `set_status` load-bearing rather than tidy, and the comment now says
    so.
  - **A lane in review against a fast-moving `main` needs its gates rerun
    per integration, not per push.** Main advanced twice inside this
    review round, and the second time landed while the first
    integration's sweep was still running. The numbers below describe the
    twice-merged tree.
- Verification on the twice-merged tree (`main` @ `b889873`):
  `cargo fmt --check` clean; strict workspace Clippy clean; **1,832
  default + 2,009 CRDT** library tests; dired acceptance **25 default +
  25 CRDT**; m8_1 10 / m8_2 15 / m8_3 32 unchanged; multi-root 13 and
  vterm Stage 3 5 (both suites main added, green under this lane's
  `mod.rs` and `editor.rs` changes); M4 121; required GPU 155;
  **isolated-`XDG_CONFIG_HOME` workspace sweep 3,205 passed across 93
  suites, zero failures**; `git diff --check` clean. The sweep needs the
  isolated config for the reason recorded in the bottom-panel lane
  below.
- Coherence (framing §0.5, required since #163): serves `COHERENCE.md` §20
  Priority 1, which names this work explicitly; journey step 7's file half
  goes from no surface to a surface; **adds no interaction island** — keys
  are a mode-scoped keymap, and wdired will be a mode swap; adopts
  `pmacs.config` for `dired.kill-when-opening`; inherits §9's
  worker-attribution gap for its `read_dir` jobs without worsening it. The
  audited claims this changes are updated in `COHERENCE.md` itself, per its
  §25.
- **Boundary with the Journey Stage 1 arc** (`COHERENCE.md` §20 arc-cut
  1): CLI directory-argument handling (`pmacs .` exits 1) belongs there,
  not here — Stage 1 does **not** fix it. The two meet at
  `resolve_target_buffer`; dired supplies the buffer a directory should
  resolve *to*, and `pmacs .` should route into it rather than growing a
  second directory surface.

## GPU terminal input lane — IN REVIEW

- Portable branch: `githubsucks/gpu-terminal-input`, worktree
  `../pmacs-gui-term-input`, based on `githubsucks/main` @ `46a1b8f`.
- Approved framing: `docs/gpu-terminal-input-framing.md` revision 2,
  committed as the branch's first commit (`9a0df21`). Bug fix, not a
  feature; **no protocol change (stays v20)**.
- Reported as "text input within the terminal doesn't work on GUI, this is
  fine in TUI". Root cause: the dispatcher applied **both** terminal-layout
  syncs to **every** attached frontend, and a semantic session satisfies both
  conditions (a `term_sizes` entry from `AttachRequest` *and* a terminal
  declaration). Its PTY was resized twice per tick forever — grid arm installs
  the TUI placement size, semantic arm installs the declared content
  rectangle, each arm's idempotence guard seeing only what the other just
  wrote — so the child took a `SIGWINCH` storm at tick cadence.
- **The fix is a split, not a guard.** The grid arm is also the only per-tick
  controller-liveness release a semantic frontend gets, and
  `sync_semantic_terminal_layout` cannot take that over: the buffer-follow
  snapshot clears the viewport declaration (`on_buffer_snapshot_sent`), so
  that arm stops running in exactly the switch-away case that needs the
  release. `sync_terminal_layout` is therefore split into a
  frontend-kind-neutral half (panel reconcile + liveness) and a grid-only
  geometry half, with the loop body extracted to
  `sync_terminal_layouts_for_tick` so the exclusivity is structural and tests
  drive the real thing.
- **Trap for anyone touching this again:** the release at the "no
  `window_placements` entry" arm reads like liveness and is grid geometry. A
  semantic frontend has no placement entry at all, so moving it into the
  neutral half releases a GPU controller every tick.
- Bite-verified against **two** pre-images, because the naive guard fixes the
  storm and introduces the leak:

  | pin | `main` | naive guard | the split |
  |---|---|---|---|
  | settle (acc 2+3) | FAIL | pass | pass |
  | controller release (acc 6) | pass | FAIL | pass |
  | grid still resizes (acc 5) | pass | pass | pass |

- Real-path evidence: a quiet child trapping `SIGWINCH` reports **144 frames
  in 4 s and `WINCH 1..12` on screen** against the pre-fix tree, versus a
  settled screen with the fix.
- **Deliberately out of scope, named:** interactive-shell echo on a raw-mode
  PTY (Q#GT5 — reproduces in-process too, so it is not the GUI/TUI
  asymmetry), and a geometry change appearing to clear the visible screen
  (reproduces pre-fix; why acceptance 4 latches its observation across
  frames).
- Verification on this branch: `cargo fmt --check` clean; strict workspace
  Clippy clean; 1,829 default + 2,006 CRDT library tests; vterm Stage 1/2/3
  10 / 6 / 9 CRDT; bottom-panel Stage 1 46; M4 121; required GPU 155;
  **isolated-config workspace sweep 3,177 across 92 suites, zero failures**;
  `git diff --check` clean. Gates were run against the committed tree.

## Terminal config + copy mode arc — Stage 1 IN REVIEW

- Approved framing: `docs/terminal-config-and-copy-mode-framing.md`
  **revision 4** (four review rounds), committed as the first commit of
  Stage 1's branch. Two stages, two branches, two PRs; **no protocol
  change**.
- **Stage 1 = `githubsucks/terminal-config`**, worktree
  `../pmacs-terminal-config`, based on `githubsucks/main` @ `d152120`
  and merged up to `c93f9ee` during review round 1. Profiles,
  scrollback, escape key, and the `C-c t` opening binding.
- **Stage 2 = `terminal-copy-mode`, not started.** Branch it off `main`
  after Stage 1 merges: no dependency, but both edit
  `builtin/runtime/terminal.lua`.
- Load-bearing decisions, each forced by scouted ground truth:
  - profiles are a **raw Lua table** — `ConfigValue` is four scalars with
    no table kind, so they join `pmacs.lsp.config` / `pmacs.pair.sets`;
  - the **two open-time settings resolve through the global chain**,
    because they are read before the identity buffer exists; only
    `terminal.escape-key` resolves per buffer;
  - the escape cache lives on **`TerminalSession`** so its lifetime is
    the terminal's. `value_epoch` alone is not a sufficient key: it does
    not advance when focus moves between terminals with different
    buffer-local values;
  - repeating the escape sends **that chord**, not a hardcoded `0x03`.
- **Four bites, each against a different plausible wrong
  implementation** — hardcoded ETX fails acc6/9; epoch-only cache key
  fails acc7; single last-entry cache fails acc8's parse count; removing
  the invalid-value fallback fails acc10. The first version of acc7
  passed against the epoch-only bite because it asserted only that
  terminal A still worked; the discriminating assertion is that **each**
  terminal honors its own chord and not the other's.
- Test instruments worth reusing: `cat -v` is the echo probe, because the
  screen rejects C0 controls before they reach cells so a raw echoed
  `Ctrl-X` is invisible; and the probe **counts occurrences** rather than
  testing presence, because a single-character probe collides with the
  child's own banner text.
- **Review round 1 (2026-07-25) — five findings, all real, all fixed.**
  One blocker and two majors were the same failure in three places: a
  claim asserted somewhere cheaper than where it lives.
  - *Blocker — `COHERENCE.md` was stale in four places, not the three
    reported.* Step 8 still read "no keybinding"; §11 still read "five
    settings"; and §6's dispatch table still cited
    `is_terminal_escape_chord`, **a symbol this PR deletes**. §25 makes
    that update ride the PR. A PR that changes audited ground truth has
    to re-grep the audit for its own symbols, not only for its topic.
  - *Major — acceptance 5 was vacuous.* It asserted a registry
    round-trip, so it stayed green with the setting's **only** consumer
    deleted. It now opens a real terminal whose child overflows the
    24-row screen, scrolls the view to its oldest retained row, and
    asserts `LINE001` is present at 10,000 and absent at 0. **Asserting
    a value was stored is not asserting anything reads it.**
  - *Major — acceptance 8a asserted the session count, not the cache.*
    An editor-side map with no purge hook — the exact rejected design —
    leaks *while* sessions drain, so it passed. Fixed with a
    `TerminalManager::escape_caches()` seam. **A lifecycle claim needs a
    lifecycle observable.**
  - *Moderate — `table.sort` over user-controlled profile keys.* A
    table holding both a string and a numeric key raised `attempt to
    compare number with string` **on the unknown-profile path**,
    replacing the diagnostic being asked for; `%q` raised likewise on a
    non-string `profile` argument. Both are partial functions applied to
    user input **on a diagnostic path** — the error reporter was the
    thing that failed.
  - *Minor — the committed framing still said "not yet approved".*
- **Three new bites, each falsified by revert**: deleting the scrollback
  consumer fails acc5 (and only acc5); restoring the raw-key sort
  reproduces `attempt to compare string with number` verbatim; and
  implementing the rejected editor-side map fails the new acc8a at
  `left: 2, right: 1` **while passing the old session-count version** —
  which is the review finding demonstrated rather than argued.
- Verification after the round-1 fixes, on the tree merged with
  `githubsucks/main` @ `c93f9ee`: `cargo fmt --check` clean; strict
  workspace Clippy clean; 1,832 default + 2,009 CRDT library tests;
  `terminal_config_acceptance` **12/12 in both configurations**; vterm
  Stage 1/2 9+10 / 6+6; config registry 16+16; bottom-panel Stage 1
  46+46; M4 121; required GPU 202; `git diff --check` clean.
  - `compile_mode_acceptance` fails 11/67 against the **real** user
    config and passes 67/67 with an isolated `XDG_CONFIG_HOME` — the
    known pre-existing trap, not this branch.
  - **`vterm_stage3_acceptance::a37` fails on this machine — and fails
    identically on the PR's own base `d152120`**, so it is not this
    branch's regression. It is load-sensitive: it passed at `d152120`
    once and failed at that same commit twenty minutes later, with a
    second agent saturating the machine with `rustc` in between. Two
    ways it lies, both worth knowing: it **silently returns `ok` when
    `pmacs-gpu` is not built** in the same target dir (only
    `PMACS_REQUIRE_GPU=1` promotes that skip to a failure, and the gate
    list applies that flag to `-p pmacs-gpu`, a *different* package), and
    it is **crdt-gated, so CI has never run it at all**. A green a37 in
    a gate log means nothing unless the binary was built and the flag
    was set. Needs its own lane; see the CI `crdt`-coverage lane on #168.
  - `pmacs-gpu` itself failed 201/202 once under the same load and passed
    202/202 on immediate rerun.

## Bottom-panel lane (Arc 7) — Stage 1 MERGED; Stage 2 IN FRAMING

Stage 1 is on `main`. **Stage 2 is in framing**, no implementation in
flight.

- Stage 1 merged as **#155** (`main` @ `e745068`, 2026-07-24, after two
  review rounds). No protocol change. Durable substrate facts live in
  `docs/agent-handoff.md` §1; the two round lessons are in §5.
- Landed-docs follow-up merged as **#156** (`main` @ `d152120`,
  2026-07-25).
- **Stage 2 framing: `docs/bottom-panel-stage2-framing.md` revision 4**,
  on branch `githubsucks/bottom-panel-stage2-framing` (three commits,
  one per revision), worktree `../pmacs-bp-stage2`, based on
  `githubsucks/main` @ `ccf29e3`. Round 1 closed 2 blocking + 3 high;
  round 2 closed 1 blocking + 2 high + 1 medium and decided both open
  items; round 3 closed 1 blocking + 1 high + 1 medium. No open items
  remain. The approved
  parent framing `docs/bottom-panel-framing.md` (rev 4) remains
  authoritative, **including its acceptance criteria 37–55**.
- Retained, carrying nothing unmerged: branch `bottom-panel` and worktree
  `../pmacs-bottom-panel`.
- **Stage 2 ships as two serial slices**, 2A landing before 2B branches:
  **2A** = classified §1.3 census routing + `paint_frame` per-window
  painter extraction (with the active-window auto-scroll preparation), no
  protocol change; **2B** = protocol **v21**
  (`InstanceMessage::PanelFrame` plus
  `FrontendEvent::{FrontendCellGeometry, PanelResizeRows, PanelPointer}`,
  gated both directions, each extended enum byte-pinned on its own
  previous final variant), daemon panel projection, the GPU band, and the
  negotiated `panel_capable` flip. Stage 3 is the adopter default flip.
- **Correction — this entry previously mis-stated the census contract.**
  It is **not** "route every consumer through `primary_document_window`".
  Q#BP14 classifies the 23 reads into four classes and routes only the
  **Projection** class that way; focus/input (#13–#15, #23), focus chrome
  and surface-routed (#16–#19), and focus/session (#20) keep their own
  authorities. Rerouting them would break remote-op validation and
  application, `DispatchIdle`, presence, focused search/menu/completion
  routing, and terminal bell ownership. The Stage 2 framing carries the
  full table.
- **The GPU document bottom is three boundaries, not one.**
  `text_area_bottom` (`pmacs-gpu/src/main.rs:8490`) is today
  `status_band_top`, `geometry_capacity_bottom`, and
  `document_text_bottom` at once. Once a band is installed they diverge:
  the status chrome must stay pixel-identical at the physical window
  bottom while document consumers move. A blanket rewrite of that helper
  moves both together and passes an "everything moved" assertion, so the
  Stage 2 criterion asserts **both directions in one scenario**. The
  census is 20 production sites (8 status-owned, 12 document-owned) + 1
  definition + 8 test sites = 29 matches; the framing carries the
  per-site table. The three easiest to misclassify are document
  completion `:6140`, minibuffer candidates `:7351`, and edge scrolling
  `:8561` — each with its own visible symptom.
- **Folding Stage 3 and this arc's Stage 2 both touch the semantic
  projection.** Whichever is framed second re-scouts the other's landed
  state.

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

## Documentation lane

- Portable branch: `githubsucks/handoff-2026-07-20`
- Carries synchronized `AGENTS.md` / `CLAUDE.md`, this ledger, the
  durable handoff refresh, and the keybinding reference correction.
- It changes no runtime code.
- Review and merge this documentation branch separately; it must not be
  folded into a feature framing branch.
- Now also absorbs both landed arcs: Vterm Stage 1 (#126) and the config
  registry (#127). Canonical `main` is merged into it up to `2e37c04`,
  so its diff against `main` is documentation only.

## Closed since the last snapshot

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
  - Gating fact found on the way: **the workspace sweep must run with an
    isolated `XDG_CONFIG_HOME`**, because the real user `init.lua`
    installs a local package and the losing race leaks a status message
    into painted-frame comparisons. There is also a latent pre-existing
    `main` bug in the buffer CRDT undo path, unrelated to this arc.
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
