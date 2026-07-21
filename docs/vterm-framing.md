# Vterm — framing (Arc 5 stage 2, three-PR delivery)

**Revision 7 — 2026-07-21. Status: Stage 1 landed on `main` as PR #126
at merge `643d1e1`; Stages 2 and 3 are not implemented. Stage 2 framing review
is complete and the contract is ready for implementation.**

Revision 7 closes the final three precision findings: `at_bottom` is geometric
and distinct from live-tail following; the fixed `C-c` transport escape
deliberately makes ordinary `C-c`-leading bindings unreachable in terminal
windows; and context-implicit Lua view operations require an authenticated
interactive origin with exact boolean/error results. Revision 6's durable
controller and per-view identities, logical-cell anchors, frontend-explicit
grid rendering, per-frontend dispatch, authenticated v18 input, local
clipboard/BEL drainage, default-name uniquification, mouse override, and
resize ordering remain unchanged. Main-screen resize reflows while alternate
screen clips/pads; exited buffers remain with an Emacs-style process message;
protocol v19 is additive with complete frames; shared `Style` stays unchanged;
and one `BufferId` owns one shared process/screen whose most recently accepted
frontend/window context controls size.

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

## 0. Revision 7 — landed Stage 1 and reviewed Stage 2 contract

The first of the three vterm PRs landed on `main` at merge `643d1e1`.
Implementation commits `bbc1f33` and `962944b`, first-review fixes through
`bf972a7`, and second-review hardening through `9797ada` shipped in PR #126,
<https://github.com/levineuwirth/pmacs/pull/126>. The landed stage is
deliberately headless: there is no `pmacs.terminal` Lua module, interactive
terminal command, TUI paint branch, or GPU/protocol surface yet.

### 0.1 Public seam and ownership

`src/terminal/session.rs` exports:

- owned `TerminalSpec { command, args, cwd, env, name, rows, cols,
  scrollback_rows }`, with `new` and strict pre-side-effect `validate`;
- `TerminalProcessState::{Running, Exited(i32), Signaled(String),
  Crashed(String)}`;
- owned `TerminalSnapshot { buffer_id, size, cells, cursor, title,
  screen_generation, selection, scroll_offset, at_bottom, pid, process }`;
- `SharedTerminalManager = Rc<RefCell<TerminalManager>>`;
- `TerminalManager::{new, len, is_empty, open, is_terminal, process_id,
  snapshot, tick, send, resize, terminate, prune, shutdown}`.

The Stage 1 `snapshot(BufferId)` is intentionally context-free:
`selection=[]`, `scroll_offset=0`, and `at_bottom=true`. Stage 2 adds
per-`(FrontendId, WindowId, BufferId)` state and an owned
`snapshot_for_view(...)`; it must not add a second screen.

`EditorState` owns the one shared manager. Tick order is supervisor `tick` →
terminal-owned PID drain/watchdog/prune → `process.after-tick`; LSP, MCP, and
ordinary Lua process events retain their existing owners. `ProcessSpec` gained
an `AnsiParserProfile`: `ProcessSpec::new` and Lua `ansi=true` stay
`LineOriented`, while terminal sessions explicitly request `FullScreen`.
Synchronous unpublished terminal spawn failure emits no orphan process event.

Terminal identity buffers are pathless, clean, empty, and buffer-read-only.
The guard runs before direct edits, split Lua begin/skip-intercept edits,
undo/redo, and local/remote CRDT content import. Attaching an immutable empty
CRDT for semantic identity remains valid.

### 0.2 Stage 1 acceptance mapping

All fourteen Stage 1 criteria are implemented:

1. whole/split parser equivalence:
   `screen::tests::parser_split_points_produce_identical_screen` plus the ANSI
   split matrix;
2. malformed/truncated/over-cap recovery: the 44 `ansi::tests`, including
   split UTF-8, ignored CSI/OSC/DCS, and forward-progress cases;
3. cursor/region/edit exactness:
   `cursor_erase_insert_delete_scroll_and_margins_mutate_exact_regions`;
4. pending wrap/wide/combining invariants: wide overwrite, split ZWJ/RI/
   modifier/variation, right-edge, and continuation tests;
5. alternate/synchronized publication:
   `alternate_screen_preserves_main_and_has_no_history`,
   `synchronized_output_gates_snapshot_and_finish_releases`, and watchdog;
6. DEC G0/G1 + SI/SO: `acs_and_device_replies_are_exact`;
7. SGR fidelity and ignored attributes: ANSI SGR/color/underline tests plus
   screen operation coverage;
8. resize semantics: soft-wrap reflow, wide-boundary/cursor/hard-break tests,
   alternate clipping, and atomic invalid resize;
9. dual history limits: `history_obeys_row_and_cell_caps`;
10. bounded DA/DSR/CPR and unsupported-output safety:
    `acs_and_device_replies_are_exact` plus parser ignore cases;
