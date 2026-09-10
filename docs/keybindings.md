# pmacs keybindings — reference

pmacs keys come from two independent places:

- **The Lua keymap** (§1–2) — `pmacs.keymap.bind{...}` calls, resolved
  by the Rust dispatcher against whatever `init.lua` has bound at
  runtime. Fully user-rebindable: unbind or rebind any of these from
  init.lua (§5). `pmacs.keymap.bind` REFUSES a sequence that is already
  bound, so taking over a default chord means `pmacs.keymap.unbind`
  first.
- **Rust-hardcoded modal shadows** (§3) — isearch, query-replace,
  the minibuffer/prompt, the completion popup, and the context menu
  each shadow the Lua keymap while active: `EditorInstance::dispatch_key`
  checks these modes, highest-priority first, before a key ever reaches
  the Lua dispatcher. **Not user-configurable** — there is no
  `pmacs.keymap` surface for them; changing one means editing the mode's
  `from_chord` decoder in Rust.

Notation matches what `pmacs.keymap.bind` accepts: `C-` = Ctrl, `M-` =
Alt/Meta, `S-` = Shift, bare letters/punctuation self-insert when
unmodified. Named keys are angle-bracketed (`<left>`, `<up>`, `<home>`)
or all-caps (`RET`, `BS`/Backspace, `DEL`/Delete, `TAB`, `SPC`).
Sequences separated by spaces (`C-x C-s`) are chords typed in order.

## 1. Lua keymap

**Generated.** Everything between the markers below is rendered from
`KeymapStack::iter_all` — the same source `pmacs.keymap.list()` and
`M-x help.list-keybindings` read — and pinned byte for byte by
`tests/keybindings_doc_acceptance.rs`. Do not edit it by hand; after
changing a binding, regenerate:

```text
PMACS_WRITE_KEYBINDINGS=1 cargo test --test keybindings_doc_acceptance
```

This is what the file's own §2 note used to admit it could not manage:
the table had missed a `TAB` binding and nothing could tell. The
buffer-local panel keymaps below are NOT in here, because they exist
only while a panel does; §3's modal keys are not either, because they
are not in the keymap at all.

<!-- keymap:begin -->

### Scope: global

