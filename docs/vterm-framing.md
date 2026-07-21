# Vterm — framing (Arc 5 stage 2, three-PR delivery)

**Revision 2 — 2026-07-21. Status: decisions folded; awaiting approval, no implementation.**

Revision 2 records the architecture discussion: `C-c` is the terminal editor
escape (`C-c C-c` sends interrupt); main-screen resize reflows while alternate
screen clips/pads; exited buffers remain with an Emacs-style process message;
protocol v19 is additive with complete frames; shared `Style` stays unchanged;
and one `BufferId` owns one shared process/screen whose most recently active
frontend controls size.

This framing follows the compile-mode terminal substrate that landed in PR
#113. `src/process.rs` already owns PTY creation, process groups, bounded
readers, stdin writes, resize, exit/restart state, and final drain.
`src/ansi.rs` already owns a streaming UTF-8/CSI/OSC parser, but deliberately
emits only the line-oriented subset compile-mode needs. Vterm does not replace
either subsystem. It extends their contracts and adds the missing terminal
screen state machine.

Arc 5 stage 2 ships as three separately reviewed PRs:

1. **terminal core** — full-screen VT events, `TerminalScreen`, internal
   session ownership/contracts, and headless real-PTY acceptance;
2. **TUI integration** — terminal-window composition, input, resize,
   scrollback, selection, and copy;
3. **GPU integration** — protocol v19 terminal messages, semantic-daemon
   routing, and a native GPU terminal renderer.

There is no single mega-PR. Each stage is useful and testable by itself, and a
later stage starts only after the preceding stage lands on `main`.

## 1. Problem and ownership boundary

Pmacs can supervise a PTY and can parse enough ANSI to turn command output into
a line-oriented compilation buffer. It cannot host `nvim`, `htop`, a shell
using cursor motion, or any other application whose output means “mutate a
terminal screen” rather than “append text.” The current parser intentionally
recognizes and discards cursor addressing, alternate-screen state, scrolling,
and most terminal modes. The editor renderer only knows an ordinary rope and
its views.

The missing abstraction is a real terminal state machine:

```text
PTY bytes -> AnsiParser -> terminal operations -> TerminalScreen
                                               -> response bytes -> PTY stdin
Frontend input -> terminal input encoder ------------------------^ 

TerminalScreen -> TUI cell composition
               -> protocol v19 TerminalFrame -> GPU cell layout
```

Ownership is explicit:

- `ProcessSupervisor` owns the child, PTY file descriptors, bounded worker
  pipeline, signal delivery, and final drain.
- `AnsiParser` owns byte-stream framing and escape-sequence decoding. It does
  not own a screen.
- `TerminalScreen` owns main/alternate grids, cursor and modes, scroll regions,
  tab stops, and scrollback.
- `TerminalManager` owns the mapping from a special editor `BufferId` to one
  PTY process plus one `TerminalScreen`, and owns per-window/per-frontend
  scroll/selection state.
- The ordinary buffer is an identity and lifecycle anchor only. Terminal
  screen contents are never mirrored into its rope.
- TUI and GPU frontends own final glyph drawing. They consume cells; they do
  not reinterpret ANSI or maintain a second VT state machine.

This keeps the existing semantics-down boundary intact. Document text remains
a CRDT/rope semantic surface. A terminal is inherently a cell protocol, so its
new wire family carries terminal cells, not daemon-formatted document text and
not pixels.

## 2. Ground truth in the current tree

The implementation must preserve these existing contracts:

- `ProcessMode::Pty`, `write_stdin`, `resize_pty`, group-directed signals,
  bounded output channels, and the TERM/KILL final drain already exist in
  `src/process.rs`.
- `ProcessSpec::ansi_events` moves parsing onto a bounded worker and emits
  `ProcessEventKind::Ansi`; raw pipe/LSP consumers retain their byte contract.
- `AnsiParser` is stateful across arbitrary feed boundaries, has a per-state
  escape-sequence cap, safely recovers malformed sequences, carries truecolor
  and underline color, and resets after `finish()`.
- The parser currently emits `Text`, `SetStyle`, line-oriented controls,
  `Erase`, `SetTitle`, and alternate-screen markers. Cursor motion and most
  CSI/DEC operations are parsed but discarded by design.
- Alternate-screen suppression is currently parser-global and load-bearing for
  compile/REPL consumers. Vterm therefore requires an explicit parser profile:
  `LineOriented` preserves today's suppression and event contract;
  `FullScreen` emits all screen/mode operations. `ProcessSpec` selects the
  profile when `ansi_events` is enabled; existing Lua process specs default to
  `LineOriented`, while terminal sessions construct `FullScreen` specs.
