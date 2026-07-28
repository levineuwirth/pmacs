# Framing — `apply_resource_op` delete destroys unsaved work

**Revision 1.** Status: **PROPOSED — needs explicit user approval before
implementation. DO NOT implement, DO NOT merge.** Lane:
`resource-op-delete-guard`, worktree `../pmacs-resource-op-delete`,
based on `githubsucks/main` @ `ad41cf1`.

This is a live data-loss bug, reproduced four ways against `ad41cf1`
(§1.1). A language server can destroy a buffer's unsaved edits *and*
the file that would have held them, with no prompt, no status message,
and no error return.

## Revision history

**Revision 1.** First cut. Every claim about pmacs in §1 was verified by
reading the tree at `ad41cf1` and, for §1.1, by running throwaway tests
in this worktree (removed before commit — this lane ships no runtime
code). Every claim about Emacs is marked as such in §1.9 and was
verified against `lisp/progmodes/eglot.el` on `emacs-mirror/emacs`
`master`, not from recollection.


## 0. Coherence impact (COHERENCE §20)

- **Journey step 6, "Receive language intelligence"** (§2), and by
  consequence **step 5, "Edit immediately"** — the loss is of exactly
  the edits step 5 grades as "genuinely excellent". §2's verdict table
  grades step 6 **Partial**; this lane does not raise that grade, it
  removes a way the step can destroy the user's work. A journey that
  eats unsaved edits at step 6 is not a journey worth protecting, so
  this serves **Priority 1** ("treat regressions as release blockers")
  as a correctness floor rather than a feature.
- **Interaction islands: none added.** This is the load-bearing
  constraint, not an afterthought. §6 grades islands "weak, and growing
  by one island per modal feature", and the count is **six**. A
  confirmation prompt for this op would need a seventh (§2.2), which is
  the single strongest argument against the prompt option.
- **Config registry: not adopted.** No knob is proposed. The refusal is
  unconditional. An `lsp.confirm-server-edits`-style setting is the
  natural future adopter and is parked in §6, not shipped here.
- **Background-work attribution: unchanged.**
- **No audited claim in COHERENCE.md changes**, so under §25 no
  COHERENCE edit rides this PR.


## 1. Ground truth (scouted and verified @ `ad41cf1`)

### 1.1 The bug, reproduced

`pmacs.buffer.apply_resource_op` with `kind = "delete"` destroys a
modified buffer and the file backing it. Reproduced in this worktree by
throwaway acceptance tests against `ad41cf1` (written, run, then
removed — no runtime code ships in this lane). Four distinct modes:

**(a) The reported bug.** Open a file, edit it without saving, apply a
delete op for its path:

```
PRE: modified=true text="UNSAVED EDIT ORIGINAL ON DISK\n"
apply_resource_op result: Ok(())
buffers before=3 after=2
file still on disk? false
```

The call returns `Ok(())`. The file is gone. The buffer is gone. The
text existed in exactly one place and now exists nowhere.

**(b) `ignore_if_not_exists = true` destroys the buffer having done no
filesystem work at all.** When the path is already absent, the delete
arm skips the `remove_file` — and then falls through to the buffer
reconciliation anyway:

```
file gone; buffers=3; buffer holds the only copy
result: Ok(())
buffers after=2
CONFIRMED: ignore_if_not_exists=true did ZERO fs work yet still destroyed the only copy.
```

This is the sub-case where the buffer is *most* certainly the only
copy, and it is the one arm where the op explicitly decided to do
nothing. Compare the `create` arm, which returns early
(`return Ok(())`) under the analogous `ignore_if_exists` condition; the
delete arm's `Err(NotFound)` branch does not return, it falls through.

**(c) `recursive = true` fails in the opposite direction.** A recursive
directory delete removes the whole tree from disk but reconciles *no*
buffer, because the lookup is for the directory path and buffers hold
file paths:

```
recursive delete result: Ok(())
inner file exists? false
buffers before=3 after=3
inner buffer still in registry? true
```

The most destructive arm does the least reconciliation. Here the data
survives — in an orphaned buffer pointing at a path that no longer
exists — which is strictly safer than (a) and is why the fix cannot be
"make delete behave like the recursive case".

**(d) Removal is not `kill_buffer`.** The delete path leaves a window
bound to a removed `BufferId`, and can drive the registry to empty:

