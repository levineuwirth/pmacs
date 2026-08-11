# Git integration — Stage 1: seeing what changed

**Status: revision 5, APPROVED 2026-08-09. Implementation may
proceed.**

**Revision 5 completes the unborn-repository policy, which revision 4
wrote as three disjoint rows when a single file can be in two states at
once.** `AM` — staged, then edited again — is not exotic; it is what a
first commit looks like halfway through. The states below were
**enumerated from a real unborn repository**, not reasoned about, and
one of them settles a case by ruling it out entirely.

**Revision 4 fixes two contracts that would have failed in ordinary
use, both verified against real behaviour rather than reasoned about:**
re-binding `d` on every refresh (keymap binds refuse duplicates, so
*every successful refresh* would have errored), and two git exit states
the failure predicate got wrong. Measured, not assumed — the exit codes
below were produced in a scratch repository.

**Revision 3 pins four Stage 1 contracts revision 2 left loose, and two
of those were again claims I made without reading the code I was
crediting.** I attributed selection preservation to `listview` and
`d` to its key surface; neither is true, and both were checkable in the
file I had already cited. The pattern is worth naming since it has now
recurred across three revisions: **I cite a file, then describe what I
expect it to contain.**

**Revision 2 answers four blockers, two of which were factual errors in
revision 1 that scouting should have caught and did not.** I read
`ProjectKind::Git`'s name instead of its doc comment, and I quoted
`COHERENCE.md` §15's "no Git integration anywhere in the tree" without
checking whether it was still true of the tree. It is not.

---

## 1. Why this, and why now

`COHERENCE.md` §15 is blunt about it:

> **There is no Git integration at all** — no status, stage, diff,
> blame, or gutter markers anywhere in the tree (gutter git riders and
> the `ResourceOffer` diff/blame family are named deferrals). The Git
> affordance list above has nothing to attach to yet.

**That sentence is literally false about the tree, and revision 1
repeated it without checking.** `tests/fixtures/pmacs-magit/` is a
tracked, installable package — 1,914 lines across four modules, with
`status.lua` spawning git through `pmacs.process.spawn` and parsing
**`--porcelain=v2 --branch`** into structured sections, plus a 662-line
acceptance suite (`tests/m8_6_acceptance.rs`, 32 tests) covering
status, refresh, staging, commit, push and branch behaviour.

**The PRODUCT gap is real and unchanged** — none of that is bundled
runtime, so a user who installs pmacs gets no git integration. But
"nothing to attach to" understates what exists to *learn from*, and
§15's wording should be corrected when this lands.

For a **daily driver**, this is the largest remaining gap. Not because
git is the most architecturally interesting thing missing — §7
workspaces and §9 worker identity are both deeper — but because it is
the one a user touches *every working hour*, and pmacs currently makes
them leave the editor to answer "what have I changed?".

That is the criterion this lane is chosen against: **frequency of use
per day**, not depth of model.

## 2. Ground truth — what already exists

Scouted, not assumed:

- **`ProjectKind::Git` is NOT general repository detection**, and
  revision 1 said it was. Its doc comment is explicit: *"A bare git
  repository (no language marker found inside)"* (`src/project.rs:89`).
  Markers are ordered and a language marker beside `.git` **wins**
  (`src/project.rs:10`), so a normal Rust repository reports
  `kind = "rust"` and would have been invisible to a lane that gated on
  `kind == "git"`. That gate would have failed on this very repository.

  **The rule this lane uses instead: never ask pmacs whether it is a
  git repo.** Run git in the **active file's directory** and let git
  resolve its own worktree — `git -C <dir> rev-parse --show-toplevel`
  establishes the root, and a non-zero exit *is* the "not a repository"
  answer. Git's own resolution handles submodules, worktrees, `GIT_DIR`
  and `.git` files; a marker walk reimplements a subset of that and
  gets it subtly wrong.
- **`pmacs.process.spawn` / `events_take` / `terminate` / `forget`**
  is the working model for running an external tool asynchronously;
  `builtin/runtime/compile.lua` is a full worked example, including
  spawn-failure handling and exit markers.
- **`pmacs.listview.open`** is a real primitive with existing adopters
  (`*references*`, `*lsp*`), carrying optional `depth`/`id`,
  primitive-owned collapse, and selection re-seated by id. `COHERENCE.md`
  P5 says the remaining work there is **adoption, not construction** —
  a `*git-status*` panel is exactly that.