- `EditorState::tick_processes` is the main-thread process drain point.
  LSP/MCP and Lua packages drain events only for process IDs they own.
- `EditorCore::round_trip_buffers` already disables optimistic frontend edits
  for special buffer-local input surfaces.
- Grid frontends already receive generic `CellDelta`; a terminal window can be
  composed into that grid without a new grid protocol.
- Semantic frontends hold document text through CRDT snapshots and receive
  byte-anchored style/decorations. They require a new terminal-specific
  message because an empty terminal identity buffer contains no screen text.
- Desktop persistence saves file-backed buffers only. A pathless terminal
  buffer is already omitted; no special persistence exception is required.

## 3. Terminal core model

### 3.1 Types and files

Stage 1 adds a `src/terminal/` module rather than growing `editor.rs` or
`ansi.rs` into a second monolith:

- `screen.rs`: `TerminalScreen`, `TerminalGrid`, `TerminalRow`, cursor, modes,
  scrollback, resize, snapshots;
- `input.rs`: normalized key/mouse/paste to VT byte encoding;
- `session.rs`: `TerminalManager`, `TerminalSession`, process ownership and
  event application;
- `view.rs` (stage 2): per-context viewport/selection helpers and TUI
  composition.

`src/ansi.rs` remains the one escape parser. It gains terminal operations; it
does not depend on editor state or `TerminalScreen`.

The public core shape is:

```rust
pub struct TerminalManager { /* BufferId -> TerminalSession */ }

pub struct TerminalSelectionSpan {
    pub row: u32,
    pub start_col: u32, // inclusive
    pub end_col: u32,   // exclusive
}

pub enum TerminalProcessState {
    Running,
    Exited(i32),
    Signaled(String),
    Crashed(String),
}

pub struct TerminalSnapshot {
    pub buffer_id: BufferId,
    pub size: CellSize,
    pub cells: Vec<Cell>,       // row-major visible slice
    pub cursor: Option<CellCoord>,
    pub title: Option<String>,
    pub screen_generation: u64,
    pub selection: Vec<TerminalSelectionSpan>,
    pub scroll_offset: u32,
    pub at_bottom: bool,
    pub pid: u32,
    pub process: TerminalProcessState,
}
```

A snapshot is owned data taken only after all parser events for the current
main-thread tick are applied. Renderers never borrow the mutable screen across
Lua, editor-core, or process-supervisor calls.

### 3.2 Parser extension

`AnsiEvent` gains enough operations to drive a VT-style screen:

- printable text, BEL, CR, LF/VT/FF, BS, HT, and tab-stop set/clear;
- relative and absolute cursor movement (`CUU/CUD/CUF/CUB`, `CNL/CPL`,
  `CHA/HPA`, `VPA`, `CUP/HVP`);
- erase display/line/characters;
- insert/delete characters and lines;
- scroll up/down and set/reset scrolling margins;
- save/restore cursor for both DEC and CSI spellings;
- main/alternate screen enter/exit (`47`, `1047`, `1049`);
- mode set/reset for insert, origin, autowrap, application cursor,
  application keypad, cursor visibility, bracketed paste, focus reporting,
  synchronized output (`?2026`), and supported mouse reporting modes;
- SGR and title changes;
- DEC G0/G1 character-set selection plus SI/SO, including the line-drawing
  characters full-screen TUIs depend on;
- device-status/device-attribute queries represented as typed requests. The
  session, not the parser, writes bounded response bytes to PTY stdin.

Unknown, private, or malformed sequences stay non-fatal and bounded. Parser
state always makes forward progress. DCS/APC/PM payloads remain ignored under
the same per-sequence cap; they must not leak into visible text.

The parser-profile split is compatibility-critical. Full-screen support must
not make compile-mode start appending alternate-screen text or receiving
cursor events it does not understand. `AnsiParser::new()` and the existing
Lua `ansi = true` process option retain the line-oriented profile; terminal
session construction is the only initial full-screen caller. On `finish()`,
`LineOriented` preserves today's synthetic alternate-screen/style balancing.
`FullScreen` flushes pending text but does not invent an alternate-screen exit
or clear cells. The session manager, after applying the actual final parser
events, adds the process-exit annotation described in §4.1. Parser internals
reset in both profiles.

The shared `Cell::Style` remains unchanged in this arc. Existing support covers
indexed/truecolor foreground/background, bold, italic, underline variants,
underline color, and reverse. Faint, conceal, blink, and strikethrough remain
ignored rather than being mapped to an unrelated attribute. Extending the
shared style would change every cell-carrying postcard encoding and is a
separate protocol-wide decision, not hidden vterm scope.

