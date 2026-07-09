# Autosave + crash recovery — framing (Arc 3 phase 3)

Kill pmacs mid-edit and the unsaved work is gone. **Autosave** writes a
recovery copy of each modified file buffer on a configurable interval;
**crash recovery** notices that copy on next open and lets you restore
it. Emacs's `auto-save-mode` + `recover-file`.

Closes the persistence arc: phase 1 (PR #98) gave the `pmacs.state`
confined store and `state.remove`; phase 2 (PR #99) gave the all-Rust
`pmacs.session.*` precedent and `get_or_load_buffer`. Parent decision:
`docs/persistence-framing.md` Q#PS8.

## Ground truth (scouted; file:line as of `a0a4e7f`)

- **Timers advance while idle.** Both run loops block on a *frame
  timeout*, not on input, and fall through to `tick_async` regardless
  (`src/editor.rs:1619,1643-1646`; `src/daemon.rs:1273,1304,1312-1314`).
  So a periodic Lua loop keeps running when nobody is typing.
- **But `workers.sleep` parks a pool thread** for its full duration
  (`src/async_runtime.rs:724-733,1277-1290`), and the pool is only
  `available_parallelism - 1` (`:566-602`). A 30-second sleep would hold
  a worker hostage. `process.after-tick` (`builtin/hooks/default.lua:70`,
  fired every frame from `src/editor.rs:411-420`) plus
  `pmacs.editor.monotonic_ms()` (`src/lua_bindings/mod.rs:10804`) is the
  zero-thread alternative — already the LSP-debounce idiom
  (`builtin/runtime/lsp.lua:299`).
- **Lua cannot see a non-active buffer's path.** `BufferIdLua` exposes
  `:len() :name() :is_modified() :is_valid() :slice()` and the mutators
  (`src/lua_bindings/mod.rs:1156-1235`) — **no `:file_path()`**. The only
  path getter is `pmacs.editor.file_path()` (active buffer,
  `:10902`). Contents *are* readable for any buffer (`:slice` resolves by
  id). Rust has everything: `registry.ids()`, `Buffer::file_path()`
  (`src/buffer.rs:263`), `is_modified()` (`:452`), and the whole-buffer
  byte snapshot `save()` already uses (`src/editor_core.rs:1246-1250`).
- **`FileMeta`** (`src/file_io.rs:47-62`, `{mtime: SystemTime, size:
  u64}`, `PartialEq`) is **not serde and not exposed to Lua**.
  `current_meta(path)` exists (`:66`); each buffer stores its load/save
  meta (`src/buffer.rs:194,274-280`). **Nothing compares them today** —
  `EditorCore::save()` overwrites unconditionally (`:1231-1268`), so the
  external-change guard is new code.
- **No `buffer.before-load` seam** — contents are installed, *then*
  `after-load` fires. Substitution must happen after the fact via
  `buf:replace(0, buf:len(), bytes)` (`:1216-1234`).
- **`after-load` fires once per restored leaf during desktop-restore**
  (phase 2, `src/desktop.rs`). A modal prompt there would stack N
  prompts. There is also **no `y_or_n`/confirm helper** — only the
  callback-driven `pmacs.minibuffer.read` (`:11040`).
- **Cleanup seams exist**: `buffer.after-save` hook
  (`builtin/hooks/default.lua:57`, active buffer), per-buffer
  `pmacs.buffer.on_removed(id, fn)` (`:2695-2718` — there is **no**
  global kill hook), and `editor.before-quit` already has two listeners
  (multiple listeners are fine).
- **`pmacs.state.remove(name)`** confirmed (`:2073`). But
  **`state::read` returns `String`** (`read_to_string`, `src/state.rs`)
  — recovery contents are arbitrary bytes, so a `read_bytes` is needed.
- **State files are not private.** `state::write` does a plain
  `create_dir_all` (`src/state.rs:186`, default `0755`), and
  `save_atomic` only preserves the mode of an **existing** target
  (`src/file_io.rs:143-145`) — a *new* file gets the umask default,
  typically `0644`. Autosave stores **unsaved file contents**, not
  metadata; world-readable recovery copies could be more exposed than
  the original file.
- **"New file" buffers have no origin meta and fire no hook.** Opening a
  missing path sets `file_path` but leaves `file_meta` unset and
  `fire_after_load = false` (`src/editor.rs:512-519`). And Lua's
  `from_file`/`find_or_open` **error** on a missing path
  (`src/lua_bindings/mod.rs:2488,2551`) — so a `[new file]` buffer only
  ever arrives via argv `EditorState::open`, which fires *nothing*.
- **`buf:replace` does not fire `buffer.after-edit`.** The mutators call
  `notify_buffer_edit_to_windows` (windows + CRDT queue only,
  `src/lua_bindings/mod.rs:1374-1385`). `after-edit` is fired solely by
  `dispatch_key`'s post-command revision check (`src/editor.rs:739`) and
  the modal shadows that return before it (`:887,:922`). The minibuffer
  shadow is one of those — so an edit made inside an `M-x` command body
  is invisible to LSP/syntax. `pmacs.hook.run(name)` *is* public
  (`src/lua_bindings/mod.rs:4962`), the escape hatch
  `builtin/commands/default.lua:226` already uses.
- **`sha256_hex` is duplicated privately twice** (`src/desktop.rs:144`,
  `src/packages/fetcher.rs:517`). A third copy would be wrong.
- **There is no configuration system.** No `pmacs.config`, no options
  table, no defcustom registry; `src/config.rs` only loads `init.lua`.
  The convention is an ad-hoc *validated setter*:
  `pmacs.async_config.frame_target_ms(ms)`
  (`builtin/runtime/async.lua:458-466`) and `fs.watch{interval_ms}`
  (`builtin/runtime/fs.lua:237-241`) both do
  getter-when-nil / type-check / `>= 1` / `math.floor`.

## Decisions

### Q#AS1 — Hybrid: Rust owns the sweep + the guard; Lua owns cadence, config, UX

The same split phase 2 landed on, forced by the same two gaps: Lua can't
read a non-active buffer's path, and `FileMeta` is neither Lua-visible
nor serde. So:

- **Rust (`src/autosave.rs`)**: `sweep()` (walk the registry, write a
  recovery file per modified file buffer), `status(path)` (the
  external-change guard), `recover_bytes(path)`, `discard(path)`.
- **Lua (`builtin/runtime/autosave.lua`)**: the timer, the interval and
  enable knobs, the after-load notification, the `recover-file` /
  `discard-recovery` commands, and the save/kill/quit cleanup wiring.

Notably this needs **no new Lua per-buffer path getter** — the sweep
never leaves Rust, and every Lua cleanup seam (`after-save`,
`on_removed`, `after-load`) is either active-buffer or captures the path
at registration.

### Q#AS2 — Cadence: `process.after-tick` + `monotonic_ms`, not `workers.sleep`

Three reasons, in order of weight:
1. **No worker thread is parked.** A long `workers.sleep` holds one of
   `available_parallelism - 1` pool threads for the whole interval.
2. **The interval becomes live-reconfigurable for free** — the handler
   re-reads it each tick, so `interval_ms(60000)` takes effect
   immediately. A sleeping timer would ignore the change until it woke.
3. It matches the existing debounce idiom (`lsp.lua`).

The handler is: bail if disabled; `monotonic_ms()`; if
`now - last >= interval` then `last = now` and sweep. Every frame this
costs one clock read and a compare. Wrapped in `pcall` —
`process.after-tick` is `all-must-succeed`, and a sweep error must not
poison the chain.

The sweep itself is **synchronous on the main thread** (a `save_atomic`
per dirty buffer). Bounded by Q#AS8's skip rules; offloading large-buffer
writes to a worker is deferred.

### Q#AS3 — Configuration (the interval)

pmacs has **no config registry**, so this follows the established
validated-setter convention rather than inventing one:

```lua
pmacs.autosave.interval_ms()        -- getter → current value
pmacs.autosave.interval_ms(60000)   -- setter, validated
pmacs.autosave.enable(false)        -- disable knob
pmacs.autosave.sweep()              -- force a sweep now (manual/test)
```

`interval_ms(ms)`: returns the current value when `ms` is nil; otherwise
requires a `number`, rejects `< MIN_INTERVAL_MS` (**1000**, since each
sweep `fsync`s), applies `math.floor`, and errors on anything else —
byte-for-byte the shape of `frame_target_ms`. **Default 30_000 ms**
(Emacs's `auto-save-timeout`). Changes apply on the next tick (Q#AS2).

Tests drive `sweep()` directly rather than waiting on a timer, so the
1-second floor never makes the suite slow.

> **Bigger picture, flagged not built:** the absence of any config
> registry is itself a gap. Real configurability — typed, validated,
> introspectable, defaulted, `M-x customize`-able options — is an arc of
> its own. `interval_ms`/`enable` are deliberately shaped as
> get-or-set-with-validation so they can be *migrated into* such a
> registry later without changing call sites. Not in this PR.

### Q#AS4 — Recovery file: one atomic file, header line + raw bytes

Key: `autosave/<sha256hex(absolute path)>` (lowercase hex passes
`state::validate_name`'s `[A-Za-z0-9._-]` charset).

**One file, not a contents/sidecar pair.** A pair is two writes: a crash
between them leaves contents without meta (or vice versa). Instead a
single atomic write of:

```
<one line of JSON>\n<raw buffer bytes>
```

The header is `{version, path, origin: null | {mtime_secs, mtime_nanos,
size}}` — `FileMeta` hand-serialized, since it is not serde
(`SystemTime` → `duration_since(UNIX_EPOCH)`).

**`origin` is nullable** (finding). A `[new file]` buffer — a path that
does not exist on disk yet — has no `file_meta`, and its unsaved
contents are exactly the work most worth recovering. Requiring an origin
meta would have silently excluded it. `origin: null` means "there was no
file on disk when this was autosaved."

Contents may contain newlines and non-UTF-8 bytes; the reader splits at
the **first** `\n` only. This needs a Rust-only **`state::read_bytes`**
(today's `state::read` is `read_to_string`, which would fail on non-UTF-8
buffer contents).

### Q#AS5 — Recovery status: the external-change guard

`status(path)` reads the envelope and compares `header.origin` against
`current_meta(path)`:

| `header.origin` | on disk now | status | meaning |
|---|---|---|---|
| `Some(m)` | exists, meta `== m` | **`Fresh`** | disk untouched; recovery is newer |
| `Some(m)` | exists, meta `!= m` | **`Stale`** | file changed externally |
| `Some(m)` | missing | **`Stale`** | the base file was deleted |
| `None` (new file) | missing | **`Fresh`** | still a new file; nothing to conflict with |
| `None` (new file) | exists | **`Stale`** | someone created the file meanwhile |
| unparseable / bad version | — | **`Corrupt`** | never offered; discardable |
| no file | — | **`None`** | |

Only **`Fresh`** is announced (Q#AS6). **`Stale`** is never auto-offered
— silently clobbering a file someone else changed is the one
unrecoverable mistake here; `recover-file` will still recover it, but
says so plainly and requires confirmation. **`Corrupt`** is a typed
status, not an error (finding): a malformed envelope must not make
startup noisy or break the commands. It is counted separately, never
offered, and `discard-recovery` removes it.

### Q#AS6 — Notify (aggregated, on the tick), don't prompt

Recovery **must not** open a modal minibuffer prompt from `after-load`:
desktop-restore fires `after-load` once per restored leaf (phase 2), so a
prompt would stack N modals mid-restore — and no `y_or_n` helper exists
to build one cleanly anyway.

But a per-`after-load` **status message** is also wrong (finding): N
restored leaves would each overwrite `core.status`, so only the last
recoverable file is ever mentioned. And it would miss `[new file]`
buffers entirely, which fire no hook at all (ground truth).

So the report is **pull-based and aggregated on the tick we already own**
(Q#AS2):

- Rust `pmacs.autosave.pending()` → for **every open file buffer**,
  the `status(path)`; returns the `Fresh` paths (and a `Corrupt` count).
  Enumerating buffers in Rust is what makes this cover argv `[new file]`
  buffers and desktop-restored buffers uniformly, with no hook at all.
- Lua sets a `needs_report` flag on module load (the startup scan) and on
  `buffer.after-load` (runtime opens). The **tick handler** — not the
  hook — does the reporting: if flagged, call `pending()` once, emit a
  single aggregate status, clear the flag. N synchronous `after-load`
  fires during a restore therefore collapse into **one** message:
  *"3 files have autosave recovery — M-x recover-file"* (or the filename
  when there is exactly one).
- `pending()` runs one `stat` per open file buffer, only when flagged —
  never per frame.

Recovery itself happens through an explicit command:

- **`recover-file`** — confirms via `minibuffer.read` (typed `yes`),
  **pins to the origin *buffer handle*, not merely its path** (finding:
  `pmacs.buffer.from_file` does not dedup, so two buffers can visit one
  path and a path check alone could recover into the wrong one), then
  `buf:replace(0, buf:len(), recovery_bytes)`,
  **then explicitly `pmacs.hook.run("buffer.after-edit")`**. That last
  step is load-bearing (finding): the mutators only notify windows and
  queue CRDT, and `after-edit` is fired by `dispatch_key`'s post-command
  check — which the minibuffer shadow returns before. Without the
  explicit fire, LSP `didChange` and the syntax reparse would never see
  the recovered contents. (Both read the *active* buffer, which is
  exactly the one `recover-file` operates on.)
  The replace leaves the buffer **modified** — the user must save to
  accept, which is what deletes the recovery file (Q#AS7).
- **`discard-recovery`** — delete the recovery file for the active file,
  whatever its status (including `Corrupt`).

This also sidesteps re-entrancy: no modal surface is opened from inside a
hook fired by Rust.

### Q#AS12 — Never overwrite unclaimed crash data (the ownership rule)

The failure this closes (finding): you crash with unsaved work, reopen
the file, and start editing *before* running `recover-file`. The next
sweep writes the current buffer to the same key — **destroying the crash
copy**, which is precisely what autosave exists to protect.

So autosave tracks **ownership**. A per-session `owned` set records which
path hashes *this session* wrote or adopted. A recovery file at a key we
do not own is unclaimed crash data, and the rule is total:

> **Exactly two things may release an unclaimed recovery file:**
> `recover-file` (which *adopts* it) and `discard-recovery` (explicit
> user intent). Nothing else — not a sweep, not a save, not a kill.

Concretely:

- the **sweep refuses to write** that buffer, counts it `blocked`, and
  surfaces *"autosave paused for N file(s) with unclaimed recovery — M-x
  recover-file or M-x discard-recovery"*;
- **`save` and `kill` delete only keys this session owns** (finding). You
  reopen a crashed file, edit, and save without recovering: the on-disk
  file now holds your new work, but the crash copy still holds work that
  was *never written anywhere*. Deleting it would be the same data loss by
  a different door. It survives — as `Stale`, so it is never auto-offered,
  but it is still there to recover or discard.
- `recover-file` **adopts by buffer**, not by path (finding). Adopt
  records a `written` entry for that `BufferId` at the revision whose
  contents the file now holds. That makes the skip cache correct *and*
  lets a later kill retire the copy — a removal callback fires after the
  buffer has left the registry, when there is no path left to read.
- `discard-recovery` clears the matching `written` entries too (finding),
  so a still-dirty buffer is re-protected on the very next sweep instead
  of hitting the unchanged-`(path_hash, revision)` fast path and going
  unprotected until its next edit.

The trade is deliberate: while blocked, edits made *after* the reopen are
not autosaved — and the user is told so, every sweep. Losing the new
edits to a second crash is recoverable by retyping; losing the original
crash copy is not.

### Q#AS7 — Cleanup lifecycle (keyed by buffer, not by a captured path)

- **`buffer.after-save`** → `discard_buffer(active buffer)`.
- **Buffer killed** → `discard_buffer(id)`. There is no global kill hook,
  so `after-load` registers a per-buffer `pmacs.buffer.on_removed`.
- Both go through **`discard_buffer(BufferId)`**, not a path captured at
  load time (finding). It removes *both* the buffer's current-path key
  and the key its last sweep actually **wrote** under — which differ
  after a rename (an LSP `WorkspaceEdit` changes the path while the
  `BufferId` stays). A path-captured callback would delete the wrong key
  and leave the real recovery file behind.
- **Sweep-time GC** is the backstop: any cache entry whose `BufferId` has
  left the registry has its recovery file deleted. This is what covers
  argv **`[new file]`** buffers, which fire no `after-load` and so never
  get a removal callback registered (finding).
- **`editor.before-quit`** → one **final synchronous sweep**, then return
  nil (never veto). Async ticks stop after quit, so this must be a direct
  call. Result: quitting with unsaved changes leaves a recovery copy that
  the next open notices — which is exactly the point.
- Recovery files for buffers never reopened linger. Orphan GC is
  deferred.

### Q#AS8 — What gets swept, and the cost bound

Only buffers with `file_path().is_some() && is_modified()` — which
**includes `[new file]` buffers** (path set, no origin meta, Q#AS4).
Scratch and `*special*` buffers are skipped (deferred). Two skips keep
the main-thread cost down:

1. If no buffer qualifies, the sweep returns immediately (no IO).
2. Skip a buffer whose contents are unchanged since its last successful
   autosave. Without this, a 30-second interval re-`fsync`s an
   idle-but-dirty buffer forever.

**The skip cache is keyed `BufferId → (path_hash, revision)`, not
`BufferId → revision`** (finding). A buffer keeps its `BufferId` across a
path change (LSP `WorkspaceEdit` rename calls `set_buffer_path`), so a
revision-only cache would skip the write, never create the recovery file
under the *new* key, and orphan the old one. On sweep, a `path_hash`
mismatch counts as changed: write the new key **and** `discard` the old
one.

### Q#AS9 — Extract the duplicated `sha256_hex`

Two private copies exist (`desktop.rs`, `packages/fetcher.rs`); autosave
needs a third. Instead extract one `pub(crate) fn sha256_hex` into a
small `src/hash.rs` and point all three at it. In-scope cleanup, not a
drive-by: the alternative is knowingly adding the third copy.

### Q#AS11 — Private storage (a **precondition** for default-on)

Autosave stores **unsaved file contents** — a different class of secret
from saveplace's cursor offsets or recentf's path list. Today
`state::write` creates parents with a default `0755` and `save_atomic`
gives a *new* file the umask default (typically `0644`), preserving mode
only for an already-existing target. A recovery copy of an unsaved edit
to a `0600` file would land world-readable — **more exposed than the
original** (finding).

So this PR makes autosave storage private:

- **`file_io::save_atomic_with_mode(path, content, mode)`** — sets the
  temp file's permissions **before** the rename, so the target is never
  momentarily visible at `0644`. (A chmod-after-write leaves exactly that
  window.) Plain `save_atomic` delegates with `None`.
- **`state::write_private(base, name, content)`** — creates the parent
  with `DirBuilder::mode(0o700)` and writes the file `0600`. It also
  **tightens a pre-existing lax `autosave/`** to `0700` (finding): the
  birth-mode only applies to directories *that call* creates, so a
  `0755` directory left by an older run would still leak recovery-file
  names, sizes, and mtimes despite `0600` contents. It never re-modes
  `base` itself — the state root is shared with history/recentf/desktop
  and may predate us.
- Recovery files use it; the `autosave/` directory is `0700`.
- Unix-only (`PermissionsExt` / `DirBuilderExt` are safe under
  `#![forbid(unsafe_code)]`); on other platforms it degrades to today's
  behavior, documented.

Retention and the disable knob are documented in the same breath: files
live under `$XDG_STATE_HOME/pmacs/autosave/`, are deleted on save/kill,
survive a crash or an unsaved quit, and `pmacs.autosave.enable(false)`
stops all of it. (Hardening the whole state-dir root to `0700` is an
obvious neighbour — noted as deferred, since it would re-mode a directory
users already have.)

### Q#AS10 — Default on, conditional on Q#AS11

**On by default**, with the interval configurable, a disable knob, and
**only because Q#AS11 lands in the same PR**. If private storage slips,
this drops to opt-in.

The parent framing (Q#PS9) tentatively said opt-in, grouping autosave
with desktop-save because "background writes are surprising." That
grouping conflated two things: desktop-save is opt-in because
*auto-restore* changes what you see at startup. Autosave changes nothing
observable until the day it saves your work; it writes only into the
state dir (never your files), it is inert when nothing is modified, and
it is the highest-value safety net in the arc. Emacs ships it on;
saveplace and recentf are already default-on and also write.

The reviewer's condition is the right bar and is now the plan of record:
default-on **requires** `0700`/`0600` storage plus documented retention
and disable. Both are in scope.

## Phasing

One PR (the pieces are useless apart). In-diff order: `src/hash.rs` +
`state::read_bytes` + `save_atomic_with_mode`/`state::write_private`
(Q#AS11) → `src/autosave.rs` (envelope, sweep, status, recover, discard,
pending) → `pmacs.autosave.*` bindings → `autosave.lua` (timer, config,
commands, cleanup wiring) → tests.

## Bets (score at close)

1. **`after-tick` + `monotonic_ms` is the right substrate** — no worker
   thread parked, interval live-reconfigurable, no measurable per-frame
   cost.
2. **The one-file envelope is crash-atomic** — a mode-aware
   `save_atomic` means a recovery file is never a torn header/contents
   pair nor briefly world-readable, and the first-newline split survives
   arbitrary binary contents.
3. **The nullable-origin guard covers new files** — `[new file]` buffers
   round-trip, and the `Fresh`/`Stale` table never offers to clobber an
   externally-changed (or externally-created) file.
4. **Pull-based aggregated notify is sufficient UX** — one message
   however many files are recoverable, it covers argv `[new file]`
   buffers that fire no hook, and desktop-restore stays clean.

## Deferred (named)

- **Idle-gated autosave** (Emacs's `auto-save-timeout` idle semantics);
  v1 is plain wall-clock elapsed.
- Autosaving non-file (scratch) buffers.
- Orphan recovery-file GC / a `list-recovery-files` browser.
- Offloading large-buffer writes to a worker thread.
- **An external-change guard on `save()` itself** — the scout found
  pmacs overwrites unconditionally today. Real bug, adjacent, its own PR.
- A general `y_or_n` minibuffer helper (build it when a second caller
  appears).
- A central, typed config registry (Q#AS3's note).
- Hardening the whole `$XDG_STATE_HOME/pmacs/` root to `0700` (Q#AS11) —
  it would re-mode a directory users already have.
- Firing `buffer.after-load` for `[new file]` buffers. Today argv-opening
  a missing path fires no hook at all, so a new file gets no syntax, no
  LSP, and no saveplace. That is a real latent gap, but changing it
  ripples through four builtins and does not belong in an autosave PR.
- Hidden-buffer LSP initial attach (carried from phase 2).

## Acceptance (tempdir state root injected; `sweep()` called directly)

- Modify a file buffer → `sweep()` → recovery file exists; its header
  path + origin meta match, its bytes equal the buffer.
- **Non-UTF-8 contents** round-trip through the envelope (the
  `read_bytes` reason).
- **`[new file]` buffer** (path that does not exist): swept, header
  `origin: null`, status `Fresh` while the file is still absent; and
  `Stale` once the file exists on disk. Recoverable either way.
- **Permissions (Q#AS11)**: on Unix, the `autosave/` dir is `0700` and
  each recovery file is `0600` — asserted, not assumed.
- Sweep skips clean buffers, scratch buffers, and buffers unchanged
  since the last sweep (no second write).
- **Unclaimed crash data is never overwritten (Q#AS12)**: session 1
  crashes with a recovery copy; session 2 reopens, edits, sweeps →
  `(written, blocked) == (0, 1)` and the crash copy is byte-identical.
  `_adopt` (what `recover-file` calls) or `_discard` resumes the sweep.
- **…nor deleted by a save or a kill**: session 2 reopens, edits, and
  saves (or kills) without recovering → the crash copy survives
  byte-identical, now reported `Stale`.
- **Recover then kill immediately** (before any save or sweep) → the
  adopted copy *is* retired, not left to be re-offered.
- **Explicit `discard-recovery` on a still-dirty buffer** → the next
  sweep re-protects it at once, with no intervening edit.
- **Path change**: rename a buffer's path (`set_buffer_path`) without
  editing it → next sweep writes the new key **and** removes the old
  recovery file (the `(path_hash, revision)` cache).
- `after-save` → recovery deleted. Kill buffer → recovery deleted.
- **Rename then save with no intervening sweep** → the recovery written
  under the *old* key is removed (buffer-keyed cleanup, Q#AS7).
- **Killing a `[new file]` buffer** → the sweep-time GC removes its
  recovery (no `after-load` fired, so no removal callback exists).
- **A pre-existing `0755` `autosave/` dir is tightened to `0700`.**
- Open a file with a **`Fresh`** recovery → the aggregate report names
  it; buffer contents are still the on-disk ones (no silent
  substitution).
- **Aggregation**: three recoverable files opened → **one** status
  message reporting `3`, not three messages.
- Touch the file on disk, then open → **`Stale`**: distinct message, not
  offered.
- **`Corrupt`**: a malformed envelope (no newline / bad JSON / bad
  version) yields `Corrupt`, is never offered, does not error the report
  or the commands, and `discard-recovery` removes it.
- `recover-file` → buffer contents become the recovery bytes, the buffer
  is `is_modified()`, and **a probe on `buffer.after-edit` observes the
  recovery** (the explicit `hook.run`); a subsequent save deletes the
  recovery file.
- `interval_ms()` getter/setter: rejects non-numbers and `< 1000`,
  floors floats, and a changed interval takes effect without a restart.
- `enable(false)` → `sweep()` is a no-op.
- `before-quit` sweeps once, synchronously, and does not veto quit.
