# Framing — `apply_resource_op` delete destroys unsaved work

**Revision 5, plus §§9–10.** Status: **APPROVED and MERGED as #186
(framing only); the implementation is PR #190** on branch
`resource-op-delete-guard-impl`, worktree `../pmacs-rd-impl`. The
revision-5 body below is unchanged except for the two bookkeeping
edits §9.6 names and makes in place; **§§9–10 record the corrections
implementation review rounds 1–2 found**, including corrections to this
document. The "DO NOT implement, DO NOT merge" banner this line replaces
was true when revision 5 was written and is not now.

Revision 5's lane header — `resource-op-delete-guard`, worktree
`../pmacs-resource-op-delete`, based on `githubsucks/main` @
`7586905` — describes the framing branch, which merged. §8's
one-PR-for-both branch plan is superseded for the same reason and is
annotated there.

Revision 5 removes volatile sibling-branch counts from the normative
contract. A count is a reading, not a dependency; where history retains
one, it names the revision at which it was measured.

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

### Revision 4 → 5, after review round 4

Round 4 accepted the core and both surfaced decisions — the defensive
parse stub and fail-closed filesystem uncertainty — but found two
contract defects plus a ledger-ownership defect. All accepted.

**P1 — the narrowed reporting promise had one stale consumer.** Q#RD7
correctly says the unattended path always **attempts** a response while
the channel remains live, but §2.1 still said it **always answers**.
That normative consumer now uses the exact Q#RD7 promise. The revision
4 audit missed it because the literal search `always answers` did not
match Markdown's `**always** answers`. §1.15 therefore adds one more
procedural rule: search normalized prose or term stems, not only an
exact rendered phrase containing markup.

**P1 — Q#RD12 called a three-row verdict total when it was not.** It
omitted absent-without-ignore, defined `conflict` as a named modified
buffer while also assigning stat failures with no buffer to it, and did
not carry Q#RD2's `editing_in_progress` condition into the shared query.
The verdict is now `no-op` / `clear` / `refuse`, with a required message
on every refusal and an optional buffer name only for buffer-caused
refusals. The total mapping is explicit: absent-plus-ignore is `no-op`;
absent without ignore and an unanswerable stat are `refuse`; a modified
or mid-edit affected buffer is `refuse`; only a present target with a
clean, quiescent affected set is `clear`. Criteria 11c and 11d pin the
two filesystem refusal directions before any earlier batch op mutates.

**P1 — merge order was being used as ledger ownership.** Revision 4
rewrote #171's full lane entry from #186. #171 revision 8 landed 67
seconds earlier with different state, making #186's copy stale before
it was pushed; #186's own `3 / 0` count was also one short because the
revision commit itself had not yet been counted. The sibling block is
restored to `main`'s version, so #186 no longer changes it. #171 owns
its entry on its branch. This lane records only the stable split it
depends on and checks the pushed sibling framing for semantic changes,
without copying its volatile head/count/line state.

**Coordination decision.** #186 lands before #171 for a product reason,
not a merge convenience: it closes live data loss, and #171 explicitly
adopts its refusal and shared query. #171 integrates the result and
reconciles its own ledger entry. The generated-buffer lane is
independent; `journey-stage1a-directory-open` has no unmerged ledger
delta.

### Revision 3 → 4, after review round 3

Round 3 accepted the core — pre-filesystem refusal, four-phase ordering,
the #171 split, Q#RD10 — and raised four P1s and a P2. All accepted.

**P1 — the lane state and ledger were still false.** Re-measured:

```
$ git rev-list --left-right --count fd7ae37...7586905
13      2
```

#171 is **two commits behind**, not zero. Revision 3's "0 behind" was
measured against `ad41cf1` and reported in present tense after `main`
had moved. The ledger additionally still described #186 as revision 2 on
`ad41cf1` and retained the superseded `ab42a79` / 153-behind entry for
#171. **Both lane entries are now corrected in place** rather than
having a correction layered above stale ground truth, which is what
produced a self-contradicting ledger. §1.12 carries its count as pasted
command output.

**P1 — the withdrawn LSP claim survived in the normative decision.**
Q#RD3 still called partial application "`FailureHandlingKind.Abort`, the
strategy the spec itself assigns to any edit containing resource
changes", and §1.11 called `Abort` the default "by omission". Both are
withdrawn. Both sites now say only that **verified pmacs behaviour
resembles abort-style application**, with the justification resting on
§1.6's reproduction. This mattered more than an ordinary error because
§1.15's audit had certified the document clean while the claim was still
load-bearing three sections away.

