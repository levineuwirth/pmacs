# pmacs keybindings — reference

**Last verified against `main` @ `f8096ff` (2026-07-20).** This is a
snapshot, not generated output — when a PR adds, removes, or rebinds a
key, update this file in the same PR (see §6). If you're an agent and
this file looks stale against the code it cites, trust the code.

pmacs keys come from two independent places:

- **The Lua keymap** (§1–2) — `pmacs.keymap.bind{...}` calls, resolved
  by the Rust dispatcher against whatever `init.lua` has bound at
  runtime. Fully user-rebindable: unbind or rebind any of these from
  init.lua (§5).
- **Rust-hardcoded modal shadows** (§3) — isearch, query-replace,
  the minibuffer/prompt, the completion popup, and the context menu
  each shadow the Lua keymap while active: `EditorInstance::dispatch_key`
  (`src/editor.rs:658-733`) checks these modes, highest-priority first,
  before a key ever reaches the Lua dispatcher. **Not user-configurable**
  — there is no `pmacs.keymap` surface for them; changing one means
  editing the mode's `from_chord` decoder in Rust.

Notation matches what `pmacs.keymap.bind` accepts: `C-` = Ctrl, `M-` =
Alt/Meta, `S-` = Shift, bare letters/punctuation self-insert when
unmodified. Named keys are angle-bracketed (`<left>`, `<up>`, `<home>`)
or all-caps (`RET`, `BS`/Backspace, `DEL`/Delete, `TAB`, `SPC`).
Sequences separated by spaces (`C-x C-s`) are chords typed in order.

## 1. Global keymap

Source: `builtin/keymaps/default.lua` unless noted. All bound at
`scope = "global"`.

### Cursor motion

| Key | Command |
|---|---|
| `C-a` / `<home>` | `cursor.line-start` |
| `C-e` / `<end>` | `cursor.line-end` |
| `C-f` / `<right>` | `cursor.right` |
| `C-b` / `<left>` | `cursor.left` |
| `C-n` / `<down>` | `cursor.down` |
| `C-p` / `<up>` | `cursor.up` |
| `C-<left>` / `M-b` | `cursor.word-left` |
| `C-<right>` / `M-f` | `cursor.word-right` |
| `C-<up>` / `M-{` | `cursor.paragraph-up` |
| `C-<down>` / `M-}` | `cursor.paragraph-down` |
| `<pageup>` / `M-v` | `cursor.page-up` |
| `<pagedown>` / `C-v` | `cursor.page-down` |
| `M-g g` / `M-g M-g` | `cursor.goto-line` (`builtin/runtime/editops.lua`) |

### Selection (CUA shift-select)

Plain motion preserves an existing selection instead of dropping it
(Emacs-flavored default, not strict CUA).

| Key | Command |
|---|---|
| `S-<left>` / `S-<right>` | `cursor.select-left` / `cursor.select-right` |
| `S-<up>` / `S-<down>` | `cursor.select-up` / `cursor.select-down` |
| `S-<home>` / `S-<end>` | `cursor.select-line-start` / `cursor.select-line-end` |
| `C-S-<left>` / `C-S-<right>` | `cursor.select-word-left` / `cursor.select-word-right` |
| `C-S-<up>` / `C-S-<down>` | `cursor.select-paragraph-up` / `cursor.select-paragraph-down` |

### Editing

