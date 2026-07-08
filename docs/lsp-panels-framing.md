# LSP panels — framing (Arc 1b)

> **Status: ARC COMPLETE** (PRs #94 + #95 merged 2026-07-08; this doc
> is the as-built record). The framing below is preserved as written;
> the intro and "What already exists" sections describe the state
> *before* the arc (present tense = the pre-arc baseline), and the
> scored bets + as-built divergences at the end record what actually
> shipped and diverged.

*(Pre-arc baseline.)* pmacs's LSP data layer answers references,
document symbols, code actions, and hover in full — and then renders
one line in the modeline. `lsp.find-references` reports "12
references" and throws eleven away; `lsp.code-actions` applies
`acts[1]` blind; `lsp.document-symbols` prints a count; multi-line
hover shows its first line. Every one of these is flagged "future UX
work" in `builtin/runtime/lsp.lua`. This arc builds that UX — as
**buffers**, not new UI surfaces, so both frontends get it for free.
*(As-built: all four now open panels — references/outline/hover-doc
via `listview`, code actions via the minibuffer picker. See the
scored bets at the end.)*

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
  re-seats the cursor (because `EditorCore::switch_active_buffer`
  zeroes cursor/overlays). Gaps: no read-only enforcement, logic not
  reusable.
- **Read surfaces are uniform** `pmacs.<domain>.{getter, clear}(sid,
  uri)` over Rust stores: references = `pmacs.references.locations`
  (rows `{uri, line, col}`, 0-based; `col` is a pmacs byte offset —
  the transport layer converts to/from the server's negotiated
  encoding, see Q#P7);
  outline = `pmacs.document_symbol.symbols` (**flat** rows with
  `depth` + optional `container` — indent, don't recurse); actions =
  `pmacs.code_action.actions` (the full ordered array; each item
  carries its own `edit`/`command`, so applying by index is pure Lua);
  hover = `pmacs.hover.current` (`contents` is the full multi-line
  string).
- **The cross-file visit template** is `go_to_definition`'s SP-4 path
  (the "Cross-file (SP-4)" block in `builtin/runtime/lsp.lua`):
  `push_jump` → `path_for_uri` → `find_or_open` →
  `move_active_cursor_to`. *(As-built: extracted as the shared
  `visit_location` helper, which references and outline both call.)*
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

Panels inherit whatever the existing cross-file visit does — no new
coordinate handling. **As-built correction (the wire encoding has
since landed):** the LSP *transport* now negotiates
`general.positionEncoding` and converts every `Position` at the
request/response boundary (`PositionEncoding` +
`char_to_byte`/`byte_to_char` in `src/lsp.rs`), so location rows reach
Lua as pmacs byte offsets — the "byte==UTF-16 on the wire" caveat this
section originally cited is resolved, not deferred. The one residual
is narrower and shared with `go_to_definition`, not introduced here:
`move_active_cursor_to` (`builtin/runtime/lsp.lua`) walks `col` as
codepoint steps (`move_right`), exact for single-byte-per-codepoint
text but off for multi-byte lines. Panels call the same helper, so
they are exactly as correct as go-to-definition — the framing's
"inherit, don't multiply" intent holds; only the *residual* is a
cursor-walk detail, not a wire-encoding gap.

## Phasing (each phase green + user-validated)

1. **`listview` module + Q#P6 round-trip seam + references list.**
   The seam is the only Rust change (core set + one binding +
   `dispatch_idle` clause + tests). Validate in TUI *and* GPU —
   phase 1's GPU validation is what scores bet #3.
2. **Outline + code-action picker + hover doc.** Pure Lua on the
   phase-1 substrate.
3. **As-built notes** + deferral ledger update.

Arc 2 interleave point (query-replace) after phase 2.

## Categorical bets (scored at close — ARC COMPLETE, PRs #94 + #95)

1. **The listview module covers all three panels without per-panel
   Rust — HELD.** One core seam (Q#P6: a `HashSet` + one binding +
   one `dispatch_idle` clause), zero protocol change; outline,
   references, and hover-doc are all pure Lua `listview.open` calls,
   and the code-action picker reused the minibuffer dropdown with no
   panel at all.
2. **Cursor-zeroing bites once — HELD** (mild): the re-seat
   discipline (`seat_cursor` after every render/switch) was applied
   from the start, so the predicted surprise never surfaced in the
   panels; the *latent* form of it bit elsewhere (bet #3's finding 2).
3. **GPU panels render with no GPU changes — SCORED FALSE, the
   arc's load-bearing finding.** Two blocking bugs at GPU validation,
   neither a GPU change but both invisible to the bet as framed: (a)
   the F29 snapshot push fires only on the *upgrade* tick, so
   switching *back* to a known buffer sent nothing and the GPU froze
   on the old buffer while input targeted the new one — fixed with a
   daemon active-buffer-*follow* path (semantic sessions only; a
   first cut gated on `crdt_replica` duplicated the TUI's init-once
   snapshot and broke *all* attach — round-2 regression). (b) A
   long-latent bug the panels made constant: `switch_active_buffer`
   clears window overlays and the runtime dedup tables blocked
   re-attach, so *every* buffer switch (plain `C-x b` included) had
   been stripping syntax/LSP/diagnostic styling — fixed with a new
   `buffer.after-switch` hook. **Lesson: "the mechanism exists" is
   not "the mechanism fires on this trigger" — snapshot delivery was
   coupled to CRDT *upgrade*, not active-buffer *change*.**
4. **Round-trip routing has one gap somewhere — NOT OBSERVED.** Q#P6
   held cleanly in both frontends; no key mis-routed while a panel
   was focused.

## As-built divergences

- **Code actions took no panel.** The picker is `minibuffer.read`
  with `"N: title"` candidates (bare index also accepts), not a
  `listview` — a single action still applies directly. Cleaner than a
  list buffer for a pick-one-and-close flow.
- **The two blocking findings were daemon/runtime, not panel Lua**
  (bet #3) — the panel code itself needed no correctness fixes across
  either PR.
- **Coverage caught a process gap**: the phase-2 tests initially
  shipped with a `too-many-lines` clippy deny masked by a swallowed
  exit code in a chained local gate command. Run `clippy` as its own
  gate step, not `&&`-chained after a test whose exit code you read.

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
