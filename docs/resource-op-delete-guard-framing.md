# Framing — `apply_resource_op` delete destroys unsaved work

**Revision 3.** Status: **PROPOSED — needs explicit user approval before
implementation. DO NOT implement, DO NOT merge.** Lane:
`resource-op-delete-guard`, worktree `../pmacs-resource-op-delete`,
rebased at revision 3 onto `githubsucks/main` @ `7586905` (PR #189,
COHERENCE.md only — no bearing on any decision here).

This is a live data-loss bug, reproduced four ways against `ad41cf1`
(§1.1). A language server can destroy a buffer's unsaved edits *and*
the file that would have held them, with no prompt, no status message,
and no error return.

**This PR is the implementation PR.** Revision 2 drops revision 1's
framing-PR-then-implementation-PR plan, which conflicted with
one-feature/one-branch/one-PR. The framing is revised in place; once the
design is approved, the implementation lands on this same branch and in
this same PR (§8).

## Revision history

### Revision 2 → 3, after review round 2

Round 2 confirmed everything central from round 1 as fixed and raised
four P1s. All four accepted; two sweeps run.

**P1-1 — the ownership boundary was stale.** Revision 2 described PR
#171 as "OPEN, STALE, 153 commits behind, under re-scout" and said it
claimed the **rename** side only. Re-checked directly: #171 is at
**revision 7, `fd7ae37`, merge-base `ad41cf1`, 0 commits behind** — not
stale. Revision 6 had assigned **both** rename and delete to Stage 2a,
with the **opposite** policy: its `reconcile_delete` "kills unmodified
buffers and keeps modified ones alive", i.e. the file is deleted and the
modified buffer orphaned, and its §11 named that orphaning as accepted
residue. Two lanes, opposite answers, same event. §1.12 and Q#RD5 now
carry the settled split verbatim, and #171 revision 7 has adopted it
from the other side.

**P1-2 — the LSP failure-handling claim was wrong.** Revision 2 said the
spec "assigns `Abort` to any edit containing resource operations" and
rested B2 on it. **It does not.** Recovery is described by the client's
advertised `failureHandling`; `Abort` is one of four strategies, and
only `TextOnlyTransactional` degrades to abort when resource changes are
present. pmacs advertises **none** (§1.11, established by revision 2's
own sweep), so the spec assigns pmacs no strategy at all. §1.7 and B2 are
rewritten to stand on **verified pmacs behaviour** — §1.6's reproduced
partial batch — rather than on borrowed protocol authority. This was the
document's second external-spec overclaim; every external claim now
carries a direct quote or is marked not established (§1.15).

**P1-3 — Q#RD7 had no implementable, tested reporting seam.** Three gaps
confirmed by reading: `_parse_workspace_edit` is called at
`builtin/runtime/lsp.lua:1835`, **outside** the `apply_workspace_edit`
call revision 2 proposed to wrap, and it is fallible
(`lua_to_json(edit)?`, `src/lua_bindings/mod.rs:10161`), so "always
answers" was false for a parse failure; `append_to_errors_buffer`
(`src/lua.rs:401`) is **private**, so revision 2's promise to log
through it was not implementable from where it was made, and a Lua
preflight rejection never reaches Rust anyway; and acceptance 13 tested
the response but not the promised `*errors*` trace. Q#RD7 is rewritten
around **one seam at the server-request boundary**, and of the two
options offered, this revision **picks wrapping parse-plus-apply**
rather than narrowing the claim — the fix is one line up from the
existing wrap and it makes "always answers" true rather than qualified.

**P1-4 — clean duplicate reconciliation was unspecified.** Revision 2
said validation scans every match but never said what reconciliation
does afterwards. Now an explicit decision, **Q#RD10**, taking the user's
steer: **validate every match, reconcile only today's first exact-path
match.** Widening would enlarge the parked lifecycle defect that Q#RD5
exists to contain; the surviving clean duplicate is named as residue
handed to #171, not left silent. Acceptance 14 pins it in both
directions.

**Sweep — external claims.** Every non-repo claim re-audited (§1.15).
One was a paraphrase standing in for a quote: revision 2 asserted that
"in Emacs `kill-buffer` on a modified file-visiting buffer prompts"
without establishing it. It is true, but **not for the reason a reader
would assume**, and the precise version matters to the argument — see
§1.13, which now quotes `Fkill_buffer` and the `INTERACTIVE` macro.

**Sweep — cross-lane claims.** Beyond P1-1: revision 2's Q#RD8 said mode
(d) "needs its own lane and a census". **It has one** — #171's
`reconcile_delete` composes both removal phases and adopts the trap
verbatim. Q#RD8 and §6 now name the owner instead of describing the
defect as unowned. Q#RD6 additionally **claims the shared query
explicitly**, per the boundary's "whichever lands first owns the query",
so the duplicate resolves in one direction. And a gap neither lane
closes — `pmacs.fs.remove` has no dirty check of its own — is now named
as explicitly out of scope with its owner (§6), because "both lanes
guard deletion" otherwise reads as the primitive being guarded.

### Revision 1 → 2, after review round 1

The refusal strategy was approved in principle; revision 1 as written
was not. Q#RD1 (refuse unconditionally) and Q#RD5 (take prefix-awareness
now) are **settled yes**; Q#RD9 is **settled no** and is withdrawn.
Six blocking points, all accepted, plus three further overclaims found
by the sweep the review asked for.

1. **Q#RD2 conflated inspection with removal — rewritten.** Revision 1
   proposed removing the buffer *before* the filesystem call. That fires
   arbitrary Lua `on_removed` callbacks while the file still exists,
   destroying today's useful invariant that a subscriber observes the
   path already gone, and it accepts losing the buffer if the deletion
   then fails. The correct sequence separates the two:
   `stat/no-op → enumerate and validate → mutate filesystem → reconcile`.
   Validation needs no removal, so a failed deletion leaves the buffer
   intact automatically. **The review is right and revision 1 was
   wrong.** `Buffer::editing_in_progress()` (`src/buffer.rs:747`) is a
   public getter, so the re-entrancy condition can also be checked
   during validation rather than discovered during removal.
