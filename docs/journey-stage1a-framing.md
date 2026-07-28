# Journey Stage 1a — open a directory, on one path

**Status: framing, rev 8 — APPROVED at rev 5; revs 6–8 record
corrections found during implementation and review of PR #182.**
**Serves `COHERENCE.md` §2 (the golden product journey), §19 (coherence
acceptance tests), §20 Priority 1.**

## 0. Revision history

- rev 1 (2026-07-26) — first framing. Scouted against `main` @ `d400f30`.
- rev 2 (2026-07-26) — review round 1. Q#JR2 withdrawn (its ground truth
  was false); destination pinning added; the resolver chain restructured
  around append-only hook registration; `ResolvedTarget` typed;
  `display_file`'s contract specified; the GPU-framing supersession named.
- rev 3 (2026-07-26) — review round 2. Two blockers, two contract gaps:
  - **Fail-closed was not failure-atomic** (§4.4). dired mutates handle
    state — claim, listing, `prev`, paint — *before* it ever attempts
    `display`, so rev 2's "does nothing" left a hidden buffer and could
    corrupt an existing handle's `prev`. **Per the review's decision, 1a
    now carries the destination-scope substrate**: a `commit_to`
    primitive that revalidates and enters the captured frontend's scope
    *before* any dired mutation, so the whole post-await commit — `prev`
    capture, claim, paint, display, seat — executes against the captured
    destination or not at all.
  - **Expected-buffer validation** (Q#JR14): the destination carries the
    buffer it was requested against, so a user who replaces the bootstrap
    buffer mid-listing is not overwritten by stale launch intent. Rev 2's
    window-only pin said launch intent wins; it should not, and B2's
    "before the user can act" was false (§8).
  - **Acceptance 6 was still vacuous**, and the blanket "each fails with
    the change reverted" rule cannot hold for preservation guards. §6 is
    split into new-behavior acceptances and preservation pins, each pin
    naming the targeted mutation that falsifies it (§6.0).
  - **Hook error policy specified** (Q#JR15): in a short-circuit hook a
    raise and a `false` both yield `proceed = false` (`hook.rs:299-323`);
    only `HookOutcome.errors` distinguishes them. An error now stops the
    chain *and* suppresses the fallback.
  - The fallback slot is described honestly as an **unowned singleton**
    (§0.5), not an "ownership-carrying registration".
- rev 4 (2026-07-26) — review round 3. Three substrate details and one
  inverted bite mutation:
  - **`InteractiveCommandOrigin` was the wrong mechanism, twice over**
    (§2.11). It does not scope the APIs rev 3 claimed — no-arg
    `pmacs.window.buffer()` reads `core.active_buffer_id()` directly
    (`mod.rs:12547`) and `move_to_line` mutates the core's ambient active
    window (`mod.rs:12703`) — so `prev` capture and cursor seating stayed
    ambient. And it is *authenticated interactive-command authority*:
    entering it would make dired's `paint` satisfy the pre-edit unfold
    guard (`mod.rs:1391`), `invoke_interactive`'s rotation (`:5400`), and
    terminal command context (`:8515`). Rev 4 uses a **separate scoped
    frontend override** that also swaps `core.active_frontend`, and
    `commit_to` does not touch the interactive origin (Q#JR14e).
  - **`dest` becomes nonconstructible userdata** (Q#JR14d). As a table it
    is shared across hook listeners, so an earlier listener could mutate
    the destination and decline — redirecting later listeners or dired —
    and any Lua could fabricate a valid triple.
  - **Preflight was missing replaceability** (Q#JR14f). Exact display
    also refuses a window dedicated to another buffer
    (`editor_core.rs:3566`), so a live destination holding its expected
    buffer could still refuse *after* dired claimed and painted — rev 2's
    hidden-buffer failure through another door.
  - **N8's falsifier was inverted** (§6.1). Both a claim and a raise give
    `proceed == false`, so keying the fallback on `proceed` alone is
    *correct*; `errors` decides the extra report, not the fallback.
- rev 5 (2026-07-26) — review round 4. The remaining predicate input and
  acceptance details:
  - **Replaceability now names the incoming buffer** (Q#JR14f). The
    shared predicate takes `Option<BufferId>` and serves all three existing
    consumers: exact display passes its requested buffer,
    `probe_display_target` passes its existing-buffer result, and
    `commit_to` passes `None` because dired's replacement does not exist
    yet. Thus a destination dedicated to its still-current bootstrap
    buffer is refused before dired mutates anything.
  - **N6c is executable:** the first listener catches the userdata
    mutation rejection and declines, the second verifies the token stayed
    unchanged and declines, and only the fallback commits.
  - The dired accessor spelling, revision heading, and Stage 2 ledger
    claim are corrected.

- rev 6 (2026-07-26) — **corrections found while implementing**, not a
  new design round. Four, all confirmed against the tree:
  - **Q#JR3 was false.** `replace_active_buffer` does *not* drop the
    startup scratch buffer; its body is one `switch_active_buffer` call,
    which reassigns `aw.buffer_id` and removes nothing. The claim came
    from that function's own doc comment (`editor.rs:1071`), which has
    been wrong for as long as it has existed, and rev 5 propagated it
    into §2.2, §3, P4, and the decision list without checking the body.
    Corrected in all four places; the stale comment is corrected in this
    PR too, since this PR would otherwise add *more* false references to
    it. **Actually removing the stale scratch is separate work** —
    buffer-lifetime changes have their own consequences (what else holds
    the id, what `C-x b` lists) and are not smuggled into a directory-open
    stage.
  - **The daemon bootstrap could report the wrong buffer** (§4.5). The
    directory arm captured `dest.buffer`, ran the resolver chain
    *synchronously*, then returned the captured id — so a handler that
    opened something synchronously (through `commit_to`, the supported
    way) had already replaced the window's buffer, and the reply would
    pair one buffer's snapshot with another's identity. The early return
    also skipped the post-hook revalidation this framing claimed stayed
    active. Rev 6 decides: **report what the window actually holds after
    the dispatch**, and rehome through `non_side_target` exactly as the
    file arm does.
  - **N11 tested neither `RET` nor self-insert.** It called
    `display_file` and `buf:insert` directly, so it stayed green with
    dired's `RET` binding, its entry dispatch, and the editor's
    self-insert path all broken — most of what "the journey works" means.
    Both gestures are now dispatched as real keys.
  - **P7 was vacuous and is removed, not weakened.** Q#JR12 has nothing
    to pin: `run` computes `had_file = file.is_some()` and a directory
    path is `Some` like any other, so suppression is structural and the
    named mutation would require inventing the branch first. The rev 5
    test additionally never armed restore and hard-coded `had_file`, so
    it asserted nothing about `run`. Q#JR12 is downgraded to an
    observation.

- rev 7 (2026-07-26) — **found while writing the `commit_to` suite and
  bite-testing it.** Three, all confirmed:
  - **N4 did not pin what its comment claimed.** Deleting the
    `ScopedFrontend` arm from `acting_frontend` left N4 green, because
    `ScopedFrontend::enter` *also* swaps `core.active_frontend` and the
    ambient fallback then answers correctly on its own. The arm is
    load-bearing in exactly one situation — a commit reached from inside
    an interactive command, where the origin sits between the override
    and the ambient value and would otherwise win. **N4b** is added,
    driven through `dispatch_key` (the only thing that establishes an
    interactive origin), and the mutation now bites it. The general
    lesson is the §6.0 one again from a new angle: two mechanisms that
    agree on the common path make either one look load-bearing.
  - **`commit_to`'s forged-destination message was unreachable.** With
    the parameter typed `mlua::AnyUserData`, mlua rejected a table during
    argument conversion, so a caller who fabricated one got "error
    converting Lua table to userdata" — true, but naming neither the rule
    nor how to obtain a real destination. The parameter is now
    `mlua::Value` and the pointed message actually fires. The refusal is
    unchanged; only its legibility is.
  - **P1 and P2 also fail on full revert**, since `commit_to` does not
    exist on the pre-image. §6.0's "legitimately green on the pre-image"
    does not describe them. They stay in the P list because their
    *discriminating* falsifier is the named mutation, not the revert: a
    revert-only check cannot distinguish "validates" from "validates in
    time", which is the entire claim. Noted at each pin rather than
    silently mislabelled.
  - Bite results recorded: mutation A (scope stops swapping
    `core.active_frontend`) fails N6a and P3 and nothing else; mutation B
    (preflight moved after the callback) fails P1 and P2 and nothing
    else; mutation C (drop the `ScopedFrontend` arm) fails N4b and
    nothing else.

- rev 8 (2026-07-26) — **review of PR #182.** One implementation gap and
  two stale claims:
  - **dired did not honor the captured window.** §4.4 specified
    `display{ window = dest:window() }`; the implementation still ended
    in `pmacs.window.switch_buffer`, which targets whatever window the
    *scoped frontend* has selected. The scope pins the frontend; it does
    not pin the window. So a split or panel that took focus while
    `read_dir` was pending received the listing, and `prev` was captured
    from it too — with every preflight check passing, because the
    captured window was still live and still held its captured buffer.
    Fixed in both places (`display` and the `prev` read), and **N4c**
    added. The suite's routing pins all varied *frontend* identity;
    none varied the selected window within one frontend, which is why
    23 green pins missed it.
  - **The §0 scorecard row still graded §2 "Broken at entry"** while §2's
    own ground truth had been rewritten — the scorecard is a second copy
    of the same claim and §25's update protocol covers both. §19's row
    and ground truth were stale in the same way (this PR creates the
    first cross-subsystem suite) and are corrected too.
  - **P4 still said "leaves exactly one buffer"**, the exact claim rev 6
    corrected as false everywhere else. Restated to what it actually
    pins — the file is in the *active window* — matching the test that
    was already written correctly.

---

## 0.5. Coherence impact (`COHERENCE.md` §20, required since #163)

- **Journey steps.** §20's first-named arc; 1a takes the broken half of
  **step 3**. After 1a, `pmacs .` opens the directory. Steps 4 and 6–12
  do not change grade. §2's verdict table and §20 Priority 1's "State:
  broken at step 3" line are rewritten in this PR per §25.
- **Interaction islands: adds none, removes one.** No new keymap, mode,
  or modal surface; the directory arm routes into #165's dired buffer —
  the "must not invent a second directory surface" constraint. The
  unification (§3) removes an island: startup and the daemon bootstrap
  resolve paths through two independently-written implementations today.
- **Config registry.** Adds no keys. The directory fallback is a function
  slot, not a setting — `ConfigValue` is four scalars and a handler is
  none of them (the reason terminal profiles could not be settings, #173).
- **Ownership, stated honestly (rev 3).** That slot is an **unowned
  singleton**: last writer wins, no owning package, no `SourceLocation`,
  no removal lifecycle, and it does not appear in any inspection surface.
  That is a real §13 gap and this framing does not dress it up — §20
  Priority 3 is deliberately deferred, and 1a is not the place to invent
  ownership machinery for one slot. **Named migration:** when Priority 3
  lands registration ownership and `pmacs.hook.remove`, the slot becomes
  an ordinary lowest-priority hook subscription carrying its owner, and
  this primitive is deleted rather than extended.
- **Background-work attribution (§9).** No new `JobKind` variant, no new
  `PendingJob` field; the listing uses `pmacs.fs.read_dir`, whose kind
  #165 added. Neutral.
- **Frontend parity (§16).** Both frontends get the behavior from the
  same primitive. One asymmetry ships knowingly: the GPU path displays
  its pre-existing bootstrap buffer until the listing settles (§8 B2).
- **New substrate (rev 3, revised rev 4–5).** `commit_to` (§4.4) is a
  general fix for a general problem — *every* post-await
  `pmacs.window.*` call in the tree acts on the ambient frontend by
  documented design (`dired.lua:68-73`). 1a introduces it for one caller
  and does not migrate the others; that migration is named as deferred
  rather than smuggled in. It adds a **scoped frontend override**
  distinct from `InteractiveCommandOrigin` (§2.11), deliberately: a
  background continuation gets destination scope **without** acquiring
  interactive-command authority, which keeps the "programmatic vs
  interactive" distinction the unfold guard, command boundaries, and the
  terminal surface all depend on.

---

## 1. What Stage 1a ships

1. **`pmacs .` opens the directory**, on the local TUI path and the
   daemon/GPU bootstrap path, routed into #165's dired buffer.
2. **One path-resolution primitive** — `EditorState::open` adopts
   `EditorCore::resolve_target_buffer` wholesale.
3. **A scoped-destination commit primitive** (§4.4) so an async open
   lands where it was requested, or nowhere.
4. **The first cross-subsystem journey acceptance suite** (§19).

Not in 1a — Stage 1b: a compile keybinding and `cargo build`/`test`
defaults from the existing `ProjectKind::Cargo`, LSP spawn-failure
guidance (§1.2), a welcome buffer.

---

## 2. Ground truth (scouted 2026-07-26, `main` @ `d400f30`; re-verified rev 4)

### 2.1 `pmacs .` still exits 1, and why

`load_file` (`src/file_io.rs:81`) does `File::open` — which succeeds on a
directory — then `read_to_end`, returning `EISDIR`. Not
`ErrorKind::NotFound`, so every `NotFound` arm is skipped and the error
propagates; `main` prints and exits (`src/main.rs:400-403`).

### 2.2 There are two path-open implementations, not one

`resolve_target_buffer` (`editor_core.rs:885`) documents itself as *"One
primitive, so two path-normalization, dedup, and hook transactions cannot
drift apart."* Callers: `display_file` (`window_panel.rs:402`) and the
daemon bootstrap (`daemon.rs:1641`). **Local startup is not one of them**
— `EditorState::open` (`editor.rs:757`) hand-writes the same shape.

| | `EditorState::open` | `resolve_target_buffer` |
|---|---|---|
| Stored buffer path | **normalized** — `set_buffer_path` normalizes internally (`editor_core.rs:810-822`) | **normalized** — same setter |
| Displayed name | `path.display()` raw (`editor.rs:772`) | `path.display()` raw |
| `NotFound` arm | empty path-backed buffer, `[new file]` | identical |
| Dedup | none | `find_buffer_for_path` |
| Window install | `replace_active_buffer` — switches the ACTIVE window (`editor.rs:797`). **It does not drop the startup scratch** (rev 6): its body is one `switch_active_buffer` call, which reassigns `aw.buffer_id` and removes nothing. The doc comment claiming otherwise was wrong before this stage and is corrected in this PR | none; caller installs |
| Error type | `io::Error`, bare | `String`, prefixed `cannot open {path}: ` |

**The two agree on every observable except the error prefix and the
window install.** Rev 1 claimed a raw-vs-normalized split and built a
decision, a bet, and an acceptance on it; all three were withdrawn in rev
2. Rev 3 draws the further consequence the review identified: because the
implementations already agree, **no equivalence assertion can prove the
unification happened** — such a test passes on the pre-image. §6.0
restructures the acceptance list around that.

The unification's value is therefore (a) the directory arm reaching
startup once rather than being written twice, and (b) closing drift the
primitive was created to prevent and did not. Not a behavior fix.

### 2.3 dired creates its own buffer and refuses adoption

`claim_handle` (`dired.lua:486`) creates the buffer, applies the
read-only intercept, `set_round_trip_input`, and the `dired` major mode.
Its comment is explicit that finding a buffer by name is **not**
adoption. Handles are pathless. No Lua `buffer.set_name` /
`set_file_path` exists; dired Stage 2 (PR #171 §5) is scoped to add one.

### 2.4 The listing is async; the bootstrap reply is not

`read_listing` (`dired.lua:462`) awaits `pmacs.fs.read_dir` and its
comment says *"Must run inside `pmacs.async`"*. The daemon bootstrap is
one synchronous block: `open_initial_target` (`daemon.rs:1624`) →
`initial_target_snapshot` (`:1823`) → `InitialTargetResult::Opened`
(`:1888`), with the GPU frontend blocking on the reply before creating
its window (`pmacs-gpu/src/attach.rs:551`).

`tick_async` resuming a coroutine in the frame its result arrives does
**not** bound the listing to one frame — the worker must still finish.
`tests/dired_acceptance.rs:103`'s `pump` drives until parked-coroutine
*and* pending-job counts both reach zero: *"nothing dired does is
observable until this returns."*

### 2.5 Post-await, dired acts on the ambient frontend — by design

`dired.lua:68-73`: *"`pmacs.window.*` calls made after the await act for
the **ambient** active frontend, since interactive origin does not
survive the tick boundary; and `pmacs.editor.move_to_line` acts on the
ambient **buffer**, which is why every post-await re-seat is guarded."*

Correct for an interactive `C-x d`. Wrong for a startup open that must
land in a specific frontend's specific window.

**And the ambient reach is wider than `display`.** `open_directory`
(`dired.lua:607-655`) after the await, in order:

1. `read_listing` — the await;
2. `handle_for_path(canonical)` / `claim_handle(canonical)` — **creates a
   buffer**, applies intercept/mode, registers a handle;
3. assigns `entries`, `errors`, `sort_mode`;
4. `handle.prev = pmacs.window.buffer()` — **reads the ambient buffer**;
5. `paint(handle)` — mutates the buffer;
6. `display(handle, opts, departed)` — the first call that could refuse;
7. `seat_cursor` — `move_to_line` on the ambient buffer;
8. `kill_departed`.

`lookup_window` refuses a foreign window id (`window_panel.rs:202-212`),
but only at step 6. **Rev 2's "fails closed, does nothing" was false**:
steps 2–5 have already run. A refusal leaves a hidden dired buffer and a
registered handle, and step 4 can capture an unrelated frontend's buffer
as `prev`. §4.4 fixes this by revalidating and scoping *before* step 2.

### 2.6 Subscribers exist before the hook fires — but ordering is fixed

`EditorState::new()` loads the builtin runtime (dired at `editor.rs:539`)
then user `init.lua` (`:609`, `cfg(not(test))`). `HookRegistry::add`
**appends** (`hook.rs:240`); no prepend, no priority, no removal
(`COHERENCE.md` §13 names `pmacs.hook.remove`'s absence as a Priority 3
prerequisite). **A builtin subscriber always runs before any user
subscriber, forever.**

### 2.7 Short-circuit cannot distinguish a claim from a crash

`run_short_circuit` (`hook.rs:299-323`) returns `proceed: false` for a
literal `false` return **and** for a raising callback; only
`HookOutcome.errors` (non-empty in the second case) tells them apart.
A resolver chain that keys only on `proceed` treats a broken user
callback as a successful claim. Q#JR15 decides the policy.

### 2.8 `open_initial_target` reasserts after hooks

It re-checks the buffer exists (`daemon.rs:1665-1670`) then reinstalls it
into the origin document window, rehoming if a hook closed it
(`:1673-1690`). §4.5's design does not fight this.

### 2.9 This deliberately supersedes part of the GPU initial-target framing

`docs/gpu-initial-target-framing.md` Q#GT6 (`:278`) lists `IsADirectory`
among initial-target failures; its acceptance 10 (`:550`) requires *"a
directory/permission-denied target returns a specific failure before
ready/window creation"*. **1a supersedes the directory half only.**
Permission-denied, invalid path bytes, session teardown, and the
"existing daemon remains connectable" clause keep their contract. The
superseded assertions are amended in that framing in this PR, per §25.

### 2.10 `display_file`'s directory failure is load-bearing today

`builtin/commands/default.lua:724` wraps `display_file` in a `pcall`
whose comment says *"only a real failure (a directory, a permission
error) reaches here"*, pinned by
`find_file_accepting_a_directory_reports_instead_of_raising`
(`tests/find_file_acceptance.rs:235`). §4.6 answers to it.

### 2.11 There is a scope mechanism, and it is the wrong one

`acting_frontend` (`window_panel.rs:46-50`) reads
`InteractiveCommandOrigin` app data, falling back to
`core.active_frontend_key()`; `InteractiveCommandOrigin::enter(fid)`
(`editor.rs:63-69`) returns an RAII guard. Rev 3 proposed reusing it.
Two independent reasons it cannot be:

**(a) It does not scope what rev 3 claimed.** Only the window-panel
bindings consult `acting_frontend`. Two of dired's post-await steps do
not go through it at all:

- **no-arg `pmacs.window.buffer()`** — step 4's `prev` capture — reads
  `core.active_buffer_id()` directly (`mod.rs:12547`). Its comment is
  explicit that this is deliberate and infallible, and states the
  assumption it rests on: *"dispatch sets `active_frontend` to the acting
  frontend before running a command, so the two agree on every real
  path."*
- **`pmacs.editor.move_to_line`** — step 7's cursor seating — is
  `cc.borrow_mut().move_to_line(line)` on the core's ambient active
  window (`mod.rs:12703`).

So entering the interactive origin would scope `display` and leave `prev`
capture and seating ambient — precisely the two steps §2.5 identifies as
corrupting.

**(b) It is authenticated user-command authority, and a startup
continuation must not impersonate one.** `InteractiveCommandOrigin` is
what distinguishes a user command's edit from a plugin's or the data
API's. Three consumers would be misled:

- the **pre-edit unfold** guard (`mod.rs:1385-1400`), whose doc calls it
  *"the scoped authority that distinguishes a user command's edit from a
  plugin's or the data API's programmatic one"* — dired's `paint` would
  satisfy it and unfold at the edit site;
- `invoke_interactive`'s command-boundary rotation (`:5400`), which
  raises without it and would silently succeed with it;
- `terminal_command_frontend` / `active_terminal_view_key` (`:8515`,
  `:8527`), which treat its presence as "an interactive frontend context".

Q#JR14e therefore introduces a **separate** override. Note that the
`window.buffer()` comment above is not an obstacle but a specification:
swapping `core.active_frontend` for the scope's extent is exactly what
makes its stated assumption true for a continuation, restoring the
invariant rather than working around it.

---

## 3. The unification (Q#JR1)

`EditorState::open` becomes a thin caller of `resolve_target_buffer`,
keeping `replace_active_buffer` (which switches the **active** window,
Q#JR3 as corrected in rev 6 — it does not destroy the old scratch, and
never did)
and keeping its "fire the hook after the core borrow ends" structure
(`editor.rs:786-795`) — listeners re-enter `pmacs.editor.*` and re-borrow
the core (Q#JR1a).

**Q#JR4** — startup errors gain the `cannot open {path}: ` prefix.
`pmacs /root/secret` names the file, which today's bare message does not.
This is the *only* user-visible change from the unification (§2.2).

**Q#JR12 (downgraded to an observation, rev 6)** — a directory argument
suppresses desktop restore, on Q#DS7's reasoning that a positional
argument means "open this" rather than "restore my session". This needs
no work and cannot be pinned: `run` computes `had_file = file.is_some()`
(`editor.rs:3152`), and a directory path is `Some` like any other, so
there is no directory-specific branch that could get it wrong. Rev 5
carried an acceptance for it; that test never armed restore and
hard-coded `had_file`, asserting nothing, and is removed rather than
repaired.

---

## 4. The directory arm, the resolver, and the destination

### 4.1 Q#JR5 — a typed result

```rust
pub enum ResolvedTarget {
    Buffer { id: BufferId, fire: HookKind },
    Directory { path: PathBuf },   // normalized: absolute, ~-expanded, lexically clean
}
```

Rev 1's `(Option<BufferId>, HookKind)` admitted states that cannot occur.
`resolve_target_buffer` checks `path.is_dir()` ahead of the load.

**Q#JR8** — the `Directory` variant carries an explicitly normalized
path. It is *not* free: normalization lives inside `set_buffer_path`, and
this arm creates no buffer, so nothing would normalize anything and the
local caller would still hold `"."`. Same lesson as the Lean 4 arc's URI
affinity — a handler keying state by path must never receive `"."`.

**Q#JR5b** — `editor_core::HookKind` and `hook::HookKind` are unrelated
types sharing a name; both are written path-qualified in every file this
PR touches, and `window_panel.rs:37`'s bare import is changed to match.

**Q#JR6** — Rust creates no buffer for a directory. A placeholder needs
reaping, is reinstalled by §2.8's reassert, and — if dired adopted it —
would drag in dired Stage 2's rename prerequisite (§2.3).

### 4.2 Where the directory arm is consumed

`EditorState::open` and `open_initial_target` dispatch the resolver
chain. `display_file` does not (§4.6).

### 4.3 Q#JR7 — a user-only hook, then a replaceable fallback

Given §2.6, "a package subscribes ahead of dired" is unreachable. So the
two roles are split:

**The chain.** `path.open-directory`, `kind = "short-circuit"`, fired
first. Returning `false` claims the directory and stops the fan-out. **No
builtin subscribes** — the rule that makes "user code runs first" true
under append-only registration, stated in the hook's own description.

**The fallback.** If unclaimed, the arm calls the directory handler — a
function slot defaulted by `dired.lua`:

```lua
pmacs.path.set_directory_handler(function(path, dest)
  open_async(path, { dest = dest }, nil, "dired")
end)
```

Users replace it, chain it (capture the previous value first), or
**disable** it (`set_directory_handler(nil)`), which is what makes
acceptance 10's unclaimed path reachable. It is an unowned singleton
slot, with the honest accounting and named migration in §0.5.

**Q#JR15 (new) — a raising callback stops the chain *and* suppresses the
fallback.** §2.7 shows `proceed` alone cannot distinguish a raise from a
claim. Policy: inspect `HookOutcome.errors`; when non-empty, report
through `*errors*` **and** `pmacs.editor.set_status`, and do **not** run
the fallback. Rationale: this preserves the existing short-circuit
contract (a raising `buffer.before-save` callback already vetoes the
save), and running the fallback after a user's resolver crashed would
open dired on a directory the user's code may have been mid-way through
handling. The cost — a broken user callback disables directory opening
until fixed — is visible, reported through two surfaces, and preferable
to silently ignoring the user's resolver.

*Deferred, named:* hook priority/prepend is the general fix for §2.6 and
belongs with `pmacs.hook.remove` in Priority 3. When it lands, the
fallback becomes an ordinary lowest-priority subscription.

### 4.4 Q#JR14 (rev 5) — the scoped-destination commit

**The blocker rev 2 missed:** §2.5 shows dired mutates handle state at
steps 2–5 and only reaches a refusable call at step 6. "Fails closed,
does nothing" was false — a refusal left a hidden buffer, a registered
handle, and a `prev` captured from whichever frontend happened to be
ambient. Per the review's decision, **1a carries the substrate fix.**

**Q#JR14d — the destination is an opaque capability, not a table.**
`dest` is **nonconstructible userdata**, created only by Rust, holding
three private ids:

| field (private) | source | purpose |
|---|---|---|
| frontend | local: `FrontendId::LOCAL`; bootstrap: the attaching `frontend_id` | the scope to commit in |
| window | local: the active window; bootstrap: `origin_window` (`daemon.rs:1637`) | where the listing goes |
| buffer | the buffer that window holds at capture time | **stale-intent detection** |

A table would be wrong in two ways, both reachable: the *same* `dest` is
passed to every hook listener in turn, so an earlier listener could
mutate it and then decline — redirecting later listeners or the fallback
— and any Lua could fabricate a plausible triple and call `commit_to`
directly. Userdata makes both unrepresentable rather than merely
discouraged.

The only accessor is read-only `dest:window()`, which dired needs for its
exact `display{window = …}` target. `commit_to` accepts **only** this
userdata and revalidates its private contents itself; it never trusts a
caller-supplied id.

**Q#JR14e — a separate scoped frontend override, not the interactive
origin.** §2.11 gives both reasons. Rev 4 adds a distinct app-data
override with resolution order:

```
acting_frontend  =  scoped override  →  interactive origin  →  ambient
```

Its RAII guard **also** swaps `core.active_frontend` and restores it on
drop, which is what covers the core-ambient APIs `acting_frontend` never
sees (`window.buffer()` no-arg, `move_to_line`). `commit_to` does **not**
enter `InteractiveCommandOrigin`, so a startup continuation never
acquires interactive-command authority.

**The primitive.** `pmacs.window.commit_to(dest, fn)`:

1. **Preflight, before running anything** — the destination's frontend
   has a registered view; its window is live in that view's layout; the
   window still holds the captured buffer (Q#JR14c); and the window is
   **replaceable** (Q#JR14f).
2. On any failure, returns `false, reason` **without calling `fn`** — so
   nothing is claimed, painted, or captured.
3. On success, enters the scoped override for the dynamic extent of `fn`
   and calls it. Inside, `display{window = …}`, no-arg
   `window.buffer()`, `move_to_line`, and every other ambient primitive
   resolve against the captured destination — which is why a `frontend`
   option on `display` alone would have been insufficient.

**Q#JR14f — preflight must establish replaceability, through the same
predicate every exact-target probe and display uses.** Exact display
refuses a window that is `dedicated` unless it already shows the
*incoming* buffer (`editor_core.rs:3566`). The distinction is
load-bearing here: `dest.buffer` is the captured bootstrap buffer, not
dired's future buffer. Passing it as the incoming buffer would approve a
window dedicated to that bootstrap buffer; dired would then claim and
paint its different buffer, and exact display would refuse afterward —
rev 2's hidden-buffer failure through another door.

The eligibility test is therefore extracted once, with the semantic
input `incoming: Option<BufferId>`:

| caller | input | dedicated-window result |
|---|---|---|
| `display_buffer` exact-target arm | `Some(request.buffer_id)` | eligible only when already showing that buffer |
| `probe_display_target` | its existing `Option<BufferId>` | preserves today's load-before-placement probe contract |
| `commit_to` preflight | `None` | always ineligible — the replacement does not exist yet |

`probe_display_target` already carries the correct `Option<BufferId>`
shape (`editor_core.rs:3470-3483`), so leaving it on a private copy while
sharing only the other two would preserve the same drift this extraction
exists to remove. Core unit coverage pins the three decisive rows:
dedicated + `Some(current)` is eligible; dedicated + `Some(other)` is
refused; dedicated + `None` is refused.

**Q#JR14b — `fn` must not await.** The scope is an RAII guard on the
Rust stack; a yield inside it would let the guard's extent and the
coroutine's suspension diverge, restoring the override while the
continuation is still parked. `commit_to` sets a flag that `Handle:await`
checks and raises on, naming the rule. Enforced, not documented — pinned
by N6.

**Atomicity, stated precisely.** `commit_to` is atomic **against
destination-precondition failure**: if any preflight check fails, no
callback runs and nothing is mutated. It is **not** a transaction over
the callback — if `fn` raises halfway through, `commit_to` restores the
scope and propagates, but whatever `fn` already mutated stays mutated.
Rolling that back would require dired to make its claim/paint sequence
undoable, which is a dired change well beyond 1a. What 1a guarantees is
that the *destination* checks happen before the first mutation, which is
the failure the review identified.

**dired's change.** `open_directory` keeps `read_listing` (the await)
outside, then performs steps 2–8 inside a single `commit_to` callback,
displaying with `{ window = dest:window() }` rather than the ambient
`switch_buffer`. On a `false` return it reports through
`pmacs.editor.set_status` and returns, having mutated nothing.

**Q#JR14c — stale intent loses to the user.** If the destination window
now holds a different buffer than at capture, the request is stale and
**fails closed**. Rev 2's window-only pin said launch intent overwrites
whatever the user did meanwhile; that was wrong, and it rested on B2's
"before the user can act", which §2.4 disproves — a large directory takes
many frames and the user can act in every one of them. The user's action
is newer information than the launch argument.

**What this buys, stated as the review framed it:** competing frontend
activity no longer turns a valid startup request into a nondeterministic
no-op. A live, unchanged destination receives its listing regardless of
what other frontends did meanwhile. Fail-closed is reserved for a
destination that is genuinely dead or stale.

*Deferred, named:* migrating dired's other post-await paths (`C-x d`,
tree descent/ascent, refresh) and every other ambient post-await
`pmacs.window.*` call in the tree onto `commit_to`. 1a introduces the
primitive for the startup path and does not sweep; the sweep is its own
PR with its own acceptance, and this framing does not pretend the general
problem is solved.

### 4.5 Q#JR9 — what the bootstrap reply names, and what it shows

`open_initial_target` on a `Directory` installs nothing: it dispatches the
resolver, then replies `Opened { buffer_id }` naming **whatever the
destination window holds once that dispatch returns** — re-read, not the
id captured beforehand (Q#JR9b, rev 6).

The distinction is not academic. The chain runs **synchronously**.
dired's handler defers, because its listing must await; a user's resolver
is under no such obligation, and one that opens something synchronously
through `commit_to` — the supported way to do it — has already replaced
the window's buffer by the time the reply is built. Reporting the
captured id would pair one buffer's snapshot with another's identity, and
the frontend would render a document nobody asked for.

Re-reading also subsumes the case where a hook closed the window, so this
arm rehomes through `non_side_target` exactly as the file arm's reassert
does, rather than returning early and skipping that check — which rev 5's
implementation did while this section claimed the revalidation stayed
active.

Absent a synchronous claimant the re-read yields the buffer the window
already held, which is the ordinary case.

**That buffer is not necessarily `*scratch*`.** `build_fresh_frontend_view`
clones **LOCAL's primary document buffer** (`daemon.rs:2997`) — M10.9 made
attaching frontends share LOCAL's buffer so overlays fire; the
bottom-panel arc narrowed it to the *primary document* buffer so a TUI
panel could not become a new frontend's document. If LOCAL holds a real
document, `pmacs --gpu .` briefly displays and snapshots that unrelated
document.

**Decision: accept and document.** A bootstrap placeholder re-creates
everything Q#JR6 rejected to fix a transient, and the session genuinely
*is* showing LOCAL's document — the same thing a no-argument `--gpu`
attach shows. Acceptance N5 pins it with a deliberately non-scratch LOCAL
primary so it is observed rather than assumed.

### 4.6 Q#JR13 — `display_file` keeps its directory error

`display_file` does **not** dispatch the resolver. On
`ResolvedTarget::Directory` it raises:

- the message names the path and the directory reason (an improvement on
  the raw `EISDIR` text, and the only user-visible change here);
- the active buffer, window layout, and selected window are unchanged —
  nothing created, nothing switched;
- `find_file_accepting_a_directory_reports_instead_of_raising` passes
  **unmodified**.

`display_file` is "put this file in a window", not a CLI router. Routing
it into dired would silently change `C-x C-f` on a directory, in a PR
about the CLI, through a `pcall` arm whose comment guarantees the
opposite.

*Deferred, named:* Emacs's `find-file` does open dired on a directory,
and that is reasonable eventual behavior. It is a find-file UX decision
with its own acceptance, belonging to the dired arc or 1b. When taken it
is a small change at `default.lua:724`, and the pinned test above is what
gets deliberately rewritten.

---

## 5. The journey acceptance suite (§19)

New: `tests/journey_acceptance.rs`, seeded with steps 2 (launch
unconfigured), 3 (open a real project), and 5 (edit immediately),
driving the **real startup entry point** — a directory arm with no
production caller passes every direct-call test. Steps 6–12 enter as
later stages make them real; the file is a ratchet.

Every dired-dependent assertion pumps to quiescence using
`dired_acceptance.rs:103`'s idiom (parked coroutines *and* pending jobs
at zero), never a fixed frame count (§2.4).

---

## 6. Acceptance

### 6.0 Two kinds of pin, and why the distinction matters

Rev 2 asserted that every acceptance "fails with the change reverted".
The review is right that this cannot hold for preservation guards — and
rev 2's acceptance 6 was the proof: because both implementations already
agree on every observable (§2.2), an equivalence assertion passes on the
pre-image. **Behavioral equivalence cannot demonstrate structural reuse.**
The list is therefore split, and each preservation pin names the
*targeted mutation* it is bite-tested against:

- **(N) New-behavior acceptances** — must fail on full revert.
- **(P) Preservation pins** — legitimately green on the pre-image;
  falsified by a named targeted mutation, not by revert.

That local startup reaches the new directory behavior is proven by N1,
not by any equivalence assertion — which is also why rev 2's acceptance 6
is **removed rather than recast**: it proved nothing N1 does not.

### 6.1 New-behavior acceptances (N)

- **N1** `pmacs .` in a project directory exits 0 and, after pumping to
  quiescence, the active buffer is dired's, listing that directory.
  Today: exit 1.
- **N2** Daemon/GPU bootstrap with a directory initial target receives
  `InitialTargetResult::Opened`, not `Failed`, and after quiescence the
  document window shows the dired buffer. Supersedes the GPU framing's
  acceptance 10 for directories (§2.9).
- **N3** `pmacs .` on an unreadable directory reports through dired's
  status path and leaves the session running — no exit 1, no half-built
  buffer.
- **N4 — delivery despite competing frontends (the blocker's positive
  half).** Two registered frontends; a directory bootstrap for frontend
  A; frontend B dispatches unrelated activity (buffer switch, window
  focus) while the listing is in flight. After quiescence the listing is
  in **A's** captured window, and B's active buffer and window are
  unchanged. Falsified by reverting `commit_to` to the ambient
  `switch_buffer`.
- **N4b — the scope outranks an *interactive origin*, added rev 7.** N4
  alone does not pin `acting_frontend`'s ordering claim: with the
  `ScopedFrontend` arm deleted, N4 still passes, because `enter` also
  swaps `core.active_frontend`. The arm matters only when an interactive
  origin is set, which sits between the override and the ambient value.
  A command dispatched by frontend B calls `commit_to` with A's
  destination; the commit must still land in A's window. Falsified by
  deleting the arm, or by ordering it after the interactive origin.
- **N5** Bootstrap with a deliberately **non-scratch** LOCAL primary
  document buffer: the reply's `buffer_id` is that buffer, and after
  quiescence the window shows dired (Q#JR9, §4.5).
- **N4c — the captured *window*, not the captured frontend's selected
  one (added rev 8).** One frontend, two windows: capture a destination,
  then split and move focus to the other window and give it a buffer of
  its own, then run dired's handler path with the captured destination.
  The listing lands in the captured window, the focused window is
  untouched, and `q` returns to the buffer the *captured* window showed.
  Falsified independently by restoring `switch_buffer` in dired's
  `display` and by reading `prev` from the ambient window — both were
  verified to fail only this pin.
- **N6 — `commit_to` scopes and restores, on every exit path.** Three
  cases, each asserting that **both** the scoped override and
  `core.active_frontend` return to their prior values: (a) `fn` returns
  normally; (b) `fn` raises; (c) `fn` awaits and is refused (Q#JR14b).
  Case (c) additionally asserts the raise names the rule. Rev 3 checked
  only the interactive origin's restoration on the success path, which
  §2.11 shows is neither the right value nor enough paths. Falsified by
  dropping the flag, or by restoring on success only.
- **N6b — `commit_to` refuses a forged destination.** A Lua-constructed
  table with plausible `frontend`/`window`/`buffer` fields is rejected as
  a type error, and userdata cannot be constructed from Lua (Q#JR14d).
  Falsified by accepting a table. *Rev 7:* the parameter is typed
  `mlua::Value` and `commit_to` performs the check itself, so the refusal
  names the rule — typed as `AnyUserData`, mlua rejected the table during
  argument conversion with a message naming neither the rule nor the
  remedy, leaving the pointed one unreachable.
- **N6c — a declining listener cannot redirect the destination.** Two
  listeners: the first receives `dest`, attempts mutation inside `pcall`,
  observes the read-only rejection, and declines; the second verifies
  `dest:window()` still names the original window and also declines; then
  the fallback commits there (Q#JR14d). Falsified by passing a shared,
  mutable table.
- **N7 — the resolver chain.** `path.open-directory` is short-circuit and
  first-claimant-wins, exercised through an **ordinary user-registered
  listener** (no builtin subscribes, §4.3): two listeners, the first
  returns `false`, the second must not run, and the fallback must not
  run. Falsified by `all-must-succeed` or `accumulate`.
- **N8 — a raising callback suppresses the fallback *and* is reported
  (Q#JR15).** A listener that raises: the fallback does not run, the
  directory does not open, and the failure reaches both `*errors*` and
  the status line.
  *Falsifier, corrected in rev 4:* keying the fallback on `proceed` alone
  is **already correct** for suppression — §2.7 shows a raise gives
  `proceed == false` just as a claim does. `errors` decides the *report*,
  not the fallback. So N8 is falsified by either (a) running the fallback
  when `errors` is non-empty — i.e. treating a raise as a decline — or
  (b) mutating the short-circuit outcome so a raise yields
  `proceed = true`. Rev 3 named the inverse mutation, which does not
  falsify anything.
- **N9** The hook and the handler receive a **canonical absolute path** —
  firing on `.` from a known cwd delivers that cwd, not `"."` (Q#JR8).
- **N10** With the handler slot cleared and no listener claiming,
  `pmacs .` exits **0**, leaves the bootstrap buffer in place, and sets a
  status naming the path (Q#JR10).
- **N11** `pmacs .` → dired lists → `RET` on a listed file visits it → a
  self-insert lands in **that file's** buffer. (Rev 1 self-inserted into
  the dired buffer, whose intercept rejects every edit, `dired.lua:506`.)

### 6.2 Preservation pins (P), each with its falsifying mutation

*Rev 7 correction:* **P1 and P2 also fail on full revert** — `commit_to`
does not exist on the pre-image, so §6.0's "legitimately green on the
pre-image" does not describe them. They stay here because their
*discriminating* falsifier is the named mutation: a revert-only check
cannot distinguish "validates" from "validates in time", which is their
entire claim. P3–P8 are preservation pins in the strict sense.

- **P1 — precondition failure is atomic (the blocker's negative half).**
  **Three** destination failures, each asserted the same way — after
  quiescence the buffer count is unchanged, **no dired buffer or handle
  exists for that path**, no window's buffer changed, and a status names
  the failure:
  1. **dead** — the destination window was closed;
  2. **stale** — its buffer was replaced (Q#JR14c);
  3. **ineligible** — it is `dedicated` to its still-current captured
     buffer, but dired's incoming replacement does not exist yet
     (Q#JR14f, completed rev 5). This is the case a preflight that
     mistakenly passes `dest.buffer` as the incoming buffer approves and
     `display` then refuses *after* dired has claimed and painted.
  *Mutation:* move the preflight from before `claim_handle` to after
  `paint` — rev 2's design. P1 fails on all three; rev 2's acceptance 3b
  passes. *Second mutation, for case 3 specifically:* pass
  `Some(dest.buffer)` instead of `None` to the shared eligibility
  predicate while keeping liveness and stale-buffer validation. Only case
  3 fails — which is the point of separating it.
- **P2 — stale intent loses (Q#JR14c).** The user replaces the
  destination window's buffer while the listing is in flight; their
  buffer survives and dired does not overwrite it.
  *Mutation:* drop `dest.buffer` from revalidation (rev 2's window-only
  pin). P2 fails.
- **P3 — dired's existing handles are not corrupted.** With a dired
  buffer already open in another frontend, a failed startup open leaves
  that handle's `prev`, entries, and cursor untouched.
  *Mutation:* restore the ambient `handle.prev = pmacs.window.buffer()`
  outside the scope (§2.5 step 4).
- **P4 — startup shows the file in the *active window* (Q#JR3, corrected
  rev 6, restated rev 8).** `EditorState::open` displays the loaded
  buffer in the active window and no window is left showing the startup
  scratch. It does **not** assert a buffer count: `replace_active_buffer`
  does not drop the scratch buffer, and rev 5's "leaves exactly one
  buffer" wording — which survived rev 6's correction here by oversight,
  caught in review of PR #182 — asserted a guarantee the editor does not
  make.
  *Mutation:* replace `replace_active_buffer` with a bare
  `install_buffer_in_window` into some other window.
- **P5 — the `NotFound` arm survives the refactor.** A nonexistent path
  yields an empty path-backed buffer with `[new file]` and fires no hook.
  *Mutation:* delete the `NotFound` arm from `resolve_target_buffer`.
- **P6 — `display_file` keeps its contract (Q#JR13).** It raises on a
  directory naming path and reason; active buffer, layout, and selected
  window unchanged; `find_file_accepting_a_directory_reports_instead_of_raising`
  passes unmodified.
  *Mutation:* route `display_file` into the resolver chain.
- **P7 — REMOVED in rev 6.** Q#JR12 is structural: `run` computes
  `had_file = file.is_some()` and a directory path is `Some` like any
  other, so there is no directory-specific branch to break and the named
  mutation would have to invent one first. Rev 5's test never armed
  restore and hard-coded `had_file`, so it could not fail against any
  implementation. Removed rather than repaired — a green test that cannot
  fail reads as coverage.
- **P8 — startup errors name the file (Q#JR4).** A non-`NotFound`,
  non-directory failure produces a message containing `cannot open` and
  the path. *(Legitimately N-shaped for the prefix, P-shaped for the
  failure itself; listed here because the failure behavior is preserved
  and only the message changes.)*

`scripts/bite` runs over the new suite. A VACUOUS report on any N is a
blocker; each P's named mutation is run as its bite check, since revert
cannot falsify it.

---

## 7. Deferred (named)

- **Migrating the rest of the tree onto `commit_to`** (§4.4) — dired's
  other post-await paths and every other ambient post-await
  `pmacs.window.*` call. Its own PR, its own acceptance.
- **Hook priority / prepend**, with `pmacs.hook.remove`, in §20 Priority
  3 — at which point the fallback slot becomes an ordinary lowest-priority
  subscription and §0.5's unowned-singleton gap closes.
- **True adoption (option B).** Rust creates the buffer, dired adopts —
  one buffer, no transient — but it needs dired Stage 2's rename /
  clear-path capability (§2.3). Dired Stage 3; Q#JR6 does not block it.
- **The bootstrap transient** (§4.5, §8 B2).
- **`C-x C-f` on a directory opening dired** (§4.6).
- **Multiple path arguments** (`main.rs:227`, `:232`, `:240`).
- **`pmacs .` opening a panel** rather than the document window.
- Stage 1b and the rest of §20 Priority 1.

---

## 8. Bets

- **B1 — "one thing opens a directory" holds.** If a picker and dired
  should both run, short-circuit is wrong and the hook must become a
  resolver returning a target.
- **B2 (corrected twice) — the bootstrap transient is acceptable.** The
  window shows its pre-existing buffer **until the listing settles** —
  not "one frame" (rev 1), and **not** "before the user can act" (rev 2):
  §2.4 disproves the bound and Q#JR14c is the consequence — the user
  *can* act, so stale intent must lose. The bet is only that the
  transient is visually acceptable at process start.
- **B3 — withdrawn** (rev 2). There was no path-normalization change.
- **B4 — failing closed on a genuinely dead or stale destination is
  better than guessing.** Narrowed in rev 3: it applies only after
  revalidation says the destination is gone, not to any competing
  activity (N4).
- **B5 — `commit_to`'s no-await rule is livable.** Every commit step
  dired performs after the listing is synchronous today, so the rule
  costs nothing here. If a future handler genuinely needs to await
  mid-commit, the primitive needs a re-entrant design and this bet is
  what will have failed.
- **B6 (rev 5) — extracting the eligibility predicate is
  behavior-preserving.** Q#JR14f shares one predicate between
  `commit_to`'s preflight, `probe_display_target`, and `display_buffer`'s
  exact-target arm rather than writing a third copy. The bet is that the
  two existing callers' behavior survives the extraction unchanged —
  core unit tests pin the `Option<BufferId>` matrix, and
  `bottom_panel_stage1_acceptance` catches placement-level drift, which
  is why it is in the gate list. The alternative has no extraction risk
  and a certain cost: a future eligibility rule added to one copy reopens
  Q#JR14f's exact hole. Taking the risk tests can catch over drift they
  cannot.

---

## 9. Gates

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings    # own step
cargo test --lib
cargo test --lib --features crdt
cargo test --test journey_acceptance
cargo test --test dired_acceptance
cargo test --test find_file_acceptance                   # P6, unmodified
cargo test --test gpu_initial_target_acceptance          # §2.9 supersession
cargo test --test theme_faces_acceptance                 # EditorState::open caller
cargo test --test m4_acceptance -- --skip basedpyright   # 4 open() callers
cargo test --test bottom_panel_stage1_acceptance         # commit_to touches display
PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu
cargo test --workspace -- --skip basedpyright
git diff --check
```

`m4_acceptance` and `theme_faces_acceptance` call `EditorState::open`
directly (§2.2) — the unification's blast radius. `find_file_acceptance`
and `gpu_initial_target_acceptance` encode contracts this PR preserves
(§4.6) and supersedes (§2.9). `bottom_panel_stage1_acceptance` is
included because `commit_to` scopes the frontend that `display`'s
placement policy resolves against and Q#JR14f extracts the exact-target
eligibility rule that suite already pins.

---

## 10. Sequencing

**1a implements after PR #177 merges.** #177 touches `src/daemon.rs` and
`src/editor.rs`; §2.8's reassert logic sits next to its census work.
#179 also touches `src/editor.rs`.

No dired code is in flight — #169 and #171 are docs-only and no open PR
touches `builtin/runtime/dired.lua` (verified 2026-07-26). 1a's dired
change (a handler registration, plus wrapping `open_directory`'s
post-await commit in `commit_to`) does not collide with dired Stage 2,
which is unapproved for implementation. The `commit_to` wrap is a larger
dired change than rev 2's, touching the body Stage 2's rename work also
touches.

**Rev 5 — decided: 1a stays ahead of dired Stage 2.** Stage 2 is not in
implementation, and the scoped commit boundary gives it a better shape to
build on than it would have had — a rename transaction across five path
owners is exactly the kind of multi-step commit that wants a validated,
scoped destination rather than ambient state. **Obligation this creates:**
when 1a lands, dired Stage 2 re-scouts and revises its framing around
`commit_to` before implementation; that revision is a prerequisite of
Stage 2's branch, recorded here and in `docs/active-work.md` so it is not
discovered late.

---

## 11. Numbered decisions

- **Q#JR1** `EditorState::open` adopts `resolve_target_buffer` wholesale.
- **Q#JR1a** The hook fires outside the core borrow.
- **Q#JR2** *Withdrawn (rev 2)* — its premise was false.
- **Q#JR3 (corrected rev 6)** Startup keeps using
  `replace_active_buffer`, which switches the **active** window — not
  because it drops the old scratch (it does not, and never did) but
  because an `install_buffer_in_window` elsewhere would load the file
  while leaving the user looking at scratch. Removing the stale scratch
  buffer is separate work.
- **Q#JR4** Startup errors gain the `cannot open {path}: ` prefix.
- **Q#JR5** `resolve_target_buffer` returns a typed `ResolvedTarget`.
- **Q#JR5b** Both `HookKind` types are written path-qualified.
- **Q#JR6** Rust creates no buffer for a directory.
- **Q#JR7** `path.open-directory` is a short-circuit **user-only** chain;
  builtins do not subscribe; dired is a replaceable fallback slot.
- **Q#JR8** `ResolvedTarget::Directory` carries an explicitly normalized
  path.
- **Q#JR9** The bootstrap reply names the destination window's buffer —
  absent a synchronous claimant, LOCAL's primary document buffer, not
  necessarily scratch. Accepted and documented.
- **Q#JR9b (rev 6)** That id is **re-read after the dispatch**, and the
  arm rehomes through `non_side_target` rather than returning early: a
  synchronous resolver may already have replaced the buffer.
- **Q#JR10** An unclaimed directory with the handler cleared exits 0 with
  a status message.
- **Q#JR12 (observation, rev 6)** A directory argument suppresses
  desktop restore structurally, via `had_file = file.is_some()`. No work,
  no pin.
- **Q#JR13** `display_file` keeps its directory-is-an-error contract.
- **Q#JR14** The destination `{frontend, window, buffer}` is captured at
  resolve time; `commit_to` preflights and scopes the **entire**
  post-await commit.
- **Q#JR14b** A `commit_to` callback must not await; enforced, not
  documented.
- **Q#JR14c** Stale intent loses to the user: a replaced destination
  buffer fails closed.
- **Q#JR14d** `dest` is nonconstructible userdata with a read-only
  `window()` accessor — not a table a listener can mutate or Lua can
  forge.
- **Q#JR14e** A **separate** scoped frontend override, resolved ahead of
  the interactive origin and also swapping `core.active_frontend`.
  `commit_to` never enters `InteractiveCommandOrigin`.
- **Q#JR14f** Preflight establishes **replaceability** via the same
  `Option<BufferId>` eligibility predicate used by
  `probe_display_target` and `display_buffer`; `commit_to` passes `None`
  because its replacement does not exist yet.
- **Q#JR15** A raising resolver callback stops the chain **and**
  suppresses the fallback, reported through `*errors*` and the status
  line.

---

## 12. Branch and PR plan

One feature, one branch, one PR: `journey-stage1a-directory-open`.

1. Commit this framing.
2. Unification (§3) + P4, P5, P7, P8.
3. `ResolvedTarget` + the directory arm + the resolver chain, fallback
   slot, and error policy (§4.1–4.3) + N7, N8, N9, N10; `display_file`'s
   preserved contract (§4.6) + P6.
4. The scoped frontend override + the shared eligibility predicate
   (Q#JR14e, Q#JR14f), then `commit_to` and the opaque destination
   (§4.4) + N4, N6, N6b, N6c, P1, P2, P3. The override and the predicate
   extraction land first as separable core changes: both are testable
   without dired, and the predicate's `Some(current)` / `Some(other)` /
   `None` unit matrix plus `bottom_panel_stage1_acceptance` must prove the
   extraction behavior-preserving before anything depends on it.
5. `tests/journey_acceptance.rs` (§5) + N1, N2, N3, N5, N11.
6. `COHERENCE.md` §2 verdict table and §20 Priority 1 rewritten per §25;
   `docs/gpu-initial-target-framing.md` Q#GT6 + acceptance 10 amended for
   the superseded directory case (§2.9); `docs/agent-handoff.md` §1 and
   `docs/active-work.md` updated.
