# LSP panels — framing (Arc 1b)

pmacs's LSP data layer answers references, document symbols, code
actions, and hover in full — and then renders one line in the
modeline. `lsp.find-references` reports "12 references" and throws
eleven away; `lsp.code-actions` applies `acts[1]` blind;
`lsp.document-symbols` prints a count; multi-line hover shows its
first line. Every one of these is flagged "future UX work" in
`builtin/runtime/lsp.lua`. This arc builds that UX — as **buffers**,
not new UI surfaces, so both frontends get it for free.

Roadmap context: `docs/roadmap-2026-07.md` Arc 1b (also the panel
substrate DAP will reuse). Direct trigger: the PR #93 validation note
("we probably want a more fine grained UI for this sort of stuff and
error checking").

## What already exists (verified)

- **The `*buffer-list*` idiom** (`builtin/commands/default.lua:320-545`)
  is a working panel: a named persistent buffer, wholesale
  re-rendered (`buf:delete` + `buf:insert`), buffer-local keymap
  (`scope = "buffer"` binds, installed once, never torn down), a
  `line_to_buffer` row→item map read via `pmacs.editor.cursor_line()`,
  `prev_buffer_id` capture + `q` restore, and a refresh that manually
  re-seats the cursor (because `switch_active_buffer` zeroes
  cursor/overlays — `editor_core.rs:2052-2071`). Gaps: no read-only
  enforcement, logic not reusable.
- **Read surfaces are uniform** `pmacs.<domain>.{getter, clear}(sid,
  uri)` over Rust stores: references = `pmacs.references.locations`
  (rows `{uri, line, col}`, 0-based, the byte==UTF-16 v0.1 caveat);
  outline = `pmacs.document_symbol.symbols` (**flat** rows with
  `depth` + optional `container` — indent, don't recurse); actions =
  `pmacs.code_action.actions` (the full ordered array; each item
  carries its own `edit`/`command`, so applying by index is pure Lua);
  hover = `pmacs.hover.current` (`contents` is the full multi-line
  string).
- **The cross-file visit template** is `go_to_definition`'s SP-4 path
  (`lsp.lua:1187-1213`): `push_jump` → `path_for_uri` →
  `find_or_open` → `move_active_cursor_to`.
- **The picker substrate** is `pmacs.minibuffer.read { source =
  function() return {...} end, on_accept }` — the
  `window.set-line-numbers` shape (`default.lua:231-246`), already
  dual-frontend since protocol v12.
- **Read-only is intercept-only** (no buffer flag exists): the REPL's
  `add_intercept` + `error()` + self-write bypass
  (`repl/init.lua:686-726`) is the reference implementation.

## Decisions

### Q#P1 — Panels are buffers; one shared `listview` runtime module

`builtin/runtime/listview.lua`:

```lua
listview.open {
  name = "*references*",       -- persistent buffer, found-or-created
  header = "12 references to `foo`   RET visit  n/p move  g refresh  q quit",
  rows = { { text = "...", item = <any> }, ... },
  on_visit = function(item) ... end,   -- RET/SPC
  on_refresh = function() return rows end,  -- g (optional)
}
```

It owns everything the buffer-list hand-rolls: ensure-buffer,
wholesale render, `line→item` map, buffer-local keymap (RET/SPC
visit, n/p + arrows move, g refresh, q quit), prev-buffer capture +
restore, cursor re-seat after render, and the read-only intercept
(Q#P3). Bindings install once per panel buffer and persist — the
buffer-list precedent; per-open teardown buys nothing for persistent
buffers. `*buffer-list*` itself is NOT migrated in this arc (working
code, separate risk); named as a follow-up.

### Q#P2 — Presentation: switch-in-place, `q` restores

Panels open in the current window via `pmacs.window.switch_buffer`;
`q` restores the saved previous buffer (scratch fallback if killed).
This is the only presentation that behaves identically in both
frontends today — the GPU renders exactly one buffer and cannot show
splits. TUI split placement (`display-buffer`-style) is a named
deferral, not a v1 behavior fork.

### Q#P3 — Read-only via intercept; its honest limits

Each panel buffer gets an `add_intercept` that `error("read-only
panel")`s every op; `listview`'s own renders write with
`{ bypass_intercept = true }`. Two recorded limits: (a) the intercept
guards the daemon's command/edit path — a CRDT-import write (a
semantic frontend's optimistic op) does not run the intercept chain;
Q#P6's round-trip seam is what actually keeps GPU typing out of
panels. (b) A real `read_only` buffer flag (enforced in both edit
paths, shipped to frontends) is deferred — it's the proper fix for
this AND the REPL's identical exposure, and deserves its own small
arc.

### Q#P6 — Panel input routing: round-trip buffers (the load-bearing catch)

RET on a panel row must *visit*, but in the GPU, Enter is
optimistic-eligible — it would locally insert `\n` into the panel
mirror and ship a CrdtOp, never reaching the buffer-local RET
binding. Same for plain chars (vandalizing the panel around the
intercept, per Q#P3a).

Fix, wire-free: a core-side set of **round-trip buffers**.
`pmacs.buffer.set_round_trip_input(buf, true)` marks a buffer;
`EditorState::dispatch_idle()` returns `false` while the active
buffer is marked. `DispatchIdle { idle: false }` already gates every
semantic frontend's optimistic path (M11.6), so with a panel focused,
every GPU key round-trips: RET dispatches into the buffer-local
binding (visit works), typing dispatches to self-insert and the
intercept rejects it cleanly. No protocol change, no GPU change.
`listview` marks every panel it creates. (The REPL can adopt the same
flag later for its GPU story — named follow-up.)

### Q#P4 — References list (`*references*`, the flagship)

`lsp.find-references` (`M-?`) keeps its request/store flow, then
opens a panel: one row per location, `path:line+1:col+1` with the
path shortened relative to the project root when possible. RET =
the SP-4 visit template (jump ring included), so `M-,` returns.
Modeline summary stays (it's now the panel's byline). Row *snippets*
(the referenced line's text) require reading target files that may
not be open — deferred, named.

### Q#P5 — Outline (`*outline*`) + code-action picker + hover doc

- **Outline**: `lsp.document-symbols` (`C-c o`) → panel rows indented
  `2 × depth` with a kind tag; RET restores the source buffer and
  moves to the symbol (same-file visit: `switch_buffer(prev)` +
  `move_active_cursor_to`).
- **Code actions**: no panel needed — `lsp.code-actions` (`C-c a`)
  opens `pmacs.minibuffer.read` with `"N: title"` candidates (index
  prefix disambiguates duplicate titles); `on_accept` applies the
  *chosen* item's `edit`/`command` through the existing apply branch.
  One action still auto-applies without prompting? No — always prompt
  when more than one; a single action applies directly (today's
  behavior, now correct instead of lucky).
- **Hover doc**: `lsp.hover` (`C-c h`) unchanged (first line in the
  echo area). New `lsp.hover-doc` (`C-c H`) renders the full
  `contents` into a `*lsp-help*` panel (rows non-visitable; q quits).

### Q#P7 — Coordinates

Panels inherit the v0.1 byte==UTF-16 assumption exactly as
`go_to_definition` does today (`lsp.lua:658-662`); the v0.2
position-encoding hardening remains one deferral, not four new ones.

## Phasing (each phase green + user-validated)

1. **`listview` module + Q#P6 round-trip seam + references list.**
   The seam is the only Rust change (core set + one binding +
   `dispatch_idle` clause + tests). Validate in TUI *and* GPU —
   phase 1's GPU validation is what scores bet #3.
2. **Outline + code-action picker + hover doc.** Pure Lua on the
   phase-1 substrate.
3. **As-built notes** + deferral ledger update.

Arc 2 interleave point (query-replace) after phase 2.

## Categorical bets (score at close)

1. **The listview module covers all three panels without per-panel
   Rust.** The whole arc lands with one small core seam (Q#P6) and
   zero protocol change.
2. **Cursor-zeroing bites once.** `switch_active_buffer` clears
   cursor/overlays; some flow (refresh, visit-then-return) will land
   the cursor somewhere surprising before the re-seat discipline is
   applied everywhere.
3. **GPU panels render via the existing BufferSnapshot/F29 path with
   no GPU changes.** The mechanism exists (mid-session CRDT upgrade +
   snapshot push); a panel buffer exercising it from a `switch_buffer`
   is the untested claim in this arc.
4. **Round-trip routing has one gap somewhere** — a key that neither
   round-trips nor falls through correctly while a panel is focused
   (the Q#C6-class finding of this arc).

## Deferred (named, not silently dropped)

- A real `read_only` buffer flag enforced on both edit paths and
  shipped to frontends (fixes panels + REPL properly; Q#P3).
- Migrating `*buffer-list*` onto `listview`.
- TUI split placement for panels (`display-buffer`-style).
- Reference row snippets (needs off-buffer file reads).
- Fuzzy filtering inside panels (type-to-narrow).
- Peek/preview on n/p (echo the target line without visiting).
- Call-hierarchy / workspace-symbols panels (data partially present;
  same idiom when wanted).
- REPL adopting the Q#P6 round-trip flag for its GPU story.