**P1 — acceptance 15 had no reachable payload.**
`WorkspaceEditResponse::from_lsp_value` (`src/rename.rs:95`) returns
`Self`, its doc says "A `null` / shapeless result yields an empty
response", and the binding's only `?` is `lua_to_json` over a value that
arrived through `json_to_lua`. **No server payload can make the parse
fail**, so the criterion could not fail either. **Decision (Q#RD11):
keep the wrap, drive the test with an explicit throwing stub, and label
it defensive.** Q#RD7's promise is correspondingly narrowed to **"always
attempts a response while the response channel remains live"** —
`send_response` is itself under an ignored `pcall`
(`builtin/runtime/lsp.lua:1843`).

**P1 — absent-plus-ignore had no synchronous seam.** `pmacs.fs.stat`
(`builtin/runtime/fs.lua:133`) dispatches async; the only synchronous
filesystem binding is `canonicalize`
(`src/lua_bindings/mod.rs:6743`), which resolves symlinks and returns
`nil` for a dangling one — so it disagrees with the primitive's
`symlink_metadata` on precisely the input this query turns on. **New
Q#RD12** specifies a structured Rust-backed verdict (`no-op` / `clear` /
`conflict`) evaluated with the same `symlink_metadata` call, with an
error contract that fails toward refusal. New criteria **11a**
(present + ignore + modified ⇒ still refused) and **11b** (dangling
symlink counts as present) supply the missing opposite direction.

**P2 — acceptance and file-scope bookkeeping.** Criterion 14's
"fails in both directions" was **false**: with both duplicate buffers
clean, the setup cannot distinguish first-match validation from full
validation. **The claim is fixed, not the setup** — criterion 6 already
pins validation breadth, and duplicating it would add no bite; the two
are now labelled by which half of Q#RD10 each covers. §8's touch table
and §7's gate list are reconciled: `lsp_dispatch_seams_acceptance` was
named in one and omitted from the other, and the parse-stub work was
missing from both.

**Sweep — corrections applied at one site while a dependent site kept
the old claim.** Whole-document pass over every claim withdrawn or
revised in revisions 2 and 3, checked at each consuming site rather than
only where defined. **Count: 4.** Two were the P1-2 sites above (Q#RD3,
§1.11). Two were the stale #171 count, which had propagated into both
the revision-history entry and §1.12. Claims checked and found clean at
every consuming site: rev 1's buffer-first ordering; "whole-batch
atomicity"; `find_buffer_for_path` as the lookup; primitive-only
`ignore_if_not_exists`; the three withdrawn impossibility claims about
prompting; "no `*Messages*`/`*warnings*` buffer"; "only path 1 is
unattended"; mode (d) as unowned; and Q#RD9. Each of those appears only
in withdrawal text or in correctly-scoped ground truth.

### Revision 2 → 3, after review round 2

Round 2 confirmed everything central from round 1 as fixed and raised
four P1s. All four accepted; two sweeps run.

**P1-1 — the ownership boundary was stale.** Revision 2 described PR
#171 as "OPEN, STALE, 153 commits behind, under re-scout" and said it
claimed the **rename** side only. Re-checked directly: #171 is at
**revision 7, `fd7ae37`, merge-base `ad41cf1`** — not stale. *(That
sentence originally read "0 commits behind"; it was measured before
`main` moved and is corrected in the rev 3 → 4 section above.)* Revision 6 had assigned **both** rename and delete to Stage 2a,
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
  (`docs/archive/framings/dired-framing.md:854`).

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
strategy at all — **which is not the same as defaulting to one.** The
spec establishes no default for a client that advertises nothing, so
pmacs's actual behaviour is simply whatever its code does, which §1.6
verified **resembles** abort-style application without being licensed as
it. *(Revision 3 wrote "`Abort` semantics are the de facto behaviour by
omission"; that smuggled the withdrawn claim back as a default and is
itself withdrawn at revision 4.)* **Neither is fixed by this lane** —
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

**Cross-lane contract, rechecked at revision 5.** PR #171's pushed
revision 8 (`7ecea94`) retains revision 7's split unchanged. This
document deliberately does **not** copy its ahead/behind count, line
count, or full lane status: those are volatile state owned by #171's
branch, and revision 4 proved that a sibling copy can be false before
the copying commit is pushed. The historical correction still matters:
revision 2 described #171 as stale and rename-only, while its revision 6
had assigned rename *and* delete reconciliation to Stage 2a with the
opposite policy — `reconcile_delete` killing unmodified buffers and
keeping modified ones alive, so the file was deleted and the modified
buffer orphaned. Revision 7 withdrew that policy and established the
split below; revision 8 does not reopen it.

**The stable ownership split carried by both lanes:**

> #186 owns the urgent **pre-filesystem refusal** for synchronous
> `apply_resource_op`. #171 later owns **full post-delete lifecycle
> reconciliation**, including the **async race where a buffer becomes
> modified after dired dispatch**.

#171 has adopted this from its side: its Q#DR18 takes this
document's Q#RD1 refusal rather than re-deciding it, and it records the
reason the refusal cannot simply be extended to cover dired — **dired
never calls `apply_resource_op`**. It calls `pmacs.fs.remove`, which
dispatches a worker, so a synchronous refusal inside the primitive
cannot reach it at any strength. That asynchronous window is #171's, and
naming it here is what keeps this lane from appearing to close a defect
it does not close.

The older in-tree note at `docs/archive/framings/dired-framing.md:807-819` (Stage 1-era)
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
repository is listed here with its evidence. The standing rule: an
external claim carries a direct quote or it is marked not established.

**This audit itself failed at revision 3, and the failure mode is
recorded because the table is now something readers trust.** Revision 3
marked the `Abort` claim WITHDRAWN in row 4 while Q#RD3 — the normative
decision — still asserted it, and §1.11 still called it a default. The
audit checked each claim **where it was defined**, not at every site
that **consumed** it, so it certified a document that was internally
contradictory. A withdrawal recorded in an audit while the claim stays
load-bearing elsewhere is worse than no withdrawal, because the audit
converts an error into a false assurance.

