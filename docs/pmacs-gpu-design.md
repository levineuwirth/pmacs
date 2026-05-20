# pmacs-gpu — design note

**Status: framing pass closed; pre-implementation.** Sessions queued:
session 1 = `pmacs-protocol` crate extraction; session 2 = `pmacs-gpu`
workspace setup + hello-world rendering; session 3+ = Phase A proper.

This is the post-v1.0 design artifact for the GPU/GUI frontend. It
inherits the contract boundary established by
[`semantic-frontend-protocol.md`](semantic-frontend-protocol.md) and
applies the M10-matured framing discipline (verification-milestone
premise-check, predictive-density, categorical scoring, per-phase audit
docs) to a multi-month effort.

## Why this exists

The M11 producer arc shipped without a real consumer. Everything we've
built — `StyleSpans` (policy A, tree-sitter + LSP), `Decorations`
(diagnostics + selection), `InlineAdornments` (inlay hints),
`FileStyleSummary` (minimap), `CursorByte` — is a *hypothesis about
wire shape* until something renders against it. `pmacs-gpu` is that
consumer. Phase A's adversarial verification of the producer arc is its
load-bearing job; the "read-only viewer that renders text" is a side
effect of validating the wire.

This framing is the most consequential commitment in the note. Phase A
is **not** "build a viewer that works" — it is "exercise every field of
every producer-arc message in conditions that would surface shape
issues, and ship a viewer as the artifact of having done so."

## Contract inheritance

From `semantic-frontend-protocol.md`'s "Contract boundary" section,
verbatim: *the instance never learns a pixel.* Not viewport pixel size,
not DPI, not font metrics, not glyph advances. The only spatial fact
the instance learns is which buffer byte range is on screen
(`FrontendEvent::Viewport`).

