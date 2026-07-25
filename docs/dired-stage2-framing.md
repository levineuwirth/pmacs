# Dired Stage 2 — marks and operations — framing

**Revision 1 — 2026-07-25. Status: PROPOSED, awaiting review.**
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

## 0. What changed from the parent framing

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
  at all — `pmacs.minibuffer` is entirely Rust-provided. §6 decides where
  the helper lives.
- **C5.** The parent's list of three missing primitives is correct, and
  `remove` covers more than it implies: `remove_blocking`
  (`src/fs.rs:561-583`) already deletes **files and empty directories**,
  and correctly unlinks a symlink-to-a-directory rather than following it.
  So `remove_dir_all` is needed only for **non-empty** directories. This
  is what makes the staging cut in §9 possible.

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
  the *reason* §6 spends a section on it.
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
| `R` | `dired.do-rename` | Rename the entry at point |
| `M` | `dired.do-chmod` | Change mode bits on the marked set (or entry at point) |
| `+` | `dired.create-directory` | Create a subdirectory (**2b**) |
| `C` | `dired.do-copy` | Copy the marked set (or entry at point) (**2b**) |

Plus, invisibly: **`pmacs.fs.rename` starts rebinding open buffers**
(§5) — a correctness fix to shared substrate that is the reason `R` is
safe on a directory.

Not in Stage 2: `wdired` (Stage 3), subdirectory insertion (`i`),
shell commands on marks (`!`), regexp marking (`% m`), and
compress/symlink/hardlink ops. §10 names them.

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
  each op before dispatching the next)."* §8 takes that instruction
  literally.
- `chmod` **follows symlinks** (`fs.lua:145-153`) while `read_dir`/`stat`
  lstat. So `M` on a symlink line changes the *target's* mode and a
  refresh shows the link's own, unchanged mode. This is documented
  substrate behavior, not a bug to fix here; §7 decides what `M` does
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
routine. §8's serialize-and-await contract means dired's own ops reap
every entry they create; the rebind exists for *other* callers, who leak
today regardless. Named as a deferral (§10) with the note that
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
re-enters the trap deliberately — which is what §6 is about.

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

## 4. Batch semantics: what "the marked set" means (Q#DR13)

Every operation resolves its target set the same way, and this is the
one rule that makes the whole surface predictable:

> **The marked set, or — if nothing is marked — the entry at point.**

Emacs's rule, and it is why `D` and `M` need no separate "at point"
binding. `x` is the exception: it consumes **`D` flags only**, never
`*` marks, and never falls back to point. A `d`-then-`x` sequence and a
`m`-then-`D` sequence are two different gestures and collapsing them
would make `x` unpredictable after a stray `m`.

A basename in the set that has vanished from disk since it was marked is
**dropped from the batch and reported**, not silently skipped and not
fatal to the rest — the parent framing's §6 rule, kept.

---

## 5. The rename rebind (Q#DR14, continuing Q#DR5)

The decision is the parent's: rebind **at the primitive**, in the
main-thread drain, unconditionally on success, because the fs ops are
fire-and-forget-capable and a rebind hung off result-consumption would
miss every rename whose handle is never taken. What this framing adds is
where it can actually live and what it must be careful about.

### The split (C1)

**`AsyncRuntime` harvests; the Lua binding rebinds.**

1. `PendingJob` gains `rename_paths: Option<(PathBuf, PathBuf)>`, set
   only by `dispatch_fs_rename`, which currently moves both paths into
   the worker closure and must clone them for the field.
2. `AsyncRuntime` gains `take_settled_renames(&self) -> Vec<(PathBuf,
   PathBuf)>`, which drains the paths of jobs that settled
   `PendingState::Complete` this tick. It **takes** (leaving `None`), so
   a rebind can never be applied twice, and it filters on success —
   a failed or cancelled rename rebinds nothing. The natural
   implementation collects during the existing `:1074-1108` post-loop
   block, which already borrows `pending` and reads `job.kind`.
3. `pmacs._async._tick` (`mod.rs:6911-6924`) calls it after `rt.tick()`,
   resolves `SharedCore` via `lua.app_data_ref::<SharedCore>()` the way
   `apply_resource_op` does (`:3251`), and applies the rebind.

