# Right-click context menus — framing + as-built

pmacs's first interactive floating surface. Unlike search (a band +
washes), a context menu is a thing you *point at and click*, which broke
new ground in both frontends: pointer hit-testing against a popup, a
highlighted active row, and dismissal rules. This doc records the design
as shipped; where the build diverged from the framing stance, the
"As-built" notes and the divergences section say what actually landed
and why.

User-decided up front (AskUserQuestion):

- **Context-sensitive** — items adapt to what's under the pointer (a
  selection → Cut/Copy/Paste; a symbol → LSP go-to-def / rename; a
  diagnostic → quick fix).
- **Both frontends, GPU-primary** — a shared core menu mode rendered in
  the TUI *and* pmacs-gpu; the GPU surface was the bulk of the work.
- **Lua registry** — items defined in Lua like commands and keymaps,
  user-configurable, actions invoking named commands.
- **OS clipboard** — copy/cut publish to the system clipboard (the menu
  needed real Cut/Copy/Paste, which didn't exist). See Q#CM6.

## Architecture

### Q#CM1 — Menu state lives in the core, mirroring the search store

The open menu is a frontend-agnostic **`SharedMenu =
Arc<Mutex<Option<MenuState>>>`** on `EditorCore` — the menu twin of
`search_store`. `MenuState { rows, active, anchor, width }`; a
`MenuRow` is `Separator | Item { label, command }`; `active` indexes
`rows` and always points at an `Item` (navigation skips separators).
The core drives it through `menu_open` / `menu_close` / `menu_step` /
`menu_set_active_row` / `menu_active_command` / `menu_hit`.

`EditorState` (which has both the core and the Lua host) is where the
menu is *built* and *invoked*: `open_context_menu` (TUI, cell anchor) /
`open_menu_at_byte` (GPU, byte anchor) resolve the rows and call
`menu_open`; `dispatch_menu_key` navigates; `menu_invoke_active` runs
the chosen command by name. This is the *same* dispatch path the daemon
runs for round-tripped GPU input, so the menu behaves identically in
both frontends — only the *surface* differs. The SearchSession bet held;
the new axis was mouse interaction.

### Q#CM2 — Items defined in a Lua registry, mirroring commands/keymaps

A `MenuRegistry` (`Rc<RefCell<…>>`, like `CommandRegistry` /
`KeymapStack`), installed as `pmacs.menu`, loaded from
`builtin/menus/default.lua` after keymaps in `lua.rs::attach_editor`
(items reference commands, so they load last). The Lua surface:

```lua
pmacs.menu.item {
  id      = "edit.cut",      -- optional; enables override / removal
  label   = "Cut",
  command = "edit.cut",      -- invoked by name via the command registry
  context = "selection",     -- sugar (Q#CM3); or predicate = fn(cx)
  group   = "edit", order = 10,
}
```

`MenuItem { id, label, command, context, predicate, group, order }`.
`item` registers (validating non-empty label/command and that `context`
is one of `always|selection|symbol|diagnostic` — the same R50
typo-paranoia as the command spec); a matching `id` replaces in place,
so re-running config and user overrides are idempotent. `list` /
`remove` / `clear` round out the surface; `_raw` (internal) exposes
items *with* their predicate functions for the Lua builder. The default
menu lives in Lua, so users re-order, hide, or add items without
recompiling — the pmacs way.

### Q#CM3 — Visibility resolves in Lua; `context` is sugar over a predicate

The dormant `Command.predicate` field finally got a consumer. The whole
resolve happens in **`pmacs.menu.build()`** (Lua), called once from Rust
(`EditorState::build_menu_rows`) at open: it reads `_raw()`, evaluates
each item's visibility, groups and sorts, and returns the rows. Building
in Lua keeps the LSP/diagnostic queries where their APIs live and the
core frontend-agnostic.

Visibility = an explicit `predicate(cx)` (a `pcall`, so a throwing
predicate hides its item rather than aborting the menu), else the
`context` tag via `pmacs.menu._context_eval(tag, cx)`, else visible. The
context table is built from core facts:

```lua
cx = { has_selection,   -- ed.region() ~= nil
       word,            -- ed.word_at_cursor() (core helper), or nil
       line, col,       -- ed.cursor_line() / cursor_col() (0-based)
       attachment }     -- pmacs.lsp.active_attachment(), or nil
```

Tag semantics: `always` → true; `selection` → `cx.has_selection`;
`symbol` → `cx.word ~= nil and cx.attachment ~= nil`; `diagnostic` → a
published diagnostic in `pmacs.diag.list(cx.attachment.uri)` spans
`(cx.line, cx.col)`. After filtering, items sort by `(group
first-appearance order, item order, insertion)`, and a separator falls
between distinct groups. **Bet (held):** synchronous local context picks
the right groups without an LSP round-trip at open.

`ed.word_at_cursor()` is a core helper (identifier run of ASCII
alphanumerics / `_` around the cursor). `pmacs.lsp.active_attachment()`
is a new *pure* accessor added for this — the existing
`attached_for_active` triggers an attach as a side effect, which a
visibility check must not do.

### Q#CM4 — Right-click anchoring

If a selection exists, right-click keeps it (so Copy/Cut act on it) and
leaves the cursor; otherwise it moves the cursor to the click and clears
any selection. The byte anchor means LSP items act where you clicked.
(The framing's finer "keep iff click is *inside* the selection" was
simplified to "keep iff a selection exists" — uniform across both
frontends, and the TUI path never needs the click byte.)

## Commands & the clipboard

### Q#CM5 — The default menu, as shipped

`builtin/menus/default.lua`, grouped (separators between groups):

- **edit** — Cut, Copy (`selection`), Paste, Select All (`always`).
- **symbol** (`symbol`: word + attached server) — Go to Definition,
  Find References, Rename, Hover. Each invokes the existing async
  `lsp.*` command, which acts at the cursor the right-click anchored.
- **diagnostic** (`diagnostic`: a diagnostic spans the cursor) — Quick
  Fix → `lsp.code-actions`. Streaming the *individual* fix titles into
  the menu is deferred (Q#CM10); one "Quick Fix" entry that fires the
  async command was the Phase-1 stance.
- **history** — Undo, Redo (`always`).

### Q#CM6 — Clipboard with OS interop

A right-click menu without working Cut/Copy/Paste isn't credible, and
none existed — pmacs had no clipboard at all, and had **never honored a
paste** (`FrontendEvent::Paste` was silently dropped). This arc added
`edit.copy/cut/paste/select-all` with copy/cut publishing to the real
system clipboard.

The wrinkle: **commands run in the core (daemon), but the OS clipboard
belongs to the frontend's environment** (the winit display, or the
terminal — possibly across SSH). So the model is an *internal slot*
(core-owned, the synchronous paste source) plus outbound publish and
inbound capture across the wire:

- **`edit.copy` / `edit.cut`**: extract the region text → write the slot
  → queue an outbound publish the dispatcher drains as
  **`InstanceSignal::Clipboard(bytes)`** (a v6-floor variant, already in
  the protocol but never produced — **no new message, no version
  bump**) → the frontend writes the OS clipboard (pmacs-gpu via
  `arboard`, TUI via **OSC 52**, write-reliable). Cut then deletes the
  region.
- **`edit.paste`**: insert the slot at the cursor, replacing any region.
- **Inbound OS → pmacs** rides each frontend's native paste affordance,
  landing as `FrontendEvent::Paste`: the TUI's bracketed paste (already
  enabled), and a new pmacs-gpu `Ctrl-V` → `arboard.get()` → `Paste`.
  The core inserts *and* refreshes the slot, so a later in-app paste
  repeats the external text.