```
victim is the active buffer? true
delete result: Ok(())
window.buffer():is_valid() after delete => Ok(Boolean(false))
editor.file_path() after delete => Ok(Nil)
```
```
buffers now: 1
delete of the LAST buffer: Ok(())
buffers after: 0  (0 => registry driven empty)
```

`EditorCore::kill_buffer` (`src/editor_core.rs:4590`) explicitly refuses
the last buffer and rebinds every window to a fallback. This path does
neither. **(d) is named here for the record and is parked** — see §6 and
Q#RD8; it is a different failure from data loss and it is shared with
`pmacs.buffer.remove`.

### 1.2 There is no dirty check at any link in the chain

Three links, verified by reading each:

1. **`apply_resource_op`'s delete arm** (`src/lua_bindings/mod.rs:3313`)
   — stats the path, calls `remove_file` / `remove_dir` /
   `remove_dir_all`, then `find_by_path`, then `remove_buffer_and_fire`.
   No `is_modified` anywhere in the arm.
2. **`remove_buffer_and_fire`** (`src/lua_bindings/mod.rs:1592`) — four
   lines: `registry.borrow_mut().remove(id)`, then
   `after_buffer_removed`. `after_buffer_removed`
   (`src/lua_bindings/mod.rs:1602`) clears keymaps, config, folds and
   fires `on_removed` callbacks. No dirty check.
3. **`BufferRegistry::remove`** (`src/buffer_registry.rs:127`) — has
   exactly one guard, and it is not this one:

   ```rust
   if let Some(buf) = self.buffers.get(&id)
       && buf.editing_in_progress()
   {
       return Err(RegistryError::ConcurrentEdit { ... });
   }
   ```

   That refuses re-entrant removal from inside an edit intercept
   (T M7.4). It says nothing about unsaved content.

The user's description of the bug is **accurate at every link**. Modes
(b), (c) and (d) are additional and were not in the report.

### 1.3 The ordering is disk-first, so no guard placed later can help

The arm performs the irreversible filesystem operation **before** it has
even looked for a buffer. By the time any `is_modified` check could run,
the file is gone. This is the single most important structural fact in
this document: **a fix that adds a dirty check to the buffer-reconcile
block is not a fix.** It converts "file deleted, buffer destroyed" into
"file deleted, buffer orphaned" — mode (c) — which is better, but the
user's on-disk file is still gone and they never consented.

### 1.4 `is_modified` is available, singular, and not on `pmacs.buffer`

- One field, `Buffer::is_modified: bool` (`src/buffer.rs:164`); one
  accessor `Buffer::is_modified()` (`src/buffer.rs:473`); one public
  mutator `Buffer::mark_clean()` (`src/buffer.rs:488`). There is no
  `dirty`, no `saved_revision`, no public setter to true.
- **Zero features refuse or confirm on it today.** `kill_buffer`
  (`src/editor_core.rs:4590`) does not check it; `editor.quit`
  (`builtin/commands/default.lua:250`) has a veto hook
  (`editor.before-quit`) that nothing subscribes to with a modified
  check; `dired.revert` does not check it. The one behavioural consumer
  is autosave's `gather` filter (`src/autosave.rs:363`), which *includes*
  rather than refuses. **So this lane introduces the first refusal in the
  codebase keyed on unsaved state**, and should be read as setting that
  precedent.
