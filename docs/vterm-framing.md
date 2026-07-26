# Vterm — framing (Arc 5 stage 2, three-PR delivery)

**Revision 9 — 2026-07-22. Status: Stage 1 landed on `main` as PR #126
at merge `643d1e1`; Stage 2 landed as PR #130 at merge `86fc1bc`; Stage 3
is IMPLEMENTED on `vterm-gpu` and awaits review. Revision 8's framing text
below is preserved verbatim as the approved contract; §0.9 records what the
implementation actually did, including the two places it had to go beyond
the letter of the framing.**

Revision 8 re-scouts the final stage against the integrated protocol-v18 tree
and closes its remaining producer/frontend boundary decisions. Protocol v19
uses one validated complete `TerminalFrame`; a shared 8 MiB aggregate glyph
budget keeps the largest legal postcard message below the existing 16 MiB
transport cap instead of broadening every connection's allocation ceiling.
The first-frame bootstrap is explicit: after every `BufferSnapshot`, a v19 GPU
sends the document viewport and, when the drawable terminal cell size is
nonzero, terminal-cell geometry; the daemon accepts only the declaration
appropriate to the authenticated active buffer.
Semantic terminal rendering is per frontend/window. Passive views never resize
the PTY, and terminal mode retains existing statusline, menu, minibuffer, and
BEL/clipboard channels while suppressing document projection and presence.

Revision 7's geometric `at_bottom`, fixed `C-c` transport escape,
authenticated context-implicit Lua operations, durable controller and per-view
identities, logical-cell anchors, frontend-explicit grid rendering,
per-frontend dispatch, authenticated v18 input, local clipboard/BEL drainage,
default-name uniquification, mouse override, and resize ordering remain
unchanged. Main-screen resize reflows while alternate screen clips/pads;
exited buffers remain with an Emacs-style process message; shared `Style`
stays unchanged; and one `BufferId` owns one shared process/screen whose most
recently accepted frontend/window context controls size.

This framing follows the compile-mode terminal substrate that landed in PR
#113. `src/process.rs` already owns PTY creation, process groups, bounded
readers, stdin writes, resize, exit/restart state, and final drain.
`src/ansi.rs` already owns a streaming UTF-8/CSI/OSC parser, but deliberately
emits only the line-oriented subset compile-mode needs. Vterm does not replace
either subsystem. It extends their contracts and adds the missing terminal
screen state machine.

Arc 5 stage 2 (vterm) ships as three separately reviewed internal stages,
one PR each:

1. **terminal core** — full-screen VT events, `TerminalScreen`, internal
   session ownership/contracts, and headless real-PTY acceptance;
2. **TUI integration** — terminal-window composition, input, resize,
   scrollback, selection, and copy;
3. **GPU integration** — protocol v19 terminal messages, semantic-daemon
   routing, and a native GPU terminal renderer.

There is no single mega-PR. Each stage is useful and testable by itself, and a
later stage starts only after the preceding stage lands on `main`.

## 0. Revision 8 — landed internal Stages 1–2 and framed Stage 3

The first of the three vterm PRs landed on `main` at merge `643d1e1`.
Implementation commits `bbc1f33` and `962944b`, first-review fixes through
`bf972a7`, and second-review hardening through `9797ada` shipped in PR #126,
<https://github.com/levineuwirth/pmacs/pull/126>. That landed stage is
deliberately headless; this Stage 2 branch adds the Lua and TUI surfaces while
leaving GPU/protocol integration for Stage 3.

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
resize/pointer sending and coalescing. New daemon event variants apply the
same authenticated-source rule. Wire-facing terminal state, selection, and
limits live in or are re-exported from `pmacs-protocol`. Revision 8 resolves
the complete-frame size finding with a shared 8 MiB aggregate glyph-byte
bound: the maximum legal encoded frame is measured below the unchanged
16 MiB transport cap. The producer rejects rather than truncates or silently
chunks an over-bound internal snapshot.

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

### 0.7 Stage 2 landing and Stage 3 re-scout

Stage 2 landed on `main` as PR #130 at merge `86fc1bc` after two review
rounds. Its integrated tree is the Stage 3 base: protocol v18, four terminal
view-state acceptance cases in both default and CRDT builds, 109 required-GPU
tests, and the authenticated daemon/TUI seams described in §§5 and 9. Stage 3
does not reopen the landed escape, controller, selection, copy, resize, or Lua
contracts.

The current tree exposes five Stage 3 integration facts that are now locked:

- A semantic frontend learns a buffer switch from `BufferSnapshot`, but an
  empty terminal identity snapshot does not identify itself as terminal.
  Therefore the GPU sends both its ordinary byte viewport and a
  version-gated terminal-cell size after each snapshot; the daemon ignores the
  inapplicable declaration. No buffer-kind flag or pixel geometry is added.
- The existing TUI layout adapter subtracts modelines and handles splits from
  a whole terminal size. Reusing it for the GPU would subtract chrome twice.
  Stage 3 adds exact active-window semantic adapters whose `CellSize` already
  describes the terminal content rectangle.
- `screen_generation` does not cover selection, scroll, or process-state
  changes. Silence is therefore based on equality of the complete ordered
  wire payload, not on any one generation counter.
- The legal Stage 1 cell bounds can exceed the 16 MiB transport cap when every
  cell carries a maximal combining cluster. Rather than raising the allocation
  ceiling for all pre-handshake and established messages, v19 adds an 8 MiB
  aggregate glyph-byte limit and proves the largest legal encoded frame stays
  below `MAX_FRAME_BYTES`.
- The GPU's document shaper cannot define terminal column origins. Terminal
  mode uses fixed cell geometry: explicit row/column rectangles own
  background, underline, selection, cursor, clipping, and hit testing; text
  runs are positioned at cell origins and never determine later cell
  positions.

