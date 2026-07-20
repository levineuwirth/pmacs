# Kill-ring browser + persistence — framing (Lua lane)

**Revision 2 preserved 2026-07-20. Status: PARKED by user decision.**
There is no implementation and no PR. The original scout was based on
`0efb5cd`; compile-mode and later substrate have since landed. Before
this feature is un-parked, re-scout every cited path and invariant
against the then-current `githubsucks/main`, revise the framing, and
obtain fresh approval. This document preserves the reviewed design; it
is not implementation-ready ground truth.

The handoff §6 backlog item: a browsable panel over the kill ring
(`M-x edit.browse-kill-ring` — Emacs's browse-kill-ring shape: pick
an entry, it inserts at the point you came from), plus the ring
surviving restarts via `pmacs.state`. Browser ownership stays in
`builtin/runtime/killring.lua`; two narrow substrate touches ride
along (an after-edit suppression latch and a `state.write_private`
Lua binding — both forced by R1 findings, both named). Zero file
overlap was claimed against the then-concurrent compile-mode lane;
compile-mode has since landed, so that concurrency claim is historical
and the touch set must be re-scouted.

Revision 2: R1 findings — exactly-once `buffer.after-edit` via a
per-dispatch suppression latch instead of a tolerated double-fire
(all-must-succeed is failure isolation, not an idempotence license,
and the autosave manual-run precedent rides a path that never
double-fires); per-frontend `origins[fid]` (a single slot lets A's
RET insert into B's origin — destructive, not an inheritable
limitation); the settled context guard around the origin insert,
with the manual hook run skipped on a context switch (gap named
against the origin-pinned fan-out deferral); hex-encoded
persistence (ring bytes need not be UTF-8, and `state.read` is
`read_to_string`-backed) with `?`-sanitized previews; 0600
private-write posture via a new `pmacs.state.write_private`
binding; no-throw three-state seeding that cannot interrupt a kill
and never overwrites a file it could not read; write-on-mutation
persistence closing the daemon-SIGTERM loss window (before-quit
never runs there). The two-file touch-set claim is dead: the latch
and the binding put `src/editor.rs` + `src/lua_bindings/mod.rs` in
scope.

## Original ground truth (as of `0efb5cd`; stale, re-scout required)