Default keys are the **Emacs kill/yank set** — `M-w` copy, `C-w` cut,
`C-y` yank, `C-x h` select-all — because the CUA trio collides (`C-a` is
line-start, `C-v` is page-down). In pmacs-gpu these chords are forwarded
like the search-entry chords (otherwise withheld); `Ctrl-V` is handled
locally for OS paste. `arboard` (with `wayland-data-control`) is a new
pmacs-gpu dep.

## Input & routing

Right-click **opens**; while open, the menu **captures** pointer and
keyboard until an item fires or it's dismissed.

- **Keyboard (both):** `Down`/`C-n` next, `Up`/`C-p` prev, `RET`
  invoke, `Esc`/`C-g` dismiss; any other key dismisses. Decoded by
  `MenuKey::from_chord`, the SearchKey pattern — the same path both
  frontends reach via the `FrontendEvent::Key` round-trip. `dispatch_key`
  / the GPU's `dispatch_idle` gate both treat an open menu like an
  active search.
- **TUI:** a self-suppressing **`MenuView` overlay** (the `SearchView`
  pattern — deduped by kind, renders nothing while closed) pushed on the
  active window at open, drawn to cells over the buffer text. Right-click
  arrives already as `FrontendEvent::Mouse(Down(Right))` (was dropped);
  `dispatch_mouse` gains a Right arm to open and a while-open branch
  routing `Move`/`Drag`→highlight, `Down(Left)`→invoke (hit-testing the
  cell against the popup rect), click-outside→dismiss.