This keeps `AsyncRuntime` free of buffer knowledge — it returns paths,
not decisions — and puts the editor-side effect in the layer that
already has the editor.

### What the rebind does

- **Path-component prefix, not string prefix.** Renaming `/tmp/foo` to
  `/tmp/bar` rebinds `/tmp/foo` and `/tmp/foo/a.txt`, and must **not**
  touch `/tmp/foobar`. This is the whole point of the widening — `R` on
  a directory is an ordinary dired operation and `find_by_path` strands
  every buffer beneath it today.
- **Normalize before lookup**, through `find_buffer_for_path`
  (`editor_core.rs:864-867`) rather than a raw `find_by_path`, because
  stored paths are normalized on write (`:819`).
- **Every match, not the first.** `find_by_path` returns one id; the
  rebind needs to walk the registry, because a directory rename has many
  affected buffers by construction.
- **`apply_resource_op`'s raw first-match lookup (`mod.rs:3249`) is
  fixed in the same change.** It is the same bug one call site away, on
  the LSP-facing path, and leaving it is how the trap survives a fix
  aimed at it.

### The acceptance that makes this real

An acceptance that awaits the rename would pass against a rebind wired
anywhere, including the wrong place. So Stage 2 pins:

- **a no-await rename** — dispatch, never take the result, pump, assert
  the open buffer's path moved. This is the test that fails if the
  rebind is hung off `_take_result`.
- **a directory rename with an open child buffer** — the prefix case.
- **a `/tmp/foobar` sibling** — the false-prefix case, which a naive
  `starts_with` on strings passes and must not.
- **a failed rename** — nothing rebinds.

Bite target: move the rebind from the drain to `_take_result` and the
no-await test must fail.

### Additivity

`ReplyKind` and the wire between worker and main are untouched; the new
field is main-thread-only state. `pmacs-protocol` is **not** involved and
does not bump — Stage 1 set that precedent by adding `ReadDir` without
one. The frozen `m8_1`/`m8_3` suites must stay at their current counts,
and `m8_3`'s monkeypatch means it never reaches the Rust path at all.

---

## 6. Confirmation (Q#DR15)

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

**Migrating `autosave.lua` to the helper is a named follow-up (§10), not
part of this stage.** Both shapes already answer "no" to an empty `RET`,
so the migration is behavior-preserving; it still touches the
crash-recovery prompt, which does not belong in a dired PR.

---

## 7. The operations, individually

**`d` / `x` (flag and execute).** `d` sets `D` and advances. `x` collects
`D`-flagged basenames, confirms with the count, then deletes them
serially (§8) via `pmacs.fs.remove`, which handles files and empty
directories (C5). A non-empty directory fails with the kernel's
`ENOTEMPTY`, which is **reported as such** in 2a and is what 2b's
`remove_dir_all` plus `dired.recursive-deletes` addresses. Then revert.

**`D` (delete now).** Same deletion path, targeting the §4 set rather
than the flags.

**`R` (rename).** Single entry at point only — a multi-file
rename-into-a-directory needs a target-directory concept Stage 2 does
not build (§10). Prompts with `initial` = the current basename and **no
source**, so the typed name comes through verbatim. A bare name resolves
against `handle.path`; a name containing `/` is taken as a path, relative
to `handle.path` if not absolute. Refuses an existing target rather than
clobbering it — `rename(2)` would silently replace a file, and dired must
not. Rebinds open buffers by §5, including on a directory.

**`M` (chmod).** Prompts for an **octal** mode string, no source,
validated to `[0, 07777]` before dispatch to match `fs.chmod`'s own guard
(`fs.lua:181-183`). Applies to the §4 set. On a symlink the change lands
on the target and the refreshed listing shows the link's own unchanged
mode (`fs.lua:145-153`); the status line says so once per batch that
included a symlink, because a silent no-op-looking result is the
confusing case. Symbolic modes (`u+x`) are deferred (§10).

**`+` (create directory, 2b).** Prompts for a name, no source, resolved
against `handle.path`. `opts.parents` for `create_dir_all`.

**`C` (copy, 2b).** Targets the §4 set. A single source prompts for a
destination; multiple sources require the destination to be an existing
directory. **A directory source is refused** rather than shallow-copied
— the parent's decision, and the honest one given there is no recursive
copy primitive. Preserves mode bits. `opts.overwrite` defaults false and
an existing target is refused otherwise.

