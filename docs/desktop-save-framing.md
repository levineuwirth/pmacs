# Desktop-save — framing (Arc 3 phase 2)

Reopen pmacs and your session is gone: which files were open, how the
window was split, where each cursor sat. **desktop-save** serializes the
open file buffers + the window layout + per-window positions on quit and
rebuilds them on startup — Emacs's `desktop.el`, opt-in.

Builds on phase 1 (PR #98, merged): the `pmacs.state` confined store,
`state_dir()`, `goto_byte`/`set_view_top`/`view_top`, and saveplace
(which already restores a file's cursor on open — desktop leans on it).
Parent decisions: `docs/persistence-framing.md` Q#PS5-7. This doc nails
the phase-2 implementation against ground truth.

## Ground truth (scouted; file:line in the commit)

- **Layout tree**: `LayoutNode = Leaf(WindowId) | Split { orientation:
  Orientation, weights: Vec<u32>, children: Vec<LayoutNode> }`
  (`src/window.rs:264`). **Not serde.** Owned per-frontend at
  `core.views[fid].layout.root` (`FrontendView`, `src/window.rs:301`);
  `core.active_layout()/_mut()` reach the active one
  (`src/editor_core.rs:355`). `core.windows` is a **`pub BTreeMap<
  WindowId, Window>`** (`src/editor_core.rs:135`).
- **`Window`** (`src/window.rs:158`) stores its own `buffer_id`,
  `cursor` (byte), `view_top` (line). So a window→buffer→path chain is
  fully readable in Rust.
- **`WindowId`** is a process-lifetime `AtomicU64` counter
  (`src/window.rs:55`) — **not restart-stable**; rebuild structurally,
  never persist raw ids.
- **No tree read/rebuild API** (Lua or Rust): `iter_ids()` is a *flat*
  preorder id list (`src/window.rs:334`); `split_window` hardcodes 1:1
  weights (`src/window.rs:456`). Arbitrary shape/weights must be built
  by constructing `LayoutNode` + `Window`s directly against the `pub`
  fields (the window unit tests already mutate `layout.root` this way).
- **No per-`BufferId` path getter in Lua** — but irrelevant here:
  save/restore live in Rust and read paths straight off the registry
  (`registry.ids()` → `registry.get(id).file_path()`,
  `src/buffer.rs:263`; `is_modified()` `:452`; `find_by_path` `:168`).
- **`serde_json`** + **`serde` derive** + **`sha2` (SHA-256)** are all
  existing deps (`Cargo.toml`). SHA-256 is the established key hasher
  (`sha256_hex`, `src/packages/fetcher.rs:517`).
- **`instance.identity()`** returns `instance_name: Option<String>` and
  `working_directory: String` (`InstanceIdentity`, serde-derived,
  `pmacs-protocol/src/message.rs:1394`).
- **Startup**: `editor::run(file: Option<PathBuf>)` (`src/editor.rs:1520`)
  — the `match file` at `:1522` **consumes** `file`; capture
  `had_file` *before* it. `install_state_dirs()` is at `:1527` (the
  natural post-construction trigger point). `run_daemon` takes **no
  file arg** (`src/daemon.rs:448`), constructs at `:468`, wires state at
  `:470`.
- **No `editor.after-init` hook** — restore must be **Rust-triggered**.
  `editor.before-quit` is a short-circuit hook fired by the quit command
  (`builtin/commands/default.lua:239`) — the save seam.
- **Rust can fire a Lua hook** (precedent: daemon remote-op fires
  `buffer.after-edit`) — so restore can fire `buffer.after-load` per
  opened file to attach saveplace/LSP/syntax.

## Decisions

### Q#DS1 — Rust-owned `pmacs.session.*` + a thin Lua `desktop.lua`

Everything load-bearing (registry walk, layout serde, structural
rebuild, per-window state) is Rust, because the tree types aren't serde
and there is no Lua tree API. Surface:

- `pmacs.session.save_desktop()` — Rust; serialize the active
  frontend's layout + file buffers + positions to the state store.
- `pmacs.session.restore_desktop()` — Rust; rebuild from the store.
- `pmacs.session.arm_restore()` — Rust; set the "restore on startup"
  flag (read by the Rust startup trigger).
- `builtin/runtime/desktop.lua` — `pmacs.session.desktop_mode(on)`:
  when `on`, registers an `editor.before-quit` hook that calls
  `save_desktop()` **and** calls `arm_restore()`. Plus `desktop-save` /
  `desktop-restore` commands for manual use. **Opt-in**: nothing runs
  unless init.lua calls `desktop_mode(true)`.

No per-buffer Lua path getter is added — the framing's phase-2 "per
`BufferId` `file_path()`" primitive turns out unnecessary because
enumeration is Rust-side.

### Q#DS2 — The serialized format

A serde-derived mirror (its own types, leaving the core enums
untouched), `serde_json` to `state_dir()/desktop/<key>`:

```
SavedDesktop { version: u32, session_key: String,
               buffers: Vec<SavedBuffer>,   // ALL open file buffers
               root: SavedNode,             // the window layout
               active_leaf: usize }
SavedBuffer  { path: String, modified: bool }
SavedNode    = Leaf(SavedLeaf) | Split { orientation, weights: Vec<u32>,
                                         children: Vec<SavedNode> }
SavedLeaf    { path: String, cursor: u64, view_top: usize }
```

**`buffers` is every file buffer in the registry** (`registry.ids()` →
`file_path().is_some()`), not just those visible in a window — so a
file opened then switched away from (live but hidden) survives restore.
The scope really is "open file buffers + layout" (finding: the earlier
draft saved only layout leaves and silently dropped hidden buffers).
Restore opens the whole `buffers` set, then rebuilds the layout on top.

`root` preserves exact **orientation + weights + nesting**. `active_leaf`
is the **preorder index** into the *surviving* leaf sequence (Q#PS5 — a
path can't identify which leaf had focus when the same file shows in
several).

**Only file buffers.** A leaf whose window shows a scratch/`*special*`
buffer is dropped and its parent split collapses (remaining siblings'
weights kept, renormalized by the layout math). If that dropping removes
the leaf `active_leaf` pointed at, `active_leaf` **falls back to the
nearest surviving preorder neighbor** (Q#DS10). If no file leaf
survives, no desktop is written.

`modified` rides on `SavedBuffer` so the restore-time warning (Q#DS6)
has a source; contents are never saved.

### Q#DS3 — Restore: structural rebuild in Rust

The ordering constraint that drives this: **`buffer.after-load` hooks
read *active* state** — saveplace/recentf via `pmacs.editor.file_path()`,
syntax via `pmacs.window.buffer()`, LSP's `attach_buffer` derives
language/path/text from the active buffer. So a restored buffer must be
*active* when its `after-load` fires, or the hooks attach to the wrong
buffer (finding). `get_or_load_buffer` (Q#DS4) deliberately does not
switch focus, so restore sequences activation explicitly.

`restore_desktop()`:
1. Read + parse `desktop/<key>`; if absent or `session_key` mismatches,
   no-op.
2. **Open every `SavedBuffer`** via `get_or_load_buffer(path)` (Q#DS4),
   recording which ids are newly loaded. A path that no longer exists on
   disk is skipped with a warning (its leaves collapse per Q#DS10).
3. **Prune the entire old LOCAL layout**: remove *all* windows belonging
   to `core.views[LOCAL]` from `core.windows` (not just the startup
   scratch window — leftover windows would linger in the `pub` map and
   still take part in edit notifications and buffer-liveness checks,
   finding). 
4. Build a fresh `LayoutNode` from `SavedNode` with new `WindowId`s and
   a `Window` per surviving leaf (weights copied verbatim), install it
   as `core.views[LOCAL].layout.root`.
5. **Fire `after-load` with the right leaf active**: for each surviving
   leaf in preorder, set its window active; the first time a given
   buffer is seen, fire `buffer.after-load` (once per newly-loaded
   buffer, so saveplace/LSP/syntax attach against the correct active
   buffer); then set that window's exact `cursor`/`view_top` from the
   leaf. Desktop's per-leaf write lands *after* the hook, so it wins
   over saveplace for precision — and same-file-two-leaves keeps
   distinct positions a single saveplace entry could not.
6. Set `active` to the `active_leaf` window (Q#DS10 fallback if that
   leaf didn't survive).

Structural construction against the `pub` fields — no new tree-builder
API, matching how the window unit tests already assemble layouts.

### Q#DS4 — `get_or_load_buffer(path)` core helper

The one genuinely new Rust seam. Reuses `EditorState::open`'s internals:
`registry.find_by_path(path)` → return the existing id; else
`file_io::load_file` → create buffer → `set_buffer_path`/`set_buffer_meta`
→ return the new id. It does **not** switch the active window (restore
places buffers into windows it builds explicitly). Returns `io::Result`
so a since-deleted file is skipped (its leaf collapses) with a warning,
not a hard failure.

### Q#DS5 — Session key

`instance.identity()` → key, then **SHA-256 hex** (the established key
hasher), tag-prefixed for legibility and to satisfy the Q#PS2 state-key
charset (`:` is disallowed, so a dot separator):
`name.<sha256hex(instance_name)>` when a socket name is set, else
`cwd.<sha256hex(working_directory)>`. Stored as state key
`desktop/name.<hex>` (both components pass `validate_name`). Hashing
both uniformly sidesteps odd characters in either value.

### Q#DS6 — No contents; modified = warning-only

Saves the *file list + layout + positions*, never buffer contents
(Emacs `desktop.el`). Each `SavedBuffer.modified` records whether that
buffer was dirty at save time; restore opens the on-disk file (clean)
and, if any `modified` flags are set, surfaces a one-line count ("N
buffers had unsaved changes when the desktop was saved") via
`core.status`. Unsaved work is autosave's job (phase 3).

### Q#DS7 — Startup gate (the Q#PS7 trap, made concrete)

Restore is **armed, never inline in init** — `desktop_mode(true)` runs
inside `new()` (before the file opens), so it only sets the flag +
before-quit hook. The Rust startup trigger fires restore:

- `editor::run`: capture `let had_file = file.is_some();` **before** the
  `match file` at `src/editor.rs:1522` consumes `file`. But fire restore
  **inside the `RunLocal` arm** of the attach dispatch (after
  `take_requested_attach` + `dispatch_attach`), *not* right after
  `install_state_dirs()` — at the earlier point `run()` hasn't yet
  resolved an init-time `pmacs.attach{}` request, so a restore could
  populate an `EditorState` that is about to be dropped for attach
  hand-off (finding). In the `RunLocal` arm, call
  `state.restore_desktop_if_armed(had_file)` — restores only when armed
  **and** `!had_file`.
- Manual `desktop-restore` command ignores the gate (explicit user
  intent).

### Q#DS8 — before-quit save semantics

The `editor.before-quit` hook is short-circuit; the desktop save handler
performs its write and returns `nil` (never vetoes quit). It serializes
the layout as it stands at quit. A save failure is logged, not fatal —
quitting must not be blockable by a state-write error.

### Q#DS9 — Scope v1 to local (in-process) mode — save *and* restore

The daemon holds a layout **per attached frontend** (`views` keyed by
`FrontendId`), and the Q#DS5 key has no frontend component; at
`run_daemon` construction no frontend is attached, so there is nothing
to restore *into* until first attach. v1 targets **only** the local
`editor::run` path (single `LOCAL` frontend view built at startup).

**`desktop_mode(true)` auto-save and auto-restore are both no-ops in
daemon mode** — not half-enabled. Serializing "wherever a layout exists"
is ambiguous with multiple frontend layouts sharing one key, so the
before-quit save simply doesn't register (or early-returns) when the
process is a daemon; a diagnostic notes desktop-save is local-only in
v1. Daemon + GPU-attach save/restore is **deferred** to the first-attach
design. (Manual `desktop-save`/`desktop-restore` commands likewise
refuse in daemon mode in v1.)

### Q#DS10 — Active-focus fallback

Two prunings can orphan the focus target: a scratch/`*special*` leaf
dropped at **save** time, or a missing file's leaf collapsed at
**restore** time. In both cases, resolve `active_leaf` to the **nearest
surviving preorder neighbor** (the next later leaf, else the previous),
and assert the result indexes a real surviving leaf. A desktop with zero
surviving file leaves is never written (save) / is a no-op (restore), so
`active_leaf` always resolves to something.

## Phasing

One PR — save and restore are only useful paired. In-diff order: mirror
types + `save_desktop` + `get_or_load_buffer` first, then
`restore_desktop` + the startup trigger + `desktop.lua`.

## Bets (score at close)

1. **Structural rebuild is faithful** (the parent bet #2) — a
   nested/asymmetric weighted tree round-trips exactly. *Highest risk.*
2. **Activate-then-fire attaches everything** — firing
   `buffer.after-load` with the restored leaf *active* makes saveplace,
   LSP, and syntax behave on a restored buffer exactly as on a
   hand-opened one (the whole point of Q#DS3's ordering).
3. **Preorder is a stable leaf identity** — `active_leaf` index +
   restore's own preorder walk agree, so focus lands on the right leaf.
4. **No content-save is unsurprising** — restoring a modified buffer
   clean (with a warning) matches expectations, doesn't read as data
   loss.

## Deferred (named)

- **Daemon / GPU-attach restore** (Q#DS9) — first-attach trigger.
- Multiple named desktops per session key (one per key in v1).
- Window-local overlays / minor state (buffers + positions only).
- Remote/cross-machine desktops (paths are local).
- Saving unsaved buffer *content* (autosave, phase 3).
- Non-file (scratch/`*special*`) buffers in the desktop.

## Acceptance (Rust, tempdir state root injected)

No Lua tree API exists, so tests drive setup/inspection through Rust +
the `pmacs.session.*` bindings:
- Build a nested, asymmetric weighted split (two+ files); `save_desktop`;
  construct a fresh editor; `restore_desktop`; assert tree shape +
  weights + each window's buffer path + cursor + view_top + the active
  leaf.
- **Hidden buffer survives**: open file A, open file B in the same
  window (A now hidden), save, restore → both A and B are live buffers.
- **after-load sees the right active buffer**: a probe hook recording
  `(file_path, buffer)` at `buffer.after-load` fires once per restored
  buffer with that buffer active.
- Same file in two leaves → two distinct restored positions.
- Session-key scoping: a `name.*` desktop and a `cwd.*` desktop don't
  collide.
- Startup gate: armed + no file arg restores; armed + file arg does not.
- A modified buffer at save → restore opens clean + the warning count
  reflects it.
- A since-deleted file's leaf collapses, focus falls back to a surviving
  leaf (Q#DS10), and the restore doesn't abort.
- **No orphan windows**: after restore, `core.windows` for LOCAL holds
  exactly the rebuilt leaves — the pre-restore windows are gone.
