# Mouse deferred set — framing pass

Date: 2026-06-12. PR #61 (mouse framing Q#M1–M3) shipped click, drag,
double-click word select, and wheel scroll, deferring four gestures.
This session takes them. Survey facts: `dispatch_pointer`
(editor.rs:800) ignores `mods`; `classify_pointer_down`
(pmacs-gpu main.rs:2073) keeps a one-deep click history and a comment
promising triple-click; the minimap has pure geometry (x =
`width - MINIMAP_RIGHT - MINIMAP_WIDTH`, linear y→line) but clicks
over it fall through to text hit-testing; no repeating tick exists in
the winit loop; `begin_selection` / `select_word_at_cursor` are the
daemon's reusable primitives, and nothing daemon-side selects a line.

## Q#M4 — triple-click line select

**Stance: new `PointerKind::TripleDown`, protocol v7, daemon
`select_line_at_cursor`.** Gesture semantics stay daemon-side (the
Q#M2 contract): the frontend only classifies. The GPU's click history
deepens to two entries so a third same-byte click inside the window
classifies as `TripleDown` (and the chain then restarts). Daemon
selects the line *including* its trailing newline — the convention
that makes consecutive triple-click-drag select whole lines.

The wire note: v6 narrowed `SUPPORTED` to `[6]` because the encoding
broke. `TripleDown` is the old cheap kind of bump — an additive
variant on a frontend→instance enum, gated at send (`>= 7`), so
`SUPPORTED` becomes `[6, 7]` on both sides and the compat ladder
restarts on top of the v6 floor.

## Q#M5 — Shift-click selection extension

**Stance: daemon-side, zero wire change.** `mods` already rides every
`Pointer` (carried "for future Shift-click" since v5). In
`dispatch_pointer`, a `Down` with SHIFT extends instead of restarts:
keep the existing anchor when a selection exists, else anchor at the
*pre-click* cursor; then move the cursor to the clicked byte. Drag
after a Shift-Down extends from that inherited anchor unchanged.
Frontend side, a Shift-Down must not enter the double-click chain
(Shift-click twice ≠ word select).

## Q#M6 — minimap click-to-jump

**Stance: GPU-local, consumed before text hit-testing, scrubbable.**
A press inside the minimap band never becomes a `Pointer` event: it
maps pixel y through the same linear line interpolation the painter
uses, centers the viewport on that line, and ships the new viewport
through the existing scroll machinery (`viewport_send_if_origin_
changed`). Holding and moving scrubs continuously — same mapping per
`CursorMoved` while the press started in the band. The viewport is
frontend-owned (S1), so no daemon involvement at all.

## Q#M7 — drag auto-scroll at window edges

**Stance: `ControlFlow::WaitUntil` tick, ~35ms, armed only while
dragging in the edge band.** While `pointer_drag_active` and the
pointer sits within an EDGE_BAND (~24px) of the text area's top or
bottom, each tick scrolls one line toward the pointer and re-runs the
drag hit-test at the *current* pointer position (the mouse may not
move — `CursorMoved` alone would stall the selection). Disarmed the
moment the button releases or the pointer leaves the band; the event
loop returns to `Wait`. No daemon involvement: the scroll is local,
and the re-fired `Drag` rides the existing pointer path.

## Predicted findings (categorical bets)

1. **Click-chain misclassification**: the deepened history interacts
   wrongly with some sequence (double, pause, click; or Shift-click
   between clicks) and a gesture fires as the wrong kind — surfaces
   as an unexpected word/line selection during validation.
2. **Jump-then-stale styling**: a far minimap jump exercises the
   viewport-end drift / overscan path harder than wheel scroll does;
   one frame of unstyled or stale-styled text flashes after the jump.
3. **Auto-scroll boundary stickiness**: at buffer top/bottom the tick
   keeps firing (or the viewport oscillates against the clamp) —
   surfaces as jitter or a hot loop at the extremes.

## Session plan

Order: Shift-click (daemon only) → triple-click (v7 + daemon + GPU)
→ minimap jump (GPU) → edge auto-scroll (GPU). Tests per piece;
manual validation: shift-click extend in both directions, triple-click
then drag, double-then-triple chains, minimap jump near top/bottom,
drag-select past both window edges.
