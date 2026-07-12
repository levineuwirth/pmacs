# Auto-pairing — framing (Arc 2, editing table stakes)

Typing `(` should give `()` with the cursor between; typing `)` when
the next char is already `)` should step over it instead of doubling
it. Language-aware pair sets, conservative insertion predicate. Last
Arc 2 item; after it merges, Arc 2 closes and the compile-mode vs
themes decision discussion is due.

Roadmap: `docs/roadmap-2026-07.md` Arc 2 ("auto-pairing").
Revision 2: R1 findings — pair chars now route through dispatch
(peer-bound undo falsified the pure-reaction undo story), pair.lua
loads before lsp.lua (the sighelp path flushes didChange
synchronously mid-hook), rejected/transformed intercept outcomes
separated (the effective edit has already landed), the type-over
claim is path-qualified with a region guard, and the CRDT acceptance
gains a second replica plus undo cases for both routing models.

Revision 3: R2 findings — the dispatch route no longer claims global
chronological undo (an older source-peer edit still wins the TUI's
local undo arbitration), source self-inserts gain an exact ephemeral
typed-edit record so a relocated opener can never be inferred from
unrelated text, the context-switch/LSP limit is stated rather than
hidden by the cursor guard, non-typed acceptance now drives callbacks
that actually fire, and redo is exercised rather than merely named.

Revision 4: PR #110 round 1 — the typed-edit record pins the edited
buffer's revision after the completing edit and dies at dispatch end
if the command edited again (a redefined self-insert that replaces
the typed char can no longer leave a stale-but-clean record);
pair-set relevance is established before any provenance report
(transformed non-pair characters stay silent); pair entries parse as
exactly two codepoints (malformed entries are skipped entirely, never
partially honored); the record-capture seam is an opt-in test
facility, off in production; and the source-context-change *report*
is scoped as best-effort under the active-buffer edit-epoch limit —
an equal-revision context switch skips the fan-out and fails closed
silently.

## Ground truth (as of `7e127ab`)

- **Dispatch is keymap-first for printables** — `Char('(')` resolves
  through the keymap stack before the self-insert fallback
  (`src/editor.rs:713-717`, fallback `:748-761`); printable keys are
  bindable (the buffer list binds `n`/`p` buffer-locally,
  `builtin/commands/default.lua:407-423`).
- **But a key binding cannot carry pairing.** The GPU applies plain
  printables optimistically **even mid-line** — classifier
  `pmacs-gpu/src/main.rs:1525-1533`, eligibility `:2054-2079` — so a
  `(`-binding would never run for GPU typing. The TUI applies
  printables optimistically **at end of line** (the F19 paint
  constraint: mid-line inserts round-trip, EOL appends do not —
  `src/optimistic.rs:269-271`, contract `:168-200`). And a binding
  that inserted the pair atomically would break classification
  (below) even where it did run.
- **The typed-char substrate exists and has a working consumer.** The
  daemon classifies single-codepoint optimistic inserts as
  `buffer.self-insert` (exact byte decode,
  `src/daemon.rs:1985-2002`, `:2085-2087`), explicitly "for
  typed-char consumers (signature help; …)". `buffer.after-edit`
  fires on BOTH paths — dispatch (`src/editor.rs:772-776`) and the
  optimistic CRDT arm (`src/daemon.rs:2156-2169`) — with core
  borrows released, so a hook may edit the buffer. Signature-help
  auto-trigger (`builtin/runtime/lsp.lua:761-791`) is the in-tree
  template: `this_command() == "buffer.self-insert"` plus
  `char_before`; a paste can never trigger it.
- **An atomic 2-byte `"()"` insert breaks that classification**
  (`is_single_codepoint_insert` decodes the actual bytes,
  `src/daemon.rs:2079-2084` names signature help as the reason), and
  **intercepts cannot rewrite `(` into `()`** (M6.4: kind and payload
  immutable, `src/lua_bindings/mod.rs:1061-1073`, `:1657-1705`). Any
  pairing design must keep the opener a genuine single-codepoint
  self-insert.