### 3.3 Screen invariants

`TerminalScreen` maintains two grids:

- **main screen** with bounded scrollback;
- **alternate screen** with no scrollback and an independently saved cursor.

Every physical row records a monotonic logical-line ID, its cell offset within
that logical line, and whether it ended in a soft autowrap. This is necessary
for copy and resize: hard line breaks become `\n`; soft-wrapped physical rows
are joined into one logical line. Per-view top/selection anchors use
`(line_id, cell_offset)`, not a physical row index, so reflow can remap them.

Core invariants:

- `cells.len() == rows * cols`; every row has exactly `cols` cells.
- Cursor and scrolling margins are always in bounds after every operation.
- A wide grapheme occupies a leading glyph plus `Glyph::Continuation`; an
  overwrite, erase, insert, delete, or resize never leaves an orphaned half.
- Combining codepoints extend the preceding grapheme when one exists; at the
  left edge they combine with a space cell. Cluster byte length is bounded.
- Autowrap uses the pending-wrap rule: writing the last column arms a wrap;
  the following printable grapheme performs it. Cursor motion/control clears
  the pending wrap where VT behavior requires.
- Origin mode interprets absolute row addressing relative to the active
  scrolling region.
- Insert/delete/scroll operations affect only their defined region and fill
  exposed cells with the current erase style.
- Entering `1049` saves the main cursor and clears the alternate grid; leaving
  it restores the main grid/cursor. Repeated set/reset is idempotent.
- Resizing the main screen reflows soft-wrapped logical lines, preserves hard
  breaks and logical-line IDs, and maps the cursor to the corresponding
  logical offset. The alternate screen is never reflowed: it is clipped/padded
  in place, matching a full-screen application's expectation that it will
  repaint after `SIGWINCH`.
- Scrollback eviction removes the oldest complete physical rows. It clamps a
  per-view anchor whose logical line was evicted to the oldest retained row.
  The default cap is 10,000 rows and an independent 4,000,000-cell budget;
  either limit may trigger eviction.

Synchronized output is a publication gate, not a second screen. Operations
continue mutating `TerminalScreen`, but snapshots retain the last published
generation until `?2026l`. Exit/truncation releases the final state. A bounded
one-second watchdog also releases and clears synchronization if a buggy child
never resets it, preventing a permanently frozen editor surface.

Hard limits are shared constants, not frontend-local guesses:

- maximum rows: 512;
- maximum columns: 512;
- maximum visible cells: 262,144;
- maximum UTF-8 bytes in one grapheme cluster: 256;
- maximum retained history cells: 4,000,000;
- maximum parser control-string payload: the existing 1 KiB cap.

Invalid creation/resize arguments reject atomically. A rejected resize leaves
the process and prior screen unchanged.

### 3.4 Device responses

The core responds to the small query set required by normal shells and TUIs:

- primary and secondary device attributes;
- operating-status report;
- cursor-position report, using current origin semantics.

Responses are fixed templates plus checked decimal coordinates and pass
through the existing bounded `write_stdin` queue. OSC 52 clipboard writes,
window manipulation, palette mutation, hyperlinks, sixel, and arbitrary DCS
responses are not honored. Child output is untrusted; it cannot directly set
the host clipboard or execute an editor command.

## 4. Session, buffer, and Lua contract

### 4.1 Session lifecycle

One terminal session owns exactly one process ID, one screen, and one identity
buffer. A buffer may appear in several windows/frontends, but there is never a
second parser or screen for it.

Creation order is transactional:

1. validate and copy the complete `TerminalSpec`;
2. create the read-only identity buffer;
3. spawn a raw PTY with `ansi_events = FullScreen`;
4. install the session in `TerminalManager`;
5. mark the buffer round-trip-only.

Stage 1 exposes that operation as a Rust manager contract for headless tests
and future callers, but deliberately registers no interactive command: opening
an unrenderable blank terminal buffer would be a broken partial feature.
Stage 2's Lua binding calls the same operation and switches the caller's active
window only after it succeeds.
On synchronous spawn failure, the temporary buffer is removed and no session
entry remains. The terminal process ID is not exposed through
`pmacs.process`; only `TerminalManager` drains its events, so two consumers
cannot steal batches from each other.

`EditorState::tick_processes` becomes:

1. supervisor `tick()`;
2. terminal manager drains only its owned process IDs, applies all ANSI
   batches, queues device responses, and records terminal exit state;