### 0.8 Stage 3 framing review round 1

The first external Stage 3 review verified the Revision 8 base, named seams,
limits, Stage 1/2 compatibility, and continuity state and found no
architectural defect. This round closes its three precision findings:

- The measured maximum payload fixture now maximizes legal `Style` bytes on
  every cell and distributes the exact glyph budget across legal clusters with
  lengths chosen to maximize serialized length-prefix overhead.
- GPU acceptance names terminal copy through `InstanceSignal::Clipboard` and
  the existing OS clipboard path; it does not imply that child OSC 52 is
  honored.
- Arc 5 stage 2 is the vterm delivery; capitalized Stages 1–3 are its three
  internal PR stages.

### 0.9 Stage 3 as built

Stage 3 is implemented on `vterm-gpu`, cut from canonical `main` @ `1dd47fc`
rather than stacked on the documentation branch, with the approved Revision 8
framing as its first commit. Criteria 28–37 are implemented.

**Protocol.** `pmacs-protocol` gained `src/terminal.rs`, which now owns the
shared row/column/visible-cell/grapheme/metadata bounds (re-exported from
`crate::terminal::*` so every Stage 1/2 caller keeps its path and no duplicate
type exists), `TerminalProcessState`, `TerminalSelectionSpan`, `TerminalFrame`,
and `TerminalFrame::validate`. `unicode-width` was promoted to a workspace
dependency so the terminal screen and the wire validator measure glyph columns
with one table. Discriminants: `InstanceMessage::TerminalFrame` is 26,
`FrontendEvent::TerminalResize` 11, `TerminalPointer` 12 — each appended after
its enum's final v18 variant, with placement pins on `StatuslineSegments` and
`MenuPointer` guarding them. `SUPPORTED_PROTOCOL_VERSIONS` is `[6..=19]`.

The measured maximum legal frame — 512x512, one maximal selection span per row,
maximum title and process metadata, the maximal-encoding `Style` on every cell,
and the exact 8 MiB aggregate glyph budget distributed to maximize serialized
length-prefix overhead — encodes to **13,437,863 bytes**, against the unchanged
16,777,216-byte transport cap. A one-byte-over aggregate is rejected before
serialization.

**Two decisions the implementation had to make.**

1. *The `Viewport` gate keys on the ACTIVE buffer, not the declared one.*
   §6.2 says "a document buffer accepts only `Viewport`; an active terminal
   buffer accepts only `TerminalResize`", and the first implementation read
   that as a test on the buffer the message names. That is not sufficient:
   `Viewport` also ALIGNS the frontend's window to the buffer it names, so a
   stale document viewport still in flight when a command opened a terminal
   dragged the frontend straight back off it — the real-daemon acceptance
   showed the window oscillating and no frame ever arriving. The daemon now
   drops `Viewport` when the authenticated source's active window shows a
   terminal (and, defensively, when the declared buffer is itself a terminal).
   This is the framing's own wording taken literally; it is recorded here
   because the weaker reading looks correct and silently produces a terminal
   that never paints.
2. *The producer clears terminal mode on every exit path.* `in_terminal_mode`
   is used daemon-side to suppress `CursorByte` and the presence sweep. An
   early return from the terminal pass that left the flag set kept those
   suppressed after the frontend went back to a document. Every path out of
   the pass now clears it explicitly.

**Renderer.** `pmacs-gpu/src/terminal.rs` is a pure, cell-space paint planner:
it resolves a validated frame into background/underline/selection/cursor runs
and explicitly positioned text runs, taking the frontend's two default colors
as parameters so every paint rule is unit-testable without a GPU. `main.rs`
holds a two-state machine (`Document` / `Terminal`), builds one shaped buffer
per text run so a wide or cluster glyph's advance can never choose the next
column's origin, and swaps the document quad/squiggle/caret/minimap/gutter
batches for terminal ones while leaving the status band and popup layers alone.

**Criterion 37.** The GPU is a separate binary that depends only on
`pmacs-protocol`, so the single real path is driven as a process:
`pmacs-gpu --headless-probe <socket> <report>` attaches through the REAL
`attach` client (the reader sink was generalized so the winit path and the probe
share one handshake, outbox, and writer), presses a real key that opens a real
`/bin/sh` child, applies real `TerminalFrame`s, composites real pixels through
`render_to_view`, sends real input and a real geometry change, and writes named
observations the acceptance asserts on. `tests/vterm_stage3_acceptance.rs`
`a37_…` is that test; it is CRDT-gated because the daemon advertises
`crdt_replica` / `semantic_render` only on CRDT builds.

**Verification (from a clean tree).** `cargo fmt --check`; strict workspace
Clippy; 1,757 default + 1,933 CRDT library tests (3 ignored each); Stage 1
acceptance 9 default + 10 CRDT; Stage 2 acceptance 4 default + 4 CRDT; Stage 3
acceptance 4 default + 5 CRDT (the fifth is the CRDT-gated real-daemon path);
statusline acceptance 7 default + 8 CRDT; M4 120 passed (3 ignored, 1 filtered);
required GPU 127; one-invocation workspace sweep 2,919 passed across 83 suites
(19 ignored); `git diff --check` clean. One unexplained single failure of the
required-GPU suite occurred once mid-session and did not reproduce across eight
subsequent runs including the full sweep; its identity was not captured.

### 0.10 Stage 3 review round 1

