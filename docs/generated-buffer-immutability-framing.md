# Generated-buffer immutability

**PROPOSED — needs explicit user approval before implementation. DO NOT
implement, DO NOT merge.**

**Revision 2 — scouted against canonical `githubsucks/main` @ `ad41cf1`,
2026-07-28. Every claim below about pmacs was executed, not read.** The
reproductions in §0 and §2 are transcripts of throwaway probes run in this
worktree at `ad41cf1` and deleted before each commit; the counts in §1
are whole greps with the arithmetic shown, never `| head`.

## Revision history

**Revision 2 answers five review findings on PR #188 @ `9101bf5` — three
P1, two P2 — and two sweeps the review asked for by class rather than by
item. Nothing was silently rewritten; each change is attributed below.**

| finding | what it changed |
|---|---|
| **P1-1** — three Stage 2 criteria are non-discriminating | Stage 2 criteria 3, 4 and 5 rewritten (§6). All three **passed on the pre-image**: ordinary edits are already refused by the existing intercept, and `Buffer::undo` checks `read_only` *before* it looks at history (`src/buffer.rs:1302`), so "undo returns false" passes against an implementation that locks and never clears. The new wording uses a **bypass write** or Rust-side `Buffer::is_read_only()` to prove locking, and **lifts the lock inside a Rust test** before asserting `NothingToUndo` / `can_undo() == false` to prove clearing. Confirmed against the tree. |
| **P1-2** — staging omits the ownership prerequisite | New §2.8 (measured), new **Q#GB13**, amended **Q#GB5**, and staging changes in §5: ownership-by-handle is now a **prerequisite of the stage that locks each writer**, not a follow-up. Confirmed and materially worse than the review stated — §2.8 measures that a *failed* `pmacs.compile.run` already leaves a foreign buffer permanently un-editable today, and that `M-x buffer.undo` is currently the **only** way to recover a clobbered one. This arc removes that accidental safety net, which is exactly why it cannot ship without ownership. |
| **P1-3** — `mark_clean` can suppress recovery | New §2.9 and a rewritten **Q#GB10**. **Revision 1 was wrong**: it claimed `is_modified` "drives only the mode-line indicator and the buffer-list column". It is also read by `src/autosave.rs:363` — the skip that decides whether a crash-recovery slot is written — and `src/desktop.rs:302`. The rule chosen and framed: **a generated write refuses a buffer that has a `file_path`**, which bounds the contents clobber and the lock as well as the flag. |
| **P2-4** — Q#GB6 conflates byte extent with line extent | **Q#GB6** rewritten. `win.view_top` is a **line index** (`src/window.rs:373-374`) bounded by `TextView::line_count`; `win.cursor` is a byte position bounded by `Buffer::len`. The clamp is now per-coordinate and ungated, matching `rebuild_views_for`'s own shape (`src/editor_core.rs:1853-1857`), and §6 gains a longer-in-bytes / fewer-in-lines pin. |
| **P2-5** — the CRDT-dark count was knowingly stale | **Re-measured at `ad41cf1`: 276 dark** (3,251 vs 3,527), with the command shown in §10. Revision 1 quoted **264**, which `docs/active-work.md:107-115` explicitly labels historical with "the number moves with every merge and must be re-measured, not quoted". |