2. **Q#RD3 overclaimed whole-batch atomicity — rewritten and
   downgraded.** Revision 1 called the preflight "whole-batch
   atomicity" and said "**nothing** in the batch is mutated". That is
   false for a sequential batch: an earlier text edit can dirty a
   clean buffer, and an earlier rename can move a modified buffer
   *into* a later delete's subtree, after the snapshot was taken. The
   preflight is now described as an **early conflict check** — a
   cheap, honest first filter, not a transaction (§2.1, Q#RD3). The
   real robustness comes from per-op `pcall` and an always-sent
   response (Q#RD7).
3. **The lookup cannot be `EditorCore::find_buffer_for_path` — accepted
   and independently re-verified.** It normalizes but delegates to
   `BufferRegistry::find_by_path` (`src/buffer_registry.rs:168`), whose
   own doc says "First buffer bound to `path`" — singular, insertion
   order. And `pmacs.buffer.from_file` (`src/lua_bindings/mod.rs:3112`)
   calls `create_from_bytes` with no dedup check, so duplicate
   path-bound buffers are reachable from public Lua. A clean first match
   hides a modified second. Q#RD6 now requires a full scan.
4. **Do not expand the parked lifecycle defect — accepted.** Revision 1
   left it ambiguous whether a recursive delete should reconcile
   descendants. It must not: mode (d)'s dangling-window and
   last-buffer defects would be promoted from exact-path to tree-wide.
   Q#RD5 now says explicitly that the tree is **inspected** but only the
   exact path is **reconciled**.
5. **Q#RD4 must hold at both layers — accepted.** Revision 1 applied the
   `ignore_if_not_exists` early return only to the primitive. The Lua
   preflight must not reject an absent path merely because a modified
   buffer still names it.
6. **The prompt argument was overclaimed — rewritten, and this was my
   error.** Revision 1 said prompting was "architecturally
   unavailable", "the only option that is *possible*", and that the
   server-initiated path "cannot produce that answer". All three are
   wrong. `pmacs.lsp.send_response` (`src/lua_bindings/mod.rs:9680`)
   takes `request_id` as an ordinary value, so a `workspace/applyEdit`
   **can** be answered on a later tick; and a callback continuation
   would reuse the **existing** minibuffer shadow (rung 4), not add a
   seventh rung. The honest claim is that prompting is *expensive and
   separately scoped*, and §2.2 now argues only that, on evidence
   (§1.8).

**Three further overclaims found by the sweep** (the review asked for
the defect class, not just the three cited instances — all three are
the same shape: revision 1 asserted an absence or a guarantee it had
not established):

7. **"There is no `*Messages*` buffer and no `*warnings*` buffer" was
   misleading by omission.** There is a durable append-only error
   surface: `LuaHost::append_to_errors_buffer` (`src/lua.rs:401`)
   writing `*errors*` (`ERRORS_BUFFER_NAME`, `src/lua.rs:32`; 49
   references across `src/` and `builtin/`), already used by
   `log_hook_error` (`src/lua_bindings/mod.rs:6061`),
   `log_statusline_provider_error` (`:6099`) and `log_buffer_removed_error`.
   This **improves the design**: Q#RD7 now records the refusal there
   too, so it survives the status line being cleared and leaves a trace
   on the unattended path.
8. **"Only path 1 is fully unattended" understated the problem.**
   Revision 1 said a raise on paths 2 and 3 "propagates out of the
   `pmacs.async` coroutine" without establishing where it lands. It
   lands nowhere useful: `step` (`builtin/runtime/async.lua:196`) routes
   an uncaught coroutine error to `pmacs.error`, **which is undefined**
   — 11 call sites in `builtin/`, zero definitions — so the `error(...)`
   fallback re-raises at the spawn site. **No caller reliably reports a
   raise**, which strengthens rather than weakens point 2's requirement.
9. **pmacs advertises no `workspace.workspaceEdit` capability at all.**
   `default_client_capabilities` (`src/lsp.rs:3242`) sends
   `"applyEdit": true` but no `workspaceEdit` object, so no
   `documentChanges`, no `resourceOperations`, and no `failureHandling`
   — `grep -rn failureHandling` over the tree returns **0**. Revision 1
   discussed batch semantics without noting that pmacs declares no
   failure-handling strategy. Named as ground truth (§1.11); **not
   fixed here** (§6).

### Revision 1

First cut. Established the bug and the four modes, the caller
inventory, the Emacs prior art, and the refusal recommendation.


## 0. Coherence impact (COHERENCE §20)

- **Journey step 6, "Receive language intelligence"** (§2), and by
  consequence **step 5, "Edit immediately"** — the loss is of exactly
  the edits step 5 grades as "genuinely excellent". §2's verdict table
  grades step 6 **Partial**; this lane does not raise that grade, it
  removes a way the step can destroy the user's work. Serves
  **Priority 1** ("treat regressions as release blockers") as a
  correctness floor rather than a feature.
- **Interaction islands: none added.** The recommended design adds no
  modal surface at all — it refuses and reports. *Revision 2 correction:*
  revision 1 additionally claimed that the rejected prompt option would
  have added a seventh dispatcher rung. It would not; a callback
  continuation reuses the existing minibuffer shadow (rung 4). The
  count stays at six either way, and §6's island budget is **not** an
  argument against prompting (§2.2).
- **Config registry: not adopted.** No knob is proposed; the refusal is
  unconditional (Q#RD1). An `lsp.confirm-server-edits`-style setting is
  the natural future adopter and is parked in §6.
- **Background-work attribution: unchanged.**
- **No audited claim in COHERENCE.md changes**, so under §25 no
  COHERENCE edit rides this PR. The `docs/active-work.md` lane for this
  PR does ride it, per that file's "When a PR is opened, give it a
  lane."


## 1. Ground truth (scouted and verified @ `ad41cf1`)

### 1.1 The bug, reproduced

`pmacs.buffer.apply_resource_op` with `kind = "delete"` destroys a
modified buffer and the file backing it. Reproduced in this worktree by
throwaway acceptance tests against `ad41cf1`, written, run, then removed
— they are the model for §5's pins, not shipped artefacts. Four modes:

**(a) The reported bug.**

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
arm skips the `remove_file` — and falls through to the buffer
reconciliation anyway:

```
file gone; buffers=3; buffer holds the only copy
result: Ok(())
buffers after=2
CONFIRMED: ignore_if_not_exists=true did ZERO fs work yet still destroyed the only copy.
```

The `create` arm returns early (`return Ok(())`) under the analogous
`ignore_if_exists` condition; the delete arm's `Err(NotFound)` branch
does not return.

**(c) `recursive = true` fails in the opposite direction.**

```
recursive delete result: Ok(())
inner file exists? false
buffers before=3 after=3
inner buffer still in registry? true
```

The tree goes; no buffer is reconciled, because the lookup is for the
directory path and buffers hold file paths. The most destructive arm
does the least reconciliation. Here the data survives — in an orphaned
buffer — which is strictly safer than (a), and is why the fix must not
be "make delete behave like the recursive case".

**(d) Removal is not `kill_buffer`.**

```
victim is the active buffer? true
window.buffer():is_valid() after delete => Ok(Boolean(false))
editor.file_path() after delete => Ok(Nil)
```
```
buffers now: 1
delete of the LAST buffer: Ok(())
buffers after: 0  (0 => registry driven empty)
```

`EditorCore::kill_buffer` (`src/editor_core.rs:4590`) refuses the last
buffer and rebinds every window to a fallback. This path does neither.
**(d) is parked** (§6, Q#RD8) and, per Q#RD5, must not be *widened* by
this lane.

### 1.2 There is no dirty check at any link in the chain

1. **The delete arm** (`src/lua_bindings/mod.rs:3313`) — stats, deletes,
   then `find_by_path`, then `remove_buffer_and_fire`. No `is_modified`.
2. **`remove_buffer_and_fire`** (`:1592`) — `registry.remove(id)` then
   `after_buffer_removed` (`:1602`), which clears keymaps, config, folds
   and fires `on_removed`. No dirty check.
3. **`BufferRegistry::remove`** (`src/buffer_registry.rs:127`) — one
   guard, and not this one:

   ```rust
   if let Some(buf) = self.buffers.get(&id)
       && buf.editing_in_progress()
   {
       return Err(RegistryError::ConcurrentEdit { ... });
   }
   ```

   That refuses re-entrant removal from inside an edit intercept
   (T M7.4). It says nothing about unsaved content.

The report's description is **accurate at every link**. Modes (b), (c)
and (d) are additional.

### 1.3 The ordering is disk-first, so no guard placed later can help

The arm performs the irreversible filesystem operation **before** it has
looked for a buffer. **A fix that adds a dirty check to the existing
buffer-reconcile block is not a fix** — it converts mode (a) into mode
(c), and the user's file is still gone. The check must happen in a phase
that precedes the filesystem call, which is what Q#RD2 introduces.

### 1.4 `is_modified` is available, singular, and the lookup around it is not

- One field, `Buffer::is_modified: bool` (`src/buffer.rs:164`); one
  accessor (`:473`); one public mutator `mark_clean` (`:488`).
  `Buffer::editing_in_progress()` (`:747`) is likewise a public getter,
  so both conditions Q#RD2 needs are inspectable without mutating.
- **Zero features refuse or confirm on unsaved state today.**
  `kill_buffer`, `editor.quit` (whose `editor.before-quit` veto hook has
  no subscriber) and `dired.revert` all ignore it; the only behavioural
  consumer is autosave's `gather` filter (`src/autosave.rs:363`), which
  *includes* rather than refuses. **This lane introduces the first
  refusal keyed on unsaved state** and should be read as setting that
  precedent.
- **The registry lookup is singular and duplicates are reachable.**
  `BufferRegistry::find_by_path` (`src/buffer_registry.rs:168`) returns
  the *first* match in insertion order — its own doc says "First buffer
  bound to `path`". `EditorCore::find_buffer_for_path`
  (`src/editor_core.rs:935`) normalizes and then delegates to it, so it
  inherits the singularity. And duplicates are creatable from public
  Lua: `pmacs.buffer.find_or_open` (`src/lua_bindings/mod.rs:3162`)
  dedups via `find_by_path`, but **`pmacs.buffer.from_file` (`:3112`)
  does not** — it calls `create_from_bytes` unconditionally and then
  `set_buffer_path`. Two `from_file` calls on one path yield two
  path-bound buffers, and a clean first match hides a modified second.
  This is why Q#RD6 requires a full scan rather than the existing
  wrapper.
- Lua reaches modified state as `buf:is_modified()` (`:1261`) and
  `pmacs.describe.buffer(id).modified` (`:6359`). It is **not** a key on
  the `pmacs.buffer` module table, so the preflight needs a new query
  (Q#RD3).

### 1.5 Who calls this, and what happens to a raised error

One production caller: `apply_workspace_edit`
(`builtin/runtime/lsp.lua:1301`), at `:1346`. Three callers of that:

| # | Call site | Origin | Disposition of a raise |
|---|---|---|---|
| 1 | `handle_server_requests` (`lsp.lua:1815`), call at `:1836` | **server-initiated** `workspace/applyEdit` | **Swallowed.** The pump runs under `pcall(handle_server_requests)` (`:1892`); the raise unwinds past the `pcall(pmacs.lsp.send_response, ...)` that answers the request, so the user sees nothing **and the server is never answered**. |
| 2 | LSP rename (`lsp.lua:2311`) | user, `M-x` | Raises out of the `pmacs.async` coroutine — see below. |
| 3 | code action apply (`lsp.lua:2373`) | user, `M-x` | as #2. |

**Revision 2 correction.** Revision 1 called only path 1 unattended.
Paths 2 and 3 are no better: `step` (`builtin/runtime/async.lua:196`)
handles an uncaught coroutine error by calling `pmacs.error` if it
exists and `error(...)` otherwise —

```lua
if not ok then
  if pmacs.error then
    pmacs.error("pmacs.async: coroutine raised: " .. tostring(yielded))
  else
    error("pmacs.async: coroutine raised: " .. tostring(yielded))
  end
  return
end
```

— and **`pmacs.error` is undefined**: 11 call sites across `builtin/`,
zero definitions. So the fallback always runs and re-raises at the spawn
site. **No caller reliably surfaces a raise to the user.** Hence Q#RD7:
the refusal travels as a value, never as an exception alone.

`src/rename.rs:25` documents the division of labour — `rename.rs` parses
and never mutates; Lua drives the primitives "so the application
strategy stays configurable". That strategy is what this framing picks.

### 1.6 A partial batch is already possible today — verified

The applier's loop (`lsp.lua:1340-1349`) calls the primitive
unprotected. Two delete ops where the second raises:

```
batch result: Err(... "apply_resource_op delete: No such file or directory (os error 2)")
a.txt still exists? false (false ⇒ partial batch)
```

The first op stayed applied. **Partial application on I/O error is the
status quo**, not something a refusal introduces. Data loss is strictly
worse than a failure class the code already tolerates.

It also shows the preflight's contract is narrower than its comment
implies. `lsp.lua:1287-1291` says the applier "refuses to mutate
*anything* unless every URI it touches resolves to a real file path
first". True — but URI resolution is the *only* precondition; the plan
loop (`:1302-1336`) validates nothing about the filesystem or the
registry. That loop is where Q#RD3's conflict check goes.

### 1.7 The batch is sequential; the protocol assigns pmacs no failure strategy

Claims about **the LSP specification** (3.18), each a direct quote, not
a paraphrase — and note carefully what they do *not* say.

**Sequential execution** — this is the load-bearing one, and it is
unconditional:

> "If resource operations are present, clients need to execute the
> operations in the order in which they are provided."

**Failure recovery is the client's declared choice, not a fixed rule:**

> "How the client recovers from the failure is described by the client
> capability: `workspace.workspaceEdit.failureHandling`"

`FailureHandlingKind` has four values, quoted from the spec's own
namespace block:

| Value | Doc comment (verbatim) |
|---|---|
| `Abort` | "Applying the workspace change is simply aborted if one of the changes provided fails. All operations executed before the failing operation stay executed." |
| `Transactional` | "All operations are executed transactionally. That means they either all succeed or no changes at all are applied to the workspace." |
| `TextOnlyTransactional` | "If the workspace edit contains only textual file changes they are executed transactionally. If resource changes (create, rename or delete file) are part of the change the failure handling strategy is abort." |
| `Undo` | "The client tries to undo the operations already executed. But there is no guarantee that this is succeeding." |

**Revision 3 correction.** Revision 2 read this as "the protocol assigns
`Abort` to any edit containing resource operations". **That is wrong.**
`Abort` is one of four strategies a *client* may advertise;
`Transactional` covers all operations and `Undo` attempts rollback. Only
`TextOnlyTransactional` degrades to abort in the presence of resource
changes, and that degradation is a property of *that* strategy, not of
resource operations in general.

**And pmacs advertises none of them** (§1.11). So the specification does
not tell us what pmacs should do here; it tells us the question is the
client's to answer. Revision 2 borrowed authority it did not have.

What survives, and is sufficient: **sequential execution is
unconditional**, which is why a snapshot preflight is necessarily
incomplete — an earlier op can change the facts a later op's
precondition was evaluated against. That, plus §1.6's *verified* pmacs
behaviour (a partial batch already happens today on I/O error), is the
whole basis for Q#RD3 and B2. The justification is that **partial
application is already what pmacs does and is safer than data loss** —
not that the protocol blesses it.

### 1.8 What prompting would actually cost — corrected

Revision 1 called prompting impossible. It is not. Establishing what is
and is not true:

**True, and verified:**
- The primitive cannot suspend. `apply_resource_op` is a synchronous
  Rust closure performing its `std::fs` calls inline; there is no yield
  point. A prompt therefore cannot be issued *from inside it* — the
  applier would have to be restructured into a continuation chain.
- `pmacs.minibuffer.read` (`src/lua_bindings/mod.rs:13380`) is
  asynchronous-by-callback, and `Minibuffer::accept`
  (`src/minibuffer.rs:334`) deliberately *returns* the callback rather
  than invoking it, because "firing user code from inside the minibuffer
  would re-enter the registry" (`:332`).
- **The minibuffer is a single slot that replaces without asking.**
  `Minibuffer::session: Option<MinibufferSession>` (`src/minibuffer.rs:71`),
  and `begin` (`:106`) is documented "**Replaces any existing
  session**". A prompt raised mid-batch while the user has a minibuffer
  open silently destroys the in-flight prompt and its callbacks.
- **There is no `y_or_n` helper in the tree** — a named deferral
  (`docs/dired-framing.md:854`).

**False, as revision 1 had it:**
- *"The server-initiated path cannot produce that answer."* It can.
  `pmacs.lsp.send_response` (`src/lua_bindings/mod.rs:9680`) takes
  `(server_id, request_id, result, err)` as ordinary values; nothing
  binds it to the pump's call frame, and `request_id` arrives on the
  event as a plain Lua value that can be stashed. A `workspace/applyEdit`
  **can** be answered on a later tick.
- *"It costs a seventh dispatcher shadow."* It does not. The minibuffer
  is already rung 4; a continuation reuses it.

**So the honest case against prompting** (§2.2) is scope, not
possibility: queuing, cancellation, collision with an already-active
single-slot minibuffer, and revalidation of every precondition after the
user turn — because the world moves during the turn, which is §1.7's
problem again, only worse.

### 1.9 Autosave cannot serve as a pre-delete backup

Verified against `src/autosave.rs`: there is no per-buffer write entry
point (the only public writer is `sweep`, `:261`, which walks the whole
registry); `sweep` skips clean buffers (`:363`); **removing a buffer
purges its recovery file** — the `on_removed` callback registered at
`builtin/runtime/autosave.lua:167` calls `discard_buffer` (`:511`), with
a sweep-time GC backstop (`:290-306`), pinned by
`tests/autosave_acceptance.rs:702`; and deleting the file flips the
recovery to `Stale`, which is never auto-offered.

### 1.10 Report channels — corrected

- `pmacs.editor.set_status` (`src/lua_bindings/mod.rs:13036`) is
  transient; it is cleared at the top of every `dispatch_key`.
- **Revision 2 correction: a durable surface exists.**
  `LuaHost::append_to_errors_buffer` (`src/lua.rs:401`) appends to
  `*errors*` (`ERRORS_BUFFER_NAME`, `src/lua.rs:32`), creating it on
  first use, and is the established idiom for "a callback failed and the
  user was not watching" — `log_hook_error`
  (`src/lua_bindings/mod.rs:6061`), `log_statusline_provider_error`
  (`:6099`), `log_buffer_removed_error`, and the config error path
  (`src/lua_bindings/config.rs:511`). Revision 1 claimed no such channel
  existed. It does, it is Rust-side, and Q#RD7 now uses it.

### 1.11 pmacs advertises no `workspace.workspaceEdit` capability

`default_client_capabilities` (`src/lsp.rs:3242`) sends `"applyEdit":
true` (`:3259`) inside its `"workspace"` block (`:3253`) but **no
`workspaceEdit` object at all**. So pmacs declares neither
`documentChanges` ("The client supports versioned document changes in
`WorkspaceEdit`s"), nor `resourceOperations` ("The resource operations
the client supports"), nor `failureHandling` ("The failure handling
strategy of a client if applying the workspace edit fails") —
`grep -rn "failureHandling"` over the tree returns **0**.

Two consequences worth stating plainly. pmacs applies resource
operations it never declared support for; and it declares no failure
strategy, so §1.7's `Abort` semantics are the de facto behaviour by
omission rather than by choice. **Neither is fixed by this lane** —
declaring capabilities changes what servers send, which is a behavioural
change needing its own evidence (§6). It is recorded because a framing
about batch failure semantics that did not notice pmacs declares none
would be describing half the system.

### 1.12 The rename arm is more careful, and differently careful

`"rename"` (`src/lua_bindings/mod.rs:3291`) does `std::fs::rename`, then
`find_by_path`, then `set_buffer_path` — it **rebinds**, preserving
contents and modified state. Delete **destroys**. Rename treats the
buffer as the valuable thing and the path as a mutable attribute; delete
treats the buffer as a cache of the file.

Both arms share the §1.4 lookup defects.

**Cross-lane state, re-checked directly at revision 3 rather than
inherited.** PR #171 (dired Stage 2) is at **revision 7, `fd7ae37`,
merge-base `ad41cf1`, 0 commits behind `main`**. Revision 2 of this
document described it as "OPEN, STALE, 153 commits behind, under
re-scout" and said it claimed the rename side only; **both halves of
that were out of date**. Its revision 6 assigned rename *and* delete
reconciliation to Stage 2a with the opposite policy — `reconcile_delete`
killing unmodified buffers and keeping modified ones alive, so the file
is deleted and the modified buffer orphaned, with that orphaning named
as accepted residue.

**The settled split** (identical wording carried by both lanes):

> #186 owns the urgent **pre-filesystem refusal** for synchronous
> `apply_resource_op`. #171 later owns **full post-delete lifecycle
> reconciliation**, including the **async race where a buffer becomes
> modified after dired dispatch**. #171's revision 7 adopts the refusal
> and stops saying LSP intentionally deletes modified files.

#171 revision 7 has adopted this from its side: its Q#DR18 takes this
document's Q#RD1 refusal rather than re-deciding it, and it records the
reason the refusal cannot simply be extended to cover dired — **dired
never calls `apply_resource_op`**. It calls `pmacs.fs.remove`, which
dispatches a worker, so a synchronous refusal inside the primitive
cannot reach it at any strength. That asynchronous window is #171's, and
naming it here is what keeps this lane from appearing to close a defect
it does not close.

The older in-tree note at `docs/dired-framing.md:807-819` (Stage 1-era)
still describes the rename-side lookup defect accurately, but it is
superseded as a statement of plan by #171's Stage 2 document, which
exists only on that branch.

### 1.13 Prior art — claims about **Emacs**, not pmacs

Verified against `lisp/progmodes/eglot.el`, `emacs-mirror/emacs`
`master`.

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

The buffer is killed **before** the file is deleted. Note also that the
`exists` guard means `ignoreIfNotExists` does **not** fall through to
the buffer kill — the asymmetry mode (b) exposes in pmacs.

**Revision 3 precision.** Revision 2 asserted that "in Emacs
`kill-buffer` on a modified file-visiting buffer prompts", which was a
paraphrase carrying real weight in the argument. It is true, but the
mechanism is not the obvious one and the difference matters. From
`Fkill_buffer` (`src/buffer.c`):

```c
    /* Is this a modified buffer that's visiting a file? */
    modified = !NILP (BVAR (b, filename))
      && BUF_MODIFF (b) > BUF_SAVE_MODIFF (b);

    /* Query if the buffer is still modified.  */
    if (INTERACTIVE && modified)
      {
	/* Ask whether to kill the buffer, and exit if the user says
	   "no".  */
	if (NILP (calln (Qkill_buffer__possibly_save, buffer)))
	  return unbind_to (count, Qnil);
```

and `INTERACTIVE` is (`src/commands.h`):

```c
/* Nonzero if input is coming from the keyboard.  */

#define INTERACTIVE (NILP (Vexecuting_kbd_macro) && !noninteractive)
```

So the gate is **"Emacs has a keyboard"**, not "this function was
reached through `call-interactively`". Eglot's `do-delete` calls
`kill-buffer` programmatically from Lisp and **still prompts** in a
normal session — but **does not** in batch mode or while a keyboard
macro is executing. Revision 2's sentence was right for a reason it
never established, and false in two environments it never considered.

Two riders, both verified: eglot ignores `kill-buffer`'s return value,
so declining the kill still deletes the file — the buffer and its text
survive, which is milder than pmacs's failure but is not a refusal. And
the prompt is **not** `buffer-offer-save`, whose own docstring says so:
"Note that this option has no effect on `kill-buffer'; if you want to
control what happens when a buffer is killed, use
`kill-buffer-query-functions'."

*Ordering note (rev 2, sharpened at rev 3):* Emacs's ordering is **not**
what Q#RD2 adopts. Emacs can afford buffer-first because `kill-buffer`
is itself the consent gate — conditionally, per the `INTERACTIVE` gate
above. pmacs has no such gate at all, so it validates first and
reconciles last (Q#RD2), which yields the same safety unconditionally
and without firing callbacks against a file that still exists.

**Eglot confirms server-initiated edits by default, as a whole-batch
decision taken before anything is applied.** `eglot-confirm-server-edits`
defaults to `'((t . maybe-summary))`; `prepare` builds closures touching
nothing, then the decision, then `apply-all`. The `maybe-*` decisions
skip the prompt only when the batch is `peaceful`:

```elisp
(peaceful
 (and
  all-text-edits
  (cl-loop for op in prepared
           always (find-buffer-visiting (cadddr op)))))
```

`all-text-edits` is a conjunction over the whole batch, so a batch
containing any create/rename/delete **always** prompts under the
default.

### 1.14 `apply_resource_op` has no direct test coverage

`grep -rn "apply_resource_op" tests/ src/` returns **4 lines**: one doc
comment in `src/rename.rs` and three inside the binding's own
definition. Zero tests name it.

One indirect acceptance exercises it:
`m4_15_workspace_edit_resource_ops_apply_in_order`
(`tests/m4_acceptance.rs:4014`), driven by the `resourceops` mode of the
fake server (`src/bin/pmacs_fake_lsp.rs:834`). **Its deleted `c.rs` is
never opened**, so the entire buffer-reconciliation half is untested.
That suite and that fake are where §5's pins belong.

### 1.15 External-claim audit (revision 3)

Two external-spec overclaims in two revisions is a pattern, not an
accident, so every claim in this document that is **not** about this
repository is listed here with its evidence. The standing rule for
revision 4 onward: an external claim carries a direct quote or it is
marked not established.

| # | Claim | Source | Status |
|---|---|---|---|
| 1 | Resource ops execute in provided order | LSP 3.18 `WorkspaceEdit` | **Quoted**, §1.7. Unconditional. |
| 2 | Recovery is described by the client's `failureHandling` | LSP 3.18 | **Quoted**, §1.7. |
| 3 | The four `FailureHandlingKind` doc comments | LSP 3.18 | **Quoted verbatim**, §1.7 table. |
| 4 | ~~The spec assigns `Abort` to resource-op edits~~ | — | **WITHDRAWN** (P1-2). Never supported; it conflated one client-selectable strategy with a protocol rule. |
| 5 | `documentChanges` / `resourceOperations` / `failureHandling` capability doc comments | LSP 3.18 | **Quoted**, §1.11. |
| 6 | eglot's `do-delete` body | `lisp/progmodes/eglot.el`, emacs-mirror master | **Quoted from source**, §1.13. |
| 7 | `eglot-confirm-server-edits` default and the `peaceful` conjunction | same | **Quoted from source**, §1.13. |
| 8 | Emacs prompts when killing a modified file-visiting buffer | `src/buffer.c` + `src/commands.h` | **Quoted at rev 3**, §1.13. Was a bare paraphrase at rev 2; the real gate is `INTERACTIVE`, i.e. keyboard present — not `call-interactively` — so it does **not** hold in batch or during a keyboard macro. |
| 9 | `buffer-offer-save` does not affect `kill-buffer` | `lisp/files.el` docstring | **Quoted**, §1.13. |

Not established, and therefore not claimed anywhere in this document:
what `lsp-mode` (as distinct from eglot) does with `DeleteFile`; and the
exact `ApplyWorkspaceEditResult` field list beyond `applied` and
`failureReason`, which this document uses only because pmacs's own code
already sends them (`builtin/runtime/lsp.lua:1841`).


## 2. The decision space

### 2.1 Recommended — validate before mutating, at the primitive; conflict-check early, in the applier

**The primitive refuses before touching disk. The applier catches what
it can early, and reports honestly what it cannot.**

**Layer 1 — the primitive (the invariant).** The delete arm becomes four
ordered phases:

```
stat / no-op decision  →  enumerate and validate affected buffers
                       →  mutate the filesystem
                       →  reconcile the registry
```

Validation inspects; it does not remove. If any affected buffer is
modified — or is mid-edit (`editing_in_progress`) — the op returns an
error having touched nothing. Because validation removes nothing, a
filesystem failure leaves every buffer intact automatically, and
`on_removed` still fires only in the reconcile phase, i.e. with the path
already gone, preserving today's invariant.

**Layer 2 — the applier (early conflict check + robust reporting).**
`apply_workspace_edit`'s existing plan loop gains a modified-buffer
conflict check for delete ops and returns its existing `nil, message`.
This is a **filter, not a transaction** (§1.7): it catches the common
case cheaply, before anything is mutated, and it is honest that a
sequential batch can still refuse mid-flight. What makes mid-flight
refusal survivable is Q#RD7: each primitive call is wrapped, every
failure becomes `nil, message`, the origin buffer is restored
best-effort, and the unattended caller **always** answers the server.

Neither layer is redundant. Layer 1 alone leaves every batch failure
reported through a channel that does not work (§1.5). Layer 2 alone
leaves the primitive armed for direct callers — `pmacs.buffer.apply_resource_op`
is public Lua API, and dired Stage 2's own plan names delete
reconciliation as its 2a substrate.

**Why this beats the runners-up, in one sentence each:** it is the only
option that puts the check strictly before the irreversible step without
either firing callbacks into a half-changed world (buffer-first) or
inventing a recovery surface (backup), and it is the only one whose cost
is bounded by this lane.

### 2.2 Prompt the user — rejected on scope, not on possibility

**Revision 2 rewrite.** Revision 1 argued impossibility on three
grounds; two were wrong (§1.8) and are withdrawn. The surviving argument
is narrower and is about cost:

- **The applier must become a continuation chain.** The primitive cannot
  suspend (§1.8), so the remaining plan has to be carried as a closure
  across the user turn — with cancellation, and with **revalidation of
  every precondition afterwards**, because the world moves during the
  turn. That is §1.7's sequential-batch problem with a human-scale delay
  inserted into it.
- **The minibuffer is a single slot that replaces without asking**
  (`Minibuffer::begin`, "Replaces any existing session"). A prompt
  raised while the user is mid-`M-x` destroys their in-flight prompt.
  Queuing is therefore a prerequisite, and no queue exists.
- **Deferred answers need a pending-request ledger.** Answering
  `workspace/applyEdit` later is possible (§1.8) but means retaining
  `(server_id, request_id)` across ticks and deciding what happens if
  the server dies first. `purge_dead_pending` exists for *client*
  requests; there is no equivalent for held server requests.

None of that is impossible; all of it is a separate lane with its own
framing. **Refusing is the correct move for a live data-loss bug**, and
a prompt can later *loosen* an unconditional refusal without either
change invalidating the other.

### 2.3 Save first, then delete — rejected

Silently converts an unsaved edit into a committed one and then destroys
it — more destructive, not less, because it overwrites the on-disk
original immediately before removing the file. It also cannot be relied
on: `save_inner` (`src/editor_core.rs:1908`) refuses at `:1917` when the
file changed on disk since it was read, so the fallback question is
unanswered and we are back to refusing.

### 2.4 Back up the contents somewhere recoverable — rejected

Rejected on evidence (§1.9): the decisive fact is that **removing the
buffer deletes the recovery file**, so the backup is destroyed by the
operation it exists to survive. Building a side-store outside
`autosave/` means a second recovery surface with its own discovery, GC
and lifecycle, to make a destructive operation *feel* safe.

### 2.5 Key the behaviour on LSP-versus-user provenance — rejected

`apply_resource_op` takes no provenance argument and there is no ambient
caller identity. Adding one makes the primitive's safety depend on a
caller-supplied flag — any caller that omits it is unguarded, which is
the failure mode the lane exists to remove. COHERENCE §10 (extension
trust classes) is unbuilt, so there is no trust dimension to key on.
**The refusal is unconditional and provenance-blind.**


## 3. Decisions

### Q#RD1 — Refuse. Do not prompt, do not save, do not back up — **SETTLED YES**

A delete whose target set contains a modified buffer **fails**, changing
nothing on disk and nothing in the registry. This is the first refusal
in the codebase keyed on unsaved state (§1.4) and is intended as the
precedent for `kill_buffer` and `editor.quit`, which have the same gap.

### Q#RD2 — Validate before mutating; reconcile last — **REWRITTEN at rev 2**

Four phases, in order: **stat/no-op decision → enumerate and validate
affected buffers → mutate the filesystem → reconcile the registry.**

- **Validation inspects only.** It checks `Buffer::is_modified()` and
  `Buffer::editing_in_progress()` (`src/buffer.rs:473`, `:747`) across
  the affected set. Nothing is removed, so nothing can be lost if a
  later phase fails.
- **`editing_in_progress` moves from discovery to validation.** Today a
  `ConcurrentEdit` refusal from `BufferRegistry::remove` arrives *after*
  the file is gone. Checking it during validation means a delete invoked
  from inside the target's own edit intercept refuses before disk.
- **`on_removed` still observes the path already gone.** Reconciliation
  is the last phase, so the invariant revision 1 would have broken is
  preserved. This is the specific defect revision 1's buffer-first
  ordering introduced, and it is why that ordering is withdrawn.
- **A filesystem failure leaves buffers untouched**, automatically
  rather than by compensation.

### Q#RD3 — The preflight is an early conflict check, **not** a transaction — **DOWNGRADED at rev 2**

`apply_workspace_edit`'s plan loop gains a modified-buffer conflict
check for delete ops and returns its existing `nil, message`. It is
described in the code comment and here as a **filter**:

- **What it guarantees:** when the conflict is visible at plan time,
  nothing in the batch is mutated at all, and the user gets one clear
  message.
- **What it does not guarantee, stated plainly:** `documentChanges` are
  sequential (§1.7). An earlier text edit can dirty a clean buffer, and
  an earlier rename can move a modified buffer *into* a later delete's
  subtree, after the snapshot. Then the preflight passes and the
  primitive refuses mid-batch, leaving earlier operations applied —
  which is `FailureHandlingKind.Abort`, the strategy the spec itself
  assigns to any edit containing resource changes.
- Revision 1 called this "whole-batch atomicity" and said "nothing in
  the batch is mutated". **That was false and is withdrawn.**

The check needs a path-keyed modified query that Lua lacks (§1.4). It
must be **one** query shared with the primitive's validation phase, so
the two cannot drift apart.

### Q#RD4 — `ignore_if_not_exists` short-circuits at **both** layers — **WIDENED at rev 2**

When the path is absent and `ignore_if_not_exists` is set, the op is a
no-op:

- **Primitive:** return early without touching the registry — the
  `create` arm's existing idiom and Eglot's `exists` guard (§1.13).
- **Preflight:** must **not** reject the batch merely because a modified
  buffer still names that absent path. Revision 1 applied this only to
  the primitive, which would have made the preflight refuse an op the
  primitive treats as a no-op — a refusal with no underlying
  destruction, i.e. a false positive that blocks legitimate edits.

Mode (b) is not a special case of the main bug; it is a missing early
return, and a fix aimed only at the "we actually deleted something"
branch leaves it live.

### Q#RD5 — Recursive deletes are **inspected** tree-wide but **reconciled** exact-path — **SETTLED YES, NARROWED at rev 2**

Mode (c) proves `recursive = true` reconciles nothing, so an exact-path
guard is bypassed by the most destructive arm. Therefore:

- **Validation is prefix-aware**: every buffer whose path lies beneath
  the deleted directory is inspected, and any modified one refuses the
  op. Without this the guard has a trivial reachable bypass.
- **Reconciliation is not widened**: after a successful *clean*
  recursive delete, descendant buffers are left exactly as today —
  orphaned and clean. **Removing them now would promote mode (d)'s
  dangling-window and last-buffer defects from an exact-path defect to a
  tree-wide one**, which is precisely the parked lifecycle work this
  lane must not expand into (Q#RD8).

The asymmetry is deliberate and is the point: **inspect widely, mutate
narrowly.**

**Boundary with dired — restated at rev 3.** The settled split (§1.12,
quoted there verbatim and carried identically by #171) is:

> #186 owns the urgent **pre-filesystem refusal** for synchronous
> `apply_resource_op`. #171 later owns **full post-delete lifecycle
> reconciliation**, including the **async race where a buffer becomes
> modified after dired dispatch**.

**The stale justification is withdrawn.** Revision 2 supported taking
the delete side now by citing the ledger's "OPEN, STALE, 153 commits
behind, under re-scout" assessment of #171. That re-scout has finished;
#171 is at revision 7, integrated to `ad41cf1`. **The conclusion is
unchanged and rests on urgency alone** — this is a live data-loss bug
with a reproduction, and a refusal that must precede the filesystem call
cannot be deferred to a lane that acts after it. It no longer rests on
any claim about #171's freshness, and it must not be re-argued from one.

### Q#RD6 — The shared query scans **all** path-bound buffers — **REWRITTEN at rev 2**

Revision 1 said the guard would use `EditorCore::find_buffer_for_path`.
**That is wrong** and is withdrawn: it normalizes but delegates to the
singular, first-match-only `find_by_path` (§1.4), and duplicate
path-bound buffers are reachable from public Lua via
`pmacs.buffer.from_file`. A clean first match would hide a modified
second — a silent guard bypass.

The shared query therefore:

- **scans every path-bound buffer**, returning all matches rather than
  the first;
- **normalizes once** and compares normalized forms, so a raw-path
  lookup cannot miss a stored normalized path;
- **matches with component-aware `Path::starts_with`**, not string
  prefix — so `/tree` does not match `/tree-sibling`;
- is the **single** query used by both the primitive's validation phase
  and the Lua preflight (Q#RD3).

**This lane claims the query.** The boundary's rule is "whichever lands
first owns the query and the other adopts it", and #171 revision 7
records that this rule's four clauses are character-for-character what
it had written independently for `reconcile_delete`. To stop both lanes
asserting ownership: **#186 owns and implements the shared walk**, #171
adopts it and extends it to `reconcile_rename`. If #171 lands first the
claim inverts and this decision is what gets deleted — but it is stated
in one direction so the duplicate resolves rather than persisting.

"Modified" is `Buffer::is_modified()`. No new notion of dirtiness.

Explicitly **not** guarded: a clean buffer. A delete whose target is
open but unmodified proceeds and removes the buffer, as today.
Overreach would break `m4_15` and would fail legitimate deletes for
users who merely have the file open.

### Q#RD7 — One reporting seam at the server-request boundary — **REWRITTEN at rev 3**

§1.5 established that **no** caller reliably surfaces a raise. Revision
2 answered that with three promises that did not compose into anything
implementable; revision 3 replaces them with **one seam**.

**Where the seam is: the server-request boundary**, i.e. the
`workspace/applyEdit` arm of `handle_server_requests`
(`builtin/runtime/lsp.lua:1833-1843`). Everything below hangs off that
single point.

- **Wrap parse *and* apply, not apply alone.** Revision 2 wrapped "every
  primitive call inside `apply_workspace_edit`", which does not cover
  `pmacs.lsp._parse_workspace_edit` — it is called at `lsp.lua:1835`,
  one line **above** `apply_workspace_edit`, and it is fallible
  (`lua_to_json(edit)?`, `src/lua_bindings/mod.rs:10161`). A parse
  failure therefore escaped, was swallowed by
  `pcall(handle_server_requests)`, and left the server unanswered — the
  exact defect being fixed, one line out of scope.

  **Of the two options offered in review, this revision picks wrapping
  parse-plus-apply** rather than narrowing "always answers" to applier
  execution failures. Reason: the narrow option documents a hole instead
  of closing one, and the wrap already exists — it moves up one line.
  With it, **"always answers" is true without qualification**.
- **Every failure becomes a value.** Refusal, I/O error, and parse
  failure all converge on the existing `nil, message` shape, which all
  three callers already handle. No exception escapes the applier.
- **The unattended caller always answers**: `{ applied = false,
  failureReason = ... }` in every failure case, including parse failure.
- **The durable trace is written at this boundary, not in the
  primitive.** Revision 2 promised logging through
  `LuaHost::append_to_errors_buffer` (`src/lua.rs:401`). Two problems,
  both confirmed: it is **private**, so it is not callable from where
  the promise was made; and a **Lua preflight** rejection never reaches
  the Rust primitive at all, so primitive-side logging would miss the
  common unattended case entirely. So: a **narrow Lua-callable surface**
  that appends one attributed record to `*errors*`, invoked at the
  server-request boundary **after any `applied = false`**, with the
  label `lsp:workspace/applyEdit`. One call site, one label, reachable
  from the layer that actually knows the outcome.
- **The origin buffer is restored best-effort on the failure path too.**
  Today `pcall(pmacs.buffer.find_or_open, origin)` runs only after a
  successful loop; an early failure return would strand the user in
  whatever buffer the last op left active.
- The message **names the buffer** and says what to do. Not a bare
  errno.

Acceptance 13 tests **both** halves of the boundary — the response the
server receives *and* the `*errors*` record — because revision 2 tested
only the first while promising the second.

### Q#RD8 — The window/last-buffer defects do **not** land here

Mode (d) is real and parked (§6). It is a different failure from data
loss, and it is shared with `pmacs.buffer.remove`.

**The trap that makes the obvious fix wrong:** the two removal paths
clean *disjoint* sets. `kill_buffer` handles the last-buffer refusal,
`round_trip_buffers`, side-window collapse and window rebinding, but
**not** keymaps, config, folds or `on_removed` callbacks;
`remove_buffer_and_fire` handles exactly the latter and none of the
former. Neither is a superset, so "just call `kill_buffer` instead"
would silently regress four cleanups.

**Revision 3 — that lane now exists.** Revision 2 said mode (d) "needs
its own lane and a census", which was true when written and is not now.
#171's `reconcile_delete` composes both phases for every id it kills and
reroutes `apply_resource_op`'s delete arm through it, and #171 revision
7 records the disjoint-set trap independently. So mode (d) is **owned,
not unowned**, and per Q#RD5 this lane's job is narrower than it looked:
not merely "don't fix it here" but **don't enlarge the surface #171 has
to fix**.

### Q#RD9 — **WITHDRAWN at rev 2**

Revision 1 proposed that, after a buffer-first removal, a filesystem
failure would leave the buffer unrestored. Q#RD2's phase ordering makes
the situation unreachable: nothing is removed before the filesystem
mutation succeeds, so there is no lost buffer to restore. The decision
number is retained rather than reused, so review can see it went away
rather than being renumbered.

### Q#RD10 — Validate every match; reconcile today's first match only — **NEW at rev 3**

Q#RD6 makes validation scan **all** path-bound buffers. Revision 2 never
said what *reconciliation* does afterwards when several match, which
left the common duplicate case undefined. It is now decided:

- **Validation: every match.** If any buffer bound to the path — or
  beneath it, for a recursive delete — is modified, the op refuses.
  A clean first match must not be able to hide a modified second (§1.4).
- **Reconciliation: exactly today's behaviour.** After a *successful*
  delete, the single first exact-path match is removed, as
  `find_by_path` does now. Additional clean duplicates are left in
  place.

**Why not remove them all.** Every extra removal goes through
`remove_buffer_and_fire`, which is phase 2 without phase 1 (Q#RD8) — so
removing N duplicates creates up to N dangling windows and brings the
registry N steps closer to empty. That is precisely the parked defect
Q#RD5 is written to contain, and widening it here would hand #171 a
larger problem in exchange for tidiness this lane does not need.

**The honest cost**, stated rather than buried: a surviving clean
duplicate is left bound to a path that no longer exists — the same
orphan shape as mode (c), on a narrower trigger. It is **residue handed
to #171**, whose lifecycle transaction can then remove all matches
safely because it composes both phases. This lane's contract is that no
*unsaved* work is lost, not that the registry ends tidy.

Acceptance 14 pins both directions, so neither widening nor narrowing
can happen silently.

## 4. Bets (falsifiable)

- **B1 — Refusing breaks no legitimate server workflow.** A server
  deleting a file the user has unsaved edits in is a conflict the user
  must resolve. Falsified by a real server whose normal operation
  deletes files the user is actively editing.
- **B2 — Mid-batch refusal is acceptable because partial application is
  already what pmacs does, and is safer than data loss.**
  *Rewritten at rev 3 (P1-2).* Revision 2 rested this on the protocol
  "assigning `Abort`" to resource-op edits, which it does not (§1.7):
  recovery is the client's advertised choice and pmacs advertises none.
  The bet now stands on repository evidence — §1.6 **verified** that an
  op failing mid-batch leaves earlier ops applied on `main` today — plus
  the ordering judgement that a partial refactor the user can see and
  redo beats unsaved work they cannot recover. Falsified if a server is
  found that requires transactional application and degrades badly under
  partial application. Acceptance 12 pins the observable behaviour
  either way.
- **B3 — Prefix-aware validation does not over-refuse.** Falsified if a
  common workflow deletes a directory while an unrelated modified buffer
  sits beneath it and the refusal is judged unhelpful.
- **B4 — Leaving clean descendants orphaned is the lesser evil**
  (Q#RD5). Falsified if orphaned clean buffers after a recursive delete
  prove more disruptive than the tree-wide lifecycle defect that
  removing them would create.


## 5. Acceptance

Each criterion states the **pre-image it must fail against**. A test
that passes against its pre-image has no bite and is rejected.

1. **A delete op targeting a modified buffer refuses, and the file
   survives.** Assert together: the call fails, the buffer is still in
   the registry with its exact unsaved text, and `path.exists()` is
   still true.
   *Bite:* fails against `ad41cf1` unmodified. **Asserting only that the
   buffer survived is vacuous** — that is mode (c)'s existing behaviour.
   The `exists()` assertion carries the bite.

2. **A delete op targeting a *clean* open buffer still succeeds**, file
   removed and buffer removed.
   *Bite:* fails against an over-broad guard that refuses whenever a
   buffer is open. Assert **both directions**.

3. **A filesystem failure preserves the clean buffer** (Q#RD2). Force
   the fs mutation to fail (e.g. a non-empty directory without
   `recursive`) and assert the buffer is still present and intact.
   *Bite:* fails against revision 1's buffer-first ordering, which would
   have removed the buffer and then failed.

4. **`on_removed` observes the path absent** (Q#RD2). Register an
   `on_removed` callback that stats the path and records the result;
   assert it saw the path already gone.
   *Bite:* fails against revision 1's buffer-first ordering, under which
   the callback would observe the file still present. This is the pin
   that keeps the phase order from silently regressing.

5. **A delete called from inside the target's own edit intercept refuses
   before disk** (Q#RD2). Assert the file still exists.
   *Bite:* fails against `ad41cf1`, where `ConcurrentEdit` is discovered
   only at removal time — after `remove_file` has already run.

6. **Duplicate path-bound buffers cannot hide a modified copy** (Q#RD6).
   Create two buffers on one path via `pmacs.buffer.from_file`, leave
   the first clean and modify the second, then delete.
   *Bite:* fails against any first-match lookup, including
   `EditorCore::find_buffer_for_path` — which is exactly what revision 1
   specified.

7. **Component-prefix false positives are rejected** (Q#RD6). A modified
   buffer under `/tree-sibling` must **not** block a recursive delete of
   `/tree`.
   *Bite:* fails against a string-prefix implementation. Pairs with
   criterion 8 so both directions of the prefix rule are pinned.

8. **`recursive = true` over a directory containing a modified buffer's
   file refuses, and the whole tree survives** (Q#RD5). Assert the inner
   file still exists — not merely that the buffer does, which is already
   true today (mode (c)).
   *Bite:* fails against exact-path-equality validation.

9. **A clean recursive delete leaves descendant buffers orphaned, not
   removed** (Q#RD5). Assert the descendant buffer is still in the
   registry after a successful recursive delete.
   *Bite:* fails against an implementation that widens reconciliation to
   the tree. This pin exists specifically to stop the parked defect from
   being enlarged, and it is expected to look odd — it asserts today's
   imperfect behaviour deliberately.

10. **`ignore_if_not_exists = true` on an absent path leaves a modified
    buffer intact** (Q#RD4), reproducing mode (b): file removed behind
    pmacs's back first, then the op.
    *Bite:* fails against a fix guarding only the branch where the fs
    delete actually ran.

11. **Absent-plus-ignore succeeds through the real server pump**
    (Q#RD4, layer 2). Drive it end to end and assert the batch is
    **not** refused and the server is told `applied = true`.
    *Bite:* fails against a preflight that rejects on the presence of a
    modified buffer without consulting `ignore_if_not_exists` — the
    false-positive Q#RD4 exists to prevent.

12. **Edit-then-delete and rename-into-delete still answer the server**
    (Q#RD3, Q#RD7). Two batches that defeat the snapshot preflight: one
    where an earlier text edit dirties the buffer a later op deletes,
    one where an earlier rename moves a modified buffer into a later
    delete's subtree. Assert in both cases that the server receives
    `applied = false` with a non-empty `failureReason`.
    *Bite:* fails against `ad41cf1` (the raise is swallowed at
    `lsp.lua:1892` and no response is sent) **and** against a
    preflight-only fix that claims atomicity — these are the cases
    revision 1's atomicity claim asserted could not happen.

13. **The refusal reaches the server on the unattended path AND leaves
    the durable trace** (Q#RD7). Assert **both**: the server receives
    `applied = false` with a non-empty `failureReason`, *and* `*errors*`
    contains a record carrying the `lsp:workspace/applyEdit` label. The
    user-initiated paths additionally report on the status line naming
    the buffer.
    *Bite:* fails against a fix that refuses by raising. A direct-call
    test on `apply_resource_op` does **not** satisfy this and is
    rejected as insufficient — the guard must be pinned through the
    outermost user-reachable seam. **The `*errors*` half fails against
    revision 2**, which promised the trace and tested only the response;
    asserting the response alone is what let that gap survive a round.

14. **Clean duplicates: every match validated, one match reconciled**
    (Q#RD10). Two clean buffers bound to one path; delete succeeds.
    Assert exactly one is removed and one remains.
    *Bite:* fails in **both** directions — against an implementation
    that removes all matches (widening the parked defect) and against
    one whose validation only consulted the first match. Pair with
    criterion 6, which covers the modified-second case; this one covers
    the all-clean case that criterion 6 cannot see.

15. **A parse failure still answers the server** (Q#RD7). Feed the
    `workspace/applyEdit` arm an edit payload that makes
    `_parse_workspace_edit` fail, and assert the server still receives
    `applied = false` with a `failureReason`.
    *Bite:* fails against `ad41cf1` **and** against revision 2's
    proposed wrap, which covered `apply_workspace_edit` only and left
    the parse one line outside — the concrete reason "always answers"
    was untrue as written.

16. **`m4_15_workspace_edit_resource_ops_apply_in_order` stays green
    unmodified**, pinning no-regression from outside. Its `c.rs` is
    never opened, so it exercises exactly the unguarded case that must
    keep working (Q#RD6).

17. **Every new test is checked with `scripts/bite`** and none reports
    VACUOUS.


## 6. Parked — not deferred-and-forgotten

- **Mode (d): the dangling window and the emptiable registry** (§1.1,
  Q#RD8). **Owned by #171** as of its revision 7 — `reconcile_delete`
  composes both removal phases and reroutes `apply_resource_op`'s delete
  arm through it. Revision 2 of this document called it unowned; that
  was true when written and is not now. Q#RD5 and Q#RD10 are written to
  avoid enlarging what that lane must fix.
- **`pmacs.fs.remove` is guarded by neither lane — explicitly out of
  scope here** (named in #171 revision 7 §11). After both lanes land,
  the refusal sits at the `apply_resource_op` primitive (this lane) and
  in dired's policy layer (#171), but `pmacs.fs.remove` is public Lua
  API with **no dirty check of its own**, so a third caller inherits
  neither guard — the guards are one layer *above* it on each side.
  Verified latent rather than live: `pmacs.fs.remove`
  (`builtin/runtime/fs.lua:187`) has **zero production callers**, its
  only references being `tests/m8_1_acceptance.rs:438`, `:439`, `:472`.
  **This lane does not extend scope to cover it.** It belongs with the
  primitive-level fs guards, i.e. #171's `pmacs.fs.*` work or a
  successor lane — recorded here because "both lanes guard deletion"
  otherwise reads as a claim that the primitive is guarded, and it is
  not.
- **`kill_buffer` and `editor.quit` have the same gap** (§1.4).
  `editor.before-quit` exists as a veto channel with no subscriber. This
  lane sets the precedent; those are separate lanes.
- **Declaring `workspace.workspaceEdit` capabilities** — `documentChanges`,
  `resourceOperations`, `failureHandling` (§1.11). Declaring them changes
  what servers send, so it needs its own evidence and its own lane.
- **`pmacs.error` is undefined** (§1.5) — 11 dead call sites in
  `builtin/`, including the one that is supposed to surface every
  uncaught async coroutine error. A known standing defect, widened in
  relevance by this framing but not fixed by it.
- **A `y_or_n` helper**, minibuffer queuing, and a held-server-request
  ledger — the three prerequisites that would make §2.2 cheap rather
  than merely expensive.
- **`lsp.confirm-server-edits`**, the config-registry adopter that would
  let a user loosen Q#RD1 once a prompt mechanism exists.
- **The rename side of prefix-aware, normalizing lookup** — dired
  Stage 2's, per Q#RD5.


## 7. Gates

Full suite per `CLAUDE.md`: `cargo fmt --check`; `cargo clippy
--workspace --all-targets -- -D warnings` as its own step; `cargo test
--lib`; `cargo test --lib --features crdt`; the touched acceptance
suites; `cargo test --test m4_acceptance -- --skip basedpyright`;
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`; `git diff --check`.

Touched suites: **`m4_acceptance`** (the resource-op home, §1.14) and
`lsp_dispatch_seams_acceptance`. `dired_acceptance` and
`autosave_acceptance` are watch items — the former for Q#RD6's shared
lookup, the latter because `on_removed` ordering (acceptance 4) is where
autosave's `discard_buffer` hangs.

Gate the pushed tree, not the worktree — commit first, then gate.

While the document is still PROPOSED the gate is `git diff --check` plus
a docs read; the full suite runs once implementation lands on this
branch.


## 8. Branch plan

`resource-op-delete-guard`, worktree `../pmacs-resource-op-delete`, one
branch, **one PR — #186, which becomes the implementation PR.**
Revision 2 withdraws revision 1's two-PR plan, which conflicted with
one-feature/one-branch/one-PR: the framing is revised in place, and once
approved the implementation commits land on this same branch.

**Implementation does not begin until the user approves this revision.**

Files the implementation will touch: `src/lua_bindings/mod.rs` (the
delete arm's four phases and the shared all-buffers query),
`builtin/runtime/lsp.lua` (the conflict check, per-op wrapping, origin
restore, always-answer), `tests/m4_acceptance.rs` and
`src/bin/pmacs_fake_lsp.rs` (fake modes for the blocked delete, the
edit-then-delete and rename-into-delete batches, and absent-plus-ignore).
It will **not** touch `src/daemon.rs`, `pmacs-protocol/`,
`builtin/runtime/dired.lua`, `docs/agent-handoff.md` or `COHERENCE.md`.
No protocol change.

**Ownership note — restated at rev 3 against #171 revision 7.** The
settled split is quoted in §1.12 and Q#RD5 and is carried identically by
both lanes. Concretely, this lane claims for its duration:

- the **pre-filesystem refusal** inside synchronous `apply_resource_op`;
- the **shared walk query** of Q#RD6 (`whichever lands first owns the
  query`), which #171 then adopts for `reconcile_rename`;
- `builtin/runtime/lsp.lua`'s `apply_workspace_edit` and the
  `workspace/applyEdit` server-request boundary.

It explicitly does **not** claim: full post-delete lifecycle
reconciliation, the dired async race between dispatch and
`remove_blocking`, the rename side of the walk, or `pmacs.fs.remove`
(§6). Revision 2's version of this note was written against a stale
reading of #171 and is superseded.
