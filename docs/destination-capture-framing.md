# A destination capture any async continuation can use

**Status: framing pass, revision 1. Pre-implementation. Awaiting
approval.**

**A prerequisite lane. PR #227 (git Stage 1) blocks on it**, and its
P1a review finding is the reason this exists.

---

## 1. Why, and why as its own lane

PR #227's review found that git's async completions mutate and display
UI without capturing the initiating frontend
(`builtin/runtime/git.lua:609`, `:854`), so a result can surface in
whichever frontend happens to be active when git exits. Run
`git.status` in frontend A, let frontend B become active, and A's panel
opens in B.

**The finding named the right mechanism.** `pmacs.window.commit_to`
exists for exactly this continuation boundary: Journey Stage 1a's
Q#JR14 built it because "the listing settles a tick or more later, and
by then the ambient frontend, selected window, and active buffer may
all name something else" (`src/editor.rs:1238-1240`).

**But it is not reachable from Lua outside one path**, which is why
this is a lane and not a line in #227:

- `commit_to` takes a `DirectoryDestinationLua`, **nonconstructible
  from Lua** by deliberate design (`src/lua_bindings/mod.rs:4256`) —
  userdata with no constructor and no setters, so a caller cannot
  fabricate a plausible triple.
- The only site that mints one is inside the `path.open-directory`
  listener dispatch (`src/editor.rs:1311`), from
  `capture_directory_destination`, which is `pub(crate)`
  (`src/editor.rs:1241`).

So any async Lua continuation that is **not** a directory open has no
way to say where its result belongs. Git is the first to need it; it
will not be the last.

Landing this inside #227 would put new Lua API surface, over another
lane's merged mechanism, inside a feature branch — the same folding
that was declined for the `scripts/gate` repair, for the same reason.

## 2. Ground truth

- **The captured data is already generic.**
  `DirectoryDestination { frontend, window, buffer }`
  (`src/editor_core.rs:159-166`) contains nothing directory-specific.
  Only its **name** and its **capture site** are.
- **The blast radius of a rename is small**: 8 references across 4
  files (`editor_core.rs`, `editor.rs`, `lua_bindings/mod.rs`,
  `lua_bindings/window_panel.rs`). Checked, not estimated.
