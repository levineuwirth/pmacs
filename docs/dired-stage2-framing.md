# Dired Stage 2 — marks and operations — framing

**Revision 5 — 2026-07-28. Status: PROPOSED — NOT APPROVED. This
document has never received a formal framing approval, and it needs one
from the user before any implementation branch is cut.** Its four
commits embody three rounds of review findings; that is not the same as
approval, and GitHub records no review on PR #171.

**Ground truth: re-scouted 2026-07-28 against canonical `main` @
`6bee09d`** (`Merge pull request #184 from levineuwirth/bottom-panel-stage2b`).
Rev 4 was scouted at `c8ec8f3` — which is dired Stage 1's *own* merge
commit — and `main` has moved **153 commits** since. Rev 5 is that
re-scout: §0's round-4 section states exactly what moved, what it
invalidated, and what survived unchanged.

Continues `docs/dired-framing.md` (rev 7, approved; Stage 0 merged as
#162, Stage 1 as #165). That document's §6 and §7 carry the *approved*
shape of marks and operations; this one re-verifies every claim in them
against the tree, corrects what has drifted or was wrong, and adds what a
Stage 2 implementation needs and the parent did not decide: the
batch-execution contract, the confirmation surface, the staging cut, and
acceptance. Decisions continue the parent's scheme from **Q#DR12**.

The parent framing's ground truth (§2) was scouted at `main` @ `e745068`
and re-verified at `0827dd1`. Stage 1 then changed three of the files
Stage 2 depends on most (`src/fs.rs`, `src/async_runtime.rs`,
`src/lua_bindings/mod.rs`), and #178/#179/#181/#182/#183/#184 have since
changed more, so **§2 below supersedes the parent's line references for
everything Stage 2 touches.** Line numbers throughout are given for
navigation only; every claim is anchored to a **symbol**, because line
numbers drift and this document has now watched them drift twice.

---

## 0. Revision history

### Re-scout round 4 (rev 4 → rev 5) — `c8ec8f3` → `6bee09d`, 153 commits

No reviewer produced these; a re-scout did. Rev 4's design survives, but
**one new dependency changes what Stage 2 must build, one changes the
shape of a mechanism rev 4 described, and seven of rev 4's own claims
about pmacs were wrong** — which is the failure mode this arc has been
burned by before (a framing that verifies its *external* facts and gets
its *internal* ones wrong). Every claim below was read on the tree at
`6bee09d`, not inferred.

#### What arrived that Stage 2 must now answer

- **N1 (new scope, blocking the mark layer) — #178 landed
  `Buffer::set_generated_contents`, and dired has not adopted it.**
  `src/buffer.rs:545` is now *the* authorized write path for a generated
  buffer: lift `read_only`, one whole-buffer `Replace` skipping
  intercepts, `clear_history`, re-assert `read_only`, and **return the
  `Edit`**. `docs/agent-handoff.md` §4 names `builtin/runtime/dired.lua:371`
  as one of **four writer mechanisms that have not adopted it**, and
  `COHERENCE.md` §14 records the same. Dired still pairs an erroring
  intercept (`dired.lua:509-511`) with a `bypass_intercept` whole-buffer
  replace (`paint`, `dired.lua:369-372`) **over a still-writable rope**,
  so **`M-x buffer.undo` empties a dired listing today** — the command
  exists (`builtin/commands/default.lua:179`), needs no keybinding, and
  `Buffer::undo` reaches the rope through `ensure_writable`
  (`src/buffer.rs:568-577`) without ever consulting the intercept chain.
  Stage 2 writes that buffer on every mark, every unmark, every toggle,
  and after every batch, so it multiplies the exposure rather than
  inheriting it quietly. Rev 5 adopts the primitive as **Q#DR25** (§3,
  §10) and states the three traps the handoff attaches to it.