---

## 8. Execution: serialize, report, then revert (Q#DR16)

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

## 9. Staging: 2a then 2b (Q#DR17)

**Recommendation: split, and cut it at "needs a new Rust primitive".**

| | 2a | 2b |
|---|---|---|
| Keys | `m u U t d x D R M` | `+ C`, recursive delete |
| New `pmacs.fs` ops | **none** | `mkdir`, `copy`, `remove_dir_all` |
| New `JobKind` variants | none | 3 (12 → 15) |
| Rust | rename rebind + `apply_resource_op` fix | three primitives |
| Config keys | none | `dired.recursive-deletes` |

The cut works because of C5: `remove` already deletes files and empty
directories, so 2a's whole surface runs on the five ops that exist. That
makes 2a *"the mark-and-operate layer, plus one substrate correctness
fix"* and 2b *"three additive primitives and the two ops that need
them"* — two PRs a reviewer can hold one at a time.

Why not one PR: 2a's only Rust is the rename rebind, whose design is
subtle (a drain-level effect, a layering split, a prefix rule with a
false-positive case, and a bite that requires *not* awaiting). Bundling
it with three new fs primitives means the reviewer who should be
scrutinizing the drain is also checking `copy`'s overwrite semantics.
The arc has already shown what that costs — #165 was one round because
its Rust was one narrowly-scoped change.

Why not three PRs: the mark layer with no operation to consume it ships
nothing a user can do, and a marks-only PR would have to invent
throwaway acceptance for state no command reads.

**2a is the approval-critical one.** If the split is rejected, the
combined PR is the same content in the same order and this framing still
applies; §12's acceptance is already labelled by stage.

---

## 10. Deferred (named)

- **`wdired`** — Stage 3, with the frozen fixture as its reference (C3).
- **The fire-and-forget pending-entry leak** (§2). Pre-existing, verified,
  orthogonal, observable as `pending_len` growth. Needs its own lane; the
  candidate fix is a settled-entry sweep with a reap policy, which is a
  decision about handle lifetime, not a patch.
- **A general `purpose`/`owner` field on `PendingJob`**, per
  `COHERENCE.md` §9, which should subsume §5's `rename_paths`.
- **Migrating `autosave.lua` to `pmacs.minibuffer.confirm`** (§6).
- **Multi-file `R` into a target directory**, and `%`-regexp marking —
  both need a target/pattern concept Stage 2 does not build.
- **Symbolic chmod** (`u+x`), needing a mode-expression parser.
- **`i` (insert subdirectory)** — the recursive in-buffer case, already a
  named deferral in `docs/dired-framing.md` §13, and the place a shared
  tree primitive (`COHERENCE.md` §14) would land.
- **`!` shell command on marks**, compress, symlink, hardlink.
- **Recursive copy** — 2b refuses directory sources; a real `copy -r`
  primitive is separate.

---

## 11. Bets

- **B1.** 2a needs **no** new `pmacs.fs` op. Falsified if any of
  `m u U t d x D R M` cannot be built on the existing five. *(Rests on
  C5, which was read off `remove_blocking` directly.)*
- **B2.** The rename rebind needs **no** protocol bump and leaves the
  worker↔main wire untouched. Falsified by any change to `ReplyKind` or
  `SUPPORTED`.
- **B3.** The frozen `m8_1`/`m8_2`/`m8_3` counts are unchanged by the
  rebind. *Not obvious:* the fixture renames files it lists, so a test
  holding a buffer on a renamed path would newly see its path move. The
  additivity gate is a real check, not a formality.
- **B4.** No GPU or TUI frontend change. Marks are buffer text and the
  keymap is mode-scoped; `set_round_trip_input` is already set by Stage 1.
- **B5.** `describe_key_identifies_every_default_binding` stays green
  without further surgery — #165 already taught it per-binding mode
  context, and Stage 2 only adds more bindings in the same mode.

---

## 12. Acceptance

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

**Stage 2a — operations**

6. `d` then `x` deletes the flagged file, after a confirm; declining
   deletes nothing.