- **Hook mechanics**: callbacks run in registration order
  (`src/hook.rs:240`, snapshot `:255-259`; fan-out `:291-297`);
  `buffer.after-edit` is **all-must-succeed**
  (`builtin/hooks/default.lua:41-45`) — one callback's error or
  return value cannot suppress the others. Hook-created edits do
  **not** re-fire `buffer.after-edit` (fired once per dispatch cycle
  / per optimistic op).
- **The LSP hook flushes synchronously mid-fan-out.** The after-edit
  callback in lsp.lua marks the buffer dirty, and — when the typed
  char is a signature trigger — calls `signature_help_quiet`, which
  calls `flush_did_change_for(rec)` **synchronously**
  (`builtin/runtime/lsp.lua:739-743`: "The server must see the
  character we just typed") before requesting. `flush_did_change`
  reads `buffer_text` at flush time and clears the pending entry
  (`:268-279`). Consequence: a closer inserted by a hook registered
  **after** lsp.lua's would miss that flush AND stay unsynchronized
  (no re-fire) until the next edit. Coalescing does not save this
  path; registration order does.
- **Undo is peer-bound, per frontend.** Loro's `UndoManager` binds to
  one peer at construction; each frontend's `BufferMirror` undoes
  only its own peer's ops (`src/buffer_mirror.rs:541-555`), and the
  daemon's `buffer.undo` covers only daemon-peer edits
  (`src/buffer.rs:1360-1380`). The TUI's undo key tries the mirror
  first and **round-trips to the daemon when the mirror has nothing
  to undo** (`src/optimistic.rs:238-247`). Consequence: a pair whose
  opener is a source-peer optimistic op and whose closer is a
  daemon-peer Lua edit is **not undoable coherently by either
  frontend** — the TUI removes the opener first (leaving `)`), the
  daemon removes the closer then unrelated daemon edits. Edits that
  route through dispatch are all daemon-peer.
- **The TUI's mirror-empty fallback is not chronological
  arbitration.** If the mirror has *any* older source-peer edit, its
  optimistic undo succeeds and the key never reaches the daemon
  (`src/optimistic.rs:230-247`). Thus `a` (optimistic) then `()`
  (daemon-routed) followed by the TUI's single-key undo removes `a`,
  not the closer. `C-x u`, which always dispatches, removes the
  daemon closer instead. Routing the pair's two edits to one peer
  makes their daemon-local order coherent; it does not merge that
  order with older source-peer history. Global chronological undo is
  existing cross-peer substrate work, not something pairing can
  truthfully claim to solve.
- **Mutator effects land before the caller sees them.** The effective
  edit is applied and only then reported as the returned triple
  (`src/lua_bindings/mod.rs:1184`, `:1246-1259`) — a transformed
  (relocated/expanded) edit has already happened; it can be
  *repaired around*, never *withheld*. Lua mutators move no cursors
  and reconcile no window state (PR #109 ground truth); the
  context-guarded right-gravity repair in
  `builtin/runtime/indent.lua:60-64`, `:100-121` is the established
  pattern.
- **The producing self-insert has the same intercept problem, and
  `after-edit` currently carries no payload.** `insert_char` advances
  the cursor from the requested position after `apply_active_edit`
  (`src/editor_core.rs:1738-1749`), even if an insert intercept moved
  the effective insertion. `this_command` proves only the input
  class; `char_before(cursor)` does not prove which character was
  typed or where it landed. Pairing therefore needs exact ephemeral
  source-edit provenance, not the signature-help heuristic alone.
- **A hook-inserted closer lands correctly by construction**: insert
  at the cursor, cursor stays before it (mutators move no cursors);
  the daemon's `CursorByte` re-grounds both frontends; the GPU
  applies incoming ops and rebases unconfirmed edits without moving
  `own_cursor` (`pmacs-gpu/src/main.rs:2463-2564`).
- **Broadcast ordering quirk on the optimistic path**: the after-edit
  hook fires (`src/daemon.rs:2167`) **before** the source opener op
  is queued for broadcast (`:2178`), so a hook-queued DaemonKey
  closer reaches **non-source replicas before the opener it depends
  on**. Loro is expected to buffer and converge; nothing pins that
  today.
- **Region-active typing**: CUA type-over consumes the region on the
  dispatch path. The GPU round-trips when a selection decoration
  exists; the TUI's optimistic gate consults no selection state (PR
  #109 named deferral) — a nonempty TUI selection ending at EOL can
  optimistically insert without consuming the region, and the daemon
  arm deliberately preserves nonempty anchors.
- **A callback may switch context, and later callbacks observe the
  switch.** The hook runner snapshots callbacks, not editor context;
  lsp.lua reads `pmacs.window.buffer()` when its callback runs. If a
  pair reaction's intercept switches from A to B, a local cursor guard
  can avoid touching B, but it cannot make the later LSP callback see
  A. Origin-pinned hook fan-out is named substrate work below.
- **Direct Lua mutation and plain `pmacs.command.invoke` do not fire
  `buffer.after-edit`.** A non-typed regression that merely calls
  either API is vacuous; the test must explicitly run the hook or use
  a production path such as paste which fires it.
- **No pair knowledge exists anywhere**; `pmacs.comment.strings`
  (`builtin/runtime/comment.lua:26-42`) is the per-language table
  precedent. No node-at-byte API on `pmacs.parse` (manual descent
  only; async/stale trees) — syntax-aware inhibit is not v1-viable.
- Rust uses `'` for lifetimes — pairing `'` per-language is a
  correctness matter, not taste.

## Decisions

### Q#AP1 — Carrier: after-edit reaction, with pair chars routed through dispatch

Two coupled decisions, each grounded in the constraints above:

**The reaction carrier** (unchanged from R1): on
`this_command() == "buffer.self-insert"`, the pairing hook reads the
exact ephemeral typed-edit record from Q#AP9 and reacts with a second
edit. This keeps the opener a genuine single-codepoint self-insert —
the classification signature help depends on — without guessing its
identity or position from surrounding buffer text.

**The routing change** (new, R1 finding): the built-in pair charset
`( ) [ ] { } " ' `` ` is **removed from both optimistic
classifiers** — the GPU's `optimistic_insert_text` and the TUI's
`classify_key` — so those chars always round-trip through dispatch.
Without this, the opener is a source-peer op and the closer a
daemon-peer op, and peer-bound undo makes the pair uncleanly
undoable on every frontend (ground truth). With it, both edits are
adjacent daemon-peer undo units. A daemon-routed undo removes the
closer and then the opener; the TUI's single-key optimistic undo does
the same **only when its older source-peer stack is empty**. This is
pair-local coherence, not global chronological arbitration (Q#AP5).
The exclusion also restores CUA type-over for pair chars on the TUI
(a round-tripped `(` consumes the region the optimistic path would
have skipped) and removes transient pair/skip paint from both
frontends.

Costs, named: one daemon round-trip per built-in-charset keystroke —
on the TUI only EOL appends change (mid-line already round-trips); on
the GPU all nine chars do. `'` and `` ` `` round-trip even in languages
whose sets don't pair them — uniform routing beats per-language
classifier state the frontends don't have. **User-extended pair
chars beyond the built-in nine still arrive optimistically**: they
pair correctly (the reaction fires either way) but their undo is
cross-peer-degraded — documented limitation, pinned in acceptance,
full fix deferred with the pre-existing mixed-history problem
(cross-peer chronological undo arbitration).

`builtin/runtime/pair.lua`; a `pmacs.pair.*` namespace.

### Q#AP2 — Pair sets: per-language table, conservative default

`pmacs.pair.sets` — the `pmacs.comment.strings` shape: language →
array of pair strings, plus a `default` entry used when the language
is unknown or has no entry (pairing is useful in scratch buffers).
An entry is EXACTLY two codepoints — opener then closer, multibyte
allowed (`"«»"`); malformed entries (trailing bytes, non-boundary
first byte) are skipped entirely, never partially honored (R4: a
`"()x"` typo must not turn `(` into `()x`):

- `default = { "()", "[]", "{}", '""' }` — no `'` (prose
  apostrophes), no backtick.
- `python`, `lua`, `javascript`/`typescript` (+react), `sh`/`bash`
  add `''`; javascript/typescript/markdown add `` `` `` pairs.
- `rust`, `c`, `cpp`, `go`, `zig` = default (lifetimes, char
  literals). Users opt in from init.lua: `pmacs.pair.sets.rust = …`.

### Q#AP3 — Insert-pair semantics (open char typed)

React when ALL hold:

- `this_command() == "buffer.self-insert"` **and** Q#AP9 returns a
  live typed-edit record for this callback (pastes, manual hook runs,
  and programmatic inserts have no record and never pair);
- **relevance first (R4)**: the record's exact typed codepoint is in
  the active pair set at all (opener or closer). Characters outside
  the set exit silently BEFORE any gate below can report — a
  transformed ordinary `a` is not auto-pairing's business;
- the record's buffer/window match the current context, its source
  edit is clean (effective triple equals the requested insert or
  replace), and the current cursor equals its recorded post-edit
  cursor. A relocated/expanded or context-switching source
  self-insert stands as the intercept produced it and gets no pair
  reaction. A non-clean triple reports *"auto-pair skipped: source
  self-insert transformed"*; a context/cursor mismatch reports
  *"auto-pair skipped: source context changed"*. This is the
  fail-closed answer to R2 finding 2. The context-change *report* is
  best-effort (R4): it requires the after-edit fan-out to run, and
  the dispatcher's active-buffer revision compare (the named
  buffer-aware edit-epoch deferral) skips the fan-out when a
  context-switching command lands on a buffer whose revision
  coincidentally equals the origin's — the record dies un-armed and
  pairing fails closed silently;
- **no active region survives the edit** (`ed.region() == nil`) —
  on the dispatch path type-over has already consumed and cleared
  it; a surviving nonempty region means the edit arrived through the
  TUI's selection-blind optimistic gate (custom chars only), where
  reacting would pile a closer onto an unconsumed region;
- the record's exact typed codepoint is an opener in the buffer's pair
  set (language via `pmacs.lsp.active_buffer_language()`, resolved
  **at callback time**, nil-guarded — pair.lua loads before lsp.lua,
  Q#AP7). `char_before` is not input provenance;
- **conservative predicate**: the char at the cursor is EOL,
  whitespace, or a closing bracket from the pair set — `foo|bar` +
  `(` gives `(bar`, never `()bar`;
- for symmetric pairs (quotes), the skip check (Q#AP4) runs first.

Reaction: one pcall'd `buf:insert(cursor, closer)`. Outcomes,
separated (R1 finding):

- **Rejected** (intercept threw): nothing landed; the opener stands
  alone; status *"auto-pair closer rejected by buffer intercept"*.
- **Transformed** (effective triple deviates): the edit has already
  landed wherever the intercept put it — the positional result
  stands. Report, then **context-guarded cursor repair**: with the
  window+buffer snapshot taken before the mutator, right-gravity-
  translate the pre-edit cursor through the effective edit and
  `goto_byte` (clamps); skip all repair if the intercept switched
  window or buffer. (The clean path needs no cursor motion at all —
  the asymmetry is deliberate: repair only on deviation, because the
  clean at-cursor insert must leave the cursor *before* the closer,
  which translation would not.)

### Q#AP4 — Skip-over-close, reactive

When Q#AP9's exact typed char is a closer in the pair set AND the char
at the recorded post-insert cursor equals it: one pcall'd
`buf:delete(cursor, cursor+len)`. Net text and cursor are exactly
Emacs's skip: `(|)` + `)` → `()|`; nested closers skip likewise;
`"` at `"|"` exits the string. With pair chars on the dispatch route
there is no transient duplicate to paint — the frontends never
locally applied the typed closer.

Same outcome separation as Q#AP3: rejected delete → the duplicate
stays (`())`), status, no retry; transformed delete → already
landed, report, context-guarded translate-and-clamp repair.

### Q#AP5 — Undo grain

Built-in pairs are two adjacent daemon-peer edits. On the GPU, and on
the TUI when the mirror has no older source-peer unit (or when the
user invokes the always-dispatched `C-x u`), daemon undo removes the
closer and then the opener. Still two steps, not one; a skip's daemon
undo restores the swallowed duplicate.

This does **not** make undo globally chronological. With optimistic
`a` already in the TUI mirror, then daemon-routed `()`, the TUI's
single-key optimistic undo removes `a` first because the mirror has a
local unit and never falls back. The GPU can undo the two daemon pair
units but cannot subsequently reach the older source-peer `a` through
daemon undo. Custom optimistic pair chars additionally split the pair
itself across peers. All three behaviors are pinned explicitly; the
general fix is a cross-peer chronological undo arbiter, deferred as
existing collaboration substrate rather than charged to pair.lua.

### Q#AP6 — Type-over composes on the dispatch path; wrap is deferred

For built-in pair chars (always dispatch-routed), typing an opener
over an active region type-overs the region, then the reaction runs
under the Q#AP3 predicate — Emacs-with-delete-selection semantics,
guaranteed for a clean, context-preserving source edit. A transformed
source edit fails closed under Q#AP9. For custom optimistic chars the
TUI selection gap persists; the Q#AP3 region guard suppresses the
reaction there rather than compounding the gap. **Wrapping** the region
in the pair is deferred (needs a command carrier plus the TUI
selection-gate fix).

### Q#AP7 — Load order: pair.lua BEFORE lsp.lua

Registration order is execution order (ground truth), and lsp.lua's
after-edit callback synchronously flushes didChange on the signature
trigger path — the closer must already be in the buffer when that
callback runs, or the server receives `(`-only text and the closer
stays unsynchronized until the next edit (hook edits don't re-fire).
So pair.lua's loader entry in `src/editor.rs` goes **before**
lsp.lua's (`:288`), with a comment naming this ordering contract;
all `pmacs.lsp.*` lookups inside the callback are lazy and
nil-guarded (lsp.lua defines them later in the load sequence).
Acceptance asserts the ordering by its observable: the **first**
didChange after an ordinary `(` carries `()`; when the closer is
position-transformed without switching context, that first didChange
carries the complete effective post-reaction text rather than an
opener-only intermediate.

Scope: if the reaction intercept itself switches active window or
buffer, later callbacks observe that new context. The pair callback's
cursor guard cannot repair hook-wide context, so the first didChange
guarantee does not extend to that legal-but-pathological case; A may
remain pending while lsp.lua observes B. This is recorded under
Deferred as origin-pinned after-edit fan-out and is no longer hidden
behind a cursor-only acceptance claim.

### Q#AP8 — Interactions, verified

- **Signature help**: the opener still classifies as self-insert on
  both routes; the closer insert moves no cursor, so `char_before`
  still reads `(`; the synchronous flush ships `()` (Q#AP7). Pinned
  with the `sighelp` fake-LSP mode. Same context-switch exception as
  Q#AP7.
- **Completion popup**: `(` never belonged to the popup's key set;
  `completion_popup_validate` runs after the hook fan-out
  (`src/editor.rs:778-783`) and judges the post-pair buffer when
  context is preserved. The Q#AP7 context-switch exception applies to
  completion as well.
- **Kill ring / boundaries**: the closer is a plain Lua mutator —
  stamps nothing, rotates nothing.
- **No recursion**: hook edits don't re-fire after-edit; the
  autosave-driven manual `pmacs.hook.run("buffer.after-edit")`
  (`builtin/runtime/autosave.lua:233`) is inert because Q#AP9 exposes
  no typed-edit record outside the Rust-owned hook boundary, even if
  `this_command` is stale.
- **Auto-indent**: RET inside `{|}` yields `{\n␣␣|}`; the electric
  closer-on-own-line split stays deferred with language-aware
  indent.

### Q#AP9 — Exact typed-edit provenance, ephemeral, one-shot, fail-closed

`this_command` remains the coarse input-origin signal used by existing
consumers, but pairing additionally requires a new
`pmacs.editor.take_typed_edit()` record. The record is per frontend,
is armed only while Rust is running the one `buffer.after-edit`
fan-out for that input, and can be consumed exactly once:

```text
{ buffer, window_id, codepoint, requested_start, requested_end,
  effective_start, effective_end, inserted_len, post_cursor, clean }
```

The dispatch path arms the record from the self-insert codepoint and
the requested Insert/Replace, then completes it from the effective
`Edit` returned by `insert_char_over_region`. The optimistic CRDT arm
already has both the decoded single codepoint and effective `Edit`, so
it builds the same record before firing the hook. A small EditorCore
outcome/report refactor is required because `apply_active_edit`
currently discards the effective range on its way back to
`insert_char`; payload immutability means the codepoint itself remains
authoritative.

The record additionally pins the edited buffer's revision immediately
after the completing edit — a producer-side postcondition, not
consumer surface (R4). At dispatch end, before arming, the revision
is re-read: if the command edited again after the self-insert (a
redefined `buffer.self-insert` that replaces or removes the typed
character while leaving the cursor in place), the record no longer
describes the buffer and dies un-armed. Note the switch-context case
is reachable only through such a redefined command: a
context-switching *intercept* cannot exist on the dispatch
self-insert path (the core borrow is held across it; the
borrow-released three-phase discipline belongs to the Lua-mutator
path the reaction uses).

The pair callback takes the record; later callbacks and a nested
manual re-run of `buffer.after-edit` see nil. The dispatcher/daemon
also clears any untaken slot immediately after the hook returns,
including error paths and the no-revision-change path. Plain
`pmacs.hook.run`, paste, programmatic mutation, and a stale
`this_command == "buffer.self-insert"` therefore see nil. Pairing
requires `clean == true`, matching buffer/window, and
`cursor == post_cursor`; otherwise it reports (pair-set characters
only, R4) and does nothing. This is deliberately narrower than
teaching every command to expose its edit: one producer class, one
consumer contract, and no persistent history. Record retention is
zero in production: the opt-in `pmacs.pair._capture_records` test
facility is the only way a consumed record outlives its fan-out (R4).

## Bets

1. **Nine round-tripped chars are imperceptible.** The TUI already
   round-trips every mid-line char; the GPU pays one local-socket hop
   on pair chars. The concrete return is real dispatch type-over, no
   optimistic duplicate paint during skip, and adjacent daemon-local
   pair units — not a false promise of global chronological undo.
2. **The conservative predicate kills the hate-mail cases** — no
   pairing before words, no apostrophe pairing in the default set.
3. **Uniform routing beats language-aware classifiers** — `'`
   round-trips in Rust for nothing, and nobody notices.
4. **`default`-set pairing in language-less buffers is wanted**, not
   surprising.

## Deferred (named)

- Wrap-region on opener with active selection (command carrier + the
  TUI selection-gate fix).
- Pair-aware backspace (delete both of a fresh empty pair).
- RET inside a pair → closer on its own line (with language-aware /
  electric indent).
- In-string/in-comment inhibit — needs a node-at-byte `pmacs.parse`
  binding and freshness guarantees.
- Undo amalgamation (pair = one undo step); cross-peer undo grouping
  and chronological arbitration (would un-degrade custom optimistic
  pair chars and mixed source/daemon history generally).
- Origin-pinned `buffer.after-edit` context. Today a legal intercept
  that switches window/buffer changes what all later callbacks see;
  pair.lua can guard its own cursor repair but cannot keep LSP and
  completion on the producing context without a hook-wide substrate.
- Balance-aware quote handling (odd/even counting).
- A per-buffer toggle (config-registry-blocked).

## Acceptance

`tests/auto_pair_acceptance.rs` (dispatch-driven):

- `(` at EOL → `()`, cursor between; mid-line before whitespace and
  before `)`; before a word char → no pair.
- Skip: `(|)` + `)` → `()`, cursor after; nested; `"` at `"|"` exits.
- Quotes pair under the predicate.
- Per-language: `'` pairs in `.py`, not in `.rs`; scratch pairs the
  default set.
- Set-entry parsing (R4): a malformed `"()x"` (and an overlong
  multibyte `"«»x"`) pairs nothing; a valid multibyte `"«»"` pairs
  and skips at byte-correct cursors.
- Non-typed provenance, with the callback actually exercised:
  production `FrontendEvent::Paste("(")` after a prior self-insert
  leaves a lone pasted opener; `buf:insert("(")` followed by explicit
  `pmacs.hook.run("buffer.after-edit")` also leaves it lone even when
  `this_command` was deliberately left as `buffer.self-insert`;
  `pmacs.command.invoke`d self-insert plus the same explicit hook run
  also has no record and no reaction.
- Type-over: region + `(` (dispatch route) → region consumed, then
  the predicate decides; selection cleared.
- Daemon undo grain pinned in the non-replica harness: `(` then undo →
  `(` alone; undo → empty; redo → `(`; redo → `()`; skip undo restores
  the duplicate.
- Intercepts: rejected closer → opener stands, status; **relocated
  closer** → landed at the intercept's position, reported, cursor
  translated not teleported; rejected skip-delete → duplicate stays;
  **expanded/relocated skip-delete** → landed, reported,
  translate-and-clamp. The **source self-insert** gets separate cases:
  relocated opener and expanded/relocated type-over produce exactly
  the intercept's positional result, Q#AP9 reports/skips, and no
  unrelated closer is inserted; a source context switch (via a
  redefined `buffer.self-insert` — the only legal producer, see
  Q#AP9) likewise fails closed, in BOTH revision shapes (R4): skewed
  revisions report "source context changed"; equal revisions skip the
  fan-out entirely and fail closed silently. A relocated **non-pair**
  character draws no auto-pair report at all (R4), and a redefined
  self-insert that edits again after the insert (replacing the typed
  char, cursor unmoved) kills the record — no `[)` (R4).
- Context-switching **reaction** intercept → pair cursor repair
  skipped, new context's text/cursor untouched by pair.lua. A probe
  callback registered after pair.lua observes the switched context,
  explicitly pinning (not concealing) the origin-context deferral; do
  not combine this case with the Q#AP7 first-didChange guarantee.
- Signature help: fake-LSP `sighelp` mode — auto-trigger still fires
  with pairing active, and the **first didChange after `(` contains
  `()`** when the reaction preserves context (the Q#AP7 ordering
  observable). A preserving-context relocated closer instead asserts
  that the first didChange contains the complete effective text.
- Hook fan-out: one fire per keystroke; the closer edit does not
  re-fire.
- Typed-edit lifecycle: `take_typed_edit()` yields the exact
  codepoint/effective triple once during both dispatch and optimistic
  self-insert hooks; a second take (including a nested manual
  after-edit run) is nil. It is also nil before/after the fan-out, for
  paste, for standalone manual hook runs, and after a rejecting edit.
  Two frontends cannot see or consume each other's slot. Exact-record
  observation goes through the opt-in `_capture_records` facility;
  with it off (production), no consumed record is retained anywhere
  (R4).

Classifier flips (in-crate): GPU `optimistic_insert_text` returns
`None` for the nine pair chars (test updated alongside Enter's);
TUI `classify_key`/orchestrator equivalents round-trip them (test
updated). Exercise both `Modifiers::NONE` and the `SHIFT` shapes real
keyboards use for `(){}"` so the test cannot pass only for synthetic
unshifted punctuation.

CRDT (`--features crdt`), **two replicas** (source + observer):

- Dispatch route: round-tripped `(` → both replicas converge to
  `()`; daemon cursor between; `)` skip converges; **undo/redo**:
  two daemon undos restore empty on both replicas, then two daemon
  redos restore `(` and `()` in order on both.
- Mixed history, dispatch route: source optimistically inserts `a`,
  then round-tripped `(` produces `a()`. On the TUI routing model the
  single-key optimistic undo removes `a` first (leaving `()`). From a
  fresh identical state, an always-dispatched `C-x u` removes the
  closer first (leaving `a(`). On the GPU routing model two daemon
  undos remove the pair but a further daemon undo cannot reach
  source-peer `a`. These are assertions of the named substrate limit,
  not frontend-equivalence claims.
- Optimistic route (custom pair char added to the set): source ships
  the opener op; the hook's DaemonKey closer is queued **before**
  the opener's broadcast — the observer receives the causally
  dependent closer first and must still converge (pairing and skip
  both). **Undo**: pinned degraded behavior — the source mirror's
  undo removes the opener, leaving the closer.
