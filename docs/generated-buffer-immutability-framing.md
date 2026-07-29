# Generated-buffer immutability

**PROPOSED — needs explicit user approval before implementation. DO NOT
implement, DO NOT merge.**

**Revision 4 — scouted against canonical `githubsucks/main` @ `7586905`,
2026-07-28.** Every count in this document was re-measured at this
revision; the command output is in the revision-4 block below.

## Revision 4

**Answers review round 3 on `8e032d7` — three P1, two P2 — and two sweeps
the review asked for by class. It withdraws three of revision 3's own
decisions. All five findings are confirmed; none is re-litigated.**

| finding | the decision |
|---|---|
| **P1-1** — the revision predicate is wrong in two directions | Confirmed, **and it is wrong in three.** §3.4 is rewritten around an **explicit `GeneratedOutcome`** reported by the apply instead of a predicate inferred from `revision`: five variants, each with its own cleanup, tabulated. Direction A (successful no-op) is fixed by restating the invariant as a property of the **buffer** — *a generated-locked buffer carries no history* — so `NoOp` clears, which needs no reference to `revision` and is therefore mode-independent. Direction B (CRDT mid-transaction) gets its own `Diverged` variant that **clears nothing and surfaces**, because clearing would destroy the last local record of the pre-edit rope. **Third direction, found while fixing the other two and not reported by the review:** revision 3's unconditional relock **locked a fresh buffer that was never successfully written** — a mid-codepoint generated insert on a writable `*scratch*` returned `Err` *and* left it read-only. `Rejected` now restores the entry value. |
| **P1-2** — criteria 15-16 never enter the transaction | Confirmed, and the contradiction was internal: revision 3 argued in §3.4 that pre-validation makes an invalid range cost nothing, then used an invalid range to test the post-apply cleanup. Both criteria are rewritten around a **valid** write that fails at the `on_edit` broadcast, staged with a Rust-side `FailingView` (`pmacs::view::View` is `pub`, `src/view.rs:221`; `Buffer::attach_view` is `pub`). Criterion 16 splits into relock-on-failure **and** no-lock-on-refusal, which §3.4 now answers differently. New 16b and 16c cover the two P1-1 directions. **Rule adopted: every criterion names the exit it drives the implementation to, and that exit must be inside the mechanism under test.** |
| **P1-3** — provenance invalidates the lift-and-restore seam | Confirmed. **Q#GB15's `generated_lock` is withdrawn.** The defect is not the rule's details but that a *derived* fact must be maintained by every mutation of what it derives from, and `set_read_only` is `pub`. Replaced by **`identity_protected`** — a property of what the buffer *is*, set once by `TerminalSession::open`, **never written by `set_read_only`**. `tests/terminal_copy_mode_acceptance.rs:578-584`'s lift/upgrade/restore cycle is transparent to it, so Q#GB12 and Stage 2 criterion 4 need no change. The `crdt`-gated terminal suite is now named explicitly in §10. |
| **P2-4** — search registration adds a hard dependency on `compile.lua` | Confirmed. Revision 3 replaced an existing *guarded, optional* dependency (`default.lua:991-994`'s triple check) with an unguarded call. Q#GB18 is rewritten to **symmetric guarded optionality**: each module answers for its own buffers and consults the other through the guard shape already in the tree. That also removes the teardown obligation revision 3's registry introduced. New Stage 2 criterion 21 pins the minimal harness. |
| **P2-5** — fold criterion 13 contradicts its classification | Confirmed. If dired folds successfully on `main`, asserting `false` **fails** on `main`, so `[fix-shape]` was the wrong label. Split into **13a `[main]`** (the behaviour change, falsifiable by revert) and **13b `[mutation]`** (the status string, whose pre-image cannot be `main` because on `main` the call succeeds and sets no status). |

**Sweep D — rules spanning the v0.1/CRDT boundary or the
fresh-lock/existing-lock boundary (obligation 1).** The review is right
that P1-1 and P1-3 are one class: *a rule derived from one mechanism and
applied to a second that does not share its ordering.* **13 rules
examined, 3 broken, 1 verified that looked at risk.**

| # | rule | boundary | verdict |
|---|---|---|---|
| 1 | cleanup keyed on `revision` | v0.1 / CRDT | **BROKEN** — P1-1 |
| 2 | `clear_history` clears whichever history exists | v0.1 / CRDT | clean — reads both, `src/buffer.rs:558-566` |
| 3 | Q#GB4's O(1) clear argument | v0.1 / CRDT | clean — in CRDT mode `self.undo` is bypassed entirely (`:1268-1272`), so the "at most one entry" premise is vacuously true rather than wrong |
| 4 | Stage 2 criterion 4 asserts `Err(NothingToUndo)` | v0.1 / CRDT | **verified, not assumed** — `undo_crdt_mode` returns `BufferError::NothingToUndo` when the manager reports nothing undone (`src/buffer.rs:1374-1376`), so the same assertion is correct in both modes |
| 5 | §3.3 "CRDT behaviour: nothing new" | v0.1 / CRDT | corrected — true except for `Diverged`; §3.4 now says so |
| 6 | Q#GB10's `mark_clean` | neither | clean — `is_modified` is mode-independent |
| 7 | Q#GB6's coordinate clamp | neither | clean |
| 8 | criterion 10's coverage rule | v0.1 / CRDT | tightened — it now names which criteria are irreducibly single-configuration |
| 9 | refuse "someone else's lock" by provenance | fresh / existing lock | **BROKEN** — P1-3 |
| 10 | relock unconditionally | fresh / existing lock | **BROKEN** — found here, not reported; a refusal locked a buffer nobody wrote |
| 11 | Q#GB5's empty `set_generated_contents` in `ensure_slot` | fresh / existing lock | clean **only because** Q#GB13 makes the buffer fresh by construction — and it is also the concrete caller that made P1-1 direction A load-bearing rather than theoretical; the dependency now runs both ways and is stated in Q#GB5 |
| 12 | `unlock_generated`'s refusal | fresh / existing lock | follows 9; fixed with it |
| 13 | Q#GB2's `generated` implies bypass | neither | clean |

**Sweep E — every criterion added in revisions 3 and 4, audited for
whether the state it asserts is reachable by the operation it performs
(obligation 2).** This is the vacuous-criterion class, which this arc has
now shipped **twice** — revision 2's Stage 2 criteria 3-5, then revision
3's 15-16. **19 criteria audited, 3 defects, all fixed here.**

| criterion | reachable by its own operation? |
|---|---|
| S1-6 (round-trip, 3 halves) | yes — and (c) is what makes (b) non-vacuous |
| S1-11 (disambiguated panel keys) | yes — foreign buffer first, panel opens as `<2>` |
| S1-12 (`q`-target not inverted) | yes — needs the two-panel sequence, which is why it is separate from 11 |
| S1-13 (fold) | **MISCLASSIFIED** — P2-5; split into 13a/13b |
| S1-14 (structural) | n/a |
| S2-6 (four surfaces) | yes |
| S2-13 (unlock, 3 halves) | yes — the terminal half needs a real terminal, which the copy-mode suite already opens |
| S2-14 (refuse a foreign lock) | yes — fails on `main` |
| S2-15 (flag cleared on error) | **VACUOUS** — P1-2; rewritten around `FailingView` |
| S2-16 (relock on error) | **VACUOUS** — P1-2; rewritten and split |
| S2-17 (invalid range keeps history) | yes — and it is the one criterion for which exit 4 *is* the mechanism under test, so it stays as written |
| S2-18 (re-entrant refused) | yes — phase 2 of `run_managed_edit` releases the borrow (`src/lua_bindings/mod.rs:1477-1487`), so the inner call reaches the buffer and finds the flag set |
| S2-19 (`is_generated_buffer` disambiguated) | yes |
| S2-20 (structural) | n/a |
| **new in rev 4:** 13a, 13b, 16 (2nd half), 16b, 16c, 21 | each names its exit; 16c carries an explicit caveat that its staging recipe is **not** verified here, with the four-variant fallback rather than a criterion that passes by never reaching its path |

**Re-measured at `7586905` (obligation: paste the output).** Every count
in this document, re-run in this worktree:

```
$ grep -rn 'describe\.buffer' builtin/ --include='*.lua' | wc -l
14
$ grep -rn bypass_intercept builtin | wc -l
21
$ grep -rn --include='*.lua' add_intercept . --exclude-dir=target | wc -l
17
$ grep -rn "is_modified()" src/ pmacs-gpu/ | wc -l
26
$ grep -rn "mark_clean()" src/ | wc -l
6
$ grep -c "dispatch_idle_for" tests/terminal_copy_mode_acceptance.rs
0
```

All unchanged from revision 3's readings, which were taken at `ad41cf1`;
#189 adds no Lua and no tests. The **276 CRDT-dark** figure is
deliberately still not re-quoted — see §10.

---

**Revision 3 — scouted against canonical `githubsucks/main` @ `7586905`,
2026-07-28. Every claim below about pmacs was executed or read at a named
line, not recalled.** The reproductions in §0 and §2 are transcripts of
throwaway probes run in this worktree at `ad41cf1` and deleted before each
commit; nothing between `ad41cf1` and `7586905` touches a file they
measure (#189 edits `COHERENCE.md` only), so they still describe the base.
The counts in §1 and §2.10 are whole greps with the arithmetic shown,
never `| head`.

## Revision history

**Revision 3 answers five review findings on PR #188 @ `516bd35` — three
P1, two P2 — and one sweep the review asked for by class. It also
withdraws one of revision 2's own decisions, reports four defects the
review did not name, and says where it thinks the review is wrong.**

| finding | the decision |
|---|---|
| **P1-1** — the proposed route cannot write to a locked buffer | Confirmed and **worse than stated: the review's first repair option cannot work.** `run_bypass_edit` calls `begin_edit`, whose *first* statement is `ensure_writable()` (`src/buffer.rs:724-735`) — but **reordering `begin_edit`'s two checks does not help**, because both checks are unconditional and both `return Err`; the reorder only changes which error a doubly-failing call reports. Any path that admits a generated write must not call `ensure_writable` **at all**, so the design space is one option, not two. **Q#GB3 is reversed**: generated writes get their **own** `run_buffer_edit` arm and never touch `begin_edit`. The whole transaction — refusal, re-entrancy gate, unlock, write, relock, history, flag clear — becomes a single `&mut Buffer` method with no exit path a caller can miss (new §3.4, new **Q#GB17**). |
| **P1-2** — disambiguation is not carried through name-based identity | Confirmed. The census is new §2.10: **19 units across 14 grep lines**, classified by reading each enclosing function. Two sites are genuinely **broken** by disambiguation, and between them they have **six** downstream consumers, not the three the review named — `listview.open`'s own never-capture-a-panel guard is the fourth listview consumer, and it fails *silently and inverted*, capturing a panel as its own `q` target. New **Q#GB18** routes both by owned `BufferId`. Five further sites are the correct use of a name lookup and are named so a reviewer does not re-derive them. |
| **P1-3** — `unlock_generated` is an unrestricted clear | Confirmed, and the review's second half is decisive: the binding does not achieve its stated purpose. **Revision 2's Q#GB7 is withdrawn as written.** The capability survives only bounded by **lock provenance** (new **Q#GB15**), moves from Stage 1 to **Stage 2**, and its justification narrows from "brick escape" to "the closure of the capability `{ generated = true }` adds". Stage 1 ships no unlock because Stage 1 adds no lock capability `main` does not already expose. |
| **P2-4** — Stage 1 criterion 6 passes through the side-window gate | Confirmed, **and the framing cited the wrong precedent.** `tests/terminal_copy_mode_acceptance.rs`'s `acc16` — which revision 2 named as the model — has **no** `is_side` assertion (`grep -c dispatch_idle_for tests/terminal_copy_mode_acceptance.rs` = 0; it goes through `state.dispatch_idle()`). The test that gets it right is `tests/dired_acceptance.rs:969`. The criterion is rewritten to carry **both** halves — the document-window premise *and* `acc16`'s positive control — because `dispatch_idle_for` has **six** ways to return false and the review named one. |
| **P2-5** — the path-backed refusal is tested only through the wrapper | Confirmed. Stage 2 criterion 6 now exercises **all four** surfaces — the wrapper plus each of `insert` / `delete` / `replace` with `{ generated = true }` — with the misplacement as the explicit bite. |

**Sweep C — "a mechanism was assumed to compose with an existing guard
without reading the guard" (obligation 3).** Findings 1 and 3 are both
that shape. Sweeping the rest of the framing for it found **two more, both
in the shipped primitive, and both unframed anywhere**:

1. **`Buffer::set_generated_contents` lifts a lock it did not install.**
   It sets `read_only = false` unconditionally (`src/buffer.rs:546`), so
   `pmacs.buffer.set_generated_contents(term_buf, "junk")` on a **live
   terminal identity buffer** — whose `read_only` was set by
   `src/terminal/session.rs:305` for reasons that have nothing to do with
   this arc — overwrites its contents and re-locks it as if the primitive
   owned it. This ships today. Q#GB15's provenance field closes it in the
   **write** direction as well as the unlock direction, which is what
   makes a second field worth its cost rather than a one-off for Q#GB7.
2. **`read_only` is also the fold subsystem's "is this a document
   buffer" test.** `document_bytes` (`src/lua_bindings/fold.rs:310-318`)
   returns `None` for any `read_only` buffer, so `pmacs.fold.fold`
   answers `false` with the status `fold rejected: not a document buffer`
   — and `tests/folding_acceptance.rs:570-590` pins exactly that. Locking
   five buffer families therefore **silently disables fold creation** on
   all five. This directly contradicts Q#GB3's own stated rationale
   ("folding a `*compilation*` buffer is possible, so changing this would
   be a silent behaviour change to a pinned seam"): revision 2 preserved
   the *unfold-on-edit* seam while the lock it recommends kills the
   *fold-creation* seam. New **Q#GB16**.

Two further defects found while answering the review, neither of them of
that shape and neither named by the review:

3. **§1.5's correction has landed and this document was about to
   re-assert it.** PR #189 (`main` @ `7586905`, merged after revision 2
   was pushed) corrects `COHERENCE.md` §14's listview consumer list to
   exactly the three call sites §1.5 measured, and moves the row from ✓ to
   ◐. §1.5, §8 and §9's "correction owed" are rewritten from *owed* to
   *landed*. A merged correction must be removed, not restated.
4. **`*help*` has two independent writers, one of them Rust, and §1.4
   named only the Lua one.** `src/help.rs:354-372` `replace_help_buffer`
   does its own `find_by_name(HELP_BUFFER_NAME)`-or-create, writes
   delete-all + insert-all through `Buffer::apply_edit`, and calls
   `mark_clean()` (`:381`) — while `builtin/commands/default.lua:1229-1246`
   does the same thing in Lua and does **not** mark clean. Two owners, two
   copies of the name constant across the FFI boundary, one buffer. It is
   a **fifth** writer mechanism, and it is a reason Q#GB9's deferral of
   Class C is right rather than merely convenient.

**Revision 2 answered five review findings on PR #188 @ `9101bf5` — three
P1, two P2 — and two sweeps the review asked for by class rather than by
item. Nothing was silently rewritten; each change is attributed below.**

| finding | what it changed |
|---|---|
| **P1-1** — three Stage 2 criteria are non-discriminating | Stage 2 criteria 3, 4 and 5 rewritten (§6). All three **passed on the pre-image**: ordinary edits are already refused by the existing intercept, and `Buffer::undo` checks `read_only` *before* it looks at history (`src/buffer.rs:1302`), so "undo returns false" passes against an implementation that locks and never clears. The new wording uses a **bypass write** or Rust-side `Buffer::is_read_only()` to prove locking, and **lifts the lock inside a Rust test** before asserting `NothingToUndo` / `can_undo() == false` to prove clearing. Confirmed against the tree. |
| **P1-2** — staging omits the ownership prerequisite | New §2.8 (measured), new **Q#GB13**, amended **Q#GB5**, and staging changes in §5: ownership-by-handle is now a **prerequisite of the stage that locks each writer**, not a follow-up. Confirmed and materially worse than the review stated — §2.8 measures that a *failed* `pmacs.compile.run` already leaves a foreign buffer permanently un-editable today, and that `M-x buffer.undo` is currently the **only** way to recover a clobbered one. This arc removes that accidental safety net, which is exactly why it cannot ship without ownership. |
| **P1-3** — `mark_clean` can suppress recovery | New §2.9 and a rewritten **Q#GB10**. **Revision 1 was wrong**: it claimed `is_modified` "drives only the mode-line indicator and the buffer-list column". It is also read by `src/autosave.rs:363` — the skip that decides whether a crash-recovery slot is written — and `src/desktop.rs:302`. The rule chosen and framed: **a generated write refuses a buffer that has a `file_path`**, which bounds the contents clobber and the lock as well as the flag. |
| **P2-4** — Q#GB6 conflates byte extent with line extent | **Q#GB6** rewritten. `win.view_top` is a **line index** (`src/window.rs:373-374`) bounded by `TextView::line_count`; `win.cursor` is a byte position bounded by `Buffer::len`. The clamp is now per-coordinate and ungated, matching `rebuild_views_for`'s own shape (`src/editor_core.rs:1853-1857`), and §6 gains a longer-in-bytes / fewer-in-lines pin. |
| **P2-5** — the CRDT-dark count was knowingly stale | **Re-measured at `ad41cf1`: 276 dark** (3,251 vs 3,527), with the command shown in §10. Revision 1 quoted **264**, which `docs/active-work.md:107-115` explicitly labels historical with "the number moves with every merge and must be re-measured, not quoted". |

**Sweep A — every criterion re-audited against its pre-image, not only
3–5.** Two results beyond the cited items. First, **eight criteria pass
on `main` by design** and their bites name a *non-`main`* pre-image; that
is legitimate (`docs/agent-handoff.md` §5: "bite against every pre-image
the fix could plausibly have taken"), but revision 1 did not say so, and
an unlabelled always-green criterion is indistinguishable from a vacuous
one. §6 now carries a **pre-image column for every criterion**. Second,
**Stage 1 criterion 7's stated bite was wrong**: a "partial conversion"
that keeps a `bypass_intercept` write beside the primitive does not
produce a stale paint, it **raises** at the bypass write (§2.4,
measured), so the criterion could never have failed the way it claimed.
Restated as an explicit mutation bite.

**Sweep B — "a capability was made public without bounding who may use it
on what."** Two results beyond P1-2 and P1-3. First, the pathless rule
bounds *what* but not *who*: any Lua, including a third-party package,
can still permanently lock `*scratch*` — pathless, the default buffer,
and the quit target of five different code paths. Second, and decisive,
**the two halves of the protection are not symmetric**: the intercept is
removable (`remove_intercept`, `src/lua_bindings/mod.rs:3433`, used by
the REPL at `repl/init.lua:325-327`) and the rope lock is one-way from
Lua. That falsifies revision 1's stated reason for deferring the unlock
("a binding whose only consumer does not exist yet cannot be pinned"):
the brick scenario **is** a consumer and **is** pinnable. **Q#GB7 is
upgraded from "name it, do not build it" to "ship the unlock in Stage
1."**

> **Revision 3 keeps sweep B's observation and reverses its conclusion**
> (review P1-3). The asymmetry is real. The brick scenario is not a
> *recovery* consumer — by the time anyone reaches for the unlock,
> `set_generated_contents` has already cleared the history — and the
> `*scratch*` exposure it turns on is one `main` already ships, since
> `set_generated_contents` is already public. Sweep B also stopped one
> step short: an unbounded clear of `read_only` is a strictly larger
> capability than the arc adds, because the same flag protects live
> terminal identity buffers. Q#GB7 now lands in **Stage 2**, bounded by
> Q#GB15's provenance.

**Revision 1** — initial framing: the confirmed bug, the classified
census, the primitive decision, staging, and acceptance.

This closes the class-wide half of the invariant `Buffer::set_generated_contents`
opened in terminal copy mode (#178) and that `docs/agent-handoff.md` §4 and
`COHERENCE.md` §14 both record as unfinished: **four writer mechanisms across
five buffer families still pair an erroring intercept with `bypass_intercept`
writes over a writable rope, and every one of them is emptied by undo.**

Two things the arc turns out NOT to be, both discovered by measurement:

- It is **not** "compile is the urgent one". `compile.lua` and the
  `*search-results*` panel rebind all seven undo chords to a no-op
  (`compile.lua:219`, `builtin/commands/default.lua:855`), so reaching
  them needs `M-x`. **`dired.lua` and `listview.lua` rebind nothing**, so
  a bare `C-/` empties them. The cheap half is also the exposed half.
- It is **not** a Lua-only change. Two of the four mechanisms write
  incrementally and cannot use the shipped primitive at all, and a
  buffer the shipped primitive has locked **refuses `bypass_intercept`
  writes** (§2.4, measured) — so partial adoption is impossible and a
  new Rust primitive is required.
- **Added in revision 2:** it is **not** safe to lock these buffers
  before fixing who owns them. Three of the five writers adopt any
  buffer that shares their name (§2.8, measured), and the only thing
  that currently recovers a clobbered user buffer is `M-x buffer.undo`
  — this arc's bug. Ownership is a prerequisite, not a follow-up
  (Q#GB13).
- **Added in revision 3:** and fixing ownership is **not** free, because
  fixing it changes what buffers are *called*. Five sites in `builtin/`
  recover a buffer's identity from its name; two of them break (§2.10),
  and one of those two — `listview.open`'s own `q`-target guard — breaks
  **inverted and silently**, producing exactly the failure its comment
  says it prevents. Q#GB18.
- **Added in revision 3:** `read_only` is **not** this arc's flag. It
  carries three unrelated authorities (§2.11), so a capability defined
  over it reaches all three — which is why the unlock needed provenance
  (Q#GB15), why the lock silently disables fold creation (Q#GB16), and
  why the *shipped* primitive can already overwrite a live terminal's
  identity buffer.

---

## 0. The bug, reproduced

`Buffer::undo` (`src/buffer.rs:1301`) gates on `ensure_writable()`
(`src/buffer.rs:568`) and nothing else. `ensure_writable` reads the Rust
`read_only` field; it never consults the intercept chain. The
`pmacs.buffer.add_intercept(buf, function() error(name .. " is read-only") end)`
idiom therefore protects the *edit* path and leaves the *history* path
wide open, while the owner's own `bypass_intercept` paint lands on the
undo stack for undo to pop.

The user-reachable chain, verified end to end:

`M-x buffer.undo` → `cmd { name = "buffer.undo" }`
(`builtin/commands/default.lua:179`) → `ed.undo()` → `EditorCore::undo`
(`src/editor_core.rs:2575`) → `Buffer::undo` → `ensure_writable`. The
chords `C-/ C-_ C-4 C-x u` are bound globally
(`builtin/keymaps/default.lua:126-136`) and the menu carries it too
(`builtin/menus/default.lua:141`). **No buffer-local rebinding removes
the command**, which is what `compile.lua`'s own comment already admits
("command/menu undo stays dispatchable", `compile.lua:236`).

### 0.1 Measured transcripts

Every line below is probe output from `ad41cf1`.

**listview panel, plain `C-/` through `dispatch_key`** — no rebinding
exists, so this is the whole distance from a keystroke to an empty panel:

```
listview BEFORE   = "H\nrow-one\nrow-two"
listview after C-/ = ""
```

**listview panel, ordinary edit** — the intercept works, which is exactly
why the idiom reads as safe:

```
listview ORDINARY EDIT = false | intercept rejected the edit:
  builtin/runtime/listview.lua:102: *probe-panel* is read-only
```

**dired listing, one `buffer.undo`:**

```
dired BEFORE  = "/tmp/.tmpEJlp9i:\n  -rw-r--r--  1 2026-07-28 17:42 alpha.txt\n  -rw-r--r--  1 2026-07-28 17:42 beta.txt"
dired AFTER 1 = ""
```

**`*shell-command*`, `M-x buffer.undo` through the real minibuffer** (`M-x`,
typed `buffer.undo`, RET):

```
shell BEFORE = "$ printf ...\nDirectory: ...\n\none\ntwo\n\n[shell exited with code 0]\n"
shell after M-x buffer.undo
             = "$ printf ...\nDirectory: ...\n\none\ntwo\n\n[output desynced by external edit]\n"
```

**Read that one carefully — it is the single most important measurement in
this document.** The Q#CM2 revision guard *noticed* and appended its
desync marker. It did **not** prevent anything: the run's exit status is
gone for good, and the buffer is still non-empty. Any acceptance
criterion phrased as "the buffer is not empty" **passes with the bug
live**. See §5, Stage 2 criterion 1.

Driven programmatically to the end, the same buffer empties completely:

```
shell AFTER 1 undo  = "... one\ntwo\n"          (exit marker gone)
shell AFTER 13 undos = ""
```

**`*search-results*`:**

```
search BEFORE = "Searching for: fn main\n\n"
search AFTER  = ""
```

### 0.2 The fix already in the tree, and what it proves

`terminal.lua` is the adopter and the precedent. `render_snapshot`
(`terminal.lua:320-337`) calls `pmacs.buffer.set_generated_contents`, and
the comment at `:322-336` documents this exact defect in these exact
terms. `claim_snapshot` (`:339-396`) keeps the erroring intercept **and**
`set_round_trip_input`, and `:351-366` states the layering that this
framing must preserve at every adopter:

> rope-level read-only protects the daemon copy, round-trip input
> protects the replica copy — and neither substitutes for the other.

A locked buffer measured at `ad41cf1`:

```
bypass write after lock            = false | buffer `*probe*` (id BufferId(4)) is read-only
M-x buffer.undo after lock leaves  = "header\n"
```

---

## 1. The census, with its arithmetic

### 1.1 `bypass_intercept` — 21 grep hits, 16 write sites

`grep -rn bypass_intercept builtin` returns **21** lines. Five of them
are prose in comments, not calls:

| file:line | what it is |
|---|---|
| `compile.lua:265` | comment above `ensure_slot`'s intercept |
| `terminal.lua:304` | comment in `unique_snapshot_name` |
| `terminal.lua:324` | comment in `render_snapshot` (the round-2 note) |
| `dired.lua:478` | comment above `claim_handle` |
| `listview.lua:9` | module header |

21 − 5 = **16 actual write call sites**, and the per-file arithmetic is
9 + 4 + 1 + 2 + 0 = 16:

| file | writes | lines |
|---|---|---|
| `compile.lua` | 9 | 319, 443, 454, 465, 506, 512, 642, 794, 798 |
| `builtin/commands/default.lua` | 4 | 827, 849, 1005, 1007 |
| `dired.lua` | 1 | 371 |
| `listview.lua` | 2 | 60, 61 |
| `terminal.lua` | **0** | — (it adopted the primitive) |

**Two corrections to the counts this lane was briefed with.** `compile.lua`
has **9** write sites, not 10 — the tenth hit is the comment at `:265`.
`terminal.lua`'s two hits are **both comments**; it performs no
`bypass_intercept` write at all, which is the correct state for an
adopter and is worth stating because the raw grep count reads as though
it still does.

### 1.2 `add_intercept` — 17 Lua sites, 6 production

`grep -rn --include='*.lua' add_intercept . --exclude-dir=target`
returns **17** lines: **6** in `builtin/`, **11** under `tests/fixtures/`.
One of the eleven (`tests/fixtures/pmacs-mcp-prompts/init.lua:84`) is a
doc comment, so the fixture *call* count is 10; 6 + 10 = 16 calls across
17 lines. The six production sites:

| site | buffer(s) | shape |
|---|---|---|
| `terminal.lua:367` | terminal copy snapshot | blanket read-only — **adopted** |
| `dired.lua:509` | every dired buffer | blanket read-only |
| `listview.lua:101` | every listview panel | blanket read-only |
| `compile.lua:266` | `*compilation*`, `*shell-command*` | blanket read-only |
| `builtin/commands/default.lua:869` | `*search-results*` | blanket read-only |
| `builtin/packages/repl/init.lua:187` | REPL buffers | **filtering** — §2.5 |

### 1.3 `set_read_only` — zero Lua callers, and no Lua binding

`grep -rn set_read_only builtin tests` returns 5 hits, **all Rust test
code** (`tests/folding_acceptance.rs:587`,
`tests/vterm_stage1_acceptance.rs:139,175`,
`tests/terminal_copy_mode_acceptance.rs:582,584`). The stronger fact:
the Lua binding table registers `"add_intercept"` and no
`"set_read_only"` / `"is_read_only"` at all
(`src/lua_bindings/mod.rs:3409` is the only match in the neighbourhood).
Lua *cannot* set `read_only` today. That matters for Q#GB7.

### 1.4 The classification, by writer mechanism

**Class A — erroring intercept + `bypass_intercept` writes over a
writable rope. This is the bug.**

1. **`terminal.lua`** — copy-mode snapshot. **ADOPTED** (`:336`). Fixed.
2. **`dired.lua`** — every dired buffer. One write, in `paint`
   (`:369-372`): `handle.buf:replace(0, handle.buf:len(), text, {bypass_intercept=true})`.
   A whole-buffer replace already. **Convertible with the shipped
   primitive.**
3. **`listview.lua`** — `*references*`, `*outline*`, `*lsp-help*` (the
   three production `listview.open` callers, all in `lsp.lua`:2056, 2102,
   2513). One writer, `render` (`:50-62`): delete-all then insert-all,
   which is a whole-buffer replace spelled in two ops.
   **Convertible.**
4. **`compile.lua`** — `*compilation*` and `*shell-command*`, both via
   `ensure_slot` (`:258`). Nine writes across five enclosing functions,
   and they are genuinely incremental:

   | enclosing function | line | shape |
   |---|---|---|
   | `resync` (`:309`) | 319 | append desync marker at end |
   | `emit_text` (`:432`) | 443 | append remainder at end |
   | `emit_text` | 454 | append `"\n"` at end |
   | `emit_text` | 465 | **positional `replace`** (CR overwrite) |
   | `apply_events` (`:480`) | 506 | **targeted delete** (erase-to-eol) |
   | `apply_events` | 512 | **targeted delete** (erase-line) |
   | `emit_text_raw` (`:639`) | 642 | append marker at end |
   | `start_run` (`:746`) | 794 | delete-all (run reset) |
   | `start_run` | 798 | insert header (run reset) |

   **NOT convertible.** `emit_text` is a terminal emulator: it tracks
   `slot.out_pos` / `slot.line_start` / `slot.parse_line_start` as byte
   anchors, reads `buf:slice(pos, len)` between writes, and settles
   `slot.expected_rev = buf:revision()` afterwards. A whole-buffer
   replace destroys every one of those anchors.
5. **`builtin/commands/default.lua`** — the independent `*search-results*`
   panel (`ensure_search_panel`, `:857`). Four writes across three
   enclosing functions:

   | enclosing function | line | shape |
   |---|---|---|
   | `search_panel_resync` (`:821`) | 827 | append desync marker |
   | `search_panel_append` (`:844`) | 849 | append match batch |
   | `pmacs.project.search` (`:982`) | 1005 | delete-all (query reset) |
   | `pmacs.project.search` | 1007 | insert header (query reset) |

   **NOT convertible**, for the same reason at smaller scale: the append
   path carries `p.next_row` / `p.expected_rev` bookkeeping.

   **Do not read `ensure_slot` as covering this panel.** It serves
   `*compilation*` and `*shell-command*` only; `*search-results*` has its
   own intercept, its own round-trip mark, its own resync and its own
   writes, and `compile.lua` names it only inside the
   `is_generated_buffer` predicate (`:216`).

**Class B — filtering intercept, deliberately partly editable.**

6. **`builtin/packages/repl/init.lua:187`** — see §2.5. Shares the root
   cause, does not share the remedy. **Out of this arc.**

**Class C — generated, but nothing ever claimed they were protected.**
Keying the inventory on `bypass_intercept` misses these entirely,
because an unprotected buffer needs no bypass:

7. **`*buffer-list*`** — `render_list` (`default.lua:387`) writes with
   plain `buf:delete(0, len)` / `buf:insert(0, body)` (`:403-404`). No
   intercept, no round-trip mark. Whole-replace shape.
8. **`*help*` — TWO independent writers, one Lua and one Rust.
   Corrected in revision 3; revision 2 named only the first.**
   - `show_help_text` (`default.lua:1239`), plain delete-all +
     insert-all (`:1245-1246`), no intercept, **no `mark_clean`**.
   - `replace_help_buffer` (`src/help.rs:354-383`) — `find_by_name(
     HELP_BUFFER_NAME)`-or-create (`:358-360`), delete-all + insert-all
     through `Buffer::apply_edit` (`:365`, `:372` — the
     intercept-*running* path, not the skip path), then
     `Buffer::mark_clean()` (`:381`) with the comment "The help buffer
     is regenerated content".

   The name constant is declared twice, independently, on either side of
   the FFI boundary: `src/help.rs:38` (`pub const HELP_BUFFER_NAME`) and
   `builtin/commands/default.lua:1226`. Neither writer knows about the
   other; they differ on `mark_clean` and on which write primitive they
   use. **Two owners for one buffer is why Class C's deferral (Q#GB9) is
   correct rather than merely convenient** — "make `*help*` immutable"
   is not a conversion, it is first a decision about who owns it.
9. **`*workers*`** — a **Rust** writer, `workers_buffer::render`
   (`src/workers_buffer.rs:65`), using `Buffer::apply_edit` (not the
   skip-intercepts path), delete-all + insert-all, then
   `Buffer::mark_clean()` (`:95`). Its fan-out is a fourth mechanism:
   `queue_generated_buffer_edits` + `rebuild_generated_buffer_views`
   (`src/lua_bindings/mod.rs:7142-7145`).

Class C is a **different defect** — nothing is defeated, because nothing
was claimed. It is named here so the inventory is complete and so a
future reviewer does not re-derive it; §4 keeps it out of this arc.

**Revision 3's correction to the inventory's headline number.** This
document, `docs/agent-handoff.md` §4 and `COHERENCE.md` §14 all say
**four writer mechanisms**. Counting `src/help.rs:354` — a distinct
mechanism by every criterion the others are counted by (its own
find-or-create, its own write primitive, its own clean-marking policy)
— the honest figure across Classes A and C is **five**, over seven
buffer families. The four-row table in the handoff is keyed on
`bypass_intercept` and structurally cannot see it. Carried to the PR
body; the handoff is not this lane's file to edit mid-flight.

### 1.5 The `COHERENCE.md` §14 correction — LANDED, not owed

**Revision 3 rewrites this section from a claim into a record, because
the correction merged while revision 2 was open.**

Revision 1 and 2 recorded that §14's "references, outline, buffer-list,
and project-search all use listview" was wrong: `pmacs.listview.open`
has **three** production callers, all in `lsp.lua` — `*references*`
(`:2056`), `*outline*` (`:2102`), `*lsp-help*` (`:2513`) — while
`*buffer-list*` is hand-rolled in `default.lua` (`render_list`, `:387`)
and `*search-results*` is the independent grep panel.

**PR #189 landed exactly that correction** (`main` @ `7586905`,
`0dd0bf2`): §14's List bullet now names the three `lsp.lua` call sites,
the scorecard row moves from ✓ to ◐, and the §6 picker/panel
parenthetical gains `*lsp-help*`. Nothing is owed. This section survives
only so a reader of the earlier revisions does not go looking for a
correction that is already in the tree, and so the *reason* stays
recorded: the miscount came from counting the `compile.lua` and
`dired.lua` comments that cite "the listview idiom" as adoptions. Those
comments are imitators, and what they imitate is the
erroring-intercept-over-a-writable-rope pattern this document exists to
fix.

---

## 2. Ground truth (measured, not recalled)

### 2.1 What the shipped primitive is

`Buffer::set_generated_contents` (`src/buffer.rs:545`, doc comment
`:507-544`): lift `read_only`, `apply_edit_skip_intercepts` a **single
whole-buffer** `EditOp::Replace`, `clear_history()` (the **call** is
`:553`; `:559` is the definition — a revision-2 miscitation), re-assert
`read_only`, **return the `Edit`**. The Lua binding
(`src/lua_bindings/mod.rs:3079-3095`) fans that `Edit` out via
`notify_buffer_edit_to_windows` (`:1573`) *after* dropping the registry
borrow, because the fan-out re-enters the core.

`clear_history` clears whichever history the buffer has: the v0.1
`undo`/`redo` stacks, and in CRDT mode `CrdtState::clear_undo_history`
(`src/crdt.rs:507`), which rebinds a fresh `UndoManager` to the same doc
because loro exposes no `clear`.

### 2.2 Why history clearing is load-bearing

The doc comment's reason is retention, not tidiness: `read_only`
guarantees the pushed entries can never be popped, so a periodically
refreshed panel accumulates full rope clones nothing will ever release.
CRDT mode has the identical retention inside loro's `UndoManager`.

### 2.3 Why a bare lock is not the answer

`ensure_writable` guards the bypass path too
(`apply_edit_skip_intercepts`, `src/buffer.rs:1055-1056`). Locking a
generated buffer without giving its owner a door refuses the refresh the
buffer exists for. That is why the *pairing* is the primitive and why
there is deliberately no Lua `set_read_only` today.

### 2.4 Partial adoption is impossible — measured

This is the fact that decides the design. Once `set_generated_contents`
has locked a buffer, a subsequent owner write through `bypass_intercept`
is refused:

```
bypass write after lock = false | buffer `*probe*` (id BufferId(4)) is read-only
```

So compile **cannot** convert its run reset (`start_run:794,798`) to the
shipped primitive and keep `bypass_intercept` for streaming: the first
append after the reset raises. The streaming owner needs a write path
that *itself* carries authority. Reads are unaffected — `buf:slice`,
`buf:len` and `buf:revision` all work on a locked buffer, which is what
makes an op-level solution viable at all.

### 2.5 The REPL: same root cause, different remedy — measured

`builtin/packages/repl/init.lua:187` installs
`function(op) return repl._intercept(h, op) end` — a **filtering** policy
(`repl._intercept`, `:686-726`): reject edits wholly inside the
history/prompt region, **truncate** edits that straddle the boundary,
pass edits in the input region. Its own writes use a `_self_write` flag
(`with_self_write`, `:111-116`) rather than `bypass_intercept`, and it
has real teardown (`remove_intercept`, `:325-327`).

Does it share the bug? **Yes — measured — and I am not asserting the
comfortable answer:**

```
repl BEFORE                 = "line one\nline two\n> "
repl ordinary edit at pos 0 = false | REPL: history/prompt region is read-only
                                      (insert at 0; input region begins at 20)
repl AFTER 1 buffer.undo    = "line one\nline two\n"
repl bookkeeping after undo = _history_end=18 / _prompt_end=20  (rope is 18 bytes)
```

Undo deleted the prompt the intercept had just refused to let anyone
touch, and left `_prompt_end` pointing two bytes past the end of the
rope. The marks (`_history_end_mark`, `_prompt_end_mark`) adjust with the
rope, but `_blocks[i].start_byte` are plain integers maintained by hand
in `drop_oldest_block` (`:643-656`) and do not.

**But the remedy cannot be rope-level `read_only`**: the input region
must accept ordinary user edits, which is the whole point of a REPL. The
REPL needs either an undo that consults the intercept chain, or
mark-anchored blocks. Both are different work. **Q#GB8: out of this arc,
named deferral, with its measurement recorded above so the next lane does
not have to rediscover it.**

### 2.6 A pre-existing defect in the shipped primitive — measured

`notify_buffer_edit` (`src/editor_core.rs:1814`) updates each window's
`TextView` and overlays. It does **not** clamp `win.cursor` or
`win.view_top`. Only `rebuild_views_for` (`:1843`) does, and its doc
comment says so explicitly (`:1841-1842`). `set_generated_contents`'s
binding calls the former.

```
cursor before = 29, len = 30
cursor after set_generated_contents(G, 'x\n') = 29, len = 2
row0 after shrink = "x"          (paint did not crash)
cursor after C-p  = 29           (motion did not recover it)
```

A shrinking generated write leaves the window cursor 27 bytes past the
end of the buffer, indefinitely. **This ships today in terminal copy
mode** — refresh a snapshot to a shorter one with the point low in the
buffer and this is the state — and every adopter inherits it.

**The two coordinates fail on different axes** (review P2-4). `cursor` is
a byte position (`src/window.rs:366-367`) bounded by `Buffer::len()`;
`view_top` is a **line index** (`:373-374`, "First buffer *line* shown at
the top") bounded by `TextView::line_count()` (`src/text_view.rs:67`).
The transcript above is the byte case. The line case is **not measured**
— staging it needs a scrolled window — but it is available from the types
alone: a write that grows in bytes while collapsing lines invalidates
`view_top` on a write no byte-length comparison calls a shrink.
`rebuild_views_for` already clamps each against its own bound
(`src/editor_core.rs:1853-1857`); the clamp added to
`notify_buffer_edit` must do the same. Q#GB6.

### 2.7 What `buffer.after-edit` does and does not do

`buf:insert` / `buf:delete` / `buf:replace` do **not** fire
`buffer.after-edit`; the dispatcher and daemon do
(`src/editor.rs:1436,1984,2128,2163`, `src/daemon.rs:2976`).
`compile.lua:714` already relies on this ("hook edits don't re-fire the
hook"). Consequence for §3: a generated write does not run arbitrary Lua,
so the *fan-out* is not a re-entrancy hazard — but a scoped primitive's
**callback body** still is, because it is arbitrary owner Lua.

### 2.8 Three writers adopt any buffer that shares their name — measured

**The invariant already exists in this codebase; three writers simply do
not honour it.** `terminal.lua:300-305` states it verbatim:

> `pmacs.buffer.create` takes any caller-chosen name, so a foreign buffer
> may already be called `*terminal-copy: sh*` [...] **found-by-name is NOT
> adoption**: ownership means "this buffer is in the handle table above",
> exactly as in dired.

`dired.lua:476-504` implements the same rule: `claim_handle` looks up its
**handle table** first, and on a name collision disambiguates
`<2>`…`<99>` (`NAME_VARIANT_LIMIT`, `:474`) or raises. Three writers
instead adopt:

| writer | line | code |
|---|---|---|
| `listview.ensure_panel` | `listview.lua:95` | `find_buffer_by_name(name) or pmacs.buffer.create(name)` |
| `compile.ensure_slot` | `compile.lua:263` | `buffer_named(name) or pmacs.buffer.create(name)` |
| `ensure_search_panel` | `default.lua:861-868` | name scan over `pmacs.buffer.list()`, then `buf or create` |

Measured at `ad41cf1`, a user buffer named `*references*` and then a
references panel:

```
foreign BEFORE               = "my precious notes"
foreign AFTER listview.open  = "H\nr1"
buffers named *references*ish = 1              (no disambiguation happened)
ordinary edit to MINE now    = false | intercept rejected the edit:
                               listview.lua:102: *references* is read-only
```

The user's buffer is clobbered **and left permanently un-editable**,
because `ensure_panel` installs an erroring intercept whose handle it
discards.

**Compile is worse, and it is worse on a path that fails.**
`pmacs.compile.run` calls `ensure_slot` (`compile.lua:1090`) *before*
`start_run` validates `opts.display` (`:752-757`). Measured:

```
compile.run('true', { display = 'bogus' })
  = false | compile.lua:754: compile.run: unknown display "bogus"
foreign *compilation* contents after the FAILED call = "my precious notes"
ordinary edit to MINE after the FAILED call
  = false | intercept rejected the edit: compile.lua:267: *compilation* is read-only
```

A call that **raised and did nothing else** left the user's buffer
uneditable. Q#GB5's revision-1 recommendation — an empty
`set_generated_contents` at the end of `ensure_slot` — would make that
same failing call **empty the buffer and lock the rope**. Q#GB5 is
amended accordingly.

**Why this is a prerequisite and not a follow-up.** Today the clobber is
recoverable, and the thing that recovers it is *this arc's bug*:

```
after clobber = "H\nr1"
after undo 1  = ""
after undo 2  = "my precious notes"
```

`M-x buffer.undo` is currently the only way back. After adoption the rope
is `read_only`, the history is cleared by the same call that wrote, and
§1.3 measured that **no Lua binding can clear `read_only`**. The arc
therefore converts a recoverable clobber into an unrecoverable one, and
it removes the accidental safety net in the same commit that removes the
need for it. Q#GB13.

**Dired needs none of this work** — it already disambiguates — which is
why it is the cheaper of Stage 1's two adopters despite being the newer
one.

### 2.9 `is_modified` reaches autosave and desktop persistence — a revision-1 error

**Revision 1 stated that the flag "drives only the mode-line indicator
and the buffer-list column". That is wrong**, and it was wrong because
the sweep was `grep -rn '\.modified' builtin` plus a narrow `src` path
rather than `grep -rn 'is_modified' src`. The full sweep finds two more
consumers, both load-bearing:

- **`src/autosave.rs:359-364`** — the per-buffer skip:
  `let Some(path) = buf.file_path() else { continue };` then
  `if !buf.is_modified() { continue; }`. A clean buffer gets **no
  crash-recovery slot written**.
- **`src/desktop.rs:298-303`** — `SavedBuffer { path, modified: b.is_modified() }`,
  again only for buffers with a `file_path`.

Both gate on `file_path()` being `Some` before they read the flag. That
is the fact Q#GB10's revised rule turns on.

**Revision 3: the sweep was under-run a second time, and the arithmetic
is stated here so it is not under-run a third.** `grep -rn 'is_modified()'
src/ pmacs-gpu/` returns 26 lines. Removing the accessor definition and
the 18 `src/buffer.rs` unit-test assertions leaves **7 production
consumers**:

| consumer | load-bearing? |
|---|---|
| `src/autosave.rs:363` | **yes** — the crash-recovery skip (§2.9 above) |
| `src/desktop.rs:302` | **yes** — the persisted `SavedBuffer.modified` |
| `src/editor.rs:3704` | no — the TUI mode-line `*` |
| `src/semantic_render.rs:1347` | no — the **semantic frontend's** statusline payload |
| `src/help.rs:131` | no — the `Modified:` line of describe-buffer text |
| `src/instance_buffer.rs:401` | no — an assertion, not a read |
| `src/lua_bindings/mod.rs:1262`, `:6359` | no — `buf:is_modified()` and `describe.buffer().modified`, which `default.lua:395` renders |

Revision 1 said two consumers; revision 2 said four; the true figure is
**seven, of which two are load-bearing**. Both new ones
(`semantic_render.rs:1347`, `help.rs:131`) are display, so **Q#GB10's
conclusion is unchanged** — but the conclusion was reached twice from an
incomplete count, and only the arithmetic makes that visible.

Also found in the same sweep, and reused below: `mark_clean()` has six
callers (`grep -rn 'mark_clean()' src/`, minus the definition).
`src/instance_buffer.rs:95,114`, `src/workers_buffer.rs:76,95` and
`src/help.rs:381` are all **generated-buffer writers that already mark
clean**; `src/editor_core.rs:1945` is the save path. So the convention
Q#GB10 adopts is established by **three** Rust writers, not the one
revision 1 cited.

### 2.10 The name-based identity census, with its arithmetic

Review P1-2 named two consumers. This is the whole set, and the
classification comes from reading each enclosing function, never from the
grep line.

**Scope.** A site is in scope when a *disambiguated* name would change
its answer: it either (i) recovers a buffer's identity by comparing that
buffer's name against an expected value, or (ii) keys a table by a buffer
name. `pmacs.describe.buffer(id).name` is the **only** Lua surface that
yields a buffer's name — `buffer_info_table`
(`src/lua_bindings/mod.rs:6352-6364`) sets `name`, `length`, `modified`,
`view_count` and nothing else, and no other binding exposes it — so
`grep -rn 'describe\.buffer' builtin/ --include='*.lua'` is a complete
frontier for `builtin/`.

**The arithmetic.** That grep returns **14** lines. Five of them are the
bodies of shared helpers rather than decisions:

| helper | file:line | callers |
|---|---|---|
| `buffer_name` | `terminal.lua:276` | 1 (`:293`) |
| `buffer_named` | `terminal.lua:283` | 2 (`:308`, `:311`) |
| `buffer_named` | `dired.lua:196` | 3 (`:491`, `:495`, `:914`) |
| `buffer_named` | `compile.lua:194` | 2 (`:263`, `:1052`) |
| `find_buffer_by_name` | `listview.lua:32` | 2 (`:95`, `:190`) |

14 − 5 helper bodies = **9 direct sites**; the five helpers expand to
1 + 2 + 3 + 2 + 2 = **10 call sites**; 9 + 10 = **19 units**, and the
five classes below partition them 2 + 3 + 2 + 5 + 7 = **19**.

**Class 1 — BROKEN by disambiguation. In scope for this arc (Q#GB18).**

1. **`listview.lua:42-44`, `panel_for_current_buffer`.** `panels` is
   written `panels[name] = p` at `:97` with the **requested** name and
   read `return panels[d.name]` at `:44` with the **actual** name. A
   panel created as `*references*<2>` can never resolve its own record.
   **Four consumers, not the three the review named:**

   | consumer | line | what breaks |
   |---|---|---|
   | `listview.visit` (RET/SPC) | `:150` | returns early; RET does nothing |
   | `listview.refresh` (`g`) | `:161` | returns early; `g` does nothing |
   | `listview.quit` (`q`) | `:177` | returns early; `q` does nothing |
   | `listview.open`'s capture guard | `:118-123` | **fails inverted, and silently** |

   The fourth is the one worth reading twice. `listview.open` captures
   the return target with
   `if active and not panel_for_current_buffer() then p.prev = active end`,
   and the comment above it states the intent: "never another panel
   (chained panels would trap `q` in a loop; restore targets the last
   real buffer)". When `panel_for_current_buffer()` cannot recognise a
   disambiguated panel it returns `nil`, the guard reads as "the current
   buffer is not a panel", and the panel is captured as its own `q`
   target — **exactly the loop the guard exists to prevent**, produced by
   the guard. The other three fail closed and visibly; this one fails
   open and quietly, which is why it needs its own criterion rather than
   riding on the other three.

2. **`compile.lua:214-216`, `pmacs.compile.is_generated_buffer`.**
   `return d.name == COMPILATION or d.name == SHELL_OUT or d.name ==
   SEARCH_RESULTS`. **Two consumers**, both the Q#CM11
   never-capture-a-generated-buffer `q`-target discipline:
   `compile.lua:762` (compile's own capture) and
   `default.lua:993-994` (the search panel's). A disambiguated
   `*compilation*<2>` is not recognised, so it gets captured as a `q`
   target and `q` returns the user to a generated buffer.

   Note that `compile.lua:232`'s `slot.name == COMPILATION` is **not** in
   this class and needs no change: `slot.name` is the record's own field,
   set from the module constant `ensure_slot` was called with, and never
   from a buffer's actual name.

   Note also that `slots` (`compile.lua:185`) is **not** broken by
   disambiguation, unlike `panels`: it is keyed by the module constant at
   both write (`:262`) and read (`:259`, `:923`, `:1104`), and the only
   buffer→slot direction is `slot_for_buffer` (`:200-206`), which
   compares `slot.buf == buf` by id. Compile got this half right and
   listview did not; the census is what makes that visible, and it means
   Q#GB18's compile work is one predicate, not a table rewrite.

**Class 2 — the find-by-name adoption Q#GB13 already removes.** In scope,
already framed. `listview.lua:95` (Stage 1), `compile.lua:263` (Stage 2),
`default.lua:863` (Stage 2). **3 sites.**

**Class 3 — the same adoption defect in Class C families.** Out of arc
(Q#GB9), named so it is not re-derived: `default.lua:380`
(`find_list_buffer`, `*buffer-list*`) and `default.lua:1231`
(`find_or_create_help_buffer`, `*help*`). **2 sites.** The Rust-side
instance of the same shape, `src/help.rs:358-360`, is outside this
census's `builtin/` frontier and is recorded in §1.4 instead.

**Class 4 — collision probes: the CORRECT use of a name lookup.**
`dired.lua:491`, `:495` (`claim_handle`'s `<2>`…`<99>` walk);
`terminal.lua:308`, `:311` (`unique_snapshot_name`); `terminal.lua:293`
(`snapshot_base_name`, which *derives* a new name from a name and
recovers no identity). **5 sites.** These are the shape Q#GB13 asks the
other three writers to adopt, so they are the reference implementation,
not debt.

**Class 5 — correct by construction; a disambiguated name does not change
the answer.** **7 sites.**

- `default.lua:391` — `render_list` prints `d.name` in a column. Display.
- `default.lua:604` — `switch-to-buffer` matches the name the **user
  typed**, sourced from the same registry that would show a
  disambiguated name. Correct precisely because it is name-based.
- The five `*scratch*` fallbacks — `default.lua:581`, `:1145`,
  `listview.lua:190`, `compile.lua:1052`, `dired.lua:914`. `*scratch*` is
  an unowned shared buffer that no writer in this arc disambiguates, so
  its name **is** its identity.

**One flag on Class 5, carried rather than fixed.** Those five
`*scratch*` fallbacks are five independent copies of one find-or-create,
and they are correct only while `*scratch*` stays unowned and
undisambiguated. If a future lane gives `*scratch*` an owner — a
plausible move, since it is the quit target of all five paths — all five
break together and nothing in the tree connects them. Named in §8.

### 2.11 `read_only` is one boolean serving three policies

Revision 2 treated `read_only` as this arc's flag. It is not, and both
review P1-3 and sweep C turn on that.

| policy | who sets or reads it | what it means there |
|---|---|---|
| **generated lock** | `Buffer::set_generated_contents` (`src/buffer.rs:546`, `:554`) | "the owner's write path is the only writer" |
| **terminal identity** | `src/terminal/session.rs:305`, at `TerminalSession::open` | "the host may not edit this at all — not by edit, not by undo, not by remote CRDT import" |
| **"is this a document buffer?"** | `document_bytes` (`src/lua_bindings/fold.rs:310-318`), **reading** it | Q#FD11's foldability test |

`Buffer::set_read_only`'s own doc comment (`src/buffer.rs:496-502`)
describes only the second, and the third is a *reader* that was written
when the second was the only writer —
`tests/folding_acceptance.rs:570-573` says so in as many words:
"terminals are read-only, so a read-only buffer is not foldable."

Six paths gate on the flag through `ensure_writable`
(`src/buffer.rs:568`): `begin_edit` (`:725`), `apply_edit` (`:773`),
`apply_remote_crdt_op` (`:845`), `apply_edit_skip_intercepts` (`:1056`),
`undo` (`:1302`), `redo` (`:1410`). That breadth is the point of the flag
and is not in question. What is in question is that a single boolean
carries three unrelated *authorities*, so a capability defined over it —
in either direction — necessarily reaches all three. Q#GB15 and Q#GB16
are the two consequences.

---

## 3. The primitive decision (Q#GB1)

**The question this arc exists to answer: what write primitive do
`compile.lua` and the search panel need?**

### 3.1 Recommendation

**`Buffer::apply_generated_edit(op: EditOp) -> Result<Edit, BufferError>`
— one authorized op at a time — exposed to Lua as a new option key on
the mutators that already exist:**

```lua
buf:insert(pos, text,   { generated = true })
buf:delete(start, end_, { generated = true })
buf:replace(s, e, text, { generated = true })
```

Semantics, per call, entirely inside one `with_registry_mut` and one
`&mut Buffer` method — the exact ordering, including every error path, is
§3.4, which revision 3 adds because review P1-1 showed revision 2 had no
workable one. The binding then fans the `Edit` out through the
`notify_buffer_edit_to_windows` call it **already makes**
(`src/lua_bindings/mod.rs:1291`, `:1302`, `:1322`), after the borrow has
dropped.

`Buffer::set_generated_contents(bytes)` is reimplemented as
`apply_generated_edit(Replace { range: 0..len, bytes })`. **It keeps its
name, its signature, its doc comment and its tests** — it becomes the
whole-buffer spelling of one primitive rather than a second primitive.

**One sentence for why it wins: it is the only candidate in which the
buffer is never observably unlocked, because the lift and the re-assert
happen inside a single registry borrow with no Lua in between — so there
is no flag to clear on an error path, no yield to defend against, and
nothing for a reviewer to audit site by site.**

### 3.2 Why the alternatives lose

**A. `append_generated_contents` — provably insufficient.** `compile.lua`
does a positional `replace` at `emit_text:465` (the CR overwrite that
makes progress bars work) and two targeted `delete`s at
`apply_events:506,512` (erase-to-eol, erase-line). Append cannot express
any of the three. Dead on the census.

**B. Scoped `with_generated_writes(buf, fn)` — the pattern this project
has already been burned by.** It is the cheapest on history (one
`clear_history` per scope instead of one per op) and that is its only
real advantage. Against it:

- **The unlocked interval is the callback's whole duration, and the
  callback is arbitrary owner Lua.** Drawn loosely around `start_run`
  (`compile.lua:746`), the scope spans `pmacs.window.switch_buffer(buf)`
  and `pcall(pmacs.process.spawn, spec)` — the buffer would be writable
  across a process spawn. Drawn tightly, compile needs four separate
  scopes (`start_run`'s reset, `resync`, `feed_bytes`, `finish_run`), each
  needing its own audit for what the body reaches.
- **Correctness reduces to "a flag cleared on every exit."** That is the
  exact shape `docs/agent-handoff.md` §5 and #155 record as a repeat
  offender, and the REPL already had to defend its own version of it:
  `with_self_write` (`repl/init.lua:111-116`) wraps in `pcall`
  specifically because "a single failed write would leave the bypass on
  for every subsequent user edit". Adding a second instance of a pattern
  the tree already documents as fragile is a poor trade for one saved
  `clear_history` per batch.
- **Yield is an error here, which helps but does not rescue it.** A Lua
  callback that yields across the Rust boundary raises
  `attempt to yield across C-call boundary` (observed in this worktree
  while probing `pmacs.dired.open`), so a yielding body surfaces as
  `Err` — but the relock must still run on that path, which is the same
  obligation.
- It is strictly harder to review: a per-site scope audit versus a
  mechanical option-key change at 16 call sites.

**C. Standalone `generated_edit(buf, op)`.** Identical semantics to the
recommendation, worse ergonomics: it re-implements the three-op argument
parsing that `buf:insert/delete/replace` already own, and turns adoption
from an option-key change into a rewrite of 16 call sites. Recommended
only if the user objects to `{ generated = true }` sitting beside
`{ bypass_intercept = true }` in the same options table.

**D. Make `Buffer::undo`/`redo` consult the intercept chain.** This would
fix all six families at once, including the REPL, and it deserves an
explicit rejection rather than silence. Against it: (i) there is no
`EditOp` to hand the chain — v0.1 undo is a whole-rope swap
(`src/buffer.rs:1327-1328`) and CRDT undo is materialize-and-replace
(`undo_crdt_mode`, `:1365`), so the chain would have to be given a
synthetic op it was never designed to see; (ii) it changes behaviour for
every intercept in the tree, including the *transforming* ones
(auto-pair, lean-input, the REPL's truncation) which have no business
rewriting an undo; (iii) an erroring intercept becomes a new Lua-raise
failure path out of `EditorCore::undo`, which today cannot fail that way.
It may still be the right answer **for the REPL specifically** — recorded
in Q#GB8's deferral, not adopted here.

### 3.3 The four questions the recommendation must answer

**How many `Edit`s are fanned out, and when?** One per generated op,
immediately, by the binding that already does it. `compile.lua`'s
`emit_text` fast path emits one insert for a whole output batch, so a
typical `feed_bytes` produces one to three ops; a CR-heavy progress bar
produces more. This is exactly today's fan-out count — the conversion
changes authority, not cardinality.

**Per-op or per-scope history clearing?** Per op, and it is cheap by
construction: because `read_only` is re-asserted immediately, **at most
one** v0.1 undo entry can exist when the clear runs and the redo stack is
always empty, so the v0.1 clear is O(1). In CRDT mode the clear rebinds
a fresh `UndoManager` (`CrdtState::clear_undo_history`), and
`create_undo_manager` (`src/crdt.rs:154`) is `UndoManager::new(doc)` plus
`set_max_undo_steps` — a subscription registration, not a document copy,
so it is O(1) in document size too. **Measurement obligation, not a
claim:** Stage 2 must show a streaming compile run does not regress
against the existing compile-mode timings in both configurations. If it
does, the escape hatch is to suppress recording rather than clear it —
recorded as a named deferral rather than designed speculatively.

**CRDT-mode behaviour?** Identical to `set_generated_contents` today. The
`Edit` carries `crdt_op` when the buffer is CRDT-backed, and
`notify_buffer_edit_to_windows` queues it via
`queue_daemon_origin_crdt_op` (`src/lua_bindings/mod.rs:1582`) so replica
mirrors import the owner's write. History clearing goes to loro's
`UndoManager`. Nothing new.

**How do the returned edits reach the fan-out without a live registry
borrow?** By construction, unchanged since #178: `run_bypass_edit`
(`src/lua_bindings/mod.rs:1445`) closes its `with_registry_mut` before
returning, and the mutator bindings call
`notify_buffer_edit_to_windows` afterwards. `run_generated_edit` (§3.4)
occupies the same position and closes its borrow the same way.

### 3.4 The transaction, and why revision 2 had none (review P1-1)

**Revision 3 adds this section. It is the substance of P1-1 and it
reverses Q#GB3.**

**The defect, confirmed at line level.** Revision 2's Q#GB3 routed
generated writes "through `run_buffer_edit`'s bypass arm". That arm is
`run_bypass_edit` (`src/lua_bindings/mod.rs:1445-1454`), whose first act
on the buffer is `buf.begin_edit()`, and `begin_edit`'s first statement
is `self.ensure_writable()?` (`src/buffer.rs:724-725`). A generated write
must pass **while `read_only` is set** — that is the entire point — so
every generated write after the first would be refused, and for compile,
whose Q#GB5 lock is installed during `ensure_slot`, even the first
streaming write would be refused. As written, revision 2's design was
dead on arrival at every buffer it governs.

**The review offers two repair options; one of them cannot work, and
saying so is the first design decision.** The suggestion to reorder
`begin_edit`'s two checks does not repair anything:

```rust
pub fn begin_edit(&mut self) -> Result<(), BufferError> {
    self.ensure_writable()?;                       // src/buffer.rs:725
    if self.editing_in_progress { return Err(ConcurrentEdit { .. }); }
    self.editing_in_progress = true;
    Ok(())
}
```

Both checks are unconditional and both `return Err`. Reordering changes
only **which** error a call that fails both reports; a locked buffer is
still refused, one line later. It is also not free: at least one shipped
test asserts on the *text* of that error
(`tests/dired_acceptance.rs:999`, `status(&s).contains("read-only")`),
and `BufferError::ReadOnly` and `ConcurrentEdit` render differently
(`src/buffer.rs:1794`, `:1824-1831`). **Any path that admits a generated
write must not reach `ensure_writable` at all.** So there is one option,
not two: a separate entry point. Recorded as a disagreement with the
review rather than complied with silently.

**Where the concurrency gate lives.** Inside `Buffer`, in the generated
path itself, duplicating `begin_edit`'s **second** check and not its
first. It is **not** exposed as a public `begin_generated_edit`: making
it a public pair would recreate at the binding layer the exact
"a flag cleared on every exit" shape §3.2 rejects candidate B for. One
method, one exit set, nothing for a caller to forget.

**What that does to the `ConcurrentEdit` contract.** Nothing observable,
and the contract is still needed. Two directions:

- **Inward** (something re-enters *during* a generated write): impossible
  by construction, and this is worth stating because it is what makes the
  transaction safe to hold across a single borrow. `apply_edit_skip_intercepts`
  runs `View::on_edit`, never `View::intercept_edit`
  (`src/buffer.rs:1055-1060`), and `LuaInterceptView`
  (`src/lua_bindings/mod.rs:1755-1798`) implements **only**
  `intercept_edit` — it inherits `View::on_edit`'s default no-op body
  (`src/view.rs:252-254`). Of the seven production `on_edit`
  implementations (`grep -rn 'fn on_edit' src/`: `text_view.rs:150`,
  `fold.rs:274`, `overlay.rs:248`, `syntax.rs:1637`, plus test doubles)
  none calls into Lua. **A generated write runs no Lua**, so nothing can
  re-enter it.
- **Outward** (a generated write issued *from inside* a managed edit on
  the same buffer): entirely possible — a Lua intercept body on buffer X
  calling `X:insert(pos, s, { generated = true })` — and it must still
  fail. `run_managed_edit` phase 2 runs that body with the registry
  borrow released and its `InterceptContext` already snapshotted
  (`src/lua_bindings/mod.rs:1477-1487`); a generated write landing in
  between would leave phase 3 applying an op computed against a rope that
  no longer exists. So the generated path sets and clears
  `editing_in_progress` exactly as `begin_edit`/`end_edit` do, and this
  case surfaces `BufferError::ConcurrentEdit` unchanged.

**Cleanup is driven by an explicit outcome, NOT by inferring one from
`revision`. Rewritten in revision 4 (review round 3, P1-1); revision 3's
predicate was wrong in three directions and this section says so before
it says anything else.**

Revision 3 wrote `if self.revision() != rev_before { clear_history(); … }`
and argued the predicate was *exact* because `revision` bumps after the
undo push and before the `on_edit` broadcast. **That argument is sound
for the v0.1 undo stack and does not extend past it.** Three failures:

1. **A successful no-op keeps history that the contract says cannot
   exist.** An empty buffer can carry substantial undo history — insert,
   then delete back to empty. `set_generated_contents("")` then takes
   the no-op arm (`src/buffer.rs:1245-1253`), returns `Ok`, never bumps
   `revision`, so revision 3 skipped **both** the clear and
   `mark_clean`: the buffer ends locked, **modified**, and carrying
   poppable history that `unlock_generated` re-exposes. Shipped
   `set_generated_contents` clears unconditionally (`:551-553`), so
   revision 3's predicate was a **regression of an existing contract**,
   not a refinement of one. And it lands on a path this document
   *prescribes*: Q#GB5's `ensure_slot` lock is exactly
   `set_generated_contents(slot.buf, "")`.
2. **CRDT mutation happens upstream of `revision` entirely.**
   `apply_to_crdt_then_normalize_bytes` runs **before** the rope edit
   (`src/buffer.rs:1194-1201`), and for `EditOp::Replace` it is two ops —
   `crdt.delete`, then `crdt.insert` (`:1140-1163`). The code's own
   comment states the hazard: if the delete succeeds and the insert
   fails, "the CRDT is mid-transaction (range deleted but replacement
   not inserted) and the rope is unchanged. This is an invariant
   violation." In that state `revision` has **not** advanced, so revision
   3's predicate neither clears the CRDT's history nor notices the
   `rope ≡ CRDT projection` divergence — in precisely the case that most
   needs both.
3. **Found while fixing the other two: revision 3's unconditional relock
   locks a buffer nobody wrote to.** `b:insert(mid_codepoint, "x",
   { generated = true })` on a *fresh, writable* `*scratch*` is rejected
   by the CRDT after the unlock, and revision 3 then set
   `read_only = true` on the way out. The caller gets an error **and** a
   locked buffer it never successfully wrote. Not reported by the review;
   same class as the other two (a rule derived for one case applied to a
   case whose ordering differs).

**The outcome value.** `run_rope_edit_and_broadcast` reports what it
actually did instead of leaving its caller to guess. This is a real
signature change and not a free one — see the cost note below.

```rust
/// What a generated write did, reported by the apply rather than
/// inferred by its caller. The variant, not `revision`, selects cleanup.
enum GeneratedOutcome {
    /// The rope changed and an undo entry exists.
    Applied(Edit),
    /// A semantic no-op: rope unchanged, no undo entry pushed. The call
    /// SUCCEEDED, so the buffer is now a generated buffer.
    NoOp(Edit),
    /// Refused before anything mutated: rope, CRDT and history are all
    /// exactly as they were.
    Rejected(BufferError),
    /// The rope changed and a later stage failed. An undo entry exists
    /// over contents no caller can see.
    AppliedThenFailed(BufferError),
    /// CRDT only: the CRDT was partially mutated and the rope was not.
    /// `rope ≡ CRDT projection` no longer holds.
    Diverged(BufferError),
}
```

**The cleanup each variant triggers.** `entry` is the `read_only` value
observed on entry.

| outcome | history | `mark_clean` | `read_only` after | returns |
|---|---|---|---|---|
| `Applied` | **cleared** | yes | `true` | `Ok(Edit)` |
| `NoOp` | **cleared** | yes | `true` | `Ok(Edit)` |
| `Rejected` | untouched | no | **restored to `entry`** | `Err` |
| `AppliedThenFailed` | **cleared** | **no** | `true` | `Err` |
| `Diverged` | **untouched** | no | `true` | `Err`, and it must **surface** |

`editing_in_progress` is cleared on **all five**, unconditionally.

**Why `NoOp` clears — the rule that replaces revision 3's.** The
invariant is a property of the **buffer**, not of the write: *a buffer
that is generated-locked carries no history.* Every outcome that leaves
the buffer generated-locked therefore clears, and `NoOp` leaves it
locked because the call succeeded. `Rejected` is the only success-shaped
exception and it is not one — the buffer is not newly generated, so
there is nothing for the invariant to apply to. Stated this way the rule
needs no reference to `revision` at all, which is what makes it
mode-independent.

**Why `Rejected` restores rather than relocks.** Revision 3's "relock
unconditionally" was justified by "the contract is *leave it genuinely
immutable*" — true of a write that **happened**. A refusal is not a
write. Restoring the entry value keeps the refusal total: a fresh buffer
stays writable, an already-generated buffer stays locked, and no caller
can lock a buffer by failing to write to it.

**Why `Diverged` must surface rather than be cleaned.** This is the
variant the review specifically asks about, and the honest answer is
that **this arc cannot fix it**:

- The rope is intact and the CRDT is not. Nothing local reconstructs the
  deleted range — loro exposes no rollback at this seam, and the
  `export_updates_since` that would name the delta runs after both ops.
- **Clearing history would destroy the last local record of the
  pre-edit rope**, which is the only material anything could later
  reconcile from. So `Diverged` clears nothing.
- Locking is the strongest available *containment*: it stops further ops
  compounding a divergence that already exists. So `read_only = true`,
  and this is the one place where locking a buffer on an error path is
  correct — because the buffer really is no longer safe to write.
- It returns a **distinct** `BufferError` variant rather than reusing
  `CrdtRejected`, and the Lua binding surfaces it via
  `pmacs.editor.set_status` rather than swallowing it. A caller must be
  able to tell "your op was refused, nothing happened" from "this
  buffer's CRDT and rope no longer agree."

**Scope, stated plainly: `Diverged` is a PRE-EXISTING hazard this arc
exposes, not one it creates.** `apply_edit` and
`apply_edit_skip_intercepts` reach the same two-op replace today and
report it as an ordinary `CrdtRejected`; nothing in the tree
distinguishes it. Making it a named variant is this arc's contribution;
**repairing the divergence is not, and is recorded in §8 as its own
lane.** Splitting a CRDT `Replace` into a single transactional op, or
reconciling the two, is loro-level work with no bearing on generated
buffers specifically.

**The ordering, with every exit path named.**

```rust
pub fn apply_generated_edit(&mut self, op: EditOp<'_>) -> Result<Edit, BufferError> {
    // (1) Q#GB10: path-backed refusal. Before any state change.
    if self.file_path.is_some() { return Err(GeneratedWriteOnFileBuffer { .. }); }
    // (2) Q#GB15: this buffer's read_only is an identity protection.
    if self.identity_protected { return Err(ReadOnly { .. }); }
    // (3) re-entrancy gate — begin_edit's SECOND check, not its first.
    if self.editing_in_progress { return Err(ConcurrentEdit { .. }); }
    // (4) bounds pre-validation, so an invalid range costs nothing.
    self.validate_op_bounds(&op)?;

    let entry_read_only = self.read_only;
    self.editing_in_progress = true;
    self.read_only = false;                       // the ONLY unlocked interval
    let outcome = self.apply_generated_inner(op); // -> GeneratedOutcome
    match &outcome {                              // (5) per-variant cleanup
        Applied(_) | NoOp(_) => {
            self.read_only = true;
            self.clear_history();
            self.mark_clean();
        }
        AppliedThenFailed(_) => { self.read_only = true; self.clear_history(); }
        Diverged(_)          => { self.read_only = true; }
        Rejected(_)          => { self.read_only = entry_read_only; }
    }
    self.editing_in_progress = false;             // (6) unconditional, all paths
    outcome.into_result()
}
```

| exit | when | `read_only` after | `editing_in_progress` | history | contents |
|---|---|---|---|---|---|
| (1) | `file_path` is `Some` | unchanged | unchanged (`false`) | **untouched** | untouched |
| (2) | identity-protected (terminal) | unchanged (`true`) | unchanged | **untouched** | untouched |
| (3) | re-entrant on the same buffer | unchanged | unchanged (`true`, the outer edit's) | **untouched** | untouched |
| (4) | range out of bounds | unchanged | unchanged (`false`) | **untouched** | untouched |
| `Rejected` | mid-codepoint position, CRDT | **restored to entry** | `false` | **untouched** | untouched |
| `AppliedThenFailed` | a view rejected `on_edit` | `true` | `false` | **cleared** | **mutated** |
| `Diverged` | CRDT delete ok, insert failed | `true` | `false` | **untouched** | rope untouched, **CRDT diverged** |
| `NoOp` | empty write over an empty rope | `true` | `false` | **cleared** | unchanged |
| `Applied` | the ordinary case | `true` | `false` | **cleared** | replaced |

**Why `end_edit` cannot be skipped.** It is line (6), unconditional and
outside the `match`, in the same function as line (3) that set it — there
is no caller who could return early past it, which is the whole reason
the transaction is one `Buffer` method rather than a binding-level pair.
Review round 2's P1-1 is right that a leaked `editing_in_progress` wedges
the buffer for **every** later edit (`begin_edit` `:726-731` and
`apply_edit` `:774-779` both refuse), and shipped
`set_generated_contents` avoids that hazard today only by never setting
the flag at all — which is also why it has no re-entrancy gate today, a
gap this closes.

**What bounds pre-validation is still for.** It is no longer load-bearing
for history (the `Rejected` variant covers that) but it is kept, for one
reason: it moves the most common caller error out of the unlocked
interval entirely, so an out-of-range op cannot even transiently unlock a
buffer. **It is not a substitute for the criteria** — review round 3's
P1-2 is exactly the trap of pinning the transaction through the one path
that never enters it (§6, Stage 2 criteria 15-16).

**Cost, stated rather than buried — and revision 4's outcome enum is the
larger half of it.** `run_rope_edit_and_broadcast` must distinguish
`Rejected` from `Diverged`, and today it cannot: both surface as
`Err(CrdtRejected)` because `apply_to_crdt_then_normalize_bytes` uses a
bare `?` on the second op (`src/buffer.rs:1155-1163`). Distinguishing
them means that function tracking whether its `delete` succeeded before
its `insert` failed. That is a real change to a shipped CRDT path, it is
in Stage 2's scope, and it is the reason `Diverged` is a *named* variant
rather than a note — a variant nothing can construct is not a design.
**If the user prefers a smaller Stage 2**, the fallback is to ship four
variants, fold `Diverged` into `Rejected`, and accept that a
CRDT-mid-transaction failure is indistinguishable from a clean refusal —
which is the status quo, and should be a stated trade rather than a
silent one.

`validate_op_bounds` is a new
private helper duplicating the bounds arithmetic `Rope::insert` /
`delete` / `replace` already perform (`RopeError::OutOfBounds`,
`src/rope.rs:371-383`). It is O(1) and it is duplication; the alternative
is a dry run, and there isn't one. Named in §7 as a bet.

---

## 4. Decisions

*Reading order note: Q#GB15–18 are new in revision 3 and sit **between
Q#GB7 and Q#GB8** rather than at the end, because each descends directly
from the decision above it — Q#GB15 and Q#GB16 are the two consequences
of §2.11's finding that `read_only` is not this arc's flag, and Q#GB17
and Q#GB18 are what review P1-1 and P1-2 turned into decisions. The
numbering is chronological; the placement is topical.*

**Q#GB1 — The streaming primitive.** `Buffer::apply_generated_edit(op)`,
exposed as `{ generated = true }` on the three Lua mutators.
`set_generated_contents` becomes its whole-buffer wrapper, keeping name,
signature and tests. Rationale and rejected alternatives: §3.

**Q#GB2 — `generated` is additive; `bypass_intercept` stays.** Seven
call sites outside `builtin/` depend on `bypass_intercept`, including
`tests/folding_stage2_acceptance.rs:1296-1315`, which pins that a bypass
edit still triggers the Q#FD19 interactive unfold. Redefining the
existing key would silently change that pinned seam. `generated = true`
implies bypass; passing both is legal and `generated` wins (it is
strictly stronger); passing `generated` on a buffer with no intercept is
legal (Class C would use it if it ever adopts).

**Q#GB3 — A generated write gets its OWN `run_buffer_edit` arm. Reversed
in revision 3 (review P1-1).**

Revision 2 said "a generated write goes through `run_buffer_edit`'s
bypass arm". **That is unimplementable** — the bypass arm is
`run_bypass_edit`, which calls `begin_edit`, which calls
`ensure_writable` first (§3.4). `run_buffer_edit`
(`src/lua_bindings/mod.rs:1353-1374`) grows a third arm:

```rust
if generated {
    unfold_before_interactive_lua_edit(lua, id, edit_start_of(&op));
    run_generated_edit(lua, id, op)          // no begin_edit; §3.4
} else if bypass_intercept {
    unfold_before_interactive_lua_edit(lua, id, edit_start_of(&op));
    run_bypass_edit(lua, id, op)
} else {
    run_managed_edit(lua, id, op)
}
```

**What revision 2 got right and revision 3 keeps: the unfold seam.** The
generated arm still calls `unfold_before_interactive_lua_edit` at the
same point the bypass arm does, and for the same reason — the guard
already requires `InteractiveCommandOrigin::current()` to be `Some`
(`src/lua_bindings/mod.rs:1424-1429`), which is false for the
`process.after-tick` pump and true for `M-x compile`, and the op is
applied verbatim so the site is known up front (the round-5 F1
distinction the comment at `:1359-1367` records). Keeping it is the
no-change option.

**But revision 2's stated *reason* for keeping it is now known to be
half-false, and Q#GB16 is the consequence.** Revision 2 wrote "folding a
`*compilation*` buffer is possible, so changing this would be a silent
behaviour change to a pinned seam". Folding a `*compilation*` buffer is
possible **only until this arc locks it**: `pmacs.fold.fold` refuses
every `read_only` buffer at `src/lua_bindings/fold.rs:313`. So the arc
preserves the unfold-on-edit seam while silently killing the
fold-creation seam that feeds it. The unfold arm stays because it costs
nothing and because `FoldRegistry::unfold_containing` is registry-side
and unaffected by the lock; the *reason* is corrected here so a reviewer
does not inherit revision 2's version of it.

**Q#GB4 — History cleared per op, with a measurement obligation.** §3.3.
Deferred optimization: suppress recording instead of clearing.

**Q#GB5 — The lock-at-creation gap, and who closes it.** A
`{ generated = true }` write locks the buffer *after* its first call, so
between `pmacs.buffer.create(name)` and the owner's first generated write
the rope is writable. `dired.lua` (`claim_handle` → `paint`),
`listview.lua` (`ensure_panel` → `render`) and the search panel
(`ensure_search_panel` → the header write in `pmacs.project.search`) all
write synchronously in the same call, so the window is not observable.
**`compile.lua`'s `ensure_slot` (`:258-282`) does not** — it creates
`*compilation*` and returns, leaving it empty and writable until
`start_run`. Recommendation: `ensure_slot` ends with
`pmacs.buffer.set_generated_contents(slot.buf, "")`, using the shipped
primitive; no third surface is needed.

**Amended in revision 2 (review P1-2), and the amendment is a hard
ordering constraint, not a caveat.** `ensure_slot` is
`buffer_named(name) or create` (`compile.lua:263`), and
`pmacs.compile.run` calls it **before** `start_run` validates
`opts.display` (`:1090` vs `:752-757`). §2.8 measures that a
`display = "bogus"` call today raises *and still leaves a foreign
`*compilation*` permanently un-editable*; with the empty write placed at
the end of `ensure_slot` that same failing call would **empty the buffer
and lock the rope**, unrecoverably. So the lock may only be installed
once **Q#GB13's ownership rule guarantees `slot.buf` is a buffer compile
created**. With ownership in place the buffer is provably fresh and the
placement in `ensure_slot` is correct; without it, no placement is.

**Revision 4: this recommendation is also the concrete caller that made
P1-1 direction A load-bearing, and the dependency runs both ways.**
`set_generated_contents(slot.buf, "")` on a buffer that is already empty
is a **semantic no-op** — it takes `src/buffer.rs:1245-1253`'s early
return. Under revision 3's `revision`-keyed cleanup that call would have
locked the slot while leaving any pre-existing history intact and the
buffer marked modified, which is the opposite of what "lock it at
creation" is for. §3.4's `NoOp` variant clears unconditionally, so the
recommendation is sound again. Stated here rather than only in §3.4
because a reader reaching Q#GB5 first should not have to derive it: **the
correctness of this placement depends on two decisions in other
sections** — Q#GB13's freshness guarantee and §3.4's `NoOp` cleanup —
and criterion 16b is what pins the second.

**Q#GB6 — Clamp each window coordinate against its OWN post-edit bound.**
§2.6 measures a shipped defect: a shrinking generated write leaves
`win.cursor` past the end of the rope, and neither paint nor `C-p`
recovers it. Recommendation: clamp in `EditorCore::notify_buffer_edit`
— a **clamp**, not a call to `rebuild_views_for`, because a rebuild is
O(buffer length) and would run per streaming op.

**Revised in revision 2 (review P2-4). Revision 1 said "clamp when the
buffer shrank", which conflates two different extents.** The two
coordinates are bounded by different things:

- **`win.cursor` is a byte position** (`src/window.rs:366-367`, "Byte
  position of this window's cursor"), bounded by `Buffer::len()`.
- **`win.view_top` is a line index** (`src/window.rs:373-374`, "First
  buffer **line** shown at the top of this window's viewport"), bounded
  by `TextView::line_count()` (`src/text_view.rs:67`).

A replacement can **grow in bytes while collapsing many lines into one**
— `"a\nb\nc\nd\ne\nf\n"` (12 bytes, 7 lines) replaced by a single
80-byte line — leaving `view_top` invalid on a write that a byte-length
comparison calls a *growth*. So the trigger cannot be "the buffer
shrank": the clamp runs **unconditionally**, each coordinate against its
own bound, exactly as `rebuild_views_for` already does
(`src/editor_core.rs:1853-1857`, which clamps `cursor` against `len` and
`view_top` against `line_count().saturating_sub(1)`).

**Argued from the types and from `rebuild_views_for`'s existing shape,
not measured** — unlike §2.6's cursor case, the `view_top` case needs a
scrolled window to stage and was not staged. §6 Stage 1 criterion 8b is
what turns the argument into a pin.

Recommended for **Stage 1**, because Stage 1's adopters refresh shrinking
panels constantly and because it fixes terminal copy mode retroactively.
Alternative if the user prefers a narrower Stage 1: its own lane, in
which case Stage 1 must say so out loud rather than inherit it silently.

**Q#GB7 — The unlock survives ONLY bounded by lock provenance, and it
moves to Stage 2. Revision 3 withdraws revision 2's recommendation
(review P1-3).**

Revision 2 recommended `pmacs.buffer.unlock_generated(buf)` — a one-way
clear of `read_only`, shipped in Stage 1 — on the strength of sweep B's
finding that the two halves of the protection are asymmetric: the
intercept half is removable (`remove_intercept`,
`src/lua_bindings/mod.rs:3433`, used by the REPL at
`repl/init.lua:325-327`) while the rope half is one-way from Lua. That
observation stands. **The capability revision 2 derived from it does
not**, on two independent counts, both of which the review is right
about.

**First: an unbounded clear of `read_only` is not a generated-buffer
capability at all.** §2.11 measures that the flag serves three unrelated
policies. `unlock_generated` as revision 2 wrote it — "clears `read_only`
and nothing else" — would let any Lua caller disable a **live terminal
identity buffer**'s protection (`src/terminal/session.rs:305`), which
this arc never locked, whose owner set it to refuse host edits, undo,
redo **and remote CRDT imports** alike, and which is Lua-reachable
(`pmacs.terminal.open` returns its id;
`pmacs.terminal.is_terminal(buf)` exists at
`src/lua_bindings/mod.rs:8858-8867`). That is a strictly larger capability
than the one the arc adds, granted by accident.

**Second: it does not achieve its own stated purpose.** Revision 2 sold
it as the escape from an accidentally bricked `*scratch*`. It is not:
by the time anyone reaches for it, `set_generated_contents` has already
replaced the contents **and cleared the history** (§3.4). Unlocking
returns writability to a buffer whose data is gone. The recovery
scenario that justified moving this from a deferral to Stage 1 work was
never a recovery.

**The decision.** The capability survives, bounded by **lock provenance**
(Q#GB15), and:

- **`pmacs.buffer.unlock_generated(buf)` refuses any buffer whose lock
  this arc's primitive did not install** — terminal identity buffers, and
  anything a future Rust owner locks for its own reasons. It is not a
  clear of `read_only`; it is the *inverse of `apply_generated_edit`'s
  lock*, and it can undo only what the same public API did.
- **It moves to Stage 2**, with Q#GB15, because it is meaningless without
  the provenance the same field provides and because Stage 2 is where the
  lock capability actually widens.
- **Its claim narrows.** It is the **closure of the capability
  `{ generated = true }` adds** — not a recovery mechanism. The honest
  statement of what it buys: after a mistaken generated write, the buffer
  becomes writable again; its former contents do not come back.

**On the standing asymmetry the review asks this to address directly**
(`remove_intercept` is exposed; `set_read_only` deliberately is not,
`src/lua_bindings/mod.rs:3072-3078` and `docs/agent-handoff.md` §4). A
provenance-bounded `unlock_generated` **does not breach that policy**,
and the reason is precise rather than rhetorical: it can only reach a
lock that a public Lua call installed, so it adds **no reachable state
that `{ generated = true }` did not already make reachable**. A
`set_read_only(buf, true)` would add the "lock with no door" state the
invariant exists to forbid; a `set_read_only(buf, false)` would reach
locks Lua never set. This reaches neither. If that argument does not
persuade, the fallback is to ship no unlock at all and let dired Stage 3
frame its own door — which costs this arc nothing, because Stage 3 is
not built.

**What Stage 1 loses, and why that is correct.** Nothing. Stage 1 adopts
`set_generated_contents`, which is **already public on `main`**, on two
more buffers. It therefore adds no brick capability the tree does not
already ship, and revision 2's argument that Stage 1 needed an escape
hatch applied equally to `main` — which is a sign the argument was about
the shipped primitive, not about Stage 1. The `*scratch*` exposure sweep
B found is real, and it is real **today**; it is recorded in §8 as a
pre-existing hazard this arc neither creates nor closes.

**Q#GB15 — `read_only` gains a provenance companion. New in revision 3
(review P1-3, sweep C).**

**Revision 4 replaces revision 3's `generated_lock` with an intrinsic
`identity_protected`, because review round 3's P1-3 showed provenance
cannot be inferred from the flag's history.**

Revision 3 proposed `generated_lock: bool`, meaning "this buffer's
`read_only` was set by a generated write", maintained by a third rule:
*every* `set_read_only` call clears it. That rule breaks a seam this very
document prescribes. `tests/terminal_copy_mode_acceptance.rs:578-584`
does exactly this cycle, with a comment explaining why:

```rust
// `read_only` refuses the upgrade's own bookkeeping path the same
// way it refuses everything else, so lift it around the upgrade.
buffer.set_read_only(false);
buffer.upgrade_to_crdt(2).expect("upgrade");
buffer.set_read_only(true);
```

Under revision 3's rule the final `true` yields
`read_only && !generated_lock`, so the **next owner refresh** of that
snapshot is refused as someone else's lock. Q#GB12 and Stage 2
criterion 4 prescribe the same lift-and-restore idiom, so revision 3
invalidated its own test strategy. The defect is not the rule's details;
it is that a *derived* fact has to be maintained correctly by every
mutation of the thing it is derived from, and `set_read_only` is `pub`
with callers this document does not control.

**The fix is to stop deriving it.** `Buffer` gains one private field that
is a property of **what the buffer is**, not of who last locked it:

```rust
/// Whether this buffer's read-only state is an intrinsic identity
/// protection rather than an ordinary lock. Set once, at construction,
/// by an owner that means "the host may not edit this at all"; never
/// derived from `read_only` and never changed by `set_read_only`.
identity_protected: bool,
```

Two rules maintain it, and `set_read_only` is not one of them:

1. `Buffer::set_identity_protected(true)` — a new `pub` method, called
   **once** by `TerminalSession::open` beside its existing
   `set_read_only(true)` (`src/terminal/session.rs:305`). That is the
   only production caller.
2. `apply_generated_edit` refuses iff `identity_protected` (§3.4 exit 2);
   `unlock_generated` refuses iff `identity_protected`. Neither ever
   writes the field.

**The lift-and-restore seam is now unaffected**, because
`identity_protected` is `false` for a snapshot buffer and stays `false`
through any number of `set_read_only` cycles. Q#GB12's prescription and
Stage 2 criterion 4 need no change, which is the test that the rule is
right.

**Why declaration beats inference, stated as a rule rather than as a
patch.** Inference required every mutation of `read_only` to maintain a
derived fact; P1-3 is the proof that it does not. Declaration puts the
burden on the locker: *if you lock a buffer and Lua must not unlock it,
mark it identity-protected.* One flag, one meaning, one writer, and a
future Rust owner that wants the same protection opts in with one line
instead of relying on an inference chain staying intact.

**What this costs relative to revision 3.** It is strictly narrower in
one respect and that is worth naming: `unlock_generated` can now release
**any** non-identity-protected `read_only` buffer, including one a future
Rust owner locked without declaring itself. Revision 3's version would
have refused that by inference. The trade is deliberate — an inference
that is wrong on a shipped seam is worse than a contract that has to be
opted into — and the contract is documented on `set_read_only` so the
next locker reads it at the point of use.

**Rule 1's refusal is the half revision 2 did not have, and it closes a
hole in the SHIPPED primitive** (sweep C item 1).
`Buffer::set_generated_contents` today does `self.read_only = false`
unconditionally (`src/buffer.rs:546`), so
`pmacs.buffer.set_generated_contents(term_buf, "junk")` on a live
terminal identity buffer overwrites its contents and re-locks it as
though the primitive owned it. Nothing in the tree refuses that, and no
test covers it. It is why the field earns its cost in **both**
directions rather than existing only to make Q#GB7 safe.

**The alternatives, and why not.**

- *A registry-side set of generated-locked ids, held as Lua app-data.* A
  second source of truth that can drift from the flag, plus a pruning
  obligation on buffer removal — the shape the terminal-config lane
  records as "`prune` **reacts** to buffer removal". A field on the
  buffer cannot drift from the buffer.
- *Replacing `read_only: bool` with an enum.* Cleaner in principle,
  and it would let §2.11's third policy (`document_bytes`) ask the
  question it actually means. It also churns `ensure_writable`, all six
  gated paths, `is_read_only`'s seven callers and the public
  `set_read_only` signature — a refactor this arc would be smuggling.
  Named as the right eventual shape in §8, not adopted.

**Cost, stated:** one bool per buffer; one new `pub` method with exactly
one production caller; **no invariant to maintain**, because the field is
never derived from `read_only`; and one new refusal that changes shipped
`set_generated_contents` behaviour, so it lands in Stage 2 with the rest
of Q#GB10's changes to that function, not in Stage 1.

**Q#GB16 — The lock silently disables fold creation on every buffer it
touches. New in revision 3 (sweep C item 2).**

`document_bytes` (`src/lua_bindings/fold.rs:310-318`) is Q#FD11's
"normal document buffer" guard and it is spelled `if buffer.is_read_only()
{ return Ok(None); }`. Its two consumers are `pmacs.fold.fold`
(`:67-70`, which then sets the status `fold rejected: not a document
buffer` and returns `false`) and `pmacs.fold.unfold`'s
normalize-an-arbitrary-range fallback (`:108`).
`tests/folding_acceptance.rs:570-590`
(`read_only_buffer_is_rejected`) pins the behaviour, and its comment
records the intent the guard was written with: "terminals are read-only,
so a read-only buffer is not foldable."

So the moment Stage 1 locks dired listings and listview panels, and
Stage 2 locks `*compilation*`, `*shell-command*` and `*search-results*`,
**`pmacs.fold.fold` starts answering `false` on all five families** —
with a status message that is now false ("not a document buffer"), on a
seam nothing in this arc's acceptance would notice, and against a guard
whose author meant "terminal", not "generated".

**Recommendation: name it, do not silently accept it, and do not fix it
here.** Three options, with the recommendation being (a):

- **(a) Accept, and pin the acceptance.** A generated buffer arguably
  *should not* be foldable — its contents are replaced wholesale and any
  stored fold range is invalidated on every refresh anyway. Then the
  change is intended, and Stage 1 owes **an explicit criterion asserting
  it**, plus the status string corrected to say `read-only`, not "not a
  document buffer". Cheap, honest, and it converts a silent behaviour
  change into a stated one.
- **(b) Preserve foldability** by changing the guard to
  `read_only && !identity_protected`. Available once Q#GB15 lands, but it
  edits a pinned Q#FD11 seam for a use case nobody has asked for.
- **(c) Do nothing and say nothing.** Rejected: this is exactly the
  defect class the review's findings 1 and 3 are instances of.

Under (a) the only code change is the status string; the substance is
the criterion. Stage 1, because Stage 1 is where the first three
families get locked.

**Q#GB17 — The transaction shape.** §3.4. One `&mut Buffer` method, its
own `run_buffer_edit` arm, `begin_edit` untouched, eight named exits, and
and cleanup driven by an explicit `GeneratedOutcome` — **not** by
inferring one from `revision`, which revision 3 did and which was wrong
in three directions (§3.4). New in revision 3 (review
P1-1).

**Q#GB18 — Route the two broken identity consumers by owned `BufferId`.
New in revision 3 (review P1-2).**

§2.10's census finds exactly two sites that a disambiguated name breaks,
with six downstream consumers between them. Q#GB13 removes the
*adoption*; this removes the *recognition* that adoption was hiding.

**`listview.lua` (Stage 1, with Q#GB13's ownership fix).** `panels`
becomes a list of records rather than a name-keyed map, with two
lookups instead of one:

- `panel_for_requested_name(name)` — matches `p.requested_name`, the
  `spec.name` the caller asked for. Stable across disambiguation, so
  repeated `listview.open{ name = "*references*" }` finds the same panel.
- `panel_for_buffer(buf)` — scans `p.buffer == buf`. This is what
  `listview.visit`, `listview.refresh`, `listview.quit` and
  `listview.open`'s capture guard use.

The list-and-scan shape is deliberate and is dired's, for the reason
`dired.lua:123-127` already records: two `BufferIdLua` values for the same
buffer are **distinct userdata**, so `panels[buf]` would miss even for the
same buffer;
`handle_for_buffer` (`dired.lua:142-148`) compares with `==` instead, and
`compile.lua`'s `slot_for_buffer` (`:200-206`) does the same. Three
existing implementations of one shape; listview adopts it rather than
inventing a fourth.

**And listview needs dired's *other* half too, which is easy to miss.**
`grep -n on_removed builtin/runtime/listview.lua` returns **nothing** —
listview registers no buffer-removal callback, unlike compile
(`:277-279`) and the search panel (`default.lua:876-883`). Today that is
harmless because `ensure_panel` re-checks `p.buffer:is_valid()` on every
`open` (`:93-94`) and a name-keyed map holds at most one entry per name.
A **list** does not self-limit: kill and reopen `*references*` ten times
and a naive list holds ten records, nine of them dead, and
`panel_for_buffer`'s scan walks all of them. So the list must compact on
scan, exactly as `dired.lua:132-140`'s `live_handles()` does. Naming it
here because "swap a map for a list" reads like a one-line change and is
not.

**`compile.lua` and the search panel (Stage 2, with Q#GB13's ownership
fix). Rewritten in revision 4 (review round 3, P2-4).**

`is_generated_buffer` stops comparing names. Its two owners are in
different files (`compile.lua`'s `slots`, `default.lua`'s
`search_panel`), so the predicate needs a seam — and **revision 3 chose
the wrong one.**

Revision 3 had `ensure_search_panel` call
`pmacs.compile._register_generated_buffer(p.buf)`. That converts an
existing *guarded, optional* dependency into a hard one.
`default.lua:991-994` currently reads:

```lua
if cur
  and not (pmacs.compile
    and pmacs.compile.is_generated_buffer
    and pmacs.compile.is_generated_buffer(cur))
then
```

— a triple check, and it exists for a reason. `default.lua` is loaded by
`LuaHost::attach_editor` (`src/lua.rs:250-251`), while `compile.lua` is
loaded much later in `EditorState::new`'s runtime sequence
(`src/editor.rs:704`). `LuaHost` is `pub` and **nine existing test files
build one directly**, so a harness that has `pmacs.project.search` and no
`pmacs.compile` is not hypothetical — it is reachable with the pattern
those nine already use. A hard call to `pmacs.compile._register_…` inside
`ensure_search_panel` raises there.

**The decision: symmetric guarded optionality, not a shared registry.**
Each module answers for **its own** buffers, and each capture site ORs
the two through the guard shape that already exists:

- `compile.lua` — `is_generated_buffer(buf)` becomes
  `slot_for_buffer(buf) ~= nil`, which is already id-based (`:200-206`).
  It answers for `*compilation*` and `*shell-command*` **only**, which is
  the truthful scope and matches §1.4's warning not to read `ensure_slot`
  as covering the grep panel.
- `default.lua` — a local `search_panel_owns(buf)`, i.e.
  `search_panel ~= nil and search_panel.buf == buf`, exposed as
  `pmacs.project._is_search_panel` for the other direction.
- Each capture site checks its own predicate directly and the other
  module's **through the existing optional guard**:
  - `default.lua:991-994` keeps its triple check verbatim and gains
    `and not search_panel_owns(cur)`;
  - `compile.lua:762` gains the mirror-image guard,
    `and not (pmacs.project and pmacs.project._is_search_panel and
    pmacs.project._is_search_panel(cur))`.

**Three things this buys over revision 3's registry**, all of them
consequences of nobody owning a list of somebody else's buffers:

1. **No load-order constraint in either direction**, and no new one to
   document beside the three `src/editor.rs` already carries.
2. **No teardown obligation.** Revision 3's registry needed
   `on_removed`-driven unregistration, and a registry that only grows is
   the defect the terminal-config lane records as "`prune` **reacts** to
   buffer removal". Each owner's own table already tracks its own
   liveness (`slot_for_buffer` scans `slots`, `search_panel_owns` reads
   one field), so there is nothing to prune.
3. **Each predicate is answerable by the module that knows the answer**,
   so neither can go stale relative to the other.

**Cost:** the guard shape is written twice rather than once. That is the
existing pattern, and duplicating a four-line guard is cheaper than a
cross-module registry with a lifetime.

**This needs its own pin, because no ordinary test reaches it** — every
`EditorState::new` loads both modules. §6 Stage 2 criterion 21 builds a
`LuaHost`, calls `attach_editor`, and drives search with no
`pmacs.compile` present.

Alternative considered and rejected: leave `is_generated_buffer`
name-based and simply never disambiguate compile's buffers. Rejected
because Q#GB13's whole argument is that a foreign `*compilation*` must
not be adopted, and refusing to adopt without disambiguating means
raising — which turns a name collision into a failed `M-x compile`.

**Q#GB8 — The REPL is out of this arc.** §2.5. Same root cause, different
remedy, its own lane. Its measured exposure is recorded above so the next
scout starts from evidence.

**Q#GB9 — Class C is out of this arc.** `*buffer-list*`, `*help*` and
`*workers*` are generated but were never claimed to be protected, so
nothing about them is *defeated*. Making them immutable is a product
decision about `COHERENCE.md` §14's list and output-channel primitives,
not a bug fix, and it should not ride a bug-fix arc. `*workers*`
additionally writes from Rust with its own fan-out pair and already
`mark_clean`s, so it is not a like-for-like conversion.

**Q#GB10 — Refuse a generated write on a path-backed buffer; then, and
only then, mark clean.** Rewritten in revision 2 (review P1-3).

`set_generated_contents` leaves `is_modified = true` (measured), so every
adopter shows `*` in the mode line (`src/editor.rs:3704`) and in
`*buffer-list*` (`default.lua:395`). `workers_buffer::render` calls
`Buffer::mark_clean()` (`src/workers_buffer.rs:95`) and
`instance_buffer.rs:401` asserts the same for its own rendered buffer,
so marking clean is the established convention for a generated buffer.

**Revision 1's justification was wrong.** It said the flag "drives only
the mode-line indicator and the buffer-list column". §2.9 measures two
more consumers: `src/autosave.rs:363`, the skip that decides whether a
crash-recovery slot is written, and `src/desktop.rs:302`. Since
`{ generated = true }` is public Lua on any buffer id, a caller could
replace a **file-backed** buffer's contents, mark it clean, and suppress
autosave recovery for it.

**The rule, stated explicitly rather than left implicit:
`Buffer::apply_generated_edit` (and therefore `set_generated_contents`)
returns an error for a buffer whose `file_path()` is `Some`.** Then
`mark_clean` is unconditionally safe, because **both** consumers gate on
`file_path()` before they read the flag (`autosave.rs:359-364`,
`desktop.rs:298-303`).

Why refuse rather than the alternative "retain modified state for
path-backed buffers": the flag rule fixes only the flag. A generated
write on a file buffer would still **replace its contents and lock its
rope**, and §1.3 measured that Lua cannot unlock. Refusing bounds all
three harms with one rule, and it is the narrower capability.

**Verified non-breaking.** None of the six generated families is
path-backed: they are all `pmacs.buffer.create`d, and no builtin Lua sets
a buffer path — `grep -rn "set_path\|set_buffer_path" builtin` finds no
call sites (only a comment in `dired.lua:42` and `lsp.lua`'s own
`active_buffer_path` local). Path binding happens Rust-side in
`from_file` / `find_file` only.

**This changes shipped `set_generated_contents` behaviour** — both the
new refusal and `mark_clean` — and therefore the terminal snapshot, so it
belongs in Stage 2 alongside the reimplementation, not smuggled into
Stage 1.

**Q#GB11 — Staging.** §5.

**Q#GB12 — The revision guard becomes near-dead, and three tests break.**
After Stage 2, an external edit to `*compilation*` or `*search-results*`
is refused at the rope, so the Q#CM2 desync machinery (`check_rev`
`compile.lua:332`, `resync` `:309`, `search_panel_check_rev`
`default.lua:832`) can essentially no longer fire. Recommendation: **keep
it** — it is cheap, and a future Rust-side writer could still mutate the
buffer — but say so, and do not delete its tests. Three compile
acceptance tests inject intruder edits through `bypass_intercept`
(`tests/compile_mode_acceptance.rs:1040`, `:1106`, `:1305`) and **will be
refused** after conversion; they must lift `read_only` Rust-side first,
exactly as `tests/terminal_copy_mode_acceptance.rs:582-584` already does.
That is a concrete, verified integration cost of Stage 2, not a surprise
to discover during implementation.

**Revision 4: the lift-and-restore idiom this prescribes was briefly
invalidated by revision 3's own Q#GB15, and is now safe again.** Revision
3's `generated_lock` was cleared by every `set_read_only` call, so the
restoring `set_read_only(true)` would have left the buffer looking like
someone else's lock and the next owner refresh would have been refused
(review round 3, P1-3). Q#GB15's `identity_protected` is never written by
`set_read_only`, so the cycle is transparent and **this prescription,
Stage 2 criterion 4 and `acc16e` all need no change**. The seam is
`crdt`-gated, so §10 now names
`--test terminal_copy_mode_acceptance --features crdt` explicitly: a
default-feature sweep never compiles `acc16e` and proves nothing about
it.

**Q#GB13 — Ownership by handle is a prerequisite, not a follow-up.** New
in revision 2 (review P1-2). `listview.ensure_panel` (`listview.lua:95`),
`compile.ensure_slot` (`compile.lua:263`) and `ensure_search_panel`
(`default.lua:861-868`) adopt any buffer that shares their name. §2.8
measures the consequence today (a clobbered, permanently un-editable user
buffer — and, for compile, from a call that *raised*), and measures that
`M-x buffer.undo` is currently the **only** recovery. Locking the rope
removes that recovery, so the rule must land in the same stage as the
lock.

Recommendation: adopt the rule the tree already states at
`terminal.lua:300-305` and implements at `dired.lua:476-504` —
**ownership means "this buffer is in my handle table"**, a name collision
disambiguates `<2>`…`<99>`, and exhausting the limit raises rather than
adopting. Three writers, one shape, each in the stage that locks it:
listview in Stage 1, compile and search in Stage 2. Dired and terminal
already comply.

Alternative considered and rejected: a standalone Stage 0 that fixes all
three at once. Rejected because each writer's ownership fix is only
load-bearing for the stage that locks that writer, and a lone ownership
PR reads as unmotivated churn without the lock that makes it urgent. If
the user prefers the standalone shape, the acceptance criteria in §6 move
with it unchanged.

**Revision 3: Q#GB13 is only half the work, and revision 2 shipped the
half that is visible.** Disambiguating a name is not free — it changes
what a buffer is *called*, and five sites in `builtin/` recover identity
from what a buffer is called. Two of them break (§2.10 Class 1) and are
Q#GB18's. Stated as an ordering constraint so it cannot be split across
PRs: **within each stage, Q#GB18's routing change must land in the same
PR as Q#GB13's disambiguation for the same writer.** Disambiguate first
and you ship a listview panel whose `RET`, `g` and `q` are dead and whose
`q`-target capture is inverted; route first and you have written a
lookup nothing yet exercises.

**Q#GB14 — The lock is not observable from Lua, and the pins depend on
it.** New in revision 2, out of P1-1's fix. `describe.buffer` returns
`name`, `length`, `modified`, `view_count` and nothing else
(`buffer_info_table`, `src/lua_bindings/mod.rs:6352-6364`), so no Lua
assertion can read `read_only` directly. Two discriminators are
available and both are used in §6: a **`bypass_intercept` write**, which
lands on `main` and raises `` buffer `X` (id BufferId(n)) is read-only ``
once the rope is locked (measured, §2.4), and **Rust-side
`Buffer::is_read_only()`** (`src/buffer.rs:494`, already `pub`).
Recommendation: use both, and do **not** add a Lua surface for it — the
acceptance suites are Rust and need no new public API. Optional and
separable: adding `read_only` to `buffer_info_table` would be a
read-only introspection field with no new capability, useful if
Lua-level pins are ever wanted; it is not required by this arc.

---

## 5. Staging

**The proposed cut is endorsed, with two amendments.** The argument for
it is not the obvious one.

### Stage 1 — `generated-buffer-immutability-stage1`

`dired.lua` and `listview.lua` adopt `pmacs.buffer.set_generated_contents`.

- **Prerequisite, in this PR, before the lock (Q#GB13):**
  `listview.ensure_panel` (`listview.lua:95`) stops adopting a
  same-named foreign buffer. Ownership is the handle table (`panels`);
  a name collision disambiguates `<2>`…`<99>` and raises at the limit,
  matching `dired.lua:486-504`. **`dired.lua` needs no ownership work**
  — it already complies, which is why it is the cheaper of the two
  adopters.
- **In the SAME PR as that disambiguation (Q#GB18):** `panels` becomes a
  compacting list; `panel_for_current_buffer` is replaced by
  `panel_for_buffer(buf)` scanning `p.buffer == buf`, and
  `ensure_panel` looks up by `p.requested_name`. Four consumers move
  with it, including `listview.open`'s capture guard (`:118-123`).
  Ordering constraint, not a preference — see Q#GB13.
- `listview.lua:50-62` — `render`'s delete-all + insert-all becomes one
  `set_generated_contents(buf, body)`.
- `dired.lua:369-372` — `paint`'s whole-buffer replace becomes one
  `set_generated_contents(handle.buf, text)`.
- Both keep their erroring intercept (named error, per the layering at
  `terminal.lua:351-366`) and both keep `set_round_trip_input`.
- Plus Q#GB6's per-coordinate clamp, if approved.
- Plus **Q#GB16's fold decision** — under recommendation (a), the
  `fold.rs:68` status string, and the criterion that makes the change
  stated rather than silent.

**Revision 3 removes `unlock_generated` from Stage 1** (Q#GB7). Revision
2 put it here as "the escape from a bricked buffer"; the escape does not
recover anything (the history is already cleared), and the brick it
escapes is one `main` already ships, since `set_generated_contents` is
already public. Stage 1 adds no lock capability that does not already
exist, so it needs no door. The capability re-appears in Stage 2, bounded
by Q#GB15's provenance.

**Revision 2 grew Stage 1 by two prerequisites and one reversal.** The
ownership rule is load-bearing for the lock rather than adjacent to it,
and it stays. The unlock does not, per the paragraph above. Stage 1
is still **not** a pure-Lua change — Q#GB6's clamp is Rust, and under
Q#GB16(a) so is a status string — so revision 1's "pure Lua" claim stays
withdrawn.

**Why this cut, and why Stage 1 is not merely "the cheap half":** it is
the *worse-exposure* half. `compile.lua:219` and
`builtin/commands/default.lua:855` rebind all seven undo chords to
`compile.undo-noop`; `dired.lua` and `listview.lua` rebind **nothing** —
`grep -n 'C-/\|C-_\|C-x u\|undo' builtin/runtime/dired.lua builtin/runtime/listview.lua`
returns zero binding lines. Measured, a bare `C-/` empties a listview
panel and a dired listing. Stage 1 closes the only two families
reachable without `M-x`.

**Why the cut is safe under every candidate primitive:** a whole-buffer
replace is expressible in all of A–C, and under the recommendation
`set_generated_contents` keeps its name and signature as
`apply_generated_edit`'s wrapper. Stage 1 is therefore not rework under
any Q#GB1 outcome — which is the decisive argument for cutting here
rather than shipping one large PR.

**The honest objection, and the answer.** A reviewer could call Stage 1
churn: two call sites converted to a primitive Stage 2 then rewrites.
Stage 2 rewrites the primitive's *implementation*, not its callers; the
diff at `dired.lua:371` and `listview.lua:60-61` is written once.

### Stage 2 — `generated-buffer-immutability-stage2`

- **Prerequisite, in this PR, before the lock (Q#GB13):**
  `compile.ensure_slot` (`compile.lua:263`) and `ensure_search_panel`
  (`default.lua:861-868`) stop adopting same-named foreign buffers, same
  shape as Stage 1's listview fix.
- **In the SAME PR as that disambiguation (Q#GB18):**
  `pmacs.compile.is_generated_buffer` (`compile.lua:212-217`) stops
  comparing names and scans an owner-registered id list, with
  registration in `ensure_slot` and `ensure_search_panel` and
  unregistration in the `on_removed` callbacks both already have.
- `Buffer::apply_generated_edit` (§3.4) + the `{ generated = true }`
  option + its own `run_buffer_edit` arm + `set_generated_contents`
  reimplemented over it (Q#GB17, Q#GB3).
- Q#GB10's path-backed refusal **and** `mark_clean` — one rule, both
  halves, since the refusal is what makes the flag change safe.
- **Q#GB15's `identity_protected` field**, its write-direction refusal, and
  `pmacs.buffer.unlock_generated` bounded by it (Q#GB7). All three edit
  `set_generated_contents` or the flag it sets, so they belong with the
  reimplementation.
- Conversion of all 13 remaining write sites (`compile.lua` 9,
  `builtin/commands/default.lua` 4).
- Q#GB5's `ensure_slot` lock, which is only placeable once ownership
  lands.
- The three `compile_mode_acceptance` intruder tests updated per Q#GB12.

All the new Rust and all the review risk in one PR, which is the point of
the cut.

**Amendments to the briefed cut:**

1. **Q#GB13 (ownership) is a prerequisite of the stage that locks each
   writer**, not a follow-up and not a separate PR. §2.8 is the
   argument: this arc removes the only recovery a clobbered buffer
   currently has.
2. **Q#GB18 (identity routing) rides in the same PR as Q#GB13 for the
   same writer**, per the ordering constraint under Q#GB13. New in
   revision 3.
3. **Q#GB7 (unlock) moves to Stage 2, bounded by Q#GB15.** Revision 1
   deferred it; revision 2 built it in Stage 1 on an argument review
   P1-3 falsified; revision 3 lands it in Stage 2 with the provenance
   that makes it a bounded capability rather than a general one. The
   two reversals are recorded rather than smoothed over because the
   *reason* moved twice and the next reader needs to know which reason
   is live.
4. **Q#GB10 (path refusal + `mark_clean`) lands in Stage 2**, because it
   edits `set_generated_contents` itself and therefore changes the
   already-shipped terminal snapshot. **Stage 1 is not pure Lua**
   (Q#GB6's clamp, and Q#GB16's status string), which revision 1
   claimed and revision 2 withdrew; the withdrawal stands for a
   different reason than revision 2 gave.

**Where the REPL lands: neither stage.** Q#GB8.

---

## 6. Acceptance, with the pre-image each criterion must fail against

`M-x buffer.undo` is the user-reachable trigger and **needs no keymap**.
A criterion that only exercises the intercept, or only the chords, proves
nothing — that is precisely what `compile.lua`'s idiom already achieves
and what this bug already defeats.

**Revision 2 re-audited every criterion, not only the three the review
named (sweep A).** Each now carries an explicit pre-image class, because
an unlabelled always-green criterion is indistinguishable from a vacuous
one:

| class | meaning |
|---|---|
| **`main`** | fails on `ad41cf1`. A regression pin in the ordinary sense. |
| **fix-shape** | **passes on `main` by design**; fails against a specific *wrong implementation*, named in the criterion. Legitimate per `docs/agent-handoff.md` §5 ("bite against every pre-image the fix could plausibly have taken"), where `acc 6` deliberately passes on `main`. |
| **mutation** | passes on `main`; fails against a named one-line mutation of the fix. |
| **structural** | no behavioural pre-image. Rides **alongside** the others, never instead — a structural comparison of two authorities does not catch a misrouted consumer. |

**Q#GB14: the lock is not observable from Lua.** `describe.buffer`
carries no `read_only` field, so every "is it locked" assertion below
uses a **`bypass_intercept` write** (lands on `main`, raises
`` buffer `X` (id BufferId(n)) is read-only `` once locked) or Rust-side
`Buffer::is_read_only()`. An *ordinary* edit is not a discriminator: the
intercept refuses it either way.

### Stage 1

1. **[`main`] `C-/` cannot empty a listview panel.** Driven by
   `dispatch_key`, not a Lua call. *Bite:* measured — `"H\nrow-one\nrow-two"`
   → `""`.
2. **[`main`] `M-x buffer.undo` cannot empty a listview panel**, driven
   through the real minibuffer (`M-x`, type `buffer.undo`, RET), not
   `pmacs.command.invoke`. *Bite:* same empty result; and a chord-only
   fix passes 1 and fails this.
3. **[`main`] `C-/` and `M-x buffer.undo` cannot empty a dired listing.**
   *Bite:* measured — one undo takes the listing to `""`.
4. **[fix-shape] The owner's own refresh still works after the lock** —
   `g` on a listview panel and on a dired buffer renders *new* content.
   *Bite:* a naive `set_read_only(true)` at creation passes 1–3 and fails
   here; that is the failure mode `src/buffer.rs:521-524` exists to
   prevent. Assert the new content appears, not that the call did not
   raise.
5. **[fix-shape] An ordinary edit is refused by the INTERCEPT, not by
   the rope** — assert on the message text, which distinguishes them.
   Measured, both forms: the intercept produces
   `intercept rejected the edit: ... listview.lua:102: *probe-panel* is read-only`;
   the rope produces `` buffer `*probe*` (id BufferId(4)) is read-only ``.
   *Bite:* an adopter that deletes the intercept and relies on the rope
   passes 1–4 and fails this. The layering at `terminal.lua:351-366`
   requires the named error to survive.
6. **[fix-shape] `set_round_trip_input` is still set on both — asserted
   so that only the round-trip mark can make it pass. Rewritten in
   revision 3 (review P2-4), and the cited precedent was wrong.**

   `dispatch_idle_for` (`src/editor.rs:1126-1155`) returns `false` for
   **six** independent reasons, only one of which is the round-trip
   mark: a pending chord or terminal escape on that frontend
   (`:1130`), an active minibuffer, an active search, an active
   query-replace, an open menu (`:1135-1138`), a focused **side** window
   and `core.buffer_round_trips(window.buffer_id)` — both on `:1153`.
   A criterion that only asserts `!dispatch_idle_for(..)` is satisfied by
   any of the six.

   **Revision 2 named the wrong model.** It cited
   `tests/terminal_copy_mode_acceptance.rs` "criterion 16", but that file
   contains **zero** `dispatch_idle_for` references — `acc16`
   (`:321-339`) goes through `state.dispatch_idle()` and asserts no
   `is_side` premise. The test that gets the side-window half right is
   `tests/dired_acceptance.rs:969-1013`, which asserts
   `!window.is_side()` as an explicit fixture premise (`:975-989`,
   commented "A document window, deliberately: the panel arm of the same
   gate would otherwise be what makes this pass") **before** asserting
   `!s.dispatch_idle_for(FrontendId::LOCAL)`.

   **Both halves are required, because neither test has both.** The
   criterion asserts, for each of the dired and listview adopters:

   - **(a) the document-window premise**, `!window.is_side()` on the
     focused window, asserted as a premise so a fixture that later
     displays in a panel fails loudly rather than passing vacuously —
     `dired_acceptance.rs:975-989`'s shape verbatim;
   - **(b) `!dispatch_idle_for(FrontendId::LOCAL)` while the panel is
     focused**;
   - **(c) the positive control** — switch the same window to a plain
     `pmacs.buffer.create("*plain*")` and require `dispatch_idle_for` to
     become **`true`**. This is `acc16:332-337`'s half, and it is what
     rules out the other five clauses in one assertion: a stuck
     minibuffer, a pending chord, an open menu or an active search would
     keep the gate `false` across the buffer switch, so (c) failing is
     the signal that (b) passed for the wrong reason.

   *Bite:* delete the `set_round_trip_input` call in `listview.lua:106`
   / `dired.lua:516` and criteria 1–5 all still pass; only this fails,
   at (b). Falsify (a) by displaying the panel in a side window: (b) then
   passes with the round-trip mark deleted, which is the whole of P2-4.
   Falsify (c) by leaving a minibuffer open in the fixture: (b) passes
   and (c) fails. A daemon-side refusal does nothing for a replica's own
   mirror, which is why this is pinned through `dispatch_idle_for` and
   not through `read_only`.

   **Note what this criterion is NOT.** `dired_acceptance.rs:999`'s
   `status(&s).contains("read-only")` passes both before and after
   adoption, because `BufferError::ReadOnly` and the intercept's own
   message both contain that substring — the trap `docs/dired-stage2-framing.md`
   §3.1 hands to this lane. Criterion 5 is where the distinction is
   asserted, on the *full* message text; this one must not be counted as
   coverage of the adoption.
7. **[mutation] A refresh reaches the window, not just the rope** —
   pinned by **painting** a shrinking render (many rows → one) and
   asserting row 1 is empty, for each adopter. **Revision 2 corrected
   this criterion's bite (sweep A).** Revision 1 claimed it caught a
   "partial conversion" that kept a `bypass_intercept` write beside the
   primitive; that is wrong — such a conversion **raises** at the bypass
   write (§2.4, measured) and never reaches a stale paint. The real bite
   is the one-line mutation *delete the `notify_buffer_edit_to_windows`
   call in the `set_generated_contents` binding*
   (`src/lua_bindings/mod.rs:3092`), which a reviewer can perform.
8. **[`main`] Cursor clamp (Q#GB6).** After a shrinking refresh,
   `pmacs.editor.cursor() <= buf:len()` and `C-p` moves. *Bite:* measured
   on `ad41cf1` — cursor 29, len 2, `C-p` leaves it at 29. Fails on
   `main` today, including for terminal copy mode.
   **8b. [`main`] `view_top` clamp, on a LONGER buffer (Q#GB6, review
   P2-4).** With a window scrolled so `view_top` sits on line 5, replace
   `"a\nb\nc\nd\ne\nf\n"` (12 bytes, 7 lines) with a single line **longer
   than 12 bytes**, then require `view_top < TextView::line_count()`.
   *Bite:* a clamp gated on "the buffer shrank" passes 8 and fails 8b,
   which is the whole of P2-4. Unlike 8, this case is argued from the
   types and from `rebuild_views_for`'s existing clamp
   (`src/editor_core.rs:1853-1857`), **not measured** — staging it needs
   a scrolled window.
9. **[`main`] A foreign buffer named `*references*` is never adopted
   (Q#GB13).** Create a plain buffer of that name with user text, then
   open the references panel. Assert **both** halves: the user's bytes
   survive **and** an ordinary edit to the user's buffer still lands;
   and the panel appears under a disambiguated name. *Bite:* measured —
   `"my precious notes"` → `"H\nr1"`, one buffer not two, and the user's
   buffer is left permanently un-editable. The second half is what fails
   if adoption is merely made "safe" by skipping the render.
10. **[fix-shape] The disambiguation limit raises rather than adopting**,
    matching `dired.lua:493-503` / `terminal.lua:309-315`. *Bite:* an
    implementation that falls back to adoption once the limit is
    exhausted passes 9 and fails this.
11. **[`main`] A disambiguated listview panel still answers `RET`, `g`
    and `q` (Q#GB18, review P1-2).** Continue criterion 9's fixture: with
    a foreign `*references*` in place, open the references panel — it
    appears as `*references*<2>` — then, **through `dispatch_key`**,
    press `g`, `RET` and `q` in turn and assert the *content produced* by
    each: `g` re-renders (the `on_refresh` rows appear), `RET` fires
    `on_visit` (assert the visited item, via a probe that records it),
    and `q` restores the previous buffer. *Bite:* this is the criterion
    that fails against **Q#GB13 landed without Q#GB18** — disambiguation
    alone leaves `panel_for_current_buffer` looking up
    `panels["*references*<2>"]`, which was stored under
    `"*references*"`, so all three commands return early and do nothing.
    Assert what each command produced, not that it did not raise: every
    one of the three fails *silently*, so a "no error" assertion passes
    against the bug.
12. **[`main`] The `q`-target capture is not inverted (Q#GB18).** The
    fourth consumer, and it needs its own criterion because it fails
    **open** rather than closed. With a disambiguated panel focused, open
    a **second** panel (`*outline*`) and assert the second panel's `q`
    returns to the buffer that was current *before the first panel*, not
    to the first panel. *Bite:* with `panel_for_current_buffer` unable to
    recognise `*references*<2>`, `listview.open`'s guard at `:118-123`
    reads "the current buffer is not a panel" and captures the panel as
    `p.prev` — the chained-panel `q` loop the guard's own comment says it
    exists to prevent. Criterion 11 passes with this bug live, because
    each command works in isolation; only the two-panel sequence shows
    it.
13. **Q#GB16's fold change — SPLIT into two criteria in revision 4
    (review round 3, P2-5), because revision 3's single criterion
    contradicted its own classification.** Revision 3 labelled it
    `[fix-shape]` and then wrote that the first half "passes on `main`
    ... a dired buffer ... folds fine". If folding succeeds on `main`,
    an assertion that it returns `false` **fails** on `main` — which is
    a `main` pre-image, the opposite label. The two halves have different
    pre-images and belong apart.

    **13a. [`main`] A locked generated buffer is not foldable.** On a
    Stage 1 dired listing, `pmacs.fold.fold(buf, range)` returns
    `false`. *Bite:* this **fails on `main`**, where the same call
    returns `true` — verifiable by revert, and `scripts/bite` reports it.
    It is a regression pin *for the intended change*: the point is to
    make sweep C's silent behaviour change into a stated one, so the
    criterion asserts the new behaviour and the bite is that the old
    behaviour is different.

    **13b. [mutation] …and the refusal says why.** The status after 13a's
    call names the read-only lock, not `not a document buffer`. *Bite:*
    the falsifying mutation is reverting `fold.rs:68`'s string; that is
    the shape that ships if 13a is written alone — correct behaviour,
    false explanation. It cannot share 13a's `main` pre-image because on
    `main` the call succeeds and sets no status at all.
14. **[structural] No `bypass_intercept` write remains in `dired.lua` or
    `listview.lua`**; `listview.ensure_panel` contains no find-by-name
    adoption; and **no `panels[` subscript remains keyed by a name
    derived from `describe.buffer`** — the Q#GB18 half. Rides alongside
    1–13, never instead: a structural comparison of two authorities does
    not catch a misrouted consumer, which is why 11 and 12 assert through
    `dispatch_key`.

**Moved out of Stage 1 in revision 3:** the unlock criterion. Revision
2's Stage 1 criterion 11 pinned `unlock_generated`; Q#GB7 moves the
capability to Stage 2, so the criterion moves with it (Stage 2 criterion
13) and grows the negative terminal-identity half review P1-3 asks for.

### Stage 2

1. **[`main`] `M-x buffer.undo` cannot destroy `*compilation*` /
   `*shell-command*` / `*search-results*` content — and the criterion
   must assert the *exit marker survives*, not that the buffer is
   non-empty.** *Bite, and this is the whole point:* measured, the result
   of `M-x buffer.undo` on `*shell-command*` is
   `[shell exited with code 0]` replaced by
   `[output desynced by external edit]`. The buffer is still non-empty,
   so a "not empty" assertion **passes with the bug live**. The revision
   guard *marks* the corruption; it does not prevent it.
2. **[fix-shape] A streaming run's incremental writes still land**,
   including CR overwrite semantics (a progress-bar fixture) and
   erase-to-eol. Assert the produced content, not the absence of an
   error. *Bite:* the tempting half-conversion — reset via
   `set_generated_contents`, stream via `bypass_intercept` — raises
   `is read-only` at the first append (§2.4, measured).
3. **[`main`] The rope is locked BETWEEN batches, not only after the
   run.** Mid-run, after one output batch has landed and before the
   next, a **`bypass_intercept`** write must be refused and
   `Buffer::is_read_only()` must be `true`. **Rewritten in revision 2
   (review P1-1).** Revision 1 said "attempt an ordinary edit and require
   the refusal", which **passes on `main`** — the intercept refuses
   ordinary edits today whether or not the rope is locked. A bypass write
   is the discriminator: it lands on `main` (`compile.lua` performs nine
   of them) and raises once the rope is locked. *Bite:* a scope-shaped
   implementation that unlocks for a whole run passes 1 and 2 and fails
   this. A state predicate, not a geometric readout.
4. **[`main`, and also fix-shape] History is discarded per generated
   write, asserted past the lock.** In a Rust acceptance test, after N
   batches: `buffer.set_read_only(false)`, then assert `buffer.undo()`
   is `Err(BufferError::NothingToUndo)` and — under `--features crdt` —
   that the CRDT reports `can_undo() == false`; restore the lock.
   **Rewritten in revision 2 (review P1-1).** `Buffer::undo` calls
   `ensure_writable()` **first** (`src/buffer.rs:1302`) and returns
   `ReadOnly` before it ever looks at the stacks, so revision 1's
   "`buf:undo()` returns false" **passes against an implementation that
   locks the rope and never clears history**. Lifting the lock inside the
   test is what makes the assertion about history rather than about the
   lock. `tests/terminal_copy_mode_acceptance.rs:582-584` is the existing
   precedent for a Rust-side lift. This criterion fails on `main` (where
   history accumulates) *and* against the locks-but-never-clears
   implementation, which is the strongest pairing available.
5. **[`main`] `ensure_slot` leaves `*compilation*` locked before any
   run (Q#GB5).** Create the slot without running anything, then require
   a **`bypass_intercept`** write to be refused and `is_read_only()` to
   be `true`. **Rewritten in revision 2 (review P1-1):** revision 1's
   "attempt an ordinary edit; without the explicit lock it lands"
   **passes on `main`**, because `ensure_slot` installs the erroring
   intercept at `compile.lua:266` at creation time.
6. **[`main`] A generated write on a path-backed buffer is refused —
   on ALL FOUR surfaces, not just the legacy wrapper (Q#GB10; rewritten
   in revision 3 for review P2-5).** Open a file, then, against its
   buffer, exercise each of:

   | surface | call |
   |---|---|
   | the wrapper | `pmacs.buffer.set_generated_contents(b, "x")` |
   | insert | `b:insert(0, "x", { generated = true })` |
   | delete | `b:delete(0, 1, { generated = true })` |
   | replace | `b:replace(0, b:len(), "x", { generated = true })` |

   Each must error, and after each the buffer's contents, `is_read_only()`
   and `is_modified` must all be unchanged (§3.4 exit 1: nothing is
   touched). Second half, once: after an ordinary edit, autosave still
   queues that buffer.

   *Bite, and this is exactly P2-5's point:* a guard placed on
   `Buffer::set_generated_contents` rather than on
   `Buffer::apply_generated_edit` **passes the wrapper row and fails the
   other three**, while the newly public surface could still replace a
   file buffer's contents, lock its rope and — with `mark_clean` — make
   `autosave.rs:363` skip it, so a crash loses the user's edits with
   **no recovery slot**. Reverting the guard to the wrapper is the
   one-line mutation that falsifies this. Assert the autosave queue, not
   just the flag: asserting a value was stored is not asserting anything
   reads it.
7. **[`main`] `mark_clean` (Q#GB10).**
   `pmacs.describe.buffer(b).modified` is `false` after a generated
   write on a pathless buffer. *Bite:* measures `true` on `ad41cf1`.
8. **[`main`] Foreign buffers named `*compilation*`, `*shell-command*`
   and `*search-results*` are never adopted (Q#GB13)** — same two-halved
   shape as Stage 1 criterion 9, plus the limit criterion of 10. *Bite:*
   measured on `ad41cf1` for `*compilation*`.
9. **[`main`] A FAILED `pmacs.compile.run` leaves a foreign
   `*compilation*` untouched AND editable.** Call it with
   `display = "bogus"` against a pre-existing foreign buffer of that
   name. *Bite:* measured — today the call raises at
   `compile.lua:754`, the contents survive, and the user's buffer is
   nonetheless left permanently un-editable (`ensure_slot` ran first and
   installed an intercept it discarded the handle for). With Q#GB5's
   lock placed naively it would additionally be **emptied and locked**.
   This is the criterion that pins the ordering constraint, and it fails
   on `main` today for the intercept half alone.
10. **Coverage, not a criterion: both configurations** — default and
    `--features crdt` — for criteria 1–5 and for the §3.4 transaction
    criteria 15, 16 (first half), 16b and 18. CI never enables the
    feature, so CRDT must not be the only home of any of them.

    **Three are irreducibly `crdt`-only and must say so rather than be
    quietly written once** (revision 4): criterion 16's *second* half
    (the `Rejected` refusal needs a mid-codepoint position, which only
    loro rejects), **16c** (`Diverged` has no default-feature analogue at
    all), and Stage 2 criterion 4's `can_undo() == false` half. Each is
    paired with a default-configuration sibling that exercises the same
    cleanup arm through a non-CRDT failure — criterion 15's `FailingView`
    is the default-feature route into `AppliedThenFailed`, and 16b's
    empty write is the default-feature route into `NoOp` — so **no
    cleanup arm of §3.4 is reachable only under `crdt`** except
    `Diverged`, which by construction cannot be. That pairing is the
    point of the rule; naming which criteria are single-configuration is
    what keeps it checkable.
11. **[structural] Zero `bypass_intercept` writes remain** in
    `compile.lua` and in `default.lua`'s search panel (comments
    excepted; §1.1's arithmetic is the reference), and neither
    `ensure_slot` nor `ensure_search_panel` contains a find-by-name
    adoption.
12. **[fix-shape] The three intruder tests still assert what they were
    written to assert** after being converted to a Rust-side `read_only`
    lift (Q#GB12), rather than being deleted or weakened. *Bite:* a
    conversion that drops the intruder edit entirely leaves the desync
    machinery unpinned while the suite stays green.

**New in revision 3 — the transaction, the provenance, and the bounded
unlock.**

13. **[`main`] The unlock is real, is narrow, and refuses a lock it did
    not install (Q#GB7 + Q#GB15; review P1-3).** Three halves, and the
    third is the one revision 2 lacked.
    - *Real:* on a plain pathless buffer with no intercept, a generated
      write locks it (a `bypass_intercept` write raises),
      `unlock_generated` releases it (a bypass write lands), and an
      ordinary edit then lands too.
    - *Narrow:* on a listview panel, after `unlock_generated` an ordinary
      edit is still refused **by the intercept**, asserted on the full
      message text per Stage 1 criterion 5.
    - *Negative terminal identity, the criterion review P1-3 asks for:*
      open a real terminal, take its identity buffer id, and require
      `pmacs.buffer.unlock_generated(term_buf)` to **error**, with
      `Buffer::is_read_only()` still `true` afterwards and an ordinary
      edit still refused. Assert the post-state, not the error alone.

    *Bite:* a no-op unlock fails the first half; an unlock that also
    tears down the intercept — "unprotect" rather than "unlock" — fails
    the second; and **revision 2's `unlock_generated` as written passes
    the first two and fails the third**, which is the whole finding.
    Falsify the third by deleting the `identity_protected` check.
14. **[`main`] A generated write REFUSES a buffer someone else locked
    (Q#GB15; sweep C item 1).** Open a real terminal; call
    `pmacs.buffer.set_generated_contents(term_buf, "junk")` and each of
    the three `{ generated = true }` mutators against it. Every one must
    error, and the terminal's contents must be **byte-identical**
    afterwards. *Bite:* this **fails on `main` today** — shipped
    `set_generated_contents` does `self.read_only = false`
    unconditionally (`src/buffer.rs:546`), overwrites the buffer and
    re-locks it, and nothing in the tree refuses it. It is the pin for a
    hole that predates this arc, which is why it is a `main` pre-image
    rather than a mutation bite. Falsify by deleting §3.4's exit 2.
**Criteria 15-16 were VACUOUS in revision 3 and are rewritten. Review
round 3, P1-2, is right and the diagnosis is worth stating because it is
the second time this arc has shipped a criterion that passes with the bug
restored.** Both used an out-of-bounds op. Bounds validation is §3.4 exit
4 — **before** `editing_in_progress` is set and **before** the unlock —
so the operation never enters the transaction the criteria claim to test,
and an implementation that omits *both* the flag clear and the error-path
relock passes both. Worse, revision 3 argued *in the same document* that
pre-validation makes an invalid range cost nothing; the design decision
and the test strategy contradicted each other in adjacent sections. **The
rule this yields: a criterion must name the exit it drives the
implementation to, and that exit must be inside the mechanism under
test.** Every criterion below names its exit.

15. **[`main`] `editing_in_progress` is cleared on a failure that
    ENTERS the transaction (Q#GB17; §3.4 `AppliedThenFailed`).** Attach
    a Rust-side `FailingView` — `pmacs::view::View` is `pub`
    (`src/view.rs:221`, `src/lib.rs:141`) and `Buffer::attach_view` is
    `pub`, so a test crate can implement one whose `on_edit` returns
    `Err(BufferError::Intercepted { .. })` — then perform a **valid**
    generated write. It fails at the broadcast, *after* the rope swap.
    Then require an **ordinary** edit on the same buffer to report the
    intercept's message, **not** `is already being edited`.
    *Bite:* an implementation that returns from the `match` without
    reaching §3.4's line (6) leaves the flag set, and `begin_edit`
    (`:726-731`) and `apply_edit` (`:774-779`) then refuse **every**
    later edit to that buffer for the rest of the session. Falsify by
    moving the flag clear inside the `Applied | NoOp` arm. Assert the
    *next* edit's outcome, not the failing call's — the failing call
    reports the same error either way, which is the whole reason this
    criterion is about the buffer's state afterwards.
16. **[`main`] A generated write relocks on that same failure, and does
    NOT lock on a refusal (Q#GB17). Two halves, because §3.4 gives them
    opposite answers and revision 3 gave them the same one.**
    - *Relock on `AppliedThenFailed`:* after criterion 15's failing
      write, `Buffer::is_read_only()` is `true` and a
      `bypass_intercept` write raises. *Bite:* an implementation that
      unlocks and returns without relocking leaves the buffer writable,
      and every criterion about undo silently stops applying. Falsify by
      deleting `self.read_only = true` from the `AppliedThenFailed` arm.
    - *No lock on `Rejected`:* on a **fresh, writable** pathless buffer
      under `--features crdt`, a mid-codepoint generated insert is
      refused; afterwards `is_read_only()` must still be **`false`** and
      an ordinary edit must land. *Bite:* **revision 3's own design fails
      this** — it relocked unconditionally, so a caller got an error and
      a locked buffer it never wrote. Falsify by replacing
      `self.read_only = entry_read_only` with `= true`. This half is
      `crdt`-only, so criterion 10's coverage rule names it explicitly.
16b. **[`main`] A successful no-op still discharges the invariant
    (Q#GB17; §3.4 `NoOp`). New in revision 4 (P1-1 direction A).** On a
    pathless buffer, insert text and delete it back to empty so the rope
    is empty **and the undo stack is not**; then call
    `set_generated_contents(b, "")`. Afterwards: `is_read_only()` is
    `true`, `describe.buffer(b).modified` is `false`, and — after a
    Rust-side lift — `buffer.undo()` returns `Err(NothingToUndo)`.
    *Bite:* **revision 3's predicate fails all three.** The no-op arm
    (`src/buffer.rs:1245-1253`) returns `Ok` without bumping `revision`,
    so revision 3 skipped the clear and the `mark_clean` and left a
    locked, modified buffer with poppable history. Falsify by restoring
    `if self.revision() != rev_before`. This is not a hypothetical path:
    Q#GB5 prescribes exactly this call in `ensure_slot`.
16c. **[`main`, `crdt`-only] A CRDT mid-transaction failure is
    distinguishable and surfaces (Q#GB17; §3.4 `Diverged`). New in
    revision 4 (P1-1 direction B).** Drive an `EditOp::Replace` whose
    CRDT `delete` succeeds and whose `insert` fails
    (`src/buffer.rs:1140-1163`), and require: a **distinct** error
    variant, not `CrdtRejected`; the buffer left `read_only`; and undo
    history **not** cleared. *Bite:* revision 3's predicate did nothing
    at all here — `revision` never advanced — so it neither cleaned nor
    reported, in the one case the code's own comment calls "an invariant
    violation". Falsify by folding the variant back into `Rejected`.
    **Honest caveat on stageability:** unlike 15, 16 and 16b, this
    criterion has **no staging recipe verified in this document** —
    loro's `insert` is expected to succeed when the position is valid,
    which it is by construction here, so provoking the failure may need a
    fault-injection seam rather than an input. If Stage 2 cannot stage
    it, the correct outcome is the four-variant fallback named at the end
    of §3.4 — **not** a criterion that passes by never reaching its
    path, which is precisely what P1-2 caught.
17. **[`main`] An invalid-range generated write does NOT destroy undo
    history (Q#GB17).** On a pathless buffer with two ordinary edits
    already on the stack, call `b:delete(0, b:len() + 1000,
    { generated = true })`; require the error, then lift `read_only`
    Rust-side and require `buffer.undo()` to **succeed** and restore the
    prior contents. *Bite:* this fails against the shipped ordering
    transplanted verbatim — `set_generated_contents` calls
    `clear_history()` **unconditionally** (`src/buffer.rs:551-553`), so a
    call that changed nothing would wipe the user's history. It is the
    concrete cost review P1-1 asks the ordering to state, and pre-
    validation (§3.4 exit 4) is what pays it.
18. **[`main`] A re-entrant generated write is refused (Q#GB17).** From
    inside an `add_intercept` body on buffer X, call
    `X:insert(0, "x", { generated = true })`; require
    `ConcurrentEdit`, and require the outer edit to complete normally
    afterwards. *Bite:* omit the gate and the inner write mutates the
    rope while `run_managed_edit` phase 3 is holding an op computed
    against the pre-edit `InterceptContext`
    (`src/lua_bindings/mod.rs:1477-1487`); the visible symptom is the
    outer edit landing at the wrong offset, so **assert the resulting
    text**, not the error.
19. **[`main`] `is_generated_buffer` recognises a disambiguated buffer
    (Q#GB18).** With a foreign `*compilation*` in place, run
    `M-x compile`; the run lands in `*compilation*<2>`. From inside that
    buffer, run `M-x compile` again (the `g`-recompile path) and require
    the `q` target still to be the user's original buffer — not
    `*compilation*<2>`. *Bite:* with `is_generated_buffer` still
    comparing names, `compile.lua:762`'s guard reads the disambiguated
    buffer as "not generated" and re-captures it, so `q` returns the
    user into a compilation buffer. Assert where `q` lands, not whether
    the predicate returned a boolean.
20. **[structural] Zero name comparisons remain in the Class 1 sites.**
    `pmacs.compile.is_generated_buffer` contains no `d.name ==`, and
    `listview.lua` contains no `panels[d.name]`. Rides alongside 11–19,
    never instead.
21. **[`main`] Search works with no `pmacs.compile` present (Q#GB18;
    review round 3, P2-4). New in revision 4.** Build a `LuaHost`
    directly, call `attach_editor`, and — with `compile.lua` never
    loaded — run `pmacs.project.search`. It must not raise, and the
    `q` target must be the buffer that was current before the search.
    Then assert the harness premise explicitly: `pmacs.compile == nil`,
    so a fixture that later gains the runtime sequence fails loudly
    rather than passing as an ordinary editor test.
    *Bite:* **revision 3's design fails this at the first search** —
    `ensure_search_panel` called `pmacs.compile._register_generated_buffer`
    unguarded, and `pmacs.compile` is nil here. Falsify by replacing the
    symmetric guard with a direct call. This configuration is reachable
    with the pattern nine existing test files already use (`LuaHost` is
    `pub`; `default.lua` loads at `src/lua.rs:250-251`, `compile.lua` at
    `src/editor.rs:704`), and **no ordinary acceptance test reaches it**,
    because every `EditorState::new` loads both — which is exactly why
    the guard it defends was invisible enough for revision 3 to remove.

---

## 7. Bets

- **That a per-op `clear_history` is not a throughput problem.** Argued
  from `create_undo_manager` being `UndoManager::new(doc)` (O(1) in
  document size) and from at most one v0.1 entry existing per clear.
  **Measured in Stage 2, not asserted here.**
- **That converting compile's nine sites does not disturb its byte
  anchors.** The conversion changes *authority*, not op shape, position
  or count — `emit_text`'s `slot.out_pos` arithmetic is untouched. The
  bet is that nothing else in the module reads `read_only` indirectly;
  criteria 2 and 3 are what test it.
- **That `{ generated = true }` sitting beside `{ bypass_intercept = true }`
  is clearer than replacing it.** Q#GB2.
- **That duplicating the rope's bounds arithmetic in
  `validate_op_bounds` is worth what it buys** (§3.4). The alternative to
  a pre-check is a dry run, and `Rope` offers none; the cost is one O(1)
  helper that must stay in step with `Rope::insert` / `delete` /
  `replace`'s own bounds rules (`src/rope.rs:174-230`). What it buys is
  criterion 17 — an invalid range that costs no history. If the user
  prefers no duplication, the fallback is to accept the shipped
  unconditional clear and **drop criterion 17**, which should be a
  stated trade rather than a silent one.
- **That one extra `bool` on `Buffer` is the right size for lock
  provenance** (Q#GB15), rather than the enum §2.11's three policies
  really want. The bet is that the enum is a separable refactor; if it
  is not, the field becomes churn the refactor has to undo.

## 8. Deferred (named)

- **The REPL's undo exposure** (Q#GB8), with the §2.5 measurement.
- **Class C: `*buffer-list*`, `*help*`, `*workers*`** (Q#GB9), and
  specifically **`*help*`'s two independent owners** (§1.4) — a Rust
  writer at `src/help.rs:354` and a Lua writer at `default.lua:1239`,
  disagreeing on `mark_clean`, each with its own copy of the name
  constant. Whoever takes Class C decides who owns `*help*` before they
  decide what it writes with.
- **The CRDT `Replace` mid-transaction divergence** (§3.4's `Diverged`).
  `EditOp::Replace` is two loro ops — `crdt.delete` then `crdt.insert`
  (`src/buffer.rs:1140-1163`) — and if the first succeeds and the second
  fails, "the CRDT is mid-transaction ... and the rope is unchanged.
  This is an invariant violation", in the code's own words. **This arc
  names the state and contains it; it does not repair it.** Repair means
  either a single transactional splice or a reconciliation pass, both
  loro-level work with no bearing on generated buffers specifically. It
  reaches `apply_edit` and `apply_edit_skip_intercepts` today and is
  reported as an ordinary `CrdtRejected`, so nothing in the tree
  currently distinguishes it — which is the smaller half this arc does
  fix. Named here so the next lane starts from the citation rather than
  the symptom.
- **Suppress-rather-than-clear history recording**, if Stage 2's
  measurement says the per-op clear costs anything.
- **`read_only` in `describe.buffer`** (Q#GB14) — separable, no new
  capability, not required by this arc.
- **Replacing `read_only: bool` with a provenance enum** (Q#GB15's
  rejected alternative). It is the shape §2.11's three policies actually
  want, and it would let `document_bytes` ask the question it means
  instead of the question the flag happens to answer (Q#GB16). Rejected
  here as a refactor this arc would be smuggling; named as the right
  eventual shape.
- **The five `*scratch*` find-or-create copies** (§2.10 Class 5) —
  `default.lua:581`, `:1145`, `listview.lua:190`, `compile.lua:1052`,
  `dired.lua:914`. Correct only while `*scratch*` stays unowned and
  undisambiguated; nothing in the tree connects them, so a future lane
  that gives `*scratch*` an owner breaks all five at once.
- **`*scratch*` can be permanently locked by any Lua caller today.**
  Sweep B (revision 2) found this and revision 2 treated it as a reason
  to ship an unlock. It is a **pre-existing** exposure:
  `pmacs.buffer.set_generated_contents` is already public on `main` and
  already locks any buffer id it is handed. This arc neither creates it
  nor closes it — Q#GB15's provenance bounds who may *unlock*, not who
  may *lock*. Recorded as a standing hazard rather than as this arc's
  work, which is the correction revision 3 makes to sweep B's
  conclusion.
- **`docs/agent-handoff.md` §4's inventory is keyed by
  `bypass_intercept`** and therefore misses Class C, and its headline
  "four writer mechanisms" is **five** once `src/help.rs:354` is counted
  (§1.4). Not this lane's file to edit mid-flight; carried in the PR
  body.
- **Removed from this list in revision 3: `COHERENCE.md` §14's listview
  consumer list.** PR #189 landed the correction (§1.5). A merged
  correction is removed, not relabelled.
- **Returned to this list in revision 3: wdired's unlock.** Revision 1
  deferred it, revision 2 made it Stage 1 work, and revision 3 lands the
  *capability* in Stage 2 (Q#GB7 + Q#GB15) while the **wdired consumer**
  itself stays deferred to dired Stage 3, which is not framed.

## 9. Coherence impact (`COHERENCE.md` §20)

**Section served: §14 Coherent Workbench Primitives**, and specifically
its **Output channel** bullet, which is where this caveat is already
recorded ("*four writer mechanisms have not yet adopted it and remain
emptiable*"). This arc discharges that entry for Class A and replaces its
"a streaming variant of the primitive that does not exist yet" with one
that does. §14's list-primitive bullet is touched too: listview is called
"the strongest coherence asset in the UI layer", and it is currently
emptiable by one keystroke.

- **Priority 5 (finish the workbench convergence)** is the priority this
  serves. It is a correctness debt inside an existing primitive rather
  than a new primitive, so it is wiring, not model.
- **§14 consistency, added in revision 2:** Q#GB13 makes three writers
  honour an ownership rule the tree already states (`terminal.lua:300-305`)
  and already implements twice (dired, terminal). That is §14's thesis
  applied to a discipline rather than a view — five generated-buffer
  owners converging on one identity rule instead of three of them
  inventing find-by-name.
- **§6 interaction islands — none added.** No new keymap scope, no new
  dispatch shadow, no new precedence rung. The count stays at six. This
  arc deliberately does **not** add undo-chord rebindings anywhere; the
  measured point of the bug is that rebinding chords was never the fix.
- **§11 configuration registry — no new settings**, no adoption change.
- **§2 golden journey** — step 6 (compile) and the dired/browse steps are
  touched only in the sense that their buffers stop being destructible.
  No journey step opens or closes.
- **Background-work attribution — unchanged.** `compile.lua`'s process
  pump and the grep stream keep their existing ownership.
- **Protocol — no change.** Nothing new crosses the wire; the fan-out
  reuses `queue_daemon_origin_crdt_op`.
- **§14 correction — LANDED, not owed.** PR #189 (`main` @ `7586905`)
  corrected the listview consumer list and moved the scorecard row from
  ✓ to ◐ (§1.5). What remains owed on merge is the **handoff §4 table**,
  whose four-row inventory is keyed by `bypass_intercept`, therefore
  misses Class C, and undercounts the mechanisms by one (`src/help.rs`,
  §1.4).

## 9b. Cross-lane boundaries

**Three lanes touch adjacent ground. The boundaries below are settled
elsewhere and are recorded verbatim rather than re-decided here.**

**#186 / #171 — recorded, not this lane's:**

> #186 owns the urgent **pre-filesystem refusal** for synchronous
> `apply_resource_op`. #171 later owns **full post-delete lifecycle
> reconciliation**, including the **async race where a buffer becomes
> modified after dired dispatch**.

**#171 → #188: Q#DR25 is deferred INTO this lane, and revision 3 is the
first revision to say so.** Revisions 1 and 2 of this document never
mentioned Q#DR25, #171, or dired Stage 2 at all — a gap, since the other
lane had already handed the work over. Read against `#171` revision 7
(`fd7ae37`, pushed 2026-07-28), which is that document's current state:

- #171 §3.1 and its Q#DR25 entry state that dired's listing becoming a
  genuinely immutable generated buffer is **"not Stage 2's decision to
  make"**, that it is **"owned by the `generated-buffer-immutability`
  lane"**, and that **"Stage 2 does not implement it, does not gate on
  it, and carries no acceptance for it."** This document's Stage 1
  claims exactly that work (`dired.lua:369-372` adopting
  `set_generated_contents`), so the claim is live and the two documents
  agree.
- **Neither ordering creates a conflict**, per #171 §3.1: Stage 2b
  changes `paint`'s *callers*, this lane changes `paint` itself. If this
  lane lands first, Stage 2b rebases onto a `paint` that already writes
  through the primitive; if Stage 2b lands first, this lane adopts a
  `paint` with more callers and needs no change to them.
- **One inherited fact this lane must not lose**, recorded in #171 §3.1
  as "a trap for that lane's acceptance":
  `tests/dired_acceptance.rs:969`'s
  `dired_buffer_is_read_only_and_round_trips_input` asserts
  `status(&s).contains("read-only")`, and `BufferError::ReadOnly` renders
  as ``buffer `{name}` (id {id:?}) is read-only`` — so **that test passes
  both before and after the adoption** and is not coverage of it. It is
  cited in Stage 1 criterion 6 as the model for the *document-window
  premise* only; the note at the end of that criterion says so
  explicitly.
- **One difference worth flagging, not a conflict.** #171 revision 5's
  withdrawn plan had dired's `paint` adopt the primitive *"dropping the
  erroring intercept"*. This document keeps the intercept at both
  adopters, per the layering `terminal.lua:351-366` states. Since rev 6
  withdrew the decision from #171 entirely, this lane owns it and there
  is nothing to reconcile — but a reader who finds rev 5's phrasing
  should know it was superseded, not contradicted.

## 10. Verification plan

Full gate suite per `CLAUDE.md` for **each** PR separately:

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings   # own step
cargo test --lib
cargo test --lib --features crdt
cargo test --test <the touched acceptance suites>
cargo test --test m4_acceptance -- --skip basedpyright
PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu
git diff --check
```

Plus, per stage:

- **Stage 1** — `cargo test --test dired_acceptance` and
  `--test listview_acceptance` (the suite exists; `tests/listview_acceptance.rs`),
  plus `--test terminal_copy_mode_acceptance` if Q#GB6 lands, since the
  clamp changes the shipped snapshot path. **Plus
  `--test folding_acceptance`**, added in revision 3: Q#GB16 touches
  `src/lua_bindings/fold.rs` and `read_only_buffer_is_rejected`
  (`tests/folding_acceptance.rs:570`) pins the guard the lock now reaches.
  **Stage 1 touches Rust** (Q#GB6's clamp; Q#GB16's status string), so
  `cargo test --lib` and `--lib --features crdt` are load-bearing for it
  rather than formalities.
- **Stage 2** — `cargo test --test compile_mode_acceptance` **and**
  `--test compile_mode_crdt_acceptance`, plus
  `--test terminal_copy_mode_acceptance` (the `set_generated_contents`
  reimplementation, `mark_clean`, **and Q#GB15's write-direction refusal**
  all reach it), plus `--test vterm_stage1_acceptance` — added in
  revision 3, because `tests/vterm_stage1_acceptance.rs:139,175,290` are
  the shipped assertions about a terminal identity buffer's `read_only`
  and Q#GB15 changes what may touch it. **The search panel has no suite
  of its own** — `grep -rln 'search-results' tests/` returns only
  `compile_mode_acceptance.rs` and `m4_acceptance.rs`, so Stage 2's
  search-panel criteria need a new home rather than an existing one to
  extend.
- **Run the `crdt`-gated terminal suite explicitly, as its own step.
  Added in revision 4 (review round 3, P1-3).**

  ```
  cargo test --test terminal_copy_mode_acceptance --features crdt
  ```

  This is **not** covered by any other line in the gate list.
  `acc16e_a_refresh_queues_the_owners_write_for_replica_mirrors` is
  `#[cfg(feature = "crdt")]` (`tests/terminal_copy_mode_acceptance.rs:567`),
  and it is the shipped consumer of the lift-and-restore idiom that
  revision 3's `generated_lock` would have broken. A default-feature run
  of the same suite **never compiles it**, so a green sweep proves
  nothing about the seam P1-3 is about. Judge it by whether the test
  count includes `acc16e`, not by the verdict alone.
- **Stage 2 additionally needs a `crdt` run of whatever suite hosts the
  §3.4 transaction criteria**, for criterion 16's second half and 16c.
  Same reasoning, same failure mode.
- **Run `scripts/bite` on every criterion expressible as a test today.**
  Stage 1 criteria 1–3, 8, 9 and Stage 2 criteria 1, 7, 8, 9, 14 have
  `main` pre-images and can be falsified by revert; the rest are
  fix-shape or mutation bites and each names its mutation inline. A
  criterion whose bite cannot be stated as either is not finished.
- **Do not gate any new test on `#[cfg(feature = "crdt")]` unless it
  genuinely needs CRDT.** CI never enables the feature — measured at
  `ad41cf1`, **276 tests are dark** as a result:

  ```
  cargo test --all-targets --no-default-features --features lua54 -- --list \
    | grep -c ': test$'          # 3251   (CI's exact flags)
  cargo test --all-targets --no-default-features --features lua54,crdt -- --list \
    | grep -c ': test$'          # 3527
  ```

  3,527 − 3,251 = **276**. **Revision 3 deliberately does NOT re-quote
  this at `7586905`.** The base moved (#189, `COHERENCE.md` only, which
  adds no tests), so the reading is very probably unchanged — but "very
  probably unchanged" is the reasoning the ledger warns against, and a
  framing doc is not the authority for this number in any case. Treat
  276 as a reading taken at `ad41cf1`, not as a constant. **Re-measured
  in revision 2 (review P2-5).**
  Revision 1 quoted **264**, which `docs/active-work.md:107-115` labels
  historical (#168's reading at `1b6a084`) and explicitly warns against:
  "the number moves with every merge and must be re-measured, not
  quoted." The ledger's own most recent figure is 273 at `74301d1`; this
  arc's base is later, and the number should be re-measured again rather
  than quoted from here.
- **Judge the touched suites by elapsed time as well as verdict** where
  they reach for a sibling binary (`docs/agent-handoff.md` §5).
- Commit before gating: `cargo fmt` after a commit splits the worktree
  from the branch and `git diff --check` will not catch it.