**So the audit procedure is, from revision 5:** for each row, search the
whole document for the claim's terms and check every hit, not only the
defining section. Search normalized prose or multiple term stems as
well as exact phrases: revision 4's literal `always answers` search
missed §2.1's `**always** answers` because Markdown markup split the
phrase. Revision 4 found the two surviving `Abort` consumers; revision
5 found and fixed that reporting consumer.

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
`apply_workspace_edit`'s existing plan loop gains a delete-precondition
check and returns its existing `nil, message`. This is a **filter, not a
transaction** (§1.7): it catches the plan-time buffer conflict and
filesystem refusals cheaply, before anything is mutated, and it is
honest that a sequential batch can still refuse mid-flight. What makes
mid-flight refusal survivable is Q#RD7: each primitive call is wrapped,
every failure becomes `nil, message`, the origin buffer is restored
best-effort, and the unattended caller always **attempts** a response
while the response channel remains live.

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

`apply_workspace_edit`'s plan loop gains a delete-precondition check and
returns its existing `nil, message`. It is described in the code comment
and here as a **filter**:

- **What it guarantees:** a plan-time modified/mid-edit buffer, a known
  missing target without `ignore_if_not_exists`, or an unanswerable stat
  refuses before anything in the batch is mutated, with one clear
  message.
- **What it does not guarantee, stated plainly:** `documentChanges` are
  sequential (§1.7). An earlier text edit can dirty a clean buffer, and
  an earlier rename can move a modified buffer *into* a later delete's
  subtree, after the snapshot. Then the preflight passes and the
  primitive refuses mid-batch, leaving earlier operations applied.
  That outcome **resembles abort-style application**, and it is what
  pmacs already does today on an I/O error (§1.6, verified). It is
  **not** licensed by the specification: the spec assigns no strategy
  to a client that advertises none (§1.7), so the justification is
  observed pmacs behaviour plus the judgement that a visible partial
  refactor beats unrecoverable unsaved work — nothing more.
- Revision 1 called this "whole-batch atomicity" and said "nothing in
  the batch is mutated". **That was false and is withdrawn.**

