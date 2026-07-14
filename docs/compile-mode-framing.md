# Compile-mode — framing (Arc 5 stage 1, terminal)

**Revision 13 — 2026-07-14. Status: implemented on branch
`compile-mode` (PR #113); revisions 7–13 fold in PR rounds 1–7.**

Revision 13 (PR #113 round 7, findings 1–2): overlay handle
attachment is validated — `attach_style_overlay` rejects a handle
whose recorded buffer differs from the target (its translator
follows edits to ITS buffer only; a cross-buffer render view showed
spans nobody maintains) and rejects a disposed handle (re-attachment
resurrected rendering without the translator); the disposed state is
shared across handle clones and both error messages point at
`add_style_overlay` as the fix. And `dispose()` no longer performs
the translator detach inside the optional `SharedCore` branch: the
window cleanup uses the core when present, while the detach goes
through the always-registered `SharedRegistry` — an
install-only/headless host previously got success with the
translator left attached (and paying per edit) for the buffer's
lifetime. Bites: cross-buffer + dispose-then-attach acceptance, and
a headless-host twin in the acceptance crate (the in-crate
registry-only unit vanishes under a mod.rs swap; the twin bites).

Revision 12 (PR #113 round 6, findings 1–3): render-view attachment
is idempotent and split-complete. Overlays expose an
`overlay_identity` (the span store's allocation address);
`Window::ensure_overlay` attaches a store-backed render view AT MOST
once per window — pre-fix every switch into the buffer blindly
pushed another copy onto EVERY matching window, so passive panes
accumulated duplicates, each cloning all spans and rescanning the
buffer per frame. A same-buffer split copies clonable overlays to
the new pane via `clone_for_split` (splits fire no switch hook and
started with an empty overlay list — the new compilation pane
rendered unstyled). The translator ignores pure no-op edits
(buffers deliberately broadcast them): pre-fix each interior no-op
split the containing span into adjacent fragments — unbounded list
growth, and a no-op at a UTF-8 continuation byte minted a
mid-codepoint span boundary. And the overlay handle has a teardown
path: `handle:dispose()` idempotently detaches the buffer-attached
translator and removes every window render view over its store —
the documented lifetime contract is one handle per buffer
incarnation (compile/REPL need no disposal; repeated creation on a
long-lived buffer must dispose retired handles or every edit keeps
paying for abandoned translators). Bites: split+bounce per-cell
acceptance (immediate post-split assert, before any switch could
heal the pane), no-op fragmentation Lua twin, dispose Lua twin;
units for genuine insertion, no-op ignore, split copy, and
ensure-once.

Revision 11 (PR #113 round 5, findings 1–3): style-span coordinate
translation belongs to the BUFFER, not to views — a new
`BufferStyleSpanTranslator` is attached to the buffer by
`pmacs.buffer.add_style_overlay`, sees every edit exactly once
(bypass writes, undo/redo, remote CRDT ops) regardless of window
count or visibility, and the window-attached `BufferStyleOverlay`
copies are render-only. Pre-fix each attached view translated the
shared store from its own `on_edit`: the duplicate attachment in
`start_run` (switch_buffer's after-switch hook already attaches)
made byte-delta rewrites shift later spans TWICE on the normal path,
splits multiplied further, and a hidden buffer shifted ZERO times.
Translation also preserves the untouched fragments of a partially
overlapped span (finding 2): left of the replaced range keeps its
styling, right of it shifts by the length delta, and only the bytes
actually rewritten lose theirs — `red abc, reset, CR, X` renders a
default X followed by red `bc`, where the old translation dropped
any overlapping span whole. Per-cell rendered assertions
(glyph, fg) pin both — `any_styled_cell` cannot see a wrong color on
the right cell. And the per-event whole-prefix line-start scan is
gone (finding 3): `slot.line_start` is tracked — advanced at every
\n, reset on recovery paths — making CR/BS/erase-line O(1); measured
2.52s → 0.67s on 2 MB + 3000 CRs (the pin is behavioral, not timed —
timing bounds flake on slow CI).

Revision 10 (PR #113 round 4, findings 1–2): CR rewrites are
COLUMN-counted and newline-segmented, not byte-counted — each
newline-free segment of a text event consumes one existing codepoint
per incoming codepoint (codepoints approximate columns; double-width
and combining characters count as one, the documented stance), and
LF is not an overwrite column: a newline arriving mid-line drops the
cursor to a fresh line and the stale remainder of the current line
survives in place (terminal semantics). Pre-fix, `abcdef\rX\n` wrote
`X\n` over `ab` — splitting the line and leaving `cdef` as a ghost
line the parser saw again at EOF — and `abc\ré` ate two ASCII
columns because é is two bytes (bites: single-batch, split-feed with
a split codepoint, and a CRDT twin — the segmented renderer makes
several byte-native edits per event, each on codepoint boundaries).
The ANSI parser now tracks the style the consumer LAST RECEIVED
(`emitted_style`): SGR changes inside the alternate screen advance
the internal style while their events are suppressed, so an ordinary
`?1049l` exit resynchronizes the effective style, and `finish()`
balances against the emitted style rather than the internal one — a
suppressed SGR reset inside the alt screen no longer strands the
consumer on stale pre-enter color (consumer-mirror units + a Lua
twin that bites).

Revision 9 (PR #113 round 3, findings 1–3): the CR/backspace
renderer is UTF-8-safe — overwrites consume WHOLE existing
codepoints (range end aligned forward past continuation bytes) in
ONE atomic replace of the complete text event, and backspace steps
to the previous codepoint boundary; pre-fix, byte-counted splits
left malformed bytes on the plain rope and made the byte-native CRDT
edit reject mid-codepoint ranges, aborting the pump after its events
were consumed (default + CRDT bites: é\rX, X\ré, é\bX).
`parser:finish()`'s reset is now OBSERVABLE: balancing events —
`AlternateScreenExit` for an unclosed enter, a default `SetStyle`
for a non-default running style — let consumers unwind mirrored
state from the event stream alone (unit applies events to consumer
state; Lua twin). The spawn-spec `stdin`/`group` fields are RAW
reads: metatable-provided fields are deliberately not honored (the
compile.lua posture) and a raising `__index` can no longer be
silently absorbed as `group = false`, disabling isolation.

Revision 8 (PR #113 round 2, findings 1–5): rule validation is a
stable, total snapshot — validated scalar fields are copied into
per-run plain tables via raw reads (rawget; metatable-provided
fields are deliberately not honored), and the container traversal is
itself protected, so neither post-run mutation of the user's rule
objects nor a hostile `__index` can alter an in-flight run or raise
through the pump mid-batch (traversal-raise semantics are
Lua-flavor-dependent: 5.2+ `ipairs` consults `__index`, LuaJIT reads
raw — both pinned); capture indexes must also be FINITE
(`math.floor(math.huge) == math.huge`); shell-command never touches
the rule table (no spurious warnings, no rule-container failure can
block a run that parses nothing); `parser:finish()` now RESETS the
parser — in-flight CSI/OSC/escape state and alt-screen suppression
included — so a post-finish feed parses a fresh stream (direct unit
tests plus a Lua-driven twin); three stale comments corrected.
Bites: four acceptance tests against pre-fix compile.lua, one
against pre-fix ansi.rs.

Revision 7 (PR #113 round 1, findings 1–10): stored coordinates must
be finite integers and both cursor walks are movement-bounded
(clamping at EOF/EOL) — an astronomical `%d+` capture can no longer
hang the editor; the grep panel gains the same immediate
`buffer.after-edit` recovery trigger as the compile slots; the rustc
rule uses the framing's `([^:]+)` spelling (paths with spaces);
rule capture indexes must be positive integers, all pattern captures
are honored (not just the first three), and a rule that names a
column its match didn't produce rejects the match; the marker/header
append helper is module-local (a user global could shadow it and its
error consumed the terminal event before forget); `stdin`/`group`
spec fields reject wrong Lua types instead of silently defaulting
(strict boolean check — mlua truthiness would coerce `"true"`);
resync also nils the public `line_start_byte` (total pre-marker
anchor invalidation includes the byte anchor); the inherited cwd
resolves through `pmacs.instance.identity().working_directory` so
the header always names a real path; a new `parser:finish()`
(additions #5) flushes a truncated UTF-8 sequence at process EOF as
U+FFFD before the exit marker; the built-in default rules are a
private deep copy, immune to in-place mutation of the public table.
Eleven bite tests, each verified failing against the pre-fix tree
via `scripts/bite`.

Revision 6 (responding to review round 5, findings 1–4, plus the
follow-up blocker audit): group-aware final drain now has an absolute
cancel deadline of `GROUP_TERM_GRACE` + one reader-poll interval even
when an escaped writer has already left the tracked group — queued
output gets one quiescent drain interval, then the reader is cancelled
and its retained join completes within one further interval, so the
setsid path cannot fall back to the old two-second synchronous stall
(supervisor additions #2); the external-edit ground truth now reflects
that round-trip buffers disable honest
frontend optimistic undo and accepted CRDT ops fire `buffer.after-edit`
— pump/anchor guards own only no-hook programmatic bypasses and defense
in depth (Q#CM2); acceptance dispatches all seven shipped undo/redo
chords, including raw-terminal `C-4` and both redo forms; after a
revision mismatch all pre-marker anchors are invalidated, while
diagnostics completed after the recovery marker may create a fresh,
reliable anchor epoch; every asynchronous producer, including grep
worker batches, checks the revision before writing so it cannot mask an
external edit (Q#CM2/Q#CM7). Acceptance remains 35 items.

Revision 5 (responding to review round 4, findings 1–6): the reap
ledger integrated into supervisor shutdown — outstanding groups are
force-killed at teardown, and restart is gated off once shut down
(supervisor additions #2); the ledger deadline enforced *inside*
`final_drain_runtime`'s loop so SIGKILL lands at the grace bound
instead of the drain timeout, with the residual synchronous stall
bounded and named (additions #2); command/menu undo recovered
*immediately* via a `buffer.after-edit` subscription rather than
waiting for a pump event that may never come (Q#CM2); on any
revision mismatch ALL in-buffer anchors are dropped — a revision
carries no edit range, and a same-length replace can move newlines
while every anchor stays in bounds (Q#CM2); ledger arming made
idempotent, earliest-deadline-wins (additions #2); the nix `poll`
feature added to the manifest touches (additions #2). Acceptance
grew to 35 items.

Revision 4 (round 3, findings 1–6): group-liveness reap ledger
decoupled from leader/reader state; reader detach replaced with
poll-based cancellable reads; the external-edit guard upgraded from
byte length to buffer revision (`buf:revision()`, additions #4);
all seven shipped undo/redo chords neutralized; sub-1 rule captures
fail closed; rule `severity` defined as an override.

Revision 3 (round 2, findings 1–7): leader-exit group reap before
drain/join; bounded TERM→KILL escalation (shutdown-time-fallback
deferral withdrawn); external-edit resilience for the streaming
buffer; grep kill-mid-search safety + root retention across
interactive supersedes; jump-back after-switch parity +
after-switch overlay re-attach; rustc/Python severity honesty.

Revision 2 (round 1, findings 1–11): pipe+ansi spawn rejection fixed
by moving ANSI parsing to Lua; stderr merged at the child boundary;
null-stdin spec option; process-group spawn + group-directed
SIGTERM; the `M-g` ordering contract corrected; tombstoned teardown;
coordinate normalization; unterminated-final-line parse; rule-table
validation; overlay handle discipline; interactive command contract.

Scope: `M-x`-style `compile.run` with a streaming `*compilation*`
buffer and error-regex navigation; grep-mode as an upgrade of the
existing `project.search` surface; `M-!` shell-command. One branch,
one PR — the three commands are thin entry points over one shared
machinery (streaming read-only output buffer + error-locations
source), and splitting them would ship the machinery twice.

This is roadmap Arc 5 stage 1 ("cheap, transformative"). Stage 2
(vterm / 2D grid) is explicitly out of scope. There is no
`spec/pmacs-tasks.tex` task for this feature — the spec predates the
July roadmap; the roadmap entry is the source of scope.

## Ground truth (as of `f0a05c5`)

Everything below was verified by reading the code, not the roadmap.

- **Process supervisor** (`src/process.rs`, T M4.4 done):
  `ProcessSpec { label, command, args, cwd, env, mode, restart,
  ansi_events }`; pipes or PTY; streaming 8 KiB chunks with a bounded
  channel; `Termination::{Exited{code}, Signaled{signal},
  Crashed{error}}` surfaced as events; signal/terminate/write_stdin/
  resize. Lua surface `pmacs.process.{spawn, signal, terminate,
  write_stdin, resize_pty, status, events_take, list, forget}`
  (`src/lua_bindings/mod.rs:7077-7232`). `EditorCore::tick_processes`
  drains the supervisor and fires the `process.after-tick` hook every
  frame (`src/editor.rs:450-458`).
- **Pipe-mode constraints** (rounds 1–2, all verified):
  - `ansi = true` + pipes is **rejected at spawn**
    (`src/process.rs:1200-1203`); structured ANSI events are
    PTY-only. Pipe consumers get raw byte events.
  - `drain_raw_output` coalesces **all stdout first, then all
    stderr** per tick (`src/process.rs:1501-1518`) — cross-stream
    arrival order is destroyed before Lua sees it.
  - Pipe children always get `Stdio::piped()` stdin whose writer is
    retained (`src/process.rs:1213-1229`); a child that reads stdin
    to EOF hangs forever.
  - `signal_target` sends group-directed signals (negative pid) for
    PTY children via the master's `process_group_leader`, but plain
    positive-pid signals for pipe children
    (`src/process.rs:607-620`).
  - **Leader-exit drain/join hazard:** on a terminal event,
    `poll_one` calls `final_drain_runtime`, which polls until the
    reader threads finish or `EXIT_OUTPUT_DRAIN_TIMEOUT` elapses
    (`src/process.rs:966-999`, `:1539-1554`); dropping the runtime
    then **joins** the readers (`src/process.rs:544-555`). The
    cancel flag only unwedges a reader stuck on a *full channel*; a
    reader blocked in `read()` on a pipe still held open by a
    surviving descendant (`sh -c "sleep 60 &"` exits immediately,
    the backgrounded sleep inherits fd1) burns the full drain
    timeout and then blocks the join — **the editor tick freezes
    until the descendant exits.** Group ownership must reap before
    drain/join, not just on explicit kill.
  - `forget` errors unless the process is already `Terminated`
    (`src/process.rs:1109-1123`); removing a pump entry before the
    terminal event is observed leaks the supervisor record.
- **ANSI** (`src/ansi.rs`): `Text` events never contain escape bytes
  (`:58-60`) — child output cannot corrupt a buffer. SGR →
  `SetStyle`; intra-line `CarriageReturn`/`Backspace`;
  `EraseToEol`/`EraseLine`; alt-screen contents suppressed; 2D cursor
  addressing parsed-and-discarded. `pmacs.ansi.parser()` gives Lua a
  stateful parser for raw bytes — the REPL's `append_output` feeds it
  exactly this way (`builtin/packages/repl/init.lua:382-384`).
- **The REPL package is the proven consumer**
  (`builtin/packages/repl/init.lua`): single `process.after-tick`
  subscription walking a proc-id-keyed pump registry (`:765-773`);
  `append_events` applies text/set_style/carriage_return/backspace/
  erase events to the buffer (`:386-427`); style overlay spans via
  `pmacs.buffer.add_style_overlay` + `overlay:add(start, end, style)`
  (`:190`, `:536-541`); exit markers with `_on_exit` as the single
  teardown point calling `pmacs.process.forget` only after the
  terminal event (`:782-822`, the M6.9 no-leak discipline).
- **Style overlay window semantics** (`src/lua_bindings/mod.rs:
  2907-2933`, `:1756-1799`): `add_style_overlay` attaches a
  buffer-level `BufferStyleSpanTranslator` (coordinate translation,
  exactly once per edit — Revision 11) plus render-only window
  overlays on windows *currently showing* the buffer; buffer
  switches clear window overlays; `attach_style_overlay(buf,
  handle)` re-attaches the render view, idempotently per window via
  the store identity, and same-buffer splits copy the render view to
  the new pane (Revision 12). Attachment validates the handle:
  wrong-buffer and disposed handles are rejected with messages
  pointing at `add_style_overlay` (Revision 13). The handle has
  `add`, `clear`, `clear_before`, `spans`, and idempotent `dispose`
  (teardown of the translator + every window render view; the
  translator detach rides the always-registered registry, not the
  optional editor core; one handle per buffer incarnation needs no
  disposal).
- **Buffer-switch hooks**: `buffer.after-switch` exists and fires on
  the ordinary switch paths (recentf subscribes,
  `builtin/runtime/recentf.lua:54`). **`pmacs.editor.jump_back` does
  not fire it**: the binding calls straight into
  `EditorCore::jump_back`, which switches via
  `switch_active_buffer` with no hook (`src/editor_core.rs:736-751`,
  `src/lua_bindings/mod.rs:10795-10801`) — so the RET → `M-,`
  round trip sheds any per-window overlay attachment.
- **Read-only + navigable panels** (`builtin/runtime/listview.lua`,
  Q#P1/P3/P6): intercept that `error()`s makes a buffer read-only;
  the module's own writes pass `{ bypass_intercept = true }`
  (`:60-61`, `:101-103`); buffer-local keymap; previous-buffer
  capture + `q` restore (with a never-capture-a-panel guard,
  `:118-124`); `pmacs.buffer.set_round_trip_input(buf, true)`
  (`:106`). **Caveat:** listview re-renders wholesale (`:50-62`) —
  wrong for a streaming compile. Display convention is
  switch-in-place (Q#P2: the GPU cannot show splits).
- **Keymap constraints**: duplicate bindings are **rejected**, not
  replaced (`KeymapError::DuplicateBinding`,
  `src/keymap_tree.rs:238-245`); `pmacs.keymap.unbind` exists
  (`src/lua_bindings/mod.rs:5511`). `M-g n`/`M-g p` =
  `diag.next`/`diag.previous` (`builtin/runtime/lsp.lua:2148-2149`;
  `diag.next` **wraps**); `M-g g`/`M-g M-g` = goto-line (editops,
  #111). Free: `M-g M-n`, `M-g M-p`, `` C-x ` `` (the parser accepts
  a single-character chord), `M-!`, `M-&`. Runtime chunks load in a
  fixed sequence in `src/editor.rs` (`:184-377`); lsp.lua at `:301`.
- **Buffer lifecycle hook**: `pmacs.buffer.on_removed(buf, cb)`
  exists with once-only semantics (`src/lua_bindings/mod.rs:2818`,
  autosave consumer `builtin/runtime/autosave.lua:147`).
- **Undo / external-edit routing (Revision 6 correction):** undo does
  bypass Lua intercepts, but these generated buffers are marked with
  `pmacs.buffer.set_round_trip_input(buf, true)`. While one is active,
  `EditorState::dispatch_idle()` returns false (`src/editor.rs:622-639`),
  and both semantic frontends gate their entire optimistic edit path
  on that signal (`src/attach.rs:835-845`,
  `pmacs-gpu/src/main.rs:2064-2067`). An honest frontend's undo key
  therefore round-trips and reaches the buffer-local binding; it is
  not a production no-dispatch escape. Even an accepted replica
  `CrdtOp` fires `buffer.after-edit` (`src/daemon.rs:2225-2238`). The
  remaining no-hook exposure is programmatic mutation (including a
  caller deliberately using `bypass_intercept`) plus defense against
  stale/malformed clients; pump/anchor revision checks make those
  paths total. The shipped undo surface is **seven chords**
  (`builtin/keymaps/default.lua:126-137`): `C-/`, `C-_`, `C-4`,
  `C-x u` → `buffer.undo`; `C-?`, `C-S-_`, `C-x r` →
  `buffer.redo` (terminal translation aliases), plus `M-x
  buffer.undo` and menu invocation as non-key dispatch paths.
- **Buffer revision**: `Buffer::revision()` exists core-side
  (`src/buffer.rs:462`), bumped by the shared edit path
  (`src/lua_bindings/mod.rs:1317` names "the revision bump") —
  edits, undo, and redo all increment it. It is **not** exposed to
  Lua yet; the auto-pair arc's `buf:path()` method is the precedent
  for adding a query method to `BufferIdLua`.
- **Jump idiom** (`builtin/runtime/lsp.lua`): `visit_location`
  (`:1451-1467`) = `push_jump` → pcall `find_or_open` (dropping the
  pushed jump on failure) → `move_active_cursor_to(line, col)`,
  which consumes **0-based** line/col (`:879-888`). Known residual
  (`:867-877`): the col walk steps per codepoint while col is a byte
  offset — multi-byte lines land the cursor short. Inherited.
- **Grep**: `pmacs.workers.grep{root, pattern}` (T M3.6,
  `builtin/runtime/async.lua:292-324`) streams structured matches:
  `file` is **relative to the search root**, `line` is **1-based**,
  `match_start`/`match_end` are **0-based byte offsets within the
  line** (`src/async_runtime.rs:117-135`). Streams expose
  `:cancel()` (cooperative, `builtin/runtime/async.lua:155`).
  Existing consumer `M-x project.search`
  (`builtin/commands/default.lua:647-718`): formats lines into
  `*search-results*`, supersedes via `supersede = "search"` +
  stream-id late-batch rejection, root defaults to `"."`; its
  `on_batch`/`on_close` callbacks **write through a retained buffer
  handle with no `is_valid()` guard** (`:693-703`) — killing the
  buffer mid-search makes every subsequent batch raise.
- **Prompting**: `pmacs.minibuffer.read` accepts `prompt`, `initial`,
  `history`, `source`, `source_root`, `on_accept`, `on_cancel`
  (`src/lua_bindings/mod.rs:11297-11330`).
- **Project root**: `pmacs.project.detect(path)` →
  `{root, kind, language_id}` or nil (`src/lua_bindings/mod.rs:
  9644-9674`).
- **Supervisor shutdown** (`src/process.rs:1129-1165`): SIGTERM to
  every managed process → poll `tick()` while `any_running()` within
  the grace period → SIGKILL leftovers → bounded reap loop. It knows
  nothing beyond managed records — a group survivor tracked only by
  the reap ledger would be discarded at `Drop` — and the `tick()`
  calls inside it perform restart accounting, so a
  `restart = always` process can respawn mid-teardown unless gated.
- **Command-path edits fire `buffer.after-edit`**:
  `with_after_edit_check` (`src/editor.rs:845-864`) wraps the
  minibuffer-accept (`M-x`), menu-invoke, and unified-paste routes,
  firing the hook when the *active* buffer's revision changed —
  exactly the paths key rebinding cannot reach. Hook edits don't
  re-fire the hook (established in the auto-pairing arc), so a hook
  callback may safely append to the buffer it was notified about.
- **Process-group std API**:
  `std::os::unix::process::CommandExt::process_group(0)` is safe
  (stable since Rust 1.64) — a new process group at spawn with **no
  `unsafe`, no trampoline**, preserving the `forbid(unsafe_code)`
  posture. `nix`'s `kill(Pid::from_raw(-pgid), sig)` and a
  signal-0 liveness probe are likewise safe. **Manifest caveat:**
  the workspace enables nix features `signal, user, fs, term,
  socket` only (`Cargo.toml:159`) — `nix::poll` is feature-gated,
  so the cancellable readers require adding `poll`.
- **Test harness precedent**: `run_with_pump`
  (`tests/m6_5_repl_acceptance.rs:87-115`) drives
  `editor.tick_processes()` until a Lua predicate holds.

## Supervisor / binding additions (the arc's only Rust changes)

No protocol change, no frontend change.

1. **`stdin = "null"`** (`ProcessSpec`, pipes-only) — spawn with
   `Stdio::null()`: no writer thread, the child reads EOF
   immediately (zero-race; strictly better here than exposing
   `close_stdin` post-spawn). `write_stdin` on such a process errors
   with the existing stdin-not-piped message. Rejected under PTY.
   The Lua spec parsing for `stdin` and `group` treats wrong types
   as HARD errors (Revision 7) — a silently-defaulted `stdin = true`
   or `group = "true"` would undo exactly the guarantees the fields
   carry; `group` is matched as a raw Value because mlua's bool
   conversion applies Lua truthiness. Both fields are RAW reads
   (Revision 9): a spec table is plain data, metatable-provided
   fields are deliberately not honored (the compile.lua rawget
   posture), and a raising `__index` cannot be silently absorbed
   into `group = false`, quietly disabling isolation.
2. **`group = true`** (`ProcessSpec`, pipes-only) — a full lifecycle
   policy, not just a spawn flag:
   - **Spawn**: `process_group(0)` — the child leads a fresh group.
   - **Signal**: `signal_target` returns `-pid` for flagged pipe
     processes, mirroring the existing PTY branch
     (`src/process.rs:607-615`).
   - **Group-liveness reap ledger (Revision 4 — replaces the
     Revision-3 conditions)**: escalation is driven by **group
     liveness itself**, not by leader or reader state. Whenever a
     group process is TERMed (kill, supersede) *or* its leader's
     terminal event is observed (normal exit included), the
     supervisor SIGTERMs the group and records `{pgid, deadline}` in
     a reap ledger that is independent of the process record. Every
     tick probes each ledger entry with `kill(-pgid, 0)` (safe nix):
     ESRCH → group gone, entry dropped; still alive past the
     deadline (`GROUP_TERM_GRACE`, 500 ms) → SIGKILL the group. The
     Revision-3 formulation had a verified hole: a leader that exits
     while a TERM-ignoring descendant redirects or closes its
     stdout/stderr produces a terminal event AND finished readers,
     so conditions keyed on either would never escalate — the
     ledger's liveness probe catches exactly that survivor. The
     ledger outlives `forget`, so `process.list()` returning to
     baseline and group death are independently guaranteed. Residual
     (named): between TERM and the KILL deadline a fully-recycled
     pgid could theoretically absorb the KILL; the window is one
     grace period and pids allocate forward — accepted.
   - **Idempotent arming (Revision 5)**: arming inserts a ledger
     entry only if the pgid is absent — earliest deadline wins.
     Repeated `terminate` calls (or kill-then-supersede races) must
     not push the SIGKILL bound out; a plain `insert` would reset
     the 500 ms clock on every TERM. Pinned by a unit test issuing
     repeated TERMs and asserting the original bound.
   - **Shutdown integration (Revision 5)**: `shutdown()` currently
     polls only `any_running()` and would discard a pre-deadline
     ledger at `Drop` (`src/process.rs:1129-1165`) — a leader that
     exits promptly while its TERM-ignoring group member survives
     would leak that member at editor exit. Two changes: (a)
     shutdown resolves the ledger — outstanding groups are SIGKILLed
     immediately (editor exit owes no grace) and probed to ESRCH
     within the existing bounded reap loop; (b) `maybe_restart` is
     gated off once `shut_down` is set, so `restart = always`
     processes cannot respawn mid-teardown. Both pinned by
     supervisor unit tests (a drop-twin of the ledger acceptance).
   - **Leader-exit ordering + bounded in-drain enforcement (Revision
     5/6)**:
     the group TERM (and ledger arming) happens **before**
     `final_drain_runtime`, closing the verified freeze: a shell
     that exits leaving `sleep 60 &` holding the merged pipe would
     otherwise burn the full drain timeout and then block the reader
     join (`src/process.rs:544-555`, `:1539-1554`). Arming alone is
     not enough: `final_drain_runtime` blocks the tick for up to two
     seconds, and no other tick runs to probe the ledger — a
     TERM-ignoring descendant holding fd1 would get its SIGKILL ~2 s
     late. The drain loop therefore **enforces ledger deadlines from
     inside each iteration**. At the grace bound it SIGKILLs a
     surviving group. If the original pgrp probes ESRCH earlier, the
     drain gives readers one quiescent `READER_SEND_POLL_INTERVAL` to
     flush already-read and kernel-buffered bytes; new data resets that
     quiescence window. Independently, **no group reader drain may pass
     the absolute cancel deadline** of the original ledger deadline plus
     one poll interval — reaching that deadline cancels the readers even
     if the liveness ledger has not yet observed ESRCH. The ledger remains
     alive and keeps probing the group after the process record/runtime
     are gone. The drain sets the shared cancel flag, drains the channel
     once more, and lets the retained join complete within one further
     poll interval.
     This closes the Revision-5 residual: a setsid'd fd holder must not
     fall through to `EXIT_OUTPUT_DRAIN_TIMEOUT` merely because its old
     group is already gone. Honest trailing output gets a bounded
     flush; escaped output may be truncated. Residual (named): the
     synchronous stall is bounded by approximately
     `GROUP_TERM_GRACE + 2 * READER_SEND_POLL_INTERVAL` (~600 ms with
     the proposed constants), plus ordinary scheduler tolerance, not
     eliminated; a fully tick-driven drain remains deferred.
   - **Cancellable readers (Revision 4 — replaces the Revision-3
     detach)**: for `group = true` processes, reader threads use
     poll-based reads (`nix::poll`, safe; FD set nonblocking via
     `nix::fcntl` — no `unsafe`) with a cancellation check each
     poll interval and again immediately after poll, before any
     read/send. `RuntimeHandles::Drop` keeps its join, which
     now completes within one interval regardless of who still
     holds the write end. Dropping a `JoinHandle` merely detaches —
     the Revision-3 "join skipped past a hard cap" traded the freeze
     for an unbounded thread+FD leak across repeated runs, and is
     retracted. Existing non-group consumers (REPL, LSP) keep the
     blocking readers they were tuned on (the M6.6 ingest gate);
     unifying is a named deferral. Manifest touch: add `poll` to
     the nix feature list (`Cargo.toml:159` — currently absent,
     Revision 5).
   - **Escape hatch (documented behavior, not a bug)**: a descendant
     that calls `setsid` leaves the process group and is deliberately
     not reaped — the standard daemonization path still works. If it
     holds the merged fd1 after the original group is gone, the
     Revision-6 quiescence/cap rule cancels the reader rather than
     waiting the two-second drain timeout. Trailing escaped output is
     truncated; threads join, FDs close, and nothing accumulates.
     Pinned by a supervisor unit test that asserts both **bounded
     latency and resource reclamation**: repeated spawn/reap cycles
     return through the retained joins within the bound, with a
     per-runtime `cfg(test)` active-reader counter back to zero before
     fixture cleanup. Join return plus that counter is
     the deterministic proof that the reader threads ended and their
     owned read FDs dropped; a process-global thread/FD count would be
     racy under Rust's parallel test runner. The fixture
     records and explicitly kills the escaped pid afterward — the
     supervisor deliberately does not own it.
3. **`jump_back` after-switch parity (Revision 3)** — the
   `pmacs.editor.jump_back` binding fires `buffer.after-switch` when
   the jump actually changed the active buffer, matching the
   ordinary switch paths. This is a parity fix with observable
   side benefits (recentf now records `M-,` re-visits); it is what
   lets overlay re-attachment ride one hook instead of special
   cases. Behavior change called out in the PR body.
4. **`buf:revision()` query method (Revision 4)** — exposes
   `Buffer::revision()` (`src/buffer.rs:462`) on `BufferIdLua`,
   the `buf:path()` precedent. Needed because byte length is not an
   edit-integrity token: the CR-overwrite rendering path performs
   same-length replaces, so undoing one changes content without
   changing length. Revision increments on every edit, undo, and
   redo.
5. **`AnsiParser::finish()` + `parser:finish()` (Revisions 7–8)** —
   stream-end finalization: the feed-boundary contract deliberately
   buffers an incomplete UTF-8 sequence for the next feed, but at
   process EOF there is no next feed — `finish()` emits U+FFFD for
   the pending prefix (the same posture an interrupting control byte
   gets) and flushes the text run, **then fully resets the parser
   (in-flight CSI/OSC/escape state, alt-screen suppression, and the
   running SGR style): a feed after finish parses a NEW stream**
   rather than continuing a pre-EOF escape, staying suppressed, or
   inheriting stale color. The reset is OBSERVABLE (Revision 9):
   balancing events — `AlternateScreenExit` for an unclosed enter, a
   default `SetStyle` — let consumers that mirror parser state
   unwind from the event stream alone. The balance point is the
   style the consumer LAST RECEIVED, not the internal style
   (Revision 10, `emitted_style`): SGR changes inside the alternate
   screen advance `current_style` while their events are suppressed,
   so a suppressed reset would otherwise strand the consumer on
   pre-enter color with nothing to compare unequal. The same field
   drives an ordinary `?1049l` exit, which resynchronizes the
   effective style whenever suppressed SGR changes drifted it.
   Compile-mode calls `finish()` once at the terminal event, before
   finalizing the pending line.

## Decisions

### Q#CM1 — Placement: one runtime module, one PR, ordered after lsp.lua

New `builtin/runtime/compile.lua` owning: the streaming output-buffer
machinery, the error-rule table, the error-source dispatcher
(`pmacs.errors`), and the `compile.*` / `shell.*` commands.

**Ordering contract:** compile.lua MUST load after lsp.lua in the
`src/editor.rs` chunk sequence, with a comment naming the contract.
Reason: it takes over `M-g n`/`M-g p`, and duplicate bindings are
rejected (`keymap_tree.rs:238`) — the takeover is `unbind` × 2 then
`bind` × 2, which requires lsp.lua's bindings to exist first.
(Placed last in the runtime sequence, after indent.lua.) Its
`process.after-tick` subscription is ordering-independent — it pumps
only its own proc-id-keyed registry, disjoint from the REPL's.

The grep upgrade edits `project.search` in
`builtin/commands/default.lua` in place; it reaches the shared
machinery through the `pmacs.errors`/`pmacs.compile` globals at
invoke time, so commands/runtime load order stays irrelevant for it.

### Q#CM2 — The `*compilation*` buffer: streaming append, intercept read-only

- Named `*compilation*`, reused across runs. Each run resets it to a
  header (command + resolved cwd), then streams output, then an exit
  marker (REPL's `format_exit_marker` shape).
- **Read-only via an erroring intercept; module writes pass
  `{ bypass_intercept = true }`** (the listview idiom — compile
  buffers have no user-editable region, so the REPL's `_self_write`
  machinery is unnecessary).
- **Streaming append via a Lua-side ANSI parser.** Spawn with
  `ansi = false` (structured events are PTY-only); feed raw byte
  events through a per-run `pmacs.ansi.parser()` — the REPL's
  `append_output` path — applying events the way its `append_events`
  does: `text` appends at the tracked output position, `set_style`
  updates the running style (a style-overlay span per non-default
  emission), `carriage_return`/`backspace`/`erase_*` get the
  intra-line treatment so progress bars collapse. Alt-screen
  suppression comes free. One parser, one running style, one output
  position — coherent because the child delivers **one merged
  stream** (Q#CM3). **Overwrite semantics (Revision 9→10):**
  overwrites are COLUMN-counted and newline-segmented — each
  newline-free segment of a text event consumes one existing
  codepoint per incoming codepoint (codepoints approximate columns;
  double-width and combining characters count as one, the documented
  stance), LF is never an overwrite column (a mid-line newline
  appends past the surviving stale remainder — terminal semantics —
  instead of being written INTO the line, which split it and left
  the remainder as a ghost line), and backspace steps to the
  previous codepoint boundary. Every buffer edit keeps both range
  ends on codepoint boundaries — Revision 9's UTF-8 invariant, held
  per-segment now rather than by one atomic replace: segments carry
  complete scalars, so the rope is valid UTF-8 after every step and
  the byte-native CRDT edit never rejects a range. `out_pos` stays
  on codepoint boundaries by induction. History: pre-Revision-9
  splits left malformed bytes and aborted the CRDT pump after
  events_take had already consumed the batch; pre-Revision-10 the
  byte count split lines (`abcdef\rX\n` → ghost `cdef` line) and ate
  columns under multibyte overwrites (`abc\ré` → `éc`).
- **External-edit resilience (Revision 4–6).** Generated-buffer keys
  round-trip, so honest frontend optimistic undo is not an escape from
  the local bindings; accepted replica ops also fire
  `buffer.after-edit`. "Only module writes move the buffer" is still
  not a hard invariant, however: programmatic Lua may deliberately
  bypass the intercept, and the revision guard is cheap defense in
  depth against stale/malformed clients. Two layers:
  1. *Shipped key aliases neutralized* (the Revision-3 "dispatch
     vector closed" claim is retracted — it covered two of seven
     chords): **all seven** shipped undo/redo bindings are rebound
     buffer-locally in every generated buffer to a status no-op
     ("generated buffer: undo disabled") — `C-/`, `C-_`, `C-4`,
     `C-x u`, `C-?`, `C-S-_`, `C-x r`
     (`builtin/keymaps/default.lua:126-137`). `M-x buffer.undo` and
     menu invocation remain dispatchable by design (rebinding
     cannot reach them) and are **guard-recovered** by layer 2.
  2. *Everything else survived by the revision guard*: the module
     records `buf:revision()` (additions #4) after each of its own
     writes and checks it at **three trigger points** —
     (a) *immediately*, via a `buffer.after-edit` subscription:
     `with_after_edit_check` (`src/editor.rs:845`) fires the hook
     for the command/menu edit routes that rebinding cannot reach;
     accepted replica `CrdtOp`s reach the same hook. Hook edits don't
     re-fire the hook, so the callback can append the marker safely.
     Without this, an `M-x
     buffer.undo` after a *completed* run — no pump event, no
     byte-anchor use ever coming — would leave the buffer corrupted
     indefinitely with no marker (round-4 finding 3);
     (b) before every asynchronous producer write — process-pump
     appends and terminal markers for compile/shell, plus grep
     `on_batch`/`on_close` writes (catches a no-hook programmatic bypass
     while streaming and prevents the producer's own next write from
     masking it by advancing the expected revision; also defense in
     depth for remote input);
     (c) before any byte-anchor use (RET, `n`/`p`).
     A mismatch triggers a resync: output position clamps to
     `buf:len()`, pending-line state resets, exactly one
     `\n[output desynced by external edit]\n` marker is appended, the
     expected revision advances to that marker write, and streaming
     (if live) continues at the end. Both newlines are load-bearing:
     post-recovery output must not share a line with damaged pre-marker
     content or with the marker itself. Revision, not byte
     length: the CR-overwrite path emits same-length replaces, so an
     undone overwrite changes content while preserving length — a
     length guard provably misses it (round-3 finding 3). **Anchor
     invalidation is total (Revisions 5/7):** a revision mismatch
     carries no edit range, and a same-length replace can remove or
     move newlines while every anchor stays in bounds — so ALL
     **pre-marker** in-buffer anchors are dropped on any mismatch,
     the public `line_start_byte` included (a pre-marker byte offset
     is exactly as untrustworthy as a row).
     Immediately after recovery, `n`/`p` report "no more errors" and
     RET reports "no error on this line" for the damaged pre-marker
     content. The marker establishes a fresh anchor epoch: diagnostic
     lines completed by subsequent process output have trustworthy
     positions and may add new in-buffer anchors, so navigation resumes
     for those rows without waiting for the next run. The file-location
     list (`M-g n`) is preserved across epochs;
     stale overlay spans may mis-style until the next run's
     `clear()` (accepted degraded state, named). The pump never
     raises.
- **Style overlay discipline:** one overlay handle per generated
  buffer, created once and retained; `overlay:clear()` on each run
  reset; render re-attach rides **one** `buffer.after-switch`
  subscription that fires whenever any switch path lands on one of
  its buffers (Revision 3) — `start_run`'s own switch included, so
  the former explicit attach after it stacked a duplicate render
  view per run and is removed (Revision 11) — combined with the
  jump-back parity fix (additions #3), this covers `C-x b` returns
  and the primary RET → `M-,` workflow. The former "user-initiated
  switch loses styling" deferral is withdrawn as covered.
  Coordinate translation never depends on any of this: it lives on
  the buffer (Revision 11), exactly once per edit, hidden or split
  or not attached at all.
- No marks needed: one append point, tracked as plain integers, with
  the revision guard above as the honesty check.

### Q#CM3 — Process shape: pipes, merged stderr, null stdin, own group

- `mode = pipes`, `ansi = false` (parsing in Lua, Q#CM2),
  `stdin = "null"`, `group = true`, `env.TERM = "dumb"`,
  `restart = never`, `label = "compile"`.
- **The command line runs as `/bin/sh -c "exec 2>&1; <cmdline>"`.**
  The `exec 2>&1;` prefix merges stderr into stdout **at the child
  boundary**: every descendant inherits fd2 = fd1, so the buffer
  receives one pipe in true kernel arrival order — sidestepping
  `drain_raw_output`'s stdout-then-stderr coalescing entirely (a
  defensive stderr-event arm still routes through the same parser
  but cannot fire when fd2 is fd1). Per-command redirections inside
  the user's cmdline (`2>/dev/null`, `> log`) still behave normally.
- `TERM=dumb` + a non-tty is Emacs's compile posture: tools emit
  plain line-oriented output; tools *forced* to color still parse
  cleanly (SGR → styled spans).
- `stdin = "null"`: noninteractive children that read stdin (`cat`,
  interactive-probe tools) see immediate EOF instead of hanging.
- `group = true`: the full lifecycle policy of additions #2 —
  group-directed TERM with bounded KILL escalation on kill and
  supersede, plus leader-exit reap so a normally exiting shell with
  surviving descendants cannot impose the old two-second/unbounded
  teardown freeze or leak. The accepted synchronous upper bound is
  approximately
  `GROUP_TERM_GRACE + 2 * READER_SEND_POLL_INTERVAL`; eliminating it
  requires the deferred tick-driven drain.
- `cwd`: explicit opt > `pmacs.project.detect(active buffer
  path).root` > `pmacs.instance.identity().working_directory`
  (Revision 7 — actually *resolved*, so the header always names a
  real path and relative error files get an explicit base rather
  than an implicit pass-through).

### Q#CM4 — Error parsing: ordered Lua-pattern rules, parsed at newline time

- `pmacs.compile.rules`: an ordered array of
  `{ pattern, file = <capture idx>, line = <idx>, col = <idx|nil>,
  severity = "error"|"warning"|nil }`; first match per line wins.
  User-extensible from init.lua. Starter rules (Lua pattern syntax):
  1. rustc arrows: `%-%->%s+([^:]+):(%d+):(%d+)`,
  2. generic `([^%s:][^:]*):(%d+):(%d+):` and `file:line:` (gcc,
     clang, most Unix tools; also matches grep-format lines),
  3. Python: `File "([^"]+)", line (%d+)`.
- **Severity and color (Revision 4, contract pinned):** a rule's
  `severity` field is an **override** — when present ("error" or
  "warning"; anything else makes the entry malformed), every match
  of that rule stores it verbatim. When the field is nil, severity
  is sniffed from `error`/`warning` keywords **on the matched line
  only**. Severity drives overlay color (error = indexed red,
  warning = indexed yellow), never navigation. The gcc-style rule
  usually colocates `error:` on the location line and gets color;
  **rustc-arrow and Python frame lines carry no severity token, so
  those built-in entries store `severity = nil` and render in the
  default style — navigable but uncolored.** A context-carrying
  classifier (rustc `error[E…]:` header lines, traceback tails) is
  a named deferral.
- **Coordinate normalization:** rule captures follow compiler
  convention — **1-based line and column**; that is the public rule
  contract. Captured values **below 1, non-integral, or non-finite
  fail closed** (Revisions 4/7): the match is discarded — `%d+`
  accepts `0` (an unvalidated `0 - 1 = -1` would walk the cursor
  loops to a silent (0,0) landing) and also accepts digit runs whose
  `tonumber` is `math.huge` (an unbounded loop bound). The cursor
  walks are independently **movement-bounded** (Revision 7): they
  stop when motion stops moving, clamping at EOF, and the column
  walk clamps at the target row's EOL instead of marching onto later
  rows. Rule capture **indexes** must be positive integers; all
  pattern captures are collected (an index above three reads the
  real capture, not nil-as-column-0); a rule that names a column
  capture its match didn't produce rejects the match. Valid stored
  entries are **0-based** (what `move_active_cursor_to` consumes):
  `line - 1`; `col - 1` when captured, else `0`. The Python rule has
  no column: `line - 1`, col `0`. (Grep normalization is in Q#CM7.)
- **Parse each line exactly once, when its `\n` lands** — CR/erase
  rewrites happen within the current *unterminated* line, so the
  content parsed at newline time is the line's final form. **At the
  terminal event: first `parser:finish()` drains the cross-feed
  state (a truncated multibyte sequence at process EOF surfaces as
  U+FFFD instead of vanishing — Revision 7, additions #5), then the
  pending unterminated line (if any) is finalized and parsed once
  before the exit marker is appended.**
- **Rule-table robustness (fail-closed per entry):** on each run's
  first use the table is validated — non-table `pmacs.compile.rules`
  degrades to **a private deep copy of the built-in defaults**
  (Revision 7: an alias of the public table would keep in-place user
  mutations live after the degradation); malformed entries are
  skipped, invalid Lua patterns are caught (and counted) at
  validation time via a probe match, and match-time pattern calls
  stay pcall'd as belt-and-braces. **Validation is a stable, total
  snapshot (Revision 8):** validated scalar fields are copied into
  per-run plain tables via raw reads — mutating the user's rule
  objects after `compile.run()` cannot alter the in-flight run, a
  hostile `__index` is a counted skip rather than an error through
  the pump, and the container traversal itself is protected
  (5.2+ `ipairs` consults `__index`; LuaJIT reads raw — both
  degrade cleanly). Capture indexes must be positive, FINITE
  integers. One status note per run counts the skipped entries; a
  later valid rule still matches; the per-frame pump never raises.
  Shell-command slots never touch the rule table at all.
- Each match appends `{ file, line, col, severity,
  line_start_byte }` to the run's ordered error list (reset per
  run). Relative paths resolve against the run's cwd.
  `pmacs.compile.errors()` exposes the current list (a getter, per
  API conventions — public surface, not a test seam).
- Visiting pcalls `find_or_open` and reports failures as status (the
  `visit_location` discipline).

### Q#CM5 — Unified next-error: dispatcher with diagnostics fallback

- New module-owned slot `pmacs.errors` with `claim(source)` where
  `source = { name, next(), previous() }`. A compile run claims it on
  spawn; a grep run claims it on search start. **Last claim wins**
  (a deliberate simplification of Emacs's `next-error-last-buffer`).
- New commands `error.next` / `error.previous`: dispatch to the
  claimed source; **when nothing has claimed, invoke
  `diag.next`/`diag.previous`** — a user who never compiles or greps
  sees exactly today's behavior.
- **Rebind mechanics:** duplicate bindings are rejected, so
  compile.lua explicitly `pmacs.keymap.unbind`s `M-g n` and `M-g p`,
  then binds the dispatchers (hence the Q#CM1 load-order contract).
  `diag.next`/`diag.previous` remain as named commands (and keep
  their wrap). `` C-x ` `` → `error.next`, unconditional. Acceptance
  pins the final dispatch-level lookup of all three chords.
- Compile-source walk semantics: a per-run current index; stepping
  past either end reports "no more errors" and stays (**no wrap**,
  Emacs compile parity; the diag fallback keeps its documented
  wrap). RET-visiting an error re-seats the index there.

The alternative — leaving `M-g n/p` on diagnostics and giving compile
`M-g M-n`/`M-g M-p` — avoids touching a shipped binding but
permanently forks the Emacs muscle memory the lsp.lua comment itself
acknowledges. The dispatcher keeps one chord pair with a
behavior-preserving fallback.

### Q#CM6 — Compilation buffer keys (buffer-local)

`RET` visit the error on the cursor's line (status "no error on this
line" otherwise; jump ring included so `M-,` returns); `n`/`p` move
to the next/previous error *line* within the buffer (cursor motion
only, no visit); `g` recompile; `q` restore the previous buffer;
`C-c C-k` kill the running compile; all seven undo/redo chords
status no-ops (Q#CM2 resilience). `set_round_trip_input(buf, true)`
per Q#P6. No
auto-scroll: the cursor starts on the header and stays where the
user puts it (an auto-scroll option is deferred).

### Q#CM7 — Grep-mode: upgrade `project.search` in place

`*search-results*` becomes a first-class locations buffer:

- Read-only intercept + bypass writes; RET/`n`/`p`/`q` buffer-local
  keys, undo no-ops, and round-trip input, mirroring Q#CM6.
- Every streamed match appends both the formatted line and a
  location entry — **no regex parsing**. Normalization: `line` is
  1-based → store `line - 1`; `match_start` is already a 0-based
  byte offset within the line → store as col unchanged; `file` is
  **relative to the search root** → resolve against the root used
  for this search (not the cwd).
- **Kill-mid-search safety (Revision 3):** `pmacs.buffer.on_removed`
  on the results buffer calls `stream:cancel()` (exists,
  `async.lua:155`) and invalidates `active_search_id`; the
  `on_batch`/`on_close` callbacks additionally guard
  `buf:is_valid()` before writing (today they write through a stale
  handle unguarded, `default.lua:693-703`). The next search
  recreates the buffer.
- Every `on_batch`/`on_close` write runs the Q#CM2 revision check
  **before** mutating the buffer, then records the post-write revision.
  Checking only process-pump appends is insufficient: a no-hook edit to
  `*search-results*` followed by a worker batch would otherwise advance
  the expected revision and permanently mask the external edit.
- **The panel has the same immediate `buffer.after-edit` recovery
  trigger as the compile slots (Revision 7):** after a COMPLETED
  search no producer write or navigation may ever come, so an `M-x
  buffer.undo` in the panel must be marked synchronously, not on the
  next guarded operation.
- **Root retention across interactive supersedes (Revision 3):** the
  panel stores the root each search ran with. Resolution order:
  explicit `opts.root` > *if the active buffer is the results panel,
  the panel's stored root* > `pmacs.project.detect(active buffer
  path).root` > `"."`. Without the panel clause, the natural UI path
  — search, land in the pathless panel, search again to supersede —
  silently degrades the root to `"."`.
- Claims the error source on search start, so `M-g n` walks matches.
- Supersede semantics (`supersede = "search"`, late-batch dropping by
  stream id) unchanged.

### Q#CM8 — Shell-command: `M-!`, same machinery, no error claim

`M-x shell.command` bound to `M-!`: prompt (history bucket
`"shell"`), run through the Q#CM3 shape (merged stderr, null stdin,
own group) into `*shell-command*` via the same streaming machinery
(read-only, exit marker, `q` restore, undo no-ops). Always async — a
separate `M-&` adds nothing (named deferral). Does **not** parse
errors or claim the error source.

### Q#CM9 — Lifecycle: one live run per buffer slot, tombstoned teardown

- One compilation at a time: `compile.run` while a run is live
  group-SIGTERMs the old process (with the additions-#2 KILL
  escalation backing it) and **tombstones** its pump entry: output
  events for a tombstoned entry are dropped on arrival, but the
  entry stays registered until its terminal event drains, at which
  point it calls `pmacs.process.forget` and removes itself —
  `forget` is only legal on terminated processes
  (`process.rs:1109-1123`). The buffer resets and the new run starts
  immediately; status notes the supersede. Same rule for
  `*shell-command*`; grep supersedes at the worker layer.
- **Killed buffer:** `pmacs.buffer.on_removed` on each generated
  buffer initiates prompt group termination of a live run; the pump
  entry is tombstoned the same way. The pump also guards
  `buf:is_valid()` defensively. The next run recreates the buffer.
- Post-condition either way: `pmacs.process.list()` returns to its
  pre-run baseline once the terminal event has drained — now
  **guaranteed** by the bounded TERM→KILL escalation (a
  TERM-ignoring group can no longer stall the tombstone forever) —
  pinned by acceptance.

### Q#CM10 — Display and replication

- Switch-in-place (Q#P2): `compile.run` switches the active window
  to `*compilation*`; `q` restores.
- All writes are daemon-side Lua bypass edits — ordinary daemon-peer
  CRDT ops; no optimistic path, no typed-edit provenance
  involvement. One CRDT acceptance test pins mirror-replica
  convergence of a full run.
- Undo in generated buffers: the shipped undo/redo chords are
  neutralized buffer-locally; command/menu undo, accepted replica ops,
  and no-hook programmatic edits are survived by the revision guard
  (Q#CM2). The
  *content* damage of a guard-recovered undo is accepted degraded
  state (desync marker); making generated buffers truly immutable is
  owned by the deferred real `read_only` flag (lsp-panels framing).

### Q#CM11 — Interactive command contract

- **`compile.run`** (the `M-x compile` equivalent; description says
  so for discoverability): prompts via `pmacs.minibuffer.read` with
  history bucket `"compile"` and `initial` = the stored previous
  command.
- **`pmacs.compile.run(cmdline, opts)`** is the programmatic API
  (`opts.cwd` override); the interactive command calls it. Each
  successful start stores `{ cmdline, cwd }` as the recompile state
  (session-scoped; persistence is a named deferral).
- **`compile.recompile`** (`g` in the buffer, also `M-x`-able):
  reruns the stored state; errors with a pointed status if nothing
  has been compiled yet.
- **`q`-target discipline:** the previous-buffer capture happens
  only when the module switches in *from a buffer that is not one of
  its own generated buffers* (listview's never-capture-a-panel
  guard, extended). `g` therefore reruns without re-capturing —
  compile → `g` → `q` restores the original buffer, pinned by
  acceptance.
- **`shell.command`** (`M-!`): prompt with history bucket `"shell"`;
  programmatic `pmacs.shell.command(cmdline, opts)`.

## Bets

- Pipes + `TERM=dumb` + child-boundary stderr merge + Lua-side ANSI
  parsing cover real compiler output (colors when forced, progress
  bars collapsed, no corruption, true arrival order) without PTY
  complexity.
- The `group = true` lifecycle policy (spawn group, `-pid` signal,
  liveness-probed TERM→KILL ledger, cancellable readers) reaps
  `sh -c` trees without the old two-second/unbounded tick freeze or
  thread/FD leaks; `setsid` remains the deliberate process-ownership
  escape hatch, while its inherited output reader is still cancelled
  within the same bounded drain cap.
- `exec 2>&1;` prefixed to the user's cmdline preserves per-command
  redirection semantics while merging by default.
- Three starter rules cover cargo/rustc, gcc/clang, Python, and
  grep-format lines; everything else is a user-added rule.
- The claim-based dispatcher preserves today's `M-g n/p` exactly for
  non-compile users while giving compile/grep the Emacs chords.
- The revision-guard resync makes the pump total: no external edit —
  undo included, same-length replaces included — can make it raise
  or write out of bounds.
- Per-newline parsing + per-tick pump stay far inside the frame
  budget (the REPL's 100 MB/s ingest discipline, minus its per-byte
  hot path).

## Deferred (named)

PTY-mode compile variant (COLUMNS/forced-color env); auto-scroll
option (`compilation-scroll-output` analog); context-carrying
severity classifier (rustc header lines, traceback tails — starter
rules are navigable but uncolored there); severity threshold for
navigation (skip warnings); per-language/project default compile
commands; echo-area display of short shell-command output and a
distinct `M-&`; error parsing in `*shell-command*`; occur-mode;
split-window display of the compilation buffer; real `read_only`
buffer flag (owns full immutability of generated buffers; already
deferred in lsp-panels framing); persistence of the last compile
command across sessions; byte-accurate col cursor walk (inherited
lsp.lua residual); next-error across multiple historical result
buffers (only the claimed source is walkable); configurable
`GROUP_TERM_GRACE`; unifying poll-based cancellable readers across
non-group pipe consumers (REPL, LSP — tuned on the M6.6 ingest
  gate, migrated separately if ever); fully tick-driven final drain
  (the in-loop ledger enforcement plus escaped-writer cancellation
  bounds the synchronous stall at approximately
  `GROUP_TERM_GRACE + 2 * READER_SEND_POLL_INTERVAL`; eliminating it
  entirely is a supervisor state-machine change).

## Acceptance

Suites: `tests/compile_mode_acceptance.rs` (dispatch-driven, pump via
the `run_with_pump` pattern) + `tests/compile_mode_crdt_acceptance.rs`
(small). Fixtures are `/bin/sh` scripts in a tempdir emitting
scripted output. Keybinding tests dispatch keys (never
`pmacs.command.invoke`), per the standing discipline.

1. `compile.run` **spawns successfully** under the exact production
   spec (pipes, `ansi = false`, `stdin = "null"`, `group = true`)
   and streams a script's output into `*compilation*` (header with
   command + cwd; exit marker with code; status).
2. Read-only: dispatched typing is rejected; buffer text unchanged.
3. stderr/stdout interleaving: a script alternating
   `echo out; echo err >&2` lines yields the buffer in **emission
   order** (would fail under per-tick stdout-then-stderr
   coalescing).
4. EOF: a command that exits only at stdin EOF (`cat`) terminates
   promptly with exit code 0 (would hang under piped stdin).
5. Group kill: a script that backgrounds a descendant
   (`sleep 60 & echo $! > pidfile; wait`) — `C-c C-k` produces a
   signaled exit marker AND the recorded descendant pid is dead
   within the timeout (would survive under positive-pid SIGTERM).
6. **Leader-exit reap (Revision 3):** a script that backgrounds a
   descendant and exits WITHOUT waiting
   (`sleep 60 & echo $! > pidfile`) — the run completes promptly
   (exit marker within a wall-clock bound well under the drain
   timeout; the tick does not freeze), the descendant pid is dead,
   and `pmacs.process.list()` returns to baseline.
7. **TERM→KILL escalation (Revision 3):** a script that traps
   SIGTERM (`trap '' TERM; sleep 60`) — `C-c C-k` still yields a
   terminal event within the escalation bound; baseline restored.
8. **Group-liveness ledger (Revision 4, bite):** a leader that
   exits normally while a TERM-ignoring descendant redirects its
   output away (`( trap '' TERM; exec >/dev/null 2>&1; sleep 60 ) &
   echo $! > pidfile`) — the terminal event arrives and the readers
   finish, yet the descendant pid is dead within the escalation
   bound (fails under leader- or reader-conditioned escalation;
   only the `kill(-pgid, 0)` probe catches it).
9. **In-drain enforcement, non-redirected twin (Revision 5):** the
   same TERM-ignoring descendant but **keeping fd1 open** (no
   redirect) — the group dies near the 500 ms grace bound, not the
   ~2 s drain timeout, and the blocking tick's latency is bounded by
   `GROUP_TERM_GRACE + 2 * READER_SEND_POLL_INTERVAL`, with a modest
   wall-clock tolerance for test scheduling (fails without ledger
   enforcement inside the drain loop). The supervisor-unit setsid
   twin in item 34 pins the same bound when the original pgrp is
   already ESRCH but an escaped writer still holds fd1.
10. Error parsing per starter rule: rustc-arrow, gcc-style
    `file:line:col:`, Python `File "...", line N` fixtures produce
    the expected `pmacs.compile.errors()` lists — including 0-based
    normalization (a `foo.rs:3:5` diagnostic lands the cursor on
    line index 2, col index 4), cwd-relative resolution, and
    severity: gcc-style entries carry `error`/`warning`,
    rustc-arrow and Python entries carry `nil`.
11. **Sub-1 coordinates fail closed (Revision 4):** a `foo.rs:0:0:`
    line produces no error entry (would otherwise store -1 and land
    the cursor silently at (0,0)).
12. **Custom-rule severity override (Revision 4):** a user rule
    with `severity = "warning"` stamps every match "warning" even
    when the line says `error:`; a rule with `severity = "fatal"`
    is rejected as malformed (skipped, counted in the status note).
13. Unterminated final line: a fixture whose last diagnostic is
    emitted via `printf` with no trailing `\n` still parses (bite:
    fails without the terminal-event finalization).
14. Malformed rules: a non-table `pmacs.compile.rules` and a table
    containing an invalid-pattern entry + a valid entry — no error
    spam (clean `*errors*`), the valid entry still matches, one
    status note.
15. RET on an error line visits the tempdir file at line/col; `M-,`
    returns; RET on a non-error line reports status and stays.
16. `n`/`p` walk error lines in-buffer; no wrap, status at the ends.
17. Chord pins (dispatch-level): `M-g n`/`M-g p` reach the
    dispatchers; `` C-x ` `` reaches `error.next`; after a compile
    they walk compile errors in order across files; past the last:
    "no more errors", no wrap.
18. Dispatcher fallback: `M-g n` with no claim falls through to
    diagnostics (pathless scratch: the diag "no LSP server" status).
19. `g` recompiles: fresh buffer content, command actually
    re-executed (script increments a counter file); old
    style-overlay spans cleared.
20. compile → `g` → `q` restores the buffer that was active before
    the *first* compile (q-target not re-captured).
21. `C-c C-k` kills a long-running script → signaled exit marker.
22. Supersede: `compile.run` during a live run terminates the old
    group; no old-run output or exit marker lands after the reset;
    `pmacs.process.list()` returns to baseline once drained.
23. **Undo/redo key aliases (Revision 4/6):** table-driven dispatch of
    all seven shipped forms in `*compilation*` — `C-/`, `C-_`, `C-4`,
    `C-x u`, `C-?`, `C-S-_`, and `C-x r` — produces the status no-op
    and leaves text unchanged. `C-4` pins the raw-terminal-deliverable
    single-key undo shape; the final three pin redo as well.
24. **Command-path undo after a completed run (Revision 5):**
    `M-x buffer.undo` in `*shell-command*` AFTER the process has
    exited — no pump event will ever arrive — yet the desync marker
    appears immediately via the `buffer.after-edit` subscription
    (bite: fails when recovery only runs at pump/anchor time).
25. **No-hook programmatic external edit (Revision 3–6):** the harness
    evaluates Lua directly, outside dispatch and
    `with_after_edit_check`, to perform a mid-stream bypass-intercept
    shrink and a **same-length bypass replace that moves a newline**.
    These are the actual no-hook producer the pump guard owns —
    generated-buffer keystrokes round-trip, and accepted remote CRDT ops fire
    `buffer.after-edit`. Both mutations trigger resync (the replace
    bite fails under a length-only guard); the pump survives, appends
    the desync marker, and continues streaming. All pre-marker anchors
    drop, so `n`/`p` and RET initially report empty-state statuses
    instead of using stale rows. The fixture then emits one new
    diagnostic line after asserting the recovery marker is newline-
    delimited: it receives a fresh post-marker anchor, and
    `n`/`p` plus RET navigate that row correctly. A grep twin edits
    `*search-results*` between batches and proves the next `on_batch`
    detects the mismatch before its own append rather than masking it.
26. ANSI: a script emitting SGR color + CR progress yields final
    text free of escape bytes, the progress line collapsed, AND —
    attachment proven, not just span existence — a rendered TUI
    cell in the active window carries the span's style; RET to a
    file then `M-,` back retains styling (rides the jump-back
    parity fix + after-switch re-attach).
27. Buffer killed mid-run (`on_removed` path): process terminated,
    no error spam, `pmacs.process.list()` back to baseline; next
    run recreates the buffer.
28. Grep: seeded tempdir; `project.search` results are read-only,
    RET visits the match (root-relative path resolved, 0-based
    landing), `M-g n` walks matches (source claimed); a second
    search still supersedes the first.
29. **Grep kill-mid-search (Revision 3):** killing
    `*search-results*` during an active stream cancels it — no
    stale-handle writes, clean `*errors*` — and a subsequent search
    recreates the buffer and works.
30. **Grep root retention (Revision 3):** an interactive search from
    a project file, then a second interactive search issued from
    inside the results panel (no `opts.root`) — both run with the
    same root (would degrade to `"."` without the panel clause).
31. Shell-command: `M-!` output lands in `*shell-command*` with exit
    marker; does not claim the error source.
32. `q` restores the previous buffer from all three buffers.
33. Rust-level: round-trip input set on the generated buffers.
34. Supervisor unit tests (in `src/process.rs`): `stdin = "null"`
    yields immediate EOF; `group = true` spawns a distinct process
    group; `terminate` signals group-wide with ledger-driven KILL
    escalation on a TERM-trapping child; the liveness probe reaps a
    TERM-ignoring survivor after a normal leader exit (unit twin of
    acceptance 8); **repeated `terminate` calls do not extend the
    ledger deadline** (earliest-deadline-wins, Revision 5);
    **`shutdown()` force-kills outstanding ledger groups and probes
    them to ESRCH** (drop-twin of acceptance 8, Revision 5);
    **`maybe_restart` is inert once `shut_down`** — a
    `restart = always` process does not respawn during teardown
    (Revision 5); leader-exit reap enforces the original deadline for
    a TERM-ignoring pipe-holding descendant (unit twin of acceptance
    9); a setsid'd descendant is not reaped, but teardown is bounded
    well below `EXIT_OUTPUT_DRAIN_TIMEOUT` and **reclaims resources** —
    repeated spawns with a still-open escaped writer all return through
    the retained reader joins within the bound, and a per-runtime
    `cfg(test)` active-reader counter returns to zero before cleanup
    (deterministic proof that the joined threads ended and their owned
    read FDs dropped, without racy process-global resource counts); each
    escaped fixture pid is explicitly killed during test cleanup; both
    options are rejected under PTY mode; `jump_back` binding fires
    `buffer.after-switch` exactly
    when the buffer changed.
35. CRDT: a full compile run converges byte-identically on a mirror
    replica. A synthetic accepted replica edit to the generated buffer
    also triggers the immediate recovery marker and converges on two
    replicas even though the hook-produced marker may queue before the
    source edit's rebroadcast (the established causal-reordering seam).

Every reviewer finding in PR rounds gets a bite-verified fix (a test
observed failing without the fix), per the standing method.