- Lua reaches it as the userdata method `buf:is_modified()`
  (`src/lua_bindings/mod.rs:1261`) and as
  `pmacs.describe.buffer(id).modified` (`:6359`). It is **not** a key on
  the `pmacs.buffer` module table. Relevant because the batch preflight
  (Q#RD3) is Lua and needs a path-keyed query it does not have today.

### 1.5 Who calls this, and what happens to a raised error

`pmacs.buffer.apply_resource_op` has exactly one production caller:
`apply_workspace_edit` (`builtin/runtime/lsp.lua:1301`), at `:1346`.
That function has three callers:

| # | Call site | Origin | Error disposition |
|---|---|---|---|
| 1 | `handle_server_requests` (`lsp.lua:1815`), call at `:1836` | **server-initiated** `workspace/applyEdit` | **Swallowed silently.** The pump is driven by `pcall(handle_server_requests)` (`lsp.lua:1892`). A raise unwinds past the `pcall(pmacs.lsp.send_response, ...)` that answers the request, so the user sees nothing *and the server is never answered.* |
| 2 | LSP rename (`lsp.lua:2311`) | user, `M-x` | `apply_workspace_edit` never returns `nil` for an op failure, so a raise propagates out of the `pmacs.async` coroutine. |
| 3 | code action apply (`lsp.lua:2373`) | user, `M-x` | as #2. |

Only path 1 is fully unattended. This matters: **the refusal cannot be
delivered by raising**, or the most dangerous path reports nothing and
hangs the server's request. That is a design constraint, not a nicety
(Q#RD7).

Note also `src/rename.rs:25`, which documents the division of labour:
`rename.rs` parses and never mutates; Lua drives `pmacs.buffer.*`
"so the application strategy stays configurable". The application
strategy is the thing this framing is choosing.

### 1.6 A partial batch is already possible today — verified

The applier's loop (`lsp.lua:1340-1349`) calls `apply_resource_op`
unprotected. Two delete ops where the second raises:

```
batch result: Err(... "apply_resource_op delete: No such file or directory (os error 2)")
a.txt still exists? false (false ⇒ partial batch)
```

The first op stayed applied. So **partial application on I/O error is
the status quo**, not something a refusal would introduce. This
substantially weakens the "is a partially-applied WorkspaceEdit worse
than the data loss?" objection — the partial batch already exists and
data loss is strictly worse than a class of failure the code already
tolerates.

It also shows the preflight's documented contract is narrower than its
comment implies. `lsp.lua:1287-1291` says the applier "refuses to mutate
*anything* unless every URI it touches resolves to a real file path
first". True — but URI resolution is the *only* thing preflighted. The
plan loop (`:1302-1336`) validates nothing about the filesystem or the
buffer registry. **The preflight phase exists and is the natural place
to add a second precondition** (Q#RD3).

### 1.7 The rename arm is more careful, and differently careful

Directly above the delete arm, `"rename"` (`mod.rs:3291`) does
`std::fs::rename`, then `find_by_path`, then
`core.borrow_mut().set_buffer_path(id, Some(to))`. It **rebinds** the
buffer and preserves its contents and its modified state. Delete
**destroys**. The asymmetry is the whole bug: rename treats the buffer
as the valuable thing and the path as the mutable attribute; delete
treats the buffer as a cache of the file.

Both arms share two latent defects, already recorded against the dired
arc: `find_by_path` (`src/buffer_registry.rs:168`) is exact `Path`
equality, first match only, called with the raw path while stored paths
are normalized on write. `EditorCore::find_buffer_for_path`
(`src/editor_core.rs:935`) is the normalizing wrapper that exists and is
bypassed. See `docs/dired-framing.md:807-819` and the ledger note at
`docs/active-work.md:605`, both of which claim the **rename** side of
this for dired Stage 2. §6 draws the boundary.

### 1.8 What pmacs cannot do, established rather than assumed

- **A prompt cannot be issued from inside this binding.**
  `pmacs.minibuffer.read` (`src/lua_bindings/mod.rs:13380`) is
  asynchronous-by-callback: it registers `on_accept`/`on_cancel` and
  returns immediately. `Minibuffer::accept`
  (`src/minibuffer.rs:334`) deliberately *returns* the callback rather
  than calling it, with the reason stated at `:332` — "firing user code
  from inside the minibuffer would re-enter the registry". The answer
  arrives on a later keystroke, through the event loop.
  `apply_resource_op` is a synchronous Rust closure that performs its
  `std::fs` calls inline and returns; there is no point at which it can
  suspend. **There is no `y_or_n` helper in the tree at all** — a named
  deferral in `docs/dired-framing.md:854`.
- **Autosave cannot be used as a pre-delete backup.** Verified against
  `src/autosave.rs`:
  - there is no per-buffer write entry point; the only public writer is
    `sweep` (`:261`), which walks the whole registry;
  - `sweep` skips clean buffers (`:363`);
  - **removing a buffer purges its recovery file** — the `on_removed`
    callback registered at `builtin/runtime/autosave.lua:167-169` calls
    `discard_buffer` (`src/autosave.rs:511`), and a sweep-time GC
    (`:290-306`) catches whatever the callback misses. Pinned by
    `tests/autosave_acceptance.rs:702`.
  - deleting the file flips the recovery's status to `Stale`, and
    `Stale` is never auto-offered.

  So "back up, then delete" is self-defeating four times over: no
  entry point, wrong filter, the backup is deleted by the very removal
  it was protecting against, and what survives is never surfaced.
- **`pmacs.editor.set_status` is the available report channel**
  (`src/lua_bindings/mod.rs:13036`), cleared at the top of every
  `dispatch_key`. There is no `*Messages*` buffer and no `*warnings*`
  buffer. `pmacs.error` is referenced in `async.lua` and **never
  defined**.

### 1.9 Prior art — claims about **Emacs**, not pmacs

Verified against `lisp/progmodes/eglot.el`, `emacs-mirror/emacs`
`master`. Everything in this subsection is a statement about Emacs.

**Eglot orders the operations the other way round.** Its `do-delete`:

```elisp
(do-delete (path &key recursive ignoreIfNotExists &allow-other-keys)
  (let ((exists (file-exists-p path)))
    (when (and (not exists) (not ignoreIfNotExists))
      (eglot--error "File %s does not exist" path))
    (when exists
      ;; Kill buffer if the file is visited
      (let ((buf (find-buffer-visiting path)))
        (when buf (kill-buffer buf)))
      (delete-file path recursive))))
```

The buffer is killed **before** the file is deleted. In Emacs,
`kill-buffer` on a modified file-visiting buffer prompts, so the consent
gate sits ahead of the irreversible step by construction. (Eglot does
not check `kill-buffer`'s return value, so declining the kill still
deletes the file — but the buffer, and therefore the text, survives.
Even Eglot's failure mode is strictly milder than pmacs's.)

Note also the `exists` guard: Eglot's `ignoreIfNotExists` path does
**not** fall through to the buffer kill. That is exactly the asymmetry
mode (b) exposes in pmacs.

**Eglot confirms server-initiated edits by default, as a whole-batch
decision made before anything is applied.** `eglot-confirm-server-edits`
defaults to `'((t . maybe-summary))`. The prepare/decide/apply structure
is explicit: `prepare` builds a list of closures touching nothing, then
`eglot--confirm-server-edits` decides, then `apply-all` runs. The
`maybe-*` decisions skip the prompt only when the batch is `peaceful`:

```elisp
(peaceful
 (and
  all-text-edits
  (cl-loop for op in prepared
           always (find-buffer-visiting (cadddr op)))))
```

`all-text-edits` is a conjunction over the whole batch, so **a batch
containing any create/rename/delete is never `peaceful` and always
prompts** under the default. Confirmation is all-or-nothing and
strictly precedes mutation; there is no mid-batch interrupt.

**The transferable lessons** (now claims about what pmacs should do):
the consent gate belongs *before* the irreversible step; the decision is
made over the *whole* batch during a preparation phase; and
`ignoreIfNotExists` must short-circuit the buffer half too.

### 1.10 `apply_resource_op` has no direct test coverage

`grep -rn "apply_resource_op" tests/ src/` returns **4 lines**: one doc
comment in `src/rename.rs`, and three inside the binding's own
definition in `src/lua_bindings/mod.rs`. Zero tests name it.

It is exercised indirectly by exactly one acceptance,
`m4_15_workspace_edit_resource_ops_apply_in_order`
(`tests/m4_acceptance.rs:4014`), driven by the `resourceops` mode of the
fake server (`src/bin/pmacs_fake_lsp.rs:834`), which emits an ordered
create / edit / rename / delete. **Its deleted file `c.rs` is never
opened in a buffer**, so the entire buffer-reconciliation half of the
delete arm is untested. That suite and that fake are where this lane's
acceptances belong.


## 2. The decision space

### 2.1 Recommended — refuse before touching disk, at both layers

**Refuse the delete when it would destroy unsaved work, and refuse it
before the filesystem call. Do this at the Rust primitive (the
invariant) *and* in the applier's existing preflight (batch
atomicity).**

Concretely:

- **Layer 1, the primitive.** The delete arm reconciles the buffer
  registry *first*. If the path resolves to a modified buffer, it
  returns an error and touches nothing. Inverting the order is what
  makes the guard possible at all (§1.3), and it also closes a smaller
  hole: today a `ConcurrentEdit` refusal from `BufferRegistry::remove`
  arrives after the file is already gone.
- **Layer 2, the applier preflight.** `apply_workspace_edit`'s plan loop
  (`lsp.lua:1302-1336`) gains a second precondition alongside URI
  resolution, and returns its existing `nil, message` when any delete op
  targets a modified buffer — so **nothing** in the batch is mutated.

Both layers are needed, and neither is redundant:

- Layer 1 alone leaves partial batches (§1.6): op 1 deletes, op 2
  refuses, and the user is left mid-edit with no way back.
- Layer 2 alone leaves the primitive armed. `pmacs.buffer.apply_resource_op`
  is public Lua API; dired Stage 2 and any package can call it directly.
  A guard that lives only in one caller is exactly the shape the project
  has been burned by — "pin the guard through the real path".

**Why this beats the runners-up, in one sentence each:**

- It is the only option that is *possible* — prompting is architecturally
  unavailable (§1.8) — and it is the option Emacs's structure already
  endorses (§1.9): consent gate before the irreversible step, decision
  taken over the whole batch during preparation.

### 2.2 Prompt the user — rejected

Rejected on three independent grounds, any one sufficient.

1. **Architecturally unavailable.** `apply_resource_op` is synchronous
   Rust; prompts are callback-continuations resumed by a later keystroke
   (§1.8). Making this work means restructuring the applier into a
   callback chain carrying the remaining plan as a closure upvalue, with
   revalidation of every precondition at each resumption — a large,
   independently-risky change to LSP edit application, for a guard.
2. **It costs an interaction island.** The alternative to a callback
   chain is a Rust modal shadow (the query-replace shape). That is a
   **seventh** dispatcher shadow. COHERENCE §6 grades this area "weak,
   and growing by one island per modal feature" and records that
   terminal copy mode was deliberately engineered *not* to become the
   seventh. Spending that budget on an error path would be a poor trade.
3. **It cannot serve the most dangerous caller.** The server-initiated
   path (§1.5, row 1) must answer `workspace/applyEdit` synchronously
   with `applied: true | false`. There is no user turn available inside
   it, and a prompt that resolves three keystrokes later cannot produce
   that answer.

Note that "what happens to the rest of the batch?" — the question that
makes prompting genuinely hard — is dissolved by the recommendation
rather than answered: the decision is taken in the preflight, before any
op runs, so there is no rest-of-batch to strand.

### 2.3 Save first, then delete — rejected

Silently converts an unsaved edit into a committed one and then destroys
it. It is *more* destructive than refusing, not less: it overwrites the
user's on-disk original — the copy they might have wanted — immediately
before removing the file. It also cannot be relied on: `save_inner`
refuses when the file changed on disk since it was read
(`src/editor_core.rs:1908`, guard at `:1917`), so the fallback question is unanswered and
we are back to refusing.

### 2.4 Back up the contents somewhere recoverable — rejected

Rejected on evidence, not taste. §1.8 establishes that the existing
autosave machinery defeats this four ways, the decisive one being that
**removing the buffer deletes the recovery file** — the backup is
destroyed by the very operation it exists to survive. Building a
parallel side-store outside `autosave/` means inventing a second
recovery surface with its own discovery, GC and lifecycle, to make a
destructive operation *feel* safe. Refusing is cheaper and honest.

### 2.5 Key the behaviour on LSP-versus-user provenance — rejected

Superficially attractive because Eglot's default keys on exactly this
(`eglot-confirm-server-edits`). But: `apply_resource_op` takes no
provenance argument and there is no ambient caller identity to read.
Adding one makes the primitive's safety depend on a caller-supplied
flag — any caller that omits it, or passes the permissive value, is
unguarded, which is the failure mode the whole lane exists to remove.
COHERENCE §10 (extension trust classes) is unbuilt, so there is no
existing trust dimension to key on either. **The refusal is
unconditional and provenance-blind.** A future
`lsp.confirm-server-edits` setting can *loosen* it once a prompt
mechanism exists; §6.


## 3. Decisions

### Q#RD1 — Refuse. Do not prompt, do not save, do not back up

An `apply_resource_op` delete whose target resolves to a modified buffer
**fails**, changing nothing on disk and nothing in the registry.
Rationale: §2.2–§2.5. This is the first refusal in the codebase keyed on
unsaved state (§1.4) and is intended as the precedent for `kill_buffer`
and `editor.quit`, which have the same gap.

### Q#RD2 — Reconcile the registry before touching the filesystem

The delete arm's order inverts: resolve the buffer set, decide, and only
then call `remove_file` / `remove_dir` / `remove_dir_all`. This is what
makes the guard expressible (§1.3) and it also moves the pre-existing
`ConcurrentEdit` refusal ahead of the irreversible step.

**What inverting breaks, considered:** the current order means a
successful `remove_buffer_and_fire` implies the file is already gone,
so `on_removed` subscribers observe a consistent "file and buffer both
gone" world. After inversion, `on_removed` fires with the file still
present for the remainder of the call. Two mitigations: the fs call
follows immediately with no yield point in between (Lua callbacks run
synchronously inside `after_buffer_removed`), and if the fs call then
fails the correct end state is *ambiguous either way* — today it cannot
happen because the buffer removal is unreachable on fs failure. Q#RD9
records the resolution.

### Q#RD3 — The batch aborts in the preflight, before any op runs

`apply_workspace_edit`'s plan loop gains a modified-buffer precondition
for delete ops and returns `nil, message`. This reuses the applier's
existing, documented abort contract — "aborts the whole edit cleanly,
origin buffer untouched, rather than half-applying"
(`lsp.lua:1287-1291`) — and all three callers already handle the
`nil, message` shape (§1.5). It is Eglot's prepare/decide/apply shape
(§1.9) implemented in the phase pmacs already has.

The preflight needs a path-keyed modified query, which Lua lacks today
(§1.4). The minimal addition is a `pmacs.buffer` surface answering
"is there a modified buffer at or beneath this path"; its exact shape is
an implementation choice, but it must be **one** query so the preflight
and the primitive cannot drift apart.

### Q#RD4 — `ignore_if_not_exists` short-circuits the buffer half too

When the path is absent and `ignore_if_not_exists` is set, the arm
returns early **without** touching the registry — matching the `create`
arm's existing `return Ok(())` idiom and Eglot's `exists` guard (§1.9).
Mode (b) is not a special case of the main bug; it is a missing early
return, and a fix aimed only at the "we actually deleted something"
branch leaves it live.

### Q#RD5 — `recursive` deletes are prefix-aware, or the guard has a documented bypass

Mode (c) proves that `recursive = true` reconciles nothing, so an
exact-path guard is bypassed entirely by the most destructive arm: a
server that sends `{kind: "delete", uri: <dir>, recursive: true}` walks
straight past it. A guard with a trivial, reachable bypass is not a
guard, so the check must cover every buffer whose path lies **beneath**
the deleted directory, not merely one whose path equals it.

**This overlaps dired Stage 2 and the boundary is drawn explicitly.**
`docs/dired-framing.md:807-819` and `docs/active-work.md:605` claim
prefix-aware, normalize-before-lookup rebinding for the **rename** side.
This lane takes the **delete** side only, because without it this lane
ships nothing. The two want the same helper; whichever lands second
adopts the first's. If the user prefers, the alternative is to sequence
this lane after dired Stage 2 and consume its helper — but the bug is
live and dired Stage 2 is unframed for this, so shipping first is
recommended.

### Q#RD6 — Lookups normalize; "modified" means `Buffer::is_modified`

The guard resolves paths through the normalizing wrapper
(`EditorCore::find_buffer_for_path`, `src/editor_core.rs:935`) rather
than raw `find_by_path`, because a lookup miss is a *silent* guard
bypass (§1.7). "Modified" is `Buffer::is_modified()` — the single
existing predicate (§1.4). No new notion of dirtiness is introduced.

Explicitly **not** guarded: a clean buffer. A delete whose target is
open but unmodified proceeds and removes the buffer, exactly as today.
Overreach here would break `m4_15` and, more importantly, would make the
LSP's legitimate deletes fail for users who merely have the file open.

### Q#RD7 — The refusal is reported on every path, and never by raising alone

Per §1.5, a raise on the server-initiated path is swallowed by
`pcall(handle_server_requests)` and the server is left unanswered. So:

- **Preflight refusal (all three callers)** returns `nil, message`.
  Callers 2 and 3 already render that to `set_status`; caller 1 already
  turns it into `{ applied = false, failureReason = ... }` and sends the
  response.
- **Primitive refusal** still raises — it must, being a Rust binding —
  but that is now a defence-in-depth path reached only by direct callers,
  because the preflight catches the LSP path first.

The message names the buffer and says what to do: save it, or use the
buffer-level command to discard. It must not be a bare errno.

### Q#RD8 — The window/last-buffer defects do **not** land here

Mode (d) is real and is parked (§6). Two reasons. It is a different
failure (a dangling window and an empty registry, not data loss), and it
is shared with `pmacs.buffer.remove` rather than specific to delete.

**And there is a trap that makes the obvious fix wrong.** The two removal
paths clean *disjoint* sets: `kill_buffer` handles the last-buffer
refusal, `round_trip_buffers`, side-window collapse and window rebinding,
but **not** keymaps, config, folds or `on_removed` callbacks;
`remove_buffer_and_fire` handles exactly the latter and none of the
former. Neither is a superset of the other, so "just call `kill_buffer`
instead" would silently regress four cleanups. Unifying them is its own
lane with its own census.

### Q#RD9 — On filesystem failure after the buffer is removed, the buffer wins

Given Q#RD2's inversion, a delete can now fail *after* the buffer is
gone. The buffer is not restored. Rationale: the buffer removal is only
reached for a clean buffer (Q#RD1), so nothing unsaved is at stake, and
re-inserting a buffer would need a new registry primitive and would
resurrect it with a fresh `BufferId` that no window, keymap or callback
refers to. The op reports the fs error as it does today. This is a
deliberate, narrow widening of the failure surface and is called out so
review can reject it rather than discover it.


## 4. Bets (falsifiable)

- **B1 — Refusing breaks no legitimate server workflow.** A server
  deleting a file the user has unsaved edits in is a conflict the user
  must resolve; no server needs that delete to succeed silently.
  Falsified by a real server whose normal operation deletes files the
  user is actively editing.
- **B2 — The preflight is the right layer for batch atomicity.**
  Falsified if a `WorkspaceEdit` legitimately depends on a delete whose
  precondition can only be evaluated after an earlier op runs (e.g. a
  rename that moves the modified buffer out of the delete's path first).
  **This is the sharpest risk in the design** and acceptance 8 pins the
  behaviour so the failure is loud rather than silent.
- **B3 — Prefix-aware checking does not over-refuse.** Falsified if a
  common workflow deletes a directory while an unrelated modified buffer
  sits beneath it and the refusal is judged unhelpful.
- **B4 — Inverting the order breaks no `on_removed` subscriber.**
  Evidence: the fs call follows synchronously with no yield in between.
  Falsified by a subscriber that stats the path.


## 5. Acceptance

Each criterion states the **pre-image it must fail against**. A test
that passes against its pre-image has no bite and is rejected.

1. **A delete op targeting a modified buffer refuses, and the file
   survives.** Assert three things together: the call fails, the buffer
   is still in the registry with its exact unsaved text, and
   `path.exists()` is still true.
   *Bite:* fails against `ad41cf1` unmodified (today: `Ok(())`, file
   gone, buffer gone). **Asserting only that the buffer survived is
   vacuous** — that is mode (c)'s behaviour, which this lane must not
   ship. The `exists()` assertion is the load-bearing one.

2. **A delete op targeting a *clean* open buffer still succeeds**, file
   removed and buffer removed.
   *Bite:* fails against an over-broad guard that refuses whenever a
   buffer is open. Assert **both directions**, per the bottom-panel
   lesson that a blanket rewrite passes an "everything moved" test.

3. **`ignore_if_not_exists = true` on an absent path leaves a modified
   buffer intact** (Q#RD4).
   *Bite:* fails against a fix that guards only the branch where the fs
   delete actually ran — reproduce mode (b) exactly: file removed behind
   pmacs's back first, then the op.

4. **`recursive = true` on a directory containing a modified buffer's
   file refuses, and the whole tree survives** (Q#RD5).
   *Bite:* fails against an exact-path-equality guard. Assert the inner
   file still exists — not just that the buffer does, which is already
   true today (mode (c)).

5. **Whole-batch atomicity: a `documentChanges` whose *second* op is a
   blocked delete leaves the *first* op unapplied** (Q#RD3).
   *Bite:* fails against a primitive-only fix. §1.6 verified that op 1
   currently stays applied, so this is a real behaviour change and the
   assertion must name op 1's effect (e.g. a file that must still exist,
   or must not yet have been created).

6. **The refusal reaches the server on the unattended path.** Drive a
   server-initiated `workspace/applyEdit` carrying a blocked delete and
   assert the server receives a response with `applied = false` and a
   non-empty `failureReason`.
   *Bite:* fails against a fix that refuses by raising — §1.5 established
   that `pcall(handle_server_requests)` (`lsp.lua:1892`) swallows the
   raise and the response is never sent. This is the "pin the guard
   through the outermost user-reachable seam" obligation; a direct-call
   test on `apply_resource_op` does not satisfy it and is rejected as
   insufficient for this criterion.

7. **The user-initiated paths report on the status line.** LSP rename
   and code action each surface a message naming the buffer.
   *Bite:* fails against a fix that returns `nil` without a message, or
   whose message is a bare errno.

8. **The B2 risk is pinned:** a batch that renames the modified buffer's
   file *and then* deletes the old path is refused by the preflight
   rather than silently mis-evaluated. Whichever behaviour review
   chooses, it is asserted, so the limitation is a documented decision
   rather than an accident.

9. **`m4_15_workspace_edit_resource_ops_apply_in_order` stays green**
   unmodified, pinning "no regression to the ordered-resource-op path"
   from outside. Its `c.rs` is never opened, so it exercises exactly the
   unguarded case that must keep working (Q#RD6).

10. **Every new test is checked with `scripts/bite`** and none reports
    VACUOUS.


## 6. Parked — not deferred-and-forgotten

- **Mode (d): the dangling window and the emptiable registry** (§1.1,
  Q#RD8). Real, reachable, shared with `pmacs.buffer.remove`. Needs its
  own lane and a census, because the two removal paths clean disjoint
  sets and the obvious unification regresses four cleanups.
- **`kill_buffer` and `editor.quit` have the same gap** (§1.4): both
  discard unsaved work with no check. `editor.before-quit` exists as a
  veto channel with no subscriber. This lane sets the precedent; those
  are separate lanes.
- **A `y_or_n` helper**, and with it any confirm-instead-of-refuse
  option. Already a named deferral (`docs/dired-framing.md:854`).
- **`lsp.confirm-server-edits`**, the config-registry adopter that would
  let a user loosen Q#RD1 once a prompt mechanism exists (§2.5).
- **The rename side of prefix-aware, normalizing lookup** — dired
  Stage 2's, per Q#RD5.
- **A general transient-keymap layer** (COHERENCE §6), the prerequisite
  that would make §2.2 cheap rather than impossible.


## 7. Gates

Full suite per `CLAUDE.md`: `cargo fmt --check`; `cargo clippy
--workspace --all-targets -- -D warnings` as its own step; `cargo test
--lib`; `cargo test --lib --features crdt`; the touched acceptance
suites; `cargo test --test m4_acceptance -- --skip basedpyright`;
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`; `git diff --check`.

Touched suites: **`m4_acceptance`** (the resource-op home, §1.10) and
`lsp_dispatch_seams_acceptance`. `dired_acceptance` is a watch item for
Q#RD5's shared lookup change.

Gate the pushed tree, not the worktree — commit first, then gate.

This framing-only PR ships no runtime code, so its own gate is
`git diff --check` plus a docs read.


## 8. Branch plan

`resource-op-delete-guard`, worktree `../pmacs-resource-op-delete`, one
PR. **This framing is the entire first PR.** Implementation does not
begin until the user approves the design — specifically Q#RD1 (refuse
rather than prompt), Q#RD5 (take the delete side of prefix-awareness now
rather than sequencing behind dired Stage 2), and Q#RD9 (the buffer is
not restored on fs failure).

Files this lane will touch when approved: `src/lua_bindings/mod.rs`
(the delete arm and a modified-at-or-beneath query),
`builtin/runtime/lsp.lua` (the preflight), `tests/m4_acceptance.rs` and
`src/bin/pmacs_fake_lsp.rs` (a fake mode carrying a blocked delete).
It will **not** touch `src/daemon.rs`, `pmacs-protocol/`, or
`builtin/runtime/dired.lua`. No protocol change.