7. `x` consumes `D` flags **only** — a `*`-marked entry survives it.
8. `D` with nothing marked targets the entry at point (§4).
9. `x` on an empty directory succeeds; on a **non-empty** directory it
   fails, the message names the entry, and **the rest of the batch still
   runs**.
10. A batch with one failure reports both counts in one status line, and
    the failed entry **keeps its mark** while the successful one loses it.
11. `R` renames; a bare name resolves against the listing's directory;
    an **existing target is refused**.
12. `M` applies an octal mode to the marked set; an out-of-range mode is
    refused before dispatch.
13. The listing reverts **once** after a batch, not per entry.

**Stage 2a — the rename rebind (§5)**

14. **No-await rename**: dispatch `pmacs.fs.rename`, never take the
    result, pump — the open buffer's path has moved. *(The pin that
    fails if the rebind lives at result-consumption.)*
15. **Directory rename**: a buffer open on `dir/child.txt` follows
    `dir` → `newdir`.
16. **False prefix**: renaming `/…/foo` does **not** rebind a buffer on
    `/…/foobar`.
17. A **failed** rename rebinds nothing.
18. `apply_resource_op`'s rename finds a buffer whose stored path is
    normalized but whose op names it un-normalized (the `:3249` fix).
19. Additivity: `m8_1`, `m8_2`, `m8_3` at unchanged counts.

**Stage 2b**

20. `+` creates a subdirectory, which appears on the next listing.
21. `C` copies a file and preserves mode bits; refuses an existing
    target without `overwrite`; **refuses a directory source**.
22. `C` with several marked entries requires an existing directory
    destination.
23. Recursive delete happens only with `dired.recursive-deletes` enabled
    **and** a confirm; disabled, the non-empty directory still fails.

**Bite obligations.** Each of 14, 16, and 7 must fail against a stated
mutation: the rebind moved to `_take_result`; a string `starts_with`
instead of a component prefix; `x` widened to consume `*`. `dired.lua`
is an existing file now, so `scripts/bite`'s swap-over-`git show` mode
applies — but per #165's lesson, **commit before biting**.

---

## 13. Gates (per PR)

The standard suite from `CLAUDE.md`, plus what this work touches:
`cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D
warnings` as its own step; `cargo test --lib` and `--lib --features
crdt`; `dired_acceptance` (default **and** `crdt`); **`m8_1`, `m8_2`,
`m8_3` at unchanged counts** (the additivity gate, B3);
`m4_acceptance -- --skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test
-p pmacs-gpu`; the isolated-`XDG_CONFIG_HOME` workspace sweep with
`--no-fail-fast`; `git diff --check`.

---

## 14. Numbered decisions

- **Q#DR12** Marks are a per-handle table **keyed by basename**, values
  `*` or `D`, pruned on every re-read following `*buffer-list*`'s
  precedent. `render_entry` takes the mark as a second argument. (§3)
- **Q#DR13** Every operation targets **the marked set, or the entry at
  point when nothing is marked** — except `x`, which consumes `D` flags
  only and never falls back to point. A vanished basename is dropped and
  reported. (§4)
- **Q#DR14** The rename rebind is **harvested in `AsyncRuntime`, applied
  in `pmacs._async._tick`**: `PendingJob` retains `rename_paths`,
  `take_settled_renames` drains successful ones once, and the binding
  rebinds through `find_buffer_for_path` over **every** buffer matching a
  **path-component** prefix. `apply_resource_op`'s raw first-match lookup
  is fixed in the same change. Pinned by a **no-await** rename. (§5, C1)
- **Q#DR15** Confirmation is `pmacs.minibuffer.confirm` in a new
  `builtin/runtime/minibuffer.lua`, with **no completion source**;
  affirmative is `y`/`yes` case-insensitively and everything else —
  including empty `RET` — is no. (§6)
- **Q#DR16** Batches **serialize** (await each op), a per-entry failure
  does not abort, successful marks clear while failed marks persist, and
  the listing reverts **once** at the end. (§8)
- **Q#DR17** Stage 2 **splits into 2a and 2b** at the "needs a new Rust
  primitive" line: 2a is the mark layer plus `d x D R M` on the existing
  five ops plus the rename fix; 2b adds `mkdir`/`copy`/`remove_dir_all`
  and `+ C` and recursive delete. (§9)

---

## 15. Branch and PR plan

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