Corollary, also from the doc: *there is no hit-test round trip.* The
frontend resolves pixel→offset locally and only ever emits buffer
offsets and edits. `pmacs-gpu` ships sophisticated client logic
specifically to honor this; if a render path would require the
instance to learn pixels, the verification-milestone premise-check
classifies that as a load-bearing finding (see [Finding feedback
loop](#finding-feedback-loop) below).

## Toolkit

**`wgpu` + custom + `cosmic-text` + `glyphon`. Unambiguous.**

### Against `gpui`

The `gpui` option is more attractive than it should be and worth
recording an unambiguous against-paragraph so this decision isn't
relitigated:

`gpui` is built around Zed's editor model — a specific text-buffer
abstraction, a specific event-loop shape, a specific approach to
composing editor UI. Pmacs's model differs in load-bearing ways:
CRDT-backed buffers, attach-protocol-driven rendering, Lua-extensible
behavior, daemon-frontend split. Each difference means either reshaping
pmacs to fit `gpui` (foreign architecture imposed on mature pmacs
design) or reshaping `gpui` to fit pmacs (rewriting parts of a
framework we don't control). Vendoring forks an upstream we'd have to
track; depending pins us to Zed's release cadence and refactors. Six
months of head-start cost the next five years of upgrade friction.

The `wgpu` + custom path has the property that every line of code in
the frontend is ours. The "biggest work item: text rendering" is real
but bounded — `cosmic-text + glyphon` resolve most of it. The
foreign-API surface to misread (the M10.10 library-API-verification
lesson) is far smaller than `gpui`'s.

### Why not the alternatives

- `iced` / `egui` — general-purpose Rust GUI; neither editor-shaped.
  Immediate-mode (egui) fights long-running editor model.
- Tauri / web stack — loses the "pmacs is a real native binary"
  character; adds JS runtime story.
- GTK or other native widget toolkits — not GPU-first, loses the
  framing.

## Scope phases

Sequential, not alternative. Recording the total honestly at
decision-time so future-us doesn't relitigate the timeline:

| Phase | Deliverable | Duration |
|---|---|---|
| **A** | Adversarial verification of the producer arc, artifact = read-only viewer | 2–3 weeks |
| **B** | Editing parity with TUI | 2–3 months after A |
| **C** | Beyond-TUI features (smooth scroll, real inline inlay hints, minimap consumption) | 3–4 months after B |

Total path: ~6 months to TUI parity, ~9–10 months to beyond-TUI. v0.1
of `pmacs-gpu` is the end of Phase A.

## Phase A: adversarial verification

### Scope framing

Phase A's success criterion is *not* "renders correctly." It is
"exercises every field of every producer-arc message in conditions
that would surface shape issues." The choice of what to render is
driven by what the producer arc needs validated, not by what looks
good in a demo.

### Test corpus

Six static probes + one temporal probe. Each is a planned exercise of
a specific wire surface, not an open-file-and-see-what-happens session.

| # | Probe | Wire surface exercised |
|---|---|---|
| 1 | Non-ASCII source (mixed CJK / emoji / combining marks) | UTF-16 col/byte conversion edge cases in `StyleSpans` LSP path; `char_to_byte` correctness across encodings |
| 2 | Mixed tree-sitter + LSP file (Rust with rust-analyzer) | The M_B3 composition path — `merge_styles` behavior when both authorities cover the same byte |
| 3 | Viewport-boundary tokens (token straddles the visible region's edge) | `StyleSpans`/`Decorations` dirty-segment edges; M11.4 clip behavior |
| 4 | 100k-line file | `FileStyleSummary` at scale; per-line dominant-style accuracy; minimap-style consumption |
| 5 | Active diagnostics + multi-frontend `PresenceUpdate` overlap | `Decorations` ↔ `PresenceUpdate` composition; peer-cursor SO_PEERCRED color stability |
| 6 | `\r\n` line endings | `line_offsets` convention; trailing-newline assumptions across producers |
| 7 | **Sustained typing** (1000 keystrokes in 30s synthetic) | Temporal probe — `CursorByte` emission cadence; `InlineAdornments` suppression timing; dirty-segment propagation under fast edits |

The temporal probe is structurally different from the six static
probes. State-shape probes test "does the wire carry the right thing";
the temporal probe tests "do the wire shapes hold under sustained
interaction." Predict 1–2 additional findings from the temporal surface
beyond the categorical bets below.

### Predicted findings — categorical bets

Five named bets, each probing a categorically different failure surface:

| # | Bet | Category |
|---|---|---|
| 1 | `StyleSpans`/`Decorations` dirty-segment edges at viewport boundaries | Headless-test-blind-spot probe |
| 2 | `InlineAdornments` M11.2-level suppression visible-flicker on edit-then-revert | Temporal-interaction probe |
| 3 | `FileStyleSummary` trailing-empty-line behavior may need wire decision | Convention-vs-contract probe |
| 4 | `CursorByte` per-tick cadence may surface as wrong frequency for a 60fps consumer | Producer-frequency-vs-consumer-need probe |
| 5 | `PresenceUpdate` peer-cursor color stability may need `M10.9` SO_PEERCRED discipline ported into the renderer, not re-derived | State-derivation-location probe |

### Scoring methodology (committed before data lands)

The predicted-vs-actual score at Phase A's close is a **category
matrix, not a count**. This is the M10.10 dual-methodology-scorecard
discipline applied here: different measurements capture different
signals; report them separately rather than collapse to one number.

At Phase A close, report:

- **Predicted categories that surfaced** (true positives): which of
  the five bets above produced findings.
- **Predicted categories that didn't surface** (false positives):
  bets that turned out not to be load-bearing.
- **Unpredicted categories that surfaced** (false negatives): genuine
  gaps the model missed entirely.
- **Count distribution within each category**: how many findings of
  each category appeared.

The model is right about *structure* if predicted categories surface
even at low count; it is right about *distribution* only if the count
within each category roughly matches the prediction. A score of "4/5
predicted categories surfaced; 2 unpredicted categories surfaced; one
predicted category produced 3 findings" is a structurally right but
distributively wrong prediction.

This methodology is recorded **before** data lands to prevent the
M10.10 Day-5 reconciliation trap where scoring criteria get decided
after results are known.

### Finding feedback loop

Classification rule (iii) — pre-authorized at framing time, not
discovered mid-implementation:

- **Small finding** (≈ half-day patch, no structural change, no
  contract violation): absorb into Phase A. Patch the producer arc,
  re-verify, continue.
- **Structural finding** (changes a contract, ripples across producers
  or consumers, invalidates a v1.0 assumption, or breaks the
  pixel-pure-instance invariant): apply the
  verification-milestone-premise-check. Defer to a numbered v0.x.
  Phase A ships with the limitation documented; the structural work
  becomes its own scoped milestone.

Classification happens *at surface-time*, not at Phase A's close.
Phase A's framing pre-authorizes both branches; neither blocks the
other. The audit doc for Phase A records each finding with its
classification and the resolution path taken.

## Distribution

**Workspace member: separate `pmacs-gpu` binary.** Single binary with a
flag would force `wgpu` runtime deps (vulkan/metal/dx12) and font
handling into every TUI install. Separate repo would sever the audit
trail and require cross-repo coordination for protocol changes.
Workspace + separate binary is the right shape: shared protocol crate,
independent dep graphs, single audit history.

### Protocol-crate extraction (prerequisite)

`src/protocol.rs` is currently inlined in the main `pmacs` crate. The
extraction is **a discrete prerequisite PR**, not folded into "workspace
setup":

1. Extract `pmacs-protocol` as a workspace crate.
2. Move wire types (`InstanceMessage`, `FrontendEvent`, capability
   structs, `NegotiatedCapabilities`, `BufferId`, `FrontendId`, the
   `SemanticFrame` family, `SelectionSnapshot`, `ResourceBody`, etc.).
3. Leave internal types (serialization wrappers, dispatch utilities,
   format-version constants if internal-only) in the main crate.
4. Use `[workspace.dependencies]` for cross-crate version pinning, not
   path-dependency duplication.

**Budget: 4 hours, not 1–2.** Mechanical if `protocol.rs` is clean;
longer if internal types need disentangling. The wire-vs-internal pass
takes care — mismatched extraction (moving too much or too little)
propagates into `pmacs-gpu`'s dependency graph in ways that are
expensive to undo.

Naming: `pmacs-protocol` is the chosen name for consistency with
`protocol.rs`. Alternatives considered (`pmacs-wire`, `pmacs-ipc`) are
more specific but break the naming continuity.

## Forced decisions

These are decisions the producer arc deferred because no frontend
existed. The framing pass commits each before Phase A starts so the
work isn't blocked on rediscovery.

### Q#1 — visual motion API: stance (β)

The frontend implements visual motion. The instance stays pixel-pure.

Concretely: on arrow-down in a soft-wrapped buffer, the frontend
computes the visual-next byte position locally and sends
`FrontendEvent::SetCursor` with the target byte. The instance never
reasons about wrap.

If Phase A surfaces (β) as unworkable, that's a load-bearing finding
the verification-milestone-premise-check classifies as structural —
defer, scope its own work, don't try to patch in Phase A.

#### Implementation: (β-impl) for Phase A, with documented upgrade path

Phase A starts with **(β-impl)**: the frontend recomputes wrap state
from `BufferMirror` on each visual-motion event. O(viewport) per motion
event; no maintained index. Cheaper to implement; fine if motion is
keyboard-driven (one event per keypress).

**Upgrade to (α-impl)** — a maintained wrapped-line tree indexed by
byte position, incrementally invalidated on edits — becomes necessary
if smooth scroll or kinetic scroll lands in v0.1. The framing
pre-authorizes the upgrade if Phase A's scope grows there.

### Cursor scope at v0.1

In scope:

- **Blink** (configurable rate; standard).
- **Multi-cursor rendering** — pmacs's data model supports multiple
  cursors; rendering is an additive concern. Renders correctly when
  present, even if not heavily exercised in Phase A.
- **Peer cursors** via `PresenceUpdate` with stable per-frontend colors
  honoring `M10.9`'s SO_PEERCRED-stable color discipline. Data already
  exists; rendering is what validates it.

Out of scope:

- **Multi-cursor commands** — TUI doesn't have ways to create multiple
  cursors; the command layer is a separate concern. Multi-cursor
  rendering ships ahead of the commands that exercise it.

### Font at v0.1

Stance (γ): **bundle JetBrains Mono as default; expose Lua hook for
override.**

- Bundled font: JetBrains Mono (Apache 2.0; broad glyph coverage;
  designed for code; ~1.5MB; dwarfed by `wgpu`'s footprint).
- Lua override: `pmacs.gpu.set_font(path)` or similar (precise binding
  shape decided session 2).
- Missing-glyph fallback: tofu (replacement character `U+FFFD`).
  Explicit non-goal to ship a sophisticated fallback chain in v0.1.
  If real users hit this, it's v0.2+ scope.

The bundled-default-plus-override shape means v0.1 works without
configuration; future customization needs no wire-protocol changes.

## Rhythm

The worktree-per-step pattern that drove v1.0 ships individual PRs at
hour-level granularity. The GPU work has portions that don't decompose
that cleanly: text rendering integration, layout, input event handling
are days of work each, not hours.

**The cadence relaxes; the discipline does not bend.**

- Worktree-per-step continues for discrete features (project setup,
  protocol extraction, isolable subsystems).
- Larger features ship at daily-PR cadence: each session is a
  day's worth of work, ending in a commit; multi-day features
  accumulate across sessions, with a final feature-completion commit.

### Discipline anchor: session, not PR

In v1.0's hour-level rhythm, each PR was an audit-doc-finding-recording
event. With daily cadence, PRs are too coarse for the discipline; the
session is the unit.

Concretely:

- A session starts with audit-doc state and ends with audit-doc state.
- Findings surface within sessions; pause-and-report happens at
  session boundaries or mid-session as the finding warrants.
- The session-end commit (or audit-doc update for sessions where no
  code-merge ships) is the rhythm anchor.

The pause-before-implementation, finding-recording-at-surface-time, and
audit-doc-as-canonical-source disciplines are unchanged.

## Audit artifacts

This doc (`pmacs-gpu-design.md`) is the **design artifact**. It lives
in the same documentation tier as
`docs/semantic-frontend-protocol.md` — forward-looking design framing,
read by future contributors and sessions.

**Per-phase audit material lives in separate per-phase audit docs**,
following the M10.x pattern:

- `docs/pmacs-gpu-phase-a-audit.md` — Phase A findings, classification
  decisions, predicted-vs-actual scoring at Phase A close.
- `docs/pmacs-gpu-phase-b-audit.md` — Phase B equivalent. (Future.)
- etc.

This doc updates with cross-references to per-phase audit docs as they
land. Design artifact stays readable; audit material gets the
per-phase shape M10 validated.

## Deliberately not committed (framing-pass scope)

The framing pass closes with the following deliberately deferred to
session 1 or later:

- **`winit`** (the window/event-loop crate). The Rust ecosystem
  alternative is sparse; `winit` is the likely default. Session 1
  confirms; if the ecosystem has shifted, the decision happens then.
- **`pmacs-gpu` internal layering** (UI module structure, threading
  model, frame-budget discipline). Decided session 2 when the
  workspace is set up, not pre-decided here.
- **Acceptance-test shape**. Golden-frame / screenshot-comparison via
  headless `wgpu` rendering is the likely substrate; pixel-exact vs
  perceptual comparison is a separate decision. If acceptance-test
  shape surfaces complications during session 1 or 2, classify as
  structural finding (iii).

## Session plan

| Session | Work | Gate at close |
|---|---|---|
| 1 | `pmacs-protocol` crate extraction. Move wire types out; `[workspace.dependencies]` pattern; main crate keeps internal protocol utilities. | Full gate set; main pmacs binary still passes all v1.0 tests. |
| 2 | `pmacs-gpu` workspace member; `wgpu`/`winit`/`cosmic-text`/`glyphon` deps; hello-world: open a window, render "hello, pmacs" with the bundled font. | Hello-world runs; no protocol consumption yet. |
| 3 | Attach loop — `pmacs-gpu` connects to a running pmacs daemon as a `semantic_render` session; receives `BufferSnapshot`; rebuilds rope locally. | Daemon ↔ frontend handshake; rope reconstruction matches. |
| 4+ | Phase A proper — render the rope; consume `StyleSpans`, `Decorations`, `InlineAdornments`, `FileStyleSummary`, `CursorByte`; work through the test corpus. | Phase A close = predicted-vs-actual scoring against the bets above. |

The session-numbered plan is forward-looking; reality will reshape it
as Phase A surfaces findings. The framing pass commits the *structure*;
the *sequence* is allowed to bend.