- **`commit_to`'s preflight is four checks**
  (`src/lua_bindings/window_panel.rs:488-525`), in order: the
  requesting frontend still has a layout; the destination window is
  still live in it; **the window still shows the captured buffer**
  (Q#JR14c stale intent); and the window is not dedicated (Q#JR14f).
- **`Handle:await` refuses inside a commit scope**
  (`builtin/runtime/async.lua:87-90`) — yielding would restore the
  scope while the coroutine is still parked. Any adopter awaits
  *before* committing, as dired does.
- **Git's two continuations do not have the same shape**, and this is
  the finding that shapes the design:
  - `*git-status*` goes through `listview.open`, which resolves
    `display` with a **`"panel"`** default
    (`builtin/runtime/listview.lua:550`). It lands in the bottom
    panel, **not** in a document window.
  - `*git-diff*` calls `pmacs.window.display(buf, { select = true })`
    — the **document** target, deliberately, "so the status panel it
    was invoked from stays visible beside it"
    (`builtin/runtime/git.lua:852-854`).

## 3. The tension this lane has to resolve

`DirectoryDestination.buffer` exists for one purpose, stated at its
definition: *"what that window held at capture time, so **stale intent
loses to the user**"* — a user who replaced the buffer while work was
in flight is newer information than the request.

**That predicate is right for a document replacement and wrong for a
panel.** The git status panel does not replace the captured window's
buffer; it opens in the bottom panel beside it. Refusing to show it
because the user switched files in the document window would be a
refusal with no relationship to what the continuation actually does —
the panel case would inherit a check about a window it never touches.

Meanwhile the diff case *is* a document replacement, and wants exactly
the dired semantics.

So a single one-size destination either **over-refuses** the panel case
or **under-checks** the document case. Q#DC-2 is where that gets
decided, and it is the substance of this lane.

## 4. The change, in outline

- **A Lua-reachable capture**, returning the same nonconstructible
  userdata for the *current* frontend and its document window.
- **Generic naming.** `DirectoryDestination` becomes something that
  does not lie about a git panel; `capture_directory_destination` and
  the userdata type follow. 8 references (§2).
- **The directory path keeps behaving exactly as it does today** — this
  lane generalizes the capture, it does not change Journey Stage 1a's
  semantics.
- **No adopter in this lane.** Git's adoption is #227's, after this
  lands. A prerequisite that also converts its first consumer makes the
  two impossible to review separately.

## 5. Open questions

### Q#DC-1 — what does the capture take as arguments?

*My vote: **no arguments** — capture the acting frontend and its
document window from the ambient state at call time.* That is what the
existing `capture_directory_destination(frontend, window)` is handed by
its one caller, and a Lua-supplied frontend id would reintroduce the
fabrication hole the userdata design closes.

### Q#DC-2 — one destination shape, or a panel/document distinction? **(the substantive one)**

§3 is the problem. Three candidates:

1. **One shape, all four checks.** Simplest; over-refuses the panel
   case, and the refusal reason would be about a window the panel does
   not touch.
2. **One shape, preflight parameterized by the continuation** — the
   caller declares whether it is replacing the captured window's
   buffer, and the stale-intent check applies only then.
3. **Two capture kinds**, document and panel, with different preflights.

*My vote: **(2)***. The four checks are not equally applicable, and
which apply is a property of *what the continuation does*, which only
the caller knows. (3) duplicates the liveness checks that both need;
(1) ships a refusal that will read as a bug the first time a user hits
it.

**I hold this one loosely.** It is the design decision of the lane, and
(1) has a real argument — a uniform rule is easier to reason about than
a parameterized one, and over-refusal is at least *safe*.

### Q#DC-3 — what is the type called?

*My vote: **`ViewDestination`***, with `pmacs.window.capture_destination()`
as the Lua entry point. It names what it is — a place in a view where a
continuation's result belongs — without claiming a directory or a
buffer kind.

The Q#JR14 doc comments should keep their references intact; a rename
that orphans the rationale is worse than a slightly stale name.

### Q#DC-4 — is the capture refused when there is no document window?

`capture_directory_destination` already returns `None` when the
frontend has no document window (`src/editor.rs:1236`). *My vote:
**return `nil`, and require every adopter to handle it***, rather than
inventing a fallback destination. A continuation with nowhere to land
should say so, and #227's adopter should degrade to today's ambient
behaviour with a status message rather than silently guessing.

## 6. Verification

- **A captured destination survives a frontend switch**: capture in A,
  make B active, commit, and assert the result lands in **A**. This is
  P1a's actual failure and the reason the lane exists — asserting only
  that the API returns userdata would pass on a capture that does
  nothing.
- **A fabricated destination is still refused** — the existing Q#JR14d
  guarantee, re-asserted after the rename so the generalization cannot
  quietly open the hole it was built to close.
- **Each preflight refusal is witnessed by its own case**: frontend
  gone, window gone, stale buffer, dedicated window — and, under
  Q#DC-2's answer, that the stale-buffer refusal does **not** fire for
  a continuation that declared it is not replacing that buffer.
- **`nil` when the frontend has no document window** (Q#DC-4).
- **The directory path is unchanged** — dired's existing acceptance
  coverage passes untouched. **If any dired test needs editing, the
  generalization changed Journey Stage 1a's semantics** and that is a
  stop signal, not a fixup.
- **`Handle:await` still refuses inside the scope**, including through
  `pmacs.async.yield_to_next_tick` if the worker-identity lane's Q#W-7
  has landed by then; if it has not, this lane does **not** add that
  guard — it belongs to that lane and duplicating it would produce a
  conflict for no benefit.

**What this will NOT prove:** that git surfaces in the right frontend —
that is #227's adoption, after this lands. This lane ships the
mechanism and one set of tests for the mechanism.

## 7. Not in scope

**Adopting the capture anywhere**, including git (#227 does that) and
including migrating other async continuations that have the same latent
bug — worth an audit, not this lane's work. Changing Journey Stage 1a's
directory semantics. The `commit_to` scope guard for
`yield_to_next_tick` (worker identity Q#W-7). Any protocol change —
this is entirely core + Lua bindings. Panel geometry or placement
policy, which is the bottom-panel arc's.
