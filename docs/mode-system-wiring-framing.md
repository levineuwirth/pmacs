# Mode system wiring — side-quest (cross-cutting substrate)

The keymap stack already supports mode-scoped bindings (`Scope::Mode`,
`KeymapStack::modes`, `bind_mode()`, and `resolve()` iteration over
`active_modes`), but **dispatch passes an empty mode list** (`&[]` at
`src/editor.rs:809`), so every `scope = "mode"` binding silently falls
through to the global keymap. This frames the minimal wiring to make
mode-scoped keybindings resolve, enabling per-language bindings (a future
markdown mode's `C-c C-c` family, Lua-mode bindings, etc.).

Side-quest backlog: `docs/side-quest-backlog.md:133-134` and the
prioritization at lines 246-249. The latter also mentions mode-scoped
settings, but `pmacs.config` deliberately has only global and buffer-local
scopes; this side-quest wires key resolution only. Per-language settings
remain the shipped pattern: a `buffer.after-load` hook calling
`pmacs.config.set_local`.

The auto-indent framing records the same empty-mode dispatch gap at lines
66-68, 291-294, and 434-436. This wiring is a necessary substrate for
modeline detection (backlog line 43), which will need somewhere to store
its result, but that feature is not blocked on it (language detection
already works through the extension chain).

## Ground truth (as of canonical `main` @ `2e37c04`, protocol v18)

All line numbers below reflect that tree.

- **`KeymapStack` already mode-aware** (`src/keymap_stack.rs`):
  - `Scope::Mode(String)` — variant ready in the scope enum.
  - `KeymapStack::modes: Vec<(String, Keymap)>` — the per-mode keymap
    storage, populated by `bind_mode()`.
  - `KeymapStack::bind_mode(name, seq, cmd)` — **works today**.
    Any Lua code can call
    `pmacs.keymap.bind { scope = "mode", mode = "rust", ... }` without
    error; it just never fires because dispatch never passes the mode name.
  - `KeymapStack::resolve(seq, buffer, active_modes)` — mode iteration
    over `active_modes` is ready, just never reached in active-context
    production resolution.
  - `KeyDispatcher::dispatch(chord, stack, buffer, active_modes)` —
    signature accepts modes; passes them through to `resolve`.

- **Dispatch callsite** (`src/editor.rs:800-810`):
  ```rust
  let active_buffer = Some(self.core.borrow().active_buffer_id());
  let action = {
      let stack = self.lua_host.keymaps().borrow();
      self.dispatcher.dispatch(chord, &stack, active_buffer, &[])
  };
  ```
  This is the sole production `KeyDispatcher::dispatch` call. The remaining
  `dispatch(…, &[])` calls are tests in `keymap_stack.rs`.

- **Effective-key introspection is independently mode-blind**:
  - `pmacs.describe.key` resolves with the active buffer but `&[]` for
    modes (`src/lua_bindings/mod.rs:5893-5907`). Its own comment says it
    and dispatch must update together when modes land.
  - `help::render_key`, reached by `pmacs.help.show_key` / describe-key
    and by followed `[key: …]` links, also resolves with `&[]`
    (`src/help.rs:84-102`, called at
    `src/lua_bindings/mod.rs:5466-5479` and `src/help.rs:417-435`).
    Existing links encode buffer context only for buffer-scoped bindings;
    a mode-scoped link carries no mode context.
  - `pmacs.keymap.lookup` resolves with neither a buffer nor modes
    (`src/lua_bindings/mod.rs:6112-6117`); it is the raw global lookup,
    not an effective-key query, and stays that way in this side-quest.

- **Buffer struct has no mode field** (`src/buffer.rs:158-219`). Language
  is tracked externally: `parse_lang_by_buffer` in `syntax.lua`,
  `attachments` in `lsp.lua`.

- **Lua API for mode bindings already exists**: `parse_scope_arg` at
  `src/lua_bindings/mod.rs:12548-12575` handles `scope = "mode"` with a
  `mode` field. `BindArgs::apply` calls `stack.bind_mode()`. Tests exercise
  this.

- **Keymap resolution order** (buffer-local → modes → global) is
  correct for the major-mode-as-primary design: buffer-local bindings
  (compile-mode's panels) take priority over mode bindings, which take
  priority over global.

- **Language detection** already runs per-buffer in `buffer.after-load`
  (`syntax.lua`), resolving through extension → LSP filetype →
  filename → shebang. The resulting language string is the natural major
  mode name. Detection can return a language with no bundled grammar; the
  syntax attach path deliberately rejects only the parse, not the language.

- **Config registry (#127):** `pmacs.config` has global and buffer-local
  scopes only. Language/project conventions are hooks that call
  `set_local`; this wiring does not add a third config scope.

- **Statusline provider mechanism (#125, protocol v18):** composable
  `pmacs.statusline` providers evaluate per frontend/window; the TUI
  composes their output into the mode line via `paint_mode_line`.
  This is the both-frontends display mechanism for any new fact that
  should appear in the mode line.

## Decisions

### Q#MSW1 — Store the major mode on `Buffer` (private field + accessors)

```rust
// src/buffer.rs
pub struct Buffer {
    // …existing fields…
    major_mode: Option<String>,         // private, per Buffer convention
}

impl Buffer {
    pub fn major_mode(&self) -> Option<&str> {
        self.major_mode.as_deref()
    }
    pub fn set_major_mode(&mut self, mode: Option<String>) {
        self.major_mode = mode;
    }
}
```

Motivation: the Rust core needs the mode at dispatch time without calling
into Lua. Adding it to `Buffer` makes it accessible via `Registry::get()`
from `EditorState`, and the field follows buffer identity without a second
lifecycle-managed map.

**Hot-path contract: resolving a key allocates nothing for mode lookup.**
Change `KeymapStack::resolve` and `KeyDispatcher::dispatch` from
`active_modes: &[String]` to `active_modes: &[&str]`. At dispatch, hold the
buffer-registry borrow only across pure keymap resolution, take
`Buffer::major_mode()` by reference, and pass `Option<&str>::as_slice()` as
the zero-or-one mode slice. `Scope::Mode` owns the name only when a binding
actually resolves, so the returned action remains valid after the registry
borrow drops and before any Lua command runs.

Rejected alternatives:
- **Lua table only** — would need a Lua call during dispatch to retrieve
  modes, risking borrow conflicts and adding latency.
- **EditorCore map** — duplicates what `Buffer` already owns and would
  need lifecycle management on buffer kill.
- **Clone into `Vec<String>` per keypress** — avoidable allocation in the
  input hot path.

### Q#MSW2 — Major mode name = detected language name, one field

The language detection chain already produces a string like `"rust"`,
`"python"`, `"markdown"`. Store exactly that as `Buffer.major_mode`.
No separate mode name space — the language name IS the mode name.
This means `pmacs.keymap.bind { scope = "mode", mode = "rust", ... }`
is both a "rust mode" binding and a "rust language" binding.

`active_modes` at dispatch will be `[major_mode]` — a single-element
slice for v1. The keymap resolution iterates modes in order, so a single
mode works without special-casing.

Displayed through the provider without cosmetic name transformation. The
raw language string (`"rust"`, `"cpp"`, `"javascript"`) enters the mode
line; the statusline framework's universal one-line sanitation still
applies. Cosmetic overrides belong in a user-registered provider. The
built-in provider (Q#MSW5) stays simple and faithful.

### Q#MSW3 — Minor modes deferred entirely

This wiring is the minimal bridge to make mode-scoped bindings resolve.
Minor modes (flycheck, evil, spellcheck) would need activation/deactivation
ordering, a toggle API, per-buffer minor-mode lists, and likely a
`buffer.after-mode-change` hook. None of that is needed for the mode
system to **work** — defer all of it to a follow-up.

What this means concretely:
- No `minor_modes: Vec<String>` field on `Buffer`.
- No `buffer.after-mode-change` hook; dispatch and statusline read the
  setter's value live, but packages receive no transition notification.
- No `pmacs.minor_mode` Lua API.
- No mode-line indication for minor modes.

### Q#MSW4 — Auto-initialize once on `buffer.after-load`; never on switch

The Rust side provides storage. The Lua side initializes it alongside
existing language detection in `syntax.lua`. Refactor
`attach_for_active_buffer` to accept an `initialize_mode` boolean and use
the language it already resolves:

```lua
local function attach_for_active_buffer(initialize_mode)
    local buf = pmacs.window.buffer()
    if not buf then return end
    local key = tostring(buf)
    local lang = pmacs.parse._has_view(buf) and parse_lang_by_buffer[key]
      or resolve_active_language(buf)

    -- Initialization is before the grammar gate: server-only languages
    -- are valid major modes even though syntax cannot attach a parse view.
    if initialize_mode and lang and pmacs.buffer.major_mode(buf) == nil then
        pmacs.buffer.set_major_mode(buf, lang)
    end

    if not lang or not pmacs.parse._has_language(lang) then return end
    -- Existing parse dispatch / overlay attach follows unchanged.
end
```

`buffer.after-load` calls `attach_for_active_buffer(true)`.
`buffer.after-switch` calls `attach_for_active_buffer(false)` after its
existing overlay reset. A switch reattaches syntax but **never**
auto-initializes or rewrites the mode.

This separation is load-bearing:

- A user hook can override the detected mode after load; later switches
  preserve it.
- `pmacs.buffer.set_major_mode(buf, nil)` is a real persistent clear, not
  an “uninitialized” state that the next switch silently repopulates.
- A server-only language detected via filetype/shebang gets a mode before
  `_has_language` rejects only its missing grammar.
- First language wins for parsing exactly as today. Changing the major mode
  explicitly does not silently swap an installed grammar or LSP attachment.

### Q#MSW5 — Mode displayed via a built-in statusline provider (both frontends)

`#125` already ships composable `pmacs.statusline` providers whose
output feeds `paint_mode_line` (TUI) and `StatuslineSegments` (GPU).
Instead of adding a `mode_name` parameter to the painter, ship a
built-in provider:

```lua
-- Registration: strict typed fields, unknown key = error.
-- ctx.buffer is the buffer handle for the window being painted,
-- NOT the active/focused buffer — this is correct for passive splits.
pmacs.statusline.register {
    name = "mode",
    side = "left",
    priority = 0,
    face = "ui.modeline",
    fn = function(ctx)
        local mode = pmacs.buffer.major_mode(ctx.buffer)
        if mode == nil then return "" end
        return "(" .. mode .. ")"
    end,
}
```

`ctx.buffer` is critical: the provider evaluates per-window, so a
passive split showing a Python buffer must say `"(python)"`, not
`"(rust)"` from the focused buffer. `pmacs.editor.active_modes()` is
unsuitable here for the same reason `pmacs.lsp.active_buffer_language()`
was replaced by `ctx.buffer` in #125's built-in LSP provider.

Position: after the protected left chrome (active marker, modified flag,
buffer name), formatted as `+* name (mode)`. `paint_mode_line` appends
custom left segments after `protected_left`, so the "between name and
modified marker" claim is impossible without painter changes — the
honest position is after the full protected block.

This gives both frontends the display at once, requires zero painter
signature changes (the `too_many_arguments` allow stays untouched),
and lets users disable/unregister the built-in handle and register their
own formatting. The `"ui.modeline"` face is inherited from the surrounding
chrome. Returning `""` for no mode is an ordinary successful omitted
segment under the provider framework; no failure latch is involved.

### Q#MSW6 — New Lua API surface

Two functions on `pmacs.buffer.*`:

- `pmacs.buffer.major_mode(id) -> string|nil` — returns the buffer's
  major mode name, or nil if none is set.
- `pmacs.buffer.set_major_mode(id, name)` — sets it; `name` is a string
  or nil to clear. A clear persists across buffer switches because only
  `buffer.after-load`, never `buffer.after-switch`, auto-initializes.

Additionally, `pmacs.editor.active_modes() -> table` returns the current
active mode list (for the active buffer), matching what dispatch would
resolve at the time of the call: either `{major_mode}` or `{}`. It is useful
for introspection from modes or the minibuffer. Passive-window consumers
must use the parameterized `pmacs.buffer.major_mode(ctx.buffer)` instead.

### Q#MSW7 — No protocol changes

Mode is a daemon-side concept. Frontends never see mode names — they
receive resolved key actions and styled text. The single exception is the
status line, where the GPU frontend receives the existing
`StatuslineSegments` payload and can display the mode if the provider
includes it. Zero new protocol messages.

### Q#MSW8 — Effective-key introspection uses the dispatch context

Every API that claims to describe the binding effective in the current
buffer must resolve with both that buffer and its major mode:

- `pmacs.describe.key`
- `pmacs.help.show_key` / `help::render_key` (the interactive
  describe-key path)

Both derive the borrowed zero-or-one mode slice from the same `Buffer`
field dispatch uses. `help::render_key` therefore accepts explicit borrowed
mode context rather than hard-coding `&[]`.

Help-link targets must also preserve the scope needed after `*help*` becomes
active. Keep the existing `@buffer:<id>` target for buffer-local bindings,
add `@mode:<name>` for mode bindings, and parse either into the context
passed to `render_key`. Global links carry neither. Following a mode link
therefore describes that mode binding rather than resolving against the
help buffer and falling through to global.

`pmacs.keymap.lookup` deliberately remains the raw global lookup. It has no
buffer parameter today and changing it into an ambient-context query would
silently change an existing API unrelated to describe-key.

## Bets

1. **Single-element `active_modes` covers the useful cases** — no one
   needs multiple active modes before minor-mode semantics exist.
   Compile-mode's buffer-local bindings already work as a substitute.
2. **Language name is the right mode name** — no user will want a
   `"rust-mode"` that differs from `"rust"`. If they do, init.lua can
   call `pmacs.buffer.set_major_mode` with a custom name.
3. **After-load-only initialization is sufficient** — every normally
   opened buffer gets `buffer.after-load`; later switches only restore
   views. The hidden-buffer (registry-only) gap is pre-existing: those
   buffers receive neither language detection nor syntax attachment and
   need their own fix.
4. **GPU optimistic-edit bypass is a non-issue for mode-scoped
   bindings** — the GPU frontend optimistically inserts plain printable
   characters (outside `BUILTIN_PAIR_CHARS`) and Tab without round-tripping
   through dispatch (RET and built-in pair chars round-trip, per Q#AI1
   and Q#AP1). A mode-scoped binding on a plain printable or Tab would
   silently not fire on GPU, but mode bindings are `C-c C-c`-style
   control chords, which the optimistic path never touches. Documented
   here so it is not a surprise if someone binds a plain printable in a
   mode.

## Deferred (named)

- **Minor mode system** — activation/deactivation ordering, toggle,
  minor mode list in buffer, minor-mode indicator in the mode line.
- **`buffer.after-mode-change` hook** — the explicit setter changes what
  dispatch/statusline read immediately, but there is no notification API
  for packages that want to react to transitions. Add that with dynamic
  mode-aware package semantics, not for this single built-in initializer.
- **Mode-scoped settings** — `pmacs.config` remains global +
  buffer-local. Per-language configuration uses an after-load hook calling
  `set_local`; a first-class mode scope needs its own precedence,
  introspection, and mode-change invalidation design.
- **Modeline detection** (`-*- mode: … -*-`, `vim: ft=…`) — a separate
  side-quest (`side-quest-backlog.md:43`). This framing just wires the
  mechanism; modeline detection can override the initialized mode via
  `pmacs.buffer.set_major_mode` when it lands.
- **Mode help display** (`describe-mode`) — straightforward once the mode
  is stored, but not table-stakes for wiring.

## Acceptance

Keymap acceptance is dispatch-driven (keypress → action) against the daemon
process. Statusline and introspection cases exercise their real evaluator /
Lua surfaces. Rust fixtures must empty `pmacs.lsp.config` before creation
unless the test intentionally exercises the LSP path — otherwise a real
server starts on buffer open.

1. **Mode binding resolves**: a Lua test registers
   `pmacs.keymap.bind { scope = "mode", mode = "rust", sequence = "C-c C-c",
   command = "test.cmd" }`, opens a Rust buffer (LSP config cleared),
   sends `C-c C-c`, and asserts `test.cmd` runs. The same sequence in a
   Python buffer is unbound and never runs `test.cmd`.

2. **No mode → no mode bindings**: a file with no detected language
   (e.g. a `.txt` with no config) has `major_mode = nil`; mode-scoped
   bindings never fire.

3. **Mode displayed in mode line**: the built-in `"mode"` statusline
   provider returns `"(rust)"` for a Rust buffer and no segment for an
   unknown-language buffer. Evaluation uses `ctx.buffer`; a split with
   Rust active and Python passive produces the correct per-window strings.

4. **Mode survives buffer switch**: open A (rust) and B (python), switch
   back and forth; `pmacs.buffer.major_mode` returns the correct language
   each time.

5. **Buffer-local beats mode beats global**: register the same sequence
   at all three scopes and drive all three cases: buffer-local fires when
   present; after removing it the active mode binding fires; in a buffer
   with no matching mode the global binding fires.

6. **`pmacs.editor.active_modes()` returns current mode list**: matches
   the single-element `{lang}` or empty table for unknown-language buffers.

7. **Explicit mode override is not clobbered**: call
   `pmacs.buffer.set_major_mode(id, "markdown")`, switch away and back;
   `pmacs.buffer.major_mode(id)` still returns `"markdown"`.

8. **Explicit clear is not clobbered**: clear a detected mode with
   `pmacs.buffer.set_major_mode(id, nil)`, switch away and back; the getter
   still returns nil and the detected-language mode binding does not fire.

9. **Server-only language receives a mode**: add a filetype/shebang mapping
   to a language with no bundled grammar, open a matching file, and assert
   that `major_mode` and `active_modes()` contain that language while no
   parse view is attached.

10. **Describe-key agrees with dispatch**: for a mode binding that dispatch
    resolves, `pmacs.describe.key` reports the mode command and
    `scope = "mode:rust"`, and `pmacs.help.show_key` renders the same command
    and scope rather than the global fallback. Following that mode binding's
    `[key: … @mode:rust]` link from command help produces the same result
    after `*help*` is active.