- **GPU:** the core ships the resolved rows as a **`MenuPrompt`**
  semantic message; the GPU renders the popup at the remembered
  right-click pixel and owns hit-testing (it drew the rect), translating
  hover/click into **`FrontendEvent::MenuPointer { index, invoke }`** —
  never shipping pixels the core can't read. Right-click opens via
  `PointerKind::Context`; Escape dismisses instead of quitting.

## Wire (protocol v11)

Additive over v10; the ladder resumes (`SUPPORTED = [6,7,8,9,10,11]`):

- **`PointerKind::Context`** — right-button-down as a semantic pointer
  (carries the hit byte). Frontend-gated like `Pointer`/`TripleDown`
  (pmacs-gpu drops it against a `< 11` daemon).
- **`FrontendEvent::MenuPointer { frontend_id, index: Option<u32>,
  invoke: bool }`** — GPU→daemon navigation. `index: None` = pointer off
  the menu; `invoke` = click (invoke the row, or dismiss when `None`).
- **`InstanceMessage::MenuPrompt { buffer_id, rows: Vec<MenuPromptRow>,
  active: Option<u32> }`**, `MenuPromptRow { label, separator }`. Empty
  `rows` closes the menu. Emitted by the semantic producer with
  cached-compare suppression (like `SearchPrompt`); daemon-gated `>= 11`,
  so a v10 peer never opens a GPU menu rather than mis-decoding it.

Clipboard added **no new wire** (reused `InstanceSignal::Clipboard` +
`FrontendEvent::Paste`). The TUI needs **no new wire at all** (menu =
overlay cells + existing `Mouse`; clipboard = existing Signal/Paste);
the v11 bump is the GPU menu's alone.

## Phasing (delivered; each commit binary-build-green)

1. **Registry** — `MenuRegistry` + `pmacs.menu` + load wiring.
   Introspection only.
2. **Clipboard + select-all** — commands + internal slot + reused
   `InstanceSignal::Clipboard` + per-frontend OS write (arboard / OSC
   52) + GPU `Ctrl-V` inbound + wired the previously-dropped
   `FrontendEvent::Paste`. Emacs kill/yank keys. Validated standalone.
3. **Core mode + TUI surface** — `MenuState`/`SharedMenu`, `MenuView`,
   `dispatch_menu_key`/`_mouse`, right-click open, the Lua `build`
   resolver. End-to-end in the terminal (user-validated).
