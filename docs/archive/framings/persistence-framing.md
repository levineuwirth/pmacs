# Persistence — framing (Arc 3)

pmacs forgets everything on exit except minibuffer history. Reopen a
file and the cursor is at the top; restart and your open buffers,
splits, and recently-visited files are gone; a crash loses unsaved
work with no recovery. This arc adds the four classic session-
persistence features — **saveplace** (cursor memory), **recentf**
(recent files), **desktop-save** (buffers + layout + positions), and
**autosave + crash recovery** — generalizing the one persistence
pattern that already works (`$XDG_STATE_HOME/pmacs/` history).

Roadmap: `docs/roadmap-2026-07.md` Arc 3, including its open question
"what is a 'session' in a daemon world" (Q#PS6).

## What already exists (verified)

- **State-dir precedent** (`src/minibuffer.rs`): `resolve_history_dir`
  → `$XDG_STATE_HOME/pmacs/history` (fallback
  `~/.local/state/pmacs/history`); `load_history_file` /
  `append_history_file` (newline-delimited, `create_dir_all` on
  write). Env is passed as args, never read inline — the
  `#![forbid(unsafe_code)]` discipline. **But the `/history` segment
  is baked in; there is no generic `state_dir()`** returning the bare
  `.../pmacs/` base (Q#PS2).
- **Buffer/path model**: `Buffer.file_path: Option<PathBuf>` (`None`
  for scratch/unsaved) + `file_meta: Option<FileMeta>`; registry
  `ids()` is stable insertion order. Open flow is Lua
  (`pmacs.buffer.find_or_open` → `file_io::load_file` →
  `set_buffer_path`/`set_buffer_meta` → `buffer.after-load`). **No
  `is_special` flag** — file buffers are just `file_path().is_some()`.
  **Gap**: `pmacs.buffer.list()` yields ids with no per-id
  `file_path()` method; only `pmacs.editor.file_path()` (active
  buffer) reads a path.
- **Layout** (`src/window.rs`): `LayoutNode = Leaf(WindowId) |
  Split{orientation, weights: Vec<u32>, children}`; ratios are the
  integer weights (survive resize). **Not serde; `WindowId` is a
  process counter (not restart-stable)** — a saved layout must key
  leaves by *path + cursor + view_top* and rebuild structurally. Lua
  window API is splits + switch only (can't read/build an arbitrary
  tree).
- **Cursor/view**: per-`Window` `cursor: Position` + `view_top`;
  `switch_active_buffer` **zeroes both** (restore must run after
  open). Reads exist (`pmacs.editor.cursor()` byte); **the only Lua
  setter is line-based `move_to_line`** — no byte-offset setter, no
  real `set_view_top`.
- **File I/O** (`src/file_io.rs`, complete): `save_atomic` (temp +
  rename + fsync, mode-preserving), `load_file`, `current_meta`;
  `FileMeta{mtime,size}` is the external-change detector.
- **Cadence**: `process.after-tick` hook (every frame — needs a
  `monotonic_ms()` throttle) *or* `pmacs.async` + `pmacs.workers.sleep`
  (the `fs.watch` pattern — cleaner for a fixed interval).
- **Session identity**: `pmacs.instance.identity()` → `{ instance_name,
  working_directory }`. `--socket work` vs `--socket personal` give
  distinct `instance_name`s; `instance_name` is `None` for the default
  daemon / in-process (fall back to a `working_directory` hash).
- **Recentf / saveplace / desktop / autosave: confirmed absent.**

## Decisions

### Q#PS1 — Hybrid: four thin Rust primitives + Lua policy

Orchestration is Lua-friendly (enumerate via `pmacs.buffer.list`,
observe via `buffer.after-load` / `buffer.before-save` /
`editor.before-quit` hooks, cadence via `pmacs.async`, session key via
`pmacs.instance.identity`). But Lua can't do the load-bearing parts.
Add exactly four Rust surfaces; keep serialization + policy + cadence
in Lua builtin modules:

1. **`state_dir()` + `pmacs.state.{read,write,remove,path}`** — a
   generic, **path-confined** key→file store under the generalized
   state dir, atomic-write via `file_io::save_atomic` (avoids the
   audit-flagged raw `io.open`). `remove` is needed by autosave
   cleanup (Q#PS8).
2. **per-`BufferId` `file_path()`** getter (desktop-save enumerates
   file buffers without switching to each).
3. **`pmacs.editor.goto_byte(pos)` + `set_view_top(n)`** — byte-exact
   restore (saveplace/desktop), since switch zeroes both.
4. **`pmacs.session.save_desktop()` / `restore_desktop()`** — the
   layout serde mirror + structural rebuild live in Rust (WindowIds
   aren't restart-stable; the tree can't be rebuilt from the thin Lua
   split API). This one primitive owns buffer-set + layout + cursor
   serialization end-to-end.

### Q#PS2 — `state_dir()` generalization + `pmacs.state.*` (path-confined)

Factor `state_dir(xdg_state, home) -> Option<PathBuf>` returning
`.../pmacs`; rewrite `resolve_history_dir` as
`state_dir().map(|d| d.join("history"))`. **This is NOT a
behavior-preserving refactor at one edge** (an empty `XDG_STATE_HOME`):
the current `resolve_history_dir` treats `Some("")` as present and
returns a *relative* `pmacs/history` (writes into the cwd — a latent
bug). `state_dir()` **fixes this deliberately**: an empty/blank
`XDG_STATE_HOME` is treated as absent and falls through to
`~/.local/state/pmacs`. A test pins the new empty-XDG behavior; history
persistence keeps working (its own tests are the guard). Bet #3 is
scored on this being the *only* observable change to history.

`pmacs.state.{write(name,str), read(name)->str?, remove(name),
path(name)}` over `state_dir().join(name)`, atomic write +
`create_dir_all`. **Name confinement (High):** a name is a *relative*
key — reject absolute paths, empty names, any `.`/`..` component,
leading/trailing/`//` separators, and control chars; the only allowed
shape is one-or-more components of `[A-Za-z0-9._-]+` joined by `/`
(so `recentf`, `places`, `autosave/<hash>` pass; `../x`, `/etc/x`,
`a//b`, `` all reject). The resolved path is additionally asserted to
start with `state_dir()` (canonical-prefix belt). Without this,
`pmacs.state` would be the arbitrary-io primitive it exists to avoid.
Reads/writes are **no-ops when the state dir is unconfigured** —
which is the case under `cfg(test)` (Q#PS9), so default-on builtins
never touch a developer's real `$XDG_STATE_HOME` in `cargo test`.

### Q#PS3 — Serialization: line-based text for Lua state; Rust serde for desktop

Decided up front so phase 1 doesn't discover a fifth primitive: **Lua
has no public JSON codec** (`lua_to_json` exists but only internally
for the MCP wire), and it doesn't need one. The Lua-owned state is
line-based text, the history-file shape `pmacs.state` already returns:
`recentf` is newline-delimited paths; `places` is one
`<cursor> <view_top> <path>` line per file (numbers first so the path —
which may contain spaces — is the whitespace-split remainder). The one
breaker, a newline inside a path, is pathological and named as
deferred. **desktop-save serializes in Rust** (`pmacs.session.*`, Q#PS5)
with `serde_json` internally — never crossing the Lua boundary — so no
Lua JSON primitive is added anywhere in the arc.

### Q#PS3b — saveplace (Lua, phase 1)

`builtin/runtime/saveplace.lua`: on `buffer.before-save` and on
`editor.before-quit`, record `path → {cursor, view_top}` into the
`places` state file (the line format above); on `buffer.after-load`,
look up the active buffer's path and `goto_byte` + `set_view_top`.
LRU-cap the map (~200 files). **On by default** with a disable knob
(Q#PS9).

### Q#PS4 — recentf (Lua, phase 1)

`builtin/runtime/recentf.lua`: a handler on **both `buffer.after-load`
(first open) and `buffer.after-switch` (re-visiting an already-open
file buffer)** moves the active buffer's path to the front of a deduped,
capped (~50) `recentf` state file — MRU, so re-visits refresh the
order, not just first loads. A `recent-files` command + binding opens
`minibuffer.read` over the list (the `editor.switch-buffer` shape) →
`find_or_open`. Recording is automatic; the picker is invoked on demand.
(The Arc 1b `listview` panel is an alternative surface if a browsable
list is wanted later.)

### Q#PS5 — desktop-save (Rust `pmacs.session.*`, phase 2)

`save_desktop()` serializes `{ session-key, layout mirror
(orientation/weights tree; leaf = {path, cursor byte, view_top}),
active_leaf: usize }` to `state_dir()/desktop/<session-key>`. **Only
file buffers** (`file_path().is_some()`) — scratch/`*special*` leaves
are dropped from the tree. **The active window is a leaf *preorder
index*, not a path** (Med/high): the same file can appear in multiple
leaves with different cursor/view state, so a path can't identify which
one had focus — the ordinal into the preorder leaf sequence can.

**Does NOT save buffer contents.** Like Emacs `desktop.el`, it saves
the *file list + positions*, not unsaved edits (that's autosave's job,
Q#PS8). A leaf whose buffer was modified at save time is recorded
**informationally only** — restore opens the on-disk file (clean) and
surfaces a one-line warning ("N buffers had unsaved changes when the
desktop was saved"); it never reconstructs dirty state. `restore_desktop()`
opens each file (`find_or_open`), rebuilds the split tree structurally,
sets each leaf's cursor/view_top, and focuses `active_leaf`.

**Opt-in** (auto-restore surprises): `pmacs.session.desktop_mode(true)`
in init.lua wires `editor.before-quit` → save and *arms* startup
restore (Q#PS7 — restore is triggered by the entry point, not inline).

### Q#PS6 — Session key (the daemon question)

Key on `pmacs.instance.identity()`. **Never use raw `instance_name` as
a filename** (even though `/` is mostly constrained today, other
separators/dots aren't) — use a stable *encoded* key: `name:<encoded>`
when `instance_name` is set (`--socket NAME`), else `cwd:<hash>` of
`working_directory` (Emacs's per-directory desktop model). The
encoding must itself satisfy the Q#PS2 name confinement (it becomes
the `desktop/<key>` subpath), so `<encoded>` is a hex/percent form or
a hash, never the raw string. So `--socket work` and `--socket
personal` restore different desktops; two plain `pmacs` sessions in
different project dirs likewise; the default daemon in one cwd shares
one desktop. Tests cover both the named-socket key and the
cwd-fallback key. No new identity is needed — the socket name already
threads to `InstanceIdentity`.

### Q#PS7 — Restore timing (restore is deferred, never inline in init)

The startup order is the trap: `EditorState::open(path)` calls
`EditorState::new()` first, and `new()` loads `init.lua` **before** the
file is opened (`src/editor.rs`). So `desktop_mode(true)` running inside
init cannot know a positional file arg is coming — if it restored
inline it would clobber (or race) the file the user asked for.

Therefore **`desktop_mode(true)` does not restore; it arms restore.**
It registers intent (a flag the core reads). The startup entry point —
after `new()`/`open()` has done its file-open routing — calls
`pmacs.session.restore_desktop()` exactly once, and **only when no
positional file arg was given** (a file arg means "open this," not
"restore my desktop" — Emacs's rule). Concretely: `main` threads a
`restore_desktop: bool` (true iff armed AND no file arg) into the
post-construction step that triggers the restore. Restore never runs
from init; init only sets the mode.

### Q#PS8 — autosave + crash recovery (Lua, phase 3)

A `pmacs.async` + `sleep(N s)` loop (the `fs.watch` pattern) writes a
recovery copy of each *modified* file buffer to
`state_dir()/autosave/<path-hash>` plus a sidecar recording the origin
path + the on-disk `FileMeta`. On `find_or_open`, if a recovery file
exists and the on-disk file's `FileMeta` matches the sidecar (the file
wasn't changed elsewhere), prompt to recover. A clean save/kill
**deletes the recovery file via `pmacs.state.remove`** (the Q#PS2
addition — no raw `pmacs.fs.remove(pmacs.state.path(...))`, which would
route around the confinement). Writes use `pmacs.state.write` under the
`autosave/` subpath; no new primitive beyond Q#PS2.

### Q#PS9 — Default-on policy + the disable knob + test inertness

saveplace + recentf: **on by default** (low-risk, quietly useful,
Emacs's `save-place-mode`/`recentf-mode` are commonly enabled).
desktop-save + autosave: **opt-in** via init.lua (auto-restore and
background writes are surprising to enable silently).

Two hard requirements on default-on:

- **A clear disable knob**: `pmacs.saveplace.enable(false)` /
  `pmacs.recentf.enable(false)` (callable from init.lua) short-circuits
  the hooks. The modules read an enabled flag at the top of each
  handler.
- **No writes in `cargo test`.** The mechanism is already there: the
  state dir is *configured once at startup* (like `history_dir`), and
  that wiring is **skipped under `cfg(test)`** (the same guard that
  skips init.lua loading). With no state dir configured,
  `pmacs.state.write/read/remove` are no-ops (Q#PS2), so the default-on
  saveplace/recentf hooks fire but touch no disk — the lib suite never
  writes a developer's real `$XDG_STATE_HOME`. Acceptance tests that
  *do* exercise persistence inject a tempdir state root explicitly (the
  `load_user_config_at` precedent).

## Phasing

Three PRs; phase 1 delivers standalone value and is validated before
phase 2.

1. **State foundation + saveplace + recentf.** Rust: `state_dir()`
   (+ empty-XDG fix + test) + `pmacs.state.{read,write,remove,path}`
   (+ name-confinement rejection tests) + `goto_byte`/`set_view_top`.
   Lua: `saveplace.lua` + `recentf.lua` + `recent-files`
   command/binding + `enable(false)` knobs. Acceptance (tempdir state
   root injected): state round-trip, confinement rejects `../x`
   / absolute / control chars, saveplace restores byte position after
   reopen, recentf records + dedups + MRU-refreshes on re-visit + the
   picker opens; and a check that with no state dir configured the
   hooks write nothing.
2. **desktop-save.** Rust: per-buffer `file_path()` + layout serde
   mirror + `pmacs.session.save_desktop/restore_desktop` + encoded
   session-key. Lua: `desktop_mode` wiring (before-quit / arm-startup).
   Acceptance: save→restore reconstructs a nested/asymmetric split
   layout with correct buffers + cursors + `active_leaf` focus; the
   same-file-in-two-leaves case; named-socket vs cwd-fallback key;
   the no-file-arg restore gate; modified-at-save surfaces a warning
   and restores clean (never dirty).
3. **autosave + crash recovery.** Lua: async-timer autosave +
   recovery-on-open prompt; `FileMeta` external-change guard.

## Categorical bets (score at close)

1. **Four Rust primitives are enough** — the Lua policy fills in
   saveplace/recentf/autosave and only desktop-save's layout needs
   Rust. No fifth surface surfaces mid-arc.
2. **Structural layout rebuild is faithful** — reopening the serde
   mirror reconstructs weighted split trees correctly (the untested
   claim; nested/asymmetric splits are the risk).
3. **The `state_dir()` change touches history at exactly one edge** —
   the deliberate empty-`XDG_STATE_HOME` fix (Q#PS2) is the *only*
   observable difference; normal history persistence is unchanged (its
   tests + a new empty-XDG test are the guard). Not a pure refactor —
   scored on nothing else shifting.
4. **On-by-default saveplace/recentf is unsurprising** — no
   "why did my cursor jump" or "what's writing this file" reports.

## Deferred (named, not silently dropped)

- Saving unsaved buffer *content* in the desktop (autosave's job).
- Named/multiple desktops per session key (one desktop per key in v1).
- Restoring window-local overlays / minor state (buffers + positions
  only).
- Remote/cross-machine desktop (paths are local).
- recentf as a browsable `listview` panel (minibuffer picker in v1).
- Per-project `init.lua` interaction with desktop scoping (post-v0.1,
  per `docs/project.md`).
- saveplace for non-file buffers.
- Paths containing a literal newline (the line-based `places`/`recentf`
  format breaks on them; pathological, unhandled in v1).