PR #135's first review found no correctness blocker, confirmed the required-GPU
suite clean across four runs (twelve total with the author's), and raised two
design questions plus three minor notes. All five are addressed.

- **Presence while in terminal mode (finding 1) — fix kept, prediction not
  reproduced.** The review predicted that skipping the presence sweep freezes
  `last_broadcast` at the abandoned document position, leaving peers painting a
  stale caret. It does not: the buffer-follow clears the terminal declaration
  when it ships the snapshot, so `terminal_active` is false on the tick a window
  first shows a terminal, and the declaration cannot arrive until a later tick
  (the frontend learns the buffer id FROM that snapshot). One truthful sweep
  always lands first. A real-daemon two-frontend test written to catch the
  freeze passes against the pre-fix tree — the bite is VACUOUS, and it is
  labelled a regression guard rather than fix evidence. The skip is removed
  anyway: it was load-bearing on tick ordering and bought nothing, and its
  removal makes "presence follows the frontend" structural.
- **Hover claimed durable control (finding 2) — real, fixed, bite-verified.**
  `apply_terminal_gesture` claimed the controller before dispatching, including
  for `Move`, which does nothing. A semantic frontend reports motion at pixel
  rate, so sweeping the mouse across a PASSIVE split's terminal took durable
  control and the next layout sync resized the shared PTY to that background
  view's geometry — exactly the theft the controller rule exists to prevent.
  Bare motion no longer claims; every deliberate gesture still does.
  `scripts/bite HEAD src/editor.rs` on
  `hover_does_not_steal_terminal_control_from_the_active_frontend` is a clean
  behavioral bite (assertion failure, not a compile error).
- **Terminal motion is deduplicated by cell (finding 3).** Sub-cell motion
  resolved to the same coordinate and still crossed the wire, where each event
  is a daemon-side gesture. `State::terminal_motion_is_new` now gates it, and
  press/release re-arm the memo so the first drag after a press still reports.
  Its unit test cannot bite — the seam did not exist pre-fix — and says so.
- **Declarations record only once sent (finding 4).**
  `terminal_declaration_if_changed` is now a pure query and
  `note_terminal_declaration_sent` records, so a failed write is retried instead
  of suppressed as already-declared. The existing `a35` test caught the contract
  change and now pins both halves.
- **Unchanged frames are no longer re-validated (finding 5).** The complete
  payload comparison runs before `validate`; only validated frames are ever
  stored, so a frame equal to the baseline has already passed. The chrome tail
  is factored into `terminal_chrome` so both exits emit it identically.

Post-review gates: `cargo fmt --check`; strict workspace Clippy; 1,757 default +
1,933 CRDT library tests; Stage 1 acceptance 9/10, Stage 2 4/4, Stage 3 5/7,
statusline 7/8 (default/CRDT); M4 120; required GPU 128; workspace sweep 2,921
across 83 suites (19 ignored); `git diff --check` clean.

### 0.11 Stage 3 review round 2

The second review verified all five round-1 fixes in code, re-ran the
required-GPU suite clean (a thirteenth consecutive pass, closing the flake
caveat), and found one new low-severity defect plus minor items.

- **A disconnect in terminal mode hid the notice (finding 1) — real, fixed,
  hand-verified.** `AttachEvent::Disconnected` set the placeholder text but
  never left terminal mode, where the document code layer is not prepared at
  all and the terminal glyph layer keeps painting its last frame. The user was
  left looking at a frozen, live-looking terminal that silently ignored input —
  and with GPU auto-reconnect a named deferral, until relaunch.
  `State::on_daemon_disconnected` now leaves terminal mode, forces a repaint
  even when the notice text is byte-identical, and requests a redraw. Its test
  lives in the same file as the fix, so `scripts/bite`'s file granularity
  cannot bite it; the equivalent was done by hand — neutralizing only the
  `exit_terminal_mode()` call makes the test fail, restoring it makes it pass.
- **Per-tick full-grid clone removed (finding 2).**
  `sync_semantic_terminal_layout` compared geometry via `snapshot(..).size`,
  cloning the whole visible cell grid every dispatcher tick to answer one
  comparison. `TerminalManager::screen_size` reads it from the borrowed
  projection instead.
- **Roadmap and handoff Arc 5 lines corrected (finding 3).** Both still said
  Stage 3 was framed and awaiting approval, contradicting this PR's own ledger.
- **A press that misses the grid no longer arms a drag (nit).** It set
  `pointer_drag_active` unconditionally, so a later in-grid motion sent a
  `Drag` with no preceding `Down`. Daemon-side impact was nil
  (`update_selection` bails without a drag anchor), but the state is now
  honest. A release still always ends the drag, including one that wandered
  outside the grid.
- **Inbound terminal events now require a negotiated v19 session (finding 5).**
  The outbound `TerminalFrame` was gated twice while the inbound declarations
  relied on the frontend's send gate alone. A pre-v19 peer cannot construct
  these variants, so this only refuses a hand-rolled client — and the a32
  forgery tests already prove such an event reaches nothing but the sender's
  own authenticated active view — but the asymmetry was not deliberate, and
  "gated in both directions" should be true of the code rather than only of the
  frontends we ship.

Deferred from this round, named: **terminal wheel gestures discard scroll
magnitude.** One winit wheel event becomes one terminal gesture regardless of
the lines it accumulated, so a two-tick event scrolls the same distance as a
one-tick event, while the document path scrolls by `lines`. Closing it means
either sending N gestures (chattier) or widening the terminal pointer event
with a magnitude — a protocol change. Not worth either inside this stage.

Post-round-2 gates: `cargo fmt --check`; strict workspace Clippy; 1,758 default
+ 1,934 CRDT library tests; Stage 1 acceptance 9/10, Stage 2 4/4, Stage 3 5/7,
statusline 7/8 (default/CRDT); M4 120; required GPU 129; workspace sweep 2,923
across 83 suites (19 ignored); `git diff --check` clean.

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

One intentional Stage 2 boundary remains until Stage 3: an authenticated v18
semantic frontend can send a key while its active buffer is a terminal, claim
that terminal's controller, and feed the PTY, but cannot display the screen.
The next accepted TUI terminal input reclaims control; v19 removes the invisible
interval by adding the semantic terminal surface.

## 6. Stage 3 — protocol v19 and GPU integration

### 6.1 Protocol-owned wire contract

Protocol v19 appends, never inserts, one instance message and two frontend
events after the final v18 variants:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalFrame {
    pub buffer_id: BufferId,
    pub size: CellSize,
    pub cells: Vec<Cell>,
    pub cursor: Option<CellCoord>,
    pub title: Option<String>,
    pub screen_generation: u64,
    pub selection: Vec<TerminalSelectionSpan>,
    pub scroll_offset: u32,
    pub at_bottom: bool,
    pub pid: u32,
    pub process: TerminalProcessState,
}

InstanceMessage::TerminalFrame(TerminalFrame)

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

`TerminalProcessState`, `TerminalSelectionSpan`, and every limit needed to
validate these values move to or are re-exported from `pmacs-protocol`; core
paths may re-export them so Stage 1/2 callers do not gain duplicate types.
The shared limits are the landed row, column, visible-cell, grapheme, and
metadata bounds plus:

```rust
pub const MAX_TERMINAL_FRAME_GLYPH_BYTES: usize = 8 * 1024 * 1024;
```

The aggregate counts each `Char`'s UTF-8 length, every `Cluster` byte, and zero
for `Continuation`, with checked addition. `TerminalFrame::validate` is the
single structural policy used before daemon emission and after GPU decode.
`pmacs-protocol` takes the direct `unicode-width` dependency needed to validate
one/two-column glyph and continuation topology; another frontend must not
copy that policy.

The existing `MAX_FRAME_BYTES = 16 MiB` remains unchanged. A protocol test
constructs the maximum row/column frame with one maximal legal selection span
per row and maximum title/process metadata. Every cell carries a legal
one-column `Cluster` plus the maximal-encoding legal `Style`: RGB foreground,
background, and underline color, a non-`None` underline, and all boolean flags
set. The fixture distributes the exact aggregate budget across a legal cluster
in every cell and chooses cluster lengths to maximize total serialized
length-prefix overhead rather than packing bytes into a few maximal clusters.
This maximizes cluster and per-cell style overhead together. It serializes
the enclosing `InstanceMessage` with `postcard::to_allocvec` and asserts the
measured length is below `MAX_FRAME_BYTES`. A one-byte-over aggregate is
rejected before serialization.
This is a wire-specific bound, not a mutation of the TUI screen: if a child
constructs a larger internal snapshot, the semantic producer retains its last
valid baseline, emits nothing malformed or truncated, and logs the first
distinct bounded error until a valid frame clears the latch.

`TerminalFrame` is a complete visible-grid replacement. Empty is not a clear
sentinel: valid terminal dimensions are nonzero and `cells.len()` equals the
checked area exactly. `screen_generation` describes the published terminal
screen/title generation; selection, scroll, viewport, and process state may
change without it. The producer compares every ordered field against one
retained baseline per semantic frontend context. A view-only change therefore
sends; a byte-identical/equal payload is silent. Buffer snapshot, detach, and
context replacement clear that baseline so the next valid terminal frame is
authoritative.

Validation is atomic and covers:

- shared nonzero row/column/checked-area and aggregate-glyph limits;
- exact cell area; each caller separately requires the expected current
  `buffer_id`;
- cursor bounds;
- title and process-state metadata length/control-character rules;
- valid nonempty cluster UTF-8 within the per-cluster limit;
- printable one/two-column leading glyphs, required wide continuations, and no
  orphan continuation; continuation style is ignored in favor of its lead;
- no `Attachment` in any terminal cell;
- strictly increasing, one-per-row, nonempty selection spans inside the frame;
- `at_bottom == (scroll_offset == 0)`.

Invalid daemon output is never written. Invalid GPU input retains the previous
valid frame, requests no redraw, and reports one latched diagnostic rather than
partially applying cells or metadata.

Compatibility remains additive:

- v18 grid peers keep receiving the Stage 2 composed `CellDelta` terminal
  windows;
- v18 semantic peers receive the immutable empty identity snapshot but no
  terminal message. Terminal use remains unsupported and invisible for those
  peers; ordinary document editing remains supported;
- v19 frontends send `TerminalResize`/`TerminalPointer` only to a negotiated
  v19 daemon;
- the v19 daemon filters `TerminalFrame` from every wire below v19;
- postcard pins preserve every v18 discriminant and pin all three appended
  v19 discriminants.

### 6.2 First-frame bootstrap and authenticated routing

`BufferSnapshot` remains the one semantic display-switch message. It carries
the terminal identity buffer's valid empty CRDT state and adds no terminal
flag. Immediately after applying any snapshot, a v19 GPU sends the existing
`Viewport` for the new replica/generation and, when its derived terminal
content rectangle has at least one row and column, `TerminalResize` with the
new `buffer_id` and size in cells. A zero-area window sends no terminal
declaration until a later geometry change yields a valid size.

The daemon authenticates the connection source before reading any claimed
`frontend_id`. A document buffer accepts only `Viewport`; an active terminal
buffer accepts only `TerminalResize`. The inapplicable declaration is dropped
without mutation or logging noise. This dual declaration occurs only after a
snapshot or an actual geometry change, not every frame, and removes the
otherwise circular dependency where the GPU would need a terminal frame before
it knew to request one.

For an authenticated active terminal window, a valid `TerminalResize` always
records that exact `(frontend, window, buffer)` view size so passive frontends
can receive their own clipped/padded projection. It resizes the PTY only when
that exact view is the durable controller from §5.4; resize never claims
control. The semantic adapter consumes a content `CellSize` directly and does
not run the TUI placement helper or subtract a modeline again. Unchanged,
zero/out-of-range, passive, and failed resizes retain existing geometry under
the Stage 2 ordering contract.

`TerminalPointer` must match the authenticated source, active terminal buffer,
and last accepted terminal viewport, and its coordinate must be in bounds.
Once accepted, it follows the same Stage 2 terminal pointer path: child SGR
mouse reporting when eligible, otherwise per-view scroll/selection/context
menu. Like other accepted pointer/input/focus events, it may claim control.
A forged source, stale buffer, missing declaration, or out-of-bounds coordinate
is dropped before view, controller, selection, menu, or PTY mutation.

Key, paste, focus, detach, BEL, and clipboard keep their landed event/message
types. The daemon continues to overwrite client-claimed IDs with the
authenticated source before terminal dispatch. Terminal mode adds no raw-byte
input message and never sends child bytes through Lua or a shell.

### 6.3 Semantic producer and editor adapters

`SemanticRenderState` stores the optional terminal viewport and last valid
terminal-frame baseline for its one authenticated frontend. When the active
buffer is terminal and a matching viewport exists, it requests an owned
snapshot for the exact active `TerminalViewKey`, validates/converts it, and
emits `TerminalFrame` only on complete-payload change.

The editor-side contract is narrow:

```rust
prepare_semantic_terminal_view(
    frontend_id: FrontendId,
    buffer_id: BufferId,
    size: CellSize,
) -> Option<TerminalSnapshot>

sync_semantic_terminal_layout(
    frontend_id: FrontendId,
    buffer_id: BufferId,
    size: CellSize,
) -> bool

dispatch_semantic_terminal_pointer(
    frontend_id: FrontendId,
    buffer_id: BufferId,
    size: CellSize,
    coord: CellCoord,
    kind: MouseKind,
    mods: Modifiers,
) -> bool
```

Each method derives the active `WindowId` from `frontend_id`, verifies that it
still displays `buffer_id`, and delegates to the existing manager/view/input
state machines. No method accepts a client-supplied window identity. Semantic
layout sync runs after event coalescing, before `tick_processes`, exactly beside
the landed grid sync; snapshot production occurs on the following render pass
from the already-published screen.

The terminal producer retains the buffer-independent/UI messages required by
the native frontend: `StatusFacts`, `ThemeFacts`, `FontFacts`,
`StatuslineSegments`, `MenuPrompt`, and `MinibufferPrompt`, plus daemon-owned
`DispatchIdle`, `Signal`, and lifecycle messages. The built-in terminal
statusline provider therefore remains the source of title/process/scroll text.
It suppresses document-only style spans, decorations, inlays, block
adornments, folds, file summaries, search/completion surfaces, line numbers,
document `CursorByte`, and presence for the terminal identity buffer.

On terminal activation the GPU clears/ignores every document-local visual
cache before painting the first frame. On switching back, snapshot reset clears
the producer's document baselines so the first matching document viewport
receives the existing full authoritative style/decoration/summary resync.
Terminal baselines are per frontend context; one split/frontend can never
suppress or overwrite another's scroll/selection projection.

### 6.4 GPU state and fixed-cell renderer

The GPU state machine is explicit: `Document` or
`Terminal { buffer_id, frame, derived paint caches }`. `BufferSnapshot`
immediately leaves terminal mode, clears the prior terminal frame and
terminal-only caches, and restores document defaults. A valid matching
`TerminalFrame` enters terminal mode. A stale-buffer frame is ignored; an
identical valid frame is retained without rebuild or redraw.

Terminal geometry is the drawable code rectangle above the existing status
band, with no document gutter or minimap. Rows and columns are
`floor(pixel_extent / active_cell_metric)`, clamped through the shared
nonzero protocol limits. Pixels, scale, DPI, and glyph advances never cross
the wire. Rows never wrap; excess frame rows/columns clip to the declared cell
rectangle and undersized content is padded with terminal defaults.

Painting is cell-derived rather than rope-derived:

- Every cell rectangle has a fixed origin from `(row, col)` and the active
  monospace metrics. Backgrounds coalesce only adjacent equal resolved colors.
- `Default` foreground/background map to the GPU's existing plain-text/window
  defaults; indexed colors use the existing xterm palette; truecolor is exact.
  `reverse` swaps the two resolved colors. A continuation draws no glyph and
  inherits its lead cell's paint semantics.
- Text shaping is split into explicitly positioned row runs. Contiguous
  single-width ASCII may share one monospace buffer with rich attribute spans;
  non-ASCII/cluster/wide leads start at an explicit cell origin and are clipped
  to their declared one/two-cell footprint. A shaped advance never chooses the
  next run's column.
- Bold and italic use font attributes. Single/double/dotted/dashed underlines
  use fixed-cell quads; curly uses the existing squiggle pipeline. Default
  underline color follows the post-reverse foreground.
- Terminal selection is a separate fixed-cell wash resolved through the
  existing GPU `ui.selection` site; it never rewrites child cell styles.
  Cursor visibility/position comes only from the frame and paints through the
  existing caret primitive inside the terminal clip.
- The status band and its provider runs remain outside/above terminal paint.
  Document decoration, presence, caret, gutter, and minimap batches are not
  prepared or drawn in terminal mode.

### 6.5 GPU input, signals, and cache invalidation

Keyboard, paste, focus, and detach reuse existing attach messages and daemon
encoding. Inside the terminal clip, GPU mouse hit testing is
pixel-to-`CellCoord`; press/release/drag/move/wheel sends `TerminalPointer`
instead of a source-byte `Pointer`. The status band/outside clip is never a
terminal hit. Move/drag events coalesce only while kind, coordinate, and
modifiers are unchanged.

Window resize, scale change, accepted `FontFacts`, and buffer snapshot
recompute the cell viewport. One changed size sends one version-gated
`TerminalResize`; an equal size is silent. A font change clears terminal shape
and geometry caches until a matching-size authoritative frame arrives.

Invalidation is deliberately narrow:

- a changed terminal frame rebuilds cell text/background/underline/selection/
  cursor data, but not statusline buffers;
- `ThemeFacts` rebuilds the selection/status color sites, not child cell
  shaping;
- `FontFacts` rebuilds shape and geometry and emits a changed cell viewport;
- status/statusline/menu/minibuffer messages touch only their existing caches;
- a duplicate valid terminal frame and an unchanged geometry declaration do
  no work.

`InstanceSignal::Bell` requests one frontend attention event for each new
active-terminal BEL; historical/passive suppression remains daemon-owned.
`InstanceSignal::Clipboard` keeps the existing OS clipboard path. OSC title is
sanitized frame/statusline metadata only and never becomes a raw window-title
or terminal-control operation.

## 7. Four-agent execution plan

The vterm roster is fixed at four agents for all three stages. Do not add
review/scout agents; reuse these owners so state-machine decisions stay
coherent.

| Owner | Stable scope | Primary files |
| --- | --- | --- |
| Lead/integrator | contracts first; Stage 1/2 manager/Lua lifecycle; Stage 3 semantic producer, authenticated daemon routing, shared acceptance, gates, docs, branches/PRs | `src/terminal/session.rs`, `src/lua_bindings/mod.rs`, `builtin/runtime/terminal.lua`, `src/semantic_render.rs`, owned Stage 3 sections of `src/daemon.rs`, `tests/vterm_*_acceptance.rs`, docs |
| VT core agent | encoder/screen seams; Stage 3 snapshot-to-wire conversion and shared terminal re-exports, no renderer ownership | `src/ansi.rs`, `src/terminal/{screen,input,session,view}.rs` in assigned non-overlapping sections |
| TUI agent | landed TUI projection/input; Stage 3 exact active-window semantic adapters and parity tests, no GPU/protocol files | owned sections of `src/editor.rs`, focused TUI/adapter tests |
| Protocol/GPU agent | protocol v19 types/limits/pins/gates and native GPU state/render/hit-test/attach sending | `pmacs-protocol`, `src/protocol.rs`, `pmacs-gpu/src/{main,attach}.rs` |

Coordination rules:

- Lead establishes invariants and method signatures before another lane edits
  callers; the protocol/GPU agent then implements the shared wire types.
- Strict Stage 3 file ownership follows the table. VT-core and TUI edits in
  `src/terminal/{session,view}.rs` / `src/editor.rs` are agreed as exact
  non-overlapping symbols before work starts. Only the lead edits
  `src/semantic_render.rs`, Stage 3 daemon routing, or shared acceptance.
  Stage 3 begins only from landed Stage 2.
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
- Stage 3: lead locks the protocol constants/types and editor method signatures,
  then owns `src/semantic_render.rs`, authenticated `src/daemon.rs` routing,
  shared acceptance, and integration. The protocol/GPU agent owns
  `pmacs-protocol` plus `pmacs-gpu/src/{main,attach}.rs`; the VT-core agent owns
  snapshot-to-wire conversion/shared terminal re-exports; the TUI agent owns
  the exact semantic adapters in `src/editor.rs` and parity tests. All four run
  in parallel only after the lead's contract checkpoint; no TUI rendering
  behavior changes.

## 8. Branch and PR plan

Stage 1 landed as PR #126 at merge `643d1e1`. Stage 2 landed as PR #130 at
merge `86fc1bc`. Revision 8 is preserved for approval on
`vterm-stage3-framing`, based on the integrated current `main`.

After explicit framing approval:

1. create worktree `pmacs-vterm-gpu` and branch `vterm-gpu` from the then-current
   canonical `githubsucks/main`;
2. implement only Stage 3 / criteria 28–37;
3. run the complete sequential gate and bite suite;
4. open the third and final vterm PR for user review; never merge it without
   explicit authorization.

The historical framing branch `vterm-framing` remains the Revision 7 record.
The Stage 3 implementation is not stacked on a documentation branch or an
unmerged feature parent.

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

#### Stage 2 verification map

The cross-surface suite is `tests/vterm_stage2_acceptance.rs`; focused unit
coverage remains beside the owning implementation. The criteria map as follows:

- **15:** `lua_surface_is_strict_fresh_transactional_and_context_safe`.
- **16:** `editor::tests::terminal_snapshot_composes_only_content_and_translates_cursor`
  plus the real-TUI smoke.
- **17:** `terminal::view::tests::{tail_projection_pads_above_and_right_and_translates_cursor,
  frozen_top_is_geometrically_at_bottom_when_view_still_reaches_tail,
  alternate_switch_clears_view_anchors_and_selection}` and
  `shared_screen_keeps_view_scroll_selection_and_controller_independent`.
- **18:** `terminal::input::tests::{utf8_ctrl_and_alt_boundaries,
  application_cursor_and_xterm_modifiers, ambiguous_digits_ignore_application_keypad,
  paste_and_focus_are_exact, unsupported_keys_are_invisible}` and the real-TUI
  input/paste path.
- **19:** `editor::tests::dispatch_prefix_state_is_independent_per_frontend`,
  `terminal_escape_gates_local_bindings_and_double_escape_sends_interrupt`,
  `lua_surface_is_strict_fresh_transactional_and_context_safe`, and the
  real-TUI escaped editor-binding/quit path.
- **20:** `terminal::input::tests::sgr_mouse_modes_modifiers_and_coordinates`
  plus the real-TUI editor-owned scroll/drag/copy path.
- **21:** `terminal::view::tests::{copy_joins_soft_wraps_trims_default_blanks_and_separates_hard_rows,
  wide_continuation_canonicalizes_to_lead_and_copies_once}`,
  `lua_surface_is_strict_fresh_transactional_and_context_safe`, and the real
  OSC 52 clipboard assertion.
- **22:** `lua_surface_is_strict_fresh_transactional_and_context_safe`,
  `shared_screen_keeps_view_scroll_selection_and_controller_independent`, and
  the real child-PTY resize assertion.
- **23:** `daemon::tests::forged_resize_mutates_only_the_authenticated_frontend`,
  `daemon::tests::inbound_paste_uses_authenticated_source_not_the_claimed_id`,
  the existing `m5_4_dispatch_{key,mouse}_threads_frontend_id_to_lua_surface`
  tests, and the multi-frontend detach assertions in
  `shared_screen_keeps_view_scroll_selection_and_controller_independent`.
- **24:** the built-in-provider and clipboard assertions in
  `lua_surface_is_strict_fresh_transactional_and_context_safe`,
  `daemon::tests::terminal_bell_baseline_suppresses_history_and_delivers_each_new_bell_once`,
  and the real-TUI BEL/OSC 52 assertions.
- **25:** `shared_screen_keeps_view_scroll_selection_and_controller_independent`.
- **26:** both in-process acceptance tests' termination/cleanup assertions and
  the real-TUI clean-exit/host-restoration assertions.
- **27:** `real_tui_terminal_smoke_restores_host_after_output_input_resize_scroll_copy_and_bell`.

### Stage 3 — GPU/protocol

28. Protocol v19 appends all three new variants after the v18 pins; v18 grid
    traffic round-trips byte-identically, v18 semantic document traffic still
    works, and both outbound directions are independently version-gated.
29. The measured maximum legal frame uses maximum dimensions, one maximum
    selection span per row, maximum metadata, the exact aggregate glyph budget
    present as one legal cluster per cell with lengths chosen to maximize
    serialized length-prefix overhead, and the maximal-encoding legal style on
    every cell. Its enclosing message encodes below the unchanged
    16 MiB transport cap. Validation accepts exact shared boundaries and
    atomically rejects one-byte-over aggregate, over-area, bad area,
    out-of-bounds cursor, control/malformed/overlong clusters, orphan/missing
    continuations, attachments, duplicate/out-of-order-row or otherwise
    invalid selection spans, inconsistent bottom state, overlong title, and
    overlong process text while retaining the prior valid frame.
30. First terminal activation emits one authoritative complete frame after the
    dual viewport declaration, then stays silent for a completely equal
    payload. View-only and process-only changes emit despite an unchanged
    screen generation. Terminal activation suppresses document projection,
    cursor, presence, gutter, and completion; switching back forces one
    complete document resync.
31. Two semantic frontends over one terminal receive independent sizes,
    scroll, selection, cursor visibility, and baselines while sharing one
    process/screen. A passive declaration produces its clipped/padded frame but
    does not resize; only the exact durable controller changes PTY geometry.
32. Forged frontend/buffer IDs, stale buffers, undeclared viewports, and
    out-of-bounds terminal pointer/resize events cannot affect another view,
    controller, terminal selection, menu, PTY size, or child input.
33. Headless GPU rendering pins default/indexed/truecolor foreground and
    background, reverse, bold/italic, all supported underline forms, wide and
    combining footprints, continuation inheritance, selection, cursor,
    padding/clipping, status-band separation, and absence of frontend wrapping,
    document gutter/minimap, and document overlays.
34. GPU key, paste, focus, press/release/drag/move/wheel input reaches the same
    Stage 2 terminal encoders and ownership paths. Pixel hit testing yields only
    in-bounds cells, never source bytes or wire pixels, and unchanged move/drag
    cells coalesce.
35. Buffer/window/scale/font transitions emit exactly one changed cell
    declaration and suppress identical sizes. Terminal, theme, font, and
    statusline changes invalidate only their named caches; duplicate valid
    frames request no redraw.
36. Each new active BEL produces one GPU attention action. Terminal copy
    publishes through `InstanceSignal::Clipboard` and the existing OS
    clipboard path exactly once. Sanitized OSC title/process/scroll metadata
    reaches the terminal provider/statusline without becoming a raw host-title
    or control effect.
37. A hermetic real daemon + required headless GPU smoke opens `/bin/sh`, runs
    a full-screen alternate-screen probe, exercises key/paste/mouse/resize,
    BEL/title metadata, scroll/select/copy through the clipboard signal, clean
    exit, and buffer switch-back, then proves the preserved main screen and all
    child/reader/session cleanup.

#### Stage 3 verification map

The cross-surface suite is `tests/vterm_stage3_acceptance.rs`; focused wire and
GPU assertions remain in `pmacs-protocol` and `pmacs-gpu` respectively.

- **28–29:** postcard discriminant/round-trip pins, common
  `TerminalFrame::validate` boundary table, measured maximum legal payload, and
  v18/v19 daemon/frontend send filters.
- **30–32:** semantic producer baseline/reset tests, dual-declaration
  bootstrap, real dispatcher source-forgery tests, and two-frontend
  controller/passive-view acceptance.
- **33:** pure fixed-cell paint-plan tests plus required headless offscreen
  pixel probes for representative color/style/wide/selection/cursor/clipping
  cases.
- **34–36:** attach/event coalescing tests, authenticated daemon input tests,
  and headless signal/statusline observations.
- **37:** one real-daemon/real-PTY/headless-wgpu acceptance path; it is not
  replaced by a decoded-message fixture.

### 0.12 As-framed audit, 2026-07-25 (after #166)

Prompted by a GPU terminal input defect that shipped in Stage 3 and was fixed
in #166. The arc is structurally complete — all 37 criteria have
implementations, and every test named in the Stage 2 verification map exists —
but the audit found two gaps worth recording against the criteria themselves.

**Criterion 22's "without thrash" was never pinned.** The criterion reads
"unchanged, zero, passive, and failed resize cases preserve prior geometry
*without thrash*". The word appears nowhere in `src/` or `tests/`. The suite
pinned the four enumerated single-arm cases and never the cross-arm
interaction — which is exactly where the thrash lived: the daemon applied
both the grid and the semantic terminal-layout sync to every attached
frontend, so a semantic session's PTY was resized twice per tick forever.
Criterion 31's "only the exact durable controller changes PTY geometry" was
violated in the same event, in spirit rather than letter: the controller was
the right frontend, but the geometry came from the grid projection. #166 adds
the settle pins; the gap was open from #135 (2026-07-22) until then.

**Why the Stage 3 suite could not see it.** Of its nine tests, only three
drive a real daemon; the other six construct `EditorState` directly and never
execute the dispatcher loop where the defect lived. `a31`, which is about two
semantic frontends sharing one session, therefore passes on the broken tree.
The same structural blindness explains why `bottom_panel_stage1_acceptance`
was unaffected. A criterion about *dispatcher* behavior needs a test that
runs the dispatcher.

**Four of the nine Stage 3 tests do not run in CI at all**, because they are
`#[cfg(feature = "crdt")]` and the workflow never enables that feature:
`a37`, the two added by #166, and
`terminal_mode_keeps_reporting_presence_so_peers_drop_the_stale_caret` — which
is Stage 3 review round 1's own regression guard. Stage 1's
`read_only_empty_crdt_bootstrap_is_immutable_against_remote_content`, the CRDT
half of criterion 14, is dark for the same reason. Stage 2 is fully covered
(6/6). This is not a vterm problem: 264 tests workspace-wide are dark,
including 177 in the library. It has its own lane in `docs/active-work.md`.

**And `a37` is darker still than that count implies: it reports `ok` without
running whenever `pmacs-gpu` is absent from the same target directory**
(measured 2026-07-26 while gating #173). It derives the sibling binary from
`CARGO_BIN_EXE_pmacs` and, finding nothing, prints a skip notice and returns.
A fresh worktree reports the suite 9/9 in 0.17 s having executed the arc's
only real-daemon/real-PTY/real-wgpu path zero times; a genuine run takes
about four seconds. `PMACS_REQUIRE_GPU=1` is the only thing that turns that
skip into a failure, and the standing gate list applies that flag to
`cargo test -p pmacs-gpu`, a different package. So the audit's claim that
"only 3 of 9 Stage 3 tests drive a real daemon" was itself optimistic —
**on a target directory without the frontend binary the honest number is 2**,
and nothing in the gate log says so. It is also load-sensitive: it passed and
then failed at the same commit twenty minutes apart under machine
contention. Criterion 22's unpinned "without thrash" and this are the arc's
two standing verification gaps.

**Not audited:** §11's blanket claim that "deferral means graceful ignore or
documented absence, never escape leakage, panic, unbounded allocation, or
child leak". That covers roughly twenty deferred items and none were
spot-checked. It remains an unproven claim rather than a known gap.

## 10. Gates and bite verification

Every PR runs the standing full gates from `AGENTS.md`, sequentially, plus its
stage acceptance suite. Stage 2 included a real hermetic TUI PTY smoke. Stage 3
adds `cargo test --test vterm_stage3_acceptance`, its CRDT variant,
`PMACS_REQUIRE_GPU=1 cargo test -p pmacs-gpu`, the real-daemon/headless-GPU
probe, and the ordinary workspace sweep.

New behavioral acceptance must be bite-verified against the immediate
pre-stage tree with `scripts/bite` where the swapped files compile. Protocol
v19 tests additionally pin postcard bytes, prove the measured aggregate-bound
maximum stays below the unchanged transport cap, and verify both
older-version send filters; a test that merely fails to decode on old code is
not a useful bite.

## 11. Explicit deferrals

Not part of these three PRs:

- terminal image protocols (sixel, kitty graphics, iTerm images);
- OSC 8 hyperlink interaction;
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
- bracketed-paste payload filtering: Stage 2 forwards exact paste bytes as
  framed, so embedded `ESC[201~` can terminate the wrapper early; xterm-style
  filtering/escaping requires a separate input-policy decision;
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

The 2026-07-22 architecture, re-scout, and final framing review resolved every
current question:

1. Fixed terminal editor escape: `C-c`; `C-c C-c` sends literal Ctrl-C.
2. Resize: reflow main-screen soft wraps; clip/pad alternate screen.
3. Exit: retain the buffer and append the process PID/outcome line from §4.1.
4. Compatibility: additive v19; v18 grid remains supported, v18 semantic has
   no terminal surface.
5. GPU wire: complete visible frames with complete-payload suppression under
   one shared 8 MiB aggregate glyph-byte bound; the 16 MiB transport cap stays.
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
15. Bootstrap: after every semantic snapshot, v19 sends both byte viewport and
    terminal cell size; the authenticated daemon accepts only the declaration
    matching the active buffer kind.
16. Semantic resize: the declaration records passive view geometry, but only
    the exact durable controller changes the shared PTY/screen size.
17. GPU layout: fixed cell rectangles own geometry and hit testing; shaped
    glyph advances never determine subsequent columns, and OSC title remains
    metadata rather than a host control effect.
