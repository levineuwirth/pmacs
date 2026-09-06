# pmacs-gpu mouse input — framing pass

Date: 2026-06-10. Follows the Phase B framing
(`pmacs-gpu-phase-b-framing.md`), which deferred the mouse wire
decision as Q#B5. The optimistic-typing arc is merged (PR #60,
main @ `7fdae76`); this is the next capability session.

## What exists today (fact-checked)

- **Wire**: `FrontendEvent::Mouse(MouseEvent)` carries a `CellCoord`
  (`pmacs-protocol/src/message.rs:210-219`) — a *cell-grid*
  coordinate. `PROTOCOL_VERSION = 4` (`message.rs:970`); the
  DispatchIdle variant set the precedent for gating new variants on
  `negotiated_protocol_version` (`daemon.rs:998`).
- **Daemon semantics**: `EditorState::dispatch_mouse` owns the full
  gesture state machine — Down activates the window, positions the
  cursor, begins a selection; Drag grows the region; Up collapses an
  empty region; a second Down in the same cell within 500ms is a
  double-click and calls `select_word_at_cursor`; ScrollUp/Down move
  the viewport (`src/editor.rs`, CUA arc). For a *semantic* session,
  `apply_semantic_input_event` already routes
  `FrontendEvent::Mouse → mouse_to_crossterm → dispatch_mouse`
  (`daemon.rs:1983-1986`) — against the session's placeholder 24×80
  cell geometry, which pmacs-gpu does not have.
- **GPU layout**: the shaped buffer holds the *visible slice* of the
  source (`current_text[vstart..vend]`) with **inline adornment text
  injected** by `projected_rich_chunks` (`pmacs-gpu/src/main.rs:3202`)
  — so shaped-buffer byte offsets ≠ source bytes whenever inlay hints
  are visible. Text is drawn at `TEXT_LEFT/TEXT_TOP` pixel offsets.
- **Hit testing**: `cosmic_text::Buffer::hit(x, y) -> Option<Cursor>`
  exists (cosmic-text 0.18.2, `buffer.rs:923`) and returns a cursor in
  the shaped buffer's (line, byte-index-within-line) space — the same
  line-relative space QB3 taught us to rebase via the slice line
  table.
- **Scroll wheel**: the GPU owns viewport + scroll state (S1:
  `scroll_top` / slice reshape / viewport re-declaration), but
  **no winit `MouseWheel` handler exists today** (fact-check: the
  window_event catch-all swallows it; scrolling is keyboard-driven).
  M-2 adds the handler — it composes from existing pieces
  (`scroll_top` adjust → reshape → `viewport_send_if_changed`) and
  needs no wire.

## Q#M1 — the wire shape (resolves Q#B5)

How does a pixel frontend tell the instance where a click landed?

- **(α) Reuse `FrontendEvent::Mouse` with synthesized cell coords.**
  The GPU would approximate `(row, col)` from pixels. REJECTED: the
  daemon's cell math runs against window geometry the GPU doesn't
  share (placeholder 24×80), inline adornments shift visual columns
  with no cell-space representation, and the design doc's contract is
  explicit — *the instance never learns a pixel, and there is no
  hit-test round trip*. α is broken-by-design, not merely lossy.
- **(β1) Local hit-test → byte-position pointer events.** The GPU
  resolves pixels to a **source byte offset** locally (it owns the
  layout) and ships gesture-level events; the daemon replays its
  existing mouse state machine in byte space. Selection semantics
  (CUA, double-click word select, future triple-click) stay
  instance-side, exactly like the Key round-trip philosophy.
- **(β2) Frontend-computed selection states.** The GPU computes
  cursor+selection itself and ships final states. REJECTED: forks the
  gesture semantics into a second implementation the TUI doesn't
  share, and fights the daemon-authoritative selection model the CUA
  arc just consolidated.

**Stance: β1.** New wire variant:

```rust
FrontendEvent::Pointer {
    frontend_id: FrontendId,
    buffer_id: BufferId,
    /// Source byte offset the frontend hit-tested locally.
    byte: u64,
    kind: PointerKind, // Down | Drag | Up | DoubleDown
    mods: Modifiers,
}
```