3. the existing `process.after-tick` hook runs;
4. LSP/MCP retain their existing later ticks.

At process exit, final drained output is applied first. The manager then writes
one synthetic, default-style hard line into the active terminal screen:
`Process <PID> exited normally with code 0` for zero; `Process <PID> exited
abnormally with code <code>` for a non-zero code; or `Process <PID> exited
abnormally with signal <signal>` for signal termination. It emits CRLF first
when needed so the annotation never overwrites child text. The annotation is
terminal-owned (not parser output and not rope text), visible and copyable like
the rest of the screen.
The buffer remains until the user kills it. Killing the buffer terminates a
still-live process/session. A periodic prune handles all buffer-removal paths,
including Lua callers that bypass the friendly terminal close API. Editor
shutdown uses the existing supervisor shutdown path and cannot restart a
terminal.

The wire-visible process state has only the four variants above: synchronous
spawn failure never publishes a session, and `terminate` remains `Running`
until the supervisor reports its final outcome. Crash/signal strings are
sanitized to one line and bounded before snapshots or Lua metadata are built.

### 4.2 Buffer semantics

A terminal buffer is pathless, unmodified, and read-only. Read-only is a
buffer-owned invariant, not only an intercept view: `Buffer` gains a flag and
typed error checked by ordinary edits, intercept-skipping host edits, undo/
redo, and local/remote CRDT content mutation. The terminal manager sets it
before publishing the session. The rope stays empty for the session lifetime.

Consequences are deliberate:

- buffer lists and window switching see a normal named buffer;
- normal save/autosave/LSP/syntax/CRDT editing does not apply;
- desktop save omits it because it has no file path;
- terminal scrollback does not participate in ordinary document search;
- copy reads from terminal rows, never from a hidden mirror rope;
- killing the buffer is the single editor-lifecycle teardown signal.

Semantic bootstrap may attach an immutable empty CRDT state and send its
`BufferSnapshot` so v18/v19 mirrors can track the buffer identity and
`CursorByte` without decode/state failure. No terminal contents enter that
CRDT, and local/remote CRDT edit validation rejects the read-only buffer. The
stage-3 terminal frame is the authoritative visible semantic surface; a forged
remote operation cannot mutate the empty identity rope.

No hidden “text projection” is maintained. Two representations would drift on
cursor rewrites, erase operations, alternate-screen swaps, and resize.

### 4.3 Lua API

Stage 2 installs `pmacs.terminal` before user config, loads
`builtin/runtime/terminal.lua`, and registers the interactive command:

```lua
local buffer = pmacs.terminal.open {
  command = os.getenv('SHELL') or '/bin/sh',
  args = {},
  cwd = nil,                 -- inherits instance cwd
  env = {},
  name = nil,                -- default: *terminal:<command>*
  rows = 24,
  cols = 80,
  scrollback_rows = 10000,
}

pmacs.terminal.is_terminal(buffer)       -- boolean
pmacs.terminal.state(buffer)             -- fresh plain metadata table
pmacs.terminal.send(buffer, bytes)       -- explicit raw bytes
pmacs.terminal.resize(buffer, rows, cols)
pmacs.terminal.terminate(buffer)         -- SIGTERM; buffer remains
pmacs.terminal.scroll(lines)             -- active terminal window
pmacs.terminal.scroll_to_bottom()
pmacs.terminal.copy_selection()          -- active terminal window
```

`open` validates exact raw table fields before side effects. Unknown fields,
metatable-provided fields, holes in `args`, non-string env keys/values,
embedded NUL, non-integer dimensions, and out-of-range scrollback reject with
the field named. The copied spec is immune to caller mutation. Returned and
accepted identity is `BufferIdLua`, following the rest of the editor API.

The built-in chunk registers `terminal` as an interactive command. It opens
`$SHELL` without a shell-command interpolation layer. There is no command
string split and no implicit `sh -c`.
It also installs terminal-buffer-local commands used after the escape prefix:
`M-w` copies the terminal selection, `M-v`/`C-v` page scrollback up/down, and
`M-<`/`M->` move to the oldest retained row/bottom. These shadow ordinary
document commands only during the one-key editor escape; normal terminal input
still sends those keys to the child.

## 5. Stage 2 — TUI integration

### 5.1 Composition and cursor

For every window whose `buffer_id` belongs to `TerminalManager`, the window
content rectangle is painted from a terminal snapshot instead of
`TextView::render`. Modeline/statusline composition remains unchanged. Normal
text overlays, line-number gutters, syntax, diagnostics, and wrapping are not
run over terminal cells.

