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

### Arc 1 — LSP utility surface: "light up the dark matter" ← ACTIVE

Data layer is done; only UI is missing.

- **1a. In-buffer completion popup** (first). Trigger on typing,
  TAB/RET accept, both frontends. GPU needs a wire message — mirror the
  minibuffer-dropdown pattern (protocol v12). Framework + popup view
  already exist.
- **1b. Panels**: hover popup, code-action picker, references list,
  document-symbol outline. Generalize the buffer-list UI pattern
  (`*buffer-list*` buffer-local bindings) into a reusable list-buffer
  idiom.
- **1c. Semantic-token auto-pull fix** (small): pull on attach + on
  edit-flush, like inlay hints already do.
- **1d. Signature-help auto-trigger** on `(`.

### Arc 2 — Editing table stakes (interleave with Arc 1)

Each small, core-only, frontend-agnostic: query-replace (isearch
exists; `search.rs` has no replace API), real kill ring + `M-y`,
comment/uncomment, auto-indent on newline, auto-pairing.

### Arc 3 — Persistence/serialization

Desktop-save (buffer set + layout + cursors → restore), recentf,
saveplace, autosave + crash recovery, optional backups. Generalize the
`$XDG_STATE_HOME/pmacs/` pattern from minibuffer history. Framing
question: what is a "session" in a daemon world; do CRDT snapshots
ride along.

### Arc 4 — Themes + extensibility surface

Extend `pmacs.theme` from syntax captures to named UI faces (modeline,
minibuffer, gutter, selection, status band); wire GPU chrome to it —
Q#UX1 lesson applies: rendering is frontend-local but control is
daemon-owned, so a wire channel (`ThemeFacts`-style) is needed. Add
`pmacs.gpu.set_font` (designed, never built) and a Lua
statusline-segment API.

### Arc 5 — Terminal, staged

- **Stage 1**: compile-mode / grep-mode / shell-command on the existing
  PTY + ANSI + REPL-package substrate (line-oriented output buffer,
  error-regex jump-to-file, `M-x compile`). Cheap, transformative.
- **Stage 2 (vterm)**: extend `ansi.rs` into a 2D grid model
  (alt-screen, cursor addressing, scrollback — parser already
  recognizes and discards these), grid-backed buffer view, GPU
  rendering question (grid cells vs text buffer).

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
