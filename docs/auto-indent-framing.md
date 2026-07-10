# Auto-indent on newline — framing (Arc 2, editing table stakes)

Pressing RET in pmacs inserts a bare `\n`; every indented line means
re-typing the indentation by hand. This adds the table-stakes behavior:
RET carries the current line's indentation onto the new line,
language-agnostic, as one undoable edit. Second-to-last Arc 2 item;
auto-pairing follows as its own framing + PR.

Roadmap: `docs/roadmap-2026-07.md` Arc 2 ("auto-indent on newline").
Revision 5: Q#AI9 is success-gated (`insert_char` reports success;
clear only on Ok — a rejected edit must not mutate selection state),
and Q#AI2 names the one deliberate behavior change `buffer.newline`
inherits through the shared primitive. Earlier: live-origin
translation (Q#AI8), M6.4 same-kind intercept contract (Q#AI5),
empty-selection core fix with the GPU residual named (Q#AI9).

## Ground truth (as of `efa41cb`)

- **RET → `buffer.newline` → `ed.insert_char_over_region(10)`**
  (`builtin/keymaps/default.lua:52`,
  `builtin/commands/default.lua:110`): one `Replace` when a region is
  active (CUA type-over, one undo step), plain insert otherwise. The
  no-region path does **not** clear an empty selection, and
  `Window::region()` returns `None` when anchor == cursor while the
  selection object persists (`src/window.rs:244-255`). So a
  zero-length selection (e.g. `S-Left` at BOF) survives the edit, and
  the moment the edit advances the cursor away from the anchor the
  region is **already nonempty** — the very next self-insert
  type-overs the just-inserted text. The same no-region arm serves
  `buffer.self-insert` and `buffer.tab`, so ordinary typing has the
  identical bug today (`S-Left` at BOF, `x`, `y` → the `y` type-overs
  the `x`) — it is not RET-specific. `insert_char` catches the
  intercept rejection internally — status message, return `()`
  (`src/editor_core.rs:1681-1692`) — so its caller cannot observe
  failure; and `insert_char_over_region`'s doc comment already claims
  "any selection is cleared" while only the region arm does it
  (`src/editor_core.rs:1698`, `:1717`).
- **Runtime modules are individually embedded** — each
  `builtin/runtime/*.lua` has an explicit `include_str!` + eval entry
  in `src/editor.rs` (comment.lua's at `src/editor.rs:336`). There is
  no discovery mechanism; a new module ships with a loader entry or it
  never loads.
- **The two frontends' Enter paths diverge.** The TUI attach client
  round-trips Enter to the daemon keymap by design —
  `src/optimistic.rs:90-93` names "indentation, newline-with-indent"
  as the reason, locked by `classify_enter_and_tab_are_round_trip`.
  The GPU frontend treats plain Enter as optimistic-eligible
  (`optimistic_insert_text`, `pmacs-gpu/src/main.rs:1520-1530`) and
  ships a raw `"\n"` CRDT op that never touches the keymap — justified
  in its doc comment by "`buffer.newline` reduces to plain
  `insert_char(10)`", the exact premise this feature invalidates. The
  GPU already round-trips Enter when a selection is active, the
  completion popup is open, or the dispatcher is busy/stale. The
  selection gate reads `DecorationKind::Selection` decorations
  (`pmacs-gpu/src/main.rs:2054-2060`); an **empty** selection paints
  no decoration, so it does not gate — GPU optimistic typing proceeds
  over an empty anchor.
- **Consequence of the GPU bypass: RET rebindings are dead in the GPU
  frontend.** The classic buffer list binds RET buffer-locally to
  `editor.buffer-list-visit` (`builtin/commands/default.lua:407-411`)
  through normal dispatch; unlike listview it is not marked for
  round-trip input, so GPU RET there inserts a raw newline instead of
  visiting. Any user rebind of RET is bypassed the same way.
- **Dispatch supplies an empty active-mode list** — the keymap stack
  is resolved with `&[]` for modes (`src/editor.rs:710`), so
  mode-scope bindings do not resolve anywhere today, on any frontend.
- **GPU test seams**: `optimistic_insert_text` is a free function
  with an existing in-crate unit test
  (`pmacs-gpu/src/main.rs:7058-7104`), and it is the sole gate at the
  top of `optimistic_crdt_insert` (`:2077-2079`). The
  eligibility/routing layer above it lives in private `App`/`State`
  methods (`:2050`, `:2077`) with no constructible unit seam — the
  winit handler's fall-through to `send_key` is not unit-reachable.
  `pmacs-gpu` declares **no cargo features**
  (`pmacs-gpu/Cargo.toml`); `crdt` is a root-package feature only.
- **Modal contexts that consume Enter before the keymap**: isearch
  accept (`src/editor.rs:1831`), context-menu invoke
  (`src/editor.rs:1934`), completion-popup accept
  (`src/editor.rs:1984`), minibuffer accept
  (`src/minibuffer.rs:482`), query-replace prompt
  (`src/editor.rs:968`). These never reach the global keymap; a RET
  rebind cannot touch them. Acceptance pins them anyway.
- **Search staleness is asymmetric across edit paths, and staleness
  is only half-honored by consumers.** Accepted isearch matches
  deliberately stay highlighted "until the next edit marks them
  stale" (`src/editor_core.rs:861-865`). `apply_active_edit` — the
  path today's `buffer.newline` takes — honors that via
  `SearchStore::mark_stale` (`src/editor_core.rs:1184-1193`), and
  highlight producers suppress stale matches. But `search_step`
  navigates stored ranges without checking `is_stale`
  (`src/editor_core.rs:844`), and `search_match_summary` exposes
  stale counts to the n/m prompt (`src/editor_core.rs:719`).
  `SearchStore::set` clears staleness (`src/search.rs:100`), so any
  pattern re-run refreshes. Meanwhile direct buffer edits notify
  through `notify_buffer_edit` (`src/editor_core.rs:1207`), which
  refreshes views/overlays only — it never marks search state stale.
  Its **three** callers: the CRDT-op apply path (`src/daemon.rs:2133`),
  the general Lua mutator path (`notify_buffer_edit_to_windows`,
  `src/lua_bindings/mod.rs:1390-1395`), and the LuaHost
  errors-buffer append (`src/lua.rs:442`). A pre-existing
  stale-highlight/stale-step bug for all three, which a
  Lua-implemented RET would inherit on the most common keystroke.
- **A live search's origin is a raw byte offset.** `SearchSession`
  stores `origin: (BufferId, byte)` at `search_begin`
  (`src/editor_core.rs:740-753`); every recompute focuses from that
  unchanged offset (`src/editor_core.rs:803-810`), and cancel
  restores the cursor to it directly (`src/editor_core.rs:866`). No
  edit path translates it — an insert or delete strictly before the
  origin skews both the recompute focus and the cancel restore even
  when the match set is fresh. (`apply_active_edit` marks stale right
  there yet leaves the origin alone, so the other-frontend dispatch
  path carries the same pre-existing skew.)
- **Direct buffer edits do not reconcile window state** —
  `notify_buffer_edit` adjusts no cursors and no selections. An
  intercept that expands a replace past the cursor can leave
  `cursor > buf:len()` and a dangling selection; nothing downstream
  clamps. The daemon's optimistic-CRDT arm shows the canonical
  repair: right-gravity cursor translation through the effective edit
  (`src/daemon.rs:2103-2130` — `pos < start` → unchanged;
  `pos > pre-edit end` → `pos - old_len + inserted_len`; within →
  `start + inserted_len`). Note the mutators' returned triple
  `(start, end, inserted_len)` has `end` = the **pre-edit**
  replaced-range end (`src/lua_bindings/mod.rs:1246`); the post-edit
  end is `start + inserted_len`.
- **Intercepts may only move an edit, never change what it does.**
  M6.4 forbids kind-changing transforms and keeps the payload bytes
  immutable; same-kind position/range overrides are the entire
  surface (`src/lua_bindings/mod.rs:1061-1074`, `:1653-1657`).
  Consequences: an intercepted `insert` can only be **relocated** —
  the buffer always grows by the payload — while only an intercepted
  `replace` can **shrink** the buffer, by expanding its replaced
  range past the payload length.
- **Intercepts can switch buffers and windows.** Phase 2 of
  `run_managed_edit` runs the intercept chain with the registry
  borrow released (`src/lua_bindings/mod.rs:1287-1324`); an intercept
  body may legally change the active window or buffer. Any post-edit
  fix-up that blindly targets "the active window" can corrupt
  unrelated state. `pmacs.window.current()` exposes the active window
  id to Lua (`src/lua_bindings/mod.rs:10617-10622`).
- **No indent infrastructure exists.** No indent command; TAB inserts
  a literal `\t` (`buffer.tab`); there is no tabs-vs-spaces or width
  setting anywhere (config registry is a standing deferral); grammars
  drive highlighting only — no `indents.scm` in the repo. Hardcoded
  tab widths exist at **five** divergent sites: `TAB_WIDTH = 8` in
  four core renderers (`src/text_view.rs:35`, `src/highlight.rs:228`,
  `src/diag.rs:364`, `src/completion.rs:594`) and the GPU minimap's
  own leading-whitespace scan at width **4**
  (`pmacs-gpu/src/main.rs:5510`, `:5529-5531`; frontend-local, not
  reusable for editing).
- **The comment.lua pattern covers the Lua mechanics**: line scans
  over `buf:slice` (`line_start_before`,
  `builtin/runtime/comment.lua:47-57` — there is no `buf:line()` API;
  the scan is the idiom), `ed.cursor()/region()/goto_byte()/
  clear_selection()`, pcall'd mutators with the exact effective-edit
  triple check. `ed.goto_byte` clamps to buffer length.
- `C-j` is bound nowhere in `builtin/`.

## Decisions

### Q#AI1 — Lua feature + four named Rust touches

`builtin/runtime/indent.lua` (new), following the comment.lua module
shape. Rust changes, each deliberate and small:

1. **Loader entry** in `src/editor.rs` — `include_str!` + eval after
   the comment.lua entry (`:336`). Without it the module never loads
   and RET would resolve to an undefined command. (Defining the
   command in `builtin/commands/default.lua` was the alternative; a
   feature with helpers gets its own module per the comment.lua
   precedent.)
2. **Search-stale substrate fix** (Q#AI8): mark stale in the shared
   notify path, make the two stale-blind consumers fail closed, and
   translate the live origin.
3. **GPU eligibility fix**: remove the `ProtocolKey::Enter` arm from
   `optimistic_insert_text` so plain Enter round-trips through the
   daemon keymap, as the TUI already does. The arm's own documented
   justification (byte-identical to what the daemon would do) stops
   being true the moment RET means newline-and-indent. Tab keeps its
   arm: `buffer.tab` is still a plain `insert_char(9)`; the doc
   comment is rewritten Tab-only.
4. **Empty-selection core fix** (Q#AI9): the no-region arm of
   `insert_char_over_region` clears a lingering selection.

### Q#AI2 — Command + binding

One new command, `edit.newline-and-indent` (the Emacs name), and the
default keymap's RET line changes to it. `buffer.newline` keeps its
command definition and plain-newline role — the escape hatch via
`M-x` or a rebind — unchanged, with one deliberate exception: its
empty-selection behavior improves through the shared primitive
(the Q#AI9 fix).
No new default chord for plain newline in v1: the natural candidate
`C-j` is LF itself — the same terminal-ambiguity class as the C-/
undo note in the comment-toggle framing — so it's a named deferral,
not a casual bind.

### Q#AI3 — Indent = verbatim copy, clipped at the split point

On the line containing the split point:

    indent = bytes[line_start .. min(first_non_ws, split_point)]

where whitespace is space/tab only. Insert `"\n" .. indent` as ONE
edit; cursor lands after the indent.

- **Verbatim copy** (not width math): with no tabs-vs-spaces or width
  config in the editor, reproducing the existing bytes is the only
  policy that cannot be wrong about the file's convention. Tabs,
  spaces, and mixed runs all round-trip untouched.
- **Clip at the split point**: splitting *inside* the leading
  whitespace would otherwise double-indent the carried text. Clipping
  preserves the carried text's total indentation exactly —
  `··|··foo` → `··` / `····foo` (split after 2 of 4 spaces: new line
  still starts at column 4).
- **No language awareness in v1**: no extra indent after `{`, no
  dedent of `}`. Electric indent needs a per-language width opinion
  pmacs has no surface for (deferral, below).
- Whitespace-only lines copy what's before the split point; the
  abandoned line keeps its trailing whitespace (cleanup is a named
  deferral, not an accident).

### Q#AI4 — Region type-over preserved; selection always cleared

Region active → one `buf:replace(start, end, "\n" .. indent)`, indent
computed on the line containing the region start (clipped at region
start); cursor after the indent. Matches `insert_char_over_region`'s
single-`Replace` / one-undo-step / one-CRDT-op contract that the
GPU's selection round-trip relies on.

**The selection is cleared after every successful edit, region or
not.** A zero-length selection (anchor == cursor) reports no region,
so the edit takes the no-region path — but the edit itself advances
the cursor off the anchor, making the region nonempty **immediately**:
the very next self-insert type-overs the newline (`S-Left` at BOF,
RET, `x` → the `x` replaces the newline). Unconditional
`clear_selection()` closes that for this command. (Q#AI9 fixes the
same lingering-anchor bug in the core command family; this command
edits via `buf:insert`/`buf:replace`, not `insert_char_over_region`,
so it must clear its own.)

### Q#AI5 — Edit discipline: comment.lua contract + window-state safety

Snapshot first, edit second, guarded fix-up third:

1. **Snapshot** `pmacs.window.current()` and the target buffer id
   before the mutator runs.
2. `pcall`'d mutator; a rejecting intercept → status message, no
   edit, no throw, no dangling state.
3. **Guard**: intercepts run with the registry borrow released and
   may switch the active window or buffer (ground truth). If the
   active window or its buffer no longer matches the snapshot, skip
   all fix-up — report only; touching whatever window is now active
   would corrupt unrelated state.
4. **Cursor repair, one formula for both paths**: translate the
   pre-edit cursor through the *effective* edit with the established
   right-gravity shape (`src/daemon.rs:2103-2130`): `pos < start` →
   unchanged; `pos > pre-edit end` → `pos - old_len + inserted_len`;
   within the replaced range → `start + inserted_len`. Then
   `goto_byte` (which clamps). For the clean insert-at-cursor case
   this lands exactly at `start + inserted_len` — the normal
   after-the-indent position — so there is no special-cased "clean"
   placement to drift from the repaired one. Never jump to a fixed
   edit endpoint: an intercept that relocates the edit elsewhere in
   the buffer must not teleport the cursor there.
5. The effective triple is compared EXACTLY against the request; on
   deviation → status *"newline-and-indent altered by buffer
   intercept"*; the interceptor's **positional** result stands —
   kind and payload are immutable under the M6.4 contract (ground
   truth). Concretely: the plain path's `buf:insert` can only be
   relocated, so the buffer always grows and the `"\n" .. indent`
   payload lands wherever the intercept moved it; only the region
   path's `buf:replace` can shrink the buffer, via a same-kind range
   expansion. This is a named deviation from the comment.lua
   precedent (which skips fix-up entirely): skipping here would
   bless `cursor > buf:len()` whenever an expanded replace shrinks
   the buffer past the cursor.
6. `clear_selection()` (Q#AI4), under the same context guard.

### Q#AI6 — Consequences of the GPU change, named

- Plain Enter in the GPU costs one daemon round-trip before echo —
  parity with every TUI keypress and with the GPU's own
  selection/popup/busy Enter today.
- **Global and buffer-local RET rebindings become effective in the
  GPU frontend.** Concretely: buffer-list RET now visits the selected
  buffer instead of inserting a raw newline into the list (today's
  behavior — the optimistic path bypasses the buffer-local binding).
  This is an incidental bug fix, named and tested, not a side effect.
  **Mode-scope bindings are excluded from this claim**: dispatch
  resolves the keymap stack with an empty mode list
  (`src/editor.rs:710`), so they stay unresolved on every frontend
  until the mode system is wired.
- `this_command` after Enter in the GPU becomes
  `edit.newline-and-indent` (was `buffer.self-insert` via the
  exact-decode CRDT classification) — now consistent across
  frontends; kill chains break across a newline in both (already true
  in TUI).
- GPU unit tests locking Enter's optimistic eligibility flip to lock
  the round-trip.

### Q#AI7 — Chain/hook plumbing: nothing to build

Keybound RET rotates the command boundary and `buffer.after-edit`
fires from dispatch's revision check; `M-x edit.newline-and-indent`
gets both via `invoke_interactive`. Same Arc 2 substrate as
comment-toggle; the acceptance suite asserts the hook fires once
anyway.

### Q#AI8 — Search staleness: mark it, honor it, and keep the origin true

Three parts, all required:

1. **Mark**: `notify_buffer_edit` gains the same
   `SearchStore::mark_stale` call `apply_active_edit` already makes.
   One place, all three callers fixed: applied CRDT ops
   (`src/daemon.rs:2133`), general Lua mutator edits
   (`src/lua_bindings/mod.rs:1390-1395`), and the errors-buffer
   append (`src/lua.rs:442` — normally a no-op, unless the *errors*
   buffer itself carries accepted search state, in which case
   marking it stale is exactly right).
2. **Honor (fail closed)**: `SearchStore::step` returns `None` while
   stale — C-s / `search.next` stops navigating byte ranges that no
   longer exist instead of teleporting the cursor to them — and
   `search_match_summary` reports `(None, 0)` while stale, so the
   TUI/GPU n/m prompt cannot show counts for suppressed highlights.
   The consumers then match what the highlight producers already do.
   A stale **live** search un-sticks on the next pattern keystroke:
   the re-run calls `SearchStore::set`, which clears staleness
   (`src/search.rs:100`). Auto-recomputing from the stored query on
   step (instead of failing closed) is a named deferral — nicer UX,
   separate change.
3. **Translate the live origin**: a small `EditorCore` helper
   right-gravity-translates `SearchSession::origin` through the
   effective edit, invoked alongside `mark_stale` in **both**
   `apply_active_edit` and `notify_buffer_edit` (the formula is the
   `src/daemon.rs:2103-2130` shape; the dispatch path has the same
   pre-existing skew, and the two call sites already mirror each
   other for `mark_stale`). Without it, a fresh recompute focuses
   from a skewed offset and cancel restores the cursor to the wrong
   place whenever an external edit lands before the origin.

Together these close a pre-existing bug family — lingering
highlights, stale stepping/counts, and skewed origins after any
direct Lua edit or optimistic GPU edit — that a Lua-implemented RET
would otherwise put on the most common keystroke in the editor.

### Q#AI9 — Empty-selection type-over: fix the core arm, name the GPU residual

The lingering-anchor bug is not RET's alone (ground truth): the
no-region arm of `insert_char_over_region` leaves an empty selection
armed, so ordinary typing already type-overs its own previous
keystroke (`S-Left` at BOF, `x`, `y` → the `y` replaces the `x`).

**In scope**: `insert_char` returns success — today it catches the
intercept rejection and returns `()` (ground truth), so the caller
cannot distinguish a landed edit from a rejected one — and the
no-region arm clears the lingering selection **only on success**.
Clearing unconditionally would mutate state on a *failed*
self-insert/Tab/plain-newline, which the rejecting-intercept contract
forbids. This fixes `buffer.self-insert`, `buffer.tab`, and the
retained `buffer.newline` escape hatch for all daemon-dispatched
input, on both frontends' round-trip paths, and makes
`insert_char_over_region`'s existing "any selection is cleared" doc
claim true.

**Out of scope, named**: the GPU optimistic path inherits the bug
independently — its eligibility gate reads Selection *decorations*,
which an empty selection never paints (ground truth), and the
daemon's CRDT-apply arm clears no anchors. Optimistic typing over an
empty anchor therefore still arms a surprise selection that the next
round-tripped key can consume. Fixing that means teaching the CRDT
arm about anchors — deferred alongside the substrate window-state
reconciliation work, explicitly, not silently.

## Bets

1. **Verbatim copy is the right v1 policy** — no tabs-vs-spaces
   complaints are possible when we only ever reproduce what's already
   on the line.
2. **GPU Enter round-trip latency is imperceptible** — it matches the
   TUI's every keypress and the GPU's existing non-optimistic paths.
3. **Clip-at-split matches muscle memory** — mid-indent splits
   preserving total indentation is what vi/Emacs users expect; nobody
   files "my line double-indented".
4. **Fail-closed staleness has no workflow regressions** — a stale
   step no-op (until the pattern re-runs) surprises nobody, because
   the highlights it would have navigated are already suppressed;
   nothing legitimate consumes stale offsets once step and summary
   are guarded.
5. **Clearing a lingering empty selection on the core insert arm
   breaks nothing** — no workflow depends on an anchor surviving
   plain typing; mainstream editors deactivate the mark on edit.

## Deferred (named)

- Language-aware indent (electric `{`/`}`, tree-sitter `indents.scm`)
  — blocked on width/style config, which is blocked on the config
  registry deferral.
- TAB as reindent (`indent-for-tab-command`) — TAB stays a literal
  tab, and stays GPU-optimistic.
- Doc-comment continuation on newline — re-deferred from the
  comment-toggle framing; needs in-comment detection
  (grammar/comment-span work); its own framing after auto-pairing.
- Strip the abandoned line's trailing whitespace on split.
- A plain-newline default chord (`C-j` is LF — terminal-ambiguous).
- `C-o` open-line-with-indent.
- Substrate-level window-state reconciliation for direct buffer edits
  (cursor clamp / selection repair in the notify path, rather than
  per-command — including windows other than the acting one), and
  aligning comment.lua's transformed-intercept fix-up with Q#AI5's
  translate-and-clamp discipline.
- **GPU-optimistic empty-anchor residual** (Q#AI9): the daemon CRDT
  arm clears no selection anchors, and an empty anchor paints no
  Selection decoration to gate on — optimistic typing can still arm
  a type-over one keystroke later.
- Auto-recompute of a stale search from the stored query on step
  (Q#AI8 fails closed instead).
- Unifying the five hardcoded tab-width sites (4× core `TAB_WIDTH=8`,
  GPU minimap `4`) behind a real setting — config-registry work.
- Mode-scope keybinding resolution (dispatch passes `&[]` today) —
  the mode system's wiring, not this feature's.
- A constructible unit seam for the GPU routing layer above
  `optimistic_insert_text` (private `App`/`State` today).

## Acceptance (`tests/auto_indent_acceptance.rs`, dispatch-driven)

- RET at EOL of a space-indented line: new line carries the indent,
  cursor after it; tab-indented and mixed `\t··` lines round-trip
  verbatim.
- Mid-line split: `····foo|bar` → `····foo` / `····bar`, cursor
  before `bar`.
- Split inside leading whitespace: `··|··foo` → `··` / `····foo`
  (clip rule — no double indent).
- Zero-indent line and empty buffer: byte-identical to old
  `buffer.newline`.
- Whitespace-only line: new line copies it; abandoned line keeps it.
- Region active: ONE `Replace` — region replaced by `"\n" .. indent`,
  selection cleared; a single `buffer.undo` restores exactly.
- **Zero-length selection, command level**: `S-Left` at BOF, RET,
  `x` → `x` inserts plainly, the newline survives (selection cleared
  unconditionally). **Core level (Q#AI9)**: `S-Left` at BOF, `x`,
  `y` → the `y` does not replace the `x`; likewise via
  `M-x buffer.newline`. **Failure regression**: empty selection
  armed, self-insert rejected by an intercept → the anchor REMAINS
  (no state mutation on a failed edit). The GPU-optimistic variant
  is the named residual, not claimed here.
- One undo step in the plain case too.
- Intercept discipline: rejecting intercept → status, buffer
  unchanged, no throw; transforming intercept → reported, positional
  result stands. **Relocating transform** (plain path: the insert's
  `pos` moved — the only transform an insert admits) → payload
  inserted at the new site, cursor translated, NOT teleported.
  **Shrinking transform** (region path: the replace's range expanded
  past the payload, shrinking the buffer below the cursor) → cursor
  right-gravity-translated and clamped within `buf:len()`, selection
  cleared — the tests assert validity, not immobility.
  **Context-switching intercept** (switches window or buffer
  mid-edit) → fix-up skipped; this proves the *new* context is
  untouched — the original window's validity after such an intercept
  belongs to the substrate-reconciliation deferral, and the test does
  not claim it.
- **Search staleness** (Q#AI8):
  - isearch, accept, RET → accepted highlights marked stale; a
    direct `buf:insert` case pins the notify-path level.
  - Direct/remote edit during **active** isearch → `C-s` is a no-op
    (cursor unmoved, no jump to obsolete offsets) and the n/m summary
    reads `(None, 0)`; typing the next pattern char refreshes and
    stepping resumes.
  - **Origin translation**: during a live search, an external insert
    and a delete strictly before the origin → the next pattern
    character focuses relative to the translated origin, and cancel
    restores the cursor to the translated position — exercised via
    both the notify path (Lua mutator/CRDT) and the other-frontend
    dispatch path.
  - Post-accept edit → `search.next` no-op instead of stale
    navigation.
- `after-edit` fires exactly once per RET (keybound and `M-x`).
- Kill-chain break: `C-k`, RET, `C-k` → two ring entries.
- Modal contexts unaffected: isearch accept, query-replace prompt,
  context-menu invoke, minibuffer accept (the `m_x` helper exercises
  it), completion-popup accept.
- Buffer-list RET still visits via dispatch (now also reachable from
  the GPU frontend).
- `this_command()` after RET is `edit.newline-and-indent`.
- **GPU coverage, two named seams (not "end to end")**:
  - In-crate (`pmacs-gpu`): the existing
    `optimistic_insert_text_covers_plain_chars_enter_and_tab` test
    flips — Enter returns `None`, Tab still optimistic. This is the
    sole gate at the top of `optimistic_crdt_insert` (`:2079`), so
    classifier-`None` forces the handler's round-trip branch; the
    handler itself has no constructible unit seam (named deferral).
  - Root package (`--features crdt`): a synthetic attached replica
    plus the root TUI optimistic orchestrator drive pending
    optimistic self-inserts followed by Enter on an indented line,
    asserting the daemon dispatches `edit.newline-and-indent` and
    the resulting multi-byte CRDT op reaches the replica with
    correct final text — the daemon side of the wire path the GPU
    takes once it round-trips.