Each `(frontend_id, window_id, buffer_id)` owns a `TerminalViewState`. At
bottom, the last screen row aligns with the content rectangle's last row.
Scrolling records a stable logical-line top anchor rather than a numeric
distance from a moving tail. New child output therefore does not move a
scrolled-back viewport or selection. If retention evicts that anchor, it
clamps once to the oldest retained row. A “not at bottom” marker is available
to the built-in terminal statusline provider.

The active terminal cursor is translated from terminal-local coordinates into
the window rectangle. It is hidden when the child hid it, the window is not
active, the viewport is scrolled away from bottom, or the coordinate is
clipped. Other terminal windows do not paint a cursor.

A smaller window clips; a larger window pads with default cells. Merely
rendering a passive view never resizes the PTY.

BEL is forwarded only from the active terminal through the existing frontend
signal path. OSC title is sanitized and exposed in terminal metadata/frame and
the terminal statusline; it does not rename the identity buffer or directly
set the host window title.

### 5.2 Input precedence

Modal editor surfaces remain authoritative. Input precedence is:

1. minibuffer, incremental search, completion/menu, query-replace, and other
   existing modal shadows;
2. terminal escape-prefix state;
3. terminal key/mouse/paste handling when the active buffer is terminal;
4. ordinary buffer-local/global keymaps and self-insert.

All terminal buffers remain in `round_trip_buffers`, so GPU/TUI input reaches
this daemon-owned decision before any optimistic edit.
Escape-prefix state is per frontend, so one attached user's pending escape
never captures another user's next key.

When terminal input owns a normalized key, `terminal/input.rs` encodes:

- UTF-8 printable characters;
- Ctrl-character mappings, Alt ESC-prefixing, Enter/Tab/Backspace/Escape;
- arrows/Home/End according to application-cursor mode;
- Insert/Delete/Page and F1–F12 xterm sequences;
- Shift-Tab and supported modifier parameters.

Unknown/lock/media keys are ignored, never converted into text. Press is the
only actionable event in the current normalized protocol; repeat arrives as
repeated press and release is not forwarded.
The normalized protocol does not distinguish number-row digits from numeric
keypad digits, so application-keypad mode is tracked but cannot transform
those ambiguous `Key::Char` events.

`C-c` is the fixed stage-2 terminal escape prefix. It is consumed and makes the
next key run through the ordinary editor dispatcher, allowing `C-c C-x ...`
for editor commands. `C-c C-c` sends the literal Ctrl-C byte required to
interrupt the child. This is an intentional fixed stage-2 policy.

Paste sends exact bytes, wrapped in `ESC[200~` / `ESC[201~` only while the
child enabled bracketed paste. It never passes through a command shell or Lua.
When the child enabled focus reporting, authenticated frontend focus gain/loss
sends `ESC[I` / `ESC[O` for the controlling terminal. With the mode off,
focus changes send no PTY bytes.

### 5.3 Mouse, selection, copy, and scrollback

If the child enabled a supported mouse mode, pointer events inside the terminal
content rectangle are encoded as SGR mouse reports, with coordinates translated
to terminal-local 1-based cells. The active mode determines whether press,
release, drag, move, and wheel are reported.

Otherwise the editor owns the gesture:

- wheel changes the per-window scrollback offset;
- primary drag creates a terminal-cell selection across history and screen;
- copy serializes selected rows as UTF-8, trims only trailing default blank
  cells, joins soft-wrapped rows without `\n`, and separates hard rows with
  `\n`;
- wide-cell continuations are never emitted twice;
- a new plain click clears the old selection;
- child output does not move a scrolled-back viewport or selection anchor.

`pmacs.terminal.copy_selection()` publishes through the existing kill-ring /
clipboard path. Ordinary document selection fields remain untouched.

### 5.4 Resize ownership

One PTY has one kernel window size even when displayed in several views. The
controlling view is the active window of `core.active_frontend` — the frontend
that most recently supplied accepted input/focus. Only that view may resize the
PTY. Passive views clip/pad.

For grid frontends, the daemon derives the terminal content `rows × cols` from
the computed split rectangle and modeline reservation. Focus/split/frontend
resize changes trigger one checked `resize_pty`; unchanged dimensions are
suppressed. The screen model resizes before the child receives `SIGWINCH`, so
its repaint lands into the new geometry.
If the computed content rectangle has zero rows or columns, rendering skips it
and the prior valid PTY size remains unchanged; zero is never sent to
`resize_pty`.

## 6. Stage 3 — protocol v19 and GPU integration

### 6.1 Wire additions

Protocol v19 appends, never inserts, these final variants:

