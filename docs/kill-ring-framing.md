# Kill ring — framing (Arc 2, editing table stakes) — rev 3

pmacs has Emacs keybindings on a one-slot clipboard: `C-w`/`M-w`/`C-y`
cut/copy/paste through `EditorCore.clipboard_slot`, a single `Vec<u8>`.
Kill something, kill something else, and the first kill is gone. There
is no `C-k`, and `M-y` is unbound. This arc adds the real thing: kills
accumulate in a ring, consecutive kills append, `C-y` yanks the head,
`M-y` immediately after a yank cycles older entries.

Roadmap: `docs/roadmap-2026-07.md` Arc 2 ("real kill ring + `M-y`").

**Rev 2** rebuilt the design around per-frontend command boundaries
after review showed `dispatch_key` is not the only input path. **Rev 3**
closes the second review's blockers: pointer gestures join the boundary
table; the shared ring gets stable entry identities so per-frontend
state survives other frontends' mutations; and two shipped bugs the
review surfaced move into scope — **GPU `Ctrl-V` paste is silently
dropped for semantic frontends**, and inbound paste fires no
`buffer.after-edit` anywhere.

## Ground truth (scouted + twice review-verified; as of `4c4295d`)

Input paths that reach a buffer or move point — **only the first runs
Lua command bodies**:

1. **Round-tripped chords** (`dispatch_key`): every CTRL/ALT chord
   (`src/optimistic.rs:134-142`) — all kill/yank commands, both
   frontends — plus all TUI keys. Commands run via `Action::Run`
   (`src/editor.rs:707`) or the self-insert fallback (`:730`); the
   post-command revision check fires `buffer.after-edit` (`:739`).
2. **GPU optimistic edits**: bare typing, Enter/Tab, Backspace/Delete
   arrive as CRDT ops at `handle_remote_crdt_op` (`src/daemon.rs:1941`)
   — no command, no dispatch_key. (Fires `after-edit` itself.)
3. **Pointer gestures**: grid `Mouse` → `dispatch_mouse`
   (`src/editor.rs:1114`) and the semantic `Pointer` path (`:1253`)
   move the cursor and set/clear selections — **no command boundary**.
   Right-click additionally opens the context menu.
4. **Inbound OS paste**: `FrontendEvent::Paste` → `paste_inbound`
   (`editor_core.rs:2096`) — **but only on the grid path**
   (`apply_event`, `src/daemon.rs:2241`). The semantic input
   dispatcher **drops `Paste`** (`apply_semantic_input_event`,
   `src/daemon.rs:2200`, `_ => {}` with a "no grid-less effect yet"
   comment). pmacs-gpu always negotiates semantic render
   (`pmacs-gpu/src/attach.rs:259`), so **GPU `Ctrl-V` is a no-op
   today** — a shipped bug. Where paste *does* land (TUI bracketed
   paste), it fires **no `buffer.after-edit`** — a second shipped gap
   (LSP never sees pasted text). And the grid handler **trusts the
   client-supplied payload id**: it sets `core.active_frontend` from
   `Paste.frontend_id` rather than the dispatcher's authenticated
   `source` (`DispatcherEvent::FrontendEvent { source, event }`,
   `src/daemon.rs:1428`) — a third shipped gap: a forged payload id
   pastes into *another frontend's* active window.
5. **Context menu**: `menu_invoke_active` (`src/editor.rs:946`) calls
   `invoke_command` directly — no rotation, no post-command check.
6. **`M-x`**: the minibuffer shadow returns before the post-command
   check (`src/editor.rs:664`), so `M-x`-invoked editing commands also
   miss `after-edit` today.

Clipboard: `clipboard_slot` + `pending_clipboard: Option<(FrontendId,
Vec<u8>)>` (`editor_core.rs:213,219`); `InstanceSignal::Clipboard`
(v6-floor) goes **to the originating frontend only**
(`daemon.rs:949-956`); GPU writes arboard, TUI writes OSC 52. External
content is visible to the daemon only via path 4.

`buf:replace` (`mod.rs:1216`) uses the Lua buffer edit path
(intercepts, then `notify_buffer_edit_to_windows`, `mod.rs:1344`) and
queues a daemon-origin CRDT op — rotation needs **no protocol change**,
but intercepts may alter/reject the edit, so callers must verify the
applied text.

Frontend lifecycle: `SessionDetached` (`src/daemon.rs:1574`) already
prunes six per-frontend maps; anything new that is keyed by
`FrontendId` must join that cleanup, and Lua-side per-frontend tables
need a detach signal (none exists).