4. **Default menu content** — the symbol/diagnostic context tags
   (`word_at_cursor`, `active_attachment`, diagnostic containment) and
   the LSP items.
5. **Protocol v11 + GPU surface** — `Context` / `MenuPointer` /
   `MenuPrompt`, the daemon routing + gate, the producer, and the GPU
   popup (a second `TextRenderer` over bg quads, pixel hit-testing →
   `MenuPointer`). End-to-end in pmacs-gpu (user-validated).
6. **Docs** — this consolidation.

Phases 1–4 validated the whole feature in the TUI before the GPU surface
(the expensive half) began. The v11 bump means the daemon and pmacs-gpu
must both be rebuilt to negotiate the menu.

## As-built divergences from the framing

1. **No `SetClipboard` message.** The framing proposed a new
   daemon→frontend clipboard message; the protocol already carried
   `InstanceSignal::Clipboard` (outbound) and `FrontendEvent::Paste`
   (inbound) since the v6 floor — both unused. Reusing them meant
   clipboard cost *zero* new wire and no bump.
2. **Resolve runs entirely in Lua, not Rust.** The framing had the core
   evaluate predicates against a Rust-built context table. As-built,
   `pmacs.menu.build()` does the whole resolve (predicate eval, context
   tags, grouping, sorting, separators) in Lua; Rust calls it once and
   parses rows. The `symbol`/`diagnostic` tags need `pmacs.lsp` /
   `pmacs.diag`, which live in Lua — so the policy belongs there.
3. **`MenuPrompt` carries rows, not "MenuItemWire".** The framing
   sketched `MenuItemWire { label, enabled, separator_before }`; the
   shipped `MenuPromptRow { label, separator }` is leaner — the GPU
   needs only labels + separator flags + the active index, never the
   command names (the core invokes by index).
4. **Menu state is a `SharedMenu`, no `origin`.** The framing's
   `MenuSession { …, origin }` implied a cursor-restore on dismiss;
   right-click already anchors the cursor intentionally, so there's
   nothing to restore. The state is the search store's `Arc<Mutex>`
   shape, read by the TUI overlay.
5. **Anchoring simplified** to keep-iff-a-selection-exists (Q#CM4).
6. **Emacs clipboard keys** (`M-w`/`C-w`/`C-y`/`C-x h`), because the CUA
   trio's keys were already bound (Q#CM6).
7. **GPU select-all has no keyboard binding** — `C-x` is a prefix the
   GPU doesn't forward, so `C-x h` is TUI-only; in pmacs-gpu select-all
   is menu-only.

## Categorical bets (all held)

- **Core-mode generalized from search to a pointed-at popup.**
  `SharedMenu` + `MenuPointer` was the right seam; the core stayed
  frontend-agnostic.
- **Predicates were enough for context** — sync local facts, no LSP
  round-trip at open.
- **The GPU popup was just quads + glyph rows** — no new pipeline; the
  status-band `MinimapRect` quad path plus a second glyphon
  `TextRenderer` (so the popup layers over the buffer text) composed
  into it. The only real risk was draw order, handled by drawing the
  menu last.
- **The internal clipboard was a clean prerequisite, not scope creep.**

## Deferred (named, not silently dropped)

- **Q#CM10** Async quick-fix titles streamed into the menu (vs the one
  "Quick Fix" item firing `lsp.code-actions`).
- menu-Paste reading the OS clipboard *directly* in pmacs-gpu (vs the
  slot), and clipboard history / kill-ring.
- Paste routed into an open minibuffer prompt (today inbound paste
  always targets the buffer).
- A keyboard menu key (Shift-F10 / Menu) to open at the caret, and
  GPU select-all-by-keyboard (needs `C-x` prefix forwarding).
- Submenus / nested groups; first-letter mnemonic jump within a menu.
- CUA clipboard chords (`Ctrl-C`/`Ctrl-X`) as an alternative binding set.