```rust
InstanceMessage::TerminalFrame {
    buffer_id: BufferId,
    size: CellSize,
    cells: Vec<Cell>,
    cursor: Option<CellCoord>,
    title: Option<String>,
    screen_generation: u64,
    selection: Vec<TerminalSelectionSpan>,
    scroll_offset: u32,
    at_bottom: bool,
    pid: u32,
    process: TerminalProcessState,
}

FrontendEvent::TerminalResize {
    frontend_id: FrontendId,
    buffer_id: BufferId,
    size: CellSize,
}

FrontendEvent::TerminalPointer {
    frontend_id: FrontendId,
    buffer_id: BufferId,
    coord: CellCoord,
    kind: MouseKind,
    mods: Modifiers,
}
```

`TerminalFrame` is a complete visible-grid replacement. Empty is not a clear
sentinel: valid terminal sizes are non-zero and `cells.len()` must equal area.
Complete replacement is chosen over a second diff/cache protocol for the first
GPU stage. `screen_generation` advances on screen/process/title mutation;
scroll/selection have their own per-context epochs. The producer caches and
compares the complete context payload, so a view-only change still sends even
when `screen_generation` is unchanged, while an identical payload is silent.

All terminal frame fields are untrusted at the GPU boundary. Validation checks
shared row/column/area limits, exact area, cursor bounds, title length,
selection ordering/non-overlap/bounds, cluster UTF-8 and cluster-byte limits,
continuation structure, and attachment absence.
Invalid input is rejected atomically and the last valid terminal frame remains
painted.

The daemon routes terminal resize/pointer events by authenticated session
source. Claimed frontend and buffer must match the source's active terminal
window. A mismatch is dropped without resizing, selecting, or writing PTY
input.

The protocol remains compatible with v18 where structurally possible:

- v18 grid peers need no new message and continue to receive composed
  `CellDelta` terminal windows;
- v18 semantic peers receive the immutable empty identity snapshot but no
  terminal variant. They cannot display the terminal screen; terminal use from
  those peers is unsupported, while normal document editing remains supported;
- v19 frontends gate the new outbound event variants on negotiated version;
- postcard byte pins cover the old final variants plus the newly appended
  discriminants.

### 6.2 Semantic producer

When a semantic frontend's active buffer is a terminal, its producer emits
`TerminalFrame` plus the existing global theme/font/statusline facts that still
apply. It suppresses document-only style spans, decorations, inlays, block
adornments, folds, file summaries, line numbers, and document cursor layout for
that buffer. On switching back to a document, existing caches are invalidated
so the first document frame is a full authoritative resync.

The terminal frame is scoped to the authenticated frontend/window context,
because scrollback offset and selection are per view. A frame for one split or
frontend must never overwrite another context's baseline.

A GPU frontend reports its terminal viewport in **cells**, computed from its
own font metrics and pixel allocation. No pixel dimensions, glyph advances,
or DPI cross the daemon boundary. The daemon accepts a resize only from the
controlling active frontend defined in §5.4.

### 6.3 GPU renderer

The GPU keeps a dedicated terminal render mode keyed by active `buffer_id`.
It does not synthesize rope text from cells. Layout rules:

- one terminal column equals the active monospace cell advance;
- `Glyph::Continuation` consumes a column and draws nothing;
- clusters shape as one cell origin with the declared one/two-column footprint;
- terminal foreground/background/reverse/style resolve from the cell, not
  syntax or UI faces;
- selection spans resolve through `ui.selection` over child cells; cursor
  placement comes from terminal snapshot state;
- rows never wrap in the frontend; clipping is by terminal cell bounds;
- status band remains outside the terminal grid;
- theme/font changes invalidate terminal shaping and geometry caches;
- a font-size or window-size change recomputes the cell viewport and sends one
  `TerminalResize` after suppression of identical sizes.

Terminal mouse hit-testing is frontend-local pixel -> terminal cell. The GPU
sends `TerminalPointer`, never a fake source byte offset.

## 7. Four-agent execution plan

The vterm roster is fixed at four agents for all three stages. Do not add
review/scout agents; reuse these owners so state-machine decisions stay
coherent.