- **Gutter signs exist in both frontends** — the TUI's leading-column
  glyph (`src/diag.rs`) and the GPU's `GUTTER_SIGN_X` bars
  (`pmacs-gpu/src/main.rs:420`).
- **A tested porcelain-v2 parser exists as a package fixture** (above).
  Its `status.lua` deliberately separates **pure `parse_*` functions
  that take a string and return structure** from the spawning around
  them — which is the shape that makes a parser testable without a
  repository, and it is already proven by 32 tests.

And the constraint that shapes the staging:

- **`DecorationKind` is a CLOSED enum on the wire**
  (`pmacs-protocol/src/message.rs:1472`): four diagnostic severities,
  `Selection`, `SearchMatch`, `SearchMatchActive`, `CurrentLine`.
  **Gutter markers for git hunks therefore require new variants, which
  is a protocol version bump.** The gutter signs that exist are keyed
  on `diagnostic_severity_rank` and have no notion of anything else.

## 3. The staging, and why the line falls where it does

**Stage 1 (this lane): read-only, panel-based, NO WIRE CHANGE.**

- `*git-status*` — a `listview` panel over
  `git status --porcelain=v2 --branch -z` (Q#G-6), rows visiting the
  file at RET, refreshed by `g` under the completion model in Q#G-1.
- `*git-diff*` — the diff for the **file** under point (Q#G-7), in a
  generated buffer rendered as **plain text** (no `diff` grammar
  exists). **No hunk model** — hunks are Stage 2's concern.

**Stage 2 (separate lane): gutter markers.** Needs new
`DecorationKind` variants and a `PROTOCOL_VERSION` bump, plus both
frontends' gutter renderers learning a second rider family.

**Stage 3+ (unscheduled): staging, commit, blame.** Staging and commit
are where an editor becomes a git *client*; blame is a lower-frequency
read. Neither belongs in front of the two above.

**The line is drawn at the wire on purpose, and it is a scheduling
decision as much as a design one.** Parallel lanes are about to start,
and `PROTOCOL_VERSION` is a strict serialization point — two lanes
bumping it collide, and this session already recorded what that costs
(eight broken version assertions on CI from a single bump). Stage 1
touching no wire is what lets it run **concurrently** with other work.
Stage 2 must be scheduled alone.

## 4. Coherence impact (§20)

Required by `CLAUDE.md` for coherence-affecting work, and this is
coherence-affecting — it is §15's named gap.

- **Journey steps touched:** none directly. Git is not currently a
  journey step; the golden journey runs open → edit → build → test →
  navigate. This lane does **not** add a step, and I would rather say
  so than inflate the claim.
- **§15 contextual affordances — the direct target.** The audit's git
  affordance list ("a Git change stage/revert/diff") has *nothing to
  attach to*. Stage 1 creates the thing to attach to; the affordances
  themselves follow it, and the menu's context vocabulary
  (`src/menu.rs:44`) would need a `git` context to host them — **out of
  scope here**, named so it is not forgotten.
- **§14 workbench primitives — adoption, which is the stated need.**
  `*git-status*` becomes the **fifth** `listview` call site and the
  first outside the LSP panels, which is the concrete evidence P5 asks
  for that the primitive generalizes past its first consumer.
- **Interaction islands (§6): none added, and this is a real
  constraint.** The panel gets no hardcoded key interception; it uses
  `listview`'s existing key handling. §6 records six such shadows and
  calls them "weak, and growing" — this lane must not make it seven.
- **Config registry adoption:** at least one setting
  (`git.enabled`, Q#G-4), defined through `pmacs.config.define` like
  `ui.line-wrap` and the zoom settings, not a bare Lua global.
- **Background-work attribution (§9): NEGATIVE, and named as such.**
  Git runs as a spawned process, and spawned processes do **not** appear
  in `*workers*` — that view is `async.lua`'s job list; processes live
  under `pmacs.process.list` (Q#G-5). This lane therefore adds a fifth
  thing running in the background with no single place to see it. The
  process is labelled honestly, which is better than anonymous, but
  **a label is not attribution and this document does not pretend
  otherwise.** Accepted because these are short-lived reads; it would
  not be acceptable for Stage 3's push/pull.

## 5. Open questions

### Q#G-1 — is the status panel a snapshot or a live view?

A snapshot is a command that opens a panel; a live view refreshes on
buffer save, on focus, or on a filesystem watch.

*My vote: **snapshot, refreshed explicitly***, with `g` re-running
inside the panel. Live refresh needs a watch mechanism, an invalidation
rule, and a §9 story for the recurring work — all real arcs. A snapshot
is honest, useful the first day, and does not pretend to a currency it
cannot maintain.

**But "explicit refresh" does not fit `listview` unmodified, and
revision 1 missed that.** `listview.refresh` is synchronous:

```lua
local rows = check_ids(p.on_refresh() or {})   -- listview.lua:402
```

The result is consumed immediately. `pmacs.process.spawn` cannot return
rows there — it returns a process id whose output is drained later. So
revision 1's "adopt `listview`" would have produced exactly one of the
two failures the reviewer named: a reimplemented list, or a `g` that
silently does nothing. The primitive's own docs already call a dead `g`
out as a defect it must not repeat (`listview.lua:416`).

**The completion model, specified.** `on_refresh` stays synchronous and
honest:

1. **`on_refresh` returns the CURRENT rows immediately**, with a
   `refreshing…` marker row appended, and *kicks off* the spawn. `g` is
   therefore never a no-op — it always re-renders and always shows that
   work started.
2. **On exit, the completion handler re-opens the panel** via
   `listview.open` with the same `name` — **and re-seats the selection
   itself.**

   Revision 2 credited that to the primitive and was wrong.
   `listview.open` **resets collapse** (`p.collapsed = {}`) and
   **always seats line 1** (`seat_cursor(p, 1)`,
   `builtin/runtime/listview.lua:337-378`). The `listview.lua:82` note
   I cited is about **name** disambiguation to `<2>`, not selection.
   Only `listview.refresh` preserves a selection, and that is the
   synchronous path this model cannot use.

   So the contract is explicit and owned here: **capture the selected
   row's git id (its current path) before re-opening, and after
   re-opening move to the line whose row carries that id**, computed
   from the handler's own rows array via `pmacs.editor.move_to_line`.
   If the id is gone from the new status — the commonest case, since a
   file that stopped being modified drops out — seat line 1 and say
   nothing; that is the correct answer, not a failure.

   **Collapse state is moot in Stage 1** because the rows are flat: no
   `depth`, so nothing to collapse. Stage 2 or a sectioned view would
   have to revisit this, and would then face the same reset.
3. **Concurrent refresh is suppressed by a generation counter.** A
   second `g` while one is in flight bumps the generation; the older
   completion sees a stale generation and **discards its rows** rather
   than racing. It does not terminate the first process — reaping is
   `pmacs.process.forget`'s job and killing git mid-read buys nothing.
4. **Failure is a row, not a silence.** Non-zero exit or spawn failure
   renders a row carrying the exit code and the first stderr line, plus
   a status message. §1.2's silence asymmetry.
5. **Panel lifetime.** If the panel's buffer is gone when the process
   exits, the handler drops the result. `compile.lua:252` already
   handles the buffer-killed case for its own slot; the same shape.

**The alternative — extending `listview` with an async contract — is
the more correct long-term answer** and is deliberately not taken here:
it changes a primitive with four existing adopters, and doing that from
inside its fifth adopter's lane is how a primitive acquires a consumer's
idiosyncrasies. **If review prefers it, it belongs in its own lane
before this one.**

### Q#G-0 — what is the relationship to `pmacs-magit`? **(new in rev 2)**

The reviewer's framing of the choice is right: adopt, replace, or
declare it out-of-product precedent. Doing none of those and quietly
writing a second parser is the option that must not happen.

*My vote: **port its pure `parse_*` functions and its test corpus into
the bundled runtime; leave the fixture itself untouched.***

- **The record TOKENIZER is deliberately rewritten, not ported.** The
  fixture parses **newline-delimited** v2; Stage 1 reads **`-z`**, and
  those are different grammars — under `-z` a record's fields are
  NUL-terminated and a rename carries its two paths as separate fields
  rather than tab-joined. Saying "port the parser" would have been
  wrong; what ports is the **separation** (pure `parse_*` functions
  over a string, testable with no repository) and the **case coverage**
  its 32 tests encode. The tokenizer underneath is new, and its
  correctness rests on this lane's own corpus.
- **Port, not import.** The fixture's purpose is to prove the *package
  system* can host this. If bundled code became its dependency, it
  would stop demonstrating an independent package and `m8_6` would test
  less than it claims.
- **The duplication is therefore deliberate**, and it is the one place
  this framing accepts two copies of a rule after a session spent
  removing them. The justification is that they answer different
  questions — one is product behaviour, one is package-system
  capability — and coupling them weakens the second. **If review
  prefers the coupling, that is a defensible call and I will take it**;
  what I will not do is leave the duplication unstated.
- **It also settles Q#G-2's format**: the existing, tested parser is
  **porcelain v2**, so Stage 1 is v2. Revision 1 said v1 for no reason
  beyond familiarity.

### Q#G-2 — `git` the binary, or a library?

*My vote: **the binary**, via `pmacs.process.spawn`. `compile.lua` is
the worked precedent, the daemon already spawns external tools, and a
git library is a dependency with a much larger surface than "run one
command and parse porcelain". `--porcelain=v2` is explicitly a stable
machine format; that is what it is for.

**Named risk:** no `git` on `PATH`. §1.2's *silence asymmetry* says the
failure must be **surfaced with guidance**, not swallowed — the same
lesson #204 landed for a missing language server.

### Q#G-6 — the status data contract **(new in rev 2)**

Revision 1 said "`--porcelain=v1`" and proposed "a path with a space"
as the parsing witness. **Both were inadequate.** Porcelain without
`-z` emits paths in git's **C quoting** for anything non-ASCII or
containing special characters, and rename/copy records carry *two*
paths whose separation is positional. A single space-in-path fixture
proves none of that.

*My vote: the exact invocation*

```
git --no-optional-locks -C <dir> status --porcelain=v2 --branch -z
```

**`--no-optional-locks` is part of the contract, not a nicety.**
`git status` is **not strictly read-only**: it may refresh and write
the index, and git's own documentation recommends this flag for
background scripts precisely so a background reader does not contend
for `index.lock` with the user's real git commands
(<https://git-scm.com/docs/git-status>). This lane runs status
*asynchronously, from an editor, while the user may be running git in a
terminal* — the exact scenario the flag exists for. Revision 2 called
the lane "read-only" and that was wrong about the mechanism.

It is **witnessed structurally** — the assembled argv is asserted to
carry the flag — because observing a lock that was *not* taken is not
something a test can do directly. Verified accepted by the git in use
here.

The rest: `--porcelain=v2 --branch -z`, also verified accepted. NUL delimiting removes C quoting from
the problem **entirely** rather than obliging a hand-written unquoter,
and it makes the two-path rename record unambiguous: the paths are
separate NUL-terminated fields rather than tab-joined inside one.

The rename/copy identity rule to pin: a `2` record carries the current
path **and** its origin, and the panel must show which file it is now
while remembering where it came from — a row whose id is the current
path, since that is what RET visits.

**Witness corpus, not one case:** modified, added, deleted, untracked,
**renamed (both paths)**, **copied**, a path with a space, a path with
a newline, and a non-UTF-8 path. The last two are exactly what `-z`
buys and what a quoted parser gets wrong.

### Q#G-7 — the diff gesture **(new in rev 2)**

Revision 1 wrote "the diff for the file or hunk under point" while also
committing RET to visiting the file. **RET cannot do both, there is no
second binding proposed, and no hunk model exists anywhere in the
tree.**

*My vote:*

- **RET visits the file** — unchanged, and the behaviour a list of
  files should have.
- **A named command, `git.diff-file`, bound to `d` inside the panel.**

  **`d` is not on `listview`'s key surface**, and revision 2 said it
  was. The bound set is exactly `RET SPC n <down> p <up> TAB g q`
  (`builtin/runtime/listview.lua:266-279`), bound buffer-locally inside
  the primitive, which is the only place the panel's buffer handle is
  known. **Looking the buffer up by name from outside is unsafe** —
  `listview` deliberately disambiguates a collision to `<2>`, so the
  name a consumer passed is not necessarily the buffer it got.

  *My vote: **a `keys` table on the open spec***, e.g.
  `keys = { d = "git.diff-file" }`, bound through the same
  `bind_local_keymap` that already binds the fixed set. It is additive,
  general to any adopter, keeps binding where the buffer is known, and
  adds **no** interception — the §6 constraint holds.

  **The registration lifecycle, which revision 3 omitted and which
  would have broken the refresh path it depends on.** `Keymap::bind`
  **refuses duplicates** — `KeymapError::DuplicateBinding`, *"Refuse
  rather than silently overwrite"* (`src/keymap_tree.rs:75`) — and the
  completion model calls `listview.open` again on **every** refresh. A
  naive `keys` implementation therefore errors on the second open, so
  **every successful refresh would have failed while re-binding `d`.**

  The contract:

  1. **Keys are installed once, when the panel's buffer is created**,
     and stored on the panel.
  2. **A later `open` for a live panel does not re-bind.** It
     **compares** the supplied `keys` against the stored table and
     **errors on divergence** rather than ignoring it. Silently keeping
     the old binding would give the consumer a key that does something
     other than what it just asked for — a dead or lying key, which is
     the defect `listview` already condemns for `g`.
  3. **Collisions are rejected at install time**, against both the
     fixed set (`RET SPC n <down> p <up> TAB g q`) and any
     **prefix conflict** — `Keymap` has a separate error for turning a
     leaf into a submap, and a `keys` table must not be able to reach
     it.

  (The alternative, idempotent re-registration, is tolerable but
  strictly weaker: it makes a consumer that changes its keys mid-session
  silently wrong instead of loudly wrong.)

  **This IS a `listview` modification, and revision 2's "no listview
  modification" was false.** I distinguish it from the async-contract
  change I deferred: that one alters *when* an existing callback's
  result is consumed for four existing adopters; this adds an optional
  field that changes nothing for a spec that omits it. **If review
  judges any primitive change out of an adopter's lane, the alternative
  is `listview.open` returning the panel buffer** so the consumer binds
  its own key — smaller still, but it pushes binding to every adopter.
- **No hunk model in Stage 1.** Hunks are precisely what gutter markers
  need, and that is Stage 2's protocol work. Introducing a half hunk
  model here to serve one gesture would prejudge Stage 2's design from
  the wrong side.

**And what `d` actually SHOWS, which revision 2 left unstated.** "File,
not hunk" is a scope, not a contract. A porcelain-v2 row carries an
**XY** pair — X staged (index vs HEAD), Y unstaged (worktree vs index)
— and the three plausible diffs answer three different questions:
`git diff` shows only Y, `--cached` only X, and neither shows an
untracked file at all.

*My vote: **`d` answers the lane's own question — "what have I
changed?" — against `HEAD`:***

| row | `d` runs | why |
|---|---|---|
| staged, unstaged, or both | `git diff HEAD -- <path>` | one view of the total change; splitting X from Y is a staging UI, which is Stage 3 |
| deleted | `git diff HEAD -- <path>` | shows the deletion; no special case needed |
| renamed / copied | `git diff HEAD -- <orig> <current>` | v2 gives both paths; passing both is what lets rename detection render it as a rename rather than an unrelated add+delete |
| **untracked** | `git diff --no-index -- /dev/null <path>` | **a normal diff shows nothing at all** for an untracked file. Without this case `d` is silently dead on the rows a user is most likely to press it on |
| non-UTF-8 path | *refuses, with a message* | see Q#G-8 |

The `HEAD` choice is deliberate and is the one thing here I would most
expect review to push back on: it is the right default for *reading*
what changed, and the wrong one for *staging*, which is why it is
correct for Stage 1 and will need revisiting when Stage 3 arrives.

**The exit-state contract, which revision 3 got wrong in two ways.**
"Non-zero exit renders a failure row" is not correct for `git diff`.
Both cases below were measured in a scratch repository, not inferred:

**(a) `--no-index` implies `--exit-code`.** It exits **1 when it
successfully finds differences** — measured: `exit=1` for an untracked
file against `/dev/null`. Under revision 3's predicate, *every*
untracked diff — the case `--no-index` exists to serve — would have
rendered a failure row instead of the diff it just produced.

So for the untracked path the success predicate is **exit ∈ {0, 1}**,
rendering whatever came out; **exit ≥ 2 is a real failure**. That
asymmetry is confined to the `--no-index` invocation and does not leak
to the others, where non-zero still means failure.

**(b) An unborn repository has no `HEAD`.** Measured:
`git diff HEAD -- <path>` exits **128** with `fatal: bad revision
'HEAD'`. This is not an edge case — it is a freshly `git init`-ed
repository with the first files staged, which is exactly when someone
opens a status panel to see what they are about to commit.

*Policy: **detect once, then split**.*

**Detection needs no extra subprocess.** `--branch` already reports
`# branch.oid (initial)` when `HEAD` is unborn — observed in the
output this lane already parses. Revision 4 proposed a separate
`git rev-parse --verify --quiet HEAD`; that is a second process for a
fact the first one hands over.

**The reachable states, enumerated from a real unborn repository** —
`git init`, stage three files, then edit one, delete one, and `git mv`
one:

```
# branch.oid (initial)
1 AD ... ad.txt
1 AM ... am.txt
1 A. ... r_new.txt      <- the `git mv`
? untracked.txt
```

Two findings fall straight out:

- **`AM` and `AD` are ordinary and carry BOTH states**, which is
  exactly the gap: `--cached` alone loses the worktree delta, plain
  `git diff` alone loses the staged base.
- **Rename and copy CANNOT occur under an unborn `HEAD`.** The
  `git mv` produced `1 A. … r_new.txt` — an ordinary add of the new
  path, **not** a `2` record. With no `HEAD` there is nothing to
  rename *from*, so the rename/copy row class is unreachable here and
  needs no unborn policy. That is a case closed by evidence rather than
  handled speculatively.

| unborn row | `d` renders |
|---|---|
| `A.` staged only | one patch: `git diff --cached -- <path>` |
| **`AM` staged + edited** | **two labelled patches** — *staged* `git diff --cached -- <path>`, then *unstaged* `git diff -- <path>` |
| **`AD` staged + deleted** | **two labelled patches**, same pair; the second renders the deletion |
| `.M` / `.D` unstaged only | one patch: `git diff -- <path>` |
| `?` untracked | `git diff --no-index -- /dev/null <path>` (exit ∈ {0,1}) |
| rename / copy | **unreachable** — see above |

All four `--cached` / plain invocations above were run against that
repository and render the expected patches.

**The split is unborn-only, and that asymmetry is deliberate.** Once
`HEAD` exists, `git diff HEAD` gives one total — which is the lane's
question — and splitting it would be a staging UI (Stage 3). The split
appears here only because there is no `HEAD` to total *against*.

The generated buffer carries a **header naming what it is showing**:
*"no commits yet — split view: staged (index) above, unstaged
(worktree) below"*. Revision 4's wording ("showing staged changes")
would have described a single total-against-`HEAD` diff, which is
precisely what this is not. A diff that silently answers a different
question than the one asked is worse than one that says so — and a
header that misdescribes a split view is the same failure in smaller
type.

### Q#G-8 — non-UTF-8 paths: an honest boundary **(new in rev 3)**

Revision 2 listed a non-UTF-8 path in the witness corpus as though it
were an end-to-end case. **It cannot be**, and the boundary is in the
bindings: `pmacs.process.spawn` takes `args: Vec<String>`
(`src/lua_bindings/mod.rs:8683`) and `pmacs.buffer.find_or_open` takes
`path: String` (`:3564`). Both are Rust `String`, i.e. UTF-8 by
construction. A path that is valid bytes but not valid UTF-8 can be
*read* from git's `-z` output and *displayed*, but it cannot be passed
back to `spawn` for a diff, nor opened.

*My vote: **parse it, show it, and refuse the gesture with a
message***:

- the row **appears** in the panel, so the user is not lied to about
  what is modified;
- **RET and `d` on that row report** that the path is not representable
  and do nothing else — a witnessed refusal, not a stack trace or a
  silent no-op;
- **it is removed from the end-to-end promise.** The witness is
  parser-and-display **plus the refusal**, and the framing does not
  claim visiting works.

Making it work end-to-end means `OsString`/bytes through two binding
boundaries — a real change to the Lua API surface, and not this lane's.

### Q#G-3 — what does the diff view render into?

*My vote: **a generated buffer**, reusing the generated-buffer
immutability work (Stage 1 merged; that lane's Stage 2 is queued).
Diff output is read-only text and that machinery exists.

**RESOLVED in rev 2 — there is no bundled `diff` grammar.**
`BUILTIN_LANGUAGES` (`src/syntax.rs`) has no `diff` entry; checked, not
assumed. **Stage 1 renders plain generated text**, and diff
highlighting is later work needing a grammar first.

### Q#G-4 — what is configurable?

*My vote: **one setting to start** — `git.enabled` (boolean, default
`true`), through the config registry. Resist more until there is use
evidence; §11's grade is "partial (foundation only)" and adding five
speculative settings is how a registry becomes noise.

### Q#G-5 — §9 attribution — **RESOLVED, and the answer is negative**

Revision 1 deferred this to implementation. That was wrong: it is
answerable by reading, and deferring it would have meant discovering a
known coherence cost *after* committing to the design.

**A spawned git process does not appear in `*workers*` at all.** That
buffer is `builtin/runtime/async.lua`'s (`:490`) and lists **async
jobs**; spawned processes live separately under `pmacs.process.list`.
They are two of the four disjoint activity views §9 grades as
"mechanism without identity".

So, stated plainly rather than dressed up:

- **This lane adds a fifth thing that runs in the background and is not
  attributable from one place.** That is a **negative** coherence impact
  against §9, and it is the honest cost of shipping git status before
  worker identity exists.
- **Labelling the process is still required** — a clear label under
  `pmacs.process.list` is strictly better than an anonymous `git`. But
  **a label does not solve attribution**, and this document does not
  claim it does. The claim is only: do not make it worse than it has to
  be.
- **The mitigation is bounded in time, not in kind.** These are
  short-lived reads, not long-running jobs; a `git status` that has not
  finished is a bug, not a background task a user needs to supervise.
  That is why the cost is acceptable *now* and would not be for
  Stage 3's push/pull.

## 6. Verification

- **Parsing, against a corpus rather than a case (Q#G-6):** modified,
  added, deleted, untracked, **renamed with both paths**, **copied**, a
  path with a space, and **a path with a newline** — the last is what
  `-z` buys, and a parser that passes only the space case is the one
  that ships broken.
- **A non-UTF-8 path is parsed and displayed, and its gestures refuse
  with a message** (Q#G-8) — a witnessed refusal at the binding
  boundary, **not** an end-to-end visit.
- **The argv carries `--no-optional-locks`** (Q#G-6), asserted
  structurally. A lock not taken cannot be observed directly, so the
  invocation is what gets pinned.
- **`d` is witnessed on every row class** (Q#G-7): staged, unstaged,
  both, deleted, renamed, and **untracked** — the last because a normal
  `git diff` shows nothing there, so a missing `--no-index` case makes
  `d` silently dead exactly where it is most used.
- **A copy is reported as a COPY, not a rename** (Q#G-7). Porcelain v2
  folds both into the one `2` record, so `kind` stays `"rename"` for
  both — every *behaviour* keyed on it is the same — and the
  distinction is made where it is a distinction: the diff header reads
  the `<Xscore>` field's leading `R`/`C` and says which one happened.
  The status row is left alone, because its `XY` prefix already reads
  `R.` against `C.`. Both classes are asserted, and so is the **argv**:
  the two-path `git diff HEAD -- <orig> <current>` is right for a copy
  and a rename alike, so a fix to what the user is *told* must not
  reach what runs. **Parser-level, deliberately** — the copy ROW is
  supplied through `_deliver_status` while the repository, the panel,
  the `d` dispatch and the spawned diff around it are real.

  **The reason, narrowed after review.** This bullet used to say real
  `git` emits no `2 C` record "even under `status.renames=copies`".
  **That is too strong, and git's own documentation contradicts it** —
  `git-status(1)` lists `C` as *"copied (if config option
  status.renames is set to `copies`)"*. What the test measures is
  narrower: **for its fixture, whose copy source is left unchanged**,
  git reports `1 A.`. That is a fact about the fixture, and it is
  sufficient reason to craft the row — a weaker and true justification
  in place of a stronger false one. No mechanism is claimed for why an
  unchanged source is not offered as a candidate; that was never
  established.
- **The untracked diff renders on exit 1**, not a failure row (Q#G-7a)
  — the case `--exit-code` semantics would otherwise break, and the
  one most likely to be "fixed" later by someone who reads exit 1 as an
  error.
- **An unborn repository is witnessed end to end**, and the fixture is
  **`AM`** specifically — staged then edited again, the shape a first
  commit actually has partway through. `git init`, stage, edit, open
  the panel, press `d`, and get **two labelled patches** with the
  split-view header — not `fatal: bad revision 'HEAD'`, and not a
  single `--cached` patch that silently drops the worktree edit.
  **`AD` rides the same fixture**, since one repository can hold both.
- **Unborn detection reads `# branch.oid (initial)`** from the status
  output already being parsed — asserted, so nobody later reintroduces
  a second `rev-parse` process for a fact already in hand.
- **Rename/copy under an unborn `HEAD` is asserted UNREACHABLE**: the
  fixture `git mv`s a staged-but-uncommitted file and the parser sees a
  `1 A.` record, never a `2`. Pinned so a future reader does not
  "fix" the missing unborn rename policy by inventing one.
- **Re-binding across a refresh does not error** (Q#G-7): two
  successive refreshes on a live panel, asserting `d` still works and
  no `DuplicateBinding` surfaced. This is the one that would have
  broken on every refresh.
- **A `keys` table colliding with the fixed set is rejected at install
  time**, as is a prefix conflict.
- **Selection is re-seated by the completion handler** (Q#G-1), across
  a refresh that reorders rows, and **falls back to line 1 without
  complaint when the selected path drops out of status** — the common
  case, not an error.
- **The pure `parse_*` functions are tested without a repository**,
  which is the shape `pmacs-magit/status.lua` already proves works and
  the reason to port that separation rather than invent one.
- **A repository fixture built with real `git`**, in a tempdir, and
  **bounded with `set_search_boundary`** — R8 was retired two commits
  ago and is precisely what happens when a fixture lets project
  detection escape into the developer's environment.
- **The root rule is witnessed on a repository whose `ProjectKind` is
  NOT `Git`** — i.e. an ordinary language project with a `.git` beside
  its manifest. That is the case revision 1's `kind == "git"` gate
  would have failed, and this repository is one.
- **Missing `git` on `PATH` is witnessed**, not assumed (Q#G-2), and
  surfaces guidance rather than silence.
- **`g` is never a no-op** (Q#G-1): it re-renders and marks that work
  started, even mid-flight. A dead `g` is a defect `listview` already
  names.
- **Concurrent refresh discards the stale generation** rather than
  racing — asserted by driving two refreshes and completing them out of
  order.
- **Failure renders a row**, carrying exit code and stderr.
- **The panel is a `listview` adopter**, asserted structurally, so a
  future re-implementation of list behaviour inside git code fails the
  test rather than passing review.
- **No new interaction island** — `d` is bound buffer-locally through
  `listview`'s own binding path (Q#G-7), not a hardcoded interception.
  §6 stays at six shadows.

Gates via `scripts/gate --acceptance <the new suite>`.

**What this will NOT prove:** that background git work is attributable
(Q#G-5 — it is not, by construction), or that the parser handles
porcelain versions other than v2.

## 7. Not in scope

Gutter markers and any `DecorationKind`/`PROTOCOL_VERSION` change
(Stage 2 — must be scheduled alone). Staging, commit, push, pull,
branch operations, merge-conflict resolution. Blame. A `git` context in
the menu vocabulary. Any git *library* dependency. Live refresh
(Q#G-1). Fixing §9's worker identity — this lane makes it marginally
worse and says so (Q#G-5). Any hunk model (Q#G-7). Modifying the
`listview` primitive to carry an async contract — the better long-term
answer, but it belongs in its own lane before this one, not inside its
fifth adopter (Q#G-1). Changing `tests/fixtures/pmacs-magit/` or
`tests/m8_6_acceptance.rs` (Q#G-0).

**A `listview` change IS in scope after all** (Q#G-7): an optional
`keys` table on the open spec. Revision 2 said no primitive
modification; that was false, because `d` cannot be bound from outside
the primitive safely. The async-contract change stays out.

**One correction this lane should carry when it lands:** `COHERENCE.md`
§15's "no Git integration anywhere in the tree" is literally false —
`tests/fixtures/pmacs-magit/` exists. The *product* gap it describes is
real; the sentence needs narrowing to say so.