Nothing tracks the previous command; no per-command hook; `C-k`/`M-y`
unbound; `M-d`/`M-BS`/`C-BS`/`C-h`/`C-DEL` discard deleted bytes; no
Emacs mark; `ed.region/cursor/delete_region/goto_byte`, `buf:slice`,
`pmacs.frontend.id()` (reads `active_frontend`) all exist; no
`clipboard_get`/`clipboard_set`.

## Decisions

### Q#KR1 — Lua ring on a Rust command-boundary substrate

The ring, append policy, yank sessions, and commands live in
`builtin/runtime/killring.lua`. Rust grows: the command-boundary
substrate (Q#KR2), `ed.clipboard_set(bytes)` + `ed.clipboard_get()`,
the **semantic-paste fix** and the **after-edit delivery fix**
(Q#KR10), the detach cleanup + `frontend.detached` hook (Q#KR11), and
the module load line in `EditorState::new`. The ring table is
daemon-global (shared across frontends, like the Emacs daemon);
everything session-shaped is per-frontend and identity-checked against
the shared ring (Q#KR4/6/7).

### Q#KR2 — Per-frontend command boundaries; every input path updates them

`EditorCore.command_history: HashMap<FrontendId, CommandBoundary>`,
`CommandBoundary { this: Option<String>, last: Option<String> }`.
`ed.last_command()` reads the active frontend's entry. Two operations:
**`rotate(fid, name)`** (`last = this; this = Some(name)`) and
**`break_chain(fid)`** (`this = None` — the next rotation yields
`last = None`, failing every chain/session check).

The boundary table — one row per input path (rev 3 adds pointer
gestures and the unified paste route):

| path | site | operation |
|---|---|---|
| keybound command | `dispatch_key` `Action::Run` | `rotate(fid, name)` |
| typed char (round-trip) | self-insert fallback | `rotate(fid, "buffer.self-insert")` |
| unbound key | `dispatch_key` unbound arm | `break_chain(fid)` |
| GPU optimistic edit | `handle_remote_crdt_op` | `break_chain(source)` |
| **pointer gesture** | `dispatch_mouse` (grid) **and** the semantic `Pointer` handler — any press/drag/release that moves point or changes the selection; menu-opening right-click included | `break_chain(fid)` |
| inbound OS paste | the **unified** paste route (Q#KR10) | `break_chain(source)` — the **authenticated** id, never the payload's |
| menu item | `menu_invoke_active` | `rotate(fid, name)` |
| `M-x` accept | `pmacs.command.invoke_interactive` (new) | `rotate(fid, name)` |

Pointer scope note: scroll-wheel events that move only the viewport do
**not** break the chain (point is untouched); anything that relocates
the cursor or edits the selection does. Emacs behaves the same way
(`mwheel-scroll` preserves `last-command` chains; `mouse-set-point`
breaks them).

`invoke_interactive(name)` rotates then invokes; `editor.execute-
command`'s accept uses it. Plain `pmacs.command.invoke` stamps nothing
(public programmatic API). This preserves rev 2's Emacs-verified `M-x`
matrix: `C-k`→`M-x kill-line` no-append; `M-x kill-line`→`C-k` append;
`M-x` twice no-append.

### Q#KR3 — What feeds the ring (v1 command set)

Unchanged from rev 2: `C-w`/`M-w`/`C-y` bodies become ring-aware
(context menu inherits by name); new `C-k` `edit.kill-line` (chunked
`buf:slice` newline scan; at EOL kills the newline) and `M-y`
`edit.yank-pop`. **Deferred**: word kills — `M-d`, `M-BS`, `C-BS`,
`C-h`, `C-DEL` all share the discard-the-bytes deleters and keep plain
deleting until the Rust deleters return bytes.

### Q#KR4 — Append requires success **and** an unmoved head (stable ids)

Rev 2's per-frontend `last_kill_ok: bool` corrupts the shared ring
under interleaving (review blocker): A kills, B kills, A kills again —
A's flag is still true, so A appends onto **B's** entry.

So ring entries carry a **stable identity**: each push mints a
monotonically increasing `id`; entries are `{id, text}`. The
per-frontend kill state is **`last_kill_id`** (nil = no live kill
chain), and the append condition becomes:

> `ed.last_command()` is a kill command **and** this frontend's
> `last_kill_id` is non-nil **and** equals the current head's `id`.

A-kill/B-kill/A-kill: A's `last_kill_id` names A's entry, the head is
B's → **push fresh**, B's kill intact. A successful push/append sets
`last_kill_id = head.id`; every failed or no-op kill-family invocation
clears it (rev 2's success rule, carried forward). Appends keep the
entry's `id` (the entry is *extended*, not replaced — a same-frontend
chain keeps appending). The appended head re-syncs to the acting
frontend's OS clipboard.

### Q#KR5 — The ring

Entries `{id, text}`, most-recent first, shared. Cap default **60**;
`pmacs.killring.max([n])` validated as in rev 2 (**non-finite
rejected** — `NaN`, `math.huge`; ≥ 1; floored; **shrink trims
immediately**). Duplicate-of-head pushes collapse (no new id).
`pmacs.killring.list()` returns texts for introspection/tests.

### Q#KR6 — Yank: per-frontend sessions, snapshot + stable cursor

`C-y` (unchanged flow from rev 2, session shape corrected):
1. Slot check: `ed.clipboard_get()` non-empty and ≠ head text → push it
   (external content joins the ring).
2. Ring empty → `"kill ring empty"`, no session.
3. `start` = region lo or cursor; `ed.clipboard_set` if the slot
   differs; `ed.clipboard_paste()` (existing insert-over-region,
   `cursor = start + len`).
4. **Session (per `FrontendId`)**: `{buffer, start, end, entry_id,
   text}` — `text` is the snapshot actually inserted (rev 2 claimed to
   verify against it but never stored it — review catch), `entry_id`
   is the ring entry's stable id (an index would be shifted by any
   other frontend's push — review blocker). Created only on successful
   paste.

**OS clipboard scope** (unchanged, explicit): the ring is shared; OS
mirroring is local to the acting frontend — `pending_clipboard`'s
existing `(FrontendId, bytes)` shape, and the right privacy call for
frontends on different machines.

Named limitation (unchanged): an external copy is invisible until
pasted; `C-y` before any paste yanks the ring head. GPU `Ctrl-V`
covers the direct gesture — **and actually works after this PR**
(Q#KR10 fixes the semantic drop).

### Q#KR7 — Yank-pop: stable-id rotation, snapshot-verified

`M-y` is valid only when, for the acting frontend: (1) `last_command`
is `edit.paste`/`edit.yank-pop`; (2) a live session exists; (3) the
session's buffer is active; (4) `buf:slice(start, end) ==
session.text`. Any failure → refuse with status, drop nothing into the
buffer, **and create/keep no session** (a second invalid `M-y` cannot
ride the first's name-stamp).

When valid:
1. Locate `session.entry_id` in the ring; **absent (evicted or
   trimmed) → invalidate the session and refuse.** Otherwise step to
   the next-older entry, wrapping. Because the cursor is an id, another
   frontend pushing entries mid-session shifts positions but not
   identity — rotation continues from where this frontend actually was
   (Emacs's `kill-ring-yank-pointer` behavior; an integer index would
   silently re-yank the wrong entry — review blocker).
2. `buf:replace(start, end, entry.text)`, then verify by re-slice.
3. On match: `ed.goto_byte(start + #entry.text)`, update `end`,
   `entry_id`, `text`.
4. On mismatch (a buffer intercept altered the edit): **accepted
   post-hoc semantics, stated as a decision** — the interceptor's
   result is left in place, the session is dropped, and the status says
   so. Kill/yank inside intercepted (round-trip/REPL-prompt) buffers is
   niche; a preflight refusal would need a new Lua-visible intercept
   probe, which is not worth the seam in v1. (Review offered both
   options; this is the explicit choice of the simpler one.)

The OS slot is untouched by `M-y`; undo treats each rotation as a
normal edit.

### Q#KR8 — No mark in v1

Unchanged: CUA selections only; `C-SPC`/set-mark deferred; `C-w`
requires a selection.

### Q#KR9 — Module shape and loading

Unchanged from rev 2: `killring.lua` holds the ring, the per-frontend
tables (keyed by `pmacs.frontend.id()`), commands, and bindings;
`default.lua` bodies delegate at invoke time; loaded explicitly in
`EditorState::new`.

### Q#KR10 — Unified paste + after-edit delivery (two shipped bugs, in scope)

**(a) Semantic paste, on the authenticated source.**
`FrontendEvent::Paste` is handled only in the grid `apply_event`; the
semantic dispatcher drops it, and the GPU is always semantic — so GPU
`Ctrl-V` does nothing today. Fix: route `Paste` **before the
grid/semantic split** (one handler for both attachment kinds) →
`paste_inbound` + `break_chain`.

That handler keys everything off the **dispatcher's authenticated
`source: FrontendId`** — `DispatcherEvent::FrontendEvent { source, .. }`
is stamped per attach session, while `Paste.frontend_id` is
client-supplied bytes. The current grid handler trusts the payload and
sets `active_frontend` from it, so a forged id pastes into another
frontend's active window (review blocker). The unified handler ignores
the payload id (logging a mismatch), targets `source`'s active view,
and calls `break_chain(source)`. A forged-id acceptance test pins it.

**(b) After-edit delivery — for the three scoped paths, not "every
command".** Three sites run edits outside `dispatch_key`'s revision
check and never fire `buffer.after-edit`: the minibuffer accept
callback (`M-x`), the menu invoke, and **paste**. Extract the
revision-compare-then-fire into a helper and call it at those three
sites. No double-fire on the keybound path.

Scope honesty (review correction): the helper keeps the existing
*active-buffer* before/after comparison, which is sound for these three
paths — kill, yank, menu Cut, and paste all edit the active buffer and
do not switch away mid-command. It is **not** a general guarantee: a
command that edits buffer A and then switches to B still evades the
compare (the check would diff B's revision). The general fix is a
buffer-aware edit epoch (any-buffer mutation counter + per-buffer hook
targeting) — deferred, named, and out of this PR's scope.

### Q#KR11 — Frontend lifecycle cleanup

`SessionDetached` already prunes six per-frontend maps
(`daemon.rs:1574`); `command_history` joins them. For the Lua side, the
same arm fires a new **`frontend.detached`** hook carrying the raw
frontend id; `killring.lua` subscribes and drops that frontend's
session/`last_kill_id` entries. (Frontend ids are monotonic, so without
this both maps grow for the daemon's lifetime — review note.) The hook
is tiny and generally useful (the first frontend-lifecycle hook).

## Phasing

One PR. In-diff order: Rust substrate (command boundaries + all eight
table rows, `clipboard_get/set`, unified paste route, after-edit helper
+ three call sites, `frontend.detached` + detach cleanup, module load)
→ `killring.lua` + `default.lua` bodies → acceptance.

## Bets (score at close)

1. **The eight-row boundary table is complete** — no further input path
   can edit a buffer or move point without rotating or breaking.
   (Falsified twice at smaller sizes; this is the bet to watch.)
2. **Stable entry ids make shared-ring interleaving safe** — no
   append-corruption, no index-shift mis-yanks, eviction invalidates
   cleanly.
3. **The unified paste route regresses nothing on the grid path** while
   making GPU paste work at all.
4. **Q#KR10's helper closes the after-edit hole at all three sites**
   with no double-fire.

## Deferred (named)

Unchanged from rev 2: word kills (five bindings) pending
bytes-returning deleters; `C-SPC`/mark; clipboard watching; ring
browser; ring persistence; `C-u C-y` / `C-M-w`. Plus: a Lua-visible
intercept probe (would upgrade Q#KR7's post-hoc accept to a preflight),
and a **buffer-aware edit epoch** so after-edit delivery covers
commands that edit one buffer then switch to another (Q#KR10b's named
general fix).

## Acceptance (dispatch_key-driven; multi-frontend via distinct `FrontendId`s)

Chain mechanics — rev 2 set, plus:
- **Pointer break, kill side**: `C-k`, click elsewhere in the same
  buffer (grid mouse path *and* semantic pointer path), `C-k` → two
  entries.
- **Pointer break, yank side**: `C-y`, click elsewhere, `M-y` →
  refused (not a yank), buffer unchanged at both locations.
- Wheel-scroll does **not** break a kill chain.

Shared-ring interleaving:
- **A-kill / B-kill / A-kill** → three entries; B's text intact
  (the `last_kill_id` blocker case).
- **A `C-y` / B kills / A `M-y`** → A rotates from its own entry
  (stable id), not from B's shifted position.
- Eviction mid-session (B pushes past the cap until A's entry drops) →
  A's `M-y` refuses cleanly.

Paste (Q#KR10a):
- `Paste` reaches the buffer on the **semantic** path (the GPU case —
  asserting the shipped bug is fixed), breaks the chain, and fires
  `buffer.after-edit` exactly once; grid path unchanged.
- **Forged-id paste**: frontend A sends `Paste` whose payload
  `frontend_id` names B → the text lands in **A's** active view, A's
  chain breaks, and B's state (view, chain, sessions) is untouched.

Hook delivery (Q#KR10b):
- `after-edit` probe fires exactly once for menu Cut, `M-x
  edit.kill-line`, and a paste; no double-fire for keybound `C-k`.

Lifecycle (Q#KR11):
- `SessionDetached` clears the frontend's `command_history` entry and
  fires `frontend.detached`; the killring module's tables for that id
  are gone (probe via `pmacs.killring._debug_sessions()` or list
  introspection).

Everything else carries over from rev 2: `M-x` three-direction matrix,
failed-kill/failed-yank no-ops, GPU-optimistic and unbound-key breaks,
slot sync + external-content integration, yank-over-selection, cap
validation (`math.huge`/`NaN`) and shrink-trim.
