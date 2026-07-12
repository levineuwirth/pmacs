# Editing conveniences pack — framing (Lua-side, parallel lane)

The doom/Emacs muscle-memory commands that are pure Lua on settled
substrate: goto-line, case ops, transpose, zap-to-char, line
move/duplicate/join, region line ops (sort/reverse/dedupe), and
delete-trailing-whitespace with an opt-in on-save hook. One runtime
chunk, one command family, zero contact with the in-flight lanes
(auto-pairing: `pmacs.pair.*` / `pair.lua` / its acceptance file;
indent: `indent.lua` / `pmacs.indent.*`; RET).

Roadmap: `docs/roadmap-2026-07.md` — Arc 2 spirit ("editing table
stakes") but deliberately outside Arc 2's remaining scope; runs in
parallel to the auto-pairing close-out without sharing files beyond
the two named coordination points (Q#EC1).

Revision 2: R1 findings — zap is now a real kill-chain member (the
no-chain premise was false: minibuffer keys never rotate the command
boundary, so `on_accept` observes exactly the state chaining needs);
the chain-unsafe `killring.push` is replaced by a chain-aware
`kill_range` + `break_chain`; Q#EC2 adopts the settled auto-indent
context-guard/translate discipline for all fix-up including
transformed-edit cursor repair; goto-line validates and bounds
before `push_jump`; transpose-words is specified against an
empirical Emacs 30.2 boundary table; ASCII conversion and sorting
are explicit-byte-range/-comparator (the Lua 5.4 backend is
locale-sensitive in `string.upper/lower`, string `<`, and pattern
classes); the trim callback gains an outer pcall (a raised error in
a short-circuit hook vetoes) and defined partial-sweep semantics.

Revision 3: R2 findings — the zap chain gains an origin-frontend
guard (the minibuffer session is GLOBAL while command boundaries and
`last_kill_id` are per-frontend: another frontend can accept or
cancel the prompt, and pointer input breaks the boundary while
leaving the prompt open — either would misattribute or falsely
extend a chain); `break_chain` takes a target frontend; selection
clearing after a landed edit is unconditional (a dormant zero-length
anchor re-activates on cursor motion — the auto-indent rule);
transpose-words' cursor endpoint is named correctly (after W1 in its
new position); acceptance drives the minibuffer by dispatching
RET/C-g (the Lua lifecycle `accept()` bypasses the after-edit
wrapper); the trim sweep checks the context guard after every
delete; goto-line's parser uses `tonumber` + `[ \t]*` explicitly.

Revision 4: R3 finding — `Minibuffer::begin` replaces a live
session without running its `on_cancel`, so zap's armed chain state
cannot live only in callbacks: killring gains a per-frontend
pending-prompt marker (arm at invoke, commit immediately before the
clean kill, cleared by `break_chain`/detach, and force-fresh +
clear when an ordinary kill meets it uncommitted), plus an arm-time
abandoned-marker break so a second zap after a silent replacement
cannot append to the pre-abandonment kill. `break_chain(fid)`
validates its argument; the zero-length-anchor acceptance case uses
a cursor-moving command so it cannot pass vacuously; the `accept()`
bypass is described as a path interactive key input never takes.

Revision 5 (post-approval, R3's optional hardening adopted):
`commit_kill_prompt()` reports whether a marker was still armed;
zap fails closed — no kill, chain broken — when public Lua consumed
the marker while the prompt was open.

Revision 6 (PR #111 round 1): codepoint recognition is full UTF-8
scalar validation (second-byte constraint table; transpose
validates the cursor scalar trailing-bytes-included, and a
length-consistent overlong/surrogate span behind the cursor fails
closed); capitalize is per-word across the span (Emacs
capitalize-region parity, empirically verified, with the pack's
`_`-is-a-word-constituent deviation named); trim-on-save reports
unexpected errors on the status line AND the `*errors*` buffer via
`pmacs.error` instead of silently discarding them, still never
vetoing.

## Ground truth (as of `7e127ab`)

- **The taken-chord registry is wider than `builtin/keymaps/
  default.lua`.** Runtime chunks bind globally too: killring
  (`C-k`, `M-y`; `builtin/runtime/killring.lua:357-358`), recentf
  (`C-x C-r`, `builtin/runtime/recentf.lua:85`), comment (`M-;`,
  `builtin/runtime/comment.lua:208`), lsp (`C-c` family, `M-.`,
  `M-,`, `M-?`, `M-g n`, `M-g p`), completion (`C-M-i`). Every
  chord this pack binds was verified free across ALL builtin bind
  sites. `M-g` is already a live prefix (`M-g n`/`M-g p`
  diagnostics), so `M-g g` extends an existing prefix map;
  multi-chord and shifted-punctuation sequences are established
  (`C-x C-s`, `M-%`, `M-{`).
- **Mutator discipline (substrate invariant).** `buf:insert/delete/
  replace` return the post-intercept effective `(start, end,
  inserted_len)`; callers pcall and compare EXACTLY against the
  request (killring documents why length-delta checks are defeated
  patterns, `builtin/runtime/killring.lua:202-223`). Lua mutators
  move no cursors. **The settled fix-up pattern is auto-indent's**
  (`builtin/runtime/indent.lua:57-124`): snapshot window + buffer +
  cursor BEFORE the edit (intercepts run with borrows released and
  may switch context); after the edit, a context guard stops ALL
  fix-up if the active window or buffer changed; cursor repair
  right-gravity-translates the pre-edit cursor through the
  effective triple and `goto_byte` clamps. Skipping transformed-edit
  repair is not an option: an intercept that expands a replace can
  shrink the buffer below the old cursor byte. Auto-indent also
  clears the selection UNCONDITIONALLY after a landed edit
  (`indent.lua:121-122`) — `ed.region()` hides an anchor equal to
  the cursor, and the moment a command moves the cursor that
  dormant zero-length anchor becomes an active selection.
- **`buffer.after-edit` coverage.** The dispatch cycle fires it
  once, post-command, gated on an active-buffer revision change
  (`src/editor.rs:772-776`). Edits performed inside a minibuffer
  accept callback are covered separately by
  `with_after_edit_check` (`src/editor.rs:840-864`) — the dedicated
  revision wrapper for the accept/menu/paste paths. Zap's edit is
  observed through that wrapper, not the `M-z` dispatch cycle —
  but only on the KEY path: the RET dispatch wraps the accept
  (`src/editor.rs:1170`), while the Lua lifecycle
  `pmacs.minibuffer.accept()` invokes the callback directly and
  BYPASSES `with_after_edit_check`
  (`src/lua_bindings/mod.rs:11464`). Tests that call `accept()`
  instead of dispatching RET therefore exercise a path interactive
  key input never takes (public Lua code CAN take it — which is a
  reason for tests to avoid it, not a claim it is unreachable). Direct Lua mutation outside these paths does
  not fire the hook; editops has no such path.
- **Minibuffer sessions preserve command-boundary state — but the
  session is global and boundaries are per-frontend.** While a
  prompt is active every key routes through the minibuffer's
  hardcoded handler and returns before normal dispatch
  (`src/editor.rs:693-700`) — no `rotate_command` happens.
  `rotate_command` runs once per interactive command dispatch
  (`src/editor_core.rs:2254-2262`), and `last_command` names the
  predecessor as observed from inside the currently-running command
  (`src/editor_core.rs:2273-2280`). Consequence, for a command that
  reads input via `pmacs.minibuffer.read`: inside `on_accept`,
  `this_command()` is still the invoking command and
  `last_command()` is still its predecessor; the NEXT command
  rotates the invoking command into `last_command`. This is exactly
  the state kill-chaining needs — in both directions. Three
  hazards, though: the minibuffer session lives on the shared core,
  not a frontend (`src/minibuffer.rs:60`); every input event
  updates `active_frontend` BEFORE minibuffer interception
  (`src/editor.rs:635`), so a different frontend can accept or
  cancel the prompt and the callback then observes THAT frontend's
  command history, buffer, and id; and pointer input is not
  minibuffer-intercepted — a click breaks the boundary
  (`this_command = nil`, `src/editor.rs:1288`) while leaving
  `last_command` AND the open prompt intact, so a later accept
  would still see the pre-invocation kill as `last_command` and
  falsely append. And a fourth: `Minibuffer::begin` REPLACES an
  existing session without invoking its `on_cancel`
  (`src/minibuffer.rs:103`) — any Lua code, async callback, or
  package calling `pmacs.minibuffer.read` mid-prompt silently
  discards the session, so no cancel-path cleanup can be relied on
  to run. Chain-sensitive minibuffer commands must pin their origin
  frontend, re-verify `this_command` at accept time, and carry
  their armed state somewhere a later kill can see it even when no
  callback ever fired (Q#EC6).
- **Kill-ring internals.** Appending requires
  `last_command ∈ KILL_CHAIN` AND the per-frontend `last_kill_id`
  matching the current head's id (`builtin/runtime/
  killring.lua:38`, `:92-107`); `fail_kill` clears the id so both
  conditions can never hold across a failed kill (`:83-85`).
  `push_entry` collapses a duplicate-of-head while KEEPING the
  existing id (`:71-79`) — so a naive "fresh push" API that leaves
  `last_kill_id` untouched is chain-unsafe: kill "x", push "x"
  (collapses to the same id), and the next `C-k` sees a matching id
  and appends. There is no public push today; any export must keep
  the id discipline intact.
- **The save pipeline is Lua; short-circuit hooks veto on raised
  errors.** `buffer.save` runs
  `pmacs.hook.run("buffer.before-save")` and only then `ed.save()`
  (`builtin/commands/default.lua:222-234`) — a before-save mutation
  lands in the written bytes. A callback returning `nil` never
  vetoes (`builtin/runtime/saveplace.lua:80`), but a RAISED error
  in a short-circuit hook vetoes immediately (`src/hook.rs:299`) —
  which is why saveplace pcall-wraps its whole callback
  (`saveplace.lua:81-91`). Callbacks run in registration order
  (`src/hook.rs:240`).
- **The Lua 5.4 backend is locale-sensitive where rev 1 assumed
  ASCII.** `string.upper/lower` call C `toupper`/`tolower` bytewise
  (vendored lua-5.4.7, `lstrlib.c:124`) — after `os.setlocale`,
  non-ASCII bytes can change and UTF-8 can be corrupted. String
  `<`/`table.sort`'s default comparator use `strcoll`
  (`lvm.c:370`) — not guaranteed byte-lexicographic. Pattern
  classes (`%l`, `%u`, `%w`) are ctype-backed and equally
  locale-sensitive. Only explicit byte ranges (`[a-z]`,
  `[A-Za-z0-9_]`, `[0-9]`) and explicit comparators are portable
  across locales and backends.
- **Word classes already diverge in-core.** Word *motion* is
  Unicode alphanumeric + `_` (`src/editor_core.rs:2721-2724`);
  `word_at_cursor` is deliberately ASCII alnum + `_`
  (`src/editor_core.rs:2110-2132`). This pack's word/case ops use
  the ASCII class — the `word_at_cursor` precedent, narrower than
  motion (named limitation, Q#EC4).
- **Cursor-byte bindings clamp to length, not to codepoint
  boundaries.** `goto_byte` clamps to `buf:len()` only; nothing
  guarantees the cursor sits on a UTF-8 boundary when a command
  starts. Codepoint-exact commands must fail closed on a
  continuation byte at the cursor (Q#EC5). `move_to_line` is
  0-based and clamps out-of-range (`src/lua_bindings/mod.rs:10761`)
  — but the Lua→integer conversion at the binding boundary errors
  on huge or negative numbers before any clamping runs, so inputs
  must be bounded Lua-side (Q#EC3).
- **Emacs transpose-words boundary behavior (empirical).** GNU
  Emacs 30.2, `-Q --batch`, buffer `"one two three"`, 2026-07-11:

  | point (1-based) | position | result | final point |
  |---|---|---|---|
  | 1 | BOB, start of "one" | `two one three` | 8 |
  | 2 | inside "one" | `two one three` | 8 |
  | 4 | separator after "one" | `two one three` | 8 |
  | 5 | exactly at start of "two" | `two one three` | 8 |
  | 6 | inside "two" | `one three two` | 14 |
  | 8 | separator after "two" | `one three two` | 14 |
  | 9 | exactly at start of "three" | `one three two` | 14 |
  | 11 | inside "three" (final word) | ERROR, buffer unchanged, point moved to 9 | 9 |
  | 14 | EOB | ERROR, buffer unchanged, point moved to 9 | 9 |

  Cursor exactly AT a word's start pairs the PREVIOUS word with
  that word; strictly inside a word pairs that word with the NEXT;
  a final word with no successor is an error (with point motion —
  a wart we do not copy, Q#EC5).
- **Recenter is not honestly buildable today.** `view_top` /
  `set_view_top` are daemon-window line indices
  (`src/lua_bindings/mod.rs:11015-11031`) which the TUI renders,
  but the GPU's scroll is frontend-local and caret-driven (scroll
  framing Q#S1/S2) and never consumes daemon `view_top`; no Lua API
  exposes viewport height, so "center" is not computable. Recenter
  is cut, not shipped TUI-only (Q#EC10).
- **`push_jump`/`jump_back` exist** (`src/lua_bindings/
  mod.rs:10770-10786`) and `M-,` already unwinds the jump stack.
- **Chunked scanning is the giant-line-safe idiom** — kill_line's
  4096-byte newline scan (`builtin/runtime/killring.lua:183-194`).
- **Runtime chunks load from an ordered `include_str!` list** in
  `src/editor.rs` (async → fs → syntax → mcp → listview → lsp →
  completion → saveplace → recentf → …). Command-body references
  resolve at invoke time, so load position matters only for
  load-time registrations — here, exactly one: the trim before-save
  callback (Q#EC9).
- **Handoff §6 owns adjacent deferrals this pack must not claim.**
  Word kills into the ring (`M-d`/`M-BS` rework), `C-SPC` set-mark,
  and undo amalgamation stay in their lanes; editops touches none
  of the delete-word commands.

## Decisions

### Q#EC1 — Shape: one chunk, one namespace, two coordination points

`builtin/runtime/editops.lua`, namespace `pmacs.editops.*` (config +
implementation), commands in the existing `edit.*` family plus
`cursor.goto-line`. All bindings are made inside editops.lua (the
killring/recentf pattern) — `builtin/keymaps/default.lua` is not
touched, keeping the "default keymap is stable" contract with the
auto-pairing lane.

File touch set: new `builtin/runtime/editops.lua`, new
`tests/editops_acceptance.rs`, a loader entry in `src/editor.rs`
(between completion.lua and saveplace.lua — the only load-order
requirement, Q#EC9), and ~40 lines in `builtin/runtime/killring.lua`
(Q#EC6: `kill_range`, `break_chain`, the pending-prompt marker). The two coordination points with the auto-pairing branch:
both add an editor.rs loader entry (different positions — pair.lua
goes before lsp.lua; trivial merge), and neither touches the other's
files otherwise.

Ten commands bound via eleven sequences (each verified free across
every builtin bind site):

| Chord | Command |
|---|---|
| `M-g g`, `M-g M-g` | `cursor.goto-line` |
| `M-u` / `M-l` / `M-c` | `edit.upcase` / `edit.downcase` / `edit.capitalize` |
| `C-t` / `M-t` | `edit.transpose-chars` / `edit.transpose-words` |
| `M-z` | `edit.zap-to-char` |
| `M-<up>` / `M-<down>` | `edit.move-line-up` / `edit.move-line-down` |
| `M-^` | `edit.join-line` |

M-x-only (no chords): `edit.zap-up-to-char`, `edit.duplicate-line`,
`edit.sort-lines`, `edit.reverse-lines`,
`edit.delete-duplicate-lines`, `edit.delete-trailing-whitespace`.

### Q#EC2 — Mutator discipline: the auto-indent guard, one replace per command

Every text-changing command is expressed as a SINGLE `buf:replace`
(or `buf:delete`) spanning the affected region wherever possible —
transpose, case, line move, join, sort/reverse/dedupe are each one
edit, hence one undo unit. The one coarser-grained command is named:
trim (one delete per trimmed line, Q#EC9).

The shared fix-up discipline is auto-indent's
(`builtin/runtime/indent.lua:57-124`), applied uniformly:

1. **Snapshot** `pmacs.window.current()`, the buffer handle, and
   `ed.cursor()` before the mutator.
2. **Edit** via one pcall'd mutator; capture the effective triple.
3. **Rejected** (intercept threw): nothing landed; status names the
   command + "rejected by buffer intercept"; no fix-up, no state
   updates (ring untouched, selection left alone).
4. **Context guard**: if the active window or buffer changed, stop
   ALL fix-up — no `goto_byte`, no `clear_selection` against the
   switched context; report "context changed during edit".
5. **Clean** (triple equals request): `goto_byte` to the
   command-defined cursor target.
6. **Transformed** (triple deviates): the intercept's result stands
   (accepted post-hoc semantics); status reports "altered by buffer
   intercept"; the ORIGINAL cursor is right-gravity-translated
   through the effective triple and `goto_byte` clamps (the
   command-defined target is meaningless against a relocated edit,
   but leaving the cursor unrepaired can strand it past
   `buf:len()`). Follow-up state updates that assert the requested
   edit happened (ring push) are skipped, matching killring.
7. **After ANY landed edit** (clean or transformed), under the same
   guard: `clear_selection()` UNCONDITIONALLY — not just when a
   nonempty region existed. `ed.region()` hides a zero-length
   anchor at the cursor, and the command's own cursor motion would
   re-activate it as a visible selection (the auto-indent rule,
   `indent.lua:121-122`).

### Q#EC3 — goto-line: validate and bound BEFORE any state changes

`cursor.goto-line` reads via `pmacs.minibuffer.read` (prompt
"Goto line: ", history bucket `goto-line`, `source = "none"`).
`on_accept`, in order:

1. Parse `^[ \t]*([0-9]+)[ \t]*$` — explicit ranges throughout
   (not `%d`, not `%s`; both are ctype-backed and the parsing
   contract is locale-independent). No match → status *"goto-line:
   enter a line number"*; nothing mutated — `push_jump` has NOT
   run.
2. `n = tonumber(capture)`, explicitly, then bound:
   `n = math.max(1, math.min(n, 2^31))`. `"0"` clamps to line 1
   (Emacs behavior); the upper bound keeps the value inside what
   the binding's integer conversion accepts — huge decimal input
   must clamp to the last line, not error.
3. Only now `push_jump()`, then `move_to_line(n - 1)` (0-based;
   clamps out-of-range to the last line).

`M-,` returns to the origin via the existing jump stack. All state
is read at accept time — nothing captured at invoke time.

### Q#EC4 — Case ops: DWIM span, explicit-byte-range conversion

`edit.upcase` / `edit.downcase` / `edit.capitalize` (Emacs
`*-dwim`): with an active region, transform the region and clear the
selection (stale byte range; CUA/killring precedent — deviation from
Emacs's kept region, named). Without one, transform from the first
word character at-or-after the cursor through that word's end
(Emacs's mid-word remainder semantics), cursor to the span end.
No word forward → status, no edit.

Word class: ASCII `[A-Za-z0-9_]` via explicit byte ranges (the
`word_at_cursor` precedent). Conversion: explicit `[a-z]`/`[A-Z]`
range gsub with a byte map — NOT `string.upper/lower` and NOT
`%l`/`%u` classes, all of which are locale-backed on the Lua 5.4
backend (ground truth); this also keeps Lua 5.4 and LuaJIT
identical. Non-ASCII bytes pass through untouched — pinned in
acceptance (an `é` in the span is byte-identical after the op).

Capitalize is PER-WORD across the span — Emacs capitalize-region
parity (PR #111 R1 finding 2; empirical, Emacs 30.2 `-Q --batch`:
`"hello WORLD"` → `"Hello World"`, `"9abc a9bc"` → `"9abc A9bc"`):
each word's first byte is upcased when it is a letter, every other
letter downcased; a digit-led word keeps its letters lowercase. One
named deviation remains: `_` is a word constituent in this pack's
class (the `word_at_cursor` precedent) but symbol-syntax in Emacs,
so `foo_bar` capitalizes as `Foo_bar` here versus Emacs's
`Foo_Bar`.

### Q#EC5 — Transpose: codepoint-aware chars, Emacs-verified word boundaries

`edit.transpose-chars` (C-t): swap the codepoints before and at the
cursor, cursor ends after both (Emacs drag-forward). At EOL (next
char is `\n` or EOF) with ≥2 preceding codepoints: swap the two
before the cursor (Emacs special case). Fewer than two reachable
codepoints → status, no edit. Codepoint recognition is FULL scalar
validation, not lead/continuation range checks (PR #111 R1 finding
1): a shared validator enforces the UTF-8 second-byte constraint
table — overlongs (`C0`/`C1`, `E0 80..9F`, `F0 80..8F`), surrogates
(`ED A0..BF`), and beyond-`U+10FFFF` (`F4 90..BF`, `F5..FF`) all
fail — and the scalar AT the cursor is validated trailing bytes
included (a valid lead followed by non-continuation bytes must not
ride along as "one character"). Failures fail closed: a
continuation byte at the cursor, a malformed scalar at the cursor,
and a length-consistent-but-invalid span behind it each report and
leave the buffer untouched — `goto_byte` does not guarantee
boundary alignment, and buffers are byte-clean, so malformed input
is reachable. (Zap's single-codepoint input check uses the same
validator as defense-in-depth; minibuffer contents arrive as
Rust-side UTF-8 — `set_contents` is `String`-typed — so the
buffer-facing checks are the load-bearing ones.) Newlines
participate (transpose across lines works). One replace spanning
exactly the two codepoints.

`edit.transpose-words` (M-t), specified against the Emacs 30.2
table in Ground truth:

- **W1** = the word containing the cursor, if the cursor lies
  STRICTLY after that word's start; otherwise the nearest word
  entirely before the cursor; if none exists (BOB / leading
  separators), the first word at-or-after the cursor. A cursor
  exactly at a word's start therefore pairs the PREVIOUS word with
  it — the point-5/point-9 rows.
- **W2** = the first word strictly after W1's end. No W2 → status,
  no edit, **no cursor motion** (Emacs errors AND moves point; the
  point motion is a wart we don't copy — named deviation).
- Swap W1 and W2's spans in one replace, separator bytes between
  them preserved verbatim; cursor ends at the replaced span's end —
  immediately after W1 in its NEW position (post-swap, W1 sits
  last; matches the observed final points 8 and 14).

Word class ASCII (Q#EC4). Named simplification: W1/W2 are always
exact word spans — Emacs's `transpose-subr` can drag leading
separators into the region at BOB edges; we never transpose
separator bytes.

### Q#EC6 — Zap: a real kill-chain member via a chain-aware killring export

Rev 1's no-chain design rested on a false premise. Ground truth:
minibuffer keys never rotate the boundary, so inside `on_accept`
`this_command()` is `edit.zap-to-char` and `last_command()` is
M-z's predecessor — and the next command rotates zap into
`last_command`. That is exactly the state real chaining needs, in
both directions. So zap chains like Emacs:

- `KILL_CHAIN` gains `edit.zap-to-char` and `edit.zap-up-to-char`:
  a zap right after `C-k` appends to that kill's entry; a `C-k`
  right after a zap appends to zap's entry; consecutive zaps
  append.
- New killring exports (replacing rev 1's chain-unsafe `push`,
  whose duplicate-of-head collapse plus untouched `last_kill_id`
  would let a later `C-k` append across a foreign push):
  - `pmacs.killring.kill_range(start, stop)` — operates on the
    active buffer (the `cut` shape). Validates before ANY mutation:
    integers, `0 <= start < stop <= buf:len()`, else it errors (a
    programmer-facing API misuse, not a status). Slices the text
    first, then one pcall'd exact-checked `buf:delete`. Clean →
    `kill_push` (chain-aware append-or-push; updates
    `last_kill_id`, mirrors the acting frontend's clipboard),
    returns `true`. Rejected → killring-standard status +
    `fail_kill`, returns `false, "rejected"`. Transformed → the
    edit stands, status + `fail_kill`, returns
    `false, "transformed", estart, estop, einserted` so the caller
    can run its Q#EC2 guarded cursor repair.
  - `pmacs.killring.break_chain([fid])` — public `fail_kill`,
    targeting `fid` when given (validated as a nonnegative integer
    before indexing per-frontend state), else the acting frontend.
    The target parameter is required by the origin guard below: the
    frontend whose chain must break is the INVOKING one, which need
    not be the frontend whose input triggered the callback.
    Clearing BOTH the chain id and the pending-prompt marker
    (below) is sufficient to break a chain: appending requires the
    id match AND the `KILL_CHAIN` predecessor together, and the
    marker fail-safes the path where no callback ever ran.
  - `pmacs.killring.arm_kill_prompt()` /
    `pmacs.killring.commit_kill_prompt()` — the pending-prompt
    marker (below).

**Pending-prompt marker (the R3 blocker).** `Minibuffer::begin`
replaces a live session WITHOUT running its `on_cancel` (ground
truth) — so zap's cancel-path `break_chain` cannot be relied on to
run: C-k, M-z, a package's `pmacs.minibuffer.read` silently
replacing the prompt, the replacement closing, then C-k would
rotate `edit.zap-to-char` into `last_command` with the old id still
matching, and append as though the zap had happened. The armed
state must therefore live where every kill can see it, not in a
callback that may never fire. Killring gains per-frontend
`pending_kill_prompt[fid]`:

- **Arm** (`arm_kill_prompt()`, called by zap at invoke time,
  before `minibuffer.read`): sets the marker for the acting
  frontend. It does NOT touch `last_kill_id` — backward chaining
  (`C-k` then a completed zap appends) needs the id alive. If the
  marker is ALREADY set, the previous armed prompt was silently
  discarded without resolution: `fail_kill` first, then arm —
  otherwise a second `M-z` after a silent replacement would commit
  the stale marker away and falsely append to the pre-abandonment
  kill (a residue the marker scheme alone would mask).
- **Commit** (`commit_kill_prompt()`): clears the marker and
  RETURNS whether one was armed (post-approval hardening, adopted
  from R3's optional note). Zap calls it immediately BEFORE
  `kill_range` on the clean-input path — before, not after, or
  `kill_push` would see the marker and force-fresh, killing
  backward chaining — and treats a `false` return as fail-closed:
  some public Lua consumed the marker while the prompt was open,
  so the armed state is no longer trustworthy — status +
  `break_chain(origin_fid)`, no kill.
- **`break_chain([fid])`** clears the marker along with
  `last_kill_id` — every failure path already routes through it.
- **Ordinary `kill_push` encountering an uncommitted marker** for
  the acting frontend forces a FRESH entry and clears the marker —
  this is the fail-safe that catches the silent-replacement case:
  the abandoned zap left its marker, and the next `C-k` refuses to
  append no matter what `last_command` and the id say.
- **`frontend.detached`** clears the marker with the existing
  per-frontend state (`killring.lua:340-343`).

**Origin guard (the R2 blocker).** The minibuffer session is global
while command boundaries and `last_kill_id` are per-frontend, and
pointer input breaks the boundary without closing the prompt
(ground truth). So zap captures `origin_fid = pmacs.frontend.id()`
when it OPENS the prompt, and `on_accept` proceeds only when BOTH
hold:

- `pmacs.frontend.id() == origin_fid` — the completing frontend is
  the invoking one (a different frontend's accept would run the
  kill against ITS buffer, history, and chain state); and
- `ed.this_command()` is still the invoking zap command — pointer
  input (or any boundary-breaking event) on the origin frontend
  sets `this_command = nil` while leaving `last_command` as the
  pre-zap kill, so without this check a later accept would falsely
  append the zap to that old kill.

On either failure: abort — no scan, no edit — with status, and
`break_chain(origin_fid)` (breaking the ACTING frontend's chain
would leave the origin's pre-zap chain alive). `on_cancel` does the
same targeted `break_chain(origin_fid)` and clears the captured
`origin_fid`, regardless of which frontend cancelled.

`edit.zap-to-char` (M-z): at invoke time, capture
`origin_fid = pmacs.frontend.id()` and `arm_kill_prompt()`, then
open the prompt ("Zap to char: "); all buffer state is read at
accept time. After the origin guard: input must be exactly one
UTF-8 codepoint, else status + `break_chain(origin_fid)`. Chunked
forward scan from the cursor; found at `p` → Q#EC2 snapshot,
`commit_kill_prompt()` (a `false` return aborts fail-closed:
status + `break_chain(origin_fid)`, no kill), then
`kill_range(cursor, p + #char)`; on the transformed return,
guarded translate-and-clamp repair. Not
found → status *"zap: no 'c' after the cursor"* +
`break_chain(origin_fid)`. `edit.zap-up-to-char` kills
`[cursor, p)`; a match AT the cursor is a zero-length no-op with
status + `break_chain(origin_fid)` (Emacs parity on the text, chain
broken on the no-op).

Every non-kill outcome breaks the origin frontend's chain: origin
mismatch, disturbed boundary, cancel, invalid input, no match,
zero-length, rejection, transformation — and when a silent session
replacement lets NONE of those paths run, the uncommitted marker
makes the next kill fail safe to a fresh entry. Only a clean kill
by the origin frontend, through the commit, extends or starts a
chain.

`cursor.goto-line` adopts the same origin guard for consistency
(abort with status on mismatch — no `push_jump`, no motion): it has
no chain stakes, but a prompt completed by a different frontend
moving THAT frontend's cursor is the same wrong-actor bug in milder
form.

### Q#EC7 — Line ops: plain byte moves, explicitly not indentation

All single-cursor-line in v1 (region-spanning variants deferred);
none of them inserts computed whitespace, calls `pmacs.indent.*`, or
reindents after moving — stated to keep this pack out of the indent
lane permanently, not just while #109's follow-ups settle.

- `edit.move-line-up/down`: swap the cursor line with its neighbor
  via one replace spanning both lines (newline placement handled
  when the last line lacks a trailing `\n`); cursor keeps its byte
  column, clamped to the moved line's length, on the line's new
  location. At the first/last line → status, no edit.
- `edit.duplicate-line`: insert a copy of the cursor line below
  (last line without `\n` → insert `"\n" .. line` at EOL); cursor
  to the same byte column in the copy.
- `edit.join-line` (M-^, Emacs delete-indentation): join the cursor
  line onto the previous one — one replace of
  [prev line's trailing-whitespace start, current line's
  leading-whitespace end) with a single space, or with nothing when
  either side of the junction is empty (prev line blank or current
  content empty — avoids `" bar"`). Cursor at the junction. On the
  first line → status, no edit.

### Q#EC8 — Region line ops: whole-line expansion, explicit byte comparator

`edit.sort-lines` / `edit.reverse-lines` /
`edit.delete-duplicate-lines` require an active region (else status
*"…: no active region (select the lines first)"*). Expansion rule:
start → beginning of the line containing `region.start`; end → end
of the line containing `region.end - 1`, including its newline when
present (a region ending exactly at a BOL excludes that line —
Emacs sort-lines). Lines split/rejoined preserving the presence or
absence of a final newline.

Sort uses `table.sort` with an EXPLICIT byte-wise comparator —
never the default string `<`, which is `strcoll`-backed and
locale-dependent (ground truth). Equal lines are identical, so
sort instability is moot. Dedupe keeps the first occurrence, status
reports the count removed. One replace; fix-up per Q#EC2 (selection
cleared, cursor to the region start, transformed edits translated).

### Q#EC9 — Trailing whitespace: command always, hook opt-in, veto-proof

`edit.delete-trailing-whitespace`: chunked line scan; one
`buf:delete` per line that has a trailing ` `/`\t` run, applied
bottom-up so earlier deletes never shift later targets. Undo grain
is one step per trimmed line — named (undo amalgamation is an
existing deferral, not this pack's).

Partial-sweep semantics: the Q#EC2 context guard is checked after
EVERY delete, not only at final fix-up — a clean delete's intercept
can switch the active window or buffer, and the sweep must stop at
that point rather than keep deleting through the saved buffer
handle behind the switched-to context's back. The sweep also stops
at the first non-clean edit (rejected or transformed), reporting
which line failed. Fix-up then reflects EVERY edit that actually
landed — the cursor is right-gravity-translated through each
applied effective triple (including a transformed one, as returned)
and clamped, and the selection is cleared (unconditionally, Q#EC2
step 7) if any delete landed — all skipped when the context guard
tripped. A clean full sweep translates the cursor the same way
(inside a trimmed run → its start).

On-save: `pmacs.editops.trim_on_save([on])` — getter/setter (the
`killring.max` shape), **default off** (silently rewriting bytes on
save is a policy, not a default). The before-save callback is
registered unconditionally at chunk load and gates on the flag
inside, so its registration position is fixed by loader order:
editops.lua loads BEFORE saveplace.lua, making trim run before
saveplace's cursor-record within the before-save fan-out (recorded
places see post-trim text). The ENTIRE callback body is wrapped in
pcall with a `nil` return on both paths (the saveplace pattern):
returning `nil` never vetoes, but a raised error in a
short-circuit hook vetoes immediately (`src/hook.rs:299`). An
unexpected error caught by that pcall is NOT silently discarded
(PR #111 R1 finding 3) — it reports on both channels the autosave
sweep uses: the status line (visible when the save fails or is
vetoed; a successful save overwrites it with "saved ...") and the
`*errors*` buffer via `pmacs.error` (durable either way; the
async/mcp/syntax/autosave convention). Both reports are
themselves pcall'd so a broken reporting channel cannot resurrect
the veto.

### Q#EC10 — Cut from the pack: recenter

`C-l` recenter is not shipped: the GPU never consumes daemon
`view_top` (its scroll is caret-driven and frontend-local) and no
API exposes viewport height, so "center/top/bottom" is either a lie
on one frontend or unimplementable. Deferred behind a
viewport-facts / frontend-scroll-control substrate (Arc 8 adjacent),
not worked around.

## Bets

1. **Free-chord verification against ALL bind sites is sufficient.**
   The registry-of-taken-chords contract with the auto-pairing lane
   is about not colliding and not rebinding — new bindings on
   verified-free chords are in-bounds.
2. **ASCII word/case semantics are acceptable v1** — they match
   `word_at_cursor`'s existing posture, and with explicit byte
   ranges non-ASCII text is passed through untouched in every
   locale, never corrupted.
3. **One-replace-per-command undo grain is what users expect** from
   transpose/move/sort — and it falls out of the mutator discipline
   rather than needing grouping substrate.
4. **Minibuffer boundary preservation is stable substrate, not
   accident** — the shadow's early return and `rotate_command`'s
   contract are documented behavior with the M-x path already
   depending on them. What is NOT assumed is who completes the
   prompt or that the boundary survives until accept: the origin
   guard re-verifies both instead of trusting them, and the
   acceptance suite pins the preserved-state observation and the
   guard's failure modes directly.

## Deferred (named)

- Recenter + any frontend scroll control (needs viewport facts on
  the wire; Arc 8 adjacent).
- Unicode-aware case conversion and word classes (would also
  reconcile the in-core motion vs `word_at_cursor` split).
- Locale-aware collation modes for sort-lines (byte order is the
  contract until then), and numeric sort.
- Region-spanning move/duplicate (drag-stuff parity).
- Emacs's separator-dragging `transpose-subr` edge at BOB (we
  always transpose exact word spans).
- Ensure-final-newline on save (separate policy from trim).
- fixup-whitespace refinements for join (punctuation-aware spacing).
- Chords for the M-x-only commands if usage earns them.

## Acceptance

`tests/editops_acceptance.rs`, dispatch-driven where a binding
exists (per the established discipline: `pmacs.command.invoke`
bypasses dispatch, so bound-key cases must go through key dispatch
or a dead binding passes vacuously). Minibuffer-driven commands may
seed input with `set_contents()`, but MUST complete the session by
DISPATCHING RET (and C-g for cancel cases) — the Lua lifecycle
`accept()` invokes the callback directly and bypasses
`with_after_edit_check` (ground truth), a path interactive key
input never takes. Cross-frontend cases ride the same multi-frontend
harness the kill-ring suite already uses.

- **Boundary-state pin** (the Q#EC6 substrate observation, asserted
  directly): inside zap's `on_accept`, `this_command()` is
  `edit.zap-to-char` and `last_command()` is the pre-M-z command;
  after accept, the next command observes `last_command() ==
  "edit.zap-to-char"`.
- goto-line: dispatch `M-g g`, accept "5" → line 5 (1-based), jump
  pushed (`M-,` returns); `"0"` → line 1, no error; a 25-digit
  input → last line, no error; `"abc"` → status, no motion, and
  the jump stack is untouched (nothing pushed before validation).
- Case ops: region upcase + selection cleared; mid-word `M-u`
  transforms cursor→word-end and moves the cursor there; cursor on
  separators skips forward to the next word; no word forward → no
  edit; `é` in the span is byte-identical while ASCII neighbors
  flip — and stays byte-identical regardless of process locale
  (explicit-range pin); capitalize: region `"hello WORLD"` →
  `"Hello World"` (per-word, the Emacs parity row), `"9abc a9bc"` →
  `"9abc A9bc"` (digit-led word keeps letters lowercase), and
  `"foo_bar baz"` → `"Foo_bar Baz"` (the named `_` deviation,
  pinned).
- Transpose-chars: mid-line swap + cursor advance; EOL two-before
  swap; BOB/single-char no-op; multi-byte: swapping `é` and `x`
  yields intact UTF-8 both orders; across-newline swap; **cursor
  parked on a continuation byte → status, no edit** (fail-closed
  pin); **malformed-scalar pins**: a valid lead with a
  non-continuation trailing byte at the cursor (`a\xC3xb`), an
  overlong span behind the cursor (`\xE0\x80\x80b`), and a
  beyond-`U+10FFFF` span behind it (`\xF4\x90\x80\x80b`) each →
  status, buffer byte-identical. Undo restores the original in ONE
  step (grain pin).
- Transpose-words: the full nine-position Emacs table from Ground
  truth, byte-for-byte including final cursor positions for the
  seven mutating rows; the two no-successor rows assert NO edit and
  NO cursor motion (the named deviation); separator bytes between
  the words preserved verbatim; one-step undo.
- Zap chain matrix: `C-k` then `M-z` → one appended entry; `M-z`
  then `C-k` → one appended entry; `M-z M-z` → one appended entry;
  each of cancel, invalid (multi-char) input, no-match, and
  zero-length up-to BREAKS the chain (shape: `C-k`, failed/aborted
  zap, `C-k` → the two `C-k`s are separate ring entries); killed
  bytes land on the ring head and the clipboard slot; up-to-char
  leaves the target; match-at-cursor up-to is a zero-length no-op;
  **after-edit pin**: a completed zap fires `buffer.after-edit`
  exactly once (the RET-dispatch wrapper — this is why the suite
  dispatches RET rather than calling `accept()`).
- **Origin-guard matrix** (multi-frontend harness; every case ends
  with frontend A's next `C-k` producing a FRESH ring entry):
  frontend A invokes `M-z`, frontend B dispatches the accept → no
  edit on either frontend, status, A's chain broken; A invokes, B
  dispatches C-g → no edit, A's chain broken, `origin_fid`
  cleared; A does `C-k`, `M-z`, then a pointer click on A, then
  accept → NO append to the pre-zap `C-k` entry (the
  `this_command` re-check), no edit, A's chain broken. Goto-line's
  milder origin guard: A invokes `M-g g`, B accepts "5" → no
  motion on either frontend, nothing on the jump stack.
- **Silent-replacement matrix** (the R3 blocker; `on_cancel` never
  runs in either case): `C-k`, `M-z`, a programmatic
  `pmacs.minibuffer.read` replacing the zap session, the
  replacement closed by dispatched RET, then `C-k` → TWO separate
  ring entries (the uncommitted marker forces the second kill
  fresh); `C-k`, `M-z`, silent replacement, replacement closed,
  then a SECOND `M-z` completed cleanly → the zap's kill is a
  FRESH entry, not an append to the pre-abandonment `C-k` (the
  arm-time abandoned-marker break). A committed normal zap right
  after `C-k` still appends (the marker must not tax the healthy
  path). **Consumed-marker pin** (the adopted hardening): public
  Lua calls `commit_kill_prompt()` while zap's prompt is open →
  the accept aborts with status, no edit, chain broken.
- `break_chain(fid)`: a non-integer or negative `fid` errors
  before any per-frontend state is touched.
- `kill_range` API: invalid arguments (non-integer, negative,
  `start >= stop`, `stop > len`) error BEFORE any ring or buffer
  mutation; a rejected delete → `false, "rejected"`, ring
  untouched, chain broken; a transformed delete → the transformed
  edit stands, `false, "transformed", triple`, ring untouched,
  chain broken, and zap's guarded repair leaves the cursor
  translated and clamped (never past `buf:len()`).
- Line ops: move down/up round-trips; first/last line no-ops;
  last-line-without-newline move and duplicate both preserve the
  no-trailing-newline invariant; duplicate places the cursor at the
  same column in the copy; join collapses the junction to one
  space, to zero when the previous line is blank; each is one undo
  step.
- Region ops: sort/reverse/dedupe on a region including a
  region-ends-at-BOL exclusion case and a final-line-without-
  newline case; **byte-order pin**: `{"b", "A", "a", "B"}` sorts to
  `{"A", "B", "a", "b"}` regardless of process locale; dedupe count
  in status; no-region → status, no edit; one undo step each;
  selection cleared.
- Intercept discipline, per Q#EC2: a rejecting intercept on each
  command class → status, no state change; a transforming intercept
  → the intercept's result stands, cursor right-gravity-translated
  and clamped (pinned with an expanding replace that shrinks the
  buffer below the old cursor), selection cleared, no ring push; a
  context-switching intercept → ALL fix-up skipped, the switched-to
  window/buffer's cursor and selection untouched; **zero-length
  anchor pin** (Q#EC2 step 7): `begin_selection` at the cursor with
  no motion, then a clean mid-word `M-u` — a command whose clean
  target MOVES the cursor, so the case cannot pass vacuously — →
  no active region afterward (the dormant anchor must not
  re-activate as a selection spanning the cursor's move to the
  word end).
- Trim: command trims multiple lines; cursor inside a trimmed run
  lands at the run start; cursor after a trimmed run shifts left
  correctly; undo grain = one step per trimmed line (pinned,
  named); partial sweep: a rejecting intercept on one line stops
  the sweep, reports the line, and cursor translation reflects
  every landed delete; **mid-sweep context switch**: a CLEAN delete
  whose intercept switches the active buffer stops the sweep at
  that delete — later (earlier-line) targets in the original
  buffer are untouched, and no fix-up lands in the switched-to
  context; `trim_on_save(true)` + `buffer.save` → file
  bytes on disk are trimmed, and the saveplace-recorded cursor
  reflects post-trim offsets (ordering pin); trim disabled
  (default) → save writes bytes untouched; **veto-immunity pin**: a
  rejecting intercept during on-save trim → the save still
  proceeds with a status report; another before-save callback's
  veto still vetoes (trim's `nil` return masks nothing);
  **unexpected-error pin**: an error raised inside the on-save trim
  (beyond the per-edit pcalls) → the save still proceeds AND the
  failure lands in the `pmacs.error` log (stubbed, the m9_6
  pattern) — never silently discarded.

No CRDT-specific suite: every editops edit is a daemon-peer edit on
the dispatch or minibuffer-accept path with no optimistic-classifier
contact — the same posture as comment-toggle (which ships without
one).
