# pmacs-gpu quad-backgrounds audit

Date: 2026-05-29

Closes the milestone framed in
[`pmacs-gpu-quad-backgrounds-framing.md`](pmacs-gpu-quad-backgrounds-framing.md),
which retires Phase A finding **A8** (background-bearing decoration
kinds storable but unrenderable). Scope: sessions 9.1 (`Selection`),
9.2 (`CurrentLine`), and 9.3 (peer-presence sourcing + the perf and
correctness fixes the manual validation forced).

## Evidence

| Session | Surface | Evidence |
|---|---|---|
| 9.1 | `Selection` quad backgrounds | Reused session-7's `QuadRenderer`; added `decoration_kind_to_bg_color` + glyph-extent rect builder + render-order (Q#2 α). Merged before visual validation. |
| 9.2 | `CurrentLine` quad backgrounds | Producer emits `CurrentLine` from the active window cursor (Q#1 α); cadence falls out of the M11.4 diff (Q#3 β, no extra state). |
| 9.3 | Peer-presence sourcing | pmacs-gpu consumes `PresenceUpdate`; `Selection`/`CurrentLine` washes track the editing peer. Manual validation drove three follow-on fixes (QB1–QB3). |

## Findings

| ID | Finding | Category | Class | Resolution |
|---|---|---|---|---|
| Bet #1 | Multi-line vertex decomposition | Geometric-decomposition | — | Surfaced as **QB3** (worse than predicted: not just multi-line, *any* line past 0). |
| Bet #2 | `Selection` ↔ `CurrentLine` overlap precedence | Convention-vs-contract | — | Did not surface as a fix. Alpha-blend in M11.4 sort order (CurrentLine under Selection) reads correctly; no precedence contract needed. |
| Bet #3 | `CurrentLine` cadence floods the consumer | Producer-cadence | Small | Structurally right, implementation smaller: the existing M11.4 diff suppresses same-line re-emits; no `last_cursor_line` cache needed. |
| QB1 | Per-window `Selection`/`CurrentLine` are inert in a read-only mirror (own cursor pinned at 0, no input path) | State-derivation-location | Structural→resolved | Source the washes from `PresenceUpdate` (the editing peer), consumer-only; wire already flowed. |
| QB2 | Daemon recomputes the whole-file projection every tick for the semantic frontend — `scoped_style_spans` runs the tree-sitter query over the entire declared (whole-buffer) viewport + clones the theme, gating TUI input on the shared loop | Producer-frequency | Small (perf) | `StyleGate` (parse-bundle `Arc` + generation + viewport) skips the query on cursor-only ticks; `scoped_decorations` materializes the rope once per call instead of twice. |
| QB3 | Background washes blind past line 0: `LayoutGlyph::{start,end}` are line-relative but were compared against whole-buffer byte ranges | Library-API-verification | Small (correctness) | Rebase glyph offsets by `line_byte_offsets[run.line_i]` before comparison. |

## Predicted vs actual

| Predicted bet | Result | Count |
|---|---:|---:|
| #1 geometric decomposition | Surfaced (as QB3) | 1 |
| #2 overlap precedence | Did not surface | 0 |
| #3 cadence | Surfaced, smaller than predicted | 1 |

Unpredicted categories that surfaced:

| Category | Count | Findings |
|---|---:|---|
| State-derivation-location (read-only mirror has no own cursor) | 1 | QB1 |
| Producer-frequency (whole-file recompute per tick) | 1 | QB2 |
| Library-API-verification (line-relative glyph offsets) | 1 | QB3 |

Score summary:

- Predicted categories surfaced: 2 of 3 (#1 via QB3, #3).
- Predicted not surfaced: 1 of 3 (#2 overlap precedence — alpha-blend
  sufficed).
- Unpredicted categories surfaced: 3 (QB1 mirror sourcing, QB2 perf,
  QB3 glyph-offset space).
- Small findings absorbed: 5 (Bet #3, QB1, QB2, QB3, plus the framing
  doc's already-recorded alpha-visibility tweak).
- Structural findings deferred: 0 (QB1 was structural-shaped but
  resolved consumer-only without a contract change).

**Methodology note.** The three load-bearing findings were all
*unpredicted*. The categorical bets were aimed at the wire-shape and
composition surfaces; the real failures were in (a) the read-only
mirror's cursor semantics, (b) per-tick producer cost, and (c) a
cosmic-text API misread. The last echoes the standing M10.10
library-API-verification lesson: glyph-offset coordinate space should
have been verified against the cosmic-text source before the first
glyph-extent rect builder shipped (session 9.1), not after manual
validation in 9.3.

## Close

A8 is closed for the foreground+background decoration arc:

- `Selection`: peer selection renders as a translucent background.
- `CurrentLine`: peer cursor line renders as a subtle wash.
- Both source from `PresenceUpdate`; own-window decorations stay
  forward-looking for when pmacs-gpu gains its own input (Phase B).

Not closed (documented, deferred):

- `SearchMatch` / `SearchMatchActive`: awaiting a search feature in
  pmacs core (framing Q#4).
- Per-peer stable colors, peer caret glyph + label: single-peer mirror
  reuses the Selection/CurrentLine colors.
- Own-cursor vs peer-cursor merge once pmacs-gpu has input.
- Consumer-side per-frame minimap rebuild (cacheable like `StyleGate`)
  — not load-bearing after QB2; revisit if frame timing shows it.

Debug aids retained (off by default): `PMACS_GPU_DEBUG_PRESENCE`,
`PMACS_GPU_DEBUG_FRAME`.