`DoubleDown` is detected frontend-side (the frontend knows pixel
proximity and its own double-click interval; the daemon's cell-based
double-click detection cannot see pixels). The daemon maps:
Down → activate semantic window + set cursor + `begin_selection`;
Drag → cursor move (region grows); Up → collapse-if-empty;
DoubleDown → `select_word_at_cursor`. This reuses the same core
primitives `dispatch_mouse` calls today — no new selection logic.

Protocol: bump `PROTOCOL_VERSION` to 5; the GPU sends `Pointer` only
when `hello.protocol_version >= 5`; the daemon ignores the variant
for sessions that lack `semantic_render`.

## Q#M2 — projected→source byte mapping

`Buffer::hit` answers in *shaped* coordinates: line index + byte
offset within the shaped line, where shaped text = source slice with
adornment text spliced in. Two-step mapping:

1. Shaped (line, index) → shaped-buffer byte offset via the shaped
   line table (the QB3 rebase, but over the *projected* text).
2. Projected byte → **source byte** via a run map built during
   `reshape`. **This is new work, not a current by-product**
   (fact-check: `RichChunk` today carries only `text` + `color`, no
   source range — `main.rs:2269`): `projected_rich_chunks` walks
   (source-run | adornment-run) chunks in order, so M-2 extends it to
   also emit
   `Vec<ProjectedRun { projected_start, len, source_start: Option<u64> }>`.
   A hit inside a source run maps linearly; a hit inside an adornment
   run snaps to the adornment's source anchor.

**Stance:** build the run map in `reshape` (it is O(chunks), walked
there anyway) rather than re-deriving adornment offsets at click
time. Clicks land between keystrokes, so the map being rebuilt per
reshape costs nothing new.

## Q#M3 — gesture scope for the first session

**In:** left-click cursor placement, left-drag selection,
double-click word select, wheel scroll (local-only, no wire).
**Deferred:** triple-click line select (needs a core
`select_line_at_cursor`), middle-click paste (needs clipboard
session), right-click menu (needs GUI chrome), drag auto-scroll at
window edges (needs a repeat timer; record as a known gap),
minimap click-to-jump (local-only; natural follow-up).

## Q#M4 — interaction with optimistic state

A pointer event is a round-trip input: it must (a) mark
`cursor_fresh = false` until the daemon's `CursorByte` answers, and
(b) defer behind the optimistic-cursor floor exactly like round-trip
keys do (`defer_round_trip_key_if_needed` shape) — a click landing
between unconfirmed keystrokes would otherwise race the cursor
confirmation. Deferred pointer events should keep only the **latest**
Down/Drag (coalescing, like the TUI's frame-boundary mouse
coalescing) rather than queueing every drag sample.

## Predicted findings (categorical bets, scored at session close)

1. **Coordinate-space bug** (the QB3 family): some offset in the
   pixel→shaped→projected→source chain is wrong on first
   implementation — most likely the adornment run snap or the
   `TEXT_LEFT/TEXT_TOP` subtraction. Surfaces only in manual
   validation with inlay hints visible.
2. **Window-activation gap** (the B1 family): the daemon-side Pointer
   handler forgets some part of what `activate_and_position` does for
   grid windows (window focus, `goal_col` reset), so a click works
   but a subsequent arrow key moves from the wrong anchor.
3. **Selection-wash latency**: the wash arrives a round trip after
   the drag (Decorations cadence), reading as rubber-band lag. If it
   surfaces, the fix is frontend-local provisional selection — out of
   scope, record it.
4. **Coalescing**: drag streams at pixel rate (hundreds of
   events/sec) flood the socket or the dispatcher. Mitigation
   in-scope: send Drag only when the hit byte changes.

## Session plan

- **M-1 (wire + daemon)**: `PointerKind`/`Pointer` variant, version
  bump, daemon handler reusing core primitives, unit tests through
  `handle_dispatcher_event`. Compile-green alone.
- **M-2 (GPU)**: winit mouse events → hit test → run map (new, from
  `projected_rich_chunks`) → Pointer sends; drag coalescing on byte
  change; NEW local wheel-scroll handler; optimistic-floor deferral.
  Manual validation gate: click placement with inlay hints on the
  same line, drag selection, double-click, wheel — in both pmacs-gpu
  and a TUI peer simultaneously.
