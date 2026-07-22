# Side-quest backlog — the non-themes deferral inventory

**Compiled 2026-07-14** from an exhaustive sweep of every framing doc's
"Deferred (named)" section, the handoff §6 consolidated list, the
`docs/roadmap-2026-07.md` arcs, and a code-level marker sweep
(src/, builtin/, pmacs-gpu/, pmacs-protocol/). **Updated 2026-07-15:**
multi-language injections shipped (#122) — item pruned to its remaining
follow-ups; north star and Jupyter gate revised.

**Scope.** This is the *side-quest* backlog: self-contained,
frontend-agnostic-ish work that does **not** belong to the **themes /
faces main quest** (Arc 4, `docs/theme-faces-framing.md`). Anything about
colors, faces, syntax-highlight *styling*, or semantic-token→color
mapping is excluded here and lives with the theme workstream — those are
listed once, at the bottom, so the reader knows they were seen.

Framing: **side quests** are one-shot-ish and mostly enabling; **raid
bosses** are big multi-PR arcs. Each framing doc's own Deferred section
remains the authoritative per-feature source; this file is the
cross-cutting index. Keep it pruned as items ship.

---

## Side quests — highlighting & language detection

The direct continuation of the #114–#118 grammar/detection stack.

- **Locals-query processing — SHIPPED (#134).** Bundled Lua/JavaScript/
  TypeScript locals queries now drive settled per-layer lexical facts, and
  both highlight producers honor `#is?`/`#is-not? local`. Non-shadowed
  builtins regain `.builtin` styling while shadowed names stay ordinary.
- **Multi-language injections — SHIPPED (#122).** The load-bearing
  engine landed: `ParseTreeBundle` holds `Vec<Layer>`, the worker builds
  child trees off the static grammar table, settle resolves per-layer
  highlight queries; markdown fenced code + `markdown_inline` are the
  first consumers. Remaining follow-ups: `injection.combined` (many
  matches → one shared parse; PHP-in-HTML, some comment schemes),
  child-tree incrementality + range-scoped layer rebuild (children
  cold-reparse on every settle today), injectable runtime/Lua-registered
  languages (v1 resolves only against `BUILTIN_LANGUAGES`), and new
  injection *consumers* gated on grammars — HTML/CSS/GraphQL/SQL
  (`<script>`/`<style>`, template literals, doc-comment code).
- ~~**Modeline detection**~~ — **SHIPPED as #132.** Bounded Emacs/Vim
  metadata now precedes extension → filetype → filename → shebang inference.
- **JSON + YAML — PR #123 open.** Grammars and LSP configs exist on the
  feature line; review fixes are preserved on
  `json-yaml-handoff-2026-07-20`. A real YAML-through-pmacs smoke,
  rebase, and full gates remain before review resumes. JSON is also the
  prerequisite for the notebook path; see `docs/active-work.md`.
- **More grammars for languages with neither grammar nor LSP** — ruby,
  php, html, css, sql, etc.
- **Grapheme / combining-mark awareness** in the text view
  (`text_view.rs` skips zero-width marks).
- **Byte-accurate multibyte cursor placement** — `move_active_cursor_to`
  (and go-to-definition) step one codepoint per LSP byte-column,
  landing short on multibyte lines.
- **Daemon query-by-range / incremental highlight** — `scoped_style_spans`
  runs the query over the whole tree even for a scoped viewport (perf
  ceiling on very large files).

## Side quests — LSP feature surface

- **Multi-root scoping** — one server *per project root* instead of the
  current first-file-wins single-root-per-language.
- **Hidden-buffer LSP attach** — a restored/registry-only buffer attaches
  LSP only on first visit (no `after-load` fires).
- **Inlay-hint inline rendering** — hint data exists but nothing renders;
  needs the virtual-text renderer (the cell-overlay model can't reflow
  glyphs around inserted columns yet). Plus hint-part interactivity.
- **Completion depth** — `completionItem/resolve` + `additionalTextEdits`
  (auto-import), snippet tabstops/placeholders, a doc panel beside the
  popup, fuzzy matching, persisted frequency ranking.
- **Panels** — reference-row line snippets, in-panel fuzzy filtering,
  peek/preview on n/p, call-hierarchy & workspace-symbols panels,
  migrate `*buffer-list*` onto `listview`.
- **Quick-fix titles streamed into the context menu** (vs one generic
  "Quick Fix" item).
- **Attach transports** — TLS / Custom (SSH landed; these still
  parse-and-defer).

## Side quests — editing (Emacs table-stakes not yet built)

- **Kill-ring / motion:** word kills (`M-d`/`M-BS`/`C-BS`/`C-h`/`C-DEL`,
  need bytes-returning deleters), `C-SPC` set-mark, `C-u C-y`, `C-M-w`
  append-next-kill, kill-ring browser, ring persistence, clipboard
  watching.
- **Comments:** block comments + mid-line spans, `comment-dwim`
  append-at-EOL, doc-comment continuation on newline, per-language
  padding.
- **Auto-indent:** language-aware/electric indent (`indents.scm`),
  TAB-as-reindent, `C-o` open-line, `C-j`, strip abandoned-line trailing
  whitespace on split.
- **Auto-pairing:** wrap-region on opener, pair-aware backspace,
  RET-inside-pair closer-on-own-line, in-string/in-comment inhibit
  (needs a node-at-byte `pmacs.parse` binding), undo amalgamation,
  balance-aware quotes, per-buffer toggle.
- **Editops:** recenter (`C-l`, blocked on viewport facts), Unicode
  case/word classes, region-spanning move/duplicate, locale/numeric
  sort-lines, ensure-final-newline on save, punctuation-aware
  `fixup-whitespace`, chords for the M-x-only commands.
- **Query-replace:** capture-group refs (`\1`), `,`/`^`/`?` keys,
  backward & whole-buffer, single undo-group per run, smart default
  from-string, lazy-highlight of remaining matches, non-interactive
  `replace-string`/`replace-regexp`.

## Side quests — search

- **Regex + Unicode case-folding** in search (`search.rs` is ASCII-fold,
  literal-only today).
- **occur-mode**; **lazy-highlight** of all matches; **search-match
  background rendering** on GPU (producer + `SearchMatch` quads).

---

## Cross-cutting substrate (unblocks whole clusters — high leverage)

- ~~**Config/settings registry**~~ — **SHIPPED as #127.** `pmacs.config`
  exists with global + buffer-local scopes. Unblocked: the per-buffer
  auto-pair toggle (shipped in the same PR as `editing.auto-pair`),
  language-aware indent, per-language comment padding, and per-project
  compile commands — the last three are now ordinary work, expressed as
  a `buffer.after-load` hook calling `set_local`, not blocked work.
- ~~**Tab-width rendering parity**~~ — **IMPLEMENTED, IN REVIEW.** One fixed
  8-column constant now drives the core/TUI display-column paths, GPU rich-text
  projection, and minimap widths. GPU expansion retains source-tab provenance,
  so caret, hit, selection, and diagnostic geometry remain byte-correct through
  adornments and soft wraps. Source text and protocol ranges remain raw; this
  adds no config key or wire change. See `docs/tab-width-parity-framing.md`.
- **Real `read_only` buffer flag** on both edit paths — true immutability
  for panels / REPL / generated buffers.
- ~~**Mode system wiring**~~ — **SHIPPED as #129.** Per-buffer major modes
  now drive key dispatch, effective-key introspection, and statusline display.
- **Buffer-aware edit epoch + origin-pinned `after-edit` fan-out** — a
  command that edits buffer A then switches to B currently evades
  `didChange` / reparse / autosave observers.
- **Undo-group boundaries (`begin/end_undo_group`) + cross-peer
  chronological undo arbiter** — coherent multi-edit undo and mixed
  source/daemon history.
- **Viewport facts on the wire** — the GPU never consumes daemon
  `view_top`; blocks recenter and any scroll command.

---

## Raid bosses (big multi-PR arcs)

- **Arc 5 stage 2 — vterm:** extend `ansi.rs` into a 2D grid model
  (alt-screen, cursor addressing, scrollback), grid-backed buffer view,
  GPU grid rendering.
- **Arc 6 — Folding:** fold engine (tree-sitter fold ranges) +
  `FoldState` wire family + gutter fold markers.
- **Arc 7 — Debugging (DAP):** DAP client (JSON-RPC/stdio, same shape as
  the LSP client), breakpoint gutter signs, stack/variables panels
  (reuse the Arc 1b listview).
- **Arc 8 — GPU splits / multi-buffer** (largest unscoped design
  problem).
- **Jupyter `.ipynb`** — reader → editable → kernel execution; now gated
  on JSON only (injections shipped in #122).
- **Git-diff gutter markers** (needs a diff source; rides the gutter).

---

## Compile-mode / process follow-ups

PTY-mode variant, auto-scroll option, severity threshold for error-only
stepping, per-language/project default commands, echo-area short output
+ distinct `M-&`, error parsing in `*shell-command*`, split-window
display, persist last compile command, next-error across historical
result buffers, occur-mode, configurable `GROUP_TERM_GRACE`, unify the
poll-based cancellable readers across REPL/LSP, fully tick-driven final
drain (kill the ~600 ms teardown stall).

## Persistence & files

Desktop daemon/GPU-attach restore, save unsaved *content* in the desktop,
multiple named desktops, window-local minor state, remote/cross-machine
desktops, non-file buffers in the desktop, saveplace for non-file
buffers, recentf as a `listview` panel, per-project `init.lua` desktop
scoping, newline-in-path handling. **Autosave:** idle-gated timing,
non-file buffers, orphan-recovery GC + browser, offload large writes to a
worker, **external-change guard on `save()` itself** (a real adjacent
bug), a `y_or_n` confirm helper, harden the whole `$XDG_STATE_HOME/pmacs`
root to `0700`, fire `after-load` for `[new file]` buffers. **Files:**
byte-preserving (round-trip-exact) save paths; the *warning* half of
external-change detection (verify-modtime-while-open, revert?).

## GPU frontend mechanics (non-theme)

- **Input:** full command/minibuffer chord forwarding to the GUI,
  Meta/Super chords, rebindable local `Ctrl-V`/`Escape`, middle-click
  paste, right-click context menu, frontend-local provisional selection.
- **Minibuffer:** `i/total` hint (already on the wire), Telescope-style
  preview pane, candidate kind/doc annotations, unify TUI inline vs GPU
  dropdown, multibyte-exact band caret, the nav highlight-wrap bug.
- **Scrolling:** scrollbar scroll, pixel-smooth sub-line scroll,
  horizontal scroll / soft-wrap.
- **Rendering:** peer caret glyph + name label, own-vs-peer cursor merge,
  `SelectionSnapshot` vs `Decorations::Selection` reconciliation,
  background-kind decorations actually painted, inline adornment
  placements beyond `AtOffset`.
- **Robustness:** auto-reconnect + "reconnecting…" banner, capability
  renegotiation (offer to relaunch the daemon `--features crdt`), the
  `AttachRequest.initial_size` cell-grid assumption.
- **Perf / harness:** glyphon full-buffer `prepare` ceiling, golden-PNG
  comparison harness, `Renderer` sub-struct extraction.

## Packages / protocol / CLI

Forge aliases (`codeberg:` / `forgejo:`, post-v1.0), namespace-preserving
package layout + install-time cross-resolve collision detection,
project-local `init.lua` wiring + after-init calls, multiple files on the
CLI, `describe-job` full-message surfacing.

## Housekeeping / refactor

`lua_bindings/mod.rs` split (~4 tranches / ~14 k lines left; the `theme`
domain excluded — see below), `editor.rs` split (7 k lines),
`pmacs-gpu/src/main.rs` split (6.7 k lines), narrow re-exported
`install_*` from `pub` → `pub(crate)`.

---

## Excluded — themes / faces main quest (seen, routed elsewhere)

The statusline-segment API (Arc 4 stage 3; framing awaiting review);
background/selection theming; per-peer stable presence colors; exact quad colors per
decoration kind; GPU gutter background layer / wash recolor / chrome
bold-italic-underline; current-line highlight refinements; multi-server
semantic-token *style* blending; the compile-mode **severity→color**
classifier (the classification is behavioral, but the coloring is theme
work); the `theme` domain + style/color converters in the lua-bindings
split. Borderline, kept above as mechanics: whitespace glyphs and indent
guides (visual, not color).

---

## North star (highest-leverage first)

**The original north-star items have shipped or reached review** —
multi-language injections (#122), the config registry (#127), JSON + YAML
(#123), mode-system wiring (#129), locals queries (#134), and tab-width
rendering parity (`tab-width-parity`, review pending). The remaining board now
starts with the broader ranked arcs below rather than another unresolved
cross-frontend rendering invariant.

Beyond those, the cleanest remaining one-shots in the highlight family are the
HTML/CSS grammars that light up more injection *consumers*; modeline detection
shipped in #132. The most-missed editing table-stakes remain **word-kills +
`C-SPC` set-mark**.
