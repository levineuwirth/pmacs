# pmacs roadmap — July 2026

Date: 2026-07-07. Produced from a five-way codebase/docs/memory sweep
(core editing + persistence, LSP surface, GPU parity, extensibility +
terminal, deferred-work inventory across all framing docs).

> **Historical planning snapshot.** Several arcs below have since
> landed. Use `docs/agent-handoff.md` for durable current state and
> `docs/active-work.md` for open branches and recovery instructions.

**Decision (2026-07-07): push Arc 1 (LSP utility surface), with Arc 2
(editing table stakes) items interleaved between sub-arcs.**

---

## State assessment (as of main @ ccb0ff6, protocol v14)

**Strong.** CRDT multi-frontend core; both frontends at near input/render
parity (GPU exceeds TUI: minimap, wavy squiggles); LSP data layer — 13
language ids / ~8 servers, rename + cross-file definition + formatting
fully working, semantic tokens + inlay hints rendering; package manager
(git installs, lockfile, resolver, SHA-256 hardening); MCP client; async
worker + PTY subprocess substrate; bundled line-oriented REPL package;
headless GPU render CI; help system; buffer-list UI; atomic save;
persisted minibuffer history.

**Dark matter — built but unwired (highest leverage).**

- Complete completion framework (`src/completion_framework.rs`:
  providers for lsp/snippets/project-symbols/dabbrev; store;
  `CompletionView` popup in `src/completion.rs`) — **no keybinding, no
  typed-char trigger, popup never instantiated, no GPU message.**
- `HoverView` (`src/hover.rs`), `SignatureView` (`src/signature.rs`),
  document-highlight store — plumbing only. Hover / signature /
  references / document-symbols surface as one-line modeline summaries
  (`builtin/runtime/lsp.lua` marks each "future UX work").
- Code actions apply the **first** action blindly — no picker.
- Semantic-token pull is refresh/manual-only (no pull on attach/edit) —
  semantic styling can silently never appear. Arguably a bug.
- Wire-declared but unproduced protocol families: `FoldState`,
  `BlockAdornments`, `ResourceOffer` (no fold engine / blame / diff
  source).

**Absent.** Query-replace, kill ring (single slot only), keyboard
macros, rectangles, registers, comment/uncomment, auto-indent,
snippets, auto-pairing; desktop-save/session restore, recentf,
saveplace, autosave, backups, crash recovery; theming beyond syntax
captures (all chrome hardcoded per-frontend; GPU font hardcoded);
terminal emulation (`src/ansi.rs` parses but discards alt-screen /
cursor addressing — no grid/scrollback); compile/grep/shell-command
modes; DAP debugging; GPU splits / multi-buffer / auto-reconnect.

---

## Arcs, ranked by value-per-effort

### Arc 1 — LSP utility surface: "light up the dark matter" — COMPLETE

The completion popup and LSP utility panels shipped across #92–#96,
followed by hardening in #102, #105, and #106. Completion, hover,
references, document symbols, and the supporting semantic/signature
paths are now wired through both frontend contracts.

### Arc 2 — Editing table stakes — COMPLETE

Query-replace (#97), the real kill ring and `M-y` (#103/#105/#106),
comment toggle (#107), auto-indent (#109), and auto-pairing (#110)
landed.

### Arc 3 — Persistence/serialization — COMPLETE

Saveplace/recentf (#98), desktop save/restore (#99),
autosave/crash-recovery (#100), and the save-clobber fix (#101) landed.

### Arc 4 — Themes + extensibility surface — COMPLETE

Stages 1–3 landed as #120, #124, and #125: named `ui.*` faces with
daemon-resolved `ThemeFacts`; the live global `pmacs.gpu.set_font`
preference at protocol v17; and composable per-window
`pmacs.statusline` providers transported to semantic/GPU frontends by
protocol-v18 `StatuslineSegments`.

### Arc 5 — Terminal, staged — VTERM STAGE 3 FRAMED

- **Compile mode landed in #113**: line-oriented PTY/ANSI output,
  error-regex navigation, and `M-x compile`.
- **Vterm Stage 1 terminal core landed in #126**: compatibility parser
  profiles, bounded VT screen/scrollback/reflow state, IND/NEL/RI, input
  encoders, internal `TerminalManager`, read-only identity buffers, process
  lifecycle, control-free renderer-boundary cells, and headless real-PTY
  acceptance.
- **Vterm Stage 2 TUI landed in #130**: terminal-window composition,
  input/resize, per-context scroll/selection/copy, authenticated frontend
  ownership, BEL/clipboard drainage, and the strict Lua surface.
- **Vterm Stage 3 protocol/GPU is framed at Revision 8**: additive protocol
  v19 complete frames/events, an aggregate glyph-byte bound under the
  unchanged transport cap, dual viewport bootstrap, authenticated semantic
  routing, and native fixed-cell GPU terminal rendering. Implementation waits
  for explicit framing approval.

### Arc 6 — Folding (keystone gutter rider)

Fold engine (tree-sitter fold ranges) unblocks gutter fold markers +
the unproduced `FoldState` wire family + a visible feature. Git gutter
markers similarly just need a diff source.

### Arc 7 — Debugging (DAP)

Greenfield but newly unblocked: gutter (breakpoint signs), process
substrate (DAP = JSON-RPC over stdio, same shape as the LSP client),
list-buffer panels from Arc 1b (stack/variables). Frame after Arc 1
ships — panels are reused here.

### Arc 8 — GPU structural parity

Splits / multi-buffer (largest unscoped design problem), auto-reconnect
after daemon restart, cursor blink, font/theme config (overlaps Arc 4).

---

## Housekeeping (do opportunistically)

- Merge PR #91 (gutter click classification + fit guard).
- Delete or mark-superseded stale docs: `pmacs-gpu-editing-perf-handoff.md`
  (freeze fixed by PR #60 arc), `session-5-stale-styling-handover.md`
  (fixed, task #25 closed), `python_experiment.md`, `#run.sh#`,
  `semantic-frontend-protocol.md.local-bak`.
- `V0.2-PREREQUISITES.md` cited twice by CHANGELOG but missing on disk —
  reconstruct or unlink.
- README still leads with "What v0.1 ships with" — refresh.
- F-016 lua_bindings split: ~5–8 tranches left; `editor.rs` (7k lines)
  and `pmacs-gpu/main.rs` (7.6k lines) splits are named follow-up arcs.
- Deferred backlogs live in each framing doc's Deferred section; the
  consolidated sweep is reflected above.