The check needs a synchronous filesystem-and-buffer query that Lua lacks
(§1.4, Q#RD12). It must be **one** query shared with the primitive's
stat/validation phases, so the two cannot drift apart.

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

**Boundary with dired — restated at rev 5.** The settled split (§1.12,
also carried by #171) is:

> #186 owns the urgent **pre-filesystem refusal** for synchronous
> `apply_resource_op`. #171 later owns **full post-delete lifecycle
> reconciliation**, including the **async race where a buffer becomes
> modified after dired dispatch**.

**The stale justification is withdrawn.** Revision 2 supported taking
the delete side now by citing the ledger's "OPEN, STALE, 153 commits
behind, under re-scout" assessment of #171. That re-scout has finished;
#171 has completed the re-scout and retains the settled split through
its pushed revision 8. **The conclusion is unchanged and rests on
urgency alone** — this is a live data-loss bug
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
first owns the query and the other adopts it", and #171's current
framing records that this rule's four clauses are character-for-character what
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
  parse-plus-apply** rather than narrowing the wrap to applier execution
  failures. Reason: the wrap moves up exactly one line and costs
  nothing, so the boundary is uniform regardless of which call fails.

  **Revision 4 correction to the strength of the claim.** Revision 3
  said this made "always answers" true *without qualification*. It does
  not, for two independent reasons, and the honest wording is **"always
  attempts a response while the response channel remains live"**:
  - `send_response` is itself called under an ignored `pcall`
    (`builtin/runtime/lsp.lua:1843`), so its failure is unobservable to
    the applier. A dead or wedged transport cannot be answered by any
    amount of wrapping upstream.
  - The parse call is, on the evidence, **not reachably fallible** —
    see acceptance 15 and Q#RD11.
- **Every failure becomes a value.** Refusal, I/O error, and parse
  failure all converge on the existing `nil, message` shape, which all
  three callers already handle. No exception escapes the applier.
- **The unattended caller always *attempts* a response**: `{ applied =
  false, failureReason = ... }` is constructed and sent in every failure
  case the applier can observe. Whether it lands is the transport's
  business, and the applier cannot tell (see above).
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

### Q#RD11 — The parse wrap is a defensive boundary, tested with a stub — **NEW at rev 4**

Revision 3 justified wrapping `_parse_workspace_edit` by asserting it
was reachably fallible. **On the evidence it is not**, and acceptance 15
as written could not fail:

- `WorkspaceEditResponse::from_lsp_value` (`src/rename.rs:95`) returns
  `Self`, not a `Result`. Its own doc comment says "A `null` /
  shapeless result yields an empty response."
- The binding's only `?` is `lua_to_json(edit)?`
  (`src/lua_bindings/mod.rs:10161`), and its input arrived as JSON
  through `json_to_lua`. Every value that round-trip produces is
  accepted going back.

So no fake server can send a payload that makes the parse fail.
**Decision — the first of the two options offered: keep the wrap, and
label it a defensive boundary test driven by an explicit throwing test
stub for `pmacs.lsp._parse_workspace_edit`.** Reasons: the wrap costs
one line and makes the boundary uniform, so a future parse that *does*
become fallible is covered by construction rather than by remembering;
and the promise in Q#RD7 is narrowed to match reality rather than
propped up by an unfalsifiable criterion.

**What this explicitly is not:** a claim that a server can trigger it.
Acceptance 15 is labelled defensive, and it substitutes the stub rather
than dressing up a reachable payload — a criterion that cannot fail is
not a pin, and pretending otherwise is the defect this decision exists
to avoid.

### Q#RD12 — The preflight needs a total Rust-backed verdict, not a registry walk alone — **REWRITTEN at rev 5**

Q#RD4 requires the Lua preflight to distinguish **absent + ignore** (a
no-op the preflight must let through) from **present + ignore** (a real
delete the preflight must judge). Q#RD6 specifies only a registry walk,
which answers a question about *buffers*, not about *the filesystem*.
Nothing in Lua closes that gap today:

- `pmacs.fs.stat` (`builtin/runtime/fs.lua:133`) dispatches through
  `async_mod._dispatch_fs_stat` and returns a handle — asynchronous, and
  the applier is synchronous.
- The only synchronous filesystem binding is `canonicalize`
  (`src/lua_bindings/mod.rs:6743`), which is realpath-like and **not**
  equivalent to the primitive's `symlink_metadata`: it resolves symlinks
  and returns `nil` for a dangling one, so it reports "absent" for a
  broken symlink that `symlink_metadata` reports as **present**. That is
  exactly the case this query turns on, so using it would produce a
  preflight that disagrees with the primitive on the one input that
  matters.

**The seam: one synchronous internal Rust binding,
`pmacs.buffer._delete_verdict(spec)`, returning a structured verdict.**
It accepts the same `path`, `recursive`, and `ignore_if_not_exists`
fields as the delete primitive and delegates to the same Rust helper,
including the same `symlink_metadata` call and affected-set walk, so the
two layers cannot disagree by construction. The Lua-visible shape is
`{ kind = "...", message = ..., buffer_name = ... }`; `message` is
required and non-empty for `refuse`, and `buffer_name` is present only
when a buffer caused the refusal:

| Verdict | Meaning |
|---|---|
| `no-op` | path absent **and** `ignore_if_not_exists` set — the op will do nothing; the preflight must not reject it |
| `clear` | path present, and every affected buffer is clean and not mid-edit — the delete may proceed |
| `refuse` | delete must not proceed: missing without ignore, stat uncertainty, or an affected buffer modified/mid-edit; `message` states which, and buffer-caused refusals name it |

**Total mapping and error contract.**

1. `symlink_metadata == NotFound` plus `ignore_if_not_exists` yields
   `no-op`.
2. `NotFound` without ignore yields `refuse` with the ordinary delete
   I/O message. Catching this deterministic failure in the plan makes
   that case more atomic than today without claiming the batch is a
   transaction; dynamic failures remain possible.
3. Any other stat error (for example `EACCES` or `NotADirectory`) yields
   `refuse` carrying that I/O reason. The preflight never reports safe
   on the strength of a question it could not answer.
4. A present path whose affected set contains a modified or
   `editing_in_progress` buffer yields `refuse` naming that buffer.
5. Only a present path with a clean, quiescent affected set yields
   `clear`.

The binding raises only on argument-type violations, matching the rest
of the `pmacs.buffer` surface. Ordinary filesystem conditions and buffer
refusals are values.

The callers consume the same Rust enum in different forms. The
primitive returns `Ok(())` for `no-op`, turns `refuse` into its ordinary
Lua error carrying the verdict message, and reaches filesystem mutation
only for `clear`. The Lua plan lets `no-op` and `clear` through and
returns its existing `nil, message` for `refuse`.

This helper **is** the single shared query of Q#RD6: the walk is its
buffer half, `symlink_metadata` its filesystem half, the binding
serializes its result, and the primitive consumes it directly. Drift is
impossible rather than merely discouraged.

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
   specified. **This is the criterion that pins validation breadth**;
   criterion 14 pins the reconciliation half and cannot see breadth
   (Q#RD10).

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

11a. **Present-plus-ignore with a modified buffer is still REFUSED in
    the preflight** (Q#RD4, Q#RD12) — the opposite direction of 11. The
    target **exists** on disk, `ignore_if_not_exists = true`, and a
    modified buffer is bound to it. Assert the batch is refused, the
    file still exists, and the buffer keeps its text.
    *Bite:* fails against a preflight that treats
    `ignore_if_not_exists` as an unconditional bypass rather than
    consulting the filesystem — i.e. against any implementation that
    reads the flag without the `symlink_metadata` verdict Q#RD12
    specifies. **Revision 3 shipped only direction 11**, and
    one-direction coverage on a two-direction rule is exactly how that
    gap survived a round; the pair is now explicit, as with 7/8.

11b. **A dangling symlink counts as present** (Q#RD12). Target is a
    symlink whose destination does not exist, `ignore_if_not_exists =
    true`, modified buffer bound to the link path. Assert refusal.
    *Bite:* fails against a preflight built on `canonicalize`
    (`src/lua_bindings/mod.rs:6743`), which returns `nil` for a broken
    symlink and would therefore mis-classify this as absent — the one
    input on which realpath and `symlink_metadata` disagree, and the
    reason Q#RD12 specifies the latter.

11c. **Absent without ignore refuses in the plan, before earlier ops**
    (Q#RD3, Q#RD12). A batch contains a text edit followed by a delete
    of a missing target with `ignore_if_not_exists = false`. Assert
    `applied = false`, a non-empty NotFound-style `failureReason`, and
    that the earlier text edit was not applied.
    *Bite:* fails if the verdict maps this state to `clear` and leaves
    the primitive to discover it mid-batch; that implementation would
    partially apply the text edit before returning the known error.

11d. **An unanswerable stat fails closed in the plan** (Q#RD12). Use a
    regular file as a would-be parent and target its child, producing
    `NotADirectory` on the supported CI platforms. Assert
    `applied = false`, the earlier batch op did not apply, and the
    `failureReason` carries the filesystem cause.
    *Bite:* fails if a non-NotFound stat error is collapsed to `clear`
    or if the binding raises past the value-returning boundary.

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

14. **Clean duplicates: one match reconciled** (Q#RD10). Two clean
    buffers bound to one path; delete succeeds. Assert exactly one is
    removed and one remains.
    *Bite:* fails against an implementation that removes **all** matches
    — i.e. it pins the reconciliation half of Q#RD10, and only that.
    *Correction at rev 4:* revision 3 claimed this criterion "fails in
    both directions", including against first-match-only *validation*.
    **That was false.** With both buffers clean there is no verdict
    difference between consulting one match and consulting all, so the
    setup cannot see validation breadth. **Criterion 6 is the one that
    detects incomplete validation** (clean first, modified second), and
    the two are now labelled by which half of Q#RD10 each pins. The
    claim is fixed rather than the setup, because criterion 6 already
    covers the other half and duplicating it here would add no bite.

15. **Defensive: a parse failure still attempts a response** (Q#RD7,
    Q#RD11). Substitute an explicit **throwing test stub** for
    `pmacs.lsp._parse_workspace_edit`, then assert the server still
    receives `applied = false` with a `failureReason`.
    *Bite:* fails against a wrap that covers `apply_workspace_edit`
    only, leaving the parse one line outside.
    **Labelled defensive, and here is why the label is load-bearing:**
    revision 3 specified this as a *payload* test, which **could not
    fail** — `from_lsp_value` (`src/rename.rs:95`) returns `Self` and
    its doc says a shapeless result yields an empty response, and the
    binding's only `?` is `lua_to_json` over a value that arrived
    through `json_to_lua`. No server payload reaches the failure. The
    stub is therefore substituted deliberately, and the criterion claims
    only what a stub can establish: that the boundary reports rather
    than that a server can provoke it.

16. **`m4_15_workspace_edit_resource_ops_apply_in_order` stays green
    unmodified**, pinning no-regression from outside. Its `c.rs` is
    never opened, so it exercises exactly the unguarded case that must
    keep working (Q#RD6).

17. **Every new test is checked with `scripts/bite`** and none reports
    VACUOUS.

**Criteria 18, 19a–19c and 20 are added by §9; criteria 21 and
22a–22b by §10**, after implementation review rounds 1 and 2. They are
listed there, with their pre-images, rather than interleaved here, so
this section stays readable as the record of what revision 5 asked for.


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

Touched suite: **`m4_acceptance`** — the resource-op home (§1.14) and
the home of every criterion, 1–16 including 11a–11d, plus §9's 18,
19a–19c and 20, and §10's 21 and 22a–22b.

*Amended at implementation (§9.6).* Revision 5 also named
**`lsp_dispatch_seams_acceptance`**, for criterion 15's throwing parse
stub. §8 permits hosting that stub in `m4_acceptance` instead
**provided the gate list moves with it**, and it does: criterion 15
drives the same server pump as 11–13, so splitting it across two
suites would have duplicated the whole fixture. The suite is therefore
struck from this list **and** from §8's touch table, in the same edit.
The lists are maintained together, which is what revision 3 got wrong.

`dired_acceptance` and `autosave_acceptance` are watch items, not
touched files — the former for Q#RD6's shared lookup, the latter because
`on_removed` ordering (criterion 4) is where autosave's
`discard_buffer` hangs.

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

**Superseded in fact, not by decision.** #186 merged as framing-only,
so the implementation necessarily got its own branch
(`resource-op-delete-guard-impl`, worktree `../pmacs-rd-impl`) and its
own PR, #190. No decision in this document changes; only the branch
plan, which described a PR that no longer existed by the time
implementation started.

**Files the implementation will touch** — reconciled at rev 5 against
the gate list below and §5, which revision 3 left disagreeing:

| File | Why |
|---|---|
| `src/lua_bindings/mod.rs` | the delete arm's four phases (Q#RD2); the shared query binding and its structured verdict (Q#RD6, Q#RD12); the narrow `*errors*` append surface (Q#RD7) |
| `builtin/runtime/lsp.lua` | the preflight conflict check (Q#RD3); the parse-plus-apply wrap, origin restore, and boundary logging (Q#RD7) |
| `tests/m4_acceptance.rs` | criteria 1–16 including 11a–11d, plus §9's 18, 19a–19c and 20 and §10's 21 and 22a–22b — **including criterion 15's throwing parse stub**, per the permitted simplification below (§9.6) |
| `src/bin/pmacs_fake_lsp.rs` | **one parameterized mode, `applyeditplan`**, whose `WorkspaceEdit` is read from a test-written file, plus a sink for the client's response — see §9.6 for why one mode replaced the eight named below |

~~`tests/lsp_dispatch_seams_acceptance.rs`~~ — struck at implementation
(§9.6), together with its entry in §7's gate list.

The eight fake modes revision 5 named — blocked delete;
edit-then-delete; rename-into-delete; absent-plus-ignore;
present-plus-ignore (11a); dangling-symlink (11b);
absent-without-ignore (11c); unanswerable-stat (11d) — are the eight
*fixtures*, and they all still exist. They are payloads now, not modes
(§9.6).

It will **not** touch `src/daemon.rs`, `pmacs-protocol/`,
`builtin/runtime/dired.lua`, `docs/agent-handoff.md` or `COHERENCE.md`.
No protocol change.

If criterion 15's stub proves cleaner to host in `m4_acceptance`
alongside the rest, that is a permitted simplification — but then
`lsp_dispatch_seams_acceptance` drops out of the gate list too, and the
two lists move together. Revision 3's defect was that they did not.

**Ownership note — rechecked at rev 5 against #171's pushed revision
8.** The settled split is quoted in §1.12 and Q#RD5 and is carried by
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


## 9. Corrections found during implementation — review round 1

This section is written **after** revision 5 was approved and
implemented, and it changes no design decision. It records four
corrections review round 1 found in the implementation, the acceptance
criteria they added, and two bookkeeping edits made in place above.
It lives here because two of the four are corrections *to this
document*, and a correction that lives only in a test comment is
invisible to the next reader of the framing.

### 9.1 Criterion 3's stated bite was wrong — fixed by fixing the setup

Criterion 3 says it "fails against revision 1's buffer-first
ordering". Against the first shipped setup it did **not**, and the
claim was found by checking rather than trusting it: that setup bound
the clean buffer to a file *beneath* the deleted directory, so no
buffer was bound to the deleted path, `find_by_path` matched nothing,
and reordering reconciliation ahead of the filesystem mutation left
the test passing.

The first fix considered was to amend the criterion's wording.
**That is not what was done.** §9.2 then narrowed the affected set to
recursive deletes only, which would have left that setup with *no*
bite at all — the pre-image it did catch (validation that removes
rather than inspects) stops touching a descendant buffer on a
non-recursive delete. So the **setup** is what changed: the buffer is
now bound to the **exact** deleted path. A file is opened, the path is
then replaced on disk by a non-empty directory, and a non-recursive
`remove_dir` fails deterministically with `ENOTEMPTY` — no permission
trickery and nothing that behaves differently under a root CI.

Against that setup the criterion fails against **both** pre-images,
which is what revision 5 claimed all along. **The framing's wording
needed no amendment; the test did.** Both directions verified by
mutation.

### 9.2 The affected set is scoped by `recursive` (Q#RD12, Q#RD6)

The shipped `delete_verdict` ignored `recursive` and scanned
descendants for **every** directory target, justified in a doc comment
on the grounds that a non-recursive delete of a non-empty directory
fails at the filesystem anyway, so widening inspection cost nothing.

**That reasoning is wrong**, and the counterexample is an orphan: a
modified buffer at `tree/gone.rs` whose file is already deleted blocks
a non-recursive delete of the now-**empty** `tree/` — an op that would
have succeeded and that removes none of that buffer's contents.
Reproduced in review.

`recursive` is therefore a parameter of the shared query, and
descendant matching is reserved for recursive deletes. This
**narrows** the query Q#RD6 hands to #171, which adopts the narrowed
version. Q#RD5's "inspect widely, mutate narrowly" is unchanged in
substance: "widely" means *the set the op can actually destroy*, which
for a non-recursive delete is the target entry alone.

A symlink to a directory is correctly excluded by the same rule:
`symlink_metadata` reports it as not-a-directory, and the primitive
`remove_file`s the link without walking through it.

### 9.3 The preflight defers for targets the batch itself changes (Q#RD3)

The shipped preflight judged **every** delete against the filesystem's
**initial** state, at plan-construction time. A valid `create X →
delete X` batch was therefore rejected because X was absent when the
plan was built, with a fabricated `NotFound` about a path the batch
was about to create; likewise `rename A → B → delete B`. This was a
regression introduced by the implementation, not a pre-existing
defect.

**Decision — defer, do not simulate.** A delete whose target is
related by path containment to a path an **earlier** op in the same
plan creates, renames onto, renames away from, or removes is not
judged at plan time; the primitive judges it when it runs. Comparison
is component-aware, like the Rust side's `Path::starts_with`.

Why this is the right half of the choice Q#RD3 already made:

- Q#RD3 calls this check a **filter, not a transaction**. Declining to
  judge an op the snapshot cannot see is inside that contract;
  refusing a legal batch is not.
- Simulating instead would mean modelling filesystem presence **and**
  the buffer registry's path bindings across create / rename / edit —
  the transaction Q#RD3 declines to build — and a simulation that got
  it wrong would emit false `clear` verdicts, which is the dangerous
  direction. Deferral only forgoes the early, cheap report.
- The primitive's four-phase guard is untouched and is the thing that
  actually stands between a server and unsaved work. Criterion 19c
  pins that deferring is not skipping.

**`edit` ops are deliberately not in the deferral set.** An edit
changes no path's existence; it can only dirty a buffer, i.e. only
turn a plan-time `clear` into a primitive-time refusal. That is the
under-refusal Q#RD3 documents and accepts, and adding edits would
merely delay a refusal that is already certain. Criterion 11c depends
on this: its delete target is touched by no earlier op, so the
buffer-and-filesystem half of the check still fires before anything is
mutated.

### 9.4 Failure reporting must not deny partial application (Q#RD3, Q#RD7)

`apply_workspace_edit` discarded, on failure, whether earlier ops had
succeeded, and returned a bare `nil, message`. The rename caller then
said "rename aborted" under a comment reading "nothing was mutated" —
false in exactly the case Q#RD3 predicts, where an earlier text edit
applies and dirties the buffer a later delete refuses.

The applier now returns `nil, message, applied_op_count`, and **one
renderer** serves both the user-facing status line and the server's
`failureReason`, so the two cannot disagree about what happened. All
three callers are updated: the server-request boundary, the rename
caller, and the code-action caller.

### 9.5 Acceptance added by this round

18. **A non-recursive delete is not blocked by a buffer beneath its
    target** (§9.2). Modified buffer at `tree/gone.rs` whose file is
    already gone; non-recursive delete of the now-empty `tree/`
    succeeds and the buffer is untouched.
    *Bite:* fails against a `delete_verdict` that ignores `recursive`.

19a. **`create X → delete X` is not refused at plan time** (§9.3).
    Assert `applied = true`, and that a later `create` in the same
    batch produced its file, so the success is not vacuous.
    *Bite:* fails against the initial-state preflight, which reports
    `NotFound` for X and rejects the batch before anything runs.

19b. **`rename A → B → delete B` is not refused at plan time** (§9.3).
    *Bite:* as 19a; the source file surviving is what carries it,
    because a plan-time rejection leaves the rename unapplied.

19c. **Deferring the check is not skipping it** (§9.3).
    `create X → edit X → delete X` gets past the plan and then refuses
    at the primitive, naming the unsaved changes the edit created —
    and reports that two operations remain applied.
    *Bite:* fails against the initial-state preflight (which reports
    `NotFound` instead) **and** against dropping the primitive's guard
    for deferred targets (which would report `applied = true`).

20. **The user-facing message reports partial application** (§9.4).
    Driven through `M-x lsp.rename`, because a status line is where a
    user reads it and the applier's return value alone would not pin
    the caller.
    *Bite:* fails against any caller that renders the failure without
    consulting the applied-op count — i.e. against the shipped
    "rename aborted".

All of 11, 11a–11d, 12, 13 and 15 also land in this round; they were
specified by revision 5 and were the named gap in the first
implementation commit.

### 9.6 Bookkeeping edits made in place

- **§7's gate list and §8's touch table both lose
  `lsp_dispatch_seams_acceptance`**, in the same edit, under §8's
  permitted simplification. Criterion 15 drives the same server pump
  as 11–13, so hosting it anywhere else would duplicate the fixture.
- **§8's fake-mode row is one parameterized mode, not eight.**
  `PMACS_FAKE_LSP_MODE=applyeditplan` reads its whole `WorkspaceEdit`
  from the file named by `PMACS_FAKE_LSP_EDIT_PLAN` and publishes the
  client's response to `PMACS_FAKE_LSP_APPLYEDIT_SINK`. The eight
  fixtures revision 5 named all exist; they are payloads the test
  writes rather than modes the fake hardcodes, which keeps each
  payload next to the assertions that depend on it instead of mirrored
  across two files. The mode is **fail-closed**: an unreadable or
  unparsable plan sends no `applyEdit` and reports itself through the
  sink, so a broken fixture cannot read as a pass. The sink is written
  to a `.part` and renamed, so a polling reader never sees a partial
  record — the wait predicate cannot be weaker than the assertion.

### 9.7 Sweep — every place the guard decides something is "affected"

§9.2 and §9.3 are one defect class: **a guard whose scope was reasoned
about rather than enumerated**, over-refusing on inputs the reasoning
never considered. The rest of the guard was swept for that shape.

| Site | Decision | Verdict |
|---|---|---|
| `delete_verdict` filesystem classification | is the target a directory whose descendants are at risk? | **Fixed** (§9.2). Now `is_dir() && recursive`. A symlink-to-directory is excluded, matching the primitive's `remove_file`. |
| `delete_verdict` buffer matching | which buffers are in the affected set? | **Fixed** (§9.2), exact path always plus descendants only when recursive. |
| Lua plan-time preflight | which deletes can be judged from the initial snapshot? | **Fixed** (§9.3). |
| `delete_verdict` path normalization | the stat uses the **raw** path; the buffer comparison uses the **normalized** one | **Latent inconsistency, fails safe, not fixed here.** A `~`-prefixed argument stats as a literal `~` directory (absent) while matching buffers bound under `$HOME`. Every branch of that disagreement is safe: without `ignore_if_not_exists` it refuses, and with it the primitive returns early having touched nothing. It matches the primitive's own `remove_file`, which also takes the raw path, so the two layers still agree with each other. |
| Phase 4 reconciliation | which buffer is removed after a successful delete? | **Unchanged by design** (Q#RD10). `BufferRegistry::find_by_path` compares paths **raw**, with no normalization, so a buffer stored under a differently-spelled path is not reconciled. This is `main`'s behaviour, Q#RD10 pins "exactly today's", and correcting it would *widen* reconciliation — the one thing Q#RD5 and Q#RD10 forbid. Named so it is not mistaken for an oversight; it belongs to #171. |
| `_delete_verdict` argument handling | `recursive` / `ignore_if_not_exists` defaults | **Consistent.** Both default to `false` on the binding and on the primitive, so an omitted `options` object means the same thing at both layers. |
| Preflight `batch_changes` membership | which prior ops can change a delete's target? | **Enumerated, not reasoned:** create (1 path), rename (both paths), delete (1 path); edit excluded with the argument in §9.3. |

**Nothing else in this lane decides an affected set.** The reporting
path names a buffer only inside a refusal it already computed, and
`restore_origin` is best-effort by construction.


## 10. Corrections found during implementation — review round 2

This section records the two remaining findings on the round-1 repair.
Neither changes the feature boundary, Q#RD1's refusal, Q#RD2's phase
order, or Q#RD3's choice of a filter rather than a transaction. Both
make the round-1 correction true for inputs its first acceptance set did
not enumerate.

### 10.1 Batch dependency comparison uses the registry's lexical path form

Section 9.3 correctly required component-aware comparison, but its first
implementation compared **raw decoded URI strings** after stripping
only trailing slashes. That is component-aware without being
path-equivalence-aware: `file:///tree/./x` and
`file:///tree/x` reach the same filesystem entry while comparing
unequal. A legal ordered `create /tree/./x → delete /tree/x` was
therefore refused at plan time with the same fabricated `NotFound`
§9.3 had just fixed for identical spellings. Reproduced through the
real server pump.

**Decision:** dependency comparison routes both operands through
`pmacs.path.canonicalize`, which is
`editor_core::normalize_buffer_path` itself — absolute, lexically clean,
redundant-separator and `.` / `..` folding, with no filesystem access
and no symlink resolution. This reuses the buffer registry's canonical
form rather than growing a Lua mirror (the COHERENCE §14 / dired Q#DR2
rule).

Only the **comparison** is normalized. The plan item retains the decoded
path for execution, so this correction does not silently widen Q#RD10's
raw phase-4 reconciliation or resolve symlinks. Section 9.7's two raw
execution-path findings remain exactly as scoped there.

### 10.2 Zero completed items does not prove zero mutation

Section 9.4's `applied_op_count` reports plan items that completed
before a failure. Its first renderer treated `0` as proof that nothing
was mutated. That inference is false **inside one failing item**:

- a `TextDocumentEdit` contains multiple buffer edits applied
  sequentially, so an intercept can accept the first and reject the
  second after the first edit changed the buffer;
- a resource primitive can have intermediate filesystem effects before
  its terminal error — today the rename arm creates destination parents
  before attempting the rename, so a missing source can leave a new
  directory behind.

Both cases were reproduced through the real server pump with the failing
item first in the plan. The response said `aborted, nothing was
mutated` while the buffer or filesystem visibly disagreed.

**Decision:** the failure result now carries
`execution_started` independently of `applied_op_count`. Only parse and
plan-time failures render `nothing was mutated`. Once execution starts,
the renderer is deliberately conservative:

- with completed items, it says those earlier changes remain applied
  and the failing item may also have changed state;
- with zero completed items, it says the first operation may have
  changed state before failing.

This does not claim that every failing primitive mutates. It refuses to
make a stronger recovery claim than the applier can prove, and one
renderer still serves the server response, rename status, and code
action status.

### 10.3 Acceptance added by this round

21. **Lexically equivalent dependency paths are related** (§10.1).
    The server sends `create /dir/./x → delete /dir/x → create witness`;
    the batch succeeds, `x` is gone, and the witness exists.
    *Bite:* fails against raw-string `paths_related`, which preflights
    `/dir/x` against the initial filesystem and refuses with `NotFound`.

22a. **A failing multi-edit item is reported conservatively** (§10.2).
     One `TextDocumentEdit` carries two replacements; a deterministic
     intercept accepts the higher-offset edit and rejects the second.
     The first edit remains in the buffer, `applied` is false, and the
     reason must not say nothing was mutated.
     *Bite:* fails when `applied_op_count == 0` alone selects the
     no-mutation message.

22b. **A failing resource item is reported conservatively** (§10.2).
     A rename with an absent source and a destination under a new parent
     fails after creating that parent. The directory remains and the
     reason must acknowledge possible state change.
     *Bite:* fails against a text-edit-only repair, or any renderer that
     still equates zero completed resource items with zero mutation.

### 10.4 Coherence and scope

This round still serves **COHERENCE §1.2's silence asymmetry** and
§23's requirement that background computation not become opaque: it
makes the already-added server/user failure trace truthful. It touches
no new golden-journey step, adds no interaction island, adds no setting,
and creates no background work. No config-registry, ownership, or
protocol change follows.
