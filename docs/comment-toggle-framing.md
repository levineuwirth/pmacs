# Comment/uncomment — framing (Arc 2, editing table stakes)

pmacs has no way to comment code out. This adds the table-stakes
toggle: `M-;` comments or uncomments the current line or the selected
lines, language-aware, as one undoable edit.

Roadmap: `docs/roadmap-2026-07.md` Arc 2 ("comment/uncomment").

## Ground truth (as of `2dde4b8`)

- **No comment-syntax knowledge exists anywhere** — not in the grammar
  registry, not in LSP config, not in Lua. A language → prefix table is
  new surface.
- **Language detection**: `active_buffer_language()` in
  `builtin/runtime/lsp.lua` chains grammar detection
  (`pmacs.parse.language_for_path`) with the user-extensible
  `pmacs.lsp.filetypes` map — but it is a **local**. Known languages
  today: rust/lua/c/cpp + js/ts via grammars; python, c, go,
  tsx/jsx, lua, bash, toml, zig via filetypes.
- **Bindings**: `M-;` (Emacs `comment-dwim`'s home) is free. `C-/` is
  taken by undo (terminal `Ctrl+/` ambiguity — three undo bindings
  exist), so the VSCode-style toggle chord is unavailable.
- **Undo granularity**: every applied edit pushes one `UndoEntry`
  (`src/buffer.rs:130`) — there is no grouping/transaction. N per-line
  edits would need N undos.
- From Arc 2 (merged): the buffer mutators return the **effective
  post-intercept edit** `(start, end, inserted_len)`; keybound commands
  rotate the command boundary and get `buffer.after-edit` from
  dispatch; `M-x` gets both via `invoke_interactive` +
  `with_after_edit_check`. Comment-toggle inherits all of it for free.

## Decisions

### Q#CT1 — Pure Lua, zero Rust changes

`builtin/runtime/comment.lua`. Everything needed exists: `buf:slice`,
the effective-edit-returning `buf:replace`, `ed.region/cursor/
goto_byte/clear_selection`, and the language chain. The one lsp.lua
touch: **export** the existing local as
`pmacs.lsp.active_buffer_language()` (one line) rather than replicating
its grammar+filetypes chain and drifting.

### Q#CT2 — Command + binding

One command, `edit.toggle-comment`, bound **`M-;`**:

- **Region active** → toggle the whole lines the region touches, then
  clear the selection (CUA convention after a region op) and leave the
  cursor at the span start.
- **No region** → toggle the current line, then **move to the next
  line** — Emacs's `comment-line` behavior, which makes repeated `M-;`
  walk down a block toggling as it goes.

Named deviation from Emacs: `M-;` in Emacs is `comment-dwim`, whose
no-region case *appends* an empty comment at end of line. That mode is
rarely what modern muscle memory wants from the toggle key; pmacs's
`M-;` behaves like Emacs `C-x C-;` (`comment-line`). DWIM's
append-comment can come later under its own name.

### Q#CT3 — The comment-string table

`pmacs.comment.strings` — a public, user-extensible map (the
`pmacs.lsp.filetypes` pattern), language → line-comment prefix:

`//`: rust, c, cpp, go, zig, javascript, typescript,
javascriptreact, typescriptreact · `--`: lua · `#`: python, bash,
toml, yaml, sh.

Unknown language (or no language): status *"no comment syntax known
for `<lang>`"*, no edit. Users add entries from init.lua:
`pmacs.comment.strings.mylang = ";;"`. **Block comments are deferred**
— line comments cover the table-stakes use, and block toggling has
real edge cases (nesting, mid-line spans) that don't belong in v1.

### Q#CT4 — Toggle semantics

Over the span's lines:

- **Uncomment** when every non-blank line starts (after its
  indentation) with the prefix; removal strips the prefix plus one
  following space if present.
- **Comment** otherwise: insert `prefix + " "` at the **minimum
  indentation column** of the span's non-blank lines (Emacs
  `comment-region` style — the comments line up instead of hugging
  each line's own indent). **Blank lines are skipped** in both
  directions and don't influence the min-indent computation.
- A span that is entirely blank is a no-op with a status.

Mixed spans (some commented, some not) therefore **comment** — the
double-prefix on already-commented lines round-trips back out, which
is Emacs's behavior and preserves inner commented-out code.

### Q#CT5 — One edit, one undo step, one CRDT op

The whole toggle is a **single `buf:replace(span_start, span_end,
new_text)`**: Lua builds the rewritten span, one edit applies it.
Consequences, all deliberate:

- **One `C-/` undoes the whole toggle** (there is no undo grouping to
  lean on; N per-line edits would need N undos).
- One CRDT op for replica frontends.
- One effective-edit verification: the kill-ring discipline — if the
  returned `(start, end, inserted_len)` deviates from the request (a
  buffer intercept rewrote it), report *"comment toggle altered by
  buffer intercept"* and skip the cursor fix-up; the interceptor's
  result stands. `pcall`'d, so a rejecting intercept reports rather
  than throws.

### Q#CT6 — Chain/hook plumbing: nothing to build

Keybound `M-;` rotates the command boundary (breaking kill chains —
correct) and fires `buffer.after-edit` from dispatch's revision check;
`M-x edit.toggle-comment` gets the same via `invoke_interactive` and
the accept-path hook wrapper. Both are the Arc 2 substrate working as
designed — the acceptance suite asserts the hook fires once anyway.

## Bets

1. **Single-replace is the right granularity** — no complaint about
   whole-span replaces (vs. per-line edits) from CRDT replicas or LSP
   didChange (full-text sync makes this moot today).
2. **Min-indent + skip-blank matches expectation** — no "why is my
   comment at column 0" or "why did my blank line get a `//`".

## Deferred (named)

- Block comments (`/* */`) and mid-line spans.
- `comment-dwim`'s append-comment-at-EOL mode.
- Doc-comment continuation on newline (belongs to auto-indent).
- Per-language *padding* config (always one space in v1).

## Acceptance (`tests/comment_toggle_acceptance.rs`, dispatch-driven)

- Rust buffer: `M-;` comments the line (`// ` at indent), cursor moves
  to the next line; `M-;` on a commented line uncomments (round-trip,
  including the space).
- Region across mixed-indent lines → prefixes at min indent, aligned;
  blank line inside the span untouched; selection cleared.
- All-commented span → uncomments; mixed span → comments (inner
  prefix preserved, round-trips).
- Lua buffer gets `--`, Python `#`; unknown/no language → status, no
  edit.
- **One undo step**: multi-line toggle then a single `buffer.undo`
  restores the original text exactly.
- Intercept discipline: rejecting intercept → status, no throw;
  transforming intercept → reported, no cursor fix-up.
- `after-edit` probe fires exactly once per toggle (keybound and
  `M-x`).
- Kill-chain break: `C-k`, `M-;`, `C-k` → two ring entries.