| Key | Command |
|---|---|
| `BS` | `buffer.delete-backward` |
| `DEL` / `C-d` | `buffer.delete-forward` |
| `RET` | `edit.newline-and-indent` |
| `TAB` | `buffer.tab` |
| `C-BS` / `C-h` | `buffer.delete-word-backward` (see §4 for the `C-h` rationale) |
| `M-BS` | `buffer.delete-word-backward` |
| `C-DEL` | `buffer.delete-word-forward` |
| `M-d` | `buffer.delete-word-forward` |
| `M-u` | `edit.upcase` (`editops.lua`) |
| `M-l` | `edit.downcase` (`editops.lua`) |
| `M-c` | `edit.capitalize` (`editops.lua`) |
| `C-t` | `edit.transpose-chars` (`editops.lua`) |
| `M-t` | `edit.transpose-words` (`editops.lua`) |
| `M-z` | `edit.zap-to-char` (`editops.lua`) |
| `M-<up>` / `M-<down>` | `edit.move-line-up` / `edit.move-line-down` (`editops.lua`) |
| `M-^` | `edit.join-line` (`editops.lua`) |
| `M-;` | `edit.toggle-comment` (`builtin/runtime/comment.lua`) |

> `M-d` / `M-BS` currently plain-delete the word — they are **not**
> kill-ring members yet (a named deferral; see `docs/agent-handoff.md`
> §6, "word kills"). `edit.kill-line` (below) is the only word/line
> kill wired into the ring so far.

### Clipboard & kill ring

| Key | Command |
|---|---|
| `M-w` | `edit.copy` |
| `C-w` | `edit.cut` |
| `C-y` | `edit.paste` |
| `C-x h` | `edit.select-all` (Emacs `mark-whole-buffer`) |
| `C-k` | `edit.kill-line` (`builtin/runtime/killring.lua`) |
| `M-y` | `edit.yank-pop` — replace the just-yanked text with the previous kill, immediately after `C-y` (`killring.lua`) |

### Undo / redo

Multiple bindings exist because terminals disagree on how `Ctrl+/`
encodes; see §4.

| Key | Command |
|---|---|
| `C-/` / `C-_` / `C-4` / `C-x u` | `buffer.undo` |
| `C-?` / `C-S-_` / `C-x r` | `buffer.redo` |

### Search & replace

Once a search is running, `C-s`/`C-r` step to the next/previous match
and `M-r` toggles literal↔regex — those are Rust-hardcoded isearch
keys, not Lua bindings (§3).

| Key | Command |
|---|---|
| `C-s` | `search.forward` (starts isearch) |
| `C-r` | `search.backward` (starts isearch) |
| `C-M-s` | `search.forward-regex` |
| `C-M-r` | `search.backward-regex` |
| `M-%` | `query-replace` (starts an interactive replace session, §3) |
| `C-M-%` | `query-replace-regexp` |

### Multi-key (`C-x`) chords

| Key | Command |
|---|---|
| `C-x C-s` | `buffer.save` |
| `C-x C-c` | `editor.quit` |
| `C-x 2` | `window.split-horizontal` |
| `C-x 3` | `window.split-vertical` |
| `C-x o` / `C-x O` | `window.focus-next` / `window.focus-prev` |
| `C-x 0` | `window.close` |
| `C-x 1` | `window.close-others` |
| `C-x b` | `editor.switch-buffer` |
| `C-x C-b` | `editor.list-buffers` (opens the `*buffer-list*` panel, §2) |
| `C-x <right>` / `C-x <left>` | `editor.next-buffer` / `editor.previous-buffer` |
| `C-x C-r` | `recent-files` (`builtin/runtime/recentf.lua`) |

### Command palette & cancellation

| Key | Command |
|---|---|
| `M-x` | `editor.execute-command` — prompts (via the minibuffer, §3) for any command by name |
| `C-g` | `editor.cancel` — resets the dispatcher / clears an unfinished prefix |

### Completion

| Key | Command |
|---|---|
| `C-M-i` | `completion.at-point` (`builtin/runtime/completion.lua`) — opens the popup; popup navigation is Rust-hardcoded (§3) |

### LSP

Source: `builtin/runtime/lsp.lua`. `M-.` follows the cross-editor
go-to-definition convention; the rest sit on the `C-c` prefix to keep
printable letters free for self-insert.