11. strict owned specifications:
    `strict_owned_spec_rejects_before_spawn_and_is_mutation_independent`;
12. real adversarial PTY/final drain:
    `final_output_precedes_exact_nonzero_annotation_and_buffer_is_retained`
    splits ESC/CSI writes, observes addressed alternate-screen output while
    running, unblocks raw stdin through `send`, restores the main screen, and
    proves final output precedes the exact PID/outcome annotation; zero,
    non-zero, signal, wrapped, and one-row annotations are separately pinned;
13. lifecycle cleanup: transactional failure, live buffer-kill prune/reap, and
    TERM-ignoring editor shutdown acceptance;
14. read-only/CRDT invariants: default + CRDT shared acceptance and focused
    buffer unit tests cover every mutation route and empty bootstrap.

### 0.3 Final gates and bite

The initial delivery gate run fixed missing acceptance-crate documentation;
PR CI then exposed Darwin's numeric `strsignal` suffix, fixed in `962944b`.
Review round 1's first Clippy pass found only identical LF/IND match arms,
consolidated in `bf972a7`. Review round 2 added one screen unit and one shared
acceptance case; the complete sequence restarted from gate 1:

- `cargo fmt --check`: clean;
- `cargo clippy --workspace --all-targets -- -D warnings`: clean;
- default library: 1,661 passed, 3 ignored;
- CRDT library: 1,837 passed, 3 ignored;
- Stage 1 acceptance: 9 default + 10 CRDT passed;
- M4 acceptance: 114 passed, 3 ignored, 1 `basedpyright` filtered;
- required GPU: 109 passed;
- workspace: 2,769 passed across 79 suites, 19 ignored, 1 filtered;
- `git diff --check`: clean.

`scripts/bite main src/lib.rs --test vterm_stage1_acceptance` returned
`bite: OK`: the swapped pre-stage crate root cannot compile the new terminal
API. This is explicitly the helper's weaker compile-time API bite, not a clean
behavioral assertion failure.

`scripts/bite HEAD^ src/ansi.rs --lib
parser_split_points_produce_identical_screen` returned `bite: OK` with a clean
behavioral assertion failure: the pre-dispatch parser left the cursor at row
zero/column four instead of applying NEL/RI/IND and landing at row one/column
zero.

`scripts/bite HEAD^ src/terminal/screen.rs --test vterm_stage1_acceptance
terminal_cells_reject_child_control_characters` returned `bite: OK` with a
clean behavioral failure: the pre-hardening screen stored control bytes in a
grapheme cluster rather than preserving the blank snapshot.

### 0.4 Stage 2 re-scout resolutions

The post-Stage 1 review and a fresh read of landed `main` found six
load-bearing Stage 2 seams. This revision resolves them before code:

- PTY resize ownership is stored durably per terminal session as an
  authenticated `(FrontendId, WindowId)` controller. Render-time
  `EditorCore::active_frontend` is not an ownership signal.
- The single global `KeyDispatcher` cannot safely carry a terminal `C-c`
  continuation in a multi-frontend daemon. Pending dispatch and terminal
  escape state become per-`FrontendId`; dispatch-idle publication becomes
  frontend-specific too.
- Terminal copy reuses the existing kill-ring/clipboard setter. The
  in-process run loop must drain that signal and feed it through
  `Frontend::present_messages`; the TUI must implement `InstanceSignal::Bell`
  rather than drop it.
- `RenderState::render_frame` and `paint_frame` need the target
  `FrontendId` explicitly. A transient mutable active frontend may still
  attribute commands, but it may not select another frontend's layout,
  terminal view state, statusline context, or resize owner during fan-out.
- Current v18 daemon key/mouse payload IDs are client supplied. Stage 2 routes
  key, mouse, paste, focus, and resize through the authenticated connection
  source before any terminal ownership or PTY effect.
- The Stage 1 screen already preserves logical-line IDs and row cell offsets
  through main-screen reflow. Stage 2 selection and scroll anchors use those
  coordinates; numeric distance from the moving tail is derived metadata,
  never stored ownership state.

The final framing review pinned three precision contracts:

- `at_bottom` reports whether the live tail is currently visible; it is not
  the separate internal “follow future output” predicate.
- `C-c` is a consumed transport escape, not an ordinary dispatcher prefix
  map; `C-c`-leading user bindings are deliberately unavailable in terminal
  windows for Stage 2.
- Context-implicit Lua view operations require an authenticated interactive
  command origin and fail closed rather than borrowing a stale
  `active_frontend`.