| Owner | Stable scope | Primary files |
| --- | --- | --- |
| Lead/integrator | contracts first; `TerminalManager`, buffer/Lua/builtin wiring, cross-surface acceptance, gates, docs, branches/PRs | `src/terminal/session.rs`, `src/lua_bindings/mod.rs`, `builtin/runtime/terminal.lua`, `tests/vterm_*_acceptance.rs`, docs |
| VT core agent | streaming parser operations, screen state machine, input encoder, model units | `src/ansi.rs`, `src/terminal/screen.rs`, `src/terminal/input.rs` |
| TUI agent | terminal window composition, cursor, per-view scroll/selection/copy, grid input and resize | `src/terminal/view.rs`, owned sections of `src/editor.rs`, focused TUI tests |
| Protocol/GPU agent | v19 types/limits/gates, semantic terminal producer, authenticated daemon routing, GPU state/render/hit-test | `pmacs-protocol`, `src/protocol.rs`, `src/semantic_render.rs`, owned sections of `src/daemon.rs`, `pmacs-gpu/src/main.rs` |

Coordination rules:

- Lead establishes types and method signatures before another lane edits a
  caller.
- Strict file ownership. `src/editor.rs` passes from lead to TUI only after
  stage-1 construction wiring is settled; `src/daemon.rs` belongs only to the
  protocol/GPU lane in stage 3.
- Workers do not update docs, ledgers, branches, or PRs and do not stash,
  checkout, rebase, or merge.
- Workers add focused tests in their owned modules. Lead alone owns shared
  acceptance files.
- Exact-path staging only; never `git add .`.
- Four agents are the total vterm team, not four implementation workers plus a
  lead.

Per-stage utilization:

- Stage 1: lead + VT core implement; TUI and protocol/GPU owners review the
  snapshot/input contracts against their future consumers.
- Stage 2: TUI implements; VT core owns encoder corrections; lead integrates
  lifecycle/acceptance; protocol/GPU owner checks that no TUI-only assumption
  enters the snapshot contract.
- Stage 3: protocol/GPU implements; TUI and VT core owners add parity cases in
  their existing surfaces; lead integrates and gates.

## 8. Branch and PR plan

After this framing is approved:

1. create sibling worktree `pmacs-vterm-core`, branch `vterm-core`, from current
   canonical `main`; implement/gate/open PR; merge only when the user says;
2. after stage 1 merges, create `pmacs-vterm-tui`, branch `vterm-tui`, from the
   new `main`; implement/gate/open a second PR;
3. after stage 2 merges, create `pmacs-vterm-gpu`, branch `vterm-gpu`, from the
   new `main`; implement/gate/open the third PR.

The framing branch is `vterm-framing` in worktree `pmacs-vterm-framing`.
Implementation branches are not stacked across an unmerged parent. This avoids
base-branch deletion/auto-close risk and makes each PR's gate evidence honest.


## 9. Acceptance

### Stage 1 — terminal core

1. Feed every supported CSI/OSC/DEC sequence at every byte split; whole-feed
   and split-feed screens are identical.
2. Split UTF-8, malformed UTF-8, truncated escape, over-cap control string,
   and unknown private sequences recover without panic, unbounded growth, or
   visible escape leakage.
3. Cursor absolute/relative movement, save/restore, origin mode, margins,
   insert/delete character/line, erase, and scroll mutate only the specified
   cells.
4. Autowrap pending state, wide glyphs, combining clusters, overwrite, erase,
   and clipping never create orphan continuations.
5. Main/alternate screen swaps preserve the main grid and cursor; alternate
   output never enters scrollback. Synchronized output publishes no
   intermediate frame and releases on reset, EOF, and watchdog expiry.
6. DEC line drawing through G0/G1 + SI/SO renders the expected Unicode box
   glyphs across split feeds.
7. SGR indexed/truecolor/underline/reverse survives screen operations; ignored
   attributes leave supported fields unchanged.
8. Main-screen resize reflows only soft wraps and preserves cursor/logical-line
   identity; alternate-screen resize clips/pads without reflow.
9. Scrollback obeys both row and cell budgets, evicts oldest rows, and keeps
   the visible grid exact.
10. DA/DSR/CPR query events produce bounded exact response bytes; unsupported
    OSC/DCS cannot write stdin or clipboard.
11. Strict owned `TerminalSpec` values reject before spawn and are
    mutation-independent after spawn.
12. A real PTY child prints cursor-addressed/alternate-screen content in
    adversarial chunks; final output lands first, then exact normal/non-zero/
    signal process annotations are visible and copyable before exit state.
13. Spawn failure leaves no buffer/session/process residue. Killing a live
    terminal buffer terminates it; editor shutdown leaves no child/reader.
14. Every rope mutation path (ordinary, intercept-skipping, undo/redo, local
    or remote CRDT edit/import) rejects the read-only terminal buffer and
    leaves its rope/revision/modified state unchanged; immutable empty CRDT
    bootstrap remains valid.

### Stage 2 — TUI

15. Lua `open` performs the same strict raw-field validation, publishes no
    partial state on error, and switches the active window only after success.
