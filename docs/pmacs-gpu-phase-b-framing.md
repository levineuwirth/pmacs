# pmacs-gpu — Phase B framing (input & editing parity)

**Status: framing pass; pre-implementation.** Opens Phase B from
[`pmacs-gpu-design.md`](pmacs-gpu-design.md) ("editing parity with the
TUI"). Phase A + the quad-backgrounds arc shipped a read-only mirror;
Phase B makes pmacs-gpu accept input and edit the shared buffer. Audit
at Phase B close goes to `docs/pmacs-gpu-phase-b-audit.md` (future).

This is a milestone-framing artifact in the same shape as
[`pmacs-gpu-quad-backgrounds-framing.md`](pmacs-gpu-quad-backgrounds-framing.md):
load-bearing decisions are committed here, before code, with predicted
findings and a scoring method recorded up front.

## Why this exists

pmacs-gpu renders text, syntax, diagnostics, inlay hints, the minimap,
and selection/current-line washes — all read-only. It cannot accept a
keystroke. That single gap is what separates "an impressive viewer"
from "a real editor," and it is Phase B's whole job.

The framing claim, load-bearing for everything below: **the daemon
side already exists.** `apply_event` routes `FrontendEvent::Key`
through `editor.dispatch_key(fid, …)` — the *same* keymap + command +
Lua stack the TUI drives (`src/daemon.rs` ~1910). pmacs-gpu already
applies inbound `CrdtOp`s into its loro replica. So the work is almost
entirely **consumer-side**: capture winit keys, translate them to
`FrontendEvent::Key`, send them, consume the `CursorByte` that comes
back, and draw a caret. Editing parity then falls out of the Key
round-trip — every TUI binding, command, and Lua extension works in
pmacs-gpu the moment keys flow, with no editing logic duplicated in the
frontend.

## Contract inheritance

From `pmacs-gpu-design.md`: *the instance never learns a pixel.* Phase
B preserves this. pmacs-gpu sends semantic input — `Key` events and
(later) byte-resolved cursor targets — never pixels. Mouse-click
positioning resolves pixel→byte locally and sends a byte, never a
hit-test round trip. If a Phase B path would require the instance to
learn pixels, that is a load-bearing finding (classify, defer).

## Scope inheritance

What already exists (daemon + protocol):

| Piece | Status |
|---|---|
| `FrontendEvent::Key(KeyEvent)` wire variant | Exists (`KeyEvent { frontend_id, key: Key, mods: Modifiers, timestamp_ns }`). |
| Daemon dispatch of `Key` through the keymap/command stack | Exists — `apply_event` → `dispatch_key`, identical to the TUI path. |
| `CrdtOp` broadcast of daemon-authored edits to replicas | Exists. A Key-driven edit is queued with `CrdtOpOrigin::DaemonKey` (`src/editor_core.rs`), which excludes **no** recipient in `broadcast_crdt_op` (`src/presence.rs`) — so pmacs-gpu receives and applies the edit it triggered. (Contrast a frontend-authored `CrdtOp`, which *is* sender-excluded.) |
| pmacs-gpu inbound `CrdtOp` apply into the loro replica | Exists (session 3). |
| `CursorByte { buffer_id, byte_pos }` per-tick for replicas | Daemon already emits it for pmacs-gpu's own window; pmacs-gpu currently **drops** it at `_ => None`. |
| Own-window `Selection`/`CurrentLine` producer emission | Exists (9.1/9.2) but inert — pmacs-gpu's window cursor is pinned at 0 because no input ever moved it. Phase B's cursor motion activates it. |
| `AttachClient` write half | Exists; sends only `Viewport` today. Adds a `send_key` (and later `send_cursor`) sibling. |

What is missing (the Phase B work):

| Gap | Session |
|---|---|
| winit `KeyEvent` → protocol `Key` + `Modifiers` translation | B1 |
| `AttachClient::send_key` + wiring `window_event` keyboard input to it | B1 |
| Consume `CursorByte` → track pmacs-gpu's own cursor | B1 |
| Render a caret at the own cursor | B1 |
| Reactivate own-window `Selection`/`CurrentLine`, reconciled with peer presence | B1 |
| Char / Backspace / Enter / Delete editing via the Key round-trip | B2 |
| Command parity validation (C-x C-f, M-x, save, …) + chord/modifier gaps | B3 |
| Mouse click → byte cursor, scrolling, blink, optimistic apply | B4+ / deferred |

## Predicted findings — categorical bets

Committed before any Phase B code lands:

| # | Bet | Category |
|---|---|---|
| B1 | winit→protocol key translation gaps — modifiers, dead keys, IME/composition, keypad, the Character-vs-Named split | Library-API-verification (winit keyboard model) |
| B2 | Key round-trip latency visible during fast typing — each keystroke is a socket round trip + a daemon render pass + a re-render | Round-trip-latency-vs-consumer-need |
| B3 | Own-cursor vs peer-presence decoration reconciliation — once pmacs-gpu has its own cursor, own `CurrentLine`/`Selection` (from `current_decorations`, suppressed since QB1) must coexist with peer washes without double-painting | State-derivation-location |
| B4 | Caret byte→pixel placement re-hits the QB3 trap — `LayoutGlyph` offsets are line-relative; the caret must rebase via `line_byte_offsets[line_i]` | Library-API-verification (cosmic-text, recurrence of QB3) |

Bet B4 is pre-flagged precisely because QB3 already burned us on glyph
coordinate space; the caret-positioning code must reuse
`line_byte_offsets` from the start, not rediscover it.

## Scoring methodology (committed before data lands)

Category matrix, not a count, reported at Phase B close: predicted
categories that surfaced (true positives), predicted that didn't
(false positives), unpredicted that surfaced (false negatives), and
the count within each. Recorded now to avoid post-hoc criteria.

## Forced decisions

### Q#B1 — editing model: stance (Key round-trip)

**pmacs-gpu sends `FrontendEvent::Key`; the daemon's existing
`dispatch_key` + keymap + command + Lua stack produces the edit; the
resulting `CrdtOp` broadcasts back and pmacs-gpu applies it.** No edit
interpretation in the frontend.

Reasoning: this is the daemon-as-authority design realized. It buys
*full* editing parity — every keybinding, command, minibuffer flow,
and Lua extension works in pmacs-gpu with zero duplicated logic. The
alternative (frontend interprets edits, authors its own `CrdtOp`s
optimistically) reimplements editing, bypasses the Lua keymap, and is
only justified if round-trip latency proves visible. Over a local Unix
socket the round trip is sub-millisecond; defer optimistic local apply
to Phase C, gated on bet B2 actually surfacing.

This is consistent with Q#1 (β) from the design doc: Q#1's
frontend-owns-visual-motion stance is about **soft-wrap** visual
motion (arrow-down in a wrapped line → frontend computes the
visual-next byte). pmacs-gpu has no soft wrap yet, so logical line ==
visual line and sending raw arrow keys to the daemon is correct.
Revisit Q#1 when soft-wrap lands (its own session); until then, Key
round-trip does not violate it.

### Q#B2 — caret rendering: stance (quad bar, no blink)

**Render the caret as a thin quad bar via the existing
`QuadRenderer`**, positioned at the cursor byte mapped through the
layout (rebased per Q#B4 / bet B4). No blink in B1.

Reasoning: reuses the shipped quad pipeline (no new render path), and
a static bar is unambiguous for validation. Blink (configurable rate,
per the design doc's cursor scope) and block-style/multi-cursor are
additive refinements once the position is proven correct.

### Q#B3 — own cursor source: stance (consume `CursorByte`)

**pmacs-gpu tracks its own cursor from the daemon's `CursorByte`**,
not from local key interpretation. A key is sent; the daemon moves the
authoritative window cursor; `CursorByte` reports it back; the caret
follows.

Reasoning: keeps the daemon authoritative (consistent with Q#B1) and
avoids the frontend guessing where the cursor "should" be after a
command it didn't interpret (a command may move the cursor anywhere —
`C-a`, search, jump). The own cursor is whatever the daemon says it is.
Cost: one round trip of caret latency, same as bet B2's editing
latency; acceptable on a local socket, revisited only if it surfaces.

### Q#B4 — own vs peer decoration reconciliation: stance (own + peers, distinct roles)

**Once the own cursor moves, render own-window `CurrentLine`/`Selection`
(from `current_decorations`, un-suppressing the QB1 path) *and* peer
presence, with the own decorations authoritative for "my" cursor and
peer presence for "their" cursors.** Single-peer mirror today shows at
most one of each; the per-peer-color work (deferred from QB1) becomes
relevant only with ≥2 editing peers.

Reasoning: QB1 suppressed own-window decorations because they were
inert (cursor pinned at 0). Phase B makes them live, so the suppression
must lift — but peer presence stays for *other* frontends. The
consumer needs a clear rule: own decorations come from
`current_decorations`; peer decorations from `peer_presences`; both
render, and a future per-peer palette distinguishes them. This is the
"own-vs-peer merge" item the quad-backgrounds audit deferred.

### Q#B5 — mouse / scroll / IME: deferred

Mouse-click-to-cursor needs either a new `FrontendEvent::SetCursor`
wire variant (the design doc's Q#1 references one that does not yet
exist) or a byte-resolved `Mouse` event; either is a wire decision
deferred to its own session. Scrolling (pmacs-gpu renders from the top
today, no scroll) and IME/composition are likewise deferred. B1–B3
cover keyboard-driven editing of a viewport-sized file; the rest is
scoped after the round-trip is proven.

## Finding feedback loop

Rule (iii) from `pmacs-gpu-design.md` carries forward: small findings
(≤ half-day, no contract change) absorb into the session; structural
findings (contract change, pixel-purity break, cross-cutting) pause,
classify, and defer to a scoped milestone. Classification at
surface-time; the Phase B audit records each.

**Process correction carried from the quad-backgrounds arc:** QB1 and
QB3 stayed latent because 9.1/9.2 merged on CI-green without visual
validation. Phase B sessions are **not merged until the input behavior
is visually confirmed in a running pmacs-gpu** (typing appears, caret
tracks, edits propagate to the TUI). CI-green is necessary, not
sufficient, for input-path work.

## Session plan

| Session | Work | Visual probe at close |
|---|---|---|
| **B1** | Key translation (winit→protocol) + `send_key` + consume `CursorByte` + caret render + un-suppress own decorations (Q#B4). Cursor-motion keys only (arrows, Home/End, PageUp/Down). | Arrow keys move the caret in pmacs-gpu; own current-line wash tracks it; the TUI is unaffected. |
| **B2** | Editing keys through the round trip: Char, Backspace, Enter, Delete. | Type in pmacs-gpu → text appears and propagates to the TUI; deletes work; CRDT stays converged. |
| **B3** | Command/chord parity: control + meta chords, `C-x C-f`, `M-x`, save. Translation-gap hardening (bet B1). | Open a file, run a command, save — all from pmacs-gpu. |
| **B4+** | Mouse→byte cursor (needs Q#B5 wire decision), scrolling, caret blink, optimistic apply (only if bet B2 surfaced). | Per-session. |

The sequence is allowed to bend as findings surface; the framing
commits the structure and the Q-decisions, not the exact order.

## Deliberately not committed (framing-pass scope)

- **Exact caret width/color** — picked in B1 (likely a 2px bar in a
  high-contrast foreground).
- **`SetCursor` wire variant vs byte-resolved `Mouse`** — Q#B5, decided
  when mouse lands.
- **Optimistic local apply** — Phase C, gated on bet B2.
- **Soft-wrap + Q#1 revisit** — out of Phase B's initial scope; its own
  milestone when visual motion grows past logical lines.
- **Multi-cursor / peer-color palette** — additive; Q#B4 leaves room.

## Phase B (initial arc) close criterion

The input arc closes when B1–B3 ship and, visually confirmed in a
running pmacs-gpu: keyboard cursor motion tracks, text editing
round-trips and propagates to the TUI, and at least one command + save
works — with the Phase B audit recording the predicted-vs-actual
scoring. Mouse, scroll, blink, and optimistic apply remain documented
and deferred.