Stage 3 additionally owns `pmacs-gpu/src/attach.rs` for gated terminal
resize/pointer sending and coalescing. New daemon event variants must apply the
same authenticated-source rule. Wire-facing terminal state, selection, and
limits live in or are re-exported from `pmacs-protocol`. The current 16 MiB
transport frame cap cannot hold the legal worst complete terminal frame (up to
roughly 64 MiB of cluster bytes before encoding overhead): Stage 3 must either
raise and test a measured cap at least as large as the legal worst case
(review estimate at least 80 MiB), or add a shared aggregate payload bound. It
must never silently chunk the locked complete-frame protocol.

### 0.5 Stage 1 review round 1

The first external review found no ownership, mutation-guard, parser-cap, or
security regressions. This round resolves its three merge-adjacent findings:

- `ESC D` (IND), `ESC E` (NEL), and `ESC M` (RI) are typed full-screen
  operations. RI scrolls down only at the top margin; IND/NEL scroll up at the
  bottom margin, with NEL additionally returning to column zero. The parser's
  every-byte-split matrix and focused screen-margin test pin the complete path.
- Terminal children no longer inherit an arbitrary host `TERM`; absent a
  caller override, their process environment gets `TERM=xterm-256color`.
- The TERM-ignoring shutdown acceptance uses `kill(pid, 0)` through `nix`
  instead of Linux-only `/proc`, so macOS now exercises the assertion.
- Resize retains every surviving application tab stop and installs default
  stops only in newly added columns.

`spawn_ansi_parser` intentionally calls `AnsiParser::finish()` on channel
disconnect for both profiles. For existing line-oriented compile/REPL
consumers, EOF therefore delivers trailing partial text and required synthetic
style/alternate-screen balancing that older code dropped. This is an
intentional latent-bug fix and an observable compatibility contract.

The Stage 2 Lua `open` surface must uniquify colliding default buffer names
(`*terminal:sh*`, `*terminal:sh*<2>`, and so on) before terminal creation
becomes user-visible.

### 0.6 Stage 1 review round 2

The second external review found no ownership, lifecycle, mutation-guard,
transactional-spawn, final-drain, parser-cap, or reflow defects and judged
Stage 1 merge-ready. Its renderer-boundary hardening and cheap cleanups are
resolved before Stage 2:

- `TerminalScreen::write_text` drops every `char::is_control()` value before
  grapheme segmentation, so parser-produced C1 and direct-event C0/C1 bytes
  cannot enter copyable or renderable cells. Unit and shared acceptance tests
  pin a byte-identical blank snapshot.
- SGR mouse release reports retain the released left/middle/right button code
  and use the lowercase `m` final.
- The dead `line_feed` mode parameter and contradictory wide-grapheme branch
  are removed; all logical-line ID allocation saturates consistently; and
  terminal prune clears stale round-trip input membership.

Out-of-range DECSTBM bottom clamping, CSI-intermediate clone removal, and a
separately named configuration-time scrollback-row cap remain explicit
deferrals in §11.

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

