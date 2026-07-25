# Dired Stage 2 — marks and operations — framing

**Revision 3 — 2026-07-25. Status: PROPOSED; review rounds 1 and 2
addressed (round 2: four blocking, two high, four cleanups).**
Continues `docs/dired-framing.md` (rev 7, approved; Stage 0 merged as
#162, Stage 1 as #165). That document's §6 and §7 carry the *approved*
shape of marks and operations; this one re-verifies every claim in them
against `main` @ `c8ec8f3`, corrects what has drifted or was wrong, and
adds what a Stage 2 implementation needs and the parent did not decide:
the batch-execution contract, the confirmation surface, the staging cut,
and acceptance. Decisions continue the parent's scheme from **Q#DR12**.

The parent framing's ground truth (§2) was scouted at `main` @ `e745068`
and re-verified at `0827dd1`. Stage 1 then changed three of the files
Stage 2 depends on most (`src/fs.rs`, `src/async_runtime.rs`,
`src/lua_bindings/mod.rs`), so **§2 below supersedes the parent's line
references for everything Stage 2 touches.**

---

## 0. Revision history

### Review round 1 (rev 1 → rev 2)

Every checkable claim in the review was verified against `c8ec8f3`
before being acted on; all seven held.

- **F1 (blocking) — the rename contract reached only one path owner.**
  Rev 1 rebound `Buffer.file_path` and stopped. Verified: `Buffer::set_name`
  documents itself as "used by save-as and **rename operations**"
  (`src/buffer.rs:452`) and `set_buffer_path` never calls it, so the
  statusline and buffer list keep the old filename; `rec.uri` is cached
  per attachment in `lsp.lua` and read at ~20 sites (didChange `:418`,
  semantic tokens `:776-790`, diagnostics `:953`, signature `:1016`,
  definition `:1648`, references `:1757`); dired's own buffers are
  **pathless** so no buffer-keyed rebind can ever reach them; and the
  workspace-edit path captures `origin = active_buffer_path()`
  (`lsp.lua:1252`) then calls `find_or_open(origin)` (`:1266`), which for
  a since-renamed path does not fail gracefully — `resolve_target_buffer`
  turns `NotFound` into "an empty path-backed buffer"
  (`editor_core.rs:876-878`), i.e. it **materializes a phantom buffer at
  the obsolete path and selects it**. Rev 2 replaces the rebind with a
  shared reconciliation transaction plus a `resource.renamed` hook (§5).
- **F2 (blocking) — deletion of visited paths had no policy.** Correct,
  and rev 1 simply did not consider it. §6 is new and decides all four
  cases; the modified-buffer case blocks rather than orphans.
- **F3 (high) — the key table silently changed approved scope.** Verified
  against the parent's table (`dired-framing.md:963-966`): it lists `w`
  (copy filename to the kill ring) in Stage 2 and contains **no** `M`.
  Rev 1 dropped `w` and added `M` without saying so. Worse, the parent
  states at `:543-546` that the fixture "rejects symlink perms edits at
  intercept time ... **that decision carries over unchanged**", and rev 1's
  `M` chose to chmod the symlink's target with a warning — a direct
  contradiction of approved text. `w` is restored (§8, Q#DR20) and `M`
  is now an explicit new decision that **refuses symlinks** (Q#DR19).
- **F4 (high) — Q#DR13 contradicted the `R` contract.** It did: "every
  operation targets the marked set ... except `x`", then `R` point-only.
  Q#DR13 is narrowed to name the set-based operations explicitly, with
  `R` and `w` as point-based by construction (§4), plus acceptance for
  `R` with unrelated marks present.
- **F5 (high) — load-bearing decisions lacked falsifying acceptance.**
  All five additions taken; see §13 items 14, 27, 38, 39, and 40
  (renumbered in rev 3).
- **F6 (medium) — `take_settled_renames()` was underspecified.** Correct:
  a no-argument drain either needs a second queue or a scan of every
  settled entry, neither of which rev 1 named. Rev 2 takes the reviewer's
  preferred shape — `tick` returns a structured outcome so settle
  identity and rename metadata stay in one transaction (§5).
- **F7 (2b) — overwrite and recursive-delete safety were incomplete.**
  §8 now defines the `C` command flow including per-collision handling in
  a multi-source batch, pins `remove_dir_all`'s lstat safety at the
  primitive, and states `dired.recursive-deletes` as boolean, default
  **false**.

One thing the review asked for that rev 2 does **not** do: it does not
widen `R` to the marked set. Multi-file rename needs a target-directory
concept that does not exist, so `R` stays point-based and is *named* as
such rather than being an unstated exception. §11 keeps it deferred.

---

### Review round 2 (rev 2 → rev 3)

Round 2's finding: rev 2 widened the rename fix into a *resource
transaction*, and four consumers of that transaction were named but not
actually reachable by it. All six substantive claims verified against
`c8ec8f3`.

- **G1 (blocking) — acceptance 29 was unimplementable by the proposed
  design.** `apply_workspace_edit` captures `origin` as a **string**
  (`active_buffer_path()` is `pmacs.editor.file_path()`,
  `lsp.lua:471-473`), so neither `reconcile_rename` nor `resource.renamed`
  can reach an already-captured Lua local; the phantom survives. Fixed by
  changing the applier: capture the **buffer handle** and restore with
  `pmacs.window.switch_buffer`, with **no path fallback** (§5).
- **G2 (blocking) — the dired subscriber could not rename its own
  buffer.** `dired.lua:34-38` records in its own module doc that **there
  is no `pmacs.buffer.set_name`**, so "updates its handle and renames its
  buffer" was not implementable. Rev 3 adds the narrow setter, which §5
  needs anyway for the `Buffer.name` half of the transaction (§5, Q#DR21).
- **G3 (blocking) — `rec.uri` was not the last LSP path owner.**
  `DiagnosticView` captures its URI **at construction** and the field's
  own doc says so: *"Set once at construction; M5 may add re-rooting if a
  buffer is renamed"* (`src/diag.rs:455-457`). Five more stores are
  URI-keyed (`pmacs.diag`, `semantic_tokens`, `signature`, `definition`,
  `references`). §5 now carries a complete teardown/re-attach contract.
- **G4 (blocking) — Q#DR18 had no shared seam and was racy across the
  prompt.** Verified: `apply_resource_op`'s delete arm
  (`mod.rs:3256-3285`) kills via `find_by_path` — raw path, **first match
  only, no descendants, and no modified check**, so an LSP-driven delete
  **destroys unsaved work today**. Rev 3 defines one shared
  `reconcile_delete` seam and revalidates modified state immediately
  before each syscall (§6).
- **G5 (high) — `w` had no implementation surface and the wrong
  semantics.** `push_entry` is a `local function` (`killring.lua:93`) and
  the public `copy()` requires a region, failing with "no region"
  otherwise (`:182-190`). Rev 3 adds `pmacs.killring.push` and — taking
  the reviewer's point that the parent approved the *binding*, and Emacs
  copies marked filenames — makes `w` **set-based**, so `R` is now the
  only point-based operation (§4, Q#DR20).
- **G6 (high) — `R`'s no-clobber was only a preflight.** `rename_blocking`
  calls plain `std::fs::rename` (`fs.rs:492-499`), which silently
  replaces an existing target, so a target appearing between the check and
  the syscall is overwritten. Rev 3 **narrows the claim** rather than
  overstating it, and names a no-replace primitive as deferred (§8).
- **G7 — four cleanups**, all taken: `lsp_multi_root` added to §14, the
  §13/§7 cross-reference slips fixed, and the stale "2a's only Rust is the
  rename rebind" line corrected — it is no longer true after the widened
  transaction.

### Corrections to the parent framing (rev 1, unchanged)

Five corrections, one of them load-bearing.

- **C1 (load-bearing).** The parent says the rename rebind belongs "in
  the main-thread completion drain, `AsyncRuntime::tick`
  (`async_runtime.rs:991`)". The *decision* is right and this framing
  keeps it, but **it cannot be implemented there**: `AsyncRuntime`
  (`src/async_runtime.rs:513-549`) holds a worker pool, a bus, and job
  tables — it has no buffer registry, no `EditorCore`, and no Lua handle.
  Putting a buffer rebind inside it would invert the layering. The actual
  seam is one level up: **`pmacs._async._tick`
  (`src/lua_bindings/mod.rs:6911-6924`)**, the production binding that
  calls `rt.tick()` and is already a `lua.create_function` closure with
  `lua` in scope — so it can reach `SharedCore` by
  `lua.app_data_ref::<SharedCore>()`, exactly as `apply_resource_op`
  does (`:3251`). §5 specifies the split: `AsyncRuntime` *harvests*, the
  binding *rebinds*.
- **C2.** The parent's line references have drifted. `tick` is now
  `:1003` (was 991); the `Sleep | FsUnit → JobResult::Unit` arm is
  `:1046-1048` (was 1022-1025); `apply_resource_op`'s raw lookup is
  `:3249` (was 3248). Stage 1's `ReadDir` variants account for the shift.
  Three references were already exact and still are: `find_by_path`
  (`buffer_registry.rs:168-174`), `find_buffer_for_path`
  (`editor_core.rs:864-867`), and the normalize-on-write in
  `set_buffer_path` (`:819`).
- **C3.** The parent says the fixture's 45 behavioral tests are the
  reference for dired behavior. For Stage 2 that is **false and worth
  knowing**: the frozen fixture has **no mark-and-operate layer at all**.
  It defines eight commands — `open-line`, `parent`, three sorts, and
  three `wdired` — and binds exactly two keys (`RET`, `Backspace`,
  `tests/fixtures/pmacs-dired/init.lua:381-388`). Its "marks" are
  `pmacs.buffer.mark_create` *text position* marks used by wdired
  (`:716-770`), not dired mark flags. **Stage 2 has no in-repo reference
  implementation and no existing test coverage to match.** Stage 3 does.
- **C4.** The parent's `y_or_n` claim is confirmed stronger than stated:
  grep across `builtin/` and `src/` finds **no** `y_or_n`, `yes_or_no`,
  or `yes-or-no` anywhere, and there is no `builtin/runtime/minibuffer.lua`
  at all — `pmacs.minibuffer` is entirely Rust-provided. §7 decides where
  the helper lives.
- **C5.** The parent's list of three missing primitives is correct, and
  `remove` covers more than it implies: `remove_blocking`
  (`src/fs.rs:561-583`) already deletes **files and empty directories**,
  and correctly unlinks a symlink-to-a-directory rather than following it.
  So `remove_dir_all` is needed only for **non-empty** directories. This
  is what makes the staging cut in §10 possible.

---

## 0.5. Coherence impact (`COHERENCE.md` §20, required since #163)

- **Journey steps.** Touches **step 7's file half**, which #165 moved from
  "missing" to "fixed but unadvertised". Stage 2 does not change that
  grade: it deepens a surface the user must still already know about.
  Discoverability is §20 Priority 4's job, not this stage's, and this
  framing does not claim otherwise.
- **Interaction islands: adds none.** Every new key is an entry in the
  existing mode-scoped `dired` keymap (`dired.lua:866-879`), and every new
  command is an ordinary `pmacs.command.define`, so all of it is reachable
  from `M-x` and describable by `describe.key`. The confirmation prompt
  uses the existing minibuffer rather than a new modal surface — that is
  the *reason* §7 spends a section on it.
- **Config registry.** Stage 2a adds no keys. Stage 2b adds
  `dired.recursive-deletes` (Emacs's `dired-recursive-deletes`), through
  `pmacs.config.define` like `dired.kill-when-opening`.
- **Background-work attribution (§9): this stage makes a real, and
  honest, dent in the wrong direction unless it is deliberate.**
  `COHERENCE.md` §9 grades the worker model "mechanism without identity"
  and names the prerequisite precisely: `PendingJob`
  (`src/async_runtime.rs:367-394`) carries no owner, purpose, buffer
  association, or parent, and `JobKind` is a **closed 12-variant enum**.
  Stage 2 pushes on both:
  - It must make a pending rename **retain its from/to paths** (§5). That
    is a new `PendingJob` field which is *not* an owner or a purpose — a
    one-off. The alternative shape, a `job_id → (from, to)` side map on
    `AsyncRuntime`, is **worse by §9's own diagnosis**: §9 singles out the
    existing parse-job→buffer link for living "in a `SyntaxCoordinator`
    side map, invisible to the workers view". So the field is the
    coherent choice of the two, and this framing takes it — while
    recording that a general `purpose`/`owner` field should later subsume
    it, and that Stage 2 is not the place to design that.
  - Stage 2b grows `JobKind` from 12 to 15 variants. That is additive and
    matches every existing fs op, but the closedness §9 objects to is not
    something this stage fixes, and the framing does not pretend the
    growth is progress.

---

## 1. What Stage 2 ships

The Emacs dired working loop: select a set of files, then act on it.

| Key | Command | Effect |
|---|---|---|
| `m` | `dired.mark` | Mark entry under cursor `*`, advance |
| `u` | `dired.unmark` | Clear the mark, advance |
| `U` | `dired.unmark-all` | Clear every mark in this listing |
| `t` | `dired.toggle-marks` | Invert `*` marks across the listing |
| `d` | `dired.flag-delete` | Flag `D`, advance |
| `x` | `dired.execute-flags` | Delete every `D`-flagged entry, after confirming |
| `D` | `dired.do-delete` | Delete the marked set (or entry at point) now, after confirming |
| `R` | `dired.do-rename` | Rename the entry **at point** (§4) |
| `w` | `dired.copy-filename` | Copy the marked filenames to the kill ring |
| `M` | `dired.do-chmod` | Mode bits on the marked set; **refuses symlinks** (NEW — Q#DR19) |
| `+` | `dired.create-directory` | Create a subdirectory (**2b**) |
| `C` | `dired.do-copy` | Copy the marked set (or entry at point) (**2b**) |

`w` is carried forward from the parent's approved table; **`M` is new
scope** and needs explicit approval, since the parent listed neither it
nor any chmod surface for Stage 2 (F3).

Two small public surfaces come with it, both because the operations have
nowhere to land otherwise: **`pmacs.buffer.set_name`** (Q#DR21) and
**`pmacs.killring.push`** (Q#DR22).

Plus, invisibly: **renaming or deleting a path starts reconciling every
consumer that holds it** — buffer path *and* name, the five URI-keyed LSP
stores and the attached diagnostic view, dired's own pathless handles,
and the workspace-edit applier (§5, §6). That is a correctness fix to
shared substrate; it is the reason `R` is safe on a directory at all, and
it closes a path on which an LSP-authored delete currently destroys
unsaved work.

Not in Stage 2: `wdired` (Stage 3), subdirectory insertion (`i`),
shell commands on marks (`!`), regexp marking (`% m`), and
compress/symlink/hardlink ops. §11 names them.

---

## 2. Ground truth (scouted 2026-07-25, `main` @ `c8ec8f3`)

Everything below was read or executed on this tree, not inferred.

### The filesystem surface

`pmacs.fs` is exactly **five ops plus `watch`**
(`builtin/runtime/fs.lua`): `read_dir` (:124), `stat` (:133), `rename`
(:167), `chmod` (:177), `remove` (:187), `watch` (:267). Rust side:
`read_dir_blocking` (`src/fs.rs:289`), `stat_blocking` (:449),
`rename_blocking` (:492), `chmod_blocking` (:521), `remove_blocking`
(:561). **No `mkdir`, no `copy`, no recursive remove exists anywhere.**

- `remove_blocking` (`:561-583`) lstats first, then `remove_dir` for a
  **real** directory (non-recursive) and `remove_file` otherwise, with an
  explicit comment that a symlink-to-a-directory is not a directory per
  `lstat` and so gets unlinked rather than followed. **So `remove`
  already handles files and empty directories, and deleting a symlink
  never touches its target.**
- Mutating ops deliberately take **no** `supersede`, and `fs.lua:155-165`
  states the reason (a cancelled mutation may still have completed) and
  the prescribed alternative: *"If a package needs at-most-one-pending
  semantics for mutations, it should serialize on the package side (await
  each op before dispatching the next)."* §9 takes that instruction
  literally.
- `chmod` **follows symlinks** (`fs.lua:145-153`) while `read_dir`/`stat`
  lstat. So `M` on a symlink line changes the *target's* mode and a
  refresh shows the link's own, unchanged mode. This is documented
  substrate behavior, not a bug to fix here; §8 decides what `M` does
  about it.
- `read_opts` (`:82-105`) rejects unknown keys. Any new op that takes
  opts must go through it or repeat that discipline.

### Rename, and why the buffer rebind is not where you would put it

- `pmacs.fs.rename` has **zero production callers.** Repo-wide the only
  callers are the frozen fixture (`init.lua:1200`, `:1210`),
  `tests/m8_1_acceptance.rs:359`, and `tests/m8_3_acceptance.rs:182-184`
  — which *monkeypatches the Lua function*. Changing rename's contract
  therefore breaks nobody, and the monkeypatch matters for test design:
  a Lua-level replacement bypasses a Rust-level rebind entirely.
- The fixture's wdired commit renames in **two phases through unique temp
  names** (`init.lua:1197-1215`), awaiting each. A prefix-aware rebind
  will therefore rebind an affected buffer twice (real→temp→final),
  landing correctly. Worth knowing before reading a confusing trace.
- `dispatch_fs_rename` (`async_runtime.rs:871-879`) **moves `from` and
  `to` into the worker closure.** Nothing retains them, which is exactly
  why §5 needs a new field.
- `PendingJob` (`:367-394`) carries `{cancel, state, supersede_key,
  stream_buffer, max_batch, kind, dispatched_at}` — `kind` is there, the
  paths are not.
- Rename settles as an **undifferentiated `ReplyKind::FsUnit`**: the
  `Sleep | FsUnit` arm maps both to `JobResult::Unit` (`:1046-1048`).
  There is no `Rename` reply variant, so a drain **cannot key on the
  reply**; it must key on the pending job's own `JobKind::FsRename`.
- **`tick`'s post-loop block already does exactly the read the harvest
  needs.** `:1074-1108` iterates `newly_settled`, borrows `pending`,
  and reads `job.kind`, `job.state`, `job.dispatched_at`, and
  `job.supersede_key` to push a `CompletedSlot`. The harvest is one more
  read in a loop that already exists.
- `find_by_path` (`buffer_registry.rs:168-174`) is exact `Path` equality
  over `self.order`, **first match only**. No prefix logic, and two
  buffers on one path means one of them is invisible to it.
- `apply_resource_op`'s rename arm (`mod.rs:3234-3255`) does a
  **synchronous** `std::fs::rename` on the main thread and then
  `reg.borrow().find_by_path(&from)` with the **raw** path (`:3249`),
  while stored paths are normalized on write (`editor_core.rs:819`) and
  the normalizing wrapper `find_buffer_for_path` (`:864-867`) exists and
  is bypassed. This is a **second**, LSP-facing rename path with the same
  two defects; §5 fixes both in one change.

### A verified pre-existing defect Stage 2 must not lean on

**A fire-and-forget non-stream job leaks its pending entry forever.** The
only two removals from `pending` are stream eviction of closed streams
(`:1220-1226`) and `take_result` (`:1262-1282`); the Lua `Handle` is a
bare `setmetatable({_id = id}, Handle)` (`async.lua:55`) with **no
`__gc`**, so dropping a handle reaps nothing.

Executed on this tree to confirm rather than infer — dispatch a rename,
pump until complete, never take the result:

```
PROBE renamed_on_disk=true pending_len_after_settle=1
PROBE is_complete=true snapshot_active=0 snapshot_completed=1
```

Two consequences. Good: **the settled job is still in `pending` when the
harvest runs**, so §5's design is sound. Bad: this is a real leak, it is
**pre-existing and not Stage 2's to fix**, and Stage 2 must not make it
routine. §9's serialize-and-await contract means dired's own ops reap
every entry they create; the rebind exists for *other* callers, who leak
today regardless. Named as a deferral (§11) with the note that
`pending_len` is the observable.

### Marks: the precedent already in the tree

`*buffer-list*` keys deletion marks by **stable id, not line index**
(`builtin/commands/default.lua:373`, set at `:507`, cleared at `:520`),
and **prunes marks whose target no longer exists** on refresh
(`:444-452`). That is the exact shape §3 adopts, one substitution
(basename for buffer id).

### Prompts: the shadowing trap has a precise boundary

Stage 0's finding is that a selected candidate shadows typed text. The
boundary is sharper than the parent framing records:
`resolve_accepted_value` (`src/minibuffer.rs:564-574`) **short-circuits
on `CompletionSource::None` at `:565-567` and returns the typed text
before the candidate branch is reached.**

So: **a prompt with no `source` returns typed text verbatim, always.**
Every Stage 2 free-text prompt (a new name, an octal mode, a directory
name) omits `source` and is immune. A prompt that *wants* candidates
re-enters the trap deliberately — which is what §7 is about.

There is no `builtin/runtime/minibuffer.lua`; `pmacs.minibuffer` is
Rust-only. The nearest existing confirm is `autosave.lua:219-224`, a
`source = function() return {"yes","no"} end` prompt tested with
`answer ~= "yes"`.

### What Stage 1 left for Stage 2 to build on

`dired.lua`'s handle is `{buf, path, entries, errors, sort_mode, prev}`
(`:521-528`) — **no mark state; §3 adds it.** `render_entry` (`:334-349`)
hardcodes `BLANK_MARK` in column 0. `paint` (`:369-372`) is a wholesale
`buf:replace` with `bypass_intercept = true`; the read-only intercept
(`:509-511`) rejects everything else. `seat_cursor` (`:405-416`) re-seats
by basename and carries the warning that `move_to_line` is **ambient** —
every post-`await` seat must first check `pmacs.window.buffer()`.
`entry_at_cursor` (`:381-385`) maps cursor line *n* to `entries[n]`,
returning nil on the header and the footer. Keys are mode-scoped through
one helper (`:866-868`), and `pmacs.dired._layout` (`:887-895`) exports
the column contract.

One stale comment to fix in passing: `:665` still says an uncaught raise
lands "in \*errors\*", which #161's COHERENCE finding falsified and
which the module doc at `:56-72` already corrects. Same file, two
answers.

---

## 3. Marks (Q#DR12, continuing the parent's Q#DR4)

`handle.marks` is a table **keyed by basename**, valued by the mark
character: `{ ["foo.txt"] = "*", ["old/"] = "D" }`. Two characters only,
per the parent: `*` (general, consumed by operations) and `D` (deletion
flag, consumed by `x`).

Basename keying is the parent's decision and it is right for the reason
it gives — a sort or a revert reorders lines, so a line-indexed set
retargets onto a different file. Three consequences the parent does not
draw:

- **Marks are per-directory-buffer, which falls out for free.** The mark
  table lives on the handle, and there is one handle per directory
  (Q#DR2). Nothing to decide, but it is why a basename key is
  sufficient — the directory is implied by the table it is in.
- **Marks must be pruned on every re-read**, following
  `*buffer-list*`'s `:444-452`. A vanished name's mark is dropped
  *before* it can be counted by the next operation, so the mark set can
  never name something the listing does not.
- **`render_entry` becomes mark-aware**, which means it needs the mark
  for the entry it is rendering. It currently takes only `entry`; it
  grows a second parameter rather than reaching for the handle, so it
  stays a pure function of its inputs and the acceptance can call it
  directly.

`t` inverts only `*` marks and leaves `D` flags alone — Emacs's
behavior, and the alternative silently converts flags into marks.
`U` clears both.

---

## 4. Target sets: which operations are set-based (Q#DR13)

Rev 1 stated one rule with one exception and then contradicted it (F4).
Operations fall into **three** classes, and the class is a property of
the command, not a special case:

**Set-based** — `D` (delete), `M` (chmod), `w` (copy filename), and `C`
(copy, 2b) target:

> **the marked set, or — if nothing is marked — the entry at point.**

Emacs's rule, and it is why none of them needs a separate at-point
binding.

**Flag-based** — `x` alone. It consumes **`D` flags only**, never `*`
marks, and **never falls back to point**: with nothing flagged, `x` does
nothing and says so. A `d`-then-`x` sequence and an `m`-then-`D` sequence
are different gestures, and collapsing them makes `x` unpredictable after
a stray `m`.

**Point-based** — `R` (rename) **alone**. It acts on the entry at point
**regardless of what is marked**, and leaves marks untouched. This is a
real limitation, not a preference: renaming a *set* means renaming into a
target directory, which needs a concept Stage 2 does not build (§11).
Naming it here is the point — rev 1 left it as an unstated exception to a
rule that claimed to have only one.

*(Rev 2 also put `w` here. Round 2 was right that this was an unapproved
narrowing: the parent approved the `w` **binding**, and Emacs copies the
marked filenames when marks exist. `w` is set-based in rev 3.)*

A basename in a set that has vanished from disk since it was marked is
**dropped from the batch and reported**, not silently skipped and not
fatal to the rest — the parent framing's §6 rule, kept. Note this is a
distinct event from a revert pruning the mark (§3): a target can vanish
*between* the last revert and the operation, and §13 item 35 pins that
the batch reports it rather than silently shrinking.

---

## 5. Rename reconciliation (Q#DR14, superseding Q#DR5's rebind)

The parent's decision — fix it at the primitive, in the main-thread
drain, unconditionally on success — is kept. Rev 1's *scope* was wrong:
it updated `Buffer.file_path` and nothing else, leaving four other owners
of the same path stale (F1). A rename is not a buffer-field update; it is
a **transaction across every consumer that holds the path**.

### The owners, all verified

| Owner | Held as | Stale after a rev-1 rebind |
|---|---|---|
| Buffer path | `Buffer.file_path` | fixed |
| Buffer **name** | `Buffer.name` | **yes** — `set_buffer_path` never calls `set_name`, which documents itself as for "save-as and rename operations" (`buffer.rs:452`). Statusline and buffer list keep the old filename |
| LSP attachment | `rec.uri`, cached per buffer (`lsp.lua:826-833`) | **yes** — didChange (`:418`), semantic tokens (`:776-790`), diagnostics (`:953`), signature (`:1016`), definition (`:1648`), references (`:1757`) all keep firing at the old URI |
| dired handles | `handle.path` in Lua; the buffers are **pathless** | **yes, and unreachable** — no buffer-keyed rebind can ever find them |
| Workspace-edit origin | `origin = active_buffer_path()` (`lsp.lua:1252`) — a **string** (`:471-473`) | **yes, and it materializes a phantom** — `find_or_open(origin)` (`:1266`) on a renamed-away path hits `resolve_target_buffer`'s `NotFound` arm, which creates "an empty path-backed buffer" (`editor_core.rs:876-878`) and selects it. **No transaction can fix this one**, because the stale value is a captured Lua local, not editor state (G1) |

### One transaction, two callers, one notification

**`EditorCore::reconcile_rename(old, new) -> Vec<RenameRebind>`**, where
`RenameRebind { buffer_id, old_path, new_path }`:

- walks the **whole** registry, not `find_by_path`'s first match — a
  directory rename has many affected buffers by construction, and two
  buffers can visit one path;
- matches normalized stored paths against normalized `old` by **equality
  or path-component prefix** (`/foo` must not match `/foobar`);
- sets the new path **and** sets the name, but only when the buffer's
  name still equals its old path — a path-backed buffer is named by its
  full path (Stage 1's finding), while a user-renamed buffer keeps the
  name it was given;
- returns every rebind it performed.

**Both rename paths call it**: the async harvest below, and
`apply_resource_op`'s rename arm (`mod.rs:3234-3255`), whose raw
first-match lookup at `:3249` is deleted in favour of it. One function,
two callers — so the two can no longer drift, which is how the trap
survived being "fixed" once already.

**Then one hook, fired once per rename:
`resource.renamed(old_path, new_path)`.** The mechanism exists —
`run_hook_if_defined` (`mod.rs:1596`) fires Rust-side, `pmacs.hook.run`
(`:5883`) fires Lua-side — but **no rename or delete hook exists today**;
the whole set is `buffer.{after-edit,after-load,after-save,after-switch,
before-save,save,self-insert}`, `editor.before-quit`,
`frontend.detached`, `process.after-tick`. Stage 2 adds this one.

The hook carries the **paths**, not the rebind list, precisely because
dired's buffers are pathless: a path-keyed consumer must be able to
reconcile from `(old, new)` alone.

#### The LSP subscriber, in full (G3)

Rev 2 said "recompute `rec.uri` and didClose/didOpen". That is necessary
and **not sufficient** — five more owners are URI-keyed, and one of them
is not reachable from Lua at all. The contract, in order:

1. **Settle in-flight work first.** Bump/flush any pending `didChange`
   for the old URI before closing it, so the server is not left with an
   edit it can no longer attribute.
2. **`didClose` the old URI.** Note what this does *not* do: it removes
   the open-document registration only. It does not clear the per-`(server,
   uri)` stores.
3. **Drop the old URI's stores explicitly** — `pmacs.diag`,
   `semantic_tokens` (including `result_id`, or the next delta request
   rides a result id the server has forgotten), `signature`, `definition`,
   `references`. Each has a `clear`-shaped entry point already.
4. **Re-run `ensure_server`.** Since #161 affinity keys on the detected
   project root, a rename *across roots* needs a **different server**, not
   a different URI. Same-root renames reuse the existing one.
5. **`didOpen` the new URI** against whichever server step 4 selected,
   with the buffer's current text and a fresh version.
6. **Re-root the diagnostic view for every window showing the buffer.**
   `DiagnosticView.uri` is **set once at construction** and the field's
   own doc anticipates exactly this: *"M5 may add re-rooting if a buffer
   is renamed"* (`src/diag.rs:455-457`). Updating `rec.uri` leaves an
   attached view rendering the **old** URI's diagnostics forever. Either
   the view gains a `set_uri`, or the attachment is torn down and
   `pmacs.diag._attach_view` re-run per window — the framing prefers
   `set_uri` because a teardown loses the view's position in the
   composition stack.

Acceptance 27 is correspondingly stronger: **diagnostics must be present
before the rename**, and afterwards only the **new** URI's diagnostics are
visible and countable, with the old URI's store empty.

#### The dired subscriber, and the setter it needs (G2)

`dired.lua` updates any handle whose `path` equals or is under `old`,
**renames its buffer**, and reverts it. Rev 2 wrote that without noticing
that the module's own doc says it is impossible: *"there is no
`pmacs.buffer.set_name`"* (`dired.lua:34-38`), which is precisely why
Stage 1 chose buffer-per-directory over in-place repaint.

**Rev 3 adds `pmacs.buffer.set_name(buf, name)`** (Q#DR21). Three reasons
it is the right call over the alternative:

- `Buffer::set_name` **already exists** in Rust and already documents
  itself as "used by save-as and **rename operations**"
  (`src/buffer.rs:452`). Nothing new is being invented; an existing,
  purpose-built setter is being exposed.
- §5 needs the same capability anyway for the `Buffer.name` half of the
  transaction, so the Rust-side name update is in scope regardless. The
  Lua binding is the increment, and it is what makes the *pathless* case
  reachable.
- The alternative — kill the dired buffer, recreate it under the new
  name, and replace it in every window showing it — is a far larger
  transaction that loses window placement, the cursor, the read-only
  intercept, `set_round_trip_input`, and the major mode, each of which
  would have to be re-established in the right order.

Uniqueness stays the **caller's** job, matching the Rust setter: dired
reuses `claim_handle`'s existing `<2>`-variant uniquifier before setting.
Acceptance 28 therefore asserts the **buffer name** and that
`handle_for_path` dedup finds the buffer under its new path — not merely
that `handle.path` changed.

#### The workspace-edit applier (G1)

`apply_workspace_edit` captures `origin` as a **string** and restores with
`find_or_open(origin)` (`lsp.lua:1252`, `:1266`). No amount of
reconciliation reaches an already-captured Lua local, so the phantom
survives every design above. The applier itself must change:

- capture the **buffer handle** (`pmacs.window.buffer()`), not the path;
- restore with `pmacs.window.switch_buffer(origin_buf)` when the handle is
  still valid;
- **no path fallback.** If the origin buffer is gone, do nothing — the
  current code's fallback is what creates the phantom, and "return the
  user somewhere plausible" is not worth inventing a file that does not
  exist. The existing comment there already concedes the path "may have
  just been renamed or deleted"; it simply drew the wrong conclusion.

### Ordering, which is already guaranteed

The reconciliation must complete **before any awaiting coroutine
resumes**, or a coroutine that renamed and then inspects a buffer sees
the pre-rename state. Verified: `pmacs._async.tick` calls
`async_mod._tick()` first and only then walks the settled ids firing
`on_complete` callbacks and resuming parked coroutines
(`async.lua:389-391`). So doing the work inside `_tick`'s Rust closure is
correctly ordered by construction, not by luck.

### The harvest (F6)

Rev 1 proposed a no-argument `take_settled_renames()` that drains "this
tick's" renames, which — as the review notes — needs either a second
queue or a scan of every settled entry. Neither was named, and both are
the one-off side channel `COHERENCE.md` §9 objects to.

Instead, **`tick` returns a structured outcome**:

```rust
pub struct TickOutcome {
    pub settled: Vec<JobId>,
    pub renames: Vec<(PathBuf, PathBuf)>,
}
```

Settle identity and rename metadata come out of **one** transaction, from
the loop at `:1074-1108` that already borrows `pending` and reads
`job.kind`. `renames` carries only jobs that settled
`PendingState::Complete` — a failed or cancelled rename reconciles
nothing. The ~17 in-crate `let _ = rt.tick();` call sites are unaffected;
`_tick` reads `.settled` for the Lua table it already builds and
`.renames` for the reconciliation.

`PendingJob` still gains `rename_paths: Option<(PathBuf, PathBuf)>`,
since `dispatch_fs_rename` currently **moves** both paths into the worker
closure (`:871-879`) and nothing retains them. §0.5 states why that field
is the coherent choice over a side map, and that a general
`purpose`/`owner` should later subsume it.

### Additivity

`ReplyKind` and the worker↔main wire are untouched; the new field and the
outcome struct are main-thread-only. `pmacs-protocol` is **not** involved
and does not bump — Stage 1 set that precedent by adding `ReadDir`
without one. `m8_3` **monkeypatches the Lua `pmacs.fs.rename`**
(`m8_3_acceptance.rs:182-184`), so it never reaches the Rust path and its
count must not move; the fixture's two-phase temp-name rename
(`init.lua:1197-1215`) will reconcile twice, real→temp→final, landing
correctly.

---

## 6. Deleting a path something is holding (Q#DR18)

New in rev 2 (F2); given a shared seam and a revalidation rule in rev 3
(G4). `pmacs.fs.remove` changes disk and nothing else, so without a
policy a deleted file's buffer stays bound to a path that no longer
exists — and **saving it recreates the file the user just deleted**.

### There are already two divergent deletion behaviors

Rev 2 wrote dired's policy and stopped, leaving the LSP path untouched
and unmentioned. Verified: `apply_resource_op`'s delete arm
(`mod.rs:3256-3285`) deletes, then kills the buffer via
`reg.borrow().find_by_path(&pb)` — the **same raw-path, first-match**
lookup §5 fixes for rename, with **no descendant handling and no
modified check**. So an LSP-authored delete **destroys unsaved work
today**, silently, and a second buffer on the same path survives.

### One seam

**`EditorCore::reconcile_delete(path) -> DeleteReconcile { killed,
kept_modified }`**, symmetric with `reconcile_rename`:

- walks the **whole** registry by normalized equality **or
  path-component prefix**, so descendants of a deleted directory are
  included and a second buffer on one path is not missed;
- **kills unmodified** buffers;
- **keeps modified ones alive** and returns them, so a caller can report.

**Both paths call it**: the drain harvest for `pmacs.fs.remove`, and
`apply_resource_op`'s delete arm, replacing its first-match lookup.

The **policy split is deliberate and asymmetric**, and this is the part
worth arguing with:

- **dired refuses the whole entry** when a visited buffer is modified —
  the file is never deleted. A direct user gesture on a file with unsaved
  changes should stop, not proceed-and-cope.
- **`apply_resource_op` still deletes the file**, because the delete is
  part of a server-authored workspace edit the user already accepted, and
  refusing mid-edit leaves a half-applied refactor. But it **no longer
  destroys the buffer**: a modified buffer survives the delete with its
  contents. That is strictly better than today and changes no file-side
  behavior.

The residue — an LSP-driven delete can still orphan a modified buffer —
is **named, not fixed** (§11). Fixing it properly means deciding what a
partially-applied workspace edit does, which is a larger question than
dired.

### Deletion is harvested, not hand-fired (G4)

Rev 2 left this ambiguous, and the two options really do produce
different primitive contracts. Rev 3 chooses the one symmetric with
rename: **`remove` is harvested in the drain**, via the same
`TickOutcome`, firing **`resource.deleted(path)`**. `PendingJob` retains
the path for `JobKind::FsRemove` exactly as for `FsRename`.

The reason is the same as Q#DR14's: a **fire-and-forget** `pmacs.fs.remove`
must reconcile too, and dired firing the hook itself after its own
`:await()` would protect only dired. The policy (what to delete) is
dired's preflight; the mechanism (what to reconcile once the syscall
lands) is the primitive's.

### The four cases

| Case | Policy |
|---|---|
| Unmodified visited file | Delete, then kill the buffer |
| **Modified** visited file | **Refuse that entry.** Report it; the rest of the batch proceeds |
| Open buffers *under* a deleted directory | Same two rules, applied to every buffer whose path is under it |
| Open dired handles on the path or under it | Killed, via `resource.deleted` |

**Refusing on modified is a deliberate divergence from Emacs**, which
deletes and leaves an orphaned buffer. The reasoning: the orphan is
indistinguishable from a normal buffer, and the next `C-x C-s` silently
resurrects the file. A refusal is visible, recoverable, and the user can
save-or-discard and retry.

### Checked twice, because a prompt is not a lock (G4)

`buf:is_modified()` is exposed to Lua (`mod.rs:1234`), so dired decides
this itself. It must decide **twice**:

- **Before the confirm**, so the prompt can state the skip up front:
  `Delete 3 entries? (1 has unsaved changes and will be skipped) (y/n) `.
- **Again immediately before each syscall.** The prompt is not modal
  against the world: another frontend attached to the same daemon can
  edit a buffer while it is open, and the batch is serialized (§9) so
  there is a real window between the answer and the *n*-th removal. A
  buffer that became modified in that window is skipped and reported, and
  §13 item 20 pins exactly that interleaving.

## 7. Confirmation (Q#DR15)

Destructive operations confirm: `x`, `D`, and (in 2b) a recursive delete
and an overwriting copy. There is no helper to do it with (C4).

**Stage 2 adds `builtin/runtime/minibuffer.lua` defining
`pmacs.minibuffer.confirm { prompt, on_yes }`**, loaded before
`dired.lua` in `editor.rs`'s explicit sequence.

It takes **no completion source**, and that is the decision, not an
omission:

- With no source, `resolve_accepted_value` returns typed text verbatim
  (`:565-567`), so the answer is exactly what the user typed and the
  shadowing question does not arise.
- Affirmative is `y` or `yes`, case-insensitively. **Everything else,
  including an empty `RET`, is no.** A destructive prompt must fail
  closed.
- The alternative — `autosave.lua`'s two-candidate source — happens to
  be safe today only by lexicographic luck. `fuzzy_score` returns
  `Some(0)` for an empty needle (`minibuffer.rs:638-640`) so **both**
  candidates match, and `filter_and_sort` breaks the score tie on
  `a.1.cmp(b.1)` — plain ascending string order (`:678`). `"no" < "yes"`,
  so `selected = Some(0)` resolves to `"no"`. The right default, reached
  by an accident of spelling: the safe answer wins **only because it
  happens to sort first**. Relabel the pair and it inverts — `{"delete",
  "keep"}` or `{"apply", "cancel"}` both put the *destructive* answer at
  index 0 and make an empty `RET` execute. (Note it is spelling, not list
  order, that decides: the tie-break is lexicographic, so writing
  `{"no", "yes"}` changes nothing.) Not a foundation for four new
  destructive call sites.

The prompt states the count and the operation — `Delete 3 marked
entries? (y/n) ` — because a confirmation that does not say what it is
confirming is decoration.

**Migrating `autosave.lua` to the helper is a named follow-up (§11), not
part of this stage.** Both shapes already answer "no" to an empty `RET`,
so the migration is behavior-preserving; it still touches the
crash-recovery prompt, which does not belong in a dired PR.

---

## 8. The operations, individually

**`d` / `x` (flag and execute).** `d` sets `D` and advances. `x` collects
`D`-flagged basenames, applies §6's visited-path policy, confirms with the
count, then deletes serially (§9) via `pmacs.fs.remove`, which handles
files and empty directories (C5). A non-empty directory fails with the
kernel's `ENOTEMPTY`, **reported as such** in 2a; 2b's `remove_dir_all`
plus `dired.recursive-deletes` addresses it. Then revert once.

**`D` (delete now).** Same deletion path and same §6 policy, targeting
the §4 set rather than the flags.

**`R` (rename).** The entry **at point**, regardless of marks (§4).
Prompts with `initial` = the current basename and **no source**, so the
typed name arrives verbatim (`minibuffer.rs:565-567`). A bare name
resolves against `handle.path`; a name containing `/` is a path, relative
to `handle.path` unless absolute. Reconciles every path owner by §5,
including on a directory.

> **The no-clobber guarantee is a preflight, and rev 3 says so (G6).**
> `rename_blocking` calls plain `std::fs::rename` (`fs.rs:492-499`),
> which on Unix **silently replaces** an existing target. Dired stats the
> destination and refuses if it exists, but that check and the syscall are
> not atomic: a target created in between is overwritten. Rev 2's
> "refuses an existing target" overstated a **best-effort, TOCTOU-bounded
> refusal**, and acceptance 12 is reworded to promise only what is
> delivered. Closing it needs a no-replace primitive —
> `renameat2(RENAME_NOREPLACE)` on Linux, `renamex_np` on macOS, with a
> link/unlink fallback elsewhere — which is a portability question of its
> own and is deferred (§11).

**`w` (copy filename).** Carried forward from the parent's approved
table, and **set-based** (§4): with marks, it copies every marked name,
newline-separated; with none, the entry at point. Non-destructive, no
confirm.

> **It needs a public kill-ring entry point, which does not exist (G5).**
> `push_entry` is a `local function` (`killring.lua:93`) and the public
> `copy()` requires a region — it calls `ed.region()` and fails with "no
> region" otherwise (`:182-190`). Rev 3 adds
> **`pmacs.killring.push(text)`** (Q#DR22) with exactly the semantics
> `copy()` already establishes for non-kill text: `push_entry`, mirror to
> the OS clipboard via `ed.clipboard_set`, and **break the kill chain**
> (`fail_kill`), because a filename copy is not an appendable kill and
> must not merge into an adjacent `C-k` run.

**`M` (chmod). New scope — needs explicit approval (F3).** Prompts for an
**octal** mode string, no source, validated to `[0, 07777]` before
dispatch to match `fs.chmod`'s own guard (`fs.lua:181-183`). Applies to
the §4 set.

> **It refuses symlink entries.** `chmod` follows symlinks
> (`fs.lua:145-153`) while the listing is lstat-based, so chmodding a
> symlink line silently changes a *different file's* mode and the
> refreshed listing shows the link's own unchanged bits — the change
> appears to have done nothing. The parent framing already decided this
> for the wdired surface: the fixture "rejects symlink perms edits at
> intercept time for exactly this reason", and that decision "carries
> over unchanged" (`dired-framing.md:543-546`). Rev 1 proposed warning
> after the fact instead, which is materially different and contradicted
> approved text. A refusal is reported per entry and does not abort the
> batch.

**`+` (create directory, 2b).** Prompts for a name, no source, resolved
against `handle.path`. `opts.parents` for `create_dir_all`.

**`C` (copy, 2b) — full command flow (F7).** Targets the §4 set.

1. **Destination prompt.** One source prompts for a destination path;
   **several sources require an existing directory** and the command
   refuses before dispatching anything if the answer is not one.
2. **Collision scan, before any copy.** Resolve every source to its
   destination path and stat each. This happens up front so the user is
   asked once, not once per file mid-batch.
3. **Confirm.** With no collisions and one source, no prompt — a copy
   onto free space is not destructive. With collisions, **one** confirm
   naming the count: `Overwrite 2 existing files? (y/n) `. Declining
   **skips the colliding entries and copies the rest**, rather than
   abandoning the batch, which matches §9's per-entry failure rule.
4. **Dispatch** with `opts.overwrite` set only for the entries the user
   confirmed. The primitive still refuses an existing target without it,
   so the guard is enforced at both layers.
5. **A directory source is refused** — the parent's decision, and the
   honest one while no recursive-copy primitive exists.

Mode bits are preserved.

## 9. Execution: serialize, report, then revert (Q#DR16)

`fs.lua:155-165` instructs packages needing at-most-one-pending
mutation semantics to *"await each op before dispatching the next"*.
Stage 2 does that, for three reasons beyond obedience: a batch is
user-initiated and small; interleaved failures are attributable to a
specific entry; and awaiting reaps the pending entry, keeping dired out
of the leak in §2.

The contract:

- **One coroutine per batch**, `pmacs.async`, awaiting each op in turn.
- **A per-entry failure does not abort the batch.** Collect
  `{basename, message}`, continue.
- **One status line at the end**, naming counts:
  `dired: deleted 2, failed 1 (report.log: Directory not empty)`. Not one
  status per entry — the last would be the only one visible.
- **Marks consumed by a successful operation are cleared; marks on
  entries that failed are kept**, so `x` again retries exactly the
  failures.
- **Revert once, after the batch**, not per entry: the listing is a
  wholesale repaint and N repaints for N deletions is both slower and
  visibly wrong.
- **Every post-`await` re-seat checks `pmacs.window.buffer()` first**
  (`dired.lua:399-404`), the rule #165's review round produced.

---

## 10. Staging: 2a then 2b (Q#DR17)

**Recommendation: split, and cut it at "needs a new Rust primitive".**

| | 2a | 2b |
|---|---|---|
| Keys | `m u U t d x D R w M` | `+ C`, recursive delete |
| New `pmacs.fs` ops | **none** | `mkdir`, `copy`, `remove_dir_all` |
| New `JobKind` variants | none | 3 (12 → 15) |
| Rust | rename **transaction** + two hooks + `apply_resource_op` fix | three primitives |
| Config keys | none | `dired.recursive-deletes` |

The cut works because of C5: `remove` already deletes files and empty
directories, so 2a's whole surface runs on the five ops that exist. That
makes 2a *"the mark-and-operate layer, plus the resource-reconciliation
transaction"* and 2b *"three additive primitives and the two ops that need
them"* — two PRs a reviewer can hold one at a time.

Why not one PR: 2a's Rust is no longer small — after rounds 1 and 2 it
is a rename **and** delete reconciliation, two hooks, a buffer-name
setter, a kill-ring entry point, an LSP teardown/re-attach contract, and
a change to the workspace-edit applier. That is exactly why it must not
also carry three new fs primitives: the reviewer who should be
scrutinizing the drain and the LSP contract would also be checking
`copy`'s overwrite semantics. The arc has already shown what that costs —
#165 was one round because its Rust was one narrowly-scoped change.

**If 2a still looks too large after that list, the natural further cut is
along the same line**: the reconciliation transaction (rename + delete +
hooks + LSP + applier) is a self-contained substrate correctness fix with
no dired surface at all, and could land before the mark layer. It is
listed here rather than chosen, because it trades one review round for
two.

Why not three PRs: the mark layer with no operation to consume it ships
nothing a user can do, and a marks-only PR would have to invent
throwaway acceptance for state no command reads.

**2a is the approval-critical one.** If the split is rejected, the
combined PR is the same content in the same order and this framing still
applies; §13's acceptance is already labelled by stage.

---

## 11. Deferred (named)

- **`wdired`** — Stage 3, with the frozen fixture as its reference (C3).
- **The fire-and-forget pending-entry leak** (§2). Pre-existing, verified,
  orthogonal, observable as `pending_len` growth. Needs its own lane; the
  candidate fix is a settled-entry sweep with a reap policy, which is a
  decision about handle lifetime, not a patch.
- **A general `purpose`/`owner` field on `PendingJob`**, per
  `COHERENCE.md` §9, which should subsume §5's `rename_paths`.
- **Migrating `autosave.lua` to `pmacs.minibuffer.confirm`** (§7).
- **Multi-file `R` into a target directory**, and `%`-regexp marking —
  both need a target/pattern concept Stage 2 does not build. This is why
  `R` is point-based in §4 rather than an unstated exception.
- **Symbolic chmod** (`u+x`), needing a mode-expression parser.
- **`i` (insert subdirectory)** — the recursive in-buffer case, already a
  named deferral in `docs/dired-framing.md` §13, and the place a shared
  tree primitive (`COHERENCE.md` §14) would land.
- **`!` shell command on marks**, compress, symlink, hardlink.
- **A no-replace rename primitive** (G6) — `renameat2(RENAME_NOREPLACE)`
  on Linux, `renamex_np` on macOS, link/unlink elsewhere. Until then `R`'s
  refusal is a TOCTOU-bounded preflight, which §8 states plainly.
- **An LSP-driven delete can still orphan a modified buffer** (G4).
  Stage 2 stops it *destroying* one, but the file still goes. Fixing it
  means deciding what a partially-applied workspace edit does.
- **The rooturi sink's weak wait predicate** (`m4_acceptance.rs:5487`) —
  same class as the config-sink race fixed in #174, not observed failing,
  and the obvious fix would trade a precise regression diff for a vague
  timeout. Needs a record terminator in the fake server first.
- **Recursive copy** — 2b refuses directory sources; a real `copy -r`
  primitive is separate.

---

## 12. Bets

- **B1.** 2a needs **no** new `pmacs.fs` op. Falsified if any of
  `m u U t d x D R w M` cannot be built on the existing five. *(Rests on
  C5, which was read off `remove_blocking` directly.)*
- **B1b.** The two new hooks need no new hook machinery —
  `run_hook_if_defined` (`mod.rs:1596`) and `pmacs.hook.run` (`:5883`)
  already exist, and `resource.renamed`/`resource.deleted` are ordinary
  names in that registry. Falsified if firing a hook from the reconcile
  path needs a new dispatch mechanism.
- **B2.** The rename rebind needs **no** protocol bump and leaves the
  worker↔main wire untouched. Falsified by any change to `ReplyKind` or
  `SUPPORTED`.
- **B3.** The frozen `m8_1`/`m8_2`/`m8_3` counts are unchanged by the
  reconciliation. *Not obvious, and less obvious in rev 2:* the fixture
  renames files it lists, so a test holding a buffer on a renamed path
  would newly see its path **and now its name** move, and `m8_3`
  monkeypatches the Lua `rename` so it never reaches the Rust path at
  all. The additivity gate is a real check, not a formality.
- **B3b.** No existing LSP test changes behavior. *Not obvious:* the
  `resource.renamed` subscriber issues didClose/didOpen and may re-run
  `ensure_server`, so any suite that renames a file with a server
  attached is in the blast radius. `m4`/`lsp_multi_root` counts are
  gated for exactly this reason.
- **B4.** No GPU or TUI frontend change. Marks are buffer text and the
  keymap is mode-scoped; `set_round_trip_input` is already set by Stage 1.
- **B5.** `describe_key_identifies_every_default_binding` stays green
  without further surgery — #165 already taught it per-binding mode
  context, and Stage 2 only adds more bindings in the same mode.

---

## 13. Acceptance

**Stage 2a — marks**

1. `m` marks the entry at point and advances; the mark renders at
   `_layout.MARK_START` and **no other column moves** (asserted against
   the exported layout, not hardcoded offsets).
2. `u` clears and advances; `U` clears every mark; `t` inverts `*` and
   leaves `D` untouched.
3. Marks survive a **sort** (`s`) — the basename-keyed set must follow
   its entries to their new lines.
4. A mark on a basename that vanishes is **pruned by a revert** and is
   absent from the next batch.
5. Marks are per-buffer: two dired buffers on two directories keep
   independent sets.
6. **`R` with unrelated marks present** renames the entry at point, not
   the marked set, and **leaves every mark intact** (F4 — pins that §4's
   point-based class is real and not an accident).

**Stage 2a — operations**

7. `d` then `x` deletes the flagged file, after a confirm; declining
   deletes nothing.
8. `x` consumes `D` flags **only** — a `*`-marked entry survives it —
   and with nothing flagged it is a reported no-op.
9. `D` with nothing marked targets the entry at point (§4).
10. `x` on an empty directory succeeds; on a **non-empty** directory it
    fails, the message names the entry, and **the rest of the batch still
    runs**.
11. A batch with one failure reports both counts in one status line, and
    the failed entry **keeps its mark** while the successful one loses it.
12. `R` renames; a bare name resolves against the listing's directory;
    a target **observed to exist at preflight is refused** — the honest
    contract (G6), since `std::fs::rename` would replace one appearing
    afterwards.
13. `M` applies an octal mode to the marked set; an out-of-range mode is
    refused before dispatch.
14. **`M` refuses a symlink entry** (Q#DR19, F3/F5): the mode is
    reported unchanged, the target file's mode is **asserted untouched**,
    and the batch continues. *(A warning-after-the-fact implementation
    fails this.)*
15. `w` copies the **marked** filenames to the kill ring,
    newline-separated, and the entry at point when nothing is marked;
    the OS clipboard mirrors it and the **kill chain is broken**, so a
    following `C-k` does not append to it.
16. The listing reverts **once** after a batch, not per entry.

**Stage 2a — deletion policy (§6, F2)**

17. Deleting an **unmodified** visited file kills its buffer.
18. Deleting a **modified** visited file is **refused**; the buffer
    survives with its contents, and the file is still on disk.
19. The confirm prompt **states the skip before the user answers**, not
    after.
20. **A buffer modified after the prompt appears but before `y`** is
    skipped and reported (G4): the check is re-run immediately before
    each syscall, because another frontend can edit during the prompt and
    the batch is serialized. *(An implementation that checks only once,
    up front, fails this.)*
21. Deleting a directory kills buffers on its **descendants**.
22. An open dired handle on a deleted directory is closed
    (`resource.deleted`).
23. **`apply_resource_op`'s delete** no longer kills a **modified**
    buffer, and now reaches **descendants** and a **second buffer on the
    same path** (G4 — today it is raw-path first-match with no modified
    check, so it destroys unsaved work).
24. A **fire-and-forget** `pmacs.fs.remove` reconciles too — never taking
    the handle still kills the unmodified buffer, which is what makes the
    drain harvest the right seam rather than dired firing the hook.

**Stage 2a — rename reconciliation (§5, F1)**

25. **No-await rename**: dispatch `pmacs.fs.rename`, never take the
    result, pump — the open buffer's path has moved. *(Fails if the
    reconciliation lives at result-consumption.)*
26. **Directory rename**: a buffer open on `dir/child.txt` follows
    `dir` → `newdir`.
27. **Every match, not the first** (F5): **two** descendant buffers under
    the renamed directory **and two buffers visiting the same exact
    path** all move. *(One child buffer does not defeat a first-match
    implementation; this does.)*
28. **False prefix**: renaming `/…/foo` does **not** rebind a buffer on
    `/…/foobar`.
29. **Buffer name** follows the path, so the buffer list and statusline
    show the new filename — and a buffer the user renamed by hand keeps
    its own name.
30. **An attached LSP buffer with diagnostics present before the
    rename**: afterwards only the **new** URI's diagnostics are visible
    and countable, the old URI's store is empty, and the **attached
    diagnostic view renders the new URI** — not merely `rec.uri` updated
    (G3; `DiagnosticView.uri` is set once at construction).
31. A rename **across project roots** re-runs `ensure_server` and the
    buffer ends up attached to a **different** server; a same-root rename
    reuses the existing one (#161's affinity key).
32. **An open dired buffer on the renamed directory** follows it: its
    `handle.path`, **its buffer name** (`*dired:<new path>*`), and
    `handle_for_path` dedup under the new path all move together (G2 —
    asserting `handle.path` alone would pass with the name still stale).
33. **The workspace-edit origin**: renaming the *active* file through
    the full `apply_workspace_edit` path leaves **no phantom empty
    buffer** at the obsolete path, and the user is returned to the
    **same buffer** (now under its new path). *(G1 — this is a change to
    the applier, which must capture the buffer handle; no reconciliation
    can reach the string it captures today.)*
34. When the origin buffer is **gone** after the edit, the applier
    restores nothing rather than falling back to the old path.
35. `apply_resource_op`'s rename finds a buffer whose stored path is
    normalized but whose op names it un-normalized (the `:3249` fix).
36. A **failed** rename reconciles nothing.
37. Additivity: `m8_1`, `m8_2`, `m8_3` at unchanged counts.

**Stage 2a — the shared helpers**

38. **`pmacs.minibuffer.confirm`** (Q#DR15, F5): an **empty `RET` does
    not call `on_yes`**; `y`, `Y`, `yes`, `YES` all do; `n` and arbitrary
    text do not. *(A typed-`n` test alone would not catch a completion
    source being reintroduced — the empty-`RET` arm is the one that
    detects it.)*
39. **Serialization** (Q#DR16, F5): in a batch of N mutations, the second
    is **not dispatched until the first has settled**. Asserted by
    observing at most one in-flight fs job at any pump step — *not* by
    the end state, which is identical if all N were dispatched at once
    and awaited afterwards.
40. A marked target that vanishes **between the last revert and the
    operation** is **reported**, not silently dropped (F5 — distinct from
    item 4's revert-time pruning).

**Stage 2b**

41. `+` creates a subdirectory, which appears on the next listing.
42. `C` copies a file and preserves mode bits; refuses a directory
    source.
43. `C` with several marked entries requires an existing directory
    destination and refuses otherwise **before copying anything**.
44. `C` onto existing targets confirms **once** with the collision count;
    **declining copies the non-colliding entries and skips the rest**
    (F7).
45. `remove_dir_all` **unlinks a symlink-to-a-directory rather than
    traversing it** — pinned at the primitive, mirroring
    `remove_blocking`'s lstat guard (F7).
46. Recursive delete happens only with `dired.recursive-deletes` enabled
    **and** a confirm; disabled, the non-empty directory still fails.

**Bite obligations.** Each of these must fail against a stated mutation:

| Item | Mutation it must catch |
|---|---|
| 8 | `x` widened to consume `*` marks |
| 14 | `M`'s symlink refusal downgraded to a warning |
| 20 | the modified check run only once, before the prompt |
| 25 | the reconciliation moved to `_take_result` |
| 27 | `find_by_path`'s first match instead of every match |
| 28 | a string `starts_with` instead of a path-component prefix |
| 30 | `rec.uri` updated without re-rooting the diagnostic view |
| 32 | `handle.path` updated without the buffer name |
| 33 | the applier restoring by path instead of by buffer handle |
| 38 | a completion source added to `confirm` |
| 39 | the batch changed to dispatch-all-then-await |

`dired.lua` is an existing file now, so `scripts/bite`'s
swap-over-`git show` mode applies — but per #165's lesson, **commit
before biting**. Items 30, 32, and 33 are the round-2 additions, and each
one is a case where rev 2's design would have passed a weaker test.

## 14. Gates (per PR)

The standard suite from `CLAUDE.md`, plus what this work touches:
`cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D
warnings` as its own step; `cargo test --lib` and `--lib --features
crdt`; `dired_acceptance` (default **and** `crdt`); **`m8_1`, `m8_2`,
`m8_3` at unchanged counts** (the additivity gate, B3);
`m4_acceptance -- --skip basedpyright`; **`lsp_multi_root_acceptance`**
(B3b's gate, omitted from rev 2's list); `PMACS_REQUIRE_GPU=1 cargo test
-p pmacs-gpu`; the isolated-`XDG_CONFIG_HOME` workspace sweep with
`--no-fail-fast`; `git diff --check`.

---

## 15. Numbered decisions

- **Q#DR12** Marks are a per-handle table **keyed by basename**, values
  `*` or `D`, pruned on every re-read following `*buffer-list*`'s
  precedent. `render_entry` takes the mark as a second argument. (§3)
- **Q#DR13** *(narrowed in rev 2, F4; `w` moved in rev 3, G5)*
  Operations fall into three classes: **set-based** (`D`, `M`, `w`, `C`)
  target the marked set or the entry at point; **flag-based** (`x`)
  consumes `D` flags only and never falls back to point; **point-based**
  (`R` alone) acts at point regardless of marks and leaves marks
  untouched. A vanished basename is dropped and reported. (§4)
- **Q#DR14** *(widened in rev 2, F1)* A rename is a **transaction across
  every path owner**, not a buffer-field update.
  `EditorCore::reconcile_rename` walks the whole registry, matches by
  equality or **path-component** prefix, and updates both `file_path`
  **and** `name`; **both** `pmacs.fs.rename`'s drain and
  `apply_resource_op` call it, so they cannot drift. A new
  **`resource.renamed(old, new)`** hook then lets path-keyed Lua
  consumers reconcile — `lsp.lua` recomputes `rec.uri`, re-runs
  `ensure_server` for a cross-root move, and issues didClose/didOpen;
  `dired.lua` follows its handles. `tick` returns a **structured
  `TickOutcome`** so settle identity and rename metadata stay in one
  transaction (F6). Ordering is guaranteed: `_tick` runs before any
  coroutine resumes. Pinned by a **no-await** rename.
  *(Widened again in rev 3, G1/G3:* the LSP subscriber owns a full
  teardown/re-attach — flush pending `didChange`, `didClose`, **drop all
  five URI-keyed stores**, re-run `ensure_server`, `didOpen`, and
  **re-root `DiagnosticView`**, whose URI is set once at construction;
  and `apply_workspace_edit` must capture the **buffer handle** rather
  than the path string, restoring with `switch_buffer` and **no path
  fallback**, since no transaction can reach a captured Lua local.*) (§5)
- **Q#DR15** Confirmation is `pmacs.minibuffer.confirm` in a new
  `builtin/runtime/minibuffer.lua`, with **no completion source**;
  affirmative is `y`/`yes` case-insensitively and everything else —
  including empty `RET` — is no. (§7)
- **Q#DR16** Batches **serialize** (await each op), a per-entry failure
  does not abort, successful marks clear while failed marks persist, and
  the listing reverts **once** at the end. (§9)
- **Q#DR17** Stage 2 **splits into 2a and 2b** at the "needs a new Rust
  primitive" line: 2a is the mark layer plus `d x D R w M` on the
  existing five ops plus the rename transaction; 2b adds
  `mkdir`/`copy`/`remove_dir_all` and `+ C` and recursive delete. (§10)
- **Q#DR18** *(new in rev 2, F2; given a seam in rev 3, G4)* Deleting a
  path something holds. **One shared `EditorCore::reconcile_delete`**,
  symmetric with `reconcile_rename` — whole registry, equality or
  path-component prefix, kills unmodified buffers and **keeps modified
  ones** — called by both the drain harvest and `apply_resource_op`,
  replacing the latter's raw first-match lookup. `remove` is **harvested
  in the drain** like rename, firing **`resource.deleted(path)`**, so a
  fire-and-forget remove reconciles too. The **policy** is deliberately
  asymmetric: dired **refuses the whole entry** when a visited buffer is
  modified, while an LSP-authored delete still removes the file (the user
  accepted the refactor) but **no longer destroys the buffer**. The
  modified check runs **before** the confirm *and again immediately
  before each syscall*, because another frontend can edit while the
  prompt is open. Deliberately diverges from Emacs, which orphans the
  buffer and lets the next save resurrect the file. (§6)
- **Q#DR19** *(new in rev 2, F3)* `M` (chmod) is **new scope** beyond the
  parent's approved table and needs explicit approval. It **refuses
  symlink entries**, because `chmod` follows links while the listing is
  lstat-based, so the operation would change a different file and appear
  to do nothing — the same reasoning the parent already applied to the
  fixture's wdired perms edits and said "carries over unchanged". (§8)
- **Q#DR20** *(restored in rev 2, F3; corrected in rev 3, G5)* `w` (copy
  filename to the kill ring) is carried forward from the parent's
  approved Stage 2 table, which rev 1 dropped without saying so. It is
  **set-based** — the parent approved the binding, and Emacs copies the
  marked filenames; rev 2's point-only narrowing was an unapproved change
  of its own. Non-destructive. (§8)
- **Q#DR21** *(new in rev 3, G2)* **`pmacs.buffer.set_name(buf, name)`**
  is exposed. `Buffer::set_name` already exists in Rust and already
  documents itself as for "save-as and **rename operations**"; §5 needs
  the same capability for the `Buffer.name` half of the transaction; and
  without it dired's **pathless** buffer cannot follow a directory
  rename, which its own module doc says is why buffer-per-directory
  exists. Uniqueness stays the caller's job, matching the Rust setter.
  (§5)
- **Q#DR22** *(new in rev 3, G5)* **`pmacs.killring.push(text)`** is
  exposed, with the semantics `copy()` already establishes for
  non-region text: push the entry, mirror to the OS clipboard, and
  **break the kill chain**. `push_entry` is private and `copy()` requires
  a region, so `w` has no surface without it. (§8)

## 16. Branch and PR plan

Framing on `dired-stage2-framing`, kept after merge per the repo's
`-framing` convention. Implementation on `dired-stage2a` cut fresh from
`main` after the framing lands, then `dired-stage2b` cut from `main`
after 2a merges — **not** stacked, since 2b needs nothing from 2a's diff
beyond a merged `main`, and the arc has already paid for a stacked
retarget once (#104 → #105).

One feature, one branch, one PR; gates green before each PR; the ledger
lane and `docs/agent-handoff.md` §1 updated per their own protocols as
each lands.

**Ledger note:** this framing branch deliberately touches **only** this
file. `docs/active-work.md` and `docs/agent-handoff.md` are held by the
open docs PR #169, and a second edit to them here would conflict for no
benefit; the Stage 2 lane goes in once #169 has landed.