- **N2 (shape change) — #182 (Journey Stage 1a) demoted dired to a
  replaceable slot, and rewrote the function Stage 2's operations run
  inside.** `resolve_target_buffer` gained a `ResolvedTarget::Directory`
  arm **ahead of** the load (`src/editor_core.rs:964-970`);
  `EditorState::open` became a caller rather than a parallel
  implementation (`src/editor.rs:953-959`); and which surface handles a
  directory is now the `path.open-directory` hook chain with dired as a
  **replaceable fallback slot**, `pmacs.path.directory_handler`
  (`dired.lua:736-738`, registered via
  `pmacs.path.set_directory_handler`, `src/lua_bindings/mod.rs:3679`) —
  deliberately **not** a hook subscriber, because `HookRegistry::add`
  only appends and builtins load before `init.lua`, so a dired
  subscription would always claim before any user listener. The
  consequence for Stage 2 is in `open_directory`, which gained
  `opts.dest` and a `commit()` closure run under
  `pmacs.window.commit_to` (`dired.lua:656-707`), and that scope
  **refuses an `await` inside it** (Q#JR14b). §9's serialize-and-await
  batch therefore has a constraint rev 4 could not have known about.
- **N3 (a ratchet Stage 2 must not break) — `tests/journey_acceptance.rs`.**
  **24 `#[test]`s**, 0 ignored, 0 feature-gated, 2 gated
  `#[cfg(target_os = "linux")]`. Its module doc states the rule
  verbatim at `:12`: *"**This file is a ratchet: stages add rows, none
  removes them.**"* Seven of its tests assert on dired directly, and its
  step-3 rows pin that a directory launch produces a `*dired:<canon>*`
  buffer. **The ratchet is split across two files**: #183 put the GPU
  journey row in `tests/gpu_invocation_acceptance.rs`
  (`public_gpu_directory_target_reaches_dired_and_leaves_the_daemon_usable`),
  not in `journey_acceptance.rs`. Both are gates now (§14).
  *(Correcting the re-scout brief: #183 did **not** extend
  `journey_acceptance.rs` — `git diff c2d56ff 7fd646d -- tests/journey_acceptance.rs`
  is empty.)*
- **N4 (a fan-out lesson Stage 2's two new hooks inherit) — #179/#181
  landed the typed-edit consumer chain.** Dired participates in
  **neither** the chain nor `buffer.after-edit` (grep of `dired.lua` for
  `typed_edit|after-edit`: **0**), and `set_generated_contents` fires no
  hook — its Lua binding routes only through
  `notify_buffer_edit_to_windows` (`src/lua_bindings/mod.rs:3092`),
  which runs no Lua. So N1 does not drag dired into the chain. But
  `resource.renamed` / `resource.deleted` are new fan-outs and inherit
  the chain's three hard-won lessons: a consumer cannot both edit and
  let a later consumer act meaningfully (the record is a **copy** taken
  before any consumer ran); `buffer.after-edit` fan-outs **nest**; and
  the thing that counts fan-outs must be **unskippable**. §5 states
  which of these bind the new hooks and which do not.

#### Rev 4 claims that are wrong about pmacs itself

- **W1 (load-bearing) — §5 step 2 names one function and cites a
  different one's line.** Rev 4: *"`drain_external_cancelled` (`:1596`)
  is the existing sweep."* These are **two distinct functions**:
  `LspManager::drain_external_cancelled` (`src/lsp.rs:1561-1576`) is the
  **server-scoped, unconditional** drain — settle every awaiter for
  `sid` cancelled and drop its `pending_external` entries; that is the
  right precedent. `LspManager::drain_cancelled_externals`
  (`src/lsp.rs:1596-1645`) is a **per-tick, per-awaiter cancellation and
  timeout sweep** that removes only awaiters whose `CancellationToken`
  was flipped or which outlived `request_timeout`. An implementer
  following rev 4's line number reaches the second: **a rename flips no
  token, so the drain half of `forget_uri` would be a silent no-op** and
  a coroutine awaiting a request against the old URI hangs forever —
  precisely the failure step 2 exists to prevent. Corrected in §5 and
  Q#DR23.
- **W2 — there is no `fn restart`.** Rev 4 models `forget_uri` on *"the
  server-scoped teardown that already exists (`restart`, `:1316-1331`)"*.
  The teardown is **`LspManager::start_generation`**
  (`src/lsp.rs:1307-1345`); its route purge is at `:1324`, which is the
  one number rev 4 got right. And there is a **second** precedent rev 4
  never names: **`LspManager::forget`** (`src/lsp.rs:3015-3042`), which
  does the same three things (`pending_routes.retain`,
  `drain_external_cancelled`, `documents.retain`) plus
  `status_tracker.forget` and `project_servers`. Rev 4's genuinely
  surprising observation survives and now applies twice: **neither**
  server-scoped teardown clears the fourteen result stores.
- **W3 — the route inventory is off, and the gap is the part an
  implementer trips on.** `ResponseRoute` (`src/lsp.rs:835-859`) has
  **15 variants, of which 14 carry a `uri`**; the fifteenth is
  `WorkspaceSymbol { query }`. There are **16** insert sites, all in
  `request_*` methods spanning `src/lsp.rs:1683-2132` — and **15 of the
  16 insert a URI-bearing route**. Rev 4's enumeration of fifteen was
  therefore *correct as a list of URI-bearing inserts* and *wrong as a
  claim about how many insert sites exist*. The arithmetic matters
  concretely: a purge predicate written as *"retain unless the route's
  uri equals the old one"* has to say something about a variant that has
  no `uri` field at all, and `request_workspace_symbol`
  (`src/lsp.rs:1877-1891`) is that variant. Stated in §5.
- **W4 — the Lua-side blast radius is ~3× what rev 4 said.** F1 put
  `rec.uri` at *"~20 sites"*. The real figure on this tree is **57
  lines** in `builtin/runtime/lsp.lua` (`grep -c "rec\.uri"` = 57, one
  per line). Rev 4's design is unaffected — the point of caching
  `rec.uri` in the attachment record is that one rebind reaches all of
  them — but the magnitude was understated and the arc has a standing
  rule against sizing anything from a partial count.
- **W5 — the path-owner census has grown to SIX.** Rev 4's five owners
  all still hold (re-verified individually in §5). A **sixth** arrived
  with the Lean 4 stages: **`lean.lua`'s `M.file_progress`**
  (`builtin/runtime/lean.lua:646`, written at `:651`), a **URI-keyed
  Lua module table** populated from the `$/lean/fileProgress`
  notification and, by its own comment, read by Stage 5's goal view. It
  lives in **no Rust store**, so `forget_uri` cannot reach it; it *is*
  reachable from a `resource.renamed` Lua subscriber, which is an
  argument for the hook rev 4 already proposed. Two weaker, **persisted**
  owners are also now named rather than fixed: `saveplace.lua`'s
  path-keyed places file and `recentf.lua`'s MRU list (§11).
- **W6 — the ledger note names a merged PR.** §16 said
  `docs/active-work.md` and `docs/agent-handoff.md` are held by open PR
  #169. #169 merged as `74301d1`. The current holder is **PR #185**
  (`docs-landed-state-184`). The instruction is unchanged; the number
  was stale.
- **W7 — the C1 seam moved, and it has two namesakes.**
  `pmacs._async._tick` is at `src/lua_bindings/mod.rs:7104-7115`, inside
  `install_async`, not `:6911-6924`. Its closure is
  `move |lua, ()|`, so `lua.app_data_ref::<SharedCore>()` is reachable
  and C1's decision stands. Worth naming because `mod.rs` defines
  **three** `_tick` bindings in different classes —
  `install_async:7104` (`pmacs._async._tick`),
  `install_process:8469` (`pmacs.process._tick`), and
  `install_lsp:10271` (`pmacs.lsp._tick`) — and only the first has
  `lua` in scope.

#### What the re-scout confirmed unchanged

Listed because "still true at `6bee09d`" is the load-bearing half of a
re-scout, and because each of these was read again rather than carried
forward: the whole `pmacs.fs` surface and its five `*_blocking`
implementations, at **identical line numbers** (§2); the absence of
`mkdir`, `copy`, and any recursive remove (the four `create_dir` hits in
`src/fs.rs` are all inside `#[cfg(test)]` bodies); `PendingJob`'s seven
fields and `JobKind`'s **12** variants; `tick`'s signature
(`pub fn tick(&self) -> Vec<JobId>`, `src/async_runtime.rs:1003`) and
the `Sleep | FsUnit` collapse at `:1046`; `dispatch_fs_rename` moving
both paths into the closure (`:871`); `LspManager`'s **fourteen**
URI-bearing store families (`src/lsp.rs:753-793`), exactly the fourteen
rev 4 tabulated, in the same order; `documents` at `:810` and
`pending_external` at `:804` with the *"drained-cancelled wherever
`pending_routes` is purged"* contract at `:801-803`;
`DiagnosticView.uri` private, set once at construction, its doc still
reading *"M5 may add re-rooting if a buffer is renamed"*
(`src/diag.rs:456-457`); `View` still carrying `overlay_identity` and
`clone_for_split` and **no** `rename_resource` and no downcast
(`src/view.rs:300`, `:310`, `:285`); the overlay-disposal sweep over
`core.windows.values_mut()` (moved to `src/lua_bindings/mod.rs:2044-2046`);
`_attach_view` taking `active_window_mut()` and erroring
(`src/lua_bindings/diag.rs:211-232`); `apply_resource_op`'s rename arm
still doing a raw-path `find_by_path` first-match rebind
(`src/lua_bindings/mod.rs:3306`) and its delete arm still killing
through `remove_buffer_and_fire` with **no modified check**
(`:3339-3341`) — **so an LSP-authored delete still destroys unsaved work
on `main` today**; `apply_workspace_edit` still capturing `origin` as a
**string** and restoring with `find_or_open` (`builtin/runtime/lsp.lua:1338`,
`:1352`), its own comment still conceding the path *"may have just been
renamed or deleted"*; `resolve_target_buffer`'s `NotFound` arm still
materializing a phantom path-backed buffer (`src/editor_core.rs:980-989`);
`Buffer::set_name` existing with the doc *"Used by save-as and rename
operations"* (`src/buffer.rs:453`) and **no** `pmacs.buffer.set_name`
Lua binding; `killring.lua`'s `push_entry` still a `local function`
(`:93`) and `copy()` still region-required (`:184-194`); **no**
`builtin/runtime/minibuffer.lua` and **no** `y_or_n`/`yes_or_no`/
`yes-or-no` anywhere in `src/` or `builtin/`; `pmacs.fs.rename` still
having **zero production callers** (only `tests/fixtures/pmacs-dired/init.lua:1200,1210`,
`tests/m8_1_acceptance.rs:359`, and `tests/m8_3_acceptance.rs:182-184`'s
monkeypatch) — and the same is now separately confirmed for
`pmacs.fs.remove` (only `tests/m8_1_acceptance.rs:438,439,472`); and
every one of dired's own Stage 1 seams at **unchanged line numbers**
(`render_entry:334`, `paint:369-372`, `entry_at_cursor:381-385`,
`seat_cursor:405-416`, the handle shape `:521-528`). The stale `*errors*`
comment rev 4 offered to fix in passing is still there, having moved
from `:665` to `:718`.

Two dired references in rev 4 **did** drift and are corrected in §2: the
mode-scoped `bind` helper is at `dired.lua:933-935` with the bindings at
`:937-946` (was `:866-868`), and `pmacs.dired._layout` is at `:954-961`
(was `:887-895`).

---

### Review round 1 (rev 1 → rev 2)

> **The three round sections below are a historical record and keep the
> line numbers they were written with (`c8ec8f3` / `c93f9ee`).** Their
> *findings* all still hold — each was re-verified at `6bee09d` — but
> for a current reference use §2 and §5, which carry the re-scouted
> positions. Where a round-section claim was found to be wrong about
> pmacs, §0's round-4 list says so and the live section is corrected;
> the round text is left as written rather than retconned.

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
  All five additions taken; see §13 items 14, 27, 39, 40, and 41
  (renumbered again in rev 4).
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

### Review round 3 (rev 3 → rev 4)

Round 3's theme: rev 3 named the right seams but two of them were sized
from a partial inventory, and one promise was still stronger than the
mechanism behind it. All three verified against `c93f9ee`.

- **H1 (blocking) — the modified check still races the syscall.**
  Correct, and rev 3's "immediately before each syscall" was wrong about
  where the boundary is: `pmacs.fs.remove` **dispatches a worker**, so the
  interval between the Lua check and `remove_blocking`'s
  `remove_file`/`remove_dir` is wide open, and acceptance 20 (edit before
  `y`) could never detect it. Rev 4 **narrows the promise** to a
  TOCTOU-bounded pre-dispatch check — the same honest framing G6 forced on
  `R` — rather than inventing a reservation primitive inside a dired
  stage. The residue is stated: reconciliation preserves the
  newly-modified *buffer*, but the *file* is gone, so **the orphan
  deferral rev 3 scoped to the LSP path applies to dired too** (§6, §11).
- **H2 (blocking) — the LSP teardown inventory was a third of the real
  one.** Rev 3 cleared five stores. `LspManager` (`src/lsp.rs:741-819`)
  holds **fourteen** URI-bearing store families, plus the `documents` text
  map that `didChange` diffs against, and `pending_routes` — whose
  `ResponseRoute` variants **carry the URI** at fifteen insert sites
  (`:1684-2133`), so an in-flight response repopulates a key *after* any
  clear. §5 now carries the full inventory and a purge/drain policy
  modelled on the server-scoped teardown that already exists (§5).
- **H3 (blocking) — the diagnostic-view seam was still "either/or".**
  Verified: `DiagnosticView.uri` is private and immutable, `View` has no
  downcast, and `_attach_view` (`lua_bindings/diag.rs:211-232`) takes
  `active_window_mut()` and **errors** if the active window is not showing
  the buffer — so it reaches one window. Rev 4 **chooses** the seam, and it
  has direct precedent: a `View::rename_resource` default-no-op hook
  alongside `overlay_identity` and `clone_for_split` (the hook family
  #113 added for this exact class), swept over
  `core.windows.values_mut()` the way overlay disposal already is
  (`mod.rs:2016-2019`). In-place mutation, so **overlay order is
  preserved by construction** (§5).
- **H4 (merge-readiness)** — #169's problem, not this PR's; fixed there by
  merging `c93f9ee` and aligning its recovery threshold with its own
  canonical anchor.
- **H5 — four cleanups**, all taken: the §4 item-35 → item-40 reference,
  the §5 acceptance 27 → 30 and 28 → 32 references, and the §10 table's
  obsolete "rename-only Rust" description.

**And the staging call, taken as directed: three PRs, not two** (§10,
Q#DR17). The reconciliation transaction lands first, on its own — after
three rounds the LSP lifecycle and multi-window diagnostic work are
substantial enough that reviewing them beside a mark layer would waste
the round.

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
  *(Rev 5, W7: the decision stands, verified again at `6bee09d`. The
  seam is now `src/lua_bindings/mod.rs:7104-7115`, inside
  `install_async`; its closure is `move |lua, ()|`, so
  `lua.app_data_ref::<SharedCore>()` is still reachable. Note that
  `mod.rs` defines **three** `_tick` bindings in different classes —
  `install_async:7104`, `install_process:8469`, `install_lsp:10271` —
  and only the async one has `lua` in scope.)*
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

**Sections served: §20 Priority 1 (protect the golden product journey)
and §14 (coherent workbench primitives).** Rev 4 cited neither by
number; the re-scout makes both concrete, and Priority 1 is a *new*
claim that only became true when #182 landed.

- **Journey steps — and this changed under rev 4.** Rev 4 said Stage 2
  touches "step 7's file half" only, and that dired "deepens a surface
  the user must still already know about". Half of that is now false:
  **#182 put dired on step 3.** `COHERENCE.md` §20 Priority 1 records
  directory-argument handling as **done** and says it "routes `pmacs .`
  into #165's dired buffer rather than growing a second directory
  surface", and `tests/journey_acceptance.rs` pins exactly that
  (`journey_step3_opening_a_directory_lists_it`). So dired is no longer
  only a surface the user must know to ask for — for anyone who runs
  `pmacs .`, **it is the first thing they see**, and step 5's row
  (`journey_step5_editing_a_file_reached_through_the_directory`) reaches
  the editable file *through* it. That raises the stakes on Stage 2's
  correctness rather than changing its scope, and it is the strongest
  argument for N1: a step-3 surface that `M-x buffer.undo` can empty is
  a journey regression waiting to be filed. Discoverability of the
  *marks* remains §20 Priority 4's job, not this stage's, and this
  framing still does not claim otherwise.
- **Workbench convergence (§14, Priority 5).** `COHERENCE.md` §14 names
  dired explicitly in the generated-buffer adoption gap — "four writer
  mechanisms have not yet adopted it and remain emptiable … and dired
  buffers" — and classifies dired, with listview, as **"the cheap
  half"**, because both already write whole-buffer replaces and so need
  none of the streaming variant the three appending buffers require.
  Q#DR25 closes dired's quarter of that gap. It does **not** close the
  other three, and this framing does not claim progress on them.
  Separately, §14's tree-primitive point is untouched: Stage 2 adds no
  tree, and `i` (insert subdirectory) stays deferred (§11) precisely so
  it can land on a shared primitive rather than inventing one.
- **Interaction islands: adds none.** Every new key is an entry in the
  existing mode-scoped `dired` keymap (`dired.lua:937-946`), and every new
  command is an ordinary `pmacs.command.define`, so all of it is reachable
  from `M-x` and describable by `describe.key`. The confirmation prompt
  uses the existing minibuffer rather than a new modal surface — that is
  the *reason* §7 spends a section on it.
- **Config registry.** Stage 2a and 2b add no keys. Stage 2c adds
  `dired.recursive-deletes` (Emacs's `dired-recursive-deletes`), through
  `pmacs.config.define` like `dired.kill-when-opening`
  (`dired.lua:78-84`).
- **Background-work attribution (§9): this stage makes a real, and
  honest, dent in the wrong direction unless it is deliberate.**
  `COHERENCE.md` §9 grades the worker model "mechanism without identity"
  and names the prerequisite precisely: `PendingJob`
  (`src/async_runtime.rs:367-375`) carries no owner, purpose, buffer
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

**And one correctness fix to Stage 1 that Stage 2 may not skip
(Q#DR25, new in rev 5): dired's listing becomes a genuinely immutable
generated buffer**, written through `pmacs.buffer.set_generated_contents`
rather than an erroring intercept over a writable rope. On `main` today
`M-x buffer.undo` empties a dired listing — no keybinding required — and
every mark, unmark, toggle and post-batch revert Stage 2 adds is another
write to that undo stack. §3 states the change; §10 places it at the
head of 2b.

Plus, invisibly: **renaming or deleting a path starts reconciling every
consumer that holds it** — buffer path *and* name, the fourteen
URI-keyed LSP store families and the attached diagnostic view, dired's
own pathless handles, `lean.lua`'s URI-keyed progress table, and the
workspace-edit applier (§5, §6). That is a correctness fix to shared
substrate; it is the reason `R` is safe on a directory at all, and it
closes a path on which an LSP-authored delete currently destroys unsaved
work.

Not in Stage 2: `wdired` (Stage 3), subdirectory insertion (`i`),
shell commands on marks (`!`), regexp marking (`% m`), and
compress/symlink/hardlink ops. §11 names them.

---

## 2. Ground truth (re-scouted 2026-07-28, `main` @ `6bee09d`)

Everything below was read or executed on this tree, not inferred. Where
a fact was already in rev 4 and survived the 153-commit gap unchanged, it
is left as written; where a line number moved it is corrected in place
without comment; where a *claim* changed, the change is called out.

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
- `PendingJob` (`:367-375`) carries `{cancel, state, supersede_key,
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
  read in a loop that already exists. *(Unchanged at `6bee09d`, line
  numbers included.)*
- `find_by_path` (`buffer_registry.rs:168-174`) is exact `Path` equality
  over `self.order`, **first match only**. No prefix logic, and two
  buffers on one path means one of them is invisible to it.
- `apply_resource_op`'s rename arm (`mod.rs:3291-3312`) does a
  **synchronous** `std::fs::rename` on the main thread and then
  `reg.borrow().find_by_path(&from)` with the **raw** path (`:3306`),
  while stored paths are normalized on write (`editor_core.rs:889-890`)
  and the normalizing wrapper `find_buffer_for_path` (`:935-938`) exists
  and is bypassed. This is a **second**, LSP-facing rename path with the
  same two defects; §5 fixes both in one change. *(Rev 4 cited
  `:3234-3255` / `:3249` / `:819` / `:864-867`; the code is unchanged,
  only its position.)*

### A verified pre-existing defect Stage 2 must not lean on

**A fire-and-forget non-stream job leaks its pending entry forever.** The
only two removals from `pending` are stream eviction of closed streams
(`:1225`) and `take_result` (`:1262-1282`); the Lua `Handle` is a
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
`resolve_accepted_value` (`src/minibuffer.rs:564-575`) **short-circuits
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
(`:509-511`) rejects everything else. **Both of those are what Q#DR25
replaces** (§3). `seat_cursor` (`:405-416`) re-seats by basename and
carries the warning that `move_to_line` is **ambient** — every
post-`await` seat must first check `pmacs.window.buffer()`.
`entry_at_cursor` (`:381-385`) maps cursor line *n* to `entries[n]`,
returning nil on the header and the footer. Keys are mode-scoped through
one helper (`bind`, `:933-935`, with the nine bindings at `:937-946`),
and `pmacs.dired._layout` (`:954-961`) exports the column contract.
`tests/dired_acceptance.rs` carries **25** tests.

One stale comment to fix in passing: `:718` still says an uncaught raise
lands "in \*errors\*", which #161's COHERENCE finding falsified and
which the module doc at `:56-72` already corrects. Same file, two
answers. *(It was `:665` in rev 4; #182 moved it, and it is still
wrong.)*

### The generated-buffer write invariant, which dired has not adopted (N1)

`Buffer::set_generated_contents` (`src/buffer.rs:545-556`) is, per
`docs/agent-handoff.md` §4, **the one authorized write** for a generated
buffer. It does four things as a unit: clear `read_only`, apply one
whole-buffer `EditOp::Replace` through `apply_edit_skip_intercepts`,
`clear_history()`, re-assert `read_only` — and **return the `Edit`**.

The three traps the handoff attaches to it, each verified here:

- **An intercept is not read-only.** `Buffer::undo` reaches the rope via
  `ensure_writable` (`src/buffer.rs:568-577`), which consults only the
  `read_only` flag and never the intercept chain. `M-x buffer.undo` is a
  defined command (`builtin/commands/default.lua:179`) reachable from
  M-x with no binding at all, so rebinding `C-/` buffer-locally would
  not close it. **Dired sets no `read_only`** — `claim_handle` installs
  an intercept and `set_round_trip_input`, nothing more
  (`dired.lua:506-519`) — so a dired listing is emptiable today.
- **A rope write is only half an edit.** The `Edit` must be fanned out
  or a displaying window keeps a `TextView` line index describing the
  previous contents. The Lua binding already discharges this:
  `pmacs.buffer.set_generated_contents`
  (`src/lua_bindings/mod.rs:3079-3095`) releases the registry borrow and
  then calls `notify_buffer_edit_to_windows` (`:3092`). **So a Lua
  caller inherits the fan-out for free** — this is not something
  `dired.lua` has to arrange, and the comment at `:3088-3091` says why
  the borrow is dropped first.
- **"Discard history" means whichever history exists.** `clear_history`
  (`src/buffer.rs:559-566`) clears the v0.1 `undo`/`redo` stacks **and**,
  under `#[cfg(feature = "crdt")]`, calls `crdt.clear_undo_history()` —
  because CRDT mode bypasses the v0.1 stacks entirely. Consequence for
  §14: the `crdt`-featured `dired_acceptance` run is not a formality
  here; it is the only run in which the CRDT half of Q#DR25 is live.

There is deliberately **no** Lua `set_read_only`
(`src/lua_bindings/mod.rs:3074-3078` states the reason: it would let a
caller lock a buffer with no way to refresh it). Pairing the lock with
the write **is** the primitive, which is why adoption is a swap of
`paint`, not an addition to it.

And it does **not** replace `set_round_trip_input`. The handoff is
explicit that the protection is layered across two copies: rope-level
`read_only` refuses the op at the daemon, while round-trip input stops a
semantic frontend applying optimistically to its **own mirror**, which a
daemon-side refusal arrives too late to prevent. `dired.lua:516` stays.

### How a directory reaches dired, after #182 (N2)

Rev 4 predates Journey Stage 1a. Three facts a Stage 2 implementer will
otherwise get wrong:

- **`resolve_target_buffer` resolves a directory before it loads.** The
  `ResolvedTarget::Directory { path }` arm is first
  (`src/editor_core.rs:964-970`), ahead of `get_or_load_buffer`, and its
  `path` is **normalized** — the type's own doc says so
  (`:118-130`) and warns that this is not free, because normalization
  otherwise happens inside `set_buffer_path` (`:889-890`), which never
  runs on this arm. **A handler keying state by path gets the canonical
  form**, which is what makes dired's `handle_for_path` dedup agree with
  it. Stage 2's reconciliation must normalize on the same seam or it
  will miss dired handles.
- **Dired is a replaceable fallback slot, not a hook subscriber.** The
  chain is the `path.open-directory` hook (`builtin/hooks/default.lua:65`),
  and dired registers through `pmacs.path.set_directory_handler`
  (`dired.lua:736-738`; binding at `src/lua_bindings/mod.rs:3679`,
  readable back as `pmacs.path.directory_handler`, `:3677`). The
  module's own comment states the reason: `HookRegistry::add` only
  appends and builtins load before `init.lua`, so a dired subscription
  would always claim first and no user listener could ever win.
  **Setting the slot to `nil` disables directory opening entirely**, and
  `journey_unclaimed_directory_starts_successfully_with_a_status` pins
  that this is a status message rather than a failure. Nothing in Stage
  2 may assume dired is what opened a directory.
- **`open_directory` now commits under a captured destination, and that
  scope refuses an `await`.** The mutating half of `open_directory` is a
  `commit()` closure (`dired.lua:656-693`) run through
  `pmacs.window.commit_to` when `opts.dest` is present (`:703-707`); the
  `read_listing` await is deliberately **outside** it, and the comment
  at `:653-655` records why: awaiting inside a commit is refused
  (Q#JR14b), because a yield would restore the scope while the coroutine
  is still parked. `journey_acceptance.rs`'s
  `commit_to_refuses_an_await_and_restores` pins it. **§9's
  serialize-and-await batch therefore cannot run inside a commit
  scope** — see §9.

### The journey ratchet (N3)

`tests/journey_acceptance.rs` — **24** `#[test]`s, 0 `#[ignore]`, 0
feature-gated, 2 under `#[cfg(target_os = "linux")]`. Its module doc at
`:12` is the rule: *"**This file is a ratchet: stages add rows, none
removes them.**"* Two further disciplines it states, both of which bind
Stage 2's own acceptance:

- **Drive the real entry point** (`:16-19`): "a directory arm with no
  production caller passes every direct-call test". The suite goes
  through `EditorState::open` / `open_directory_target`
  (`src/editor.rs:999`), never `resolve_target_buffer`.
- **Pump to quiescence, never to a frame count** (`:20-22`), because
  every listing is worker-dispatched. Stage 2's batches are more
  worker-dispatched still.

Seven rows assert on dired directly, including that a failed listing
leaves **no** `*dired:` buffer behind, that a declining resolver chain
still falls back to dired, and that `q` returns to the *destination*
window's origin buffer. **The GPU row is in a different file** —
`tests/gpu_invocation_acceptance.rs`'s
`public_gpu_directory_target_reaches_dired_and_leaves_the_daemon_usable`,
added by #183 through a real daemon child process. Stage 2 must keep
both green and must not remove a row from either (§14).

### The typed-edit chain, and why Stage 2 stays outside it (N4)

Dired participates in neither `pmacs.typed_edit` nor
`buffer.after-edit`: a grep of `dired.lua` for
`typed_edit|after-edit|after_edit` returns **0**, and it registers no
`pmacs.hook.add` at all. Nor does the generated-buffer write drag it in
— `set_generated_contents`'s binding runs `notify_buffer_edit_to_windows`
and nothing else, and `EditorCore::notify_buffer_edit`
(`src/editor_core.rs:1814-1828`) runs no Lua hook. **So Q#DR25 does not
put dired writes on the chain**, which is the answer to the obvious
worry about N1 and N4 interacting.

What *does* inherit from the chain is §5's two new hooks, because they
are new fan-outs. The relevant facts:

- `resource.renamed` / `resource.deleted` will be declared in
  `builtin/hooks/default.lua`. The existing edit hook there is
  `all-must-succeed`, and **`run_all_must_succeed` (`src/hook.rs:332`)
  iterates every callback and collects errors — it does not abort the
  fan-out.** A reconciliation subscriber may therefore not rely on a
  raising peer to stop the sequence, and §5's ordered LSP teardown must
  be internally ordered rather than ordered-by-registration.
- **A chain consumer cannot both edit and let a later consumer act
  meaningfully**, because the record handed to consumers is a copy taken
  before any ran (`builtin/runtime/typed_edit.lua:132`, `:145-159`).
  The analogue for §5 is direct: `resource.renamed` carries **paths**,
  not a rebind list, and a subscriber that re-reads editor state sees
  whatever earlier subscribers did. §5's LSP and dired subscribers are
  independent by construction — one touches URI-keyed stores, the other
  touches handles — and this framing asserts that independence rather
  than assuming it.
- **`buffer.after-edit` fan-outs nest** (`builtin/runtime/lean_input.lua:232-241`).
  Stage 2 fires no `buffer.after-edit`, so this does not bind directly;
  it is recorded because the reconciliation runs inside
  `pmacs._async._tick`, and a subscriber that edits a buffer there is
  one `pmacs.hook.run` away from re-entering a fan-out it is inside.

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

### 3.1 The listing becomes a genuinely immutable generated buffer (Q#DR25, new in rev 5)

Stage 1 protected the listing with an erroring intercept over a writable
rope, which was the best available idiom when #165 landed. #178 replaced
that idiom. Stage 2 adopts the replacement, in `dired.lua` only:

- `paint` becomes
  `pmacs.buffer.set_generated_contents(handle.buf, render_text(handle))`,
  dropping the `bypass_intercept` replace entirely.
- `claim_handle`'s `pmacs.buffer.add_intercept` (`dired.lua:509-511`)
  is **removed**. It becomes redundant: the rope-level `read_only` the
  primitive asserts refuses ordinary edits, undo, redo, and remote CRDT
  imports alike, which is strictly more than the intercept refused.
- `pmacs.buffer.set_round_trip_input(buf, true)` (`:516`) **stays**, per
  §2 — the two protections cover different copies and neither implies
  the other.

**Why this belongs in Stage 2 rather than a standalone fix.** It is a
three-line change to one file with no Rust at all, and it touches the
exact function every Stage 2 operation calls. Landing it separately
would mean two PRs racing on `paint`. It goes at the **head of 2b**
(§10), before the mark layer, so that every mark-layer acceptance
exercises the new write path rather than the old one. If the user would
rather see it land on its own — it is, after all, a live defect on a
step-3 journey surface — it detaches cleanly and 2b rebases onto it;
this framing states the preference, not a constraint.

**The acceptance trap, stated because it would otherwise be missed.**
`tests/dired_acceptance.rs`'s
`dired_buffer_is_read_only_and_round_trips_input` (`:969`) asserts
`status(&s).contains("read-only")`. `BufferError::ReadOnly` renders as
``buffer `{name}` (id {id:?}) is read-only`` (`src/buffer.rs:1794`), so
**that test passes both before and after the swap** — it has no bite
against Q#DR25 and must not be mistaken for coverage of it. The pin that
bites is a new one: **`M-x buffer.undo` on a dired listing leaves the
listing intact** (§13 item 48), which fails against `main` today.

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
*between* the last revert and the operation, and §13 item 40 pins that
the batch reports it rather than silently shrinking.

---

## 5. Rename reconciliation (Q#DR14, superseding Q#DR5's rebind)

The parent's decision — fix it at the primitive, in the main-thread
drain, unconditionally on success — is kept. Rev 1's *scope* was wrong:
it updated `Buffer.file_path` and nothing else, leaving four other owners
of the same path stale (F1). A rename is not a buffer-field update; it is
a **transaction across every consumer that holds the path**.

### The owners, all verified

Rev 4 listed five. The re-scout confirms all five and finds a **sixth**
(W5); two further owners are *persisted* rather than in-memory and are
named in §11 rather than fixed here.

| # | Owner | Held as | Stale after a rev-1 rebind |
|---|---|---|---|
| 1 | Buffer path | `Buffer.file_path` | fixed |
| 2 | Buffer **name** | `Buffer.name` | **yes** — `set_buffer_path` never calls `set_name`, which documents itself as for "save-as and rename operations" (`buffer.rs:453`). Statusline and buffer list keep the old filename |
| 3 | LSP attachment | `rec.uri`, cached per **buffer** in the `attachments` table (`lsp.lua:340`, record built at `:861-868`) | **yes** — read at **57 lines** in `lsp.lua` (W4), among them didChange (`:418`), semantic tokens (`:799-813`), the diagnostic attach (`:1039`), signature (`:1102-1108`), definition (`:1922-1937`), references (`:2031-2047`), all firing at the old URI. Buffer-keyed, so **one** rebind reaches all 57 |
| 4 | dired handles | `handle.path` in Lua; the buffers are **pathless** | **yes, and unreachable** — no buffer-keyed rebind can ever find them |
| 5 | Workspace-edit origin | `origin = active_buffer_path()` (`lsp.lua:1338`) — a **string** (`:471-473`) | **yes, and it materializes a phantom** — `find_or_open(origin)` (`:1352`) on a renamed-away path hits `resolve_target_buffer`'s `NotFound` arm, which creates an empty path-backed buffer (`editor_core.rs:980-989`) and selects it. **No transaction can fix this one**, because the stale value is a captured Lua local, not editor state (G1) |
| 6 | **Lean file progress** (NEW, W5) | `M.file_progress[uri]`, a **URI-keyed Lua module table** (`lean.lua:646`, written `:651`) | **yes, and `forget_uri` cannot reach it** — it is in no Rust store. Populated from `$/lean/fileProgress`; its own comment says Stage 5's goal view reads it to tell "no goals" from "not done yet". Reachable only from a **`resource.renamed` Lua subscriber**, which is an independent argument for the hook |

**Owner 6 is the load-bearing addition, and not because Lean matters
here.** It is evidence that the Rust-side `forget_uri` (below) is
structurally incapable of being complete: any package may key state by
URI in its own module table, and the LSP manager will never know. The
hook is not a convenience for dired — it is the only mechanism that
scales, and owner 6 is the first case that proves it outside dired.
`lean.lua`'s subscriber is **not** in Stage 2's scope (Lean's arc owns
it); what Stage 2 owes is a hook whose contract makes writing one
possible, which is why the hook carries `(old, new)` paths.

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
`run_hook_if_defined` (`mod.rs:1623`) fires Rust-side, `pmacs.hook.run`
(`:6005`) fires Lua-side — but **no rename or delete hook exists today**;
the whole set is `buffer.{after-edit,after-load,after-save,after-switch,
before-save,save,self-insert}`, `editor.before-quit`,
`frontend.detached`, `process.after-tick`. Stage 2 adds this one.

The hook carries the **paths**, not the rebind list, precisely because
dired's buffers are pathless: a path-keyed consumer must be able to
reconcile from `(old, new)` alone.

#### The LSP subscriber, in full (G3, corrected H2)

Rev 2 said "recompute `rec.uri` and didClose/didOpen". Rev 3 added five
stores. Both were sized from a partial inventory: `LspManager`
(`src/lsp.rs:741-830`) holds **fourteen** URI-bearing store families, and
two more things keyed the same way.

**The complete inventory**, from the struct itself:

| # | Field | Keyed by |
|---|---|---|
| 1 | `diag_store` | uri |
| 2 | `completion_store` | (server, uri) |
| 3 | `hover_store` | (server, uri) |
| 4 | `signature_store` | (server, uri) |
| 5 | `definition_store` | (server, uri) |
| 6 | `locations_store` | (server, uri, **kind**) — references / declaration / typeDefinition / implementation |
| 7 | `symbol_store` | **scope**-keyed — documentSymbol *and* workspace symbol |
| 8 | `document_highlight_store` | (server, uri) |
| 9 | `formatting_store` | (server, uri) |
| 10 | `rename_store` | (server, uri) |
| 11 | `prepare_rename_store` | (server, uri) |
| 12 | `code_action_store` | (server, uri) |
| 13 | `inlay_hint_store` | (server, uri) |
| 14 | `semantic_token_store` | (server, uri) — plus the retained raw int stream and `result_id` |

Plus **`documents: HashMap<(LspServerId, String), String>`** (`:810`), the
latest full text per `(server, uri)` — which is what `didChange` diffs
against, so a stale entry under the old URI is a *correctness* problem,
not just a leak. And **`pending_routes`** (`:797`), whose `ResponseRoute`
variants carry the URI.

**The route arithmetic, corrected (W3).** `ResponseRoute`
(`src/lsp.rs:835-859`) has **15 variants**, of which **14 carry a
`uri`**: `Completion`, `Hover`, `Signature`, `Definition`, `Formatting`,
`Rename`, `PrepareRename`, `CodeAction`, `InlayHint`, `SemanticTokens`,
`SemanticTokensDelta`, `Locations { uri, kind }`, `DocumentSymbol`,
`DocumentHighlight`. The fifteenth is **`WorkspaceSymbol { query }`**,
which carries **no URI at all** — its own comment explains that the
query stands in for the doc URI in the supersede key
(`src/lsp.rs:1887-1888`). There are **16** insert sites, every one in a
`request_*` method, spanning `src/lsp.rs:1683-2132`; **15 of the 16
insert a URI-bearing route**, the two `SemanticTokens` inserts coming
from `request_semantic_tokens` (`:2073`) and
`request_semantic_tokens_range` (`:2108`), and the sixteenth being
`request_workspace_symbol` (`:1889`). *Rev 4 said "fifteen insert
sites"; the fifteen was a correct count of the URI-bearing ones and an
incorrect count of the sites.* **The purge predicate must therefore
answer for a variant with no `uri` field** — `WorkspaceSymbol` is
retained unconditionally, because a workspace-symbol query is not scoped
to any document and a rename does not invalidate it.

**And that is why "clear the stores" is not enough** (H2): a response
already in flight when the rename happens routes on arrival and
**repopulates the old key after the clear**. Clearing without purging
in-flight routes is a race that reintroduces exactly the state it removed.

**One manager-level method, `forget_uri(sid, uri)`**, doing all of it —
because fourteen call sites at the Lua layer is how one gets forgotten.
Its shape is modelled on the **server-scoped** teardowns that already
exist. **There are two of them, and rev 4 named neither correctly (W2):**
`LspManager::start_generation` (`src/lsp.rs:1307-1345`, the restart-
generation flip) and `LspManager::forget` (`src/lsp.rs:3015-3042`, the
terminal-state removal). There is no `fn restart`. Both do the same
three things one axis over:

1. **Purge `pending_routes`** whose route carries this URI —
   `retain`, mirroring `start_generation`'s `:1324`
   `retain(|(sid, _), _| *sid != id)` and `forget`'s `:3028`.
   Per W3, the predicate must retain `WorkspaceSymbol` explicitly.
2. **Drain-cancel their awaiters — and this is where rev 4 pointed at
   the wrong function (W1).** `pending_external` (`:804`) holds the
   `Handle:await()` side, and the contract at `:801-803` is explicit
   that it is *"drained-cancelled wherever `pending_routes` is purged"*.
   The sweep to model on is **`drain_external_cancelled`
   (`src/lsp.rs:1561-1576`)** — server-scoped and **unconditional**,
   settling every awaiter for `sid` cancelled. It is **not**
   `drain_cancelled_externals` (`src/lsp.rs:1596-1645`), which despite
   the near-identical name is a per-tick, per-awaiter sweep that removes
   only awaiters whose `CancellationToken` was flipped or which outlived
   `request_timeout`. **A rename flips no token**, so modelling on the
   second yields a drain that silently does nothing and leaves any
   coroutine awaiting against the old URI parked forever.
   Note that **neither existing sweep is URI-scoped** — both range over
   `sid`. `forget_uri` needs a new shape: collect the `rid`s whose
   `pending_routes` entry carries the old URI, then settle
   `pending_external[(sid, rid)]`'s awaiters cancelled via
   `runtime.complete_external_cancelled`, exactly as
   `drain_external_cancelled` does per key. The route→awaiter join is
   the `rid`; there is no other index.
3. **Clear all fourteen stores plus `documents`** for the old key. Each
   store already has a keyed `clear` (`diag.rs:262`, `hover.rs:160`,
   `completion.rs:331`, `semantic_tokens.rs:263`, …). Note the two
   irregular keys: `locations_store` is **kind**-keyed, so all four kinds
   must go; `symbol_store` is **scope**-keyed and holds workspace symbols
   too, so only the document-scoped entry is dropped — the same
   asymmetry that makes `WorkspaceSymbol` route-exempt in step 1.

**Worth stating because it is surprising, and it is now true twice:**
*neither* server-scoped teardown clears the fourteen result stores.
`start_generation` clears `deferred_notifications`, `pending_routes`,
`documents`, and drains externals; `forget` clears `pending_routes`,
`documents`, externals, `status_tracker`, and `project_servers`. Rev 5
does not change that (it is a separate pre-existing question about
whether stale results should survive a restart), but it does mean
`forget_uri` has **no** precedent to copy for the store half, only for
the route/document/drain half.

Then the ordered sequence, per attachment:

1. **Flush any pending `didChange`** for the old URI, so the server is not
   left with an edit it can no longer attribute.
2. **`didClose` the old URI** — which removes the open-document
   registration and nothing else.
3. **`forget_uri(sid, old)`** — the purge/drain/clear above.
4. **Re-run `ensure_server`.** Since #161 affinity keys on the detected
   project root, a rename *across roots* needs a **different server**;
   same-root renames reuse. When the server changes, step 3 runs against
   the **old** server and `didOpen` goes to the new one.
5. **`didOpen` the new URI** with the buffer's current text and a fresh
   version.
6. **Re-root the diagnostic views** (below).

#### Re-rooting the diagnostic views: the seam, chosen (H3)

`DiagnosticView.uri` is **set once at construction** and the field's own
doc anticipates exactly this: *"M5 may add re-rooting if a buffer is
renamed"* (`src/diag.rs:456-457`). Rev 3 left it as "either a `set_uri` or
tear down and re-attach", which round 3 rightly rejected: neither option
explained how **passive** windows are reached, and the field is private
while `View` has **no downcast**, so an outside caller cannot reach it at
all. Two further facts make the naive options worse:

- **`_attach_view` reaches only the active window.** It takes
  `core_borrow.active_window_mut()` and **errors** if that window is not
  showing the buffer (`lua_bindings/diag.rs:218-224`). So a
  re-attach-per-window loop cannot be driven from Lua, and a passive split
  never got its overlay from this path in the first place.
- **A remove-and-re-push loses composition order.** Overlays are an
  ordered `Vec<Box<dyn View>>` (`window.rs:365`) merged in sequence, so
  re-pushing puts the diagnostic underline at the end of the stack rather
  than where it was.

**The seam: a `View::rename_resource` hook.** `View` (`src/view.rs`)
already carries exactly this family of cross-window overlay hooks, added
by #113 round 6 for the same class of problem — `overlay_identity`, whose
own doc says it "lets disposal remove **every window copy**", and
`clone_for_split`. Rev 4 adds a third:

```rust
/// Retarget this overlay from `old_uri` to `new_uri` after a resource
/// rename. Default: no-op --- a view that renders nothing URI-keyed is
/// unaffected. Mutates in place, so the overlay keeps its position in
/// the window's composition order.
fn rename_resource(&mut self, _old_uri: &str, _new_uri: &str) {}
```

`DiagnosticView` overrides it to swap its own private field when the URI
matches. The field **stays private**; no downcast is needed; and any
future URI-bearing overlay opts in by overriding the same hook instead of
growing another special case.

**The sweep has precedent too.** Overlay disposal already walks every
window and acts on identity:

```rust
for win in core.windows.values_mut() {
    win.overlays.retain(|v| v.overlay_identity() != Some(id));
}
```

(`lua_bindings/mod.rs:2044-2046`.) The rename sweep is the same traversal
with `retain` replaced by a call to `rename_resource` — so **every** window
showing the buffer is reached, active or passive, in one pass, and order is
preserved because nothing is removed or re-pushed.

**One thing this deliberately does not fix.** A passive split that never
received a `DiagnosticView` still has none — `_attach_view`'s
active-window-only restriction is a **pre-existing** gap (`ensure_overlay`
+ `clone_for_split` cover the split-from-an-attached-window case, not the
attach-while-passive case). Renaming cannot re-root an overlay that was
never attached, and pretending otherwise would make acceptance 30 pass for
the wrong reason. Named in §11.

Acceptance 30 is correspondingly stronger: **diagnostics present before
the rename**, **at least two windows** showing the buffer, and afterwards
only the **new** URI's diagnostics are visible and countable in *both*,
with the old URI's store empty.

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
  (`src/buffer.rs:453`). Nothing new is being invented; an existing,
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
`find_or_open(origin)` (`lsp.lua:1338`, `:1352`). No amount of
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
(`async.lua:401-421`). So doing the work inside `_tick`'s Rust closure is
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
(`mod.rs:3313-3341`) deletes, then kills the buffer via
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

### Checked twice — and still only best-effort (G4, narrowed H1)

`buf:is_modified()` is exposed to Lua (`mod.rs:1261`), so dired decides
this itself. It checks **twice**:

- **Before the confirm**, so the prompt states the skip up front:
  `Delete 3 entries? (1 has unsaved changes and will be skipped) (y/n) `.
- **Again immediately before dispatching each removal.** The prompt is not
  modal against the world: another frontend attached to the same daemon
  can edit a buffer while it is open, and the batch is serialized (§9), so
  there is a real window between the answer and the *n*-th dispatch.

**And that is where the guarantee stops. It is a pre-dispatch check, not a
lock (H1).** Rev 3 said "immediately before each syscall", which was wrong
about where the boundary is: `pmacs.fs.remove` **dispatches a worker**, so
the syscall happens later, on another thread, in `remove_blocking`. An
edit landing between dispatch and `remove_file`/`remove_dir` is not
detected by anything, and **no acceptance test can pin an interval that is
not closed** — which is why rev 3's acceptance 20 (edit before `y`) could
never have detected this.

So the promise is stated at its real strength, the same way G6 forced for
`R`: **a TOCTOU-bounded pre-dispatch refusal.** What survives the race is
worth being precise about:

- the **buffer** survives with its contents — `reconcile_delete` keeps a
  modified buffer rather than killing it, and that half *is* robust,
  because it runs at drain time on whatever state exists then;
- the **file** is gone.

Which means **the orphaned-modified-buffer residue rev 3 scoped to the LSP
path applies to dired too**, on this narrow race. §11 carries it as one
deferral, not two.

Closing it properly needs one of two things this stage should not invent:
a **reservation** (some lock or generation the worker re-validates before
the syscall), or a **synchronous** delete path. The synchronous option is
tempting because `apply_resource_op` already does main-thread
`std::fs::remove_*`, and it would remove the need for the delete harvest
entirely — but it puts N blocking syscalls on the main thread for an
N-entry batch, and a fire-and-forget `pmacs.fs.remove` from any *other*
caller would still need the harvest. Named in §11.

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
> region" otherwise (`:184-194`). Rev 3 adds
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
  (`dired.lua:400-404`), the rule #165's review round produced.
- **No part of a batch may run inside `pmacs.window.commit_to`** (new in
  rev 5, from N2). The commit scope **refuses an `await`** (Q#JR14b) —
  `commit_to_refuses_an_await_and_restores` pins the raise, whose
  message contains both `"cannot await inside"` and `"commit_to"` — and
  a batch is defined by awaiting each op in turn. Stage 1 already solved
  the same problem the same way: `open_directory` does its
  `read_listing` await **outside** the commit and puts only the
  non-yielding mutation inside (`dired.lua:640-655`). Stage 2's
  operations follow that shape — await the whole batch first, then, if a
  destination was captured, do the repaint and re-seat inside a commit.
  Stage 2's operations are all initiated from an already-active dired
  buffer rather than from a captured destination, so in practice
  `opts.dest` is nil on these paths and the ambient rule above governs;
  the constraint is stated because the *shape* must not be copied
  wrongly if that ever changes.

---

## 10. Staging: three PRs (Q#DR17, revised in rev 4)

**Rev 4 takes the further cut, as directed in round 3.** Rev 3 already
recorded that 2a's Rust had outgrown "one narrowly-scoped change"; round 3
called it, and the three rounds of findings on the reconciliation half are
the evidence — every one of G1, G3, H1, H2, and H3 was about the
transaction, not about marks.

**Rev 5 keeps the three-PR cut unchanged** and adds Q#DR25 to the head
of 2b. The cut was re-examined against the re-scout and still holds: 2a
is still substrate-only, 2b is still the dired surface, 2c is still the
three additive primitives, and Stage 3 is still wdired. Nothing that
landed in the 153 commits moves work across those lines — #178 lands
inside 2b (it is dired-local), #182 changed the *shape* of dired's entry
point but added no Stage 2 work, and #179/#181 added none.

| | **2a — reconciliation** | **2b — marks and operations** | **2c — new primitives** |
|---|---|---|---|
| User-visible surface | **none** | `m u U t d x D R w M`, plus a listing that survives `M-x buffer.undo` | `+ C`, recursive delete |
| Rust | `reconcile_rename`, `reconcile_delete`, `TickOutcome`, `PendingJob` paths, `forget_uri` (14 stores + `documents` + route purge + URI-scoped drain), `View::rename_resource` + the window sweep, `apply_resource_op` (rename **and** delete arms), `apply_workspace_edit` origin, `pmacs.buffer.set_name` | `pmacs.killring.push` | `mkdir`, `copy`, `remove_dir_all`; `JobKind` 12 → 15 |
| Lua | the two hook subscribers in `lsp.lua` | **Q#DR25 (`paint` → `set_generated_contents`, drop the intercept)**, then all of `dired.lua`'s mark/op layer, plus `minibuffer.lua` | two ops |
| Config keys | none | none | `dired.recursive-deletes` |
| Acceptance | items 25–38 | 1–24, 39–41, **48–49** | 42–47 |

**Why 2a first, with no dired surface at all.** It is a self-contained
substrate correctness fix that stands on its own merits: it closes a path
where an LSP-authored delete **destroys unsaved work today**
(`mod.rs:3313-3341`), and one where renaming the active file through a
workspace edit **materializes a phantom buffer** (`lsp.lua:1352`). Neither
needs dired to be worth fixing, and neither is dired's fault. Landing it
alone also means the LSP lifecycle work — a fourteen-store inventory, an
in-flight route purge, an awaiter drain, and a new `View` hook swept across
every window — gets a review round of its own rather than sharing one with
a mark column.

**Why marks second rather than first.** The mark layer is the more visible
work and the more pleasant to review, which is exactly the argument for
*not* putting it in the same PR as the substrate change: it would absorb
the attention. It also genuinely depends on 2a — `R` is unsafe on a
directory without the transaction, and `D`/`x` are unsafe on a visited file
without `reconcile_delete`.

**Why 2c last and separate.** Three additive fs primitives with their own
overwrite and lstat-safety semantics (§8, §13 items 42–47). Nothing in 2b
needs them; `remove` already covers files and empty directories (C5).

**The cost, stated.** Three PRs is three review cycles instead of two, and
2a ships nothing a user can see — its acceptance is entirely
substrate-level (a no-await rename, a two-window diagnostic re-root, an
LSP store inventory). That is the trade round 3 accepted, and the reason it
is worth it: a bug in 2a is a silent data-loss bug, and those are the ones
that deserve an undivided reviewer.

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
- **A delete can still orphan a modified buffer** — one deferral, two
  paths (H1). On the LSP path this is by design (the user accepted the
  refactor); on dired's it is the residue of the open
  dispatch-to-syscall interval. Stage 2 stops both from *destroying* the
  buffer; the file still goes. Closing dired's half needs a
  **reservation** the worker re-validates, or a **synchronous** delete
  path — the latter would also make the delete harvest unnecessary for
  dired, but puts N blocking syscalls on the main thread and leaves every
  other `pmacs.fs.remove` caller unprotected. Closing the LSP half means
  deciding what a partially-applied workspace edit does.
- **A passive window that never received a `DiagnosticView` still has
  none** (H3). `_attach_view` takes `active_window_mut()` and errors
  otherwise (`lua_bindings/diag.rs:218-224`); `ensure_overlay` +
  `clone_for_split` cover split-from-attached, not attach-while-passive.
  Pre-existing, and rename cannot re-root an overlay that was never
  attached.
- **The server-restart teardown does not clear the fourteen result
  stores** (`lsp.rs:1307-1345` clears routes, documents, and deferred
  notifications only). Pre-existing; whether stale results should survive
  a restart is its own question, and `forget_uri` deliberately does not
  answer it.
- **The rooturi sink's weak wait predicate** (`m4_acceptance.rs:5487`) —
  same class as the config-sink race fixed in #174, not observed failing,
  and the obvious fix would trade a precise regression diff for a vague
  timeout. Needs a record terminator in the fake server first.
- **Recursive copy** — 2c refuses directory sources; a real `copy -r`
  primitive is separate.
- **The other three non-adopters of `set_generated_contents`** (new in
  rev 5). Q#DR25 closes dired's quarter of the handoff §4 / `COHERENCE.md`
  §14 inventory. The remaining three — listview panels
  (`builtin/runtime/listview.lua:60-61`), `compile.lua`'s `ensure_slot`
  serving `*compilation*` **and** `*shell-command*`, and the independent
  `*search-results*` panel (`builtin/commands/default.lua:869`) — stay
  emptiable. Listview is the other cheap half (whole-buffer replace);
  the three appending writers need a **streaming variant of the
  primitive that does not exist**, which is why this is a lane and not a
  rider. Note the handoff's warning: `ensure_slot` does **not** cover
  `*search-results*`, and `*workers*`/`*help*`/`*buffer-list*` are
  generated but do not use this idiom, so they are not in scope either.
- **Two persisted path owners, named not fixed** (rev 5, W5).
  `saveplace.lua` keys its places file by path (`load_places`, `:31-45`;
  `restore_active`, `:69-79`) and `recentf.lua` keeps a path MRU list
  (`record`, `:36-46`). Neither is an in-memory owner — both rebuild
  their index from `pmacs.state` on every call — so a rename leaves a
  stale *line on disk*, not stale editor state. The consequence is
  bounded and self-correcting: entries are capped and LRU-evicted, and
  the worst case is a future unrelated file at the old path restoring a
  wrong cursor. Reconciling them would mean teaching the reconciliation
  transaction to rewrite persisted state, which is a persistence-arc
  question. A `resource.renamed` subscriber in each is the cheap fix
  whenever someone wants it.
- **`lean.lua`'s `M.file_progress` is not re-rooted** (rev 5, owner 6).
  Stage 2 supplies the hook that makes a subscriber possible and does
  not write one — Lean's arc owns that file, and Stage 5's goal view is
  the consumer that would notice.

---

## 12. Bets

- **B1.** **2b** needs **no** new `pmacs.fs` op. Falsified if any of
  `m u U t d x D R w M` cannot be built on the existing five. *(Rests on
  C5, which was read off `remove_blocking` directly.)* *(Rev 5: rev 4
  wrote "2a" here, which contradicted its own §10 table — 2a ships no
  dired surface at all, so it cannot need an op for a key it does not
  bind. Corrected label; the bet is unchanged.)*
- **B1b.** The two new hooks need no new hook machinery —
  `run_hook_if_defined` (`mod.rs:1623`) and `pmacs.hook.run` (`:6005`)
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
- **B6** *(new in rev 5, N1)*. Q#DR25 needs **no Rust**: the primitive,
  its Lua binding, and the fan-out all exist
  (`src/buffer.rs:545`, `src/lua_bindings/mod.rs:3079-3095`). Falsified
  if adopting it requires a new binding, a `set_read_only` exposure, or
  any change outside `builtin/runtime/dired.lua`. *Not obvious in one
  respect:* dropping the intercept removes the only thing that currently
  produces dired's refusal **message**, so if any Stage 1 acceptance
  depends on the intercept's exact wording rather than on the substring
  `read-only`, this bet fails. §3.1 checked the one test that looked
  like a risk and it does not.
- **B7** *(new in rev 5, N3)*. The journey ratchet stays at **≥ 24**
  rows in `tests/journey_acceptance.rs` and keeps its GPU row in
  `tests/gpu_invocation_acceptance.rs`, with no row weakened. Falsified
  by any deletion, `#[ignore]`, or assertion relaxation in either — and
  the ratchet's own doc says a green test that cannot fail must be
  deleted rather than kept, so "still green" is not the bet; "still
  biting" is.

---

## 13. Acceptance

**2b — marks** *(stage labels follow §10's three-PR cut)*

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

**2b — operations**

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

**2b — deletion policy (§6)**

17. Deleting an **unmodified** visited file kills its buffer.
18. Deleting a **modified** visited file is **refused**; the buffer
    survives with its contents, and the file is still on disk.
19. The confirm prompt **states the skip before the user answers**, not
    after.
20. **A buffer modified after the prompt appears but before `y`** is
    skipped and reported: the check is re-run before each **dispatch**,
    because another frontend can edit during the prompt and the batch is
    serialized. *(An implementation that checks only once, up front, fails
    this.)* **This is the whole of the guarantee** — the dispatch-to-syscall
    interval is open (H1) and deliberately has no test, because no test can
    pin an interval that is not closed. What §13 item 24 pins instead is
    the half that *is* robust: the modified buffer survives.
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

**2a — reconciliation (§5, §6)**

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
30. **An attached LSP buffer with diagnostics present before the rename,
    shown in at least TWO windows** (H3): afterwards both windows render
    the **new** URI's diagnostics, the old URI's store is empty, and each
    window's overlay keeps its **position in the composition order** — not
    merely `rec.uri` updated (`DiagnosticView.uri` is set once at
    construction, and a remove-and-re-push would pass a one-window test
    while reordering the stack).
31. **The store inventory** (H2): after a rename, an entry that existed
    under the old URI in each of the fourteen stores plus `documents` is
    gone, and **a response already in flight at rename time does not
    repopulate the old key** — the `pending_routes` purge and awaiter
    drain, without which the clear is undone by arrival.
32. A rename **across project roots** re-runs `ensure_server` and the
    buffer ends up attached to a **different** server; a same-root rename
    reuses the existing one (#161's affinity key).
33. **An open dired buffer on the renamed directory** follows it: its
    `handle.path`, **its buffer name** (`*dired:<new path>*`), and
    `handle_for_path` dedup under the new path all move together (G2 —
    asserting `handle.path` alone would pass with the name still stale).
34. **The workspace-edit origin**: renaming the *active* file through
    the full `apply_workspace_edit` path leaves **no phantom empty
    buffer** at the obsolete path, and the user is returned to the
    **same buffer** (now under its new path). *(G1 — this is a change to
    the applier, which must capture the buffer handle; no reconciliation
    can reach the string it captures today.)*
35. When the origin buffer is **gone** after the edit, the applier
    restores nothing rather than falling back to the old path.
36. `apply_resource_op`'s rename finds a buffer whose stored path is
    normalized but whose op names it un-normalized (the `:3249` fix).
37. A **failed** rename reconciles nothing.
38. Additivity: `m8_1`, `m8_2`, `m8_3` at unchanged counts.

**2b — the shared helpers**

39. **`pmacs.minibuffer.confirm`** (Q#DR15, F5): an **empty `RET` does
    not call `on_yes`**; `y`, `Y`, `yes`, `YES` all do; `n` and arbitrary
    text do not. *(A typed-`n` test alone would not catch a completion
    source being reintroduced — the empty-`RET` arm is the one that
    detects it.)*
40. **Serialization** (Q#DR16, F5): in a batch of N mutations, the second
    is **not dispatched until the first has settled**. Asserted by
    observing at most one in-flight fs job at any pump step — *not* by
    the end state, which is identical if all N were dispatched at once
    and awaited afterwards.
41. A marked target that vanishes **between the last revert and the
    operation** is **reported**, not silently dropped (F5 — distinct from
    item 4's revert-time pruning).

**2c — new primitives**

42. `+` creates a subdirectory, which appears on the next listing.
43. `C` copies a file and preserves mode bits; refuses a directory
    source.
44. `C` with several marked entries requires an existing directory
    destination and refuses otherwise **before copying anything**.
45. `C` onto existing targets confirms **once** with the collision count;
    **declining copies the non-colliding entries and skips the rest**
    (F7).
46. `remove_dir_all` **unlinks a symlink-to-a-directory rather than
    traversing it** — pinned at the primitive, mirroring
    `remove_blocking`'s lstat guard (F7).
47. Recursive delete happens only with `dired.recursive-deletes` enabled
    **and** a confirm; disabled, the non-empty directory still fails.

**2b — the generated-buffer invariant (§3.1, Q#DR25, new in rev 5)**

48. **`M-x buffer.undo` on a dired listing leaves the listing intact.**
    Driven through `pmacs.command.invoke` (or the M-x path), not through
    a chord — the command is reachable with no binding, which is the
    whole point. *(This is the pin with bite: it fails against `main`
    today, where the listing is emptied.)* Its sibling asserts **redo**
    is equally refused, since `Buffer::redo` takes the same
    `ensure_writable` path.
49. **A repaint after the swap still reaches a displaying window.** Mark
    an entry in a dired buffer shown in **two** windows and assert the
    mark column changed in **both** — the fan-out obligation the
    handoff attaches to the primitive. *(An implementation that called
    `Buffer::set_generated_contents` from Rust and swallowed the `Edit`
    would paint stale ranges; the Lua binding discharges this at
    `mod.rs:3092`, and this pins that the Lua path is the one used.)*
    Run under **both** default and `crdt` features: `clear_history`
    clears loro's `UndoManager` only under `crdt`, so the default run
    does not exercise that half at all.

Note explicitly what does **not** count as coverage of Q#DR25:
`dired_acceptance.rs`'s existing
`dired_buffer_is_read_only_and_round_trips_input` (`:969`) passes both
before and after the swap, because `BufferError::ReadOnly` renders with
the substring `is read-only` and the test asserts
`status(&s).contains("read-only")` (§3.1).

**Bite obligations.** Each of these must fail against a stated mutation:

| Item | Mutation it must catch |
|---|---|
| 8 | `x` widened to consume `*` marks |
| 14 | `M`'s symlink refusal downgraded to a warning |
| 20 | the modified check run only once, before the prompt |
| 25 | the reconciliation moved to `_take_result` |
| 27 | `find_by_path`'s first match instead of every match |
| 28 | a string `starts_with` instead of a path-component prefix |
| 30 | `rec.uri` updated without re-rooting the diagnostic view — and a remove-and-re-push, which passes a one-window test |
| 31 | the store clear without the `pending_routes` purge, so an in-flight response repopulates the old key |
| 33 | `handle.path` updated without the buffer name |
| 34 | the applier restoring by path instead of by buffer handle |
| 39 | a completion source added to `confirm` |
| 40 | the batch changed to dispatch-all-then-await |
| 48 | `paint` reverted to `bypass_intercept` over a writable rope (i.e. `main` today) |
| 49 | the `Edit` returned by `set_generated_contents` swallowed instead of fanned out |

`dired.lua` is an existing file now, so `scripts/bite`'s
swap-over-`git show` mode applies — but per #165's lesson, **commit
before biting**. Items 30, 33, and 34 came from round 2 and item 31 from
round 3; each is a case where the previous revision's design would have
passed a weaker test. Note that **item 20 has no bite for the interval it
cannot close** (H1) — only for the check it does make.

## 14. Gates (per PR)

The standard suite from `CLAUDE.md`, plus what this work touches. **2a's
  gates are the widest of the three** — it changes `lsp.rs`, `view.rs`,
  `window.rs`, `editor_core.rs`, and `lua_bindings`, so every LSP suite is
  in its blast radius, not just the dired one:
`cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D
warnings` as its own step; `cargo test --lib` and `--lib --features
crdt`; `dired_acceptance` (default **and** `crdt`); **`m8_1`, `m8_2`,
`m8_3` at unchanged counts** (the additivity gate, B3);
`m4_acceptance -- --skip basedpyright`; **`lsp_multi_root_acceptance`**
(B3b's gate, omitted from rev 2's list); `PMACS_REQUIRE_GPU=1 cargo test
-p pmacs-gpu`; the isolated-`XDG_CONFIG_HOME` workspace sweep with
`--no-fail-fast`; `git diff --check`.

**Two gates rev 4 could not have listed, both required of every PR in
this stage (N3):**

- **`journey_acceptance` at ≥ 24 tests, all passing.** It is a ratchet
  by its own declaration, seven of its rows assert on dired, and its
  step-3 and step-5 rows run *through* the surface Stage 2 modifies.
  **Assert the count, not just the colour** — a row silently dropped is
  exactly what the ratchet exists to prevent, and Q#DR25 changes how
  dired's buffer is written underneath those rows.
- **`gpu_invocation_acceptance`**, which is where #183 put the GPU
  journey row
  (`public_gpu_directory_target_reaches_dired_and_leaves_the_daemon_usable`).
  It drives a **real daemon child process**, so it is in the blast
  radius of any change to how a dired buffer is written or protected —
  Q#DR25 makes the listing rope-level `read_only`, and a daemon-side
  refusal is exactly what that test's frontend would see. Note this row
  is **not** in `journey_acceptance.rs`; running only that file leaves
  the GPU half of the journey unpinned.

Also add **`typed_edit_chain_acceptance` (13 tests)** to 2b's list, not
because dired joins the chain — it does not (§2, N4) — but because
Q#DR25 changes a `buffer.after-edit`-adjacent write path and the chain
is the tree's most order-sensitive consumer of edits.

2b's `dired_acceptance` run is expected to **grow** by items 48–49 and
otherwise hold at its current **25**; a *drop* there means Q#DR25 broke
a Stage 1 pin rather than superseding one, and the two are not the same.

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
  fallback**, since no transaction can reach a captured Lua local.*)
  *(Completed in rev 4, H2/H3:* the LSP half is one manager-level
  **`forget_uri(sid, uri)`** covering **fourteen** URI-bearing stores plus
  `documents`, **plus a `pending_routes` purge and awaiter drain** — an
  in-flight response otherwise repopulates the old key after the clear;
  and the view half is a **`View::rename_resource`** default-no-op hook
  swept over `core.windows.values_mut()`, chosen over `set_uri` or
  re-attach because it reaches passive windows and **preserves overlay
  order by mutating in place**.*) (§5)
- **Q#DR15** Confirmation is `pmacs.minibuffer.confirm` in a new
  `builtin/runtime/minibuffer.lua`, with **no completion source**;
  affirmative is `y`/`yes` case-insensitively and everything else —
  including empty `RET` — is no. (§7)
- **Q#DR16** Batches **serialize** (await each op), a per-entry failure
  does not abort, successful marks clear while failed marks persist, and
  the listing reverts **once** at the end. (§9)
- **Q#DR17** *(revised in rev 4)* Stage 2 ships as **three** PRs, not
  two: **2a** the resource-reconciliation transaction with **no dired
  surface at all** (rename + delete reconciliation, `forget_uri` over
  fourteen stores, `View::rename_resource`, the applier fix,
  `pmacs.buffer.set_name`); **2b** the mark layer plus
  `m u U t d x D R w M` on the five existing fs ops; **2c**
  `mkdir`/`copy`/`remove_dir_all` with `+ C` and recursive delete. 2a
  first because a bug in it is silent data loss, and because every
  blocking finding across three rounds landed on it. (§10)
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
  modified check runs **before** the confirm *and again before each
  dispatch*, because another frontend can edit while the prompt is open —
  but it is a **pre-dispatch check, not a lock** (H1): `pmacs.fs.remove`
  dispatches a worker, so the dispatch-to-syscall interval stays open
  and the promise is stated at that strength, as G6 forced for `R`.
  Deliberately diverges from Emacs, which orphans the
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
- **Q#DR23** *(new in rev 4, H2; corrected in rev 5, W1/W2/W3)* The LSP
  rename teardown is **one manager-level `forget_uri(sid, uri)`**, not
  per-store calls at the Lua layer: fourteen call sites is how one gets
  forgotten. It purges `pending_routes` by URI, **drain-cancels the
  matching awaiters** (the existing contract at `lsp.rs:801-803` already
  requires this wherever routes are purged), and clears all fourteen
  stores plus `documents`, handling `locations_store`'s **kind** key and
  `symbol_store`'s **scope** key specially.
  *(Rev 5 corrects three facts underneath it, none of which change the
  decision but any of which would break the implementation:* the
  teardowns to model on are **`LspManager::start_generation`**
  (`lsp.rs:1307-1345`) and **`LspManager::forget`** (`lsp.rs:3015-3042`)
  — there is no `fn restart`; the drain to model on is
  **`drain_external_cancelled`** (`lsp.rs:1561-1576`), the unconditional
  server-scoped one, **not** the near-namesake
  `drain_cancelled_externals` (`lsp.rs:1596-1645`), which only reaps
  token-cancelled and timed-out awaiters and would therefore drain
  nothing on a rename; and the route purge must **retain
  `WorkspaceSymbol` unconditionally**, since it is the one
  `ResponseRoute` variant of fifteen that carries a `query` rather than
  a `uri`. Neither existing drain is URI-scoped, so `forget_uri`'s drain
  joins route to awaiter on the **`rid`**, which is the only index
  available.*) (§5)
- **Q#DR24** *(new in rev 4, H3)* Diagnostic re-rooting is a
  **`View::rename_resource(&mut self, old, new)`** default-no-op hook,
  joining `overlay_identity` and `clone_for_split` in the family #113
  added for cross-window overlay problems, swept over
  `core.windows.values_mut()` exactly as overlay disposal already is
  (`mod.rs:2044-2046`). Chosen over exposing `set_uri` (the field stays
  private, and `View` has no downcast) and over tear-down-and-re-attach
  (which loses composition order, and `_attach_view` cannot reach a
  passive window at all). (§5)
- **Q#DR22** *(new in rev 3, G5)* **`pmacs.killring.push(text)`** is
  exposed, with the semantics `copy()` already establishes for
  non-region text: push the entry, mirror to the OS clipboard, and
  **break the kill chain**. `push_entry` is private and `copy()` requires
  a region, so `w` has no surface without it. (§8)
- **Q#DR25** *(new in rev 5, N1)* **Dired's listing becomes a genuinely
  immutable generated buffer.** `paint` writes through
  `pmacs.buffer.set_generated_contents` instead of a
  `bypass_intercept` `buf:replace`, and `claim_handle`'s erroring
  intercept is **removed** as redundant — rope-level `read_only` refuses
  strictly more (ordinary edits, **undo, redo**, and remote CRDT
  imports). `set_round_trip_input` **stays**: the two protections cover
  different copies, and a daemon-side refusal arrives after a semantic
  frontend has already painted its own mirror. This is a correctness fix
  to **Stage 1**, not new Stage 2 surface: on `main` today `M-x
  buffer.undo` empties a dired listing, and dired is now a **step-3
  journey surface** (§0.5). It lands at the head of 2b because it is
  dired-local, needs no Rust, and touches the one function every Stage 2
  operation calls — but it detaches cleanly if the user prefers it
  standalone. The fan-out obligation is discharged by the existing Lua
  binding (`mod.rs:3092`); the history-clearing obligation covers the
  CRDT `UndoManager` too, which is why the `crdt` acceptance run is
  load-bearing rather than routine. (§3.1, §10, §13 items 48–49)

## 16. Branch and PR plan

Framing on `dired-stage2-framing`, kept after merge per the repo's
`-framing` convention. Then **three** implementation branches, each cut
fresh from `main` after the previous one merges — **not** stacked, since
each needs only a merged `main` and the arc has already paid for a stacked
retarget once (#104 → #105):

1. **`dired-stage2a-reconciliation`** — the transaction, no dired surface.
2. **`dired-stage2b-marks`** — the mark and operation layer.
3. **`dired-stage2c-primitives`** — `mkdir`/`copy`/`remove_dir_all`.

One feature, one branch, one PR; gates green before each; the ledger lane
and `docs/agent-handoff.md` §1 updated per their own protocols as each
lands.

**Naming 2a for the substrate, not for dired**, following #161's
precedent: its diff contains no dired code, and a cross-cutting
correctness fix to rename/delete reconciliation must not be reviewable
only as a dired feature. The PR body should lead with the two defects it
closes on `main` today — the LSP-authored delete that destroys unsaved
work, and the workspace-edit phantom buffer — because neither needs dired
to be worth fixing.

**Ledger note (corrected in rev 5, W6):** this framing branch
deliberately touches **only** this file. Rev 4 said the durable records
were held by open PR #169; **#169 merged** as `74301d1`. The current
holder is **PR #185** (`docs-landed-state-184`), which owns
`docs/active-work.md`, `docs/agent-handoff.md`, and `COHERENCE.md`. A
second edit to any of the three from here would conflict for no benefit,
so the Stage 2 lane goes in once #185 has landed. *(This is the ledger
contention treadmill the ops lessons name: with several PRs open, every
merge re-conflicts the rest in `docs/active-work.md`. Integrate late.)*

### Ownership warning: 2a must not run concurrently with Journey Stage 1b

**2a overlaps `src/editor_core.rs`, `builtin/runtime/lsp.lua`, and the
URI-keyed LSP state with other coherence work in flight.**
`COHERENCE.md` §20 names Journey Stage 1b as Priority 1's remainder —
compile defaults, **LSP spawn-failure surfacing**, bindings, and a
welcome buffer — and the LSP-failure half lands in the same files 2a
rewrites, while 1b's compile/binding half touches `editor_core.rs`. Two
branches editing `lsp.lua`'s attachment lifecycle at once is the
worst-case shape for this repo: the conflicts are semantic rather than
textual, so a clean `git merge` proves nothing.

**Neither stage may start until those files are assigned.** The
resolution is cheap if taken up front — 1b's LSP work is a *reporting*
change (surface the spawn failure with guidance) and 2a's is a
*lifecycle* change (teardown and re-attach on rename), so they can be
sequenced in either order provided only one is open at a time. Taken
late it is three merge rounds, which this arc has already paid twice.

Stage 2b and 2c carry no such overlap: 2b is `dired.lua` plus one
killring binding, and 2c is three additive `pmacs.fs` primitives.