| Key | Command |
|---|---|
| `M-.` | `lsp.go-to-definition` |
| `M-?` | `lsp.find-references` (opens `*references*` panel, §2) |
| `M-,` | `lsp.jump-back` (unwind the cross-file jump ring) |
| `C-c o` | `lsp.document-symbols` (opens `*outline*` panel, §2) |
| `C-c r` | `lsp.rename` |
| `C-c a` | `lsp.code-actions` |
| `C-c i` | `lsp.inlay-hints` |
| `C-c y` | `lsp.semantic-tokens` |
| `C-c h` | `lsp.hover` |
| `C-c H` | `lsp.hover-doc` (opens `*lsp-help*` panel, §2) |
| `C-c s` | `lsp.signature-help` |
| `C-c f` | `lsp.format-buffer` |

`builtin/runtime/lsp.lua` initially binds `M-g n` / `M-g p` to
diagnostic navigation. `compile.lua` loads afterward and deliberately
replaces them with the unified error dispatcher below.

### Compile, shell command, and unified errors

Source: `builtin/runtime/compile.lua`.

| Key | Command |
|---|---|
| `M-g n` / `M-g p` | `error.next` / `error.previous` — compile/grep errors when that source has claimed navigation, otherwise LSP diagnostics |
| `` C-x ` `` | `error.next` |
| `M-!` | `shell.command` — asynchronous output in `*shell-command*` |
| `C-c c` | `compile.run` — prompts, prefilled from the detected project kind |

`M-x help` is the **index of the discovery family**, rendered as a
`*help*` buffer inside the editor, and is what the startup welcome
points at. The family — all reachable by name, none bound to a key:

| Command | Shows |
|---|---|
| `help.describe-command` | a command's description and bindings |
| `help.describe-setting` | a setting's type, default, effective value |
| `help.describe-key` | what a chord runs in **this** buffer |
| `help.describe-mode` | the active buffer's major mode |
| `help.describe-buffer` | the active buffer |
| `help.describe-hook` | a hook and its listeners |
| `help.where-is` | which keys run a command |
| `help.list-commands` | every command with its description |
| `help.list-keybindings` | every binding, grouped by scope |
| `help.list-settings` | every registered setting |
| `help.apropos` | substring search over names **and** descriptions |

`editor.describe-command` and `editor.describe-setting` still work as
deprecated aliases of their `help.*` counterparts. It is the root of
the eventual help family (`help.keys` and friends arrive with the
discovery arc), so it takes no keybinding yet — `C-h` is **not** free:
it deletes a word because non-kitty terminals cannot tell Ctrl+Backspace
from Ctrl+H.

`compile.recompile` is available through `M-x`, and through `g` inside
`*compilation*`; no global key is assigned to it. `C-c c` is unreachable
from inside a terminal window (`C-c` is consumed as the escape key) and
inside a repl buffer (which binds `C-c` at buffer scope); `M-x
compile.run` still works in both.

## 2. Buffer-local panel keymaps

Read-only panel buffers built on `pmacs.listview.open` (buffer scope
`{ scope = "buffer", buffer = <id> }`; see `builtin/runtime/listview.lua`)
all share one keymap:

| Key | Action |
|---|---|
| `RET` / `SPC` | `listview.visit` — act on the item under the cursor |
| `n` / `<down>` | `cursor.down` |
| `p` / `<up>` | `cursor.up` |
| `g` | `listview.refresh` — re-run the data source and re-render |
| `q` | `listview.quit` — restore the buffer that was active before the panel opened |

Panels currently built on this: `*references*`, `*outline*`,
`*lsp-help*` (hover docs). Header text always spells out the same
`RET`/`n`/`p`/`g`/`q` legend inline.

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

Update this file in the same PR whenever a binding is added, removed,
or moved — same discipline as `docs/agent-handoff.md`. To re-derive it
from scratch instead of trusting the table: grep `builtin/` for
`pmacs.keymap.bind`/`.bind(` and `pmacs.listview.open`, and grep
`src/editor.rs` / `src/minibuffer.rs` for `from_chord`.