**Sweep A — every criterion re-audited against its pre-image, not only
3–5.** Two results beyond the cited items. First, **eight criteria pass
on `main` by design** and their bites name a *non-`main`* pre-image; that
is legitimate (`docs/agent-handoff.md` §5: "bite against every pre-image
the fix could plausibly have taken"), but revision 1 did not say so, and
an unlabelled always-green criterion is indistinguishable from a vacuous
one. §6 now carries a **pre-image column for every criterion**. Second,
**Stage 1 criterion 7's stated bite was wrong**: a "partial conversion"
that keeps a `bypass_intercept` write beside the primitive does not
produce a stale paint, it **raises** at the bypass write (§2.4,
measured), so the criterion could never have failed the way it claimed.
Restated as an explicit mutation bite.

**Sweep B — "a capability was made public without bounding who may use it
on what."** Two results beyond P1-2 and P1-3. First, the pathless rule
bounds *what* but not *who*: any Lua, including a third-party package,
can still permanently lock `*scratch*` — pathless, the default buffer,
and the quit target of five different code paths. Second, and decisive,
**the two halves of the protection are not symmetric**: the intercept is
removable (`remove_intercept`, `src/lua_bindings/mod.rs:3433`, used by
the REPL at `repl/init.lua:325-327`) and the rope lock is one-way from
Lua. That falsifies revision 1's stated reason for deferring the unlock
("a binding whose only consumer does not exist yet cannot be pinned"):
the brick scenario **is** a consumer and **is** pinnable. **Q#GB7 is
upgraded from "name it, do not build it" to "ship the unlock in Stage
1."**

**Revision 1** — initial framing: the confirmed bug, the classified
census, the primitive decision, staging, and acceptance.

This closes the class-wide half of the invariant `Buffer::set_generated_contents`
opened in terminal copy mode (#178) and that `docs/agent-handoff.md` §4 and
`COHERENCE.md` §14 both record as unfinished: **four writer mechanisms across
five buffer families still pair an erroring intercept with `bypass_intercept`
writes over a writable rope, and every one of them is emptied by undo.**

Two things the arc turns out NOT to be, both discovered by measurement:

- It is **not** "compile is the urgent one". `compile.lua` and the
  `*search-results*` panel rebind all seven undo chords to a no-op
  (`compile.lua:219`, `builtin/commands/default.lua:855`), so reaching
  them needs `M-x`. **`dired.lua` and `listview.lua` rebind nothing**, so
  a bare `C-/` empties them. The cheap half is also the exposed half.
- It is **not** a Lua-only change. Two of the four mechanisms write
  incrementally and cannot use the shipped primitive at all, and a
  buffer the shipped primitive has locked **refuses `bypass_intercept`
  writes** (§2.4, measured) — so partial adoption is impossible and a
  new Rust primitive is required.
- **Added in revision 2:** it is **not** safe to lock these buffers
  before fixing who owns them. Three of the five writers adopt any
  buffer that shares their name (§2.8, measured), and the only thing
  that currently recovers a clobbered user buffer is `M-x buffer.undo`
  — this arc's bug. Ownership is a prerequisite, not a follow-up
  (Q#GB13).

---

## 0. The bug, reproduced

`Buffer::undo` (`src/buffer.rs:1301`) gates on `ensure_writable()`
(`src/buffer.rs:568`) and nothing else. `ensure_writable` reads the Rust
`read_only` field; it never consults the intercept chain. The
`pmacs.buffer.add_intercept(buf, function() error(name .. " is read-only") end)`
idiom therefore protects the *edit* path and leaves the *history* path
wide open, while the owner's own `bypass_intercept` paint lands on the
undo stack for undo to pop.

The user-reachable chain, verified end to end:

`M-x buffer.undo` → `cmd { name = "buffer.undo" }`
(`builtin/commands/default.lua:179`) → `ed.undo()` → `EditorCore::undo`
(`src/editor_core.rs:2575`) → `Buffer::undo` → `ensure_writable`. The
chords `C-/ C-_ C-4 C-x u` are bound globally
(`builtin/keymaps/default.lua:126-136`) and the menu carries it too
(`builtin/menus/default.lua:141`). **No buffer-local rebinding removes
the command**, which is what `compile.lua`'s own comment already admits
("command/menu undo stays dispatchable", `compile.lua:236`).

### 0.1 Measured transcripts

Every line below is probe output from `ad41cf1`.

**listview panel, plain `C-/` through `dispatch_key`** — no rebinding
exists, so this is the whole distance from a keystroke to an empty panel:

```
listview BEFORE   = "H\nrow-one\nrow-two"
listview after C-/ = ""
```

**listview panel, ordinary edit** — the intercept works, which is exactly
why the idiom reads as safe:

```
listview ORDINARY EDIT = false | intercept rejected the edit:
  builtin/runtime/listview.lua:102: *probe-panel* is read-only
```

**dired listing, one `buffer.undo`:**

```
dired BEFORE  = "/tmp/.tmpEJlp9i:\n  -rw-r--r--  1 2026-07-28 17:42 alpha.txt\n  -rw-r--r--  1 2026-07-28 17:42 beta.txt"
dired AFTER 1 = ""
```

**`*shell-command*`, `M-x buffer.undo` through the real minibuffer** (`M-x`,
typed `buffer.undo`, RET):

```
shell BEFORE = "$ printf ...\nDirectory: ...\n\none\ntwo\n\n[shell exited with code 0]\n"
shell after M-x buffer.undo
             = "$ printf ...\nDirectory: ...\n\none\ntwo\n\n[output desynced by external edit]\n"
```

**Read that one carefully — it is the single most important measurement in
this document.** The Q#CM2 revision guard *noticed* and appended its
desync marker. It did **not** prevent anything: the run's exit status is
gone for good, and the buffer is still non-empty. Any acceptance
criterion phrased as "the buffer is not empty" **passes with the bug
live**. See §5, Stage 2 criterion 1.

Driven programmatically to the end, the same buffer empties completely:

```
shell AFTER 1 undo  = "... one\ntwo\n"          (exit marker gone)
shell AFTER 13 undos = ""
```

**`*search-results*`:**

```
search BEFORE = "Searching for: fn main\n\n"
search AFTER  = ""
```

### 0.2 The fix already in the tree, and what it proves

`terminal.lua` is the adopter and the precedent. `render_snapshot`
(`terminal.lua:320-337`) calls `pmacs.buffer.set_generated_contents`, and
the comment at `:322-336` documents this exact defect in these exact
terms. `claim_snapshot` (`:339-396`) keeps the erroring intercept **and**
`set_round_trip_input`, and `:351-366` states the layering that this
framing must preserve at every adopter:

> rope-level read-only protects the daemon copy, round-trip input
> protects the replica copy — and neither substitutes for the other.

A locked buffer measured at `ad41cf1`:

```
bypass write after lock            = false | buffer `*probe*` (id BufferId(4)) is read-only
M-x buffer.undo after lock leaves  = "header\n"
```

---

## 1. The census, with its arithmetic

### 1.1 `bypass_intercept` — 21 grep hits, 16 write sites

`grep -rn bypass_intercept builtin` returns **21** lines. Five of them
are prose in comments, not calls:

| file:line | what it is |
|---|---|
| `compile.lua:265` | comment above `ensure_slot`'s intercept |
| `terminal.lua:304` | comment in `unique_snapshot_name` |
| `terminal.lua:324` | comment in `render_snapshot` (the round-2 note) |
| `dired.lua:478` | comment above `claim_handle` |
| `listview.lua:9` | module header |

21 − 5 = **16 actual write call sites**, and the per-file arithmetic is
9 + 4 + 1 + 2 + 0 = 16:

| file | writes | lines |
|---|---|---|
| `compile.lua` | 9 | 319, 443, 454, 465, 506, 512, 642, 794, 798 |
| `builtin/commands/default.lua` | 4 | 827, 849, 1005, 1007 |
| `dired.lua` | 1 | 371 |
| `listview.lua` | 2 | 60, 61 |
| `terminal.lua` | **0** | — (it adopted the primitive) |

**Two corrections to the counts this lane was briefed with.** `compile.lua`
has **9** write sites, not 10 — the tenth hit is the comment at `:265`.
`terminal.lua`'s two hits are **both comments**; it performs no
`bypass_intercept` write at all, which is the correct state for an
adopter and is worth stating because the raw grep count reads as though
it still does.

### 1.2 `add_intercept` — 17 Lua sites, 6 production

`grep -rn --include='*.lua' add_intercept . --exclude-dir=target`
returns **17** lines: **6** in `builtin/`, **11** under `tests/fixtures/`.
One of the eleven (`tests/fixtures/pmacs-mcp-prompts/init.lua:84`) is a
doc comment, so the fixture *call* count is 10; 6 + 10 = 16 calls across
17 lines. The six production sites:

| site | buffer(s) | shape |
|---|---|---|
| `terminal.lua:367` | terminal copy snapshot | blanket read-only — **adopted** |
| `dired.lua:509` | every dired buffer | blanket read-only |
| `listview.lua:101` | every listview panel | blanket read-only |
| `compile.lua:266` | `*compilation*`, `*shell-command*` | blanket read-only |
| `builtin/commands/default.lua:869` | `*search-results*` | blanket read-only |
| `builtin/packages/repl/init.lua:187` | REPL buffers | **filtering** — §2.5 |

### 1.3 `set_read_only` — zero Lua callers, and no Lua binding

`grep -rn set_read_only builtin tests` returns 5 hits, **all Rust test
code** (`tests/folding_acceptance.rs:587`,
`tests/vterm_stage1_acceptance.rs:139,175`,
`tests/terminal_copy_mode_acceptance.rs:582,584`). The stronger fact:
the Lua binding table registers `"add_intercept"` and no
`"set_read_only"` / `"is_read_only"` at all
(`src/lua_bindings/mod.rs:3409` is the only match in the neighbourhood).
Lua *cannot* set `read_only` today. That matters for Q#GB7.

### 1.4 The classification, by writer mechanism

**Class A — erroring intercept + `bypass_intercept` writes over a
writable rope. This is the bug.**

1. **`terminal.lua`** — copy-mode snapshot. **ADOPTED** (`:336`). Fixed.
2. **`dired.lua`** — every dired buffer. One write, in `paint`
   (`:369-372`): `handle.buf:replace(0, handle.buf:len(), text, {bypass_intercept=true})`.
   A whole-buffer replace already. **Convertible with the shipped
   primitive.**
3. **`listview.lua`** — `*references*`, `*outline*`, `*lsp-help*` (the
   three production `listview.open` callers, all in `lsp.lua`:2056, 2102,
   2513). One writer, `render` (`:50-62`): delete-all then insert-all,
   which is a whole-buffer replace spelled in two ops.
   **Convertible.**
4. **`compile.lua`** — `*compilation*` and `*shell-command*`, both via
   `ensure_slot` (`:258`). Nine writes across five enclosing functions,
   and they are genuinely incremental:

   | enclosing function | line | shape |
   |---|---|---|
   | `resync` (`:309`) | 319 | append desync marker at end |
   | `emit_text` (`:432`) | 443 | append remainder at end |
   | `emit_text` | 454 | append `"\n"` at end |
   | `emit_text` | 465 | **positional `replace`** (CR overwrite) |
   | `apply_events` (`:480`) | 506 | **targeted delete** (erase-to-eol) |
   | `apply_events` | 512 | **targeted delete** (erase-line) |
   | `emit_text_raw` (`:639`) | 642 | append marker at end |
   | `start_run` (`:746`) | 794 | delete-all (run reset) |
   | `start_run` | 798 | insert header (run reset) |

   **NOT convertible.** `emit_text` is a terminal emulator: it tracks
   `slot.out_pos` / `slot.line_start` / `slot.parse_line_start` as byte
   anchors, reads `buf:slice(pos, len)` between writes, and settles
   `slot.expected_rev = buf:revision()` afterwards. A whole-buffer
   replace destroys every one of those anchors.
5. **`builtin/commands/default.lua`** — the independent `*search-results*`
   panel (`ensure_search_panel`, `:857`). Four writes across three
   enclosing functions:

   | enclosing function | line | shape |
   |---|---|---|
   | `search_panel_resync` (`:821`) | 827 | append desync marker |
   | `search_panel_append` (`:844`) | 849 | append match batch |
   | `pmacs.project.search` (`:982`) | 1005 | delete-all (query reset) |
   | `pmacs.project.search` | 1007 | insert header (query reset) |

   **NOT convertible**, for the same reason at smaller scale: the append
   path carries `p.next_row` / `p.expected_rev` bookkeeping.

   **Do not read `ensure_slot` as covering this panel.** It serves
   `*compilation*` and `*shell-command*` only; `*search-results*` has its
   own intercept, its own round-trip mark, its own resync and its own
   writes, and `compile.lua` names it only inside the
   `is_generated_buffer` predicate (`:216`).

**Class B — filtering intercept, deliberately partly editable.**

6. **`builtin/packages/repl/init.lua:187`** — see §2.5. Shares the root
   cause, does not share the remedy. **Out of this arc.**

**Class C — generated, but nothing ever claimed they were protected.**
Keying the inventory on `bypass_intercept` misses these entirely,
because an unprotected buffer needs no bypass:

7. **`*buffer-list*`** — `render_list` (`default.lua:387`) writes with
   plain `buf:delete(0, len)` / `buf:insert(0, body)` (`:403-404`). No
   intercept, no round-trip mark. Whole-replace shape.
8. **`*help*`** — `show_help_text` (`default.lua:1239`), same plain
   delete-all + insert-all (`:1245-1246`). No intercept.
9. **`*workers*`** — a **Rust** writer, `workers_buffer::render`
   (`src/workers_buffer.rs:65`), using `Buffer::apply_edit` (not the
   skip-intercepts path), delete-all + insert-all, then
   `Buffer::mark_clean()` (`:95`). Its fan-out is a fourth mechanism:
   `queue_generated_buffer_edits` + `rebuild_generated_buffer_views`
   (`src/lua_bindings/mod.rs:7142-7145`).

Class C is a **different defect** — nothing is defeated, because nothing
was claimed. It is named here so the inventory is complete and so a
future reviewer does not re-derive it; §4 keeps it out of this arc.

### 1.5 A correction to `COHERENCE.md` §14 (not edited here)

§14 states that "references, outline, buffer-list, and project-search all
use" listview. Measured: `pmacs.listview.open` has **three** production
callers, all in `lsp.lua` — `*references*` (`:2056`), `*outline*`
(`:2102`), `*lsp-help*` (`:2513`). `*buffer-list*` is hand-rolled in
`default.lua` (`render_list`, `:387`) and `*search-results*` is the
independent grep panel. Two of §14's four examples are wrong, and
`*lsp-help*` is missing. `COHERENCE.md` is not this lane's file to edit;
recorded here and in the PR body.

---

## 2. Ground truth (measured, not recalled)

### 2.1 What the shipped primitive is

`Buffer::set_generated_contents` (`src/buffer.rs:545`, doc comment
`:507-544`): lift `read_only`, `apply_edit_skip_intercepts` a **single
whole-buffer** `EditOp::Replace`, `clear_history()` (`:559`), re-assert
`read_only`, **return the `Edit`**. The Lua binding
(`src/lua_bindings/mod.rs:3079-3095`) fans that `Edit` out via
`notify_buffer_edit_to_windows` (`:1573`) *after* dropping the registry
borrow, because the fan-out re-enters the core.

`clear_history` clears whichever history the buffer has: the v0.1
`undo`/`redo` stacks, and in CRDT mode `CrdtState::clear_undo_history`
(`src/crdt.rs:507`), which rebinds a fresh `UndoManager` to the same doc
because loro exposes no `clear`.

### 2.2 Why history clearing is load-bearing

The doc comment's reason is retention, not tidiness: `read_only`
guarantees the pushed entries can never be popped, so a periodically
refreshed panel accumulates full rope clones nothing will ever release.
CRDT mode has the identical retention inside loro's `UndoManager`.

### 2.3 Why a bare lock is not the answer

`ensure_writable` guards the bypass path too
(`apply_edit_skip_intercepts`, `src/buffer.rs:1055-1056`). Locking a
generated buffer without giving its owner a door refuses the refresh the
buffer exists for. That is why the *pairing* is the primitive and why
there is deliberately no Lua `set_read_only` today.

### 2.4 Partial adoption is impossible — measured

This is the fact that decides the design. Once `set_generated_contents`
has locked a buffer, a subsequent owner write through `bypass_intercept`
is refused:

```
bypass write after lock = false | buffer `*probe*` (id BufferId(4)) is read-only
```

So compile **cannot** convert its run reset (`start_run:794,798`) to the
shipped primitive and keep `bypass_intercept` for streaming: the first
append after the reset raises. The streaming owner needs a write path
that *itself* carries authority. Reads are unaffected — `buf:slice`,
`buf:len` and `buf:revision` all work on a locked buffer, which is what
makes an op-level solution viable at all.

### 2.5 The REPL: same root cause, different remedy — measured

`builtin/packages/repl/init.lua:187` installs
`function(op) return repl._intercept(h, op) end` — a **filtering** policy
(`repl._intercept`, `:686-726`): reject edits wholly inside the
history/prompt region, **truncate** edits that straddle the boundary,
pass edits in the input region. Its own writes use a `_self_write` flag
(`with_self_write`, `:111-116`) rather than `bypass_intercept`, and it
has real teardown (`remove_intercept`, `:325-327`).

Does it share the bug? **Yes — measured — and I am not asserting the
comfortable answer:**

```
repl BEFORE                 = "line one\nline two\n> "
repl ordinary edit at pos 0 = false | REPL: history/prompt region is read-only
                                      (insert at 0; input region begins at 20)
repl AFTER 1 buffer.undo    = "line one\nline two\n"
repl bookkeeping after undo = _history_end=18 / _prompt_end=20  (rope is 18 bytes)
```

Undo deleted the prompt the intercept had just refused to let anyone
touch, and left `_prompt_end` pointing two bytes past the end of the
rope. The marks (`_history_end_mark`, `_prompt_end_mark`) adjust with the
rope, but `_blocks[i].start_byte` are plain integers maintained by hand
in `drop_oldest_block` (`:643-656`) and do not.

**But the remedy cannot be rope-level `read_only`**: the input region
must accept ordinary user edits, which is the whole point of a REPL. The
REPL needs either an undo that consults the intercept chain, or
mark-anchored blocks. Both are different work. **Q#GB8: out of this arc,
named deferral, with its measurement recorded above so the next lane does
not have to rediscover it.**

### 2.6 A pre-existing defect in the shipped primitive — measured

`notify_buffer_edit` (`src/editor_core.rs:1814`) updates each window's
`TextView` and overlays. It does **not** clamp `win.cursor` or
`win.view_top`. Only `rebuild_views_for` (`:1843`) does, and its doc
comment says so explicitly (`:1841-1842`). `set_generated_contents`'s
binding calls the former.

```
cursor before = 29, len = 30
cursor after set_generated_contents(G, 'x\n') = 29, len = 2
row0 after shrink = "x"          (paint did not crash)
cursor after C-p  = 29           (motion did not recover it)
```

A shrinking generated write leaves the window cursor 27 bytes past the
end of the buffer, indefinitely. **This ships today in terminal copy
mode** — refresh a snapshot to a shorter one with the point low in the
buffer and this is the state — and every adopter inherits it.

**The two coordinates fail on different axes** (review P2-4). `cursor` is
a byte position (`src/window.rs:366-367`) bounded by `Buffer::len()`;
`view_top` is a **line index** (`:373-374`, "First buffer *line* shown at
the top") bounded by `TextView::line_count()` (`src/text_view.rs:67`).
The transcript above is the byte case. The line case is **not measured**
— staging it needs a scrolled window — but it is available from the types
alone: a write that grows in bytes while collapsing lines invalidates
`view_top` on a write no byte-length comparison calls a shrink.
`rebuild_views_for` already clamps each against its own bound
(`src/editor_core.rs:1853-1857`); the clamp added to
`notify_buffer_edit` must do the same. Q#GB6.

### 2.7 What `buffer.after-edit` does and does not do

`buf:insert` / `buf:delete` / `buf:replace` do **not** fire
`buffer.after-edit`; the dispatcher and daemon do
(`src/editor.rs:1436,1984,2128,2163`, `src/daemon.rs:2976`).
`compile.lua:714` already relies on this ("hook edits don't re-fire the
hook"). Consequence for §3: a generated write does not run arbitrary Lua,
so the *fan-out* is not a re-entrancy hazard — but a scoped primitive's
**callback body** still is, because it is arbitrary owner Lua.

### 2.8 Three writers adopt any buffer that shares their name — measured

**The invariant already exists in this codebase; three writers simply do
not honour it.** `terminal.lua:300-305` states it verbatim:

> `pmacs.buffer.create` takes any caller-chosen name, so a foreign buffer
> may already be called `*terminal-copy: sh*` [...] **found-by-name is NOT
> adoption**: ownership means "this buffer is in the handle table above",
> exactly as in dired.

`dired.lua:476-504` implements the same rule: `claim_handle` looks up its
**handle table** first, and on a name collision disambiguates
`<2>`…`<99>` (`NAME_VARIANT_LIMIT`, `:474`) or raises. Three writers
instead adopt:

| writer | line | code |
|---|---|---|
| `listview.ensure_panel` | `listview.lua:95` | `find_buffer_by_name(name) or pmacs.buffer.create(name)` |
| `compile.ensure_slot` | `compile.lua:263` | `buffer_named(name) or pmacs.buffer.create(name)` |
| `ensure_search_panel` | `default.lua:861-868` | name scan over `pmacs.buffer.list()`, then `buf or create` |

Measured at `ad41cf1`, a user buffer named `*references*` and then a
references panel:

```
foreign BEFORE               = "my precious notes"
foreign AFTER listview.open  = "H\nr1"
buffers named *references*ish = 1              (no disambiguation happened)
ordinary edit to MINE now    = false | intercept rejected the edit:
                               listview.lua:102: *references* is read-only
```

The user's buffer is clobbered **and left permanently un-editable**,
because `ensure_panel` installs an erroring intercept whose handle it
discards.

**Compile is worse, and it is worse on a path that fails.**
`pmacs.compile.run` calls `ensure_slot` (`compile.lua:1090`) *before*
`start_run` validates `opts.display` (`:752-757`). Measured:

```
compile.run('true', { display = 'bogus' })
  = false | compile.lua:754: compile.run: unknown display "bogus"
foreign *compilation* contents after the FAILED call = "my precious notes"
ordinary edit to MINE after the FAILED call
  = false | intercept rejected the edit: compile.lua:267: *compilation* is read-only
```

A call that **raised and did nothing else** left the user's buffer
uneditable. Q#GB5's revision-1 recommendation — an empty
`set_generated_contents` at the end of `ensure_slot` — would make that
same failing call **empty the buffer and lock the rope**. Q#GB5 is
amended accordingly.

**Why this is a prerequisite and not a follow-up.** Today the clobber is
recoverable, and the thing that recovers it is *this arc's bug*:

```
after clobber = "H\nr1"
after undo 1  = ""
after undo 2  = "my precious notes"
```

`M-x buffer.undo` is currently the only way back. After adoption the rope
is `read_only`, the history is cleared by the same call that wrote, and
§1.3 measured that **no Lua binding can clear `read_only`**. The arc
therefore converts a recoverable clobber into an unrecoverable one, and
it removes the accidental safety net in the same commit that removes the
need for it. Q#GB13.

**Dired needs none of this work** — it already disambiguates — which is
why it is the cheaper of Stage 1's two adopters despite being the newer
one.

### 2.9 `is_modified` reaches autosave and desktop persistence — a revision-1 error

**Revision 1 stated that the flag "drives only the mode-line indicator
and the buffer-list column". That is wrong**, and it was wrong because
the sweep was `grep -rn '\.modified' builtin` plus a narrow `src` path
rather than `grep -rn 'is_modified' src`. The full sweep finds two more
consumers, both load-bearing:

- **`src/autosave.rs:359-364`** — the per-buffer skip:
  `let Some(path) = buf.file_path() else { continue };` then
  `if !buf.is_modified() { continue; }`. A clean buffer gets **no
  crash-recovery slot written**.
- **`src/desktop.rs:298-303`** — `SavedBuffer { path, modified: b.is_modified() }`,
  again only for buffers with a `file_path`.

Both gate on `file_path()` being `Some` before they read the flag. That
is the fact Q#GB10's revised rule turns on.

Also found in the same sweep, and reused below: `src/instance_buffer.rs:401`
("rendered buffer must be marked clean") is a third generated-buffer
writer that already marks clean, alongside `workers_buffer::render`.

---

## 3. The primitive decision (Q#GB1)

**The question this arc exists to answer: what write primitive do
`compile.lua` and the search panel need?**

### 3.1 Recommendation

**`Buffer::apply_generated_edit(op: EditOp) -> Result<Edit, BufferError>`
— one authorized op at a time — exposed to Lua as a new option key on
the mutators that already exist:**

```lua
buf:insert(pos, text,   { generated = true })
buf:delete(start, end_, { generated = true })
buf:replace(s, e, text, { generated = true })
```

Semantics, per call, entirely inside one `with_registry_mut`: lift
`read_only` → `apply_edit_skip_intercepts(op)` → `clear_history()` →
re-assert `read_only` → return the `Edit`. The binding then fans it out
through the `notify_buffer_edit_to_windows` call it **already makes**
(`src/lua_bindings/mod.rs:1291`, `:1302`, `:1322`), after the borrow has
dropped.

`Buffer::set_generated_contents(bytes)` is reimplemented as
`apply_generated_edit(Replace { range: 0..len, bytes })`. **It keeps its
name, its signature, its doc comment and its tests** — it becomes the
whole-buffer spelling of one primitive rather than a second primitive.

**One sentence for why it wins: it is the only candidate in which the
buffer is never observably unlocked, because the lift and the re-assert
happen inside a single registry borrow with no Lua in between — so there
is no flag to clear on an error path, no yield to defend against, and
nothing for a reviewer to audit site by site.**

### 3.2 Why the alternatives lose

**A. `append_generated_contents` — provably insufficient.** `compile.lua`
does a positional `replace` at `emit_text:465` (the CR overwrite that
makes progress bars work) and two targeted `delete`s at
`apply_events:506,512` (erase-to-eol, erase-line). Append cannot express
any of the three. Dead on the census.

**B. Scoped `with_generated_writes(buf, fn)` — the pattern this project
has already been burned by.** It is the cheapest on history (one
`clear_history` per scope instead of one per op) and that is its only
real advantage. Against it:

- **The unlocked interval is the callback's whole duration, and the
  callback is arbitrary owner Lua.** Drawn loosely around `start_run`
  (`compile.lua:746`), the scope spans `pmacs.window.switch_buffer(buf)`
  and `pcall(pmacs.process.spawn, spec)` — the buffer would be writable
  across a process spawn. Drawn tightly, compile needs four separate
  scopes (`start_run`'s reset, `resync`, `feed_bytes`, `finish_run`), each
  needing its own audit for what the body reaches.
- **Correctness reduces to "a flag cleared on every exit."** That is the
  exact shape `docs/agent-handoff.md` §5 and #155 record as a repeat
  offender, and the REPL already had to defend its own version of it:
  `with_self_write` (`repl/init.lua:111-116`) wraps in `pcall`
  specifically because "a single failed write would leave the bypass on
  for every subsequent user edit". Adding a second instance of a pattern
  the tree already documents as fragile is a poor trade for one saved
  `clear_history` per batch.
- **Yield is an error here, which helps but does not rescue it.** A Lua
  callback that yields across the Rust boundary raises
  `attempt to yield across C-call boundary` (observed in this worktree
  while probing `pmacs.dired.open`), so a yielding body surfaces as
  `Err` — but the relock must still run on that path, which is the same
  obligation.
- It is strictly harder to review: a per-site scope audit versus a
  mechanical option-key change at 16 call sites.

**C. Standalone `generated_edit(buf, op)`.** Identical semantics to the
recommendation, worse ergonomics: it re-implements the three-op argument
parsing that `buf:insert/delete/replace` already own, and turns adoption
from an option-key change into a rewrite of 16 call sites. Recommended
only if the user objects to `{ generated = true }` sitting beside
`{ bypass_intercept = true }` in the same options table.

**D. Make `Buffer::undo`/`redo` consult the intercept chain.** This would
fix all six families at once, including the REPL, and it deserves an
explicit rejection rather than silence. Against it: (i) there is no
`EditOp` to hand the chain — v0.1 undo is a whole-rope swap
(`src/buffer.rs:1327-1328`) and CRDT undo is materialize-and-replace
(`undo_crdt_mode`, `:1365`), so the chain would have to be given a
synthetic op it was never designed to see; (ii) it changes behaviour for
every intercept in the tree, including the *transforming* ones
(auto-pair, lean-input, the REPL's truncation) which have no business
rewriting an undo; (iii) an erroring intercept becomes a new Lua-raise
failure path out of `EditorCore::undo`, which today cannot fail that way.
It may still be the right answer **for the REPL specifically** — recorded
in Q#GB8's deferral, not adopted here.

### 3.3 The four questions the recommendation must answer

**How many `Edit`s are fanned out, and when?** One per generated op,
immediately, by the binding that already does it. `compile.lua`'s
`emit_text` fast path emits one insert for a whole output batch, so a
typical `feed_bytes` produces one to three ops; a CR-heavy progress bar
produces more. This is exactly today's fan-out count — the conversion
changes authority, not cardinality.

**Per-op or per-scope history clearing?** Per op, and it is cheap by
construction: because `read_only` is re-asserted immediately, **at most
one** v0.1 undo entry can exist when the clear runs and the redo stack is
always empty, so the v0.1 clear is O(1). In CRDT mode the clear rebinds
a fresh `UndoManager` (`CrdtState::clear_undo_history`), and
`create_undo_manager` (`src/crdt.rs:154`) is `UndoManager::new(doc)` plus
`set_max_undo_steps` — a subscription registration, not a document copy,
so it is O(1) in document size too. **Measurement obligation, not a
claim:** Stage 2 must show a streaming compile run does not regress
against the existing compile-mode timings in both configurations. If it
does, the escape hatch is to suppress recording rather than clear it —
recorded as a named deferral rather than designed speculatively.

**CRDT-mode behaviour?** Identical to `set_generated_contents` today. The
`Edit` carries `crdt_op` when the buffer is CRDT-backed, and
`notify_buffer_edit_to_windows` queues it via
`queue_daemon_origin_crdt_op` (`src/lua_bindings/mod.rs:1582`) so replica
mirrors import the owner's write. History clearing goes to loro's
`UndoManager`. Nothing new.

**How do the returned edits reach the fan-out without a live registry
borrow?** By construction, unchanged since #178: `run_bypass_edit`
(`src/lua_bindings/mod.rs:1445`) closes its `with_registry_mut` before
returning, and the mutator bindings call
`notify_buffer_edit_to_windows` afterwards. `apply_generated_edit` slots
into the same place `apply_edit_skip_intercepts` occupies now.

---

## 4. Decisions

**Q#GB1 — The streaming primitive.** `Buffer::apply_generated_edit(op)`,
exposed as `{ generated = true }` on the three Lua mutators.
`set_generated_contents` becomes its whole-buffer wrapper, keeping name,
signature and tests. Rationale and rejected alternatives: §3.

**Q#GB2 — `generated` is additive; `bypass_intercept` stays.** Seven
call sites outside `builtin/` depend on `bypass_intercept`, including
`tests/folding_stage2_acceptance.rs:1296-1315`, which pins that a bypass
edit still triggers the Q#FD19 interactive unfold. Redefining the
existing key would silently change that pinned seam. `generated = true`
implies bypass; passing both is legal and `generated` wins (it is
strictly stronger); passing `generated` on a buffer with no intercept is
legal (Class C would use it if it ever adopts).

**Q#GB3 — Routing is otherwise identical to bypass.** A generated write
goes through `run_buffer_edit`'s bypass arm (`src/lua_bindings/mod.rs:1368`),
including `unfold_before_interactive_lua_edit`. The unfold guard already
requires `InteractiveCommandOrigin::current()` to be `Some`, which is
false for the `process.after-tick` pump and true for `M-x compile`;
folding a `*compilation*` buffer is possible, so changing this would be a
silent behaviour change to a pinned seam for no reason this lane owns.

**Q#GB4 — History cleared per op, with a measurement obligation.** §3.3.
Deferred optimization: suppress recording instead of clearing.

**Q#GB5 — The lock-at-creation gap, and who closes it.** A
`{ generated = true }` write locks the buffer *after* its first call, so
between `pmacs.buffer.create(name)` and the owner's first generated write
the rope is writable. `dired.lua` (`claim_handle` → `paint`),
`listview.lua` (`ensure_panel` → `render`) and the search panel
(`ensure_search_panel` → the header write in `pmacs.project.search`) all
write synchronously in the same call, so the window is not observable.
**`compile.lua`'s `ensure_slot` (`:258-282`) does not** — it creates
`*compilation*` and returns, leaving it empty and writable until
`start_run`. Recommendation: `ensure_slot` ends with
`pmacs.buffer.set_generated_contents(slot.buf, "")`, using the shipped
primitive; no third surface is needed.

**Amended in revision 2 (review P1-2), and the amendment is a hard
ordering constraint, not a caveat.** `ensure_slot` is
`buffer_named(name) or create` (`compile.lua:263`), and
`pmacs.compile.run` calls it **before** `start_run` validates
`opts.display` (`:1090` vs `:752-757`). §2.8 measures that a
`display = "bogus"` call today raises *and still leaves a foreign
`*compilation*` permanently un-editable*; with the empty write placed at
the end of `ensure_slot` that same failing call would **empty the buffer
and lock the rope**, unrecoverably. So the lock may only be installed
once **Q#GB13's ownership rule guarantees `slot.buf` is a buffer compile
created**. With ownership in place the buffer is provably fresh and the
placement in `ensure_slot` is correct; without it, no placement is.

**Q#GB6 — Clamp each window coordinate against its OWN post-edit bound.**
§2.6 measures a shipped defect: a shrinking generated write leaves
`win.cursor` past the end of the rope, and neither paint nor `C-p`
recovers it. Recommendation: clamp in `EditorCore::notify_buffer_edit`
— a **clamp**, not a call to `rebuild_views_for`, because a rebuild is
O(buffer length) and would run per streaming op.

**Revised in revision 2 (review P2-4). Revision 1 said "clamp when the
buffer shrank", which conflates two different extents.** The two
coordinates are bounded by different things:

- **`win.cursor` is a byte position** (`src/window.rs:366-367`, "Byte
  position of this window's cursor"), bounded by `Buffer::len()`.
- **`win.view_top` is a line index** (`src/window.rs:373-374`, "First
  buffer **line** shown at the top of this window's viewport"), bounded
  by `TextView::line_count()` (`src/text_view.rs:67`).

A replacement can **grow in bytes while collapsing many lines into one**
— `"a\nb\nc\nd\ne\nf\n"` (12 bytes, 7 lines) replaced by a single
80-byte line — leaving `view_top` invalid on a write that a byte-length
comparison calls a *growth*. So the trigger cannot be "the buffer
shrank": the clamp runs **unconditionally**, each coordinate against its
own bound, exactly as `rebuild_views_for` already does
(`src/editor_core.rs:1853-1857`, which clamps `cursor` against `len` and
`view_top` against `line_count().saturating_sub(1)`).

**Argued from the types and from `rebuild_views_for`'s existing shape,
not measured** — unlike §2.6's cursor case, the `view_top` case needs a
scrolled window to stage and was not staged. §6 Stage 1 criterion 8b is
what turns the argument into a pin.

Recommended for **Stage 1**, because Stage 1's adopters refresh shrinking
panels constantly and because it fixes terminal copy mode retroactively.
Alternative if the user prefers a narrower Stage 1: its own lane, in
which case Stage 1 must say so out loud rather than inherit it silently.

**Q#GB7 — Ship a one-way unlock in Stage 1. Revision 2 reverses revision
1's recommendation, and sweep B is why.**

The wdired case is unchanged: dired Stage 3 (`docs/dired-framing.md` §5)
makes a dired buffer editable by removing the read-only intercept and
swapping the major mode; once dired's rope is `read_only` that is no
longer sufficient, and §1.3 measured that **no Lua binding can clear
`read_only`**. Revision 1 deferred the binding on the grounds that "a
binding whose only consumer does not exist yet cannot be pinned against a
real caller".

**Sweep B falsifies that reason.** The two halves of the protection are
not symmetric:

- the **intercept** half is removable — `remove_intercept`
  (`src/lua_bindings/mod.rs:3433`), which the REPL actually calls
  (`repl/init.lua:325-327`);
- the **rope** half is one-way from Lua, permanently.

And `{ generated = true }` is public Lua callable on **any** buffer id.
Even with Q#GB10's pathless rule, a third-party package — or a typo in
one — can permanently lock `*scratch*`: pathless, the default buffer, and
the quit target of five separate code paths (`dired.lua:914`,
`compile.lua:1052`, `listview.lua:190`, `default.lua:585`, `:1150`).
That is a caller-visible failure that exists **today**, has a consumer,
and is directly pinnable — which is exactly what revision 1 said the
binding lacked.

Recommendation: **`pmacs.buffer.unlock_generated(buf)` — one-way, clears
`read_only` and nothing else — shipped in Stage 1.** One-way rather than
`set_read_only(buf, on)` because unlocking removes protection and can
therefore never brick anything, whereas a settable lock reintroduces
precisely the "lock with no door" trap `docs/agent-handoff.md` §4 warns
about. It is strictly weaker than the setter and strictly sufficient for
both consumers (the brick escape, and dired Stage 3's mode swap).

If the user prefers the symmetric setter instead, note that
`pmacs.buffer.set_read_only(buf, on)` **contradicts
`docs/agent-handoff.md` §4's "there is deliberately no Lua
`set_read_only`"** and needs an explicit ruling rather than a quiet
addition — though the objection behind that invariant ("it also refuses
the owner's refresh") is answered once `{ generated = true }` ships.

**Q#GB8 — The REPL is out of this arc.** §2.5. Same root cause, different
remedy, its own lane. Its measured exposure is recorded above so the next
scout starts from evidence.

**Q#GB9 — Class C is out of this arc.** `*buffer-list*`, `*help*` and
`*workers*` are generated but were never claimed to be protected, so
nothing about them is *defeated*. Making them immutable is a product
decision about `COHERENCE.md` §14's list and output-channel primitives,
not a bug fix, and it should not ride a bug-fix arc. `*workers*`
additionally writes from Rust with its own fan-out pair and already
`mark_clean`s, so it is not a like-for-like conversion.

**Q#GB10 — Refuse a generated write on a path-backed buffer; then, and
only then, mark clean.** Rewritten in revision 2 (review P1-3).

`set_generated_contents` leaves `is_modified = true` (measured), so every
adopter shows `*` in the mode line (`src/editor.rs:3704`) and in
`*buffer-list*` (`default.lua:395`). `workers_buffer::render` calls
`Buffer::mark_clean()` (`src/workers_buffer.rs:95`) and
`instance_buffer.rs:401` asserts the same for its own rendered buffer,
so marking clean is the established convention for a generated buffer.

**Revision 1's justification was wrong.** It said the flag "drives only
the mode-line indicator and the buffer-list column". §2.9 measures two
more consumers: `src/autosave.rs:363`, the skip that decides whether a
crash-recovery slot is written, and `src/desktop.rs:302`. Since
`{ generated = true }` is public Lua on any buffer id, a caller could
replace a **file-backed** buffer's contents, mark it clean, and suppress
autosave recovery for it.

**The rule, stated explicitly rather than left implicit:
`Buffer::apply_generated_edit` (and therefore `set_generated_contents`)
returns an error for a buffer whose `file_path()` is `Some`.** Then
`mark_clean` is unconditionally safe, because **both** consumers gate on
`file_path()` before they read the flag (`autosave.rs:359-364`,
`desktop.rs:298-303`).

Why refuse rather than the alternative "retain modified state for
path-backed buffers": the flag rule fixes only the flag. A generated
write on a file buffer would still **replace its contents and lock its
rope**, and §1.3 measured that Lua cannot unlock. Refusing bounds all
three harms with one rule, and it is the narrower capability.

**Verified non-breaking.** None of the six generated families is
path-backed: they are all `pmacs.buffer.create`d, and no builtin Lua sets
a buffer path — `grep -rn "set_path\|set_buffer_path" builtin` finds no
call sites (only a comment in `dired.lua:42` and `lsp.lua`'s own
`active_buffer_path` local). Path binding happens Rust-side in
`from_file` / `find_file` only.

**This changes shipped `set_generated_contents` behaviour** — both the
new refusal and `mark_clean` — and therefore the terminal snapshot, so it
belongs in Stage 2 alongside the reimplementation, not smuggled into
Stage 1.

**Q#GB11 — Staging.** §5.

**Q#GB12 — The revision guard becomes near-dead, and three tests break.**
After Stage 2, an external edit to `*compilation*` or `*search-results*`
is refused at the rope, so the Q#CM2 desync machinery (`check_rev`
`compile.lua:332`, `resync` `:309`, `search_panel_check_rev`
`default.lua:832`) can essentially no longer fire. Recommendation: **keep
it** — it is cheap, and a future Rust-side writer could still mutate the
buffer — but say so, and do not delete its tests. Three compile
acceptance tests inject intruder edits through `bypass_intercept`
(`tests/compile_mode_acceptance.rs:1040`, `:1106`, `:1305`) and **will be
refused** after conversion; they must lift `read_only` Rust-side first,
exactly as `tests/terminal_copy_mode_acceptance.rs:582-584` already does.
That is a concrete, verified integration cost of Stage 2, not a surprise
to discover during implementation.

**Q#GB13 — Ownership by handle is a prerequisite, not a follow-up.** New
in revision 2 (review P1-2). `listview.ensure_panel` (`listview.lua:95`),
`compile.ensure_slot` (`compile.lua:263`) and `ensure_search_panel`
(`default.lua:861-868`) adopt any buffer that shares their name. §2.8
measures the consequence today (a clobbered, permanently un-editable user
buffer — and, for compile, from a call that *raised*), and measures that
`M-x buffer.undo` is currently the **only** recovery. Locking the rope
removes that recovery, so the rule must land in the same stage as the
lock.

Recommendation: adopt the rule the tree already states at
`terminal.lua:300-305` and implements at `dired.lua:476-504` —
**ownership means "this buffer is in my handle table"**, a name collision
disambiguates `<2>`…`<99>`, and exhausting the limit raises rather than
adopting. Three writers, one shape, each in the stage that locks it:
listview in Stage 1, compile and search in Stage 2. Dired and terminal
already comply.

Alternative considered and rejected: a standalone Stage 0 that fixes all
three at once. Rejected because each writer's ownership fix is only
load-bearing for the stage that locks that writer, and a lone ownership
PR reads as unmotivated churn without the lock that makes it urgent. If
the user prefers the standalone shape, the acceptance criteria in §6 move
with it unchanged.

**Q#GB14 — The lock is not observable from Lua, and the pins depend on
it.** New in revision 2, out of P1-1's fix. `describe.buffer` returns
`name`, `length`, `modified`, `view_count` and nothing else
(`buffer_info_table`, `src/lua_bindings/mod.rs:6352-6364`), so no Lua
assertion can read `read_only` directly. Two discriminators are
available and both are used in §6: a **`bypass_intercept` write**, which
lands on `main` and raises `` buffer `X` (id BufferId(n)) is read-only ``
once the rope is locked (measured, §2.4), and **Rust-side
`Buffer::is_read_only()`** (`src/buffer.rs:494`, already `pub`).
Recommendation: use both, and do **not** add a Lua surface for it — the
acceptance suites are Rust and need no new public API. Optional and
separable: adding `read_only` to `buffer_info_table` would be a
read-only introspection field with no new capability, useful if
Lua-level pins are ever wanted; it is not required by this arc.

---

## 5. Staging

**The proposed cut is endorsed, with two amendments.** The argument for
it is not the obvious one.

### Stage 1 — `generated-buffer-immutability-stage1`

`dired.lua` and `listview.lua` adopt `pmacs.buffer.set_generated_contents`.

- **Prerequisite, in this PR, before the lock (Q#GB13):**
  `listview.ensure_panel` (`listview.lua:95`) stops adopting a
  same-named foreign buffer. Ownership is the handle table (`panels`);
  a name collision disambiguates `<2>`…`<99>` and raises at the limit,
  matching `dired.lua:486-504`. **`dired.lua` needs no ownership work**
  — it already complies, which is why it is the cheaper of the two
  adopters.
- **Prerequisite, in this PR (Q#GB7):**
  `pmacs.buffer.unlock_generated(buf)`, a one-way clear of `read_only`.
  It is the escape from a bricked buffer and dired Stage 3's mode-swap
  door. Sweep B upgraded this from a deferral.
- `listview.lua:50-62` — `render`'s delete-all + insert-all becomes one
  `set_generated_contents(buf, body)`.
- `dired.lua:369-372` — `paint`'s whole-buffer replace becomes one
  `set_generated_contents(handle.buf, text)`.
- Both keep their erroring intercept (named error, per the layering at
  `terminal.lua:351-366`) and both keep `set_round_trip_input`.
- Plus Q#GB6's per-coordinate clamp, if approved.

**Revision 2 grew Stage 1 by two prerequisites and one reversal.** Both
additions are load-bearing for the lock rather than adjacent to it: the
ownership rule is what makes locking safe, and the unlock is what makes
a mistake survivable. Stage 1 is no longer a pure-Lua change — the
unlock is a new binding — and §5's earlier claim that it was has been
corrected below.

**Why this cut, and why Stage 1 is not merely "the cheap half":** it is
the *worse-exposure* half. `compile.lua:219` and
`builtin/commands/default.lua:855` rebind all seven undo chords to
`compile.undo-noop`; `dired.lua` and `listview.lua` rebind **nothing** —
`grep -n 'C-/\|C-_\|C-x u\|undo' builtin/runtime/dired.lua builtin/runtime/listview.lua`
returns zero binding lines. Measured, a bare `C-/` empties a listview
panel and a dired listing. Stage 1 closes the only two families
reachable without `M-x`.

**Why the cut is safe under every candidate primitive:** a whole-buffer
replace is expressible in all of A–C, and under the recommendation
`set_generated_contents` keeps its name and signature as
`apply_generated_edit`'s wrapper. Stage 1 is therefore not rework under
any Q#GB1 outcome — which is the decisive argument for cutting here
rather than shipping one large PR.

**The honest objection, and the answer.** A reviewer could call Stage 1
churn: two call sites converted to a primitive Stage 2 then rewrites.
Stage 2 rewrites the primitive's *implementation*, not its callers; the
diff at `dired.lua:371` and `listview.lua:60-61` is written once.

### Stage 2 — `generated-buffer-immutability-stage2`

- **Prerequisite, in this PR, before the lock (Q#GB13):**
  `compile.ensure_slot` (`compile.lua:263`) and `ensure_search_panel`
  (`default.lua:861-868`) stop adopting same-named foreign buffers, same
  shape as Stage 1's listview fix.
- `Buffer::apply_generated_edit` + the `{ generated = true }` option +
  `set_generated_contents` reimplemented over it.
- Q#GB10's path-backed refusal **and** `mark_clean` — one rule, both
  halves, since the refusal is what makes the flag change safe.
- Conversion of all 13 remaining write sites (`compile.lua` 9,
  `builtin/commands/default.lua` 4).
- Q#GB5's `ensure_slot` lock, which is only placeable once ownership
  lands.
- The three `compile_mode_acceptance` intruder tests updated per Q#GB12.

All the new Rust and all the review risk in one PR, which is the point of
the cut.

**Amendments to the briefed cut:**

1. **Q#GB13 (ownership) is a prerequisite of the stage that locks each
   writer**, not a follow-up and not a separate PR. §2.8 is the
   argument: this arc removes the only recovery a clobbered buffer
   currently has.
2. **Q#GB7 (unlock) is now built in Stage 1, not merely recorded** —
   revision 1 had this backwards. Sweep B found the pinnable consumer
   revision 1 said did not exist.
3. **Q#GB10 (path refusal + `mark_clean`) lands in Stage 2**, because it
   edits `set_generated_contents` itself and therefore changes the
   already-shipped terminal snapshot. **Stage 1 is no longer pure Lua**
   (Q#GB7's binding), which revision 1 claimed and revision 2
   withdraws.

**Where the REPL lands: neither stage.** Q#GB8.

---

## 6. Acceptance, with the pre-image each criterion must fail against

`M-x buffer.undo` is the user-reachable trigger and **needs no keymap**.
A criterion that only exercises the intercept, or only the chords, proves
nothing — that is precisely what `compile.lua`'s idiom already achieves
and what this bug already defeats.

**Revision 2 re-audited every criterion, not only the three the review
named (sweep A).** Each now carries an explicit pre-image class, because
an unlabelled always-green criterion is indistinguishable from a vacuous
one:

| class | meaning |
|---|---|
| **`main`** | fails on `ad41cf1`. A regression pin in the ordinary sense. |
| **fix-shape** | **passes on `main` by design**; fails against a specific *wrong implementation*, named in the criterion. Legitimate per `docs/agent-handoff.md` §5 ("bite against every pre-image the fix could plausibly have taken"), where `acc 6` deliberately passes on `main`. |
| **mutation** | passes on `main`; fails against a named one-line mutation of the fix. |
| **structural** | no behavioural pre-image. Rides **alongside** the others, never instead — a structural comparison of two authorities does not catch a misrouted consumer. |

**Q#GB14: the lock is not observable from Lua.** `describe.buffer`
carries no `read_only` field, so every "is it locked" assertion below
uses a **`bypass_intercept` write** (lands on `main`, raises
`` buffer `X` (id BufferId(n)) is read-only `` once locked) or Rust-side
`Buffer::is_read_only()`. An *ordinary* edit is not a discriminator: the
intercept refuses it either way.

### Stage 1

1. **[`main`] `C-/` cannot empty a listview panel.** Driven by
   `dispatch_key`, not a Lua call. *Bite:* measured — `"H\nrow-one\nrow-two"`
   → `""`.
2. **[`main`] `M-x buffer.undo` cannot empty a listview panel**, driven
   through the real minibuffer (`M-x`, type `buffer.undo`, RET), not
   `pmacs.command.invoke`. *Bite:* same empty result; and a chord-only
   fix passes 1 and fails this.
3. **[`main`] `C-/` and `M-x buffer.undo` cannot empty a dired listing.**
   *Bite:* measured — one undo takes the listing to `""`.
4. **[fix-shape] The owner's own refresh still works after the lock** —
   `g` on a listview panel and on a dired buffer renders *new* content.
   *Bite:* a naive `set_read_only(true)` at creation passes 1–3 and fails
   here; that is the failure mode `src/buffer.rs:521-524` exists to
   prevent. Assert the new content appears, not that the call did not
   raise.
5. **[fix-shape] An ordinary edit is refused by the INTERCEPT, not by
   the rope** — assert on the message text, which distinguishes them.
   Measured, both forms: the intercept produces
   `intercept rejected the edit: ... listview.lua:102: *probe-panel* is read-only`;
   the rope produces `` buffer `*probe*` (id BufferId(4)) is read-only ``.
   *Bite:* an adopter that deletes the intercept and relies on the rope
   passes 1–4 and fails this. The layering at `terminal.lua:351-366`
   requires the named error to survive.
6. **[fix-shape] `set_round_trip_input` is still set on both.** Pinned
   ungated via `dispatch_idle_for` reporting **false** while the panel is
   focused — the shape `tests/terminal_copy_mode_acceptance.rs` criterion
   16 uses, which needs no CRDT. *Bite:* delete the
   `set_round_trip_input` call and 1–5 all still pass; only this fails.
   A daemon-side refusal does nothing for a replica's own mirror.
7. **[mutation] A refresh reaches the window, not just the rope** —
   pinned by **painting** a shrinking render (many rows → one) and
   asserting row 1 is empty, for each adopter. **Revision 2 corrected
   this criterion's bite (sweep A).** Revision 1 claimed it caught a
   "partial conversion" that kept a `bypass_intercept` write beside the
   primitive; that is wrong — such a conversion **raises** at the bypass
   write (§2.4, measured) and never reaches a stale paint. The real bite
   is the one-line mutation *delete the `notify_buffer_edit_to_windows`
   call in the `set_generated_contents` binding*
   (`src/lua_bindings/mod.rs:3092`), which a reviewer can perform.
8. **[`main`] Cursor clamp (Q#GB6).** After a shrinking refresh,
   `pmacs.editor.cursor() <= buf:len()` and `C-p` moves. *Bite:* measured
   on `ad41cf1` — cursor 29, len 2, `C-p` leaves it at 29. Fails on
   `main` today, including for terminal copy mode.
   **8b. [`main`] `view_top` clamp, on a LONGER buffer (Q#GB6, review
   P2-4).** With a window scrolled so `view_top` sits on line 5, replace
   `"a\nb\nc\nd\ne\nf\n"` (12 bytes, 7 lines) with a single line **longer
   than 12 bytes**, then require `view_top < TextView::line_count()`.
   *Bite:* a clamp gated on "the buffer shrank" passes 8 and fails 8b,
   which is the whole of P2-4. Unlike 8, this case is argued from the
   types and from `rebuild_views_for`'s existing clamp
   (`src/editor_core.rs:1853-1857`), **not measured** — staging it needs
   a scrolled window.
9. **[`main`] A foreign buffer named `*references*` is never adopted
   (Q#GB13).** Create a plain buffer of that name with user text, then
   open the references panel. Assert **both** halves: the user's bytes
   survive **and** an ordinary edit to the user's buffer still lands;
   and the panel appears under a disambiguated name. *Bite:* measured —
   `"my precious notes"` → `"H\nr1"`, one buffer not two, and the user's
   buffer is left permanently un-editable. The second half is what fails
   if adoption is merely made "safe" by skipping the render.
10. **[fix-shape] The disambiguation limit raises rather than adopting**,
    matching `dired.lua:493-503` / `terminal.lua:309-315`. *Bite:* an
    implementation that falls back to adoption once the limit is
    exhausted passes 9 and fails this.
11. **[`main`] The unlock is real and is narrow (Q#GB7).** On a plain
    buffer with no intercept: `set_generated_contents` locks it (a
    bypass write raises), `unlock_generated` releases it (a bypass write
    lands), and an ordinary edit then lands too. Separately, on a
    listview panel: after `unlock_generated`, an ordinary edit is still
    refused **by the intercept**, asserted on the message text per
    criterion 5. *Bite:* a no-op unlock fails the first half; an unlock
    that also tears down the intercept — "unprotect" rather than
    "unlock" — fails the second.
12. **[structural] No `bypass_intercept` write remains in `dired.lua` or
    `listview.lua`**, and `listview.ensure_panel` contains no
    find-by-name adoption. Rides alongside 1–11, never instead.

### Stage 2

1. **[`main`] `M-x buffer.undo` cannot destroy `*compilation*` /
   `*shell-command*` / `*search-results*` content — and the criterion
   must assert the *exit marker survives*, not that the buffer is
   non-empty.** *Bite, and this is the whole point:* measured, the result
   of `M-x buffer.undo` on `*shell-command*` is
   `[shell exited with code 0]` replaced by
   `[output desynced by external edit]`. The buffer is still non-empty,
   so a "not empty" assertion **passes with the bug live**. The revision
   guard *marks* the corruption; it does not prevent it.
2. **[fix-shape] A streaming run's incremental writes still land**,
   including CR overwrite semantics (a progress-bar fixture) and
   erase-to-eol. Assert the produced content, not the absence of an
   error. *Bite:* the tempting half-conversion — reset via
   `set_generated_contents`, stream via `bypass_intercept` — raises
   `is read-only` at the first append (§2.4, measured).
3. **[`main`] The rope is locked BETWEEN batches, not only after the
   run.** Mid-run, after one output batch has landed and before the
   next, a **`bypass_intercept`** write must be refused and
   `Buffer::is_read_only()` must be `true`. **Rewritten in revision 2
   (review P1-1).** Revision 1 said "attempt an ordinary edit and require
   the refusal", which **passes on `main`** — the intercept refuses
   ordinary edits today whether or not the rope is locked. A bypass write
   is the discriminator: it lands on `main` (`compile.lua` performs nine
   of them) and raises once the rope is locked. *Bite:* a scope-shaped
   implementation that unlocks for a whole run passes 1 and 2 and fails
   this. A state predicate, not a geometric readout.
4. **[`main`, and also fix-shape] History is discarded per generated
   write, asserted past the lock.** In a Rust acceptance test, after N
   batches: `buffer.set_read_only(false)`, then assert `buffer.undo()`
   is `Err(BufferError::NothingToUndo)` and — under `--features crdt` —
   that the CRDT reports `can_undo() == false`; restore the lock.
   **Rewritten in revision 2 (review P1-1).** `Buffer::undo` calls
   `ensure_writable()` **first** (`src/buffer.rs:1302`) and returns
   `ReadOnly` before it ever looks at the stacks, so revision 1's
   "`buf:undo()` returns false" **passes against an implementation that
   locks the rope and never clears history**. Lifting the lock inside the
   test is what makes the assertion about history rather than about the
   lock. `tests/terminal_copy_mode_acceptance.rs:582-584` is the existing
   precedent for a Rust-side lift. This criterion fails on `main` (where
   history accumulates) *and* against the locks-but-never-clears
   implementation, which is the strongest pairing available.
5. **[`main`] `ensure_slot` leaves `*compilation*` locked before any
   run (Q#GB5).** Create the slot without running anything, then require
   a **`bypass_intercept`** write to be refused and `is_read_only()` to
   be `true`. **Rewritten in revision 2 (review P1-1):** revision 1's
   "attempt an ordinary edit; without the explicit lock it lands"
   **passes on `main`**, because `ensure_slot` installs the erroring
   intercept at `compile.lua:266` at creation time.
6. **[`main`] A generated write on a path-backed buffer is refused
   (Q#GB10).** Open a file, then call `set_generated_contents` on its
   buffer: the call must error and the buffer's contents, lock state and
   `is_modified` must all be unchanged. Second half: after an ordinary
   edit, autosave still queues that buffer. *Bite:* without the guard the
   call replaces the file buffer's contents, locks the rope, and — with
   `mark_clean` — makes `autosave.rs:363` skip it, so a crash loses the
   user's edits with **no recovery slot**. Assert the autosave queue, not
   just the flag: asserting a value was stored is not asserting anything
   reads it.
7. **[`main`] `mark_clean` (Q#GB10).**
   `pmacs.describe.buffer(b).modified` is `false` after a generated
   write on a pathless buffer. *Bite:* measures `true` on `ad41cf1`.
8. **[`main`] Foreign buffers named `*compilation*`, `*shell-command*`
   and `*search-results*` are never adopted (Q#GB13)** — same two-halved
   shape as Stage 1 criterion 9, plus the limit criterion of 10. *Bite:*
   measured on `ad41cf1` for `*compilation*`.
9. **[`main`] A FAILED `pmacs.compile.run` leaves a foreign
   `*compilation*` untouched AND editable.** Call it with
   `display = "bogus"` against a pre-existing foreign buffer of that
   name. *Bite:* measured — today the call raises at
   `compile.lua:754`, the contents survive, and the user's buffer is
   nonetheless left permanently un-editable (`ensure_slot` ran first and
   installed an intercept it discarded the handle for). With Q#GB5's
   lock placed naively it would additionally be **emptied and locked**.
   This is the criterion that pins the ordering constraint, and it fails
   on `main` today for the intercept half alone.
10. **Coverage, not a criterion: both configurations** — default and
    `--features crdt` — for criteria 1–5. CRDT must not be the only home
    of any of them; CI never enables the feature.
11. **[structural] Zero `bypass_intercept` writes remain** in
    `compile.lua` and in `default.lua`'s search panel (comments
    excepted; §1.1's arithmetic is the reference), and neither
    `ensure_slot` nor `ensure_search_panel` contains a find-by-name
    adoption.
12. **[fix-shape] The three intruder tests still assert what they were
    written to assert** after being converted to a Rust-side `read_only`
    lift (Q#GB12), rather than being deleted or weakened. *Bite:* a
    conversion that drops the intruder edit entirely leaves the desync
    machinery unpinned while the suite stays green.

---

## 7. Bets

- **That a per-op `clear_history` is not a throughput problem.** Argued
  from `create_undo_manager` being `UndoManager::new(doc)` (O(1) in
  document size) and from at most one v0.1 entry existing per clear.
  **Measured in Stage 2, not asserted here.**
- **That converting compile's nine sites does not disturb its byte
  anchors.** The conversion changes *authority*, not op shape, position
  or count — `emit_text`'s `slot.out_pos` arithmetic is untouched. The
  bet is that nothing else in the module reads `read_only` indirectly;
  criteria 2 and 3 are what test it.
- **That `{ generated = true }` sitting beside `{ bypass_intercept = true }`
  is clearer than replacing it.** Q#GB2.

## 8. Deferred (named)

- **The REPL's undo exposure** (Q#GB8), with the §2.5 measurement.
- **Class C: `*buffer-list*`, `*help*`, `*workers*`** (Q#GB9).
- **Suppress-rather-than-clear history recording**, if Stage 2's
  measurement says the per-op clear costs anything.
- **`read_only` in `describe.buffer`** (Q#GB14) — separable, no new
  capability, not required by this arc.
- **`COHERENCE.md` §14's listview consumer list is wrong** (§1.5), and
  `docs/agent-handoff.md` §4's inventory is keyed by `bypass_intercept`
  and therefore misses Class C. Neither file is edited here; both
  carried in the PR body.
- **Removed from this list in revision 2: wdired's unlock.** It is now
  Stage 1 work (Q#GB7), because sweep B found it a pinnable consumer.

## 9. Coherence impact (`COHERENCE.md` §20)

**Section served: §14 Coherent Workbench Primitives**, and specifically
its **Output channel** bullet, which is where this caveat is already
recorded ("*four writer mechanisms have not yet adopted it and remain
emptiable*"). This arc discharges that entry for Class A and replaces its
"a streaming variant of the primitive that does not exist yet" with one
that does. §14's list-primitive bullet is touched too: listview is called
"the strongest coherence asset in the UI layer", and it is currently
emptiable by one keystroke.

- **Priority 5 (finish the workbench convergence)** is the priority this
  serves. It is a correctness debt inside an existing primitive rather
  than a new primitive, so it is wiring, not model.
- **§14 consistency, added in revision 2:** Q#GB13 makes three writers
  honour an ownership rule the tree already states (`terminal.lua:300-305`)
  and already implements twice (dired, terminal). That is §14's thesis
  applied to a discipline rather than a view — five generated-buffer
  owners converging on one identity rule instead of three of them
  inventing find-by-name.
- **§6 interaction islands — none added.** No new keymap scope, no new
  dispatch shadow, no new precedence rung. The count stays at six. This
  arc deliberately does **not** add undo-chord rebindings anywhere; the
  measured point of the bug is that rebinding chords was never the fix.
- **§11 configuration registry — no new settings**, no adoption change.
- **§2 golden journey** — step 6 (compile) and the dired/browse steps are
  touched only in the sense that their buffers stop being destructible.
  No journey step opens or closes.
- **Background-work attribution — unchanged.** `compile.lua`'s process
  pump and the grep stream keep their existing ownership.
- **Protocol — no change.** Nothing new crosses the wire; the fan-out
  reuses `queue_daemon_origin_crdt_op`.
- **§14 correction owed:** the listview consumer list (§1.5) and, on
  merge, the handoff §4 table, whose four-row inventory is by
  `bypass_intercept` and therefore misses Class C.

## 10. Verification plan

Full gate suite per `CLAUDE.md` for **each** PR separately:

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings   # own step
cargo test --lib
cargo test --lib --features crdt
cargo test --test <the touched acceptance suites>
cargo test --test m4_acceptance -- --skip basedpyright
PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu
git diff --check
```

Plus, per stage:

- **Stage 1** — `cargo test --test dired_acceptance` and
  `--test listview_acceptance`, plus `--test terminal_copy_mode_acceptance`
  if Q#GB6 lands, since the clamp changes the shipped snapshot path.
  **Stage 1 now touches Rust** (Q#GB7's `unlock_generated` binding and
  Q#GB6's clamp), so `cargo test --lib` and `--lib --features crdt` are
  load-bearing for it rather than formalities.
- **Stage 2** — `cargo test --test compile_mode_acceptance` **and**
  `--test compile_mode_crdt_acceptance`, plus
  `--test terminal_copy_mode_acceptance` (the `set_generated_contents`
  reimplementation and `mark_clean` both reach it). **The search panel
  has no suite of its own** — `grep -rln 'search-results' tests/` returns
  only `compile_mode_acceptance.rs` and `m4_acceptance.rs`, so Stage 2's
  search-panel criteria need a new home rather than an existing one to
  extend.
- **Do not gate any new test on `#[cfg(feature = "crdt")]` unless it
  genuinely needs CRDT.** CI never enables the feature — measured at
  `ad41cf1`, **276 tests are dark** as a result:

  ```
  cargo test --all-targets --no-default-features --features lua54 -- --list \
    | grep -c ': test$'          # 3251   (CI's exact flags)
  cargo test --all-targets --no-default-features --features lua54,crdt -- --list \
    | grep -c ': test$'          # 3527
  ```

  3,527 − 3,251 = **276**. **Re-measured in revision 2 (review P2-5).**
  Revision 1 quoted **264**, which `docs/active-work.md:107-115` labels
  historical (#168's reading at `1b6a084`) and explicitly warns against:
  "the number moves with every merge and must be re-measured, not
  quoted." The ledger's own most recent figure is 273 at `74301d1`; this
  arc's base is later, and the number should be re-measured again rather
  than quoted from here.
- **Judge the touched suites by elapsed time as well as verdict** where
  they reach for a sibling binary (`docs/agent-handoff.md` §5).
- Commit before gating: `cargo fmt` after a commit splits the worktree
  from the branch and `git diff --check` will not catch it.
