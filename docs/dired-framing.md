# Dired — framing

**Revision 7 — 2026-07-25. Status: APPROVED; Stage 0 MERGED as #162;
Stage 1 MERGED as #165 (`main` @ `c8ec8f3`, one review round). Stage 2
(marks and operations) and Stage 3 (wdired) each still need their own
framing before implementation; the frozen fixture shrinks after Stage 3.**
Rev 1 passed a ground-truth review; rev 2 fixed round 1's seven findings;
rev 3 fixed round 2's six and was approved; rev 4 recorded what Stage 0's
implementation falsified in the approved text (§0); rev 5 adds the
**coherence impact** statement now required of every framing
(`CLAUDE.md`, `COHERENCE.md` §20) — see §0.5; rev 6 records what Stage
1's implementation falsified (§0, S1-1…S1-9); rev 7 adds what its first
review round found (§0, S1-10…S1-12). Deliberately
unnumbered: the roadmap's Arc 8 is GPU
structural parity but `docs/lean4-mode-framing.md` also claims Arc 8, so
the arc space is already forked in uncommitted work. (Rev 2 also cited
`docs/dap-debugging-framing.md` as part of that fork — wrong: its Arc 7
*matches* the roadmap's Arc 7 = Debugging. R2-5.) Numbering this one
would mint a third claim; it is ranked when the roadmap is next
reconciled. Not on `docs/roadmap-2026-07.md` and not in
`docs/side-quest-backlog.md` — this framing proposes the work as well as
its design.

## 0. Revision history

### Round 1 (rev 1 → rev 2)

- **F1 (load-bearing).** §3's rationale for freezing the M8 fixture was
  **false**. Rev 1 claimed the 47 tests pin the package system —
  `install_local`, `on_unload` unregistration, `DuplicateName`, per-package
  `require` scoping. Verified: `install_local` appears only in the shared
  `editor_with_dired()` harness and in doc comments, never in an
  assertion; `on_unload`, `DuplicateName`, and require-scoping are
  asserted **nowhere** in either file; `dired_source_size_under_audit_ceiling`
  is a `lines < 1500` lint. Exactly **one** of 47
  (`dired_package_reload_is_safe_after_init_complete`,
  `m8_2_acceptance.rs:1190`) exercises package mechanics; the other 45
  assert dired/wdired behavior. §3 rewritten on honest grounds; the
  "shrink the fixture" follow-up repositioned from *if drift appears* to
  **scheduled after Stage 3**, because it is now known to be cheap.
- **F2 (load-bearing).** `RET` must not use bare `find_or_open`.
  `window_panel.rs:373-376` documents why: `find_or_open` "switches the
  ACTIVE window in both branches before firing hooks, so a visit to a
  previously unopened file would replace a focused panel." A `RET` in a
  panel-displayed dired would swallow the panel. Visits now route through
  `pmacs.window.display_file` (new Q#DR10), which also dedups by
  *normalized* path. §2 gains the window-primitive ground truth it was
  missing entirely, and the `dired` command gains the `display` opt that
  acceptance 11 was already testing without.
- **F3 (load-bearing).** Stage 0's completion mechanism does not work as
  described. A function source is re-called per keystroke but invoked as
  `f.call(())` — **zero arguments** (`src/minibuffer.rs:591`) — and the
  callback runs synchronously from Rust dispatch, outside any
  `pmacs.async` coroutine, where `Handle:await()` raises
  (`async.lua:76-79`). There is no synchronous directory listing in Lua,
  so a function source **cannot descend into directories**. Stage 0
  rewritten around the existing Rust `CompletionSource::Files`
  (`minibuffer.rs:589`) plus free-text accept; hierarchical completion is
  a named deferral (new Q#DR11). Rev 1's "a file you have never opened is
  unreachable" is corrected to *undiscoverable and uncompleted* — recentf's
  prompt already passes free text to `find_or_open`.
- **F4 (design gap).** Q#DR5(a) as specced did not cover dired's own hard
  case. `apply_resource_op`'s rebind uses `find_by_path`
  (`buffer_registry.rs:168-174`) — **exact `Path` equality, first match
  only** — so a *directory* rename strands every buffer beneath it, and
  `R` on a directory line is an ordinary dired operation. Worse, that arm
  looks up with the **raw** path (`mod.rs:3248`) while stored paths are
  normalized on write (`EditorCore::set_buffer_path`, `editor_core.rs:819`)
  and the normalizing wrapper `find_buffer_for_path` (`:864-867`) exists and
  is bypassed — so a non-normalized old path silently fails to match today.
  Q#DR5 widened to include rebind *semantics*, with new evidence:
  **`pmacs.fs.rename` has zero production callers**, so changing the
  primitive's contract breaks nobody.
- **F5 (spec fix).** Non-UTF-8 **symlink targets** are fatal too
  (`fs.rs:227`), and differ in kind from names: the entry's own name is
  fine and nothing needs to pass a target back through `rename`. Tolerant
  mode now carries `readlink` failures and target-encoding failures in the
  per-entry channel; only **names** stay fatal.
- **F6 / F7 (Q#DR2 gaps).** Name-keyed dedup needs a canonical path form
  (`/tmp`, `/tmp/`, `/tmp/../tmp` would mint three buffers), and
  found-by-name must verify **dired ownership** before painting into a
  buffer through `bypass_intercept`. Both folded into Q#DR2.
- **Minors.** Arc number dropped; `C-x C-r` attributed to `recentf.lua:85`
  rather than the default keymap; the tolerant opt must **validate**
  unknown keys (`supersede_key`, `fs.lua:73-83`, silently ignores them, so
  a typo'd `tolerant` would degrade to fatal mode unnoticed); acceptance 13
  carries the fixture's macOS ignore gate (`m8_2_acceptance.rs:211-213`);
  and §8 footnotes one per-entry failure that is *already* tolerated —
  a failing `metadata.modified()` yields mtime 0 rather than an error
  (`fs.rs:463-476`).

### Round 2 (rev 2 → rev 3)

- **R2-1.** Q#DR5's "at reply-settle time" named no seam, and the obvious
  one is wrong. The fs ops are fire-and-forget-capable — nothing obliges a
  caller to `await` or attach `on_complete` — so a rebind implemented
  where results are *consumed* (`_take_result`, `mod.rs:6758`) misses any
  rename whose handle is never taken: the rename lands on disk and the
  buffer is never rebound. The trap survives one layer down, and an
  acceptance that awaits would pass while the fire-and-forget path stayed
  broken (the pin-through-the-real-path class again). §7 now names the
  **main-thread completion drain** `AsyncRuntime::tick`
  (`async_runtime.rs:991`) as the seam, unconditional on success — plus a
  fact that makes the implementation non-obvious: **rename settles as an
  undifferentiated `ReplyKind::FsUnit`**, the same reply chmod and remove
  produce (`:1022-1025` maps `Sleep | FsUnit` alike to `JobResult::Unit`;
  there is no `Rename` variant). The drain therefore cannot key on the
  reply — it must key on the pending job's own `JobKind::FsRename`, and
  the job must **retain from/to** so the paths exist at settle. Stage 2's
  acceptance includes a **no-await** rename.
- **R2-2.** The `errors` row shape cannot always carry a name. A per-entry
  `readdir` iterator error (`fs.rs:215-218`) has no filename — the entry
  never materialized, and the error is wrapped with the *parent* path. §8
  makes `name` optional for that arm; the footer counts it without naming
  it.
- **R2-3.** §2's `pmacs.window` inventory listed five exports; there are
  **eight** — also `display_target` (`:425`), `panel` (`:440`), and
  `set_params` (`:544`). The omission was the relevant one:
  `display_target` is "the non-side window a visit from a panel should
  address", i.e. the mechanism behind `display_file`'s panel-safety. §9's
  loosest sentence — directory descent in a panel-displayed dired, left as
  "(or `display`, when dired was itself panel-displayed)" — is now
  specified, with the dedication question answered.
- **R2-4.** The Lua mirror of `normalize_buffer_path` is a second
  implementation of a canonical form — the tab-width-constants class in
  miniature. If the mirror and the Rust normalizer disagree on an edge
  (`//tmp`, `~` with `HOME` unset, root's trailing slash), dired's
  name-dedup and `display_file`'s `find_buffer_for_path` dedup diverge
  **silently**: two buffers, no error. §4 now carries a parity obligation,
  and records that the "binding whose only caller is dired" argument
  undercounts — Q#DR5's fix touches the same normalizer, so a Lua-exposed
  canonicalize has at least two consumers by Stage 2.
- **R2-5.** Header nit: only Lean 4 forks the arc numbering; DAP's Arc 7
  matches the roadmap. Conclusion unchanged, evidence corrected.
- **R2-6.** Stage 0 was "recommended first" with no acceptance and no
  ruling on nonexistent paths. `display_file` routes through
  `resolve_target_buffer` (`editor_core.rs:885-898`), which on
  `ErrorKind::NotFound` **creates** the buffer, binds the path, and sets
  status `"[new file]"` — Emacs parity, now stated rather than inherited.
  §14 gains four Stage 0 acceptance items.

### Stage 0 implementation notes (rev 3 → rev 4)

Implementing Stage 0 falsified one thing the approved text asserted, and
the correction belongs here rather than only in the code.

- **S0-1. Flat completion and free-text accept do not compose the way
  Q#DR11 described.** Rev 3 said Stage 0 is "the flat Rust `Files` source
  rooted at the current buffer's directory, plus free-text accept", as
  though the two were independent and always both available. They are
  not: `recompute_candidates` sets `selected = Some(0)` **whenever the
  candidate list is non-empty** (`minibuffer.rs:372-377`), and
  `resolve_accepted_value` (`:564-574`) returns the **selected candidate**
  in preference to the typed contents. So typed text reaches `on_accept`
  **only when the input filters every candidate away** — which, since
  candidates are bare basenames and the filter is a case-insensitive
  subsequence match, means *when the input contains a `/`*.
  Consequences, all now pinned by `tests/find_file_acceptance.rs`:
  - the deeper-path case works (`sub/inner.txt` matches no basename, so
    it arrives verbatim) — which is what acceptance 0b actually tests;
  - the new-file case works **only for names containing a separator**, so
    acceptance 0c uses one;
  - and there is a genuine hole: typing a **new bare name that is a
    subsequence of an existing entry** opens the existing file instead of
    creating the new one. `find_file_selected_candidate_shadows_typed_text`
    pins that as a decision rather than an accident.

  Closing the hole needs a Rust change to accept semantics — prefer typed
  text over the selection when the two differ and the user has not
  explicitly moved the selection — which would change `M-x` and
  `switch-buffer` too, and so is deliberately **not** made in Stage 0. It
  joins the hierarchical-completion deferral in §13.
- **S0-4. Accepting on empty input opens the first-sorted candidate.**
  A consequence of S0-1 with an empty needle: `fuzzy_score` returns
  `Some(0)` for every entry (`minibuffer.rs:637-640`) and
  `filter_and_sort` breaks the resulting tie lexicographically (`:678`),
  so an immediate RET opens whatever sorts first — dotfiles lead, and a
  directory can lead, in which case the open fails and reports (S0-6).
  `M-x` and `switch-buffer` share the mechanism, so this is inherited
  rather than introduced; it is documented at the command and listed in
  §13 beside the accept-semantics fix that would close it.
- **S0-5. Minibuffer history stores the pre-join value.** `accept`
  pushes the resolved value into the history bucket **before** `on_accept`
  joins it onto the root, so a `C-p` recall of a root-relative entry
  under a different root resolves somewhere else. Rust-side, so Stage 0
  cannot fix it; §13.
- **S0-6. The failure arm is a real path and is pinned.** Accepting a
  *directory* candidate reaches `display_file`, whose load fails
  (`File::open` on a directory succeeds; the read returns EISDIR), so the
  command's `pcall` turns it into a status message instead of letting the
  error escape mid-dispatch.
  `find_file_accepting_a_directory_reports_instead_of_raising` pins it
  through the real accept path and fails when the `pcall` is removed.
- **S0-2. The prompt field must start empty.** Emacs prefills find-file's
  field with the directory. Here any prefill contains a `/`, which by
  S0-1 filters every candidate away and silently disables completion —
  so the root is named in the *prompt string* instead, and the empty
  field is pinned by acceptance 0d.
- **S0-3. A leading `~` must be expanded before the path reaches the
  core.** `get_or_load_buffer` (`editor_core.rs:842-856`) computes a
  normalized path but calls `load_file` with the **raw** one (`:847`), so
  a `~/…` path deduplicates against an already-open buffer (dedup goes
  through the normalizing `find_buffer_for_path`) yet fails to load a
  file that is not open yet. Stage 0 expands the tilde in Lua, which
  makes both halves agree without changing core load semantics for the
  CLI, LSP, and bootstrap callers. **Using the normalized path for the
  load is the better fix and is now a named deferral** (§13) — it is the
  same normalize-before-lookup family as Q#DR5's `apply_resource_op`
  correction.

### Stage 1 implementation notes (rev 5 → rev 6)

Implementing Stage 1 (PR #165) falsified four things the approved text
asserted and settled five it left open. Recorded here rather than only
in the code, per the rev-4 precedent.

- **S1-1. The normalizer is EXPOSED, not mirrored — so B2 is partly
  false, in the direction Q#DR2 preferred.** Q#DR2 made the mirror
  conditional (`Stage 1 may still mirror if exposure turns out to drag
  in EditorCore borrow plumbing it does not otherwise need`).
  `normalize_buffer_path` is a **free function** (`editor_core.rs`), so
  exposure drags in nothing: it is now `pub` and reachable as
  `pmacs.path.canonicalize`. Consequences, all deliberate: B2 ("tolerant
  `read_dir` is the only Rust change Stage 1 needs") is false by one
  small binding; acceptance 3b degenerates to the round-trip form the
  framing described; and the Stage 2 mirror-removal follow-up **is not
  owed** — there is no second canonical form to remove. The parity
  acceptance is still carried, now as "the Lua binding and the Rust
  function agree over one shared edge list", which is exactly the claim
  a future re-mirroring would break.
- **S1-2. R2-3's dedication claim is falsified by the substrate.** It
  read "a dedicated dired panel stays dedicated across descent and the
  new dired buffer inherits it". `display_buffer` never replaces the
  buffer in a slot dedicated to another one: it discards every
  side-specific parameter and falls back to the document window (Q#BP3
  2.iii), and the exact-window arm errors outright. Dired therefore does
  **not** try to unpin the user's panel — which is also what Emacs's
  `display-buffer` does with a dedicated window. Acceptance 3c is split:
  a non-dedicated panel keeps the descent, and a dedicated one keeps its
  buffer *and* its pin while the new directory appears in the document
  window.
- **S1-3. Acceptance 3c cannot pin the descent ROUTING, and the test now
  says so.** Dired holds the focus in its own panel, so a raw
  `switch_buffer` lands in that same window and every 3c assertion holds
  either way — the mutation is *vacuous* against it. Dedication is the
  only thing that distinguishes `display { side = … }` from the raw
  switch, so the dedicated-panel test is the discriminating pin. Found
  by running the bite rather than by reading the test; the vacuity is
  documented at the assertion instead of being left to be believed.
- **S1-4. Dired is the first builtin to bind a mode-scoped key, and one
  pre-existing lib test assumed none existed.**
  `describe_key_identifies_every_default_binding` iterated *every*
  binding in the stack and asserted `pmacs.describe.key` resolves it
  context-free, which held only while the modes table was empty. It now
  sets the effective context per binding — and explicitly *clears* the
  mode for a global one, because a mode left over from a previous
  iteration legitimately shadows a global chord of the same name
  (dired's `RET` shadows `edit.newline-and-indent`, which is the point
  of the mode).
- **S1-5. `C-x d` deliberately takes NO completion source.** It is the
  direct consequence of S0-1/S0-4: with a `files` source, RET on an
  empty field opens whatever sorts first (the minibuffer selects
  candidate 0 whenever the list is non-empty, and a selected candidate
  shadows typed text), and RET-on-the-directory-you-are-in is exactly
  the gesture `C-x d` exists for. The field is **prefilled** with the
  current directory instead — Emacs's own shape here — and free text
  always reaches `on_accept` because `CompletionSource::None` bypasses
  candidate resolution entirely. Directory-name completion is what dired
  itself replaces.
- **S1-6. Ownership is the handle table ALONE**, narrower than Q#DR2's
  "present in dired's handle table, or `major_mode(buf) == "dired"`". A
  foreign buffer that carries the mode *is* the case the check exists to
  refuse, and a builtin's handle table cannot be lost the way a
  reloadable package's can. Acceptance 4 sets the mode on the foreign
  buffer to pin the stronger reading.
- **S1-7. The mark column ships in Stage 1, rendered blank.** Q#DR4 is a
  Stage 2 decision, but reserving the two columns now means Stage 2 does
  not move every offset and Stage 3's column-classifying intercept can
  be written against constants that did not shift under it. The
  constants are computed from the widths (the fixture hardcoded
  `NAME_START = 39` and paid for it in every wdired test) and exported
  as `pmacs.dired._layout` so acceptance cannot drift from them.
- **S1-8. A symlinked directory needs a probe, because kinds are
  lstat-based.** Both `read_dir` and `stat` report a link as
  `"symlink"`, so nothing in the entry says whether it points at a
  directory. `RET` on a symlink therefore *tries* to list the target
  (one extra syscall, on symlink lines only) and descends if that
  succeeds, else visits it as a file. Q#DR10 specified only the
  dir/file arms; this is the third.
- **S1-9. Interactive origin does not survive the await.** Every listing
  is worker-dispatched, so the work after the first `:await()` resumes
  inside `tick_async`, where `InteractiveCommandOrigin` is empty and
  `pmacs.window.*` falls back to the **ambient** active frontend. Single
  frontend: correct. Multi-frontend: a dired opened from peer B while A
  is ambient would display for A. Not fixable from Lua (the display
  surface takes no frontend argument) and named here rather than
  discovered later.

### Stage 1 review round 1 (rev 6 → rev 7)

Three findings changed behavior; the rest were naming and comments. Each
fix is bite-verified against the test that names it.

- **S1-10. An ambient re-seat is not safe after an await.** `dired.revert`
  painted its own buffer by name (safe) and then re-seated through
  `pmacs.editor.move_to_line`, which moves whatever window is
  **active** — so a user who switched buffers while the re-read was in
  flight had an unrelated buffer's cursor moved to a line index
  meaningful only in the dired listing. This is the buffer-level instance
  of the hazard S1-9 named at the frontend level, and it generalizes: in
  this codebase, *painting takes a buffer and seating takes the world*.
  Any post-await cursor operation needs an active-buffer guard;
  `open_directory` is exempt only because it displays the buffer first.
- **S1-11. The rendered columns are a contract, so precision yields to
  width.** `%10d` overflowed at 10 GB (VM images, core dumps), widening
  the size field and shifting mtime and name right on that line alone.
  Cosmetically harmless today, but `_layout` is exported and Stage 3's
  column-classifying intercept is planned against it, so a
  contract-violating line now is a Stage 3 trap. `fmt_size` took
  `fmt_mtime`'s shape: exact bytes while they fit, else a fixed-width
  magnitude. Not the deferred human-readable column (§13) — the exact
  count still renders right up to the point where it cannot.
- **S1-12. `open_directory`'s "changed nothing on failure" invariant is
  reusable as a PROBE.** S1-8's symlink descent originally listed the
  target to learn its kind and then opened it — two full listings of the
  same directory. Because a failed open touches no editor state
  (acceptance 15), the open itself is the probe: try the descent, fall
  back to `display_file`. One read. The comment that claimed "one
  syscall" for a full `read_dir` is corrected rather than left as a
  cost claim nobody would re-check.

Also, on the tolerant channel (Q#DR6): a `readdir` iterator may keep
yielding errors without terminating, and **cancellation is not a backstop
for a dired listing** — it carries no supersede key, so nothing cancels
it. A consecutive-error cap now fails the listing the way an unopenable
directory fails, rather than accumulating error rows on a worker thread.
It is deliberately untested: faking a failing iterator would need the
walk generic over it, a refactor with no other consumer.

## 0.5. Coherence impact (`COHERENCE.md` §20)

Required of every framing since #163. This arc was scouted and approved
before that rule existed; the statement is added here rather than
backfilled silently.

**Section served: §20 Priority 1 — protect the golden product journey**,
which already names this work: *"a find-file surface (in flight, PR
#162)"* and *"directory-argument handling"*. Secondary: §5 (unify
discoverability) and §14 (coherent workbench primitives).

**Journey steps touched (§2).**

- **Step 7, "find a symbol or file"** — the file half, which had no
  surface at all. Stage 0 (`C-x C-f`, merged as #162) covers opening a
  known path; Stages 1–3 cover browsing, which is the half a user
  reaches for when they do *not* already know the path.
- **Step 3, "open a real project"** — partially, and the boundary
  matters. §2's ground truth grades the journey *broken at step 3*
  because `pmacs .` exits 1: `load_file` (`src/file_io.rs:81-87`) does
  `File::open` (which succeeds on a directory) then `read_to_end` →
  EISDIR, which is not `NotFound`, so `resolve_target_buffer`'s
  create-a-`[new file]` arm never fires and the error escapes.
  **That is the same mechanism Stage 0 pinned** in
  `find_file_accepting_a_directory_reports_instead_of_raising` — where
  the `pcall` turns it into a status message instead. Dired Stage 1 is
  what makes a directory *open into something* rather than merely fail
  politely.
  **Boundary with the adjacent arc:** §20's arc-cut list puts CLI
  directory-argument handling in "Journey Stage 1", noted as riding
  alongside this arc. The two meet at `resolve_target_buffer`. This
  framing does **not** claim the CLI path; it supplies the buffer a
  directory should resolve *to*, and Journey Stage 1 should route
  `pmacs .` into it rather than inventing a second directory surface.
- **Step 4, "understand the visible interface"** — marginally, via the
  `dired` major mode showing in the statusline (Q#DR8).
- Steps 1–2, 5–6, 8–12: untouched.

**Interaction islands added (§6): none — deliberately.** §6 grades this
area "weak, and growing by one island per modal feature", with every
modal surface funnelling through `EditorInstance::dispatch_key`'s
precedence machine. Dired adds no Rust-level interception: its keys are
an ordinary **mode-scoped keymap** through the existing `pmacs.keymap`
registry (Q#DR8), so they are introspectable by `describe.key` and
rebindable like any other binding. Stage 3's wdired is a **major-mode
swap**, not a modal layer — which is the reason Q#DR3 chose a mode swap
over an edit-mode flag. Two existing islands are *consumed* (the
minibuffer prompt for `C-x d`, and Stage 0's), neither added by this
arc. This arc therefore moves §6's count sideways, not up.

**Config registry adoption (§11): yes.** `dired.kill-when-opening` is
defined through `pmacs.config` with a type, default, and
`mutability = "live"` (Q#DR2), not a bare Lua global — matching the
#127 adopters. Sort mode is deliberately *not* a setting in Stage 1: it
is per-buffer session state, and promoting it would need the
buffer-local scope plus a persistence story the registry does not have
yet (its own named deferral).

**Background-work attribution (§9): inherited debt, not fixed here.**
Every listing runs as a `pmacs.fs.read_dir` worker job, and those jobs
carry no owner or purpose — §9's gap. A dired refresh will therefore
show up in the activity planes exactly as anonymously as every other fs
job does today. Stage 1 does not fix that and does not make it worse;
when §20's "worker identity" arc lands, dired's jobs are ordinary
consumers of it. Naming it here so the debt is visible rather than
silently compounded (§1.3).

**Net.** One journey step goes from *no surface* to *a surface*; one
more moves from *fails* toward *resolves*; no island added; one setting
enters the registry; one attribution gap inherited and named.

## 1. Problem and what ships

Two separate facts collide here, and the second is why this is worth more
than "a file browser would be nice".

**Fact one: a complete dired already exists, and ships to nobody.**
`tests/fixtures/pmacs-dired/init.lua` is 1,384 lines of Lua implementing
the read-only directory view (T M8.2) and the wdired editable
rename/chmod layer (T M8.3), pinned by **47 acceptance tests** (15 in
`tests/m8_2_acceptance.rs`, 32 in `tests/m8_3_acceptance.rs`). It was
built as one of M8's three "universality proof" packages — the evidence
that a buffer can be a projection of external state. It lives under
`tests/fixtures/`, and `rg pmacs-dired` outside `tests/` returns zero
hits. Nothing installs it; no user can reach it.

**Fact two: pmacs has no discoverable way to open a file by path.** There
is no `find-file` command and no `C-x C-f` binding. The complete list of
builtin command names contains nothing matching `file` or `open`;
`pmacs.buffer.find_or_open` (`src/lua_bindings/mod.rs:3104`) is a Lua API
with no interactive caller of its own. A file enters a session via the
CLI (`pmacs FILE`, `pmacs --gpu FILE`), an LSP jump, a project-search
visit, or `C-x C-r` recent-files — whose prompt *does* pass free text
through to `find_or_open` (`recentf.lua:74-80`), so an arbitrary path is
technically reachable, but only by typing it blind into a prompt labelled
"Recent file:" with no completion and no discoverability.
`editor.switch-buffer` (`builtin/commands/default.lua:594`) completes over
*already-open buffers* and reports `no buffer: <name>` for anything else.

So dired is not a convenience rider on an existing file surface. **Dired
is the file surface.** That reframes both its value and its risk: it is
the first thing a new user needs, and the last place we can afford a
listing that refuses to render.

**What ships**, staged (§10):

- **Stage 1 — the dired view.** A builtin `builtin/runtime/dired.lua`:
  read-only listing, navigation, `RET` to visit (files open through
  `display_file`, directories descend), sort modes, revert, quit,
  `C-x d` / `C-x C-j`, a `dired` major mode with mode-scoped keys, cursor
  preservation across refresh — plus the one Rust change Stage 1 needs, a
  **per-entry-tolerant `read_dir`** (Q#DR6).
- **Stage 2 — marks and operations.** `m`/`u`/`U`/`t`, deletion flags
  `d`/`x`, immediate `D`, `R` rename, `C` copy, `+` mkdir — and the three
  filesystem primitives that do not exist yet, plus the rename/rebind fix.
- **Stage 3 — wdired.** The editable layer, carrying over the fixture's
  hard-won commit logic.

`find-file` itself (`C-x C-f`) is separable and is Stage 0 (§10).

## 2. Ground truth (scouted 2026-07-25, `main` @ `e745068`; verified across review rounds 1 and 2; re-verified against `main` @ `0827dd1`)

**Base note.** `main` moved from `e745068` to `0827dd1` (Lean 4 Stage 1,
#160) between the scout and approval. The diff touches exactly one file
this framing cites — `builtin/runtime/syntax.lua`, which gained a
`lean = "lean4"` modeline alias — and nothing else in the ground truth
below. The only consequence is a line drift: `set_major_mode` is now
`syntax.lua:497`, not `:492`. Every other citation is unchanged.

### The existing fixture

- `tests/fixtures/pmacs-dired/init.lua` (1,384 lines) defines eight
  commands: `open-line`, `parent`, `sort-name`, `sort-mtime`,
  `sort-size`, `wdired-edit`, `wdired-abandon`, `wdired-commit`. It binds
  `RET` and `Backspace` **buffer-locally** at open (`:381-388`), paints by
  wholesale `buf:replace` behind a `painting` passthrough flag
  (`:294-302`), and keys per-buffer handles by linear scan over
  `BufferIdLua.__eq` with a liveness compaction (`:53-67`).
- Its wdired layer is the valuable part and is not naive: a
  column-classifying `intercept_edit` (`:640-712`), fixed-width perms with
  positional validation (`:520-542`), `\\`/`\n`/`\r`/`\t`/`\xNN` filename
  escaping with an **exact inverse** so a no-op commit cannot fire a
  spurious rename (`:140-211`), field-by-field external-change detection
  including `mtime_nsec` (`:854-902`), duplicate-final-name rejection
  before any syscall, and a **two-phase rename through unique temp names**
  so swaps and chains commit safely (`:1176-1216`).
- **What the 47 tests actually assert (F1).** 45 assert dired/wdired
  *behavior*: rendered listing shape, sort order, escaping round-trips,
  intercept column rejection, on-disk chmod/rename effects, the two-phase
  swap, external-change detection, partial-application reporting. One
  (`m8_2:1190`) exercises package mechanics — reload safety after
  `set_init_complete`. One (`m8_2:1245`) is a source-line-count lint.
  `install_local` appears only in the shared `editor_with_dired()` harness
  as a setup precondition and in doc comments; `on_unload`,
  `DuplicateName`, and per-package `require` scoping are asserted nowhere.
- **Two of its own stated limitations are now false.**
  - `open-line` on a non-directory errors with "requires the
    buffer-from-file API (not yet exposed)" (`:948-961`).
    `pmacs.buffer.from_file` (`mod.rs:3054`) and `find_or_open` (`:3104`)
    both exist and ship.
  - The test seam claims "the v0.1 buffer surface doesn't expose
    move_to_byte yet, so tests can't reliably position the cursor"
    (`:1355`). `pmacs.editor.goto_byte` (`mod.rs:12765`) and
    `move_to_line` (`:12526`) landed with editops (#111).
- **A real defect in its model:** navigation mutates `handle.path` and
  repaints, but the buffer was named `*dired:<path>*` at creation and
  **there is no `pmacs.buffer.set_name`** — the `pmacs.buffer` table
  exports exactly `create`, `from_bytes`, `from_file`, `find_or_open`,
  `list`, `kill`, `remove`, `on_removed`, `major_mode`, `set_major_mode`,
  `set_round_trip_input`, `mark_create`, `add_intercept`,
  `remove_intercept`, `apply_resource_op`, and the style-overlay family.
  (`Buffer::set_name` exists Rust-side, unexposed.) So after one `RET` the
  buffer name names a directory it is no longer showing. Q#DR2 answers this.

### The filesystem surface

- `pmacs.fs` is exactly five worker-dispatched ops — `read_dir`, `stat`,
  `rename`, `chmod`, `remove` (the complete `_dispatch_fs_*` set) — plus a
  Lua-side polling `fs.watch` (`builtin/runtime/fs.lua:226`). **No
  `mkdir`, no `copy`, no symlink-create, no recursive remove.**
- **`read_dir` is all-or-nothing, and this is the load-bearing gap.**
  `read_dir_blocking` (`src/fs.rs:201`) returns
  `Result<Vec<FsDirEntry>, FsError>`. **Five** per-entry conditions fail the
  **entire listing**: a per-entry `readdir` error, a failed
  `symlink_metadata`, a failed `read_link` (`:228-234`), a non-UTF-8
  symlink target (`:227`), and a non-UTF-8 name (`:238`). The module doc
  acknowledges the shape and says "dired-class will likely want a
  per-entry-tolerant wrapper but that's the package's job, not the
  primitive's" (`:196-200`) — **that wrapper cannot be written in Lua.**
  The primitive hands Lua one structured error and no partial vec; there is
  nothing to be tolerant *with*. Three concrete failure modes, all
  ordinary:
  1. a directory readable but not searchable (`r` without `x`) — `readdir`
     succeeds, every child `lstat` fails;
  2. a file unlinked between `readdir` and `lstat` — ENOENT, i.e. a plain
     refresh of a busy directory (`/tmp`, a build tree) can just fail;
  3. any single non-UTF-8 filename or symlink target in the directory.
  One per-entry failure is *already* tolerated: a failing
  `metadata.modified()` yields mtime 0 rather than an error (`:463-476`).
- `read_dir` already takes `(path, opts)` and the opts parser
  (`supersede_key`, `fs.lua:73-83`) reads only `opts.supersede` and
  **silently ignores unknown keys** — signature-natural for Q#DR6's opt,
  but a typo'd `tolerant` would degrade to fatal mode unnoticed.
- `chmod` **follows symlinks** (`src/fs.rs:370`; `fs.lua:104`) while
  `read_dir`/`stat` use `lstat`. The fixture rejects symlink perms edits
  at intercept time for exactly this reason (`init.lua:629-638`) — that
  decision carries over unchanged.
- **`pmacs.fs.rename` has zero production callers** — only its own
  definition (`fs.lua:126`), `m8_1`/`m8_3` acceptance, and the fixture.
- `pmacs.fs.rename` does **not** rebind an open buffer's path.
  `pmacs.buffer.apply_resource_op` (`mod.rs:3206`) — the LSP
  workspace-edit applier — does, but by **exact first match**: its
  `"rename"` arm calls `reg.borrow().find_by_path(&from)` (`:3248`), which
  is exact `Path` equality over insertion order
  (`buffer_registry.rs:168-174`). It also uses the **raw** path while
  stored paths are normalized on write
  (`EditorCore::set_buffer_path`, `editor_core.rs:819`) and the normalizing
  lookup `find_buffer_for_path` (`:864-867`) exists and is bypassed. So the
  model rev 1 proposed copying is itself subtly wrong, and has no prefix
  rebind anywhere. `apply_resource_op` is also **synchronous and blocking
  on the main thread**, unlike every `pmacs.fs` op. Q#DR5.

### Windows, panels, and how anything gets displayed

- `pmacs.window` exports **eight** functions: `display`
  (`window_panel.rs:356`), `display_file` (`:379`), `display_target`
  (`:425`), `panel` (`:440`), `quit` (`:453`), `params` (`:499`),
  `set_params` (`:544`), and `resize` (`:591`) — alongside the pre-arc
  `switch_buffer`. **`display_target` is "the non-side window a visit from
  a panel should address"**, i.e. the mechanism that makes `display_file`
  panel-safe; it is what Q#DR10 rests on, and it is the reason a file
  visit and a directory descent take different routes (§9).
- **`find_or_open` is panel-hostile, by documented design.**
  `window_panel.rs:373-376`: `find_or_open` "switches the ACTIVE window in
  both branches before firing hooks, so a visit to a previously unopened
  file would replace a focused panel before any display policy could
  help." `display_file` is the Q#BP11b answer — a side-effect-free dedup
  via the **normalizing** `find_buffer_for_path` *before* any I/O, then
  destination resolution before the read, so a dedicated origin cannot
  force load-before-failure. LSP visits and compile already route through
  it.
- `pmacs.listview` (`builtin/runtime/listview.lua`) implements the
  disciplines a read-only panel needs: a read-only `add_intercept`
  (`:101-104`), `set_round_trip_input` (`:106`), a buffer-local keymap, a
  line→item map, `q`-restores-previous, and the `display = "current" |
  "panel"` opt-in (`:132-142`). **But** its keymap is a fixed set —
  `RET`/`SPC`/`n`/`p`/`g`/`q` (`:76-88`) — with no extension point, and
  its read-only intercept is installed once at panel creation and **never
  removed** (`:101`), which wdired requires. Three panels already depend
  on this module (references, outline, project-search).
- `set_round_trip_input` (`mod.rs:3085`) is what makes single-key bindings
  work on a semantic/GPU frontend: while a marked buffer is active
  `dispatch_idle` reports false, so optimistic-apply stays off and `d`
  reaches the binding instead of landing as a CRDT insert. The same gate
  (`dispatch_idle_for`, `editor.rs:818`) *also* disables optimistic apply
  for any focused side window (`!window.is_side()`, Q#BP14a), so a
  panel-displayed dired is covered twice.
- **There is no real `read_only` buffer flag** — a standing backlog item
  (`docs/side-quest-backlog.md`, cross-cutting substrate). The intercept
  idiom is what every generated buffer uses today.

### Minibuffer completion

- A `source` **function** is re-called on every keystroke
  (`recompute_candidates`, `minibuffer.rs:361`) but invoked as
  `f.call(())` — **zero arguments** (`:591`). `pmacs.minibuffer.contents()`
  exists, but the callback runs synchronously from Rust dispatch, outside
  any `pmacs.async` coroutine, and `Handle:await()` explicitly raises
  there (`async.lua:76-79`). There is **no synchronous directory listing in
  Lua**, so a function source cannot descend into directories.
- `CompletionSource::Files { root }` exists Rust-side
  (`minibuffer.rs:589` → `list_directory(root)`), reached from Lua as
  `source = "files"` with `source_root` (`mod.rs:13286`). It is flat,
  single-directory, capped at 1024 candidates, and **currently used by
  nothing outside a unit test**.
- Free text accepts: `resolve_accepted_value` (`minibuffer.rs:564`)
  returns the raw typed string when no candidate is selected.

### Buffers, modes, keys

- Mode-scoped keymaps exist since #129: `pmacs.keymap.bind { scope =
  "mode", mode = "<name>", … }` (`mod.rs:13432`), resolving buffer-local →
  mode → global. `syntax.lua:497` is the **only** `set_major_mode` caller
  in `builtin/`, firing on `buffer.after-load`; a `buffer.create`d dired
  buffer has no path and fires no `after-load`, so nothing contends.
- `find_or_open` on a directory reaches `file_io::load_file` → EISDIR.
  Dired must dispatch on `entry.kind` itself.
- **Keybinding space.** `C-x d`, `C-x C-j`, `C-x C-f`, and `C-x C-q` are
  unbound **repo-wide**. `C-x C-r` is bound to recent-files in
  `recentf.lua:85` (not the default keymap). `C-c <letter>` is fully taken
  by LSP (`lsp.lua:2248-2256`); `C-c @` is the folding prefix
  (`fold.lua:48-52`); `C-c C-k` is bound **buffer-locally** by compile
  (`compile.lua:231`) and async, not globally.

## 3. Where dired lives (Q#DR1)

**A builtin runtime module, `builtin/runtime/dired.lua`, written fresh.
The M8 fixture stays exactly where it is, frozen.**

Builtin rather than a package, because a surface that is the primary way
to open a file cannot be gated on the user installing something, and
because builtin modules get the load-order and config-registry guarantees
a package does not (the `pair.lua`-before-`lsp.lua` precedent).

**Why the fixture is not promoted.** Rev 1 argued the 47 tests pin the
package system and would be voided by re-pointing. That was false (F1) —
45 of them assert dired behavior, and behavior transfers. The honest
argument is narrower and rests on three things:

1. **The harness route is itself the proof.** 46 of 47 tests reach dired
   through `install_local` + `require`, and that *routing* — a third-party
   package, in its own environment, driving buffers, intercepts, marks,
   commands, and keymaps — is the M8 universality claim. A builtin loaded
   by the runtime demonstrates nothing about packages. Re-pointing the
   suites keeps every behavioral assertion and silently deletes the claim
   they were written to support.
2. **The builtin diverges structurally**, so the tests cannot transfer
   verbatim anyway: the mark column (Q#DR4) shifts every column offset the
   wdired tests hardcode through `_test.NAME_START`; mode-scoped keys
   (Q#DR8) replace the buffer-local `RET`/`Backspace` binds
   `dired_ret_and_backspace_keybindings_navigate` drives;
   buffer-per-directory (Q#DR2) replaces the in-place repaint that
   `dired_parent_command_navigates_up_one_level` asserts; and the whole
   `M._test` seam is package-shaped.
3. **The behavior gets re-pinned regardless**, by the builtin's own
   acceptance (§14), which is where those 45 assertions are owed a home.

The cost is real: roughly 900 lines of rendering, escaping, and commit
logic will exist in two places. The mitigation is that the fixture is
**frozen** — a proof artifact, not a maintained feature, already fully
pinned.

**Named follow-up, scheduled rather than conditional (F1 corollary).**
Because the fixture's package-system value concentrates in exactly one
test plus the harness routing, shrinking it to the minimum payload that
still proves universality is cheap — far cheaper than rev 1 implied. It
is scheduled **after Stage 3**, when the builtin owns every behavior the
45 tests currently cover, not left to "if drift shows up".

The fixture remains a *reference* for the parts that were hard —
escaping with an exact inverse, two-phase rename, external-change
detection, the symlink rules — and those carry over as decided design,
not as re-litigated questions.

## 4. Buffer model and navigation (Q#DR2)

**One buffer per directory, found-or-created by canonical name;
navigation opens the target's buffer rather than mutating the current
one.**

This is Emacs's actual behavior (`dired-find-file` on a directory yields a
dired buffer for that directory), and it answers the fixture's stale
buffer-name defect without adding a `pmacs.buffer.set_name` binding:
nothing is ever renamed, because a buffer's name always describes the
directory it was created for. It also makes `C-x d` on an
already-visited directory and `C-x C-j` dedup for free.

**Canonicalization (F6).** `/tmp`, `/tmp/`, and `/tmp/../tmp` are all
absolute and would otherwise mint three buffers. The rule is **lexical
normalization before naming and before lookup**: expand a leading `~`,
absolutize, collapse `.` and `..`, strip a trailing slash except at root.
Symlinks are **deliberately not resolved** — Emacs parity, and resolving
them would make `..` from a symlinked directory jump somewhere the user
did not navigate from. The core already has exactly this shape in
`normalize_buffer_path` (`editor_core.rs:4790`); it is not Lua-exposed,
so Stage 1 either mirrors it in Lua or exposes it.

**A mirror is a second implementation of a canonical form, and needs a
parity pin (R2-4).** This is the tab-width-constants class in miniature:
if the Lua mirror and the Rust normalizer disagree on an edge — `//tmp`
(POSIX gives a leading double slash implementation-defined meaning), `~`
with `HOME` unset, root's trailing slash, a `..` that would escape root —
then dired's name-dedup and `display_file`'s `find_buffer_for_path` dedup
**diverge silently**: two buffers for one directory, no error anywhere.
So whichever route Stage 1 takes, it carries a **parity acceptance** that
drives both implementations over one shared edge-case list and asserts
identical output (acceptance 3b).

Rev 2's "avoids a binding whose only caller is dired" also **undercounts**:
Q#DR5's rename fix touches the same normalizer, so a Lua-exposed
`pmacs.path.canonicalize` (or equivalent) has at least two consumers by
Stage 2. Exposing it and deleting the mirror is therefore the better
end state; Stage 1 may still mirror if exposure turns out to drag in
`EditorCore` borrow plumbing it does not otherwise need, but the parity
acceptance is required either way, and the mirror is then a **named
Stage 2 removal**, not a permanent duplicate.

**Ownership check (F7).** `pmacs.buffer.create` takes any caller-chosen
name, so a foreign buffer named `*dired:/tmp*` would be "found" by name
and then painted into through `bypass_intercept`. Found-by-name must
confirm the buffer is dired-owned — present in dired's handle table, or
`pmacs.buffer.major_mode(buf) == "dired"` — and otherwise create a fresh
buffer under a disambiguated name rather than clobbering it.

Buffer name: `*dired:<canonical absolute path>*`.

The cost is buffer accumulation when walking a deep tree. Emacs users
live with this; Emacs 28 added an opt-out, and we mirror it as a config
key rather than a hardcoded policy:

- `dired.kill-when-opening` (boolean, default `false`, live) — when true,
  descending or ascending kills the dired buffer being left.

## 5. Read-only discipline and the wdired seam (Q#DR3)

The dired buffer is read-only by the listview idiom — an `add_intercept`
that rejects every non-bypass edit, plus `set_round_trip_input(buf,
true)` so a GPU session's optimistic-apply cannot swallow single-key
bindings — and dired's own paints use `bypass_intercept = true`.

**Dired owns its buffer directly; it is not built on `pmacs.listview`.**
Listview would have to grow a keymap extension point and a removable
read-only mode, and three shipped panels depend on it. Bending a module
into a shape its existing callers do not need is exactly the change class
that took CI red in #155: scoping `pmacs.window.buffer()`'s no-arg arm
"for consistency" made a total function partial and silently dropped
edits from six unpcall'd runtime callers. Dired reuses listview's
*disciplines*, not its code. Factoring the shared disciplines into a
common helper is a named follow-up, to be done once dired's real shape is
known rather than predicted.

**Wdired (Stage 3) is a mode swap, not a flag.** `C-x C-q` removes the
read-only intercept, installs the column-classifying one, and calls
`set_major_mode(buf, "wdired")`; commit and abandon restore both. Because
keys are mode-scoped (Q#DR8), the entire keymap changes with the mode —
`m` means "mark" in dired and means "type an m" in wdired, with no
per-key bookkeeping.

**Wdired refuses to open on a partially-listed directory.** If the
listing carried any per-entry error (Q#DR6), rename and chmod are refused
with that reason: a rename batch is planned against a snapshot, and you
cannot safely plan against a directory you could not fully see.

## 6. Marks (Q#DR4)

Emacs's mark column is column 0, so every other column shifts right by
two (`"  "` or `"* "`). The builtin computes its offsets from the
constants rather than inheriting the fixture's `PERMS_START = 1` /
`NAME_START = 39`.

**Marks are keyed by basename, never by line index.** A sort, a revert,
or an external change reorders lines; a line-indexed mark set would
silently retarget onto a different file — the same class of defect as
keying kill-ring state by index rather than by stable id (the Arc 2
substrate rule). `*buffer-list*` keys its deletion marks by buffer id
(`builtin/commands/default.lua:507`) for the same reason.

Two mark characters, following Emacs: `*` (general mark, consumed by
operations) and `D` (deletion flag, consumed by `x`). A basename that
disappears between marking and executing is dropped from the batch and
reported, not silently skipped.

## 7. Operations and the missing primitives (Q#DR5)

Stage 2's operations need three filesystem primitives that do not exist,
and one correctness fix.

**New `pmacs.fs` ops** (worker-dispatched, matching the existing five;
mutating ops take no `supersede`, per `fs.lua:101-124`):

- `pmacs.fs.mkdir(path, opts)` — `opts.parents` for `create_dir_all`.
- `pmacs.fs.copy(from, to, opts)` — regular files in v1; a directory
  source is **refused** rather than silently shallow-copied. Preserves
  mode bits; `opts.overwrite` defaults false and the op refuses an
  existing target otherwise.
- `pmacs.fs.remove_dir_all(path)` — separate from `remove` rather than a
  flag on it, so a recursive delete can never be reached by a caller that
  meant the single-object op.

**The rename rule — decided, with the semantics widened (F4).** A rename
must rebind open buffers, and it must do so at the **primitive**:
`pmacs.fs.rename` has **zero production callers**, so changing its
contract breaks nobody, and leaving the trap armed guarantees the next
caller rediscovers it the expensive way. The rebind is:

- **prefix-aware**, not exact-match. `apply_resource_op`'s
  `find_by_path` (`buffer_registry.rs:168-174`) is exact `Path` equality,
  first match only — which strands every buffer beneath a renamed
  *directory*, and `R` on a directory line is an ordinary dired
  operation. The reconcile rebinds the renamed path itself **and** every
  buffer whose path has it as a path-component prefix.
- **normalize-before-lookup.** Stored paths are normalized on write
  (`editor_core.rs:819`) and the normalizing wrapper
  `find_buffer_for_path` (`:864-867`) already exists;
  `apply_resource_op` bypasses it with a raw lookup (`mod.rs:3248`), which
  is a latent miss today. The new path goes through the wrapper.
  `apply_resource_op`'s own raw lookup is fixed in the same change — it is
  the same bug, one call site away.
- in the **main-thread completion drain**, `AsyncRuntime::tick`
  (`async_runtime.rs:991`), unconditionally on success — **not** where
  results are consumed. This is the load-bearing half of the decision
  (R2-1). The fs ops are fire-and-forget-capable: nothing obliges a caller
  to `await` or attach `on_complete`, so a rebind hung off `_take_result`
  (`mod.rs:6758`) would miss every rename whose handle is never taken —
  the rename lands on disk, the buffer is never rebound, and the trap
  survives one layer below where we thought we fixed it. An acceptance
  that awaits the rename would pass throughout, so **Stage 2's acceptance
  includes a no-await rename** and bites against the drain.

  Two facts make this non-obvious to implement. **Rename settles as an
  undifferentiated `ReplyKind::FsUnit`** — the same reply `chmod` and
  `remove` produce; there is no `Rename` variant, and the drain arm maps
  `Sleep | FsUnit` alike to `JobResult::Unit` (`async_runtime.rs:1022-1025`).
  So the drain cannot key on the reply; it must key on the **pending job's
  own `JobKind::FsRename`**. And the from/to paths live only in the
  dispatch call today, so the pending job must **retain them** for the
  drain to have anything to rebind with. Both are additive to
  `async_runtime.rs`; neither changes the wire or the worker contract.

**v1 supports `R` on a directory** — that is precisely what prefix-aware
rebinding buys, and refusing it while `C` refuses directory sources for a
different reason (no recursive copy primitive) would be an arbitrary
asymmetry. Stage 2's acceptance pins the directory case explicitly.

**Confirmation.** Destructive operations (`x`, `D`, recursive delete,
overwriting copy) prompt. There is no `y_or_n` helper — a named
autosave-arc deferral — so Stage 2 adds one rather than repeating
`autosave.lua:219`'s two-element `minibuffer.read` at four call sites.

## 8. Tolerant listing — the Stage 1 Rust change (Q#DR6)

`read_dir` grows a per-entry error channel behind an **opt**. The Lua
result under `{ tolerant = true }` becomes:

```lua
{ entries = { <entry>, ... }, errors = { { name = "..." | nil, message = "..." }, ... } }
```

with per-entry failures recorded and enumeration continuing. Errors on
the **parent** `read_dir` itself stay fatal — a directory you cannot open
has no partial answer. Dired renders a footer line (`N entries
unreadable`) and refuses wdired (§5).

**`name` is optional (R2-2).** A per-entry `readdir` *iterator* error
(`fs.rs:215-218`) carries no filename — the entry never materialized, so
there is nothing to name, and the error is wrapped with the **parent**
path. That arm reports `name = nil`; the footer counts it without naming
it. Every other per-entry arm has an entry in hand and names it.

**What moves into the per-entry channel (F5):** per-entry `readdir`
errors, `symlink_metadata` failures, `read_link` failures
(`fs.rs:228-234`), and **non-UTF-8 symlink targets** (`:227`). A
non-UTF-8 target differs in kind from a non-UTF-8 name: the entry's own
name is fine, the listing renders it with the target shown as unknown,
and nothing needs to pass the target back through `rename`. As it stands
today, one weird symlink in `/tmp` kills the entire listing — the exact
failure class this section exists to fix.

**Non-UTF-8 names stay fatal**, and are a named deferral. Rendering them
tolerantly is not a listing problem but a *path representation* problem:
`FsDirEntry.name` is `String`, every `pmacs.fs` op takes a `String` path,
and `src/fs.rs:152-155` names byte-preserving paths as post-v0.1 work
that widens the whole surface. Doing it properly changes the type of
every path in the API; doing it improperly hands dired a name it cannot
pass back to `rename`. Stage 1 reports the directory as unlistable with
the offending bytes named, which `FsError::NonUtf8Path` already carries.

**Why an opt rather than a shape change.** `read_dir` already takes
`(path, opts)`, so it is signature-natural; it keeps the change additive
for third-party packages; and it leaves the frozen fixture's bare-array
consumption (`init.lua:312`) untouched, which matters because a proof
artifact that must be edited to accommodate new work is not frozen.

**The opts parser must validate (minor c).** `supersede_key`
(`fs.lua:73-83`) reads only `opts.supersede` and silently ignores every
other key, so a typo'd `tolerant` would degrade to fatal mode with no
signal. The tolerant change adds unknown-key rejection to the read ops'
opts parsing.

**Already tolerated, for the record:** a failing `metadata.modified()`
yields mtime 0 rather than an error (`fs.rs:463-476`), so the per-entry
channel is not the first such concession — it is the first *explicit* one.

## 9. Keybindings, display, and the major mode (Q#DR7, Q#DR8, Q#DR10)

**Global** (both unbound repo-wide):

- `C-x d` → `dired` — prompt for a directory, defaulting to the current
  buffer's directory. Takes the standard `display = "current" | "panel"`
  opt (Q#BP11b), defaulting to `"current"` in Stages 1–2 like every other
  adopter.
- `C-x C-j` → `dired-jump` — dired on the current buffer's file's
  directory, cursor seated on that file.

**Visit routing (Q#DR10, F2).** A `RET` on a **file** line goes through
`pmacs.window.display_file(path, { select = true })`, never bare
`find_or_open`. `find_or_open` switches the active window in both
branches before firing hooks (`window_panel.rs:373-376`), so a `RET` in a
panel-displayed dired would replace the panel with the visited file —
the panel swallows itself. `display_file` is the Q#BP11b answer: it dedups
side-effect-free through the **normalizing** `find_buffer_for_path`
before any I/O, resolves the destination before the read, and is what LSP
visits and compile already use. Underneath, its panel-safety comes from
`display_target` (`window_panel.rs:425`) — "the non-side window a visit
from a panel should address".

**Directory descent routes differently, and deliberately (R2-3).** A
`RET` on a **directory** line replaces the dired buffer **in the window
dired already occupies**: `switch_buffer` when dired is in a document
window, and `pmacs.window.display(buf, { side = <same side>, select =
true })` when dired is panel-displayed. This is Emacs behavior — walking
a tree in a side window keeps the side window — and it is the opposite
routing from a file visit for a principled reason: a *file* is not a
dired buffer and belongs in the document area (hence `display_target`),
while the next *directory* is the same kind of thing as the current one
and belongs in the same slot. **Dedication is a property of the slot, not
the buffer**, so a dedicated dired panel stays dedicated across descent
and the new dired buffer inherits it; Stage 1's acceptance pins that
rather than assuming it, since it is a `Layout`/`WindowParams` behavior
this framing does not otherwise touch.

**Mode-scoped on `dired`** (Q#DR8: `scope = "mode", mode = "dired"`,
bound once at load rather than per buffer — dired is the first real
consumer of #129's mode keymaps beyond language detection):

| Key | Command | Stage |
|-----|---------|-------|
| `RET`, `f` | visit (dir → descend, file → `display_file`) | 1 |
| `^` | parent directory | 1 |
| `n` / `p`, `<down>` / `<up>` | move by line | 1 |
| `g` | revert (re-read, preserve cursor and marks) | 1 |
| `q` | quit (restore previous buffer / `window.quit` in a side window) | 1 |
| `s` | cycle sort mode (name → mtime → size) | 1 |
| `m` / `u` / `U` / `t` | mark / unmark / unmark-all / toggle | 2 |
| `d` / `x` | flag for deletion / execute flagged | 2 |
| `D` | delete now (confirms) | 2 |
| `R` / `C` / `+` | rename / copy / mkdir | 2 |
| `w` | copy filename to the kill ring | 2 |
| `C-x C-q` | toggle wdired | 3 |

**Mode-scoped on `wdired`:** `C-c C-c` commit, `C-c C-k` abandon —
matching compile's buffer-local `C-c C-k` idiom without colliding with
it, since dired buffers are never compilation buffers.

Everything follows the `M-;` / `M-%` / `C-c @` precedent of shipping the
faithful Emacs default; users rebind through `pmacs.keymap`.

**Cursor preservation (Q#DR9)** is a Stage 1 requirement, not a nicety:
`g`, a sort, and every Stage 2 operation repaint wholesale, and a dired
that drops you to line 0 after each mark is unusable. The cursor is
re-seated by **basename**, falling back to the nearest surviving line
index when the file is gone — `pmacs.editor.move_to_line`
(`mod.rs:12526`) makes this exact rather than the `move_down`-in-a-loop
walk `listview.lua:68-74` uses.

## 10. Staging and scope

- **Stage 0 (separable, recommended first) — `find-file` (Q#DR11).**
  `C-x C-f` → `pmacs.minibuffer.read` with `source = "files"` and
  `source_root` set to the current buffer's directory, accepting free text
  (`resolve_accepted_value`, `minibuffer.rs:564`) into
  `pmacs.window.display_file`. **Completion is flat and does not
  descend**: a function source cannot list a directory (it is called with
  zero arguments and cannot `await`, F3), and the Rust `Files` source is
  single-directory and 1024-capped. Typing a full path still works via
  free-text accept; typing a *prefix* completes only within the root.
  Hierarchical completion is a named Rust change (§13) — either pass the
  current input to custom sources, or re-root the `Files` source per
  keystroke. **A nonexistent path creates a `[new file]` buffer** rather
  than erroring: `display_file` routes through `resolve_target_buffer`
  (`editor_core.rs:885-898`), which on `ErrorKind::NotFound` creates the
  buffer, binds the path, and sets that status. This is Emacs parity and
  is stated rather than inherited (R2-6). Stage 0 carries its own
  acceptance (§14) — it is small but no longer trivial to describe
  honestly, which is itself an argument for taking it as its own PR. It is
  not required by any later stage; Stage 1's `RET` opens files directly.
  **Say if you want it folded into Stage 1 instead; it is one branch
  either way.**
- **Stage 1 — the dired view. Approval-critical.**
  `builtin/runtime/dired.lua`; the `dired` major mode and mode keymap;
  buffer-per-directory with canonical naming and the ownership check;
  read-only intercept + round-trip input; visit routing through
  `display_file`; parent / sort / revert / quit; `C-x d` (with the
  `display` opt) / `C-x C-j`; cursor preservation across repaint; the
  `dired.kill-when-opening` config key; **and the tolerant `read_dir` opt
  plus its unknown-key validation** (Q#DR6) — the only Rust in this stage.
  No wire change; no protocol bump.
- **Stage 2 — marks and operations.** The mark column and basename-keyed
  mark set; `m`/`u`/`U`/`t`/`d`/`x`/`D`/`R`/`C`/`+`/`w`;
  `pmacs.fs.mkdir` / `copy` / `remove_dir_all`; the **prefix-aware,
  normalized rename rebind in the completion drain** and the matching
  `apply_resource_op` raw-lookup fix (Q#DR5), pinned by a **no-await**
  rename and by a directory rename that must not strand the buffers
  beneath it; a `y_or_n` confirm helper; and removal of the Lua
  canonicalization mirror if Stage 1 shipped one (Q#DR2).
- **Stage 3 — wdired.** `C-x C-q` mode swap; the column-classifying
  intercept over the mark-shifted layout; escape/unescape round-trip;
  duplicate-name and NUL/slash rejection pre-syscall; two-phase rename;
  field-by-field external-change detection; the symlink perms and symlink
  target rules; partial-application reporting.
- **After Stage 3 — shrink the M8 fixture** to the minimum payload that
  still proves package universality (§3).

Stages 2 and 3 are sketched here and each gets its own detailed framing
after the prior stage lands, per the folding-arc precedent. **This
framing asks approval for the architecture and Stage 1's detail.**

## 11. Numbered decisions

- **Q#DR1** Dired ships as `builtin/runtime/dired.lua`, written fresh;
  the M8 fixture stays frozen under `tests/fixtures/` because the
  **harness routing** (46/47 tests reaching dired through `install_local`
  + `require`) *is* the universality proof and dies if re-pointed, and
  because the builtin diverges structurally. Its 45 behavioral assertions
  are re-pinned by §14. Shrinking the fixture is **scheduled after
  Stage 3**. (§3)
- **Q#DR2** One buffer per directory, `*dired:<canonical abs path>*`,
  found-or-created by name; navigation opens the target's buffer rather
  than renaming the current one (there is no `pmacs.buffer.set_name`).
  Names and lookups are **lexically normalized** (tilde, absolutize,
  `.`/`..`, trailing slash) with **symlinks deliberately unresolved**;
  found-by-name **verifies dired ownership** before painting.
  A Lua mirror of `normalize_buffer_path` is a second canonical form and
  requires a **parity acceptance** against the Rust normalizer; exposing
  the normalizer instead is the preferred end state, since Q#DR5 gives it
  a second consumer. `dired.kill-when-opening` (default `false`) mirrors
  Emacs 28's opt-out. (§4)
- **Q#DR3** Read-only via `add_intercept` + `set_round_trip_input`, with
  dired's own paints using `bypass_intercept`; dired owns its buffer and
  does **not** extend `pmacs.listview`; wdired is a major-mode swap, and
  refuses to open on a partially-listed directory. (§5)
- **Q#DR4** Mark column at column 0 shifts all offsets; marks are keyed
  by **basename**, never line index; `*` and `D` are the two mark
  characters; a vanished basename is dropped from a batch and reported.
  (§6)
- **Q#DR5** Stage 2 adds `pmacs.fs.mkdir` / `copy` / `remove_dir_all`.
  Rename rebinding is fixed **at the primitive** (zero production callers
  to break), **prefix-aware** (a directory rename must not strand the
  buffers beneath it), **normalize-before-lookup** (fixing
  `apply_resource_op`'s raw `find_by_path` in the same change), and in the
  **main-thread completion drain** `AsyncRuntime::tick` — never in the
  take/await path, which a fire-and-forget rename never reaches. Because
  rename settles as an undifferentiated `ReplyKind::FsUnit`, the drain
  keys on the pending job's `JobKind::FsRename` and the job retains
  from/to. `R` on a directory is supported in v1; `C` refuses directory
  sources. Destructive ops confirm via a new `y_or_n` helper. (§7)
- **Q#DR6** `read_dir` becomes per-entry tolerant behind an **opt**
  (`{ tolerant = true }`), carrying per-entry `readdir`/`lstat`/`readlink`
  failures **and non-UTF-8 symlink targets** in an `errors` channel;
  parent-level failures stay fatal; non-UTF-8 **names** stay fatal,
  deferred to a byte-preserving path surface. The read ops' opts parsing
  gains unknown-key rejection. (§8)
- **Q#DR7** Emacs-parity bindings: global `C-x d` (with the `display`
  opt) / `C-x C-j`; mode-scoped in-buffer keys per the §9 table; wdired on
  `C-x C-q` with `C-c C-c` / `C-c C-k`. (§9)
- **Q#DR8** Keys are **mode-scoped** (`scope = "mode", mode = "dired"`),
  not buffer-local — bound once at load, and the wdired swap changes the
  whole keymap with the mode. Dired is #129's first non-detection
  consumer. (§9)
- **Q#DR9** Cursor is re-seated by **basename** after every repaint,
  falling back to the nearest surviving index, via
  `pmacs.editor.move_to_line`. (§9)
- **Q#DR10** File visits route through `pmacs.window.display_file`
  (panel-safe via `display_target`), never bare `find_or_open`, which
  switches the active window before hooks and would let a `RET` replace
  the panel dired is displayed in. **Directory descent instead reuses
  dired's own window** — `switch_buffer` in a document window,
  `display { side = <same side>, select = true }` in a panel — because
  the next directory is the same kind of thing as the current one.
  Dedication is a slot property and follows across descent. (§9)
- **Q#DR11** Stage 0's completion is the flat Rust `Files` source rooted
  at the current buffer's directory, plus free-text accept. Function
  sources cannot descend (zero-argument call, no `await` in the dispatch
  context); hierarchical path completion is a named Rust deferral.
  **Amended by S0-1:** the two are not independent — a selected candidate
  **shadows** typed text, so free-text accept is reached only when the
  input filters every candidate away (in practice, when it contains a
  `/`). The prompt field therefore starts empty (S0-2), and a leading
  `~` is expanded Lua-side (S0-3). (§0, §10)

## 12. Bets

- **B1** The fixture's hard parts — escaping with an exact inverse,
  two-phase rename, field-by-field external-change detection, the symlink
  rules — transfer to the builtin as decided design. FALSIFIABLE at
  Stage 3: if the mark-shifted layout or the mode swap forces a different
  commit model, the fixture stops being a reference and Stage 3 is
  re-framed from scratch.
- **B2** Tolerant `read_dir` is the *only* Rust change Stage 1 needs —
  `display_file`, mode keymaps, `move_to_line`, and
  `set_round_trip_input` all already exist. FALSIFIABLE during
  implementation; the most likely miss is **scroll** preservation, since
  the daemon owns `view_top` and "viewport facts on the wire" is a
  standing backlog gap — if preserving scroll (not just cursor) across a
  repaint needs a new fact, Stage 1 grows.
- **B3** Mode-scoped single-key bindings survive both frontends, because
  `set_round_trip_input` keeps optimistic-apply off — and, when dired sits
  in a panel, the `!window.is_side()` arm of the same gate
  (`editor.rs:818`) covers it a second time. FALSIFIABLE on a real GPU
  session: pressing `d` must flag, never insert.
- **B4** Buffer-per-directory does not produce clutter users complain
  about (Emacs parity), and `dired.kill-when-opening` is a sufficient
  escape hatch. Falsifiable only by use.
- **B5** Flat, non-descending completion is acceptable for Stage 0
  because free-text accept covers the full-path case. FALSIFIABLE
  immediately by use: if typing full paths blind is what people actually
  do, hierarchical completion stops being a deferral and becomes Stage 0's
  real scope.

## 13. Deferred (named)

- **Hierarchical path completion** — pass the current minibuffer input to
  custom sources, or re-root `CompletionSource::Files` per keystroke
  (Q#DR11).
- **Typed text vs. a selected candidate on accept** (S0-1) — prefer the
  typed contents when they differ from the selection and the user has not
  explicitly moved it. Closes Stage 0's "a new bare name that is a
  subsequence of an existing entry opens the existing file" hole, but
  changes `M-x` and `switch-buffer` accept semantics too, so it needs its
  own reasoning and gates. **It would also close the empty-input case**
  (S0-4): `fuzzy_score` returns `Some(0)` for an empty needle
  (`minibuffer.rs:637-640`) and `filter_and_sort` breaks ties
  lexicographically (`:678`), so accepting immediately opens the
  first-sorted entry — dotfiles first, and possibly a directory, which
  then fails and reports. Inherited from the shared minibuffer, not
  introduced by find-file, and recorded as decided rather than
  overlooked.
- **Minibuffer history stores the pre-join value** (S0-5) —
  `Minibuffer::accept` pushes the *resolved* value (a bare basename, or a
  root-relative path) into the history bucket Rust-side, **before** Lua
  joins it onto the root. So recalling `sub/inner.txt` with `C-p` under a
  *different* root resolves against the new root, and can silently create
  a `[new file]` buffer somewhere else. Emacs's `file-name-history` stores
  absolute paths. Lua cannot fix this — the push happens before
  `on_accept` runs — so it belongs with the other Rust-side minibuffer
  deferrals here.
- **Load through the normalized path** (S0-3) — `get_or_load_buffer`
  computes a normalized path and then loads from the raw one
  (`editor_core.rs:842-856`), so tilde paths dedup but do not load. Same
  normalize-before-lookup family as Q#DR5's `apply_resource_op` fix.
- **Non-UTF-8 filenames** — needs byte-preserving `pmacs.fs` paths
  (`src/fs.rs:152-155`); widens every path in the API. (Non-UTF-8 symlink
  *targets* are handled in Stage 1, §8.)
- **Shrinking the M8 fixture** — scheduled after Stage 3 (§3).
- **Factoring the shared panel disciplines** out of dired and
  `pmacs.listview` (§5).
- `o` / `C-o` visit-in-other-window — the GPU has no splits (GPU
  structural parity, roadmap Arc 8).
- `!` / `&` shell command on marked files; `Q` query-replace across marked
  files; `A` search across marked files.
- `i` insert-subdirectory (in-buffer recursive listing) and
  `dired-hide-details`.
- Owner and group columns — no uid/gid → name primitive exists.
- Human-readable sizes; sort by extension; reverse-sort toggle.
- `%m` / `%d` regex mark family; `dired-omit-mode`.
- Recursive copy (`C` on a directory), which v1 refuses.
- Auto-revert on external change — `pmacs.fs.watch` exists and polls
  (`fs.lua:226`), so this is wiring plus a policy decision about polling a
  directory the user is not looking at.
- Dired buffers in the desktop session — covered by Arc 3's standing
  "non-file buffers in the desktop" deferral.
- Remote / Tramp-style paths.
- Symlink creation and symlink-target editing (the fixture rejects target
  edits at commit; `init.lua:821-840`).

## 14. Acceptance

### Stage 0 — `find-file` (R2-6)

0a. **Flat completion within the root.** With the current buffer in a
    temp directory, `C-x C-f` offers that directory's entries as
    candidates and does **not** offer entries of a subdirectory —
    documenting the flat-source limitation as intended behavior rather
    than leaving it unpinned.
0b. **Free-text accept of a deeper path.** Typing a full path below the
    root and accepting opens that file, with no candidate selected
    (`resolve_accepted_value`'s raw-input path).
0c. **Nonexistent path creates.** Accepting a path that does not exist
    yields a buffer bound to it, unmodified and empty, with the
    `[new file]` status — not an error.
0d. **No-path origin.** From a buffer with no backing path, the prompt
    roots at the process cwd rather than erroring or offering nothing.

### Stage 1 — the dired view

1. **Listing shape.** `C-x d` on a temp directory renders a header line
   plus one line per entry, with kind char, perms, size, mtime, and name;
   a symlink renders `l` with ` -> target`; the entry count matches
   `read_dir`.
2. **Visit dispatches on kind, through the panel-safe primitive
   (Q#DR10).** `RET` on a subdirectory line opens that directory's dired
   buffer; `RET` on a **file** line opens the file bound to its path (the
   fixture's "not yet exposed" error is gone). `RET` on the header does
   nothing. **The panel case is the real assertion**: with dired opened
   `display = "panel"`, a `RET` on a file line leaves the dired panel
   alive and puts the file in the document window — **falsified by
   swapping `display_file` for `find_or_open`**, which must make the panel
   disappear.
3. **Buffer-per-directory and canonicalization (Q#DR2).** Descending
   twice then ascending twice yields the *same* buffer ids as the first
   visit; every dired buffer's name matches the directory it displays;
   and `C-x d` on `/tmp`, `/tmp/`, and `/tmp/../tmp` (with a real temp
   dir) yields **one** buffer, not three. With
   `dired.kill-when-opening = true`, the departed buffer is gone.
3b. **Canonicalization parity (R2-4).** One shared edge-case list —
   `//tmp`, a trailing slash, `~` with `HOME` set and unset, `.`/`..`
   segments including a `..` that would escape root, a relative path —
   driven through **both** dired's canonicalizer and the Rust
   `normalize_buffer_path`, asserting identical output. If Stage 1
   exposes the normalizer instead of mirroring it, this degenerates to a
   round-trip test and the mirror-removal follow-up is dropped.
3c. **Panel descent (Q#DR10, R2-3).** With dired opened
   `display = "panel"`, `RET` on a **directory** line leaves dired in the
   same side window showing the new directory — the panel is neither
   replaced by a document window nor duplicated — and a dedicated panel
   is still dedicated afterward.
4. **Ownership check (Q#DR2, F7).** A foreign `pmacs.buffer.create`
   buffer named exactly `*dired:<path>*` is **not** adopted: `C-x d` on
   that path leaves the foreign buffer's contents byte-identical and
   opens dired elsewhere.
5. **Read-only (Q#DR3).** A `buffer.self-insert` into a dired buffer is
   rejected by the intercept and leaves the text byte-identical; dired's
   own repaint succeeds through `bypass_intercept`. `set_round_trip_input`
   is set, pinned **through the real dispatch path** so a semantic
   frontend's `d` reaches the binding rather than optimistic-applying —
   falsified by reverting the `set_round_trip_input` call, not by a
   direct-call assertion.
6. **Mode keymap (Q#DR8).** The keys resolve through `scope = "mode"`
   with no per-buffer binding: a *second* dired buffer, created without
   any `keymap.bind` call of its own, still responds to `g` and `^`.
   `pmacs.buffer.major_mode(buf)` is `"dired"`, and the mode shows in the
   statusline.
7. **Cursor preservation (Q#DR9).** With the cursor on entry `k`, `g`
   re-seats on the same **basename** after an external file was added
   *above* it (so the line index changed); when that basename is deleted
   externally, the cursor lands on the nearest surviving line, not line 0.
8. **Sort.** `s` cycles name → mtime → size → name; mtime sorts newest
   first and size largest first, each with a stable name tiebreak; the
   cursor stays on its basename across the reorder.
9. **Tolerant listing (Q#DR6).** In a directory containing a child whose
   `lstat` fails, `{ tolerant = true }` returns the surviving entries plus
   one `errors` row naming the child; dired renders every readable entry
   plus the unreadable-count footer; and **the default (non-opt) call
   still returns a bare array** — both forms called in one test, so the
   fixture's contract cannot regress unnoticed. A failure on the parent
   directory itself is still fatal.
10. **Tolerant symlink targets (Q#DR6, F5).** A directory containing a
    symlink whose target is non-UTF-8 lists successfully under
    `{ tolerant = true }`, with that entry present and its target reported
    unknown — **falsified by reverting the `read_link`/target arm**, which
    must take the whole listing down.
11. **Unknown opts rejected (minor c).** `read_dir(path, { tolerat = true })`
    errors naming the unknown key rather than silently listing in fatal
    mode.
12. **Non-UTF-8 names stay fatal, and say so.** A directory containing a
    non-UTF-8 *name* reports the structured `NonUtf8Path` error with the
    offending bytes; dired surfaces it as a status message and creates no
    buffer.
13. **`dired-jump`.** From a file buffer, `C-x C-j` opens dired on that
    file's directory with the cursor on that file's line. From a buffer
    with no path, it reports that and creates nothing.
14. **Quit.** `q` restores the previously active buffer; in a side window
    (`display = "panel"`) it routes through `pmacs.window.quit`, matching
    `listview.quit`'s Q#BP11b split.
15. **Failure leaves nothing behind.** `C-x d` on a nonexistent or
    unreadable directory creates no buffer, switches no window, and
    reports the reason — the fixture's
    `dired_open_failure_leaves_editor_unchanged` invariant.
16. **Scale.** A 10,000-entry directory renders within the fixture's
    established 200 ms budget, on the builtin path — carrying the same
    `cfg_attr(target_os = "macos", ignore)` gate the fixture's version
    uses (`m8_2_acceptance.rs:211-213`), since hosted macOS debug runners
    do not consistently satisfy it.
17. **The fixture still passes.** `m8_2_acceptance` 15/15 and
    `m8_3_acceptance` 32/32 unchanged, proving the `read_dir` opt is
    additive.

Every behavioral claim above is bite-verified with `scripts/bite`.

## 15. Gates (Stage 1)

`cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D
warnings` as its own step; `cargo test --lib`; `cargo test --lib
--features crdt`; `tests/dired_acceptance.rs` (default + CRDT);
`tests/m8_1_acceptance.rs`, `tests/m8_2_acceptance.rs`, and
`tests/m8_3_acceptance.rs` (the additivity proof — m8_1 because it
exercises `pmacs.fs.rename` and `read_dir` directly); `cargo test --test
m4_acceptance -- --skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test -p
pmacs-gpu`; the workspace sweep **with an isolated `XDG_CONFIG_HOME`**
(the real `~/.config/pmacs/init.lua` on this desktop calls
`install_local`, which races every editor the sweep builds and leaks a
status message into frame-comparing suites); `git diff --check`.

## 16. Branch and PR plan

Branch `dired`, worktree `../pmacs-dired-arc` — **not** `../pmacs-dired`,
which would read as the fixture. **Based on canonical `githubsucks/main`
@ `0827dd1`** (Lean 4 Stage 1 #160), which is one merge ahead of the
scout's `e745068`; see §2's base note for why that movement does not
disturb the ground truth.

**The shared checkout is not the place to cut this.** It currently has
`lean4-stage1` checked out with in-progress foreign work
(`src/highlight.rs` modified), and this framing is untracked in it. Per
the §5 ops rule, the branch is cut as a **sibling worktree off `main`**
and the framing is committed there as the branch's first commit, rather
than by switching the shared checkout. The framing does not travel until
that commit is pushed.

Stage 1 implements on the same branch and opens as the first dired PR.
Stages 2 and 3 are separate branches and PRs off the `main` that results
from the prior stage, each with its own detailed framing. If Stage 0
(`find-file`) is taken separately it goes first, on its own branch
`find-file`, and Stage 1 rebases onto the resulting `main`.
