# pmacs-gpu — quad-background framing

**Status: framing pass; pre-implementation.** Retires Phase A finding
A8 (background-bearing decoration kinds storable but unrenderable).
Sessions queued: 9.1 = `Selection` backgrounds; 9.2 = `CurrentLine`
backgrounds; search backgrounds deferred to a later arc.

This is the per-milestone framing artifact for the quad-pipeline work
that closes Phase A's one deferred structural finding. It inherits the
framing discipline of [`pmacs-gpu-design.md`](pmacs-gpu-design.md) and
records load-bearing commitments before any session-9 code lands. Audit
material at Phase-A-finalization close goes to
`docs/pmacs-gpu-quad-backgrounds-audit.md` (future).

## Why this exists

Phase A's audit (`pmacs-gpu-phase-a-audit.md`) scored A8 as a single
**structural** finding deferred from Phase A by design: foreground-only
glyph attributes (cosmic-text 0.18's `Attrs`) cannot render the
background visual that `Selection`, `SearchMatch`, `SearchMatchActive`,
and `CurrentLine` need. The wire and consumer-cache infrastructure for
these decoration kinds already exists; the render pass does not.

The framing claim, load-bearing for the rest of this doc: **the
expensive part of this milestone is gone before it starts.** Session 7
shipped a wgpu quad pipeline for the minimap; this milestone reuses
that pipeline for a second purpose. There is no new shader to author,
no new vertex-buffer abstraction to design, no new blend-mode story.
The work decomposes into vertex generation + render-order discipline +
one missing producer call (`CurrentLine`). That smallness is what
qualifies this as Phase A finalization rather than a Phase B
prerequisite.

## Contract inheritance

From `pmacs-gpu-design.md`: *the instance never learns a pixel.* This
milestone strengthens that invariant rather than testing it. The
producer emits `Decoration { range: ByteRange, kind: DecorationKind }`;
the consumer translates byte ranges into pixel rectangles entirely
from local glyph layout. The pixel-pure-instance invariant is not at
risk in any session-9 work; if it appears to be, classify as a
structural finding and pause.

## Scope inheritance from Phase A

What already flows on the wire and in `pmacs-gpu` state:

| Field | Status at Phase A close |
|---|---|
| `DecorationKind::Selection` producer emission | Shipped (`src/semantic_render.rs:383`, from `win.region()`). |
| `DecorationKind::DiagnosticError/Warning/Info/Hint` producer emission | Shipped; renders as foreground override (not in scope here). |
| `current_decorations: Vec<Decoration>` consumer state | Shipped; populated by the M11.4 dirty-merge path. |
| wgpu quad pipeline | Shipped; minimap rendering uses it. |
| Glyph layout access (`Buffer::layout_runs()`) | Available; cosmic-text 0.18 stable API. |
| `decoration_kind_to_color` foreground helper | Shipped; returns `None` for background kinds (correct per its contract). |

What is NOT closed by Phase A:

| Gap | Resolution path |
|---|---|
| `DecorationKind::Selection` rendering | Session 9.1. Data flows; render pass is missing. |
| `DecorationKind::CurrentLine` producer emission | Session 9.2. Derivable from `vp.cursor`; trivial producer change. |
| `DecorationKind::CurrentLine` rendering | Session 9.2. Same render path as 9.1. |
| `DecorationKind::SearchMatch{,Active}` producer emission | Deferred. Requires a search feature in pmacs core (Lua command + core state + producer wiring). Belongs to a later arc; not this milestone. |
| `DecorationKind::SearchMatch{,Active}` rendering | Deferred with the producer. |

## Toolkit (recurrence)

| Component | Status |
|---|---|
| `wgpu` quad pipeline | Reuse Session 7's `QuadRenderer` (`pmacs-gpu/src/main.rs:375-428`). Same shader, same vertex layout, same alpha-blending state. |
| `cosmic-text` glyph layout | `Buffer::layout_runs()` yields per-visual-line layout; each `LayoutRun` exposes `glyphs: &[LayoutGlyph]` with `start_byte`, `end_byte`, `x`, `w`. |
| `pmacs-protocol` | No wire changes. `DecorationKind` already enumerates all four background kinds. `Decoration` already carries `range: ByteRange`. |

No new dependencies. No new wire-format decisions.

## Predicted findings — categorical bets

Three named bets, each probing a categorically different failure
surface. Committed before session-9 code lands so the post-milestone
scoring is honest:

| # | Bet | Category |
|---|---|---|
| 1 | Multi-line vertex generation produces wrong number of quads on selections that cross visual-line boundaries (soft-wrapped lines, lines wider than viewport). | Geometric-decomposition probe |
| 2 | Overlap composition between `Selection` and a future `CurrentLine` (or between `Selection` and a diagnostic-foreground decoration) needs an explicit precedence rule the producer arc didn't commit to. | Convention-vs-contract probe |
| 3 | `CurrentLine` emitted from the active window's cursor re-emits a full `Decorations` family every cursor-byte tick; the consumer churns its quad buffer on horizontal cursor motion that doesn't change the rendered line. | Producer-cadence-vs-consumer-cost probe |

Unpredicted categories may surface. The audit doc records them at
classification-time per rule (iii).

## Scoring methodology (committed before data lands)

Same shape as Phase A: category matrix, not a count. At session-9
arc close, report:

- Predicted categories that surfaced (true positives).
- Predicted categories that didn't surface (false positives).
- Unpredicted categories that surfaced (false negatives).
- Count distribution within each category.

Recorded before the work to prevent the M10.10 Day-5 reconciliation
trap.

## Forced decisions

These are decisions session 9.x will need to make. The framing pass
commits each so sessions don't rediscover them mid-implementation. Each
follows `pmacs-gpu-design.md`'s Q-numbering convention.

### Q#1 — `CurrentLine` source location: stance (α)

**Stance (α): the producer emits `DecorationKind::CurrentLine` derived
from the active window's cursor.** Concretely, `scoped_decorations`
reads `core.active_window_for(self.frontend_id).cursor` — the same
per-frontend access path it already uses for `Selection` at
`src/semantic_render.rs:378` — and converts that byte position into
a line range via `line_starts` (the line-offset table already built
in the diagnostic branch of the same function). The consumer treats
`CurrentLine` like any other decoration; no consumer-side cursor →
line-range derivation.

Reasoning: centralizing the line-derivation in the producer means the
TUI's future `CurrentLine` highlight (if it ships) reuses the same
derivation. The TUI today does not paint a current-line background;
when it does, deriving it consumer-side in two places would be a
duplication M10's discipline rejects. Producer-side derivation also
gives the producer a place to throttle (Q#3 below).

Stance (α) commits the producer to emit `CurrentLine` even when no
consumer renders it. The wire cost is one extra `Decoration` per
`Decorations` frame, ~16 bytes. Negligible.

### Q#2 — render order: stance (α)

**Stance (α): single render pass, two `pass.draw()` calls.** Quad
backgrounds first, text second. The minimap continues to render last
(at the right margin, after text, in the same pass).

Reasoning: a single render pass per frame is the standard wgpu shape
and matches the existing structure. Adding a second pre-text pass
introduces a second `begin_render_pass` per frame with no
correctness benefit — the same pipeline can be issued twice in one
pass with different vertex-buffer ranges or different `set_vertex_buffer`
calls. The minimap's existing draw-after-text behavior is preserved.

If session 9.1 surfaces a transparency-correctness issue (decoration
backgrounds blending against the clear color instead of against text),
that's a structural finding — classify and consider stance (β) (separate
pre-text pass with `LoadOp::Load` for the text pass).

### Q#3 — `CurrentLine` producer cadence: stance (β)

**Stance (β): the producer emits `CurrentLine` once per visible-line
change, not once per cursor tick.** Horizontal cursor motion within
the same source line is a no-op; only motion that crosses a `\n`
triggers a fresh `Decorations` family.

Implementation: the producer's per-frontend `SemanticRenderState`
tracks `last_cursor_line: Option<u64>`; on each `render_frame` it
reads the active window's cursor, computes the line via the
`line_starts` table, and compares against the cached value. Same line
→ suppress the `CurrentLine` portion of the `Decorations` emission
(but `Selection`/diagnostic portions still emit normally). Different
line → emit a full `Decorations` frame.

Reasoning: cursor moves 60–120 Hz under arrow-key autorepeat; rebuilding
the consumer's quad-vertex buffer that often is wasted work the bet #3
predicts. Throttling at the producer is cheaper than at every consumer.
Same-line motion still updates `CursorByte`; only the `CurrentLine`
decoration is suppressed.

If session 9.2 surfaces a freshness gap (e.g. `Decorations` family
arrives with stale `Selection` after a `CurrentLine` emission), that's
a structural finding — likely indicates the per-line throttle needs to
emit a no-op `Decorations { full: true, segments: [] }` to clear other
decoration kinds.

### Q#4 — search backgrounds: deferred

Search (`Find` / `FindReplace`) is a load-bearing pmacs feature with
no current implementation in pmacs core. The producer cannot emit
`SearchMatch` until search state exists; the consumer cannot test
`SearchMatch` rendering without the producer emitting it.

Defer to the editing-parity arc (`pmacs-gpu-design.md`'s Phase B).
Phase A finalization closes with `SearchMatch{,Active}` rendering
documented-but-not-implemented; the rule-(iii) classification is "small
finding deferred awaiting upstream feature."

## Finding feedback loop

Rule (iii) from `pmacs-gpu-design.md` carries forward unchanged:

- **Small finding** (≤ half-day patch, no structural change, no
  contract violation): absorb into the current session. Patch, verify,
  continue.
- **Structural finding** (changes a contract, ripples across producers
  or consumers, invalidates a v1.0 assumption, or breaks the
  pixel-pure-instance invariant): pause; classify; defer to its own
  scoped milestone or session.

Classification happens at surface-time. The session 9.x audit doc
records classification and resolution.

## Rhythm

The session-anchored cadence from `pmacs-gpu-design.md` applies. Two
sessions are framed for this arc:

- **Session 9.1 — `Selection` quad backgrounds.** Vertex generation
  from `current_decorations` filtered to background kinds; render-order
  change per Q#2; `decoration_kind_to_bg_color` helper; visual probe
  (TUI selection → pmacs-gpu rectangle). Exercises bet #1 directly;
  exercises bet #2 only insofar as `Selection` may overlap diagnostic
  foregrounds.
- **Session 9.2 — `CurrentLine` producer + consumer.** Producer
  emission per Q#1 + Q#3; consumer renders via session-9.1's path.
  Exercises bets #2 (now `Selection` ↔ `CurrentLine` overlap) and #3
  (cadence).

Each session ends in a session-end commit. Worktree-per-step applies
unless a session is short enough to land on `main` directly.

## Deliberately not committed (framing-pass scope)

The framing pass closes with the following deferred to session 9.x or
later:

- **Exact quad colors per kind.** Stance β-ish defaults will be picked
  in session 9.1 (probably `Indexed(4)` translucent for Selection,
  `Indexed(0)` slightly-lighter for CurrentLine). Real users will
  prefer theme-driven; expose via `pmacs.theme` in a follow-up if
  needed. Not framing-time work.
- **Alpha vs reverse-foreground for selection.** Some editors render
  selection by inverting the underlying text color; others by drawing
  a semi-transparent rectangle. Stance: rectangle (matches existing
  quad pipeline; reverse-foreground would require text-renderer
  cooperation we don't want yet). Decided session 9.1 if it surfaces.
- **`SelectionSnapshot` vs `Decorations::Selection` reconciliation.**
  `pmacs-protocol` has both a `SelectionSnapshot` family and a
  `DecorationKind::Selection`. Phase A used the decoration path. If a
  finding emerges that the snapshot path should drive backgrounds
  instead, classify per rule (iii). Out of framing scope.
- **Acceptance-test shape for quad rendering.** Headless `wgpu` golden-
  frame comparison was Phase A's deferred decision; quad backgrounds
  inherit the same defer. Session 9.x may surface complications;
  classify at surface-time.

## Phase A finalization criterion

Phase A finalizes when:

1. Session 9.1 ships and `Selection` renders correctly in pmacs-gpu
   against the Phase A test corpus.
2. Session 9.2 ships and `CurrentLine` renders correctly with the Q#3
   cadence holding.
3. The session-9 audit doc records the predicted-vs-actual scoring,
   matching `pmacs-gpu-phase-a-audit.md`'s shape.
4. `docs/pmacs-gpu-design.md`'s "Phase A" reference updates to point
   at the finalization audit doc.

The structural finding A8 from Phase A is then closed.
`SearchMatch{,Active}` remains documented-but-not-implemented, queued
for Phase B's editing-parity arc.