Stage 2 constructs the shared `TerminalManager` immediately after the process
supervisor, installs strict Rust primitives, then loads
`builtin/runtime/terminal.lua`; all happen before LSP/MCP builtins and before
user config. The Lua chunk owns friendly wrappers, the interactive command,
buffer-local bindings, and the built-in statusline provider.

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
pmacs.terminal.state(buffer)             -- fresh global metadata
pmacs.terminal.view_state(ctx)           -- fresh per-window metadata
pmacs.terminal.send(buffer, bytes)       -- explicit trusted raw bytes
pmacs.terminal.terminate(buffer)         -- SIGTERM; buffer remains
pmacs.terminal.scroll(lines)             -- boolean; interactive origin required
pmacs.terminal.scroll_to_bottom()        -- boolean; interactive origin required
pmacs.terminal.copy_selection()          -- boolean; interactive origin required
```

`open` accepts exactly the fields shown. It validates raw table fields before
side effects: unknown fields, metatable-provided fields, holes in `args`,
non-string environment keys/values, embedded NUL, non-integer dimensions, and
out-of-range scrollback reject with the field named. The copied specification
is immune to caller mutation. Returned and accepted identity is `BufferIdLua`,
following the rest of the editor API.

Generated names are reserved against the live buffer registry before insert:
`*terminal:sh*`, `*terminal:sh*<2>`, `*terminal:sh*<3>`, and so on. The lowest
available suffix wins; failed creation consumes no suffix. An explicit `name`
is preserved exactly after ordinary `TerminalSpec` validation.

`state(buffer)` returns a fresh plain table:

```lua
{
  buffer = buffer, pid = 123, rows = 24, cols = 80,
  title = nil, screen_generation = 7,
  process = { kind = "running" },
  -- or { kind="exited", code=0 },
  -- or { kind="signaled", signal="TERM" },
  -- or { kind="crashed", message="..." }
}
```

`view_state(ctx)` accepts the raw statusline context fields
`{frontend, window, buffer}` and returns
`{at_bottom=boolean, scroll_offset=integer, selection=boolean}`, or `nil` when
that exact context is not a live terminal view. `state`/`view_state` expose no
stored Lua tables, callbacks, or mutable manager state.

The three context-implicit view operations require a live authenticated
interactive command origin. A plain programmatic `pmacs.command.invoke`
outside such a dispatch does not synthesize one and raises a Lua error; so does
an origin whose active window is not the addressed terminal context. Errors
name the operation and leave view, selection, clipboard, and controller state
unchanged. `scroll(lines)` requires an integer: positive moves toward older
rows, negative toward the live tail, and zero returns `false`; otherwise it
returns whether `top` changed. `scroll_to_bottom()` returns whether selection
or `top` was cleared. `copy_selection()` returns `true` only when bytes were
published and `false` for a valid terminal context with no selection.

There is deliberately no public Lua `resize`: the active controller's computed
window content rectangle is the only geometry authority. `send` remains an
explicit trusted escape hatch for packages and tests; ordinary user paste and
keys use the mode-aware input path.

The built-in `terminal` command calls the public wrapper with `$SHELL` or
`/bin/sh`, no shell-command interpolation, no string split, and no implicit
`sh -c`. The wrapper installs terminal-buffer-local one-key commands:
`M-w` copies, `M-v`/`C-v` page up/down, and `M-<`/`M->` move to the oldest
retained row/bottom. They run only after the terminal escape prefix; normal
terminal input sends those keys to the child.

### 4.4 Stage 2 view and controller contracts

Stage 2 adds `src/terminal/view.rs`. It is a projection over the one
`TerminalScreen`, not another screen:

```rust
TerminalViewKey { frontend_id, window_id, buffer_id }
LogicalCellAnchor { logical_line_id, cell_offset }
TerminalSelection { anchor, head }          // inclusive display cells
TerminalViewState { top, selection, drag }  // keyed by TerminalViewKey
TerminalController { frontend_id, window_id }
```

`cell_offset` is the leading display-cell offset within one logical line.
Clicks on a wide continuation canonicalize to its lead. Ordering is resolved
against the current retained row sequence, not by comparing IDs. A top anchor
resolves to the physical row containing that logical offset after reflow.

`TerminalManager` gains owned operations to register/retain/detach view keys,
claim or release a session controller, create a
`snapshot_for_view(key, viewport_size)`, scroll/select/copy one view, and query
fresh global/view metadata. `detach_frontend` and live-layout retention remove
stale view/controller state; buffer prune removes every context for that
session. The context-free Stage 1 `snapshot(buffer)` remains available for
headless/core callers.

`snapshot_for_view` returns exactly `viewport_size.area()` cells for a nonzero
valid viewport. At bottom it tail-aligns rows, padding above and to the right
with default cells; a smaller view clips top/rows and right/columns. A
scrolled view starts at its resolved top anchor and pads only after retained
content is exhausted. Cursor and selection spans are translated into this
returned coordinate space. `scroll_offset` is a saturating derived count of
display rows between the viewport and live tail.

`at_bottom` is purely geometric: it is `true` exactly when
`scroll_offset == 0`, meaning the live tail is currently visible. It is
independent of the follow predicate (`top == None && selection == None`).
Therefore a Shift-selection frozen at the tail has `top != None`,
`scroll_offset == 0`, and `at_bottom == true`: the active cursor remains
visible, unshifted supported child mouse reporting remains eligible, and the
statusline emits no `↑0`. The first later output retained behind that anchor
makes `scroll_offset > 0` and `at_bottom == false`.

Main-screen reflow keeps logical anchors stable. Beginning a selection freezes
the current first visible row as `top`, even when it was the live tail, so
later output has an anchor to preserve. Oldest-row eviction clamps a missing
anchor once to the first surviving leading cell; a selection whose two ends
collapse is cleared. Alternate-screen entry/exit or a reset that removes the
referenced logical IDs clears affected selection/top state and returns the view
to bottom. Child output follows only views with `top=None` and no selection.

## 5. Stage 2 — TUI integration

### 5.1 Composition, cursor, and status

`RenderState::render_frame` and `paint_frame` take an explicit target
`FrontendId`. Layout lookup, statusline evaluation, active-window choice, and
terminal view keys use that ID. A shared placement helper computes each
window's outer rectangle, one-row modeline reservation, and content rectangle;
both resize synchronization and painting consume the same result.

For every terminal buffer, the content rectangle is painted from
`snapshot_for_view` instead of `TextView::render`. Cell glyph/style values are
copied directly; no ANSI is reinterpreted. Line-number gutters, document
wrapping, syntax, diagnostics, inlays, ordinary selections, and local/peer
document overlays are suppressed for that window. Modeline/statusline,
sibling splits, and cells outside the rectangle retain the existing pipeline.

Each `(frontend_id, window_id, buffer_id)` view follows §4.4. Merely rendering
a passive view never resizes the PTY. Zero-area content is skipped without
creating a view or changing the prior valid size.

The active terminal cursor is translated from snapshot-local coordinates into
the content rectangle. It is hidden when the child hid it, the window is not
the target frontend's active window, the viewport is scrolled from bottom, or
the coordinate is clipped. Other terminal windows do not paint a cursor.

BEL is an out-of-band `InstanceSignal::Bell` only when a new bell occurs in
the target frontend's active terminal. Historical bells are never replayed on
later activation, and one bell count is delivered at most once per frontend.
The in-process loop drains both clipboard and terminal signals and feeds them
through `Frontend::present_messages`; the TUI Bell arm emits one host BEL.
OSC title remains sanitized metadata (and a Stage 3 frame field). It never
renames the identity buffer, injects host control bytes, or sets the host title.

`terminal.lua` registers a right-side provider named `terminal`, priority 10,
face `ui.modeline.terminal`. It emits `TERM` while running, `TERM:<code>` on
normal exit, `TERM:<signal>` on signal, or `TERM:ERR` on crash, and appends
` ↑<scroll_offset>` only when `scroll_offset > 0`. An unset child face
inherits the existing modeline foreground through the Arc 4 contract.

### 5.2 Input precedence and per-frontend dispatch

Stage 2 replaces the one pending `KeyDispatcher` with dispatch state keyed by
`FrontendId`; keymaps and command registries remain shared. Terminal escape
state is stored beside that dispatcher. `dispatch_idle_for(frontend_id)` uses
the same frontend's pending prefix and active window; global modal surfaces
still make every frontend non-idle while they own input.

`EditorState` carries an ephemeral authenticated command origin around
key/menu/M-x interactive invocation and clears it with the invocation guard.
Nested calls inherit that origin. Plain `pmacs.command.invoke` outside the
guard neither stamps command history nor creates terminal view authority.

Input precedence is:

1. minibuffer, incremental search, completion/menu, query-replace, and other
   existing modal shadows;
2. that frontend's terminal escape-prefix state;
3. terminal key/mouse/paste handling when that frontend's active buffer is a
   terminal;
4. that frontend's ordinary buffer-local/global dispatcher and self-insert.

All terminal buffers stay in `round_trip_buffers`, so attached frontends reach
this daemon-owned decision before optimistic edit. One user's pending escape
or longer ordinary prefix cannot consume, cancel, or display as another
user's pending sequence.

When terminal input owns a normalized key, `terminal/input.rs` encodes UTF-8
printable characters; Ctrl mappings and Alt ESC-prefixing; Enter, Tab,
Backspace, Escape; arrows/Home/End according to application-cursor mode;
Insert/Delete/Page, F1–F12, Shift-Tab, and supported modifier parameters.
Unknown/lock/media keys are ignored. Press is actionable; local repeat is
treated as another press and release is not forwarded. The normalized
protocol cannot distinguish number-row from numeric-keypad characters, so
application-keypad mode remains tracked but cannot transform them.

`C-c` is the fixed terminal escape prefix. It is consumed and makes the next
key run through the same frontend's ordinary dispatcher; a resulting longer
prefix stays in that frontend's dispatcher. `C-c C-c` instead sends literal
Ctrl-C. Modal shadows consume their keys before either rule.

This is deliberately a consumed transport escape, not an Emacs-style
`C-c` prefix map: the dispatcher receives the post-escape key as a fresh
sequence, while `C-c C-c` is reserved for literal interrupt. Consequently a
global or buffer-local binding whose first chord is `C-c` cannot fire in a
terminal window. Packages may bind a post-escape one-key sequence, as the
built-in terminal-local commands do. A configurable escape/prefix-map policy
remains the named §11 deferral.

Paste sends exact bytes, wrapped in `ESC[200~` / `ESC[201~` only while the
child enabled bracketed paste. It never passes through a shell or Lua.
Authenticated focus gain claims the active terminal context and emits
`ESC[I` when enabled. Focus loss emits `ESC[O` only when that source currently
controls the session, then releases that controller; mode-off focus is silent.

### 5.3 Mouse, selection, copy, and scrollback

Child mouse reporting owns a pointer only when the exact terminal view is at
bottom, the child enabled a supported tracking mode plus SGR encoding, and
Shift is not held. Events inside the content rectangle translate to
terminal-local 1-based cells; the active mode filters press, release, drag,
move, and wheel.

Otherwise the editor owns the gesture:

- wheel scrolls that `TerminalViewKey`;
- primary drag stores inclusive logical-cell endpoints across history/screen;
- Shift is the explicit editor-selection override while child mouse reporting
  is active;
- an editor-owned right press follows the existing context-menu path;
- a new plain primary click clears the old selection before setting its anchor;
- child output does not move a scrolled viewport or selection.

Copy resolves anchors against retained logical rows, emits each leading glyph
once, trims only trailing default blank cells, joins soft wraps without `\n`,
and separates hard rows with `\n`. Wide continuations are never duplicated;
combining clusters remain one UTF-8 sequence. Reversed drags normalize by
retained row order. `pmacs.terminal.copy_selection()` writes through the
existing kill-ring/clipboard setter for the acting frontend; ordinary document
selection state is untouched.

Scrolling uses physical retained display rows and clamps at oldest/bottom.
Reaching bottom clears `top` only when no selection is active. The explicit
`scroll_to_bottom`/`M->` action clears both selection and `top` so live-tail
following resumes; ordinary copy leaves selection intact. Page commands use
the current nonzero content-row count. Selection and scroll are available in
the alternate screen only over its visible rows; no alternate output enters
main history.

### 5.4 Durable control and resize

Each terminal session stores at most one `TerminalController`. Successful
`open` claims the newly active `(frontend, window)`. Later authenticated
terminal key, paste, editor-owned/child-owned pointer, or focus gain claims the
source's currently active terminal window. A render pass never claims control.
Switching that controller away, killing its window/buffer, focus loss, or
frontend detach releases it; size stays unchanged until another accepted
context claims it.

Before the next terminal process drain and before paint, the instance
synchronizes live terminal layouts. Only a controller whose exact window still
shows that session may resize. Unchanged dimensions are suppressed before any
PTY syscall. The manager validates once, performs `resize_pty`, then applies
the same prevalidated `TerminalScreen::resize` in the same main-thread call;
no supervisor output drain can interleave between them. A PTY failure leaves
the prior screen geometry intact. The child may emit after `SIGWINCH` only on a
later drain, when the screen already has the new geometry.

Content rows/columns come from the shared placement helper after modeline
reservation and with no document gutter. A zero row/column result performs no
resize and preserves the prior valid geometry. Passive splits/frontends only
clip or pad their own snapshots.

### 5.5 Local and v18-daemon adapters

The in-process event adapter handles key press/repeat, mouse, paste,
focus-gained/lost, and resize through the same `EditorState` terminal methods.
It synchronizes layout before `tick_processes`, then presents pending
clipboard/BEL messages. Host restoration remains the existing `Frontend`
drop/error contract.

The daemon passes the authenticated connection `source` explicitly to grid
render and input. Client-supplied IDs in v18 Key, Mouse, Paste, Focus, Resize,
and Detach payloads never choose a view or controller. Session detach removes
that frontend's dispatcher, terminal views, controller claims, and bell
baseline. These are Stage 2 changes to existing v18 grid behavior; no protocol
bump or semantic/GPU terminal surface is added in this PR.

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
| Lead/integrator | contracts first; `TerminalManager`, Lua/builtin wiring, lifecycle/signal integration, shared acceptance, gates, docs, branches/PRs | `src/terminal/session.rs`, `src/lua_bindings/mod.rs`, `builtin/runtime/terminal.lua`, narrow Stage 2 authenticated-dispatch sections of `src/daemon.rs`, `tests/vterm_*_acceptance.rs`, docs |
| VT core agent | encoder corrections and screen query seams needed by view projection; no renderer ownership | `src/ansi.rs`, `src/terminal/screen.rs`, `src/terminal/input.rs` |
| TUI agent | view projection, frontend-explicit grid composition/cursor, per-frontend dispatch, local events, scroll/selection/copy/resize | `src/terminal/view.rs`, `src/instance_render.rs`, `src/frontend.rs`, owned sections of `src/editor.rs`, focused TUI tests |
| Protocol/GPU agent | Stage 2 contract review; Stage 3 v19 types/limits/gates, semantic producer, authenticated new-event routing, GPU state/render/hit-test | `pmacs-protocol`, `src/protocol.rs`, `src/semantic_render.rs`, Stage 3 sections of `src/daemon.rs`, `pmacs-gpu/src/{main,attach}.rs` |

Coordination rules:

- Lead establishes types, invariants, and method signatures before another
  lane edits callers.
- Strict file ownership. `src/editor.rs` passes from lead to TUI only after
  construction wiring; lead and TUI coordinate exact non-overlapping
  `src/daemon.rs`/signal hunks. Stage 3 daemon work begins only after Stage 2
  lands.
- Workers do not update docs, ledgers, branches, or PRs and do not stash,
  checkout, rebase, or merge.
- Workers add focused tests in owned modules. Lead alone owns shared
  acceptance files.
- Exact-path staging only; never `git add .`.
- Four agents are the total vterm team, not four implementation workers plus a
  lead.

Per-stage utilization:

- Stage 1: completed by lead + VT core; TUI and protocol/GPU reviewed future
  consumer contracts.
- Stage 2: lead establishes manager/Lua contracts and authenticated adapters;
  TUI implements projection/input/rendering; VT core adds only required
  encoder/query corrections; protocol/GPU checks snapshot neutrality.
- Stage 3: protocol/GPU implements; TUI and VT core add parity cases in their
  existing surfaces; lead integrates and gates.

## 8. Branch and PR plan

Stage 1 landed on `main` as PR #126 at merge `643d1e1`. Continue one clean PR
at a time:

1. Revision 7 framing review complete; implementation contract approved;
2. create worktree `pmacs-vterm-tui`, branch `vterm-tui`, from current
   `githubsucks/main`; carry this approved framing as the first commit,
   implement/gate, and open the second PR;
3. merge Stage 2 only when the user says;
4. create `pmacs-vterm-gpu`, branch `vterm-gpu`, from the then-current `main`;
   implement/gate/open the third PR.

The framing branch is `vterm-framing` in worktree `pmacs-vterm-framing`.
Implementation branches are not stacked across an unmerged parent. This avoids
base-branch deletion/auto-close risk and makes each PR's gate evidence honest.


## 9. Acceptance

### Stage 1 — terminal core

1. Feed every supported CSI/OSC/DEC sequence, including IND/NEL/RI, at every
   byte split; whole-feed and split-feed screens are identical. RI and
   forward-index operations additionally pin exact scrolling-margin behavior.
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
8. Main-screen resize reflows only soft wraps, preserves cursor/logical-line
   identity and application tab stops, and adds defaults only in new columns;
   alternate-screen resize clips/pads without reflow.
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

15. Lua `open` enforces the strict owned table contract, uniquifies generated
    names without consuming failed suffixes, publishes no partial state on
    error, and switches/claims the active window only after success.
    `state` and `view_state` return exact fresh plain tables; no public Lua
    resize bypass exists. Context-implicit view operations error without an
    authenticated interactive terminal origin and otherwise return the exact
    changed/copied booleans without partial mutation.
16. A terminal window paints exact cells/styles inside its content rectangle;
    top/right padding, clipping, modeline, sibling splits, outside cells, and
    suppression of document/peer overlays are exact and control-free.
17. Per-context snapshots preserve logical top/selection anchors through
    main-screen reflow, derive the correct scroll offset, clamp oldest-row
    eviction once, and clear invalid alternate/reset anchors. A selection
    frozen at the live tail pins `top != None`, `scroll_offset == 0`, and
    `at_bottom == true`; cursor, child-mouse eligibility, status suffix, and
    the first subsequent output transition follow that definition exactly.
18. Printable, Ctrl, Alt, navigation/function keys, application cursor, focus,
    unknown keys, and paste produce exact PTY bytes; bracketed wrappers appear
    only when enabled.
19. `C-c` dispatches one fresh editor key and retains the owning frontend
    through a longer prefix; `C-c C-c` sends Ctrl-C. Ordinary bindings whose
    first chord is `C-c` are deliberately unreachable in terminal windows.
    Two frontends cannot consume one another's escape/pending state or
    dispatch-idle value, and existing modal shadows remain authoritative.
20. Supported SGR mouse modes receive translated reports only at bottom without
    Shift. Mode-off, scrolled, unsupported-legacy, and Shift gestures remain
    editor-owned; wheel, drag, clear, and right-context behavior write no PTY
    bytes.
21. Copy handles soft/hard wraps, trailing blanks, wide/combining glyphs,
    reversed drags, resize/reflow, eviction, alternate screen, and
    history/screen crossings exactly once, then publishes to the acting
    frontend's kill-ring/clipboard path without touching document selection.
22. The durable authenticated controller alone resizes. Open/input/focus
    claims and focus/switch/kill/detach releases are exact; unchanged, zero,
    passive, and failed resize cases preserve prior geometry without thrash,
    and screen resize precedes the next child-output drain.
23. Forged v18 grid payload frontend IDs for key, mouse, paste, focus, resize,
    or detach cannot select or affect another frontend's terminal context.
    Detach and layout retention remove only the matching views, dispatcher,
    controller, and bell baseline.
24. Local and daemon-grid paths deliver clipboard and each new active-terminal
    BEL exactly once. Historical/passive bells and OSC titles do not become
    host control effects. The built-in terminal provider reports exact
    process/scroll state for each split.
25. Two frontends and sibling splits over one terminal retain independent
    bottom/scroll/selection snapshots while sharing one process, screen, title,
    process outcome, and controller.
26. Killing the identity buffer terminates/reaps the child and removes all
    views; switching away leaves it running; editor/drop and error paths
    restore the host terminal and leak no child, reader, or stale round-trip
    state.
27. A hermetic real TUI smoke opens `/bin/sh`, runs a cursor-addressed probe,
    exercises key/paste, resize, scroll/select/copy, BEL, and clean exit, then
    proves host raw/alternate-screen state is restored.

### Stage 3 — GPU/protocol

28. Protocol v19 appends all new variants after v18 pins; v18 grid traffic
    round-trips unchanged and new outbound variants are version-gated.
29. Terminal frame validation accepts exact shared boundaries and atomically
    rejects over-area, bad area, out-of-bounds cursor, malformed cluster,
    orphan continuation, invalid selection spans, attachment, overlong title,
    and overlong process-state text while retaining the prior valid frame.
30. Semantic terminal activation suppresses document-only messages; switching
    back forces a complete document resync.
31. Two frontends/splits on one terminal keep independent scroll/selection
    snapshots; only the active controlling context resizes or writes input.
32. Forged frontend/buffer IDs in terminal resize/pointer events cannot affect
    another terminal or process.
33. Headless GPU rendering pins background rectangles, indexed/truecolor,
    reverse, wide/combining cells, clipping, cursor visibility, status-band
    separation, and no frontend wrapping.
34. Font/window resize emits cell dimensions, never pixels, and identical
    resize requests are suppressed.
35. Theme/font/terminal generation changes invalidate exactly the affected
    caches; an unchanged terminal frame produces no redraw message.
36. A real daemon + required-GPU smoke runs a full-screen alternate-screen
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
- RIS (`ESC c`), DECALN (`ESC # 8`), and DEC cursor save/restore mode `?1048`;
- CUU/CUD region clamping when the cursor starts inside scroll margins while
  origin mode is disabled;
- combining a character into the preceding cell across intervening SGR or
  cursor-control events;
- DECSTBM clamping when an explicit bottom margin exceeds the current screen
  height; the current core leaves the existing scrolling region unchanged;
- exact xterm `?1047` clear-on-exit and scroll-margin preservation across
  alternate-screen switches;
- legacy X10 mouse byte encoding when a child enables mouse tracking without
  SGR mode; Stage 2 sends no report for that unsupported combination;
- nonstandard `CSI 3 K` ignore semantics (the current core clears the line);
- the ASCII fast path that avoids grapheme-candidate allocation and
  segmentation for every printable character after another ASCII character,
  and avoiding the per-sequence `intermediates` clone in CSI dispatch;
- cleanup of the defensive impossible-state path where a terminal spawn
  returns a process without a running PID, and borrow-tolerant `EditorState`
  drop; normal spawn/rollback/prune/shutdown paths remain covered;
- a separately named configuration-time scrollback-row cap; the current
  validation conservatively reuses the history-cell cap before the runtime
  row and cell budgets enforce the effective limit;
- shell integration, prompt marks, command semantic zones, and cwd reporting;
- ordinary document search over terminal history;
- terminal session persistence/reconnect across editor restart;
- reparenting a live terminal process into a second daemon instance;
- user-configurable escape key and scrollback policy.

Deferral means graceful ignore or documented absence, never escape leakage,
panic, unbounded allocation, or child leak.

## 12. Resolved decisions

The 2026-07-21 architecture, re-scout, and final framing review resolved every
current question:

1. Fixed terminal editor escape: `C-c`; `C-c C-c` sends literal Ctrl-C.
2. Resize: reflow main-screen soft wraps; clip/pad alternate screen.
3. Exit: retain the buffer and append the process PID/outcome line from §4.1.
4. Compatibility: additive v19; v18 grid remains supported, v18 semantic has
   no terminal surface.
5. GPU wire: complete visible frames with complete-payload suppression.
6. Style: preserve the shared encoding and defer unsupported attributes.
7. Identity: one process/screen per terminal `BufferId`.
8. View state: logical-line/cell anchors per
   `(FrontendId, WindowId, BufferId)`; `at_bottom` means
   `scroll_offset == 0`, while following additionally requires no top anchor
   and no selection; no second screen.
9. Control: the most recently authenticated accepted terminal context owns
   resize until focus/switch/kill/detach releases it; render never claims.
10. Dispatch: terminal escape and ordinary pending prefixes are per frontend;
    the consumed `C-c` escape deliberately hides ordinary `C-c`-leading
    bindings in terminal windows.
11. Mouse: Shift or scrollback forces editor selection; supported SGR child
    reporting owns unshifted at-bottom gestures.
12. Resize order: validate, suppress unchanged, resize PTY, resize the screen
    before any subsequent child-output drain; failure preserves old geometry.
13. Lua: strict open/state/view/send/terminate/scroll/copy surface; no public
    geometry bypass; implicit view operations require authenticated interactive
    origin and otherwise error without mutation.
14. Host effects: clipboard and BEL use explicit frontend signals; OSC title
    remains sanitized metadata.