| Key | Command |
|---|---|
| `<down>` | `cursor.down` |
| `<end>` | `cursor.line-end` |
| `<f1>` | `help` |
| `<home>` | `cursor.line-start` |
| `<left>` | `cursor.left` |
| `<pagedown>` | `cursor.page-down` |
| `<pageup>` | `cursor.page-up` |
| `<right>` | `cursor.right` |
| `<up>` | `cursor.up` |
| `BS` | `buffer.delete-backward` |
| `C-+` | `gpu.zoom-in` |
| `C--` | `gpu.zoom-out` |
| `C-/` | `buffer.undo` |
| `C-0` | `gpu.zoom-reset` |
| `C-4` | `buffer.undo` |
| `C-<down>` | `cursor.paragraph-down` |
| `C-<left>` | `cursor.word-left` |
| `C-<right>` | `cursor.word-right` |
| `C-<up>` | `cursor.paragraph-up` |
| `C-=` | `gpu.zoom-in` |
| `C-?` | `buffer.redo` |
| `C-BS` | `buffer.delete-word-backward` |
| `C-DEL` | `buffer.delete-word-forward` |
| `C-M-%` | `query-replace-regexp` |
| `C-M-i` | `completion.at-point` |
| `C-M-r` | `search.backward-regex` |
| `C-M-s` | `search.forward-regex` |
| `C-S-<down>` | `cursor.select-paragraph-down` |
| `C-S-<left>` | `cursor.select-word-left` |
| `C-S-<right>` | `cursor.select-word-right` |
| `C-S-<up>` | `cursor.select-paragraph-up` |
| `C-S-_` | `buffer.redo` |
| `C-SPC` | `region.set-mark` |
| `C-_` | `buffer.undo` |
| `C-a` | `cursor.line-start` |
| `C-b` | `cursor.left` |
| `C-c @ C-M-h` | `fold.close-all` |
| `C-c @ C-M-s` | `fold.open-all` |
| `C-c @ C-c` | `fold.toggle` |
| `C-c @ C-h` | `fold.close` |
| `C-c @ C-s` | `fold.open` |
| `C-c H` | `lsp.hover-doc` |
| `C-c a` | `lsp.code-actions` |
| `C-c c` | `compile.run` |
| `C-c f` | `lsp.format-buffer` |
| `C-c h` | `lsp.hover` |
| `C-c i` | `lsp.inlay-hints` |
| `C-c l` | `lsp.status` |
| `C-c o` | `lsp.document-symbols` |
| `C-c r` | `lsp.rename` |
| `C-c s` | `lsp.signature-help` |
| `C-c t` | `terminal` |
| `C-c y` | `lsp.semantic-tokens` |
| `C-d` | `buffer.delete-forward` |
| `C-e` | `cursor.line-end` |
| `C-f` | `cursor.right` |
| `C-g` | `editor.cancel` |
| `C-h` | `buffer.delete-word-backward` |
| `C-k` | `edit.kill-line` |
| `C-l` | `window.recenter` |
| `C-n` | `cursor.down` |
| `C-p` | `cursor.up` |
| `C-r` | `search.backward` |
| `C-s` | `search.forward` |
| `C-t` | `edit.transpose-chars` |
| `C-v` | `cursor.page-down` |
| `C-w` | `edit.cut` |
| `C-x 0` | `window.close` |
| `C-x 1` | `window.close-others` |
| `C-x 2` | `window.split-horizontal` |
| `C-x 3` | `window.split-vertical` |
| `C-x <left>` | `editor.previous-buffer` |
| `C-x <right>` | `editor.next-buffer` |
| `C-x C-^` | `window.shrink` |
| `C-x C-b` | `editor.list-buffers` |
| `C-x C-c` | `editor.quit` |
| `C-x C-f` | `find-file` |
| `C-x C-j` | `dired-jump` |
| `C-x C-r` | `recent-files` |
| `C-x C-s` | `buffer.save` |
| `C-x C-w` | `buffer.write-file` |
| `C-x C-x` | `region.exchange-point-and-mark` |
| `C-x O` | `window.focus-prev` |
| `C-x ^` | `window.enlarge` |
| `C-x `` | `error.next` |
| `C-x b` | `editor.switch-buffer` |
| `C-x d` | `dired` |
| `C-x g` | `git.status` |
| `C-x h` | `edit.select-all` |
| `C-x k` | `buffer.kill` |
| `C-x l` | `window.toggle-line-numbers` |
| `C-x o` | `window.focus-next` |
| `C-x p f` | `project.find-file` |
| `C-x r` | `buffer.redo` |
| `C-x t` | `ui.toggle-line-wrap` |
| `C-x u` | `buffer.undo` |
| `C-x w` | `editor.list-workers` |
| `C-y` | `edit.paste` |
| `DEL` | `buffer.delete-forward` |
| `M-!` | `shell.command` |
| `M-%` | `query-replace` |
| `M-,` | `lsp.jump-back` |
| `M-.` | `lsp.go-to-definition` |
| `M-;` | `edit.toggle-comment` |
| `M-<` | `cursor.buffer-start` |
| `M-<down>` | `edit.move-line-down` |
| `M-<up>` | `edit.move-line-up` |
| `M->` | `cursor.buffer-end` |
| `M-?` | `lsp.find-references` |
| `M-BS` | `edit.kill-word-backward` |
| `M-^` | `edit.join-line` |
| `M-b` | `cursor.word-left` |
| `M-c` | `edit.capitalize` |
| `M-d` | `edit.kill-word-forward` |
| `M-f` | `cursor.word-right` |
| `M-g M-g` | `cursor.goto-line` |
| `M-g g` | `cursor.goto-line` |
| `M-g n` | `error.next` |
| `M-g p` | `error.previous` |
| `M-l` | `edit.downcase` |
| `M-t` | `edit.transpose-words` |
| `M-u` | `edit.upcase` |
| `M-v` | `cursor.page-up` |
| `M-w` | `edit.copy` |
| `M-x` | `editor.execute-command` |
| `M-y` | `edit.yank-pop` |
| `M-z` | `edit.zap-to-char` |
| `M-{` | `cursor.paragraph-up` |
| `M-}` | `cursor.paragraph-down` |
| `RET` | `edit.newline-and-indent` |
| `S-<down>` | `cursor.select-down` |
| `S-<end>` | `cursor.select-line-end` |
| `S-<home>` | `cursor.select-line-start` |
| `S-<left>` | `cursor.select-left` |
| `S-<right>` | `cursor.select-right` |
| `S-<up>` | `cursor.select-up` |
| `TAB` | `buffer.tab` |

### Scope: mode:dired

| Key | Command |
|---|---|
| `<down>` | `cursor.down` |
| `<up>` | `cursor.up` |
| `RET` | `dired.visit` |
| `^` | `dired.parent` |
| `f` | `dired.visit` |
| `g` | `dired.revert` |
| `n` | `cursor.down` |
| `p` | `cursor.up` |
| `q` | `dired.quit` |
| `s` | `dired.sort-cycle` |

<!-- keymap:end -->

## 2. Buffer-local panel keymaps

Read-only panel buffers built on `pmacs.listview.open` (buffer scope
`{ scope = "buffer", buffer = <id> }`; see `builtin/runtime/listview.lua`)
all share one keymap:

| Key | Action |
|---|---|
| `RET` / `SPC` | `listview.visit` — act on the item under the cursor |
| `n` / `<down>` | `cursor.down` |
| `p` / `<up>` | `cursor.up` |
| `TAB` | `listview.toggle` — collapse/expand the tree node under the cursor; a panel with no tree rows delegates to `buffer.tab` |
| `g` | `listview.refresh` — re-run the data source and re-render |
| `q` | `listview.quit` — restore the buffer that was active before the panel opened |

(`TAB` arrived with the tree primitive and this table had not recorded
it. Noted rather than quietly added: the omission predates the git lane
that found it.)

Panels currently built on this: `*references*`, `*outline*`,
`*lsp-help*` (hover docs), `*lsp*` (`lsp.status`), and `*git-status*`
(`git.status`). Header text always spells out the panel's own legend
inline.

A panel may add keys of its own through an optional `keys` table on the
open spec, bound through the same buffer-local path — so they are
inspectable by `describe-key` and rebindable from `init.lua`, exactly
like the fixed set. They are installed once with the panel's buffer and
may not collide with the fixed set, nor prefix it. One panel uses this
today:

| Buffer | Key | Command |
|---|---|---|
| `*git-status*` (`git.status`) | `d` | `git.diff-file` — the diff for the file under the cursor, into `*git-diff*` |

`git.status` gets **no global chord**: an opening key is a
command-surface decision the Stage 1 framing did not make, so the entry
point is `M-x git.status`.

`*buffer-list*` (`editor.list-buffers`, `C-x C-b`) uses its own
keymap, layered on the same idiom, in `builtin/commands/default.lua`:

| Key | Command |
|---|---|
| `RET` / `SPC` | `editor.buffer-list-visit` |
| `n` / `<down>` | `cursor.down` |
| `p` / `<up>` | `cursor.up` |
| `d` | `editor.buffer-list-mark-delete` |
| `u` | `editor.buffer-list-unmark` |
| `x` | `editor.buffer-list-execute` — kill every marked buffer |
| `k` | `editor.buffer-list-kill-now` |
| `g` | `editor.buffer-list-refresh` |
| `q` | `editor.buffer-list-quit` |

One-off buffer-local bindings, each scoped to a single generated
buffer:

| Buffer | Key | Command |
|---|---|---|
| `*workers*` (`editor.list-workers`) | `C-c C-k` | `workers.cancel-at-point` (`builtin/runtime/async.lua`) |
| `*pmacs-instance*` (`editor.describe-instance-buffer`) | `q` | `buffer.kill-this` (`commands/default.lua`) |
| `*help*` (`editor.describe-command`) | `q` | `buffer.kill-this` |
| REPL buffers (`builtin/packages/repl/init.lua`) | `RET` | `pmacs.repl.submit-current` |
| REPL buffers | `C-c` | `pmacs.repl.send-sigint-current` |
| REPL buffers | `C-d` | `pmacs.repl.send-eof-current` — closes stdin on an empty line, else deletes forward |

Compile-mode generated buffers (`*compilation*` and
`*shell-command*`) have their own buffer-local map:

| Key | Command |
|---|---|
| `RET` | `compile.visit-error` |
| `n` / `p` | `compile.next-error-line` / `compile.previous-error-line` |
| `q` | `compile.quit` |
| `C-c C-k` | `compile.kill` |
| `g` | `compile.recompile` (`*compilation*` only) |
| every shipped undo/redo chord | `compile.undo-noop` — generated output is intercept-read-only |

The REPL package (`builtin/packages/repl/`) is shipped but opt-in —
loaded via `require`, not part of the always-on `builtin/runtime`
lane. Its bindings only exist in a buffer created by a REPL session.

## 3. Rust-hardcoded modal keys

These live in `src/editor.rs` (and `src/minibuffer.rs` for the
prompt) as small `from_chord(chord) -> Action` decoders, one per mode,
checked in priority order by `EditorInstance::dispatch_key`
(`src/editor.rs:658-733`, highest first): **context menu → isearch →
query-replace → minibuffer → completion popup → normal Lua dispatch.**
Each decoder's rustdoc names its own key list; this table mirrors
those. They are not reachable through `pmacs.keymap` — there is
deliberately no Lua surface for them (keeps the set curated; see the
`R51` rationale cited in `lib.rs`/`lua_bindings/mod.rs`).

**Isearch** (`SearchKey`, `editor.rs:1858-1919`) — active after
`C-s`/`C-r`/`C-M-s`/`C-M-r`:

| Key | Action |
|---|---|
| `C-s` / `<down>` | next match |
| `C-r` / `<up>` | previous match |
| `RET` / `C-m` | accept — keep cursor + highlights |
| `C-g` / `Esc` | cancel — restore the origin cursor |
| `BS` / `C-h` | shorten the query by one character |
| `M-r` | toggle literal ↔ regex |
| any printable char | extend the query |

**Query-replace** (`QueryReplaceKey`, `editor.rs:1926-1961`) — active
after `M-%`/`C-M-%`:

| Key | Action |
|---|---|
| `y` / `SPC` | replace this match, advance |
| `n` / `BS` / `Delete` | skip this match, advance |
| `!` | replace this and every remaining match, no more prompts |
| `.` | replace this match, then quit |
| `q` / `RET` / `Esc` / `C-g` | quit (replacements already made are kept) |

**Minibuffer / prompt** (`MinibufferAction`, `minibuffer.rs:418-527`)
— backs every `pmacs.minibuffer.read` call: `M-x`, query-replace's
from/to prompts, `find-file`, etc.:

| Key | Action |
|---|---|
| `RET` / `C-m` | accept |
| `C-g` | cancel |
| `TAB` / `C-i` | complete to the selected candidate |
| `<up>` / `<down>` | prev/next candidate if a dropdown is showing, else history navigation |
| `C-p` / `C-n` | history prev/next, unconditionally |
| `<left>` / `C-b`, `<right>` / `C-f` | cursor move |
| `<home>` / `C-a`, `<end>` / `C-e` | line start/end |
| `BS` | delete backward |
| `Delete` / `C-d` | delete forward |
| `M-n` / `M-p` | scroll the selected candidate forward/back |
| any other printable char | self-insert |

**In-buffer completion popup** (`CompletionPopupKey`,
`editor.rs:2019-2059`) — active after `C-M-i` or an LSP-triggered
popup. Unlike the others this is a **partial** shadow: only the keys
below are intercepted; everything else (typing, motion) falls through
to normal dispatch, so typing keeps self-inserting while the popup is
open.

| Key | Action |
|---|---|
| `<down>` / `C-n` | next candidate |
| `<up>` / `C-p` | previous candidate |
| `TAB` / `RET` | accept the highlighted candidate |
| `Esc` / `C-g` | dismiss |

**Context menu** (`MenuKey`, `editor.rs:1966-2009`) — opened by
right-click, not a keybinding itself, but shadows the keymap while
open:

| Key | Action |
|---|---|
| `<down>` / `C-n` | next item |
| `<up>` / `C-p` | previous item |
| `RET` | invoke the highlighted item |
| `Esc` / `C-g` | cancel |
| any other key | dismiss (click-away semantics) |

**Frontend detach** — `F12` (any modifiers) detaches an attached
frontend from the daemon (`src/attach.rs:997-1006`, checked at
`attach.rs:818`). Not a UI mode inside the editor core, but another
literal-`KeyCode` interception outside the Lua keymap; tentative for
v0.1 per the comment there (chosen because F12 is rarely bound to
anything else).

## 4. Terminal-compatibility caveats

- **`C-h` doubles as `C-BS`.** Most terminals without the kitty
  keyboard protocol can't disambiguate `Ctrl+Backspace` from
  `Ctrl+H` — both legacy paths send byte `0x08`. `C-h` is bound to
  `buffer.delete-word-backward` alongside `C-BS` so the shortcut works
  on legacy terminals. pmacs does not use `C-h` as an Emacs-style help
  prefix; a user who wants that can rebind it.
- **Undo/redo have redundant bindings** (`C-/`, `C-_`, `C-4` for undo;
  `C-?`, `C-S-_` for redo) because terminals encode `Ctrl+/` several
  different ways. Kitty's keyboard protocol routes most cleanly
  through `C-/`; the alternates keep legacy/remote terminals working.
- Kitty-protocol-only chords (e.g. distinguishing `C-i` from `TAB`)
  degrade gracefully where noted above — check the frontend's terminal
  capability negotiation if a chord seems to not fire.

## 5. Changing bindings

`pmacs.keymap.bind` / `pmacs.keymap.unbind` are ordinary Lua API,
callable from `init.lua`:

```lua
pmacs.keymap.bind { scope = "global", sequence = "C-c g", command = "cursor.goto-line" }
pmacs.keymap.unbind { scope = "global", sequence = "M-z" }
```

`scope = "buffer"` additionally takes `buffer = <id>`; buffer-local
bindings are pruned automatically when that buffer is removed. This
covers §1 and §2 only — §3's Rust-hardcoded modal keys have no Lua
surface (see §3's intro).

## 6. Keeping this file honest

§1 is generated and pinned; a binding added without regenerating fails
`tests/keybindings_doc_acceptance.rs` by name. §2 to §5 are hand-written
because nothing derives them: panel keymaps exist only while a panel
does, and §3's decoders are Rust `match` arms with no registry to read.
Update those in the same PR as the change they describe. To re-derive
them from scratch: grep `builtin/` for `pmacs.listview.open`, and
`src/editor.rs` / `src/minibuffer.rs` for `from_chord`.