- **Ring internals (post-#111).** Entries `{id, text}` MRU-first;
  `list()` returns texts only (`killring.lua:76-81`); `push_entry`
  collapses duplicate-of-head keeping the id; chains and yank
  sessions are per-frontend and id-checked; `kill_push` mirrors the
  acting frontend's OS clipboard; `yank()` inserts via
  `ed.clipboard_paste()` and opens an M-y session; ring text is
  byte-clean buffer content — NOT guaranteed UTF-8.
- **`buffer.after-edit` is contractually exactly-once per
  dispatch.** "Fired after a key dispatch that mutated the active
  buffer" (`builtin/hooks/default.lua:10-13`); all-must-succeed
  means one callback's failure doesn't block the others — failure
  isolation, NOT permission to run side-effectful user callbacks
  twice. The automatic fire site interleaves the Q#AP9 typed-edit
  choreography (arm before the fan-out, clear after,
  `src/editor.rs:808-827`) — any suppression mechanism must leave
  that intact. Autosave's manual run (`autosave.lua:233`) rides the
  async-tick path, which the automatic dispatch check never covers
  — there is NO in-tree precedent for a double-fire.
- **The automatic check compares the ACTIVE buffer only**
  (`src/editor.rs:817-819`; `with_after_edit_check` likewise, with
  the same honestly-documented scope) — a visit that switches
  panel→origin mid-dispatch compares different buffers' counters.
  Revisions bump exactly once per applied edit op
  (`src/buffer.rs:457-463`), so both the unequal and the equal
  cross-buffer comparison are deterministically constructible in
  tests by counting edits.
- **`pmacs.hook.run` is a single Lua binding**
  (`run_hook_from_lua`, `src/lua_bindings/mod.rs:5118-5128`) — one
  choke point that can observe "Lua ran buffer.after-edit" and
  record it for the dispatch's automatic check.
- **Listview rows are single lines by construction.** `render()`
  joins one `row.text` per line and maps line→item by index
  (`builtin/runtime/listview.lua:50-62`) — an embedded `\n` shears
  the map. Panels are persistent read-only buffers with
  `set_round_trip_input`, buffer-local RET/SPC/n/p/g/q, and a
  single panel record per name; `on_visit` runs inside a normal
  RET dispatch, so the boundary rotates to `listview.visit` — not
  in `KILL_CHAIN`, and `yank_pop` requires
  `last_command ∈ {edit.paste, edit.yank-pop}`
  (`killring.lua:308-320`).
- **State strings are UTF-8 at every boundary.** `state::read` is
  `read_to_string` (`src/state.rs:169-175` — a non-UTF-8 file is an
  Io error, and the Lua binding RAISES it,
  `src/lua_bindings/mod.rs:2184-2191`); the `write` binding takes a
  Lua→Rust `String` (UTF-8-validated at conversion). Arbitrary
  ring bytes therefore cannot round-trip raw — the on-disk payload
  must be ASCII-safe.
- **A 0600 private-write path exists Rust-side only.**
  `state::write_private` (0700 dir + 0600 file,
  `src/state.rs:203-205`, re-tightening pre-existing lax modes)
  with a Unix-mode unit-test precedent (`state.rs:459-478`); the
  Lua surface exposes only plain `write` (umask-default file
  modes). A kill ring holds passwords and private buffer contents
  — default-on persistence cannot ship world-readable.
- **State becomes available AFTER the Lua chunks load** on every
  real entry point (`src/daemon.rs:470`, `src/editor.rs:1722`;
  `EditorState::new()` configures nothing —
  `tests/persistence_acceptance.rs:75-84`). Restore must be lazy;
  the `StateDir` app-data tempdir fixture
  (`persistence_acceptance.rs:20-34`) exercises exactly this shape.
- **`editor.before-quit` does not run on daemon SIGTERM/SIGINT** —
  the dispatcher exits directly. Quit-time-only persistence would
  silently lose the ring on the most common daemon restart path.
  recentf and saveplace both persist per-event (every open / every
  save), so write-on-mutation has direct house precedent.
- **Cursor/selection substrate**: `switch_buffer` zeroes the window
  cursor; `goto_byte` clamps; the zero-length-anchor rule and
  right-gravity `translate` repair are settled (indent/editops);
  the UTF-8 scalar validator exists only inside editops.lua — the
  browser needs a second copy (with `translate`, a third) —
  promoting both to a shared util is named, not smuggled.
- **Chords at the original scout:** no canonical Emacs binding.
  Compile-mode has since merged and owns `M-g n`/`M-g p`, ``C-x ` ``,
  `M-!`, and buffer-local `C-c C-k`. The browser still binds NOTHING
  in this preserved v1 design, but any future chord decision must be
  re-scouted against the current keymap.

## Decisions

### Q#KB1 — Shape and touch set

`pmacs.killring.browse()` + command `edit.browse-kill-ring` (M-x
only; `C-c k` deferred until compile-mode's chords settle), and
`pmacs.killring.persist([on])` — getter/setter, the `max()` shape,
**default on**, inert without a state dir.

File touch set (the R1 revision): `builtin/runtime/killring.lua`;
`src/lua_bindings/mod.rs` (the `state.write_private` binding + the
latch mark in `run_hook_from_lua`); `src/editor.rs` (latch honor at
the two automatic fire sites); `tests/killring_browser_acceptance.rs`.
No loader change (no new chunk). The old “zero overlap with the
compile-mode lane” concurrency claim is retired because compile-mode
has landed; a fresh scout must revalidate the touch set itself.

Empty ring → status `"kill ring empty"`, no panel.

### Q#KB2 — Panel rows: sanitized single-line previews, text-as-item

One row per entry, MRU-first:
`<index>: <sanitized first line>` + ` … (N lines, M bytes)` when
truncated or multiline. Sanitization walks the preview budget
(~72 bytes) with the scalar validator: valid UTF-8 scalars pass
verbatim, TAB becomes one space, and any byte that is not part of a
valid scalar renders as `?` — ring entries need not be UTF-8
(ground truth), and continuation-byte trimming alone only protects
well-formed text. The elision cut lands on a scalar boundary. Never
a raw `\n` or raw invalid byte in `row.text`.

`item` = the ENTRY TEXT captured at render — RET inserts the EXACT
original bytes, sanitization is presentation-only (pinned with an
invalid-byte entry). Stale-safe by construction: concurrent ring
movement cannot misinsert; `g` re-renders. Id-carrying rows
deferred.

### Q#KB3 — Insert at the origin: per-frontend origins, settled guard

`origins[fid]` — `browse()` stores
`{ win, buf, cursor }` under the ACTING frontend's id;
`frontend.detached` clears its slot (joining killring's existing
detach cleanup). A's RET can never act on B's origin (R1 finding:
a shared slot was destructive, not a limitation) — simultaneous
A-opens-then-B-opens browsing is pinned.

`on_visit(text)` (acting fid resolved at RET time):

1. No `origins[fid]` or `origin.buf:is_valid()` false → status
   *"kill-ring: origin buffer is gone"*, stay in the panel.
2. `switch_buffer(origin.buf)`; `goto_byte(origin.cursor)`
   (clamps). Snapshot `win0 = window.current()` and the origin
   buffer handle — the deliberate switch is complete; everything
   after this point follows the settled discipline.
3. One pcall'd `buf:insert(pos, text)`, exact-triple checked.
   Rejected → status, nothing further.
4. **Context guard**: if `window.current() ~= win0` or
   `window.buffer() ~= origin.buf`, the insert intercept switched
   context — skip ALL contextual fix-up (cursor, selection, AND
   the Q#KB5 manual hook run; running it would make every callback
   observe the wrong buffer), status *"kill-ring: context changed
   during insert"*. The origin's didChange visibility gap in this
   pathological case rides the origin-pinned after-edit fan-out
   deferral (pair.lua's Q#AP7 named the same scope), pinned by a
   probe — not concealed.
5. Clean → `goto_byte(pos + #text)`; transformed → status +
   right-gravity translate-and-clamp. After any landed edit:
   `clear_selection()` unconditionally, then the Q#KB5 manual run.

### Q#KB4 — A browser insert is a plain insert

No yank session (M-y right after refuses — pinned), no ring
rotation, no clipboard mirror, no chain or pending-marker contact.
The ring is read-only to the browser.

### Q#KB5 — Exactly-once after-edit: the suppression latch

R1 rejected the double-fire, so the narrow substrate lands now:

- A per-dispatch latch (an `AfterEditLatch(Cell<bool>)` Lua
  app-data, reachable from both the binding and the dispatcher
  without new core plumbing). `run_hook_from_lua` sets it when the
  hook being run is `buffer.after-edit`
  (`src/lua_bindings/mod.rs:5118` is the single choke point).
- `dispatch_key`'s post-command check and `with_after_edit_check`
  clear the latch on entry and SKIP their automatic fire when it
  is set on exit. The Q#AP9 typed-edit choreography is untouched:
  the record is finished before the check either way and only
  re-armed when the automatic fan-out actually runs; manual runs
  continue to observe no record (the #110 contract).
- Behavior-neutral elsewhere: no in-tree Lua runs
  `buffer.after-edit` during a key dispatch today (autosave's
  manual run is tick-path, outside both fire sites; the latch is
  cleared at the next dispatch entry regardless).

The browser's visit then runs the hook manually once, after a
landed insert, with the origin buffer active — and the automatic
check stays silent whether the cross-buffer revision comparison
came out unequal (the common case) or equal (the miss the latch
exists to close). Acceptance asserts EXACTLY one fire in both
cases, constructed deterministically by edit counting (revisions
bump once per applied op — ground truth). The full buffer-aware
edit epoch remains deferred substrate; the latch is the minimal
exactly-once mechanism this feature needs.

### Q#KB6 — Persistence: hex lines, private writes, no-throw seeding, write-on-mutation

**Format** (`pmacs.state` key `killring`): line 1 `v1`; then ONE
lowercase-hex line per entry, MRU-first. Hex is forced by the UTF-8
boundary on `state.read`/`write` (ground truth) and restores the
house line-oriented format (hex is newline-free by construction).
Decode-side caps, named: 30 entries, skip entries over 64 KiB
decoded, stop at 256 KiB decoded total (~512 KiB file worst case —
the 2× hex tax is bounded by the caps). Any parse anomaly — bad
header, odd-length line, non-hex byte — discards the WHOLE file
(fail closed; also covers torn writes).

**Private writes**: a new `pmacs.state.write_private(key, text)`
binding routing `state::write_private` — 0700 dir + 0600 file,
re-tightening pre-existing lax modes (the Rust implementation and
its mode test already exist; only the Lua exposure is new). The
ring holds secrets; default-on persistence ships 0600 or not at
all. A Unix-mode acceptance assertion rides the existing
`state.rs:459` precedent.

**Write trigger — every ring mutation**: after `kill_push` (fresh,
append, and collapse), `copy`, `yank`'s OS-slot adoption, and a
`max()` trim, the ring is re-persisted (pcall'd; a write failure
reports on the status + `pmacs.error` channels — the
autosave-sweep convention — and never disturbs the ring
operation). This is the recentf/saveplace per-event posture, and
it closes finding 7 by construction: daemon SIGTERM loses nothing,
because nothing waits for `before-quit` (which never runs on that
path — ground truth). No quit hook at all; the loss window is a
crash between a mutation and its write.

**Seeding — lazy, no-throw, three-state**: `seed_once()` runs at
the START of `kill_push` (before any ring change — a raising seed
inside a kill path could otherwise lose already-deleted text),
`copy`, `yank`, and `list` (browse enters via `list`). It is
entirely pcall-wrapped and CANNOT throw. States:

- *unseeded* → on each call, if `persist()` is on and
  `state.available()`: attempt `state.read` under pcall.
- read ok, parse ok (or file absent) → *seeded*: entries appended
  BEHIND any in-session ring content (session kills are newer;
  this is the R1 ordering pin — a ring touched before the StateDir
  appears still restores), fresh monotonic ids, `trim()` applied,
  NO clipboard write, no chain/session state touched.
- read ok, parse bad → *seeded* with nothing: fail-closed
  emptiness; subsequent mutation writes MAY overwrite the corrupt
  file (it was readable garbage).
- read RAISED (I/O — permissions, a directory squatting on the
  key) → *io-blocked*: reported once per distinct fault
  (status + `pmacs.error`), ALL persistence writes suppressed — a
  file we could not read must not be overwritten — and the seed
  retries on subsequent ops until it succeeds.

`persist(false)` stops seeding and writes, leaves any existing
file alone; `persist(true)` re-enables (seed state unchanged).

## Bets

1. **The latch is the honest exactly-once** — a dozen lines of
   substrate beats either a tolerated double-fire or an edit LSP
   can miss, and it composes with (rather than preempts) the
   deferred buffer-aware epoch.
2. **Hex under caps is cheap enough** — ≤512 KiB worst-case file,
   zero encoding dependencies, and the UTF-8 state boundary stays
   untouched.
3. **Write-on-mutation matches how this codebase already persists**
   (recentf, saveplace) and quietly removes the whole
   quit-path/SIGTERM question.
4. **Text-as-item and M-x-only survive review** (R1-confirmed).

## Deferred (named)

- Move-to-front on browser insert; per-entry delete (`d`); a chord;
  pre-browse selection type-over; yank-session integration;
  id-carrying rows.
- Interval/debounced persistence (write-on-mutation makes it a
  performance nicety, not a correctness need).
- Byte-oriented `pmacs.state` reads (would retire the hex tax).
- Shared Lua util for `scalar_len` + `translate` (now 2× and 3×
  copied).
- Buffer-aware edit epoch; origin-pinned after-edit fan-out (both
  pre-existing substrate deferrals this framing leans on).

## Acceptance

`tests/killring_browser_acceptance.rs`, dispatch-driven (panel keys
via `dispatch_key`); persistence via the `StateDir` tempdir
fixture; multi-frontend cases on the unregistered-second-id
harness.

- Browse: 3 kills → 3 MRU-first rows + header; a multiline kill is
  ONE row with the `(N lines, M bytes)` suffix; an entry with
  invalid UTF-8 renders `?` for the bad bytes with the panel
  structure intact, and RET still inserts the EXACT original bytes
  (sanitization is presentation-only); a long entry's cut lands on
  a scalar boundary; TAB renders as a space.
- Visit: RET → origin active, text at origin cursor, cursor after
  insertion, selection cleared (zero-length-anchor variant), ring
  order unchanged, clipboard slot untouched; `n` then RET inserts
  the second entry; `M-y` right after → "previous command was not
  a yank".
- **Exactly-once matrix** (counter callback): a visit where the
  panel-pre vs origin-post revisions are UNEQUAL → one fire, with
  the origin buffer active at callback time (the automatic check
  is suppressed, not doubled); a visit engineered by edit-counting
  so the cross-buffer revisions are EQUAL → still exactly one fire
  (the latch's raison d'être); a plain non-browser edit still
  fires exactly once (latch is inert without a manual run).
- **Origins matrix**: A browses from bufA, B browses from bufB,
  A's RET inserts into bufA (never bufB) and B's RET into bufB;
  `frontend.detached` clears the detached frontend's origin slot.
- Context guard: an insert intercept that switches buffers → the
  switch stands, no cursor/selection fix-up in the new context, NO
  manual hook run (a probe callback registered for after-edit
  observes no browser-driven fire in the switched context), status
  reports; a transforming (relocating) intercept without a switch
  → translate-and-clamp, selection cleared, exactly one fire.
- Origin gone: kill the origin buffer, RET → status, panel intact,
  `g` still refreshes.
- Stale rows: render, push a new kill, RET on the old row → the
  OLD text inserts.
- Persistence round-trip: kills including multiline, `é`, and an
  invalid-UTF-8 byte entry → the state file exists IMMEDIATELY
  (write-on-mutation, no quit involved), `v1` + hex lines, file
  mode 0600 and parent dir 0700 (PermissionsExt, the `state.rs:459`
  shape); a fresh `EditorState` + same StateDir seeds on first
  `list()` byte-identically, `C-y` yanks the restored head.
- Ordering pin: ring touched BEFORE the StateDir appears →
  StateDir installed → next op seeds, persisted entries land
  BEHIND the in-session ones, MRU semantics preserved.
- io-blocked: a directory squatting on the `killring` key → seed
  reports once (status + stubbed `pmacs.error`), ring operations
  proceed normally, NO write ever lands (the unreadable file is
  not clobbered); removing the obstruction → a later op seeds and
  writes resume.
- Fail-closed parse: bad header / odd-length / non-hex → empty
  seed, no error, and the next mutation overwrites with a valid
  file.
- Caps: 35 kills → 30 persisted; an oversized entry skipped while
  neighbors persist.
- `persist(false)` → no writes, no seeding, existing file
  untouched; getter reflects state; unconfigured state (plain
  `EditorState::new()`) → everything inert, no errors.
- Latch regression guard (lib-side, next to the Rust change): the
  latch clears at dispatch entry; a tick-path manual run (the
  autosave shape) does not suppress the NEXT dispatch's automatic
  fire.

No CRDT-specific suite: the browser insert is a plain daemon-peer
edit on the dispatch path — the editops/comment-toggle posture.