16. A terminal window paints exact cells/styles inside its content rectangle;
    statusline, sibling splits, and outside cells are untouched.
17. Active cursor translation, child-hidden cursor, passive window, clipping,
    and scrolled-back hiding are exact.
18. Printable, Ctrl, Alt, arrows, Home/End, function keys, application cursor,
    focus reporting, and unknown keys produce the specified PTY bytes.
19. `C-c` dispatches one editor key; `C-c C-c` sends Ctrl-C; modal minibuffer,
    search, menu, and query-replace remain authoritative.
20. Paste is byte-exact with bracketed wrappers only when enabled.
21. Mouse-reporting modes receive translated SGR reports. With reporting off,
    the same gestures scroll/select/copy and write no PTY bytes.
22. Copy handles soft/hard wraps, trailing blanks, wide/combining glyphs,
    resize/reflow, eviction-clamped anchors, and selections crossing
    history/screen exactly once.
23. The controlling active view alone resizes the PTY; passive split/frontend
    renders never cause resize thrash.
24. A hermetic real TUI smoke opens `/bin/sh`, runs a cursor-addressed probe,
    resizes, scrolls/copies, exits, and restores the host terminal cleanly.

### Stage 3 — GPU/protocol

25. Protocol v19 appends all new variants after v18 pins; v18 grid traffic
    round-trips unchanged and new outbound variants are version-gated.
26. Terminal frame validation accepts exact shared boundaries and atomically
    rejects over-area, bad area, out-of-bounds cursor, malformed cluster,
    orphan continuation, invalid selection spans, attachment, overlong title,
    and overlong process-state text while retaining the prior valid frame.
27. Semantic terminal activation suppresses document-only messages; switching
    back forces a complete document resync.
28. Two frontends/splits on one terminal keep independent scroll/selection
    snapshots; only the active controlling context resizes or writes input.
29. Forged frontend/buffer IDs in terminal resize/pointer events cannot affect
    another terminal or process.
30. Headless GPU rendering pins background rectangles, indexed/truecolor,
    reverse, wide/combining cells, clipping, cursor visibility, status-band
    separation, and no frontend wrapping.
31. Font/window resize emits cell dimensions, never pixels, and identical
    resize requests are suppressed.
32. Theme/font/terminal generation changes invalidate exactly the affected
    caches; an unchanged terminal frame produces no redraw message.
33. A real daemon + required-GPU smoke runs a full-screen alternate-screen
    probe, handles input and resize, exits, and returns to the preserved main
    screen.

## 10. Gates and bite verification

Every PR runs the standing full gates from `AGENTS.md`, sequentially, plus its
stage acceptance suite. Stage 2 includes a real hermetic TUI PTY smoke; stage 3
includes `PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu` and the real-daemon GPU
probe.

New behavioral acceptance must be bite-verified against the immediate
pre-stage tree with `scripts/bite` where the swapped files compile. Protocol
v19 tests additionally pin postcard bytes and verify the older-version daemon
filter; a test that merely fails to decode on old code is not a useful bite.

## 11. Explicit deferrals

Not part of these three PRs:

- terminal image protocols (sixel, kitty graphics, iTerm images);
- OSC 52 host clipboard writes and OSC 8 hyperlink interaction;
- faint, blink, conceal, and strikethrough additions to shared `Style`;
- kitty keyboard protocol, key release events, media keys, and IME preedit;
- cursor-shape/blink rendering and numeric-keypad distinction absent from the
  current normalized input/cursor protocol;
- shell integration, prompt marks, command semantic zones, and cwd reporting;
- ordinary document search over terminal history;
- terminal session persistence/reconnect across editor restart;
- reparenting a live terminal process into a second daemon instance;
- user-configurable escape key and scrollback policy.

Deferral means graceful ignore or documented absence, never escape leakage,
panic, unbounded allocation, or child leak.

## 12. Resolved decisions

The 2026-07-21 architecture discussion resolved every Revision 1 question:

1. Fixed terminal editor escape: `C-c`; `C-c C-c` sends literal Ctrl-C.
2. Resize: reflow main-screen soft wraps; clip/pad alternate screen.
3. Exit: retain the buffer and append the process PID/outcome line from §4.1.
4. Compatibility: additive v19; v18 grid remains supported, v18 semantic has
   no terminal surface.
5. GPU wire: complete visible frames with complete-payload suppression.
6. Style: preserve the shared encoding and defer unsupported attributes.
7. Identity: one process/screen per terminal `BufferId`; the most recently
   active frontend's active view controls PTY size.
