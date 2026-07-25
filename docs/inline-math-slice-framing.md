# Inline math — the first vertical slice (framing)

**Revision 3 — pre-implementation, framing only. Ground truth scouted against
canonical `main` @ `352bf0b`, protocol v20, 2026-07-24. Rev 2 closed review
round 1 (F1–F9); rev 3 closes round 2 (R2-1 – R2-4).**

### Round 2 (rev 2 → rev 3)

Verdict: converging — one deletion, one real gap, two nits.

| # | Finding | Closed in |
| --- | --- | --- |
| R2-1 | The tree-sitter paragraph appeared **twice** in Q#MS3; the second was stale rev-1 text. Introduced by rev 2's own rewrite, which added a copy without removing the original | Q#MS3 |
| R2-2 | The F7 italic fix stopped at ASCII, so `$\alpha x$` drew an **upright α beside an italic 𝑥** — mixed styles inside one expression, and Greek is the slice's second flagship | Q#MS2, acceptance 13 |
| R2-3 | §9 sat between §6 and §7 | section order |
| R2-4 | Q#MS11 said a wash "covers" a span; a search match can **partially overlap** (`2$ af` in `before $x^2$ after`), which "covers" leaves unspecified | Q#MS11 |

Carried into acceptance from a round-2 non-finding: the Q#MS10 arithmetic puts
`\frac{a}{b}` near 0.85 and suggests `\frac{x^2}{y}` also clears the 0.6
floor, so criterion 12's fallback case must be **computed rather than guessed**
or it will surprise-pass by rendering.

### Round 1 (rev 1 → rev 2)

Verdict: the slice's shape survived, both load-bearing corrections held, and
nine findings landed — two of them decisions the implementation could not have
proceeded without, one a compliance error.

| # | Finding | Closed in |
| --- | --- | --- |
| F1 | The fraction height budget was never confronted; lines cannot grow (`BASE_CODE_LINE_HEIGHT = 22.0` fixed) and a textstyle fraction does not fit | Q#MS10 (new) |
| F2 | "Contributes no glyphs, reserves width" is a mechanism the chunk model does not have — a `RichChunk`'s only width is its `text` | Q#MS4, B1 |
| F3 | Criterion 10 required source-width boxes while Q#MS4 implied layout-chosen width; the contradiction *is* the caret-toggle reflow question | Q#MS4, acceptance 10 |
| F4 | Q#MS5 makes shaping depend on the caret — a new invalidation edge, and it must read the *effective* caret or flap during optimistic typing | Q#MS5 |
| F5 | Detection had no currency guard (`$5 and $6` pairs) and no newline rule | Q#MS3 |
| F6 | **Factual:** Latin Modern Math is GUST Font License and ~717 KiB, not OFL and ~200 KB | Q#MS7, §9 |
| F7 | Without a math-italic mapping, `$x^2$` renders an upright roman `x` | Q#MS2 |
| F8 | Layout still resolves glyph IDs internally; drawing must pin `Attrs` to the math family or measured and drawn advances diverge | Q#MS6, Q#MS7 |
| F9 | Smaller: "after shaping decisions" contradicts Q#MS4; no selection/wash rule; `$$…$$` degradation untested; `Char` vs `Symbol` unmotivated | Q#MS3, Q#MS2, Q#MS11 (new), acceptance |

Two rev-1 claims were **wrong, not merely imprecise**, and are called out
where they occur: the zero-glyph strut (F2) and the font licence (F6).

Parent arc: `docs/inline-math-framing.md` (rev 2, merged as #154). Sibling
substrate lane: `docs/latex-grammar-math-substrate-framing.md` (rev 3), whose
Stage 1 landed as #144.

This lane builds the **first end-to-end slice** of the parent's four-tier
pipeline: a deliberately small LaTeX-math subset that is detected, parsed,
laid out against a real OpenType MATH table, and **actually drawn on screen**.

## 0. Why a slice, and not "Tier 2 + Tier 3"

The obvious next unit was the parser (Tier 2) plus the layout engine
(Tier 3). It is rejected here for the parent arc's own reason.

The substrate lane's **Q#LX5** refused to land the parser ahead of layout
because *"the `MathNode` shape is only validated once [a layout consumer]
exists"*. That argument does not stop at Tier 2. `MathBox` is only validated
once a **renderer** consumes it: an unrendered layout engine can be
self-consistent and still have the wrong shape — wrong units, wrong origin
convention, a baseline the draw path cannot use. Landing Tiers 2+3 with no
Tier 4 reproduces exactly the objection Q#LX5 raised, one layer up.

So the unit of work is **thin and vertical, not broad and horizontal**: the
smallest grammar subset worth rendering, carried all the way to pixels. Every
layer acquires a real consumer immediately. Breadth — big operators, stretchy
fences, radicals, accents, display math — becomes follow-on work against an
API that has already been exercised rather than one that has only been
designed.

The cost is honest and named in §7: the slice touches
`pmacs-gpu/src/main.rs`'s render path, which two other arcs also want.

## 1. Ground truth (scouted 2026-07-24 @ `352bf0b`)

### 1.1 Crate boundaries — the parent's file placement cannot work

The parent framing's integration table lists `src/math_parse.rs` and
`src/math_layout.rs`, i.e. the **core `pmacs` crate**. Verified against the
tree, that placement is unusable:

- **`pmacs-gpu` depends only on `pmacs-protocol`** (`pmacs-gpu/Cargo.toml:60`;
  there is no `pmacs` dependency). A parser in the core crate is therefore
  **unreachable from the frontend that renders it**.
- **`ttf-parser` reaches only `pmacs-gpu`.** Per-crate check: `pmacs` no,
  `pmacs-protocol` no, `pmacs-gpu` yes (via `fontdb` → `cosmic-text` →
  `glyphon`). A layout module in the core would be a genuinely new dependency
  there, which is not what the parent's C1 established.

Both also contradict the parent's own prose — its design contract ("the
instance never learns a pixel") and its protocol section ("math rendering is a
pure frontend responsibility in v0"). The table was the outlier. Q#MS1 fixes
it.

### 1.2 The GPU text pipeline this slice hooks

- `rebuild_code_slice` (`pmacs-gpu/src/main.rs:6136`) shapes **only the
  visible byte slice**; spans/decorations/adornments arrive in whole-file
  coordinates and are clipped and rebased onto it.
- Per line, `chunks_for_line` (`:5100`) produces `RichChunk`s whose
  `ChunkSource` (`:7715`) is one of `Source { start }`,
  `SourceTab { start }`, `Adornment { anchor }`.
- **Every existing variant is additive.** Adornments (inlay hints) inject text
  *between* source bytes; nothing today *replaces* a source range with a box
  of chosen width. That mechanism is what this slice must build (Q#MS4).
- `build_hit_runs` (`:7739`) derives the projected→source hit map from the
  same chunks that feed glyphon, so the map and the shaped buffer cannot
  disagree. Any new chunk kind must participate here or clicks land wrong.
- Custom drawing precedent: `SquiggleRenderer` (`:2825`) owns its WGSL shader
  and pipeline; the menu/background quad pipeline is the precedent for filled
  rectangles.
- Fonts are embedded with `include_bytes!` from `pmacs-gpu/fonts/` under OFL
  (`JETBRAINS_MONO`, `:63`); `build_font_system` (`:217`) loads them into
  `fontdb`.

### 1.3 The acceptance seam already exists

`headless_or_skip(w, h, text)` builds a real headless GPU state and
`render_offscreen()` returns mapped pixels (`copy_texture_to_buffer` at
`:6570`). `headless_diag_face_recolors_band_counter_despite_unchanged_text`
(`:12022`) is the precedent: render, mutate, render again, and assert on the
pixel difference. Real-GPU tests run under `PMACS_REQUIRE_GPU=1`.

This matters because the slice's central claim — *math is actually drawn* —
is exactly the kind of claim that a non-rendering test would pass vacuously.

## 2. What ships

One PR: detection (inline `$…$` only) → parse → layout against the MATH table
→ draw, for the subset in Q#MS2, with the raw source shown whenever the
cursor is inside the span (Q#MS5).

Explicitly **not** in this slice: display math `$$…$$`, big operators,
stretchy fences, radicals, accents, `\text{}`, style overrides, tree-sitter
injection detection, any wire surface, and the TUI.

## 3. Decisions

### Q#MS1 — Both modules live in `pmacs-gpu`

`pmacs-gpu/src/math_parse.rs` and `pmacs-gpu/src/math_layout.rs`. Not
`src/`, for the three independent reasons in §1.1. This keeps v0 exactly what
the parent says it is — a pure frontend responsibility — and keeps the core
crate free of a font-metrics dependency it has no use for.

If instance-side detection ever lands (the parent's v1 `MathSpans`), the
*parser* may move to a shared crate at that point. Nothing in this slice
should assume it will.

### Q#MS2 — The subset: characters, sub/superscript, fraction

`MathNode` for this slice:

```rust
enum MathNode {
    Char(char),                                 // resolved codepoint: x, 2, +, α
    Group(Vec<MathNode>),
    Script { base: Box<MathNode>, sub: Option<Box<MathNode>>, sup: Option<Box<MathNode>> },
    Fraction { num: Box<MathNode>, den: Box<MathNode> },
}
```

Rev 1 had both `Char` and `Symbol`, each carrying a `char`, with no stated
difference (F9d). Folded: `\alpha` resolves to `'α'` **in the parser**, so
layout sees one kind. Provenance would only matter for error messages, which
Q#MS8 does not produce.

This subset is chosen because it is the smallest one that **forces the MATH
table to matter**. Characters alone could be positioned by guesswork and prove
nothing. Scripts require `ScriptPercentScaleDown`, `SuperscriptShiftUp` and
`SubscriptShiftDown`; fractions require `AxisHeight` and the fraction rule
constants, plus nested box composition. Get those right and the remaining node
kinds are more of the same; get them wrong and no amount of breadth helps.

The symbol map ships as a **seed** (Greek letters only, ~50 entries), not the
parent's full ~200. Growing it is mechanical and needs no design.

**Math italic is in scope, and it covers Greek too (F7, R2-2).** Neither rev 1
nor the parent mentioned italics, and without them `$x^2$` renders an upright
roman `x` — which does not look like math, and would make the slice's flagship
acceptance case visibly wrong.

Rev 2 fixed that for ASCII only, which reintroduced the same defect one symbol
over: `\alpha` resolves to U+03B1 in the parser, so `$\alpha x$` would have
drawn an upright α beside an italic 𝑥 — **mixed styles inside one
expression**, with the Greek seed map being the slice's *second* flagship case.
The mapping therefore follows TeX's actual convention:

| Class | Treatment | Range |
| --- | --- | --- |
| ASCII letters | math italic | U+1D434–U+1D467, **with the U+210E hole for `h`** (Letterlike Symbols, not in the 1D4xx run) |
| Lowercase Greek | math italic | U+1D6FC–U+1D714 |
| Uppercase Greek | **upright** | left at U+0391–U+03A9 |
| Digits, operators | upright | unchanged |

Uppercase-Greek-upright is not an omission; it is what TeX does, and matching
it is why the table is stated rather than left as "letters become italic".
Because the slice positions characters, this stays a pure char→char mapping —
the same mechanical class as the Greek seed itself.

### Q#MS3 — Detection is the frontend byte scanner, inline only, currency-guarded

A two-pass scan over the visible slice for unescaped `$…$` pairs, run in
`rebuild_code_slice` **off the edit path**. (Rev 1 said "after shaping
decisions", inherited from the parent's "post-shape hook"; that contradicts
Q#MS4, since a suppression chunk must exist *before* the line is shaped. The
property that actually matters is that detection does not run per keystroke —
F9a.)

**Currency guards are mandatory, not a refinement (F5).** Rev 1 relied on the
parent's lone-`$` case and would have rendered `prices are $5 and $6 today` as
math over `5 and ` — in exactly the grammar-less prose buffers this rule
targets. Adopt Pandoc's rule:

- an opening `$` must be followed by a **non-space**;
- a closing `$` must be preceded by a **non-space** and not followed by a
  **digit**;
- `\$` is an escape and neither opens nor closes.

**A span may not cross a newline in v0.** Chunking is per line and the visible
slice is line-ranged, so single-line spans are what keep visible-slice-scoped
scanning stable under scroll. A `$` with no same-line partner yields no span.

Tree-sitter injection detection is deliberately not used, even though #144
gives us `math_environment` / `math_delimiter` for `.tex`: that path is
instance-side, the substrate lane already deferred it to this arc, and the
slice must work in the grammar-less buffers where most inline math is typed.
It stays available as the natural upgrade — and it is the principled fix for
currency false-positives, which guards only approximate.

### Q#MS4 — Suppression is a spacer chunk, width-quantized, layout-chosen (F2, F3)

**Rev 1 was wrong about the mechanism.** It said the chunk "contributes no
glyphs… reserves width". A `RichChunk`'s only width *is* its `text: String`
(`pmacs-gpu/src/main.rs:7703`), which `line_from_chunks` feeds straight into a
`BufferLine`; cosmic-text has no zero-glyph strut. There is nothing to reserve
width with except text.

The mechanism is therefore the **`SourceTab` precedent**: `ChunkSource` gains a
variant carrying the suppressed source range, and the chunk projects **spacer
text** — runs of spaces — whose advance covers the box. Reserved width is
consequently **quantized up to whole space advances**, which is a feature, not
a rounding error: the projection stays grid-aligned with the surrounding
monospace text, and hit runs stay integral.

**Width is layout-chosen, not pinned to the source width (F3).** Rev 1 implied
both, and acceptance 10 demanded the latter. Resolved deliberately in favour of
layout-chosen:

- Pinning to source width removes reflow, but `$\frac{a}{b}$` is 13 source
  columns against a box roughly 2 wide, so every fraction would sit in a large
  blank gap. That defect is permanent and visible on every render.
- Layout-chosen width means the line **reflows when the caret crosses a span
  boundary** (Q#MS5 toggles suppression). That is a jump, but it is confined to
  one line, it happens only on a deliberate caret move, and it is the same
  behaviour `org-appear` has trained users to expect from Emacs.

A permanent visual defect is worse than a transient one tied to an explicit
user action. Acceptance 10 is rewritten to match: text *before* the span never
moves, text *after* it moves by exactly the quantized difference, and the
reflow is confined to the affected line.

`build_hit_runs`'s invariant — the hit map derives from the same chunks
glyphon shaped — is not weakened; the new variant participates like any other.
A click inside a math box maps to the **start byte of the suppressed range**,
the same snap-to-anchor rule `Adornment` uses. Sub-expression hit-testing is
deferred; it needs a box→byte map this slice deliberately does not build.

### Q#MS10 — The height budget: fit to the line, or fall back (F1)

The code buffer is one cosmic-text `Buffer` with uniform metrics —
`BASE_CODE_FONT_SIZE = 16.0`, `BASE_CODE_LINE_HEIGHT = 22.0`
(`pmacs-gpu/src/main.rs:362`, `:359`). **Lines cannot grow.** A textstyle
fraction at those metrics is roughly 17 px tall against an above-baseline
budget of ~12–14 px, so a simple fraction is marginal and acceptance 1's own
nested `\frac{x^2}{y}` plainly exceeds. Rev 1 hid this inside Q#MS8's "a box
that would exceed the line" without saying whether that meant width or height,
or what the budget was.

**Rule: the box is uniformly scaled to fit the line box, down to a floor of
0.6×; below the floor the span falls back to source (Q#MS8).** No overdraw, no
reflow of line height, no clipping surprises. The available budget is the line
box less a one-pixel margin, split at the text baseline.

Rejected alternatives, for the record:

- **Overdraw into adjacent lines' leading.** The math pass draws after
  glyphon and *could* paint outside the line box, but a tall fraction would
  then visually collide with the line above — a defect the user cannot fix
  except by not writing math.
- **Growing the line.** Not available: metrics are uniform for the whole
  buffer.

The honest consequence: **v0 shrinks nested math uniformly rather than by
proper style level.** TeX shrinks nested fractions too, but it does so through
display/text/script/scriptscript levels with per-level constants, which is the
real answer and is deferred by name in §6. A uniform scale is a visibly
cruder approximation of the same idea, and it is what keeps the slice thin.

### Q#MS5 — The cursor rule: render math only when the cursor is outside

When the caret is anywhere inside a math span (or on either delimiter), that
span is **not** suppressed — the raw `$…$` renders as ordinary source text.

This is the parent's Q#IM5 proposal ("when the cursor approaches the boundary,
the raw `$` reappears") adopted as a hard rule, and it buys the slice a great
deal: there is no caret-inside-rendered-math problem to solve, because the two
states are mutually exclusive. Editing math shows source; moving away renders
it. Q#IM6's "best-effort fractional cursor projection" is then not needed at
all in v0, and is deferred rather than approximated.

It also gives the feature an honest, self-explaining interaction model, which
is worth more in v0 than sub-glyph caret fidelity.

**This creates a new shaping-invalidation edge, and it is the #120 trap class
(F4).** Today caret motion within the visible slice touches no shaped line:
the `CursorByte` arm updates the cursor and reshapes only on scroll-follow,
and `rebuild_lines_reusing_scroll` (`pmacs-gpu/src/main.rs:5116`) retains
lines on the premise that content and styling are unchanged. Making
suppression a function of the caret breaks that premise. Two obligations
follow, both of which the implementation owns explicitly:

- **Caret motion that crosses a span boundary must dirty the affected
  lines**, and the line-reuse predicate gains suppression state as a third
  input beside content and styling. A retained line computed under the
  opposite suppression state is exactly the stale-mirror failure #120 taught.
- **The rule reads the *effective* caret the frontend draws**, not the last
  confirmed `CursorByte`. The GPU holds an optimistic cursor during
  unconfirmed edits; keying suppression off the confirmed value would make
  spans flap between rendered and source while typing.

Acceptance 7 exercises the behaviour; these two are named here because a test
that only moves the caret and re-renders would pass even if the reuse
predicate were left untouched, as long as something else happened to dirty the
line.

### Q#MS6 — Layout positions CHARACTERS, not glyph IDs

```rust
struct MathBox { width: f32, ascent: f32, descent: f32, items: Vec<MathItem> }
enum MathItem {
    Glyph { ch: char, x: f32, baseline: f32, size_px: f32 },
    Rule  { x: f32, y: f32, width: f32, thickness: f32 },   // fraction bar
}
```

Positions are in pixels relative to the box origin, resolved by the frontend
that owns font metrics — consistent with the parent's contract.

**Characters, not glyph IDs, is a deliberate boundary — on the OUTPUT only
(F8a).** Layout still resolves glyph IDs *internally*: advances and
`MathItalicsCorrection` are glyph-keyed, so a `cmap` lookup happens whatever
the item type. What the boundary buys is that the *emitted* items are
drawable by the existing text machinery.

Glyph-ID **output** exists to select *variants* from the MATH table's
`GlyphVariantRecord` / `GlyphConstruction` chains — precisely what stretchy
fences and big operators need, and precisely what this slice defers. The slice
must not pretend this generalises: when stretchy delimiters arrive they will
need glyph-ID items, and `MathItem` will gain a variant then.

The fraction rule is a filled quad on the existing quad pipeline, not a glyph.

### Q#MS7 — The MATH font and its feature declaration

Bundle **Latin Modern Math** in `pmacs-gpu/fonts/`, embedded with
`include_bytes!` beside JetBrains Mono. Two consumers read the same bytes:
`fontdb`/cosmic-text for drawing, and `ttf-parser` directly for the MATH
table, which cosmic-text does not expose.

**Licence and size, corrected (F6).** Rev 1 said "OFL, GUST" and the parent's
table says "OFL (GUST)". Both are **wrong**. Verified against a local TeX Live
copy, `latinmodern-math.otf` is **733,736 bytes (~717 KiB)** and its own
copyright string reads *"released under the GUST Font License"* — an
LPPL-derived licence, not the SIL OFL. Consequences:

- the bundled licence file must be the **GUST Font License**, named as such,
  not `OFL.txt` (the existing `fonts/OFL.txt` covers JetBrains Mono only);
- the size claim must be honest: at ~717 KiB this becomes **the largest single
  embedded asset in the repository**, roughly 3.5× the figure rev 1 quoted;
- GFL permits redistribution with its licence text, so the plan stands — but
  it is a *different* obligation from OFL and must be discharged as one;
- if OFL-only ever becomes a requirement, **STIX Two Math** is the OFL
  alternative already listed in the parent's font table.

The parent framing carries the same error and needs the same correction; that
is recorded in §9 as a follow-up rather than smuggled into this lane.

**Pin `Attrs` to the math family when drawing (F8b).** Layout measures with
`ttf-parser` against the bundled bytes; drawing goes through cosmic-text. If
fallback selects a different face for `α` or a math-italic `𝑥` than the one
measured, drawn advances diverge silently from computed geometry and the box
is subtly wrong everywhere. The draw path sets the family explicitly and does
not rely on fallback.

Declare the dependency exactly as the parent's rev-2 C1 records:

```toml
ttf-parser = { version = "0.25", default-features = false, features = ["opentype-layout"] }
```

Bare `ttf-parser = "0.25"` unions `std` in and rebuilds the font chain.

A font whose MATH table is absent or unparseable is a **hard startup error in
the math path only** — math spans fall back to raw source (Q#MS8), the editor
does not fail. Bundled-font regressions must not be silent.

### Q#MS11 — Selection, search washes, and peer carets over a box (F9b)

Rev 1 named selection as a falsifier of B4 without proposing a rule. Any
overlay addressed in *source* bytes meets a span whose source is suppressed.

- **A selection endpoint inside a span unsuppresses it.** This is Q#MS5's rule
  generalised from the caret to any selection boundary: if the user is
  addressing bytes inside the math, they see the bytes. A selection that
  merely *spans* the region (both endpoints outside) leaves it rendered.
- **A wash that *intersects* a rendered span washes the whole reserved
  rectangle.** Intersection, not containment (R2-4): for selections the
  distinction is vacuous, since a contiguous selection with both endpoints
  outside a span necessarily contains it — but a **search match can partially
  overlap**, e.g. searching `2$ af` in `before $x^2$ after` matches from
  inside the span to outside it. Search hits and peer highlights paint the
  projected box, never a sub-range of it: the box has no interior byte map
  (Q#MS4), so a partial wash cannot be placed honestly.
- **Peer carets snap to the span start**, the same rule as hits.

This keeps every overlay addressable without inventing a box→byte projection
the slice does not build, and it makes "you are addressing this text" and "you
see this text" the same condition throughout.

### Q#MS8 — Failure is always "show the source"

Unparseable expression, unsupported node kind, missing MATH constant, or a box
that would exceed the line: the span is not suppressed and renders as ordinary
source. The parent's red-squiggle treatment (its Q#IM4) is **deferred** — it
reuses the diagnostic squiggle path, which is a second integration this slice
does not need in order to be correct.

Consequence worth stating plainly: **an unsupported construct is
indistinguishable from ordinary text in v0.** That is acceptable precisely
because the subset is small and documented; it stops being acceptable when
breadth arrives, which is when Q#IM4 should land.

### Q#MS9 — Caching is deferred

The parent's hash-keyed `MathBox` cache is **not** in this slice. Layout runs
per visible span per reshape. This is a slice: the subset is tiny, the visible
span count is small, and an unmeasured cache is a guess. The parent's latency
targets stay as targets; the first measurement comes from this slice's own
render path, and the cache lands when a number justifies its invalidation
cost.

## 4. Bets (falsifiable)

- **B1' (restated after F2) — a spacer chunk composes with the existing
  pipeline.** Reserving width via projected spaces, quantized to whole space
  advances, needs only a new `ChunkSource` variant that `chunks_for_line` and
  `build_hit_runs` already iterate. Falsified if it requires changing how
  cosmic-text shapes the surrounding line, or if quantized spacer width cannot
  keep the projected hit map integral. *(Rev 1's "zero-glyph strut" wording is
  withdrawn: no such mechanism exists.)*
- **B2 — scripts and fractions are enough to validate `MathBox`.** Falsified
  if adding a deferred node kind later forces a change to `MathBox`'s width /
  ascent / descent / origin contract, rather than only adding a `MathItem`
  variant.
- **B3 — character positioning suffices for the subset.** Falsified if any
  node in Q#MS2 cannot be drawn correctly without selecting a glyph variant.
- **B4' (sharpened after F9b) — the cursor rule plus Q#MS11 remove the caret
  problem rather than hiding it.** Falsified if any caret position, selection
  endpoint, search wash, or peer caret inside or across a math span still needs
  a projected-position approximation to behave correctly.
- **B5 — `ttf-parser` supplies every constant the subset needs.** Falsified if
  script or fraction layout requires a MATH value `ttf-parser` does not
  expose.
- **B6 (new, F1) — fit-to-line with a 0.6× floor keeps the subset legible.**
  Falsified if a plain `\frac{a}{b}` at default metrics lands below the floor
  (making the flagship case fall back to source), or if scaled output is
  illegible at the floor. Either outcome means the slice needs real TeX style
  levels rather than a uniform scale, which would be a scope change.

## 5. Acceptance

Parser and layout are pure and get ordinary unit tests. Everything that claims
something reaches the screen runs on a real device through
`headless_or_skip` + `render_offscreen`, under `PMACS_REQUIRE_GPU=1`.

1. **Parser** — `x^2`, `x_i`, `x_i^2`, `\frac{a}{b}`, `\alpha`, nested
   `\frac{x^2}{y}` produce the expected `MathNode` trees. Unbalanced `{`,
   unknown command, and an empty span are errors, not panics.
2. **Detection** — `$x^2$` yields one span; `$a$ and $b$` yields two;
   `\$5` yields none. **Currency guards (F5):** `Price: $5.00` yields none,
   `prices are $5 and $6 today` yields **none** (the rev-1 rule would have
   matched `5 and `), `$ x $` yields none (space after opener), and a `$`
   whose only partner is on the next line yields none.
3. **MATH constants are actually consulted** — layout of `x^2` with the real
   font places the `2` above the baseline and scaled down. Bite: stubbing
   `ScriptPercentScaleDown` to 100% changes the laid-out box, proving the
   constant is read rather than hardcoded.
4. **Fraction geometry** — numerator above, denominator below, rule at the
   axis height, box ascent/descent enclose both.
5. **It renders** — a buffer containing `$x^2$` renders differently from the
   same buffer with the math span suppressed. Asserted on pixels, so a layout
   engine wired to nothing cannot pass it.
6. **The fraction rule is drawn** — `$\frac{a}{b}$` produces horizontal rule
   pixels between the two operand rows.
7. **Cursor rule** — with the caret inside `$x^2$`, the raw `$x^2$` glyphs
   render and no math is drawn; moving the caret out re-renders the math.
   Both directions asserted.
8. **Hit mapping** — a click on a rendered math box places the caret at the
   span's start byte, and the surrounding text's hit runs are unchanged.
9. **Failure shows source** — `$\frac{a$` and `$\unknown{}$` render as
   ordinary source text with no panic and no missing glyphs.
10. **Reflow is bounded and predictable (F3)** — in `before $x^2$ after`,
    `before` occupies identical pixels whether or not the span is rendered;
    `after` shifts by exactly the quantized width difference; no other line
    moves. Toggling via the Q#MS5 caret rule reflows only the affected line.
11. **Line reuse honours suppression (F4)** — moving the caret across a span
    boundary changes the rendered output. Bite: with suppression left out of
    the line-reuse predicate, the retained line keeps the stale state and this
    fails. Suppression follows the **effective** caret, so it does not flap
    during an unconfirmed optimistic edit.
12. **Height budget (F1) — measured, not assumed.** Computed against the
    bundled font, with the budget derived as Q#MS10 defines it (the line box
    less a 1 px margin, baseline placed by the **code** font — JetBrains Mono
    asc 16.32 / desc 4.80 at 16 px inside the 22 px line, *not* the math
    font's own 12.90/3.10):

    | expression | ascent | descent | scale |
    | --- | --- | --- | --- |
    | `x^2`, `\alpha x` | 13.27 | 0.18 | 1.000 |
    | `\frac{a}{b}` | 10.57 | 5.40 | **0.867** |
    | `\frac{x^2}{y}` | 14.91 | 4.75 | 0.986 |
    | nesting depth 2 | — | — | 0.872 |
    | nesting depth 4 | — | — | 0.613 |
    | nesting depth 5 | — | — | **0.540** |

    So **B6 holds** — the flagship fraction renders at 0.867 — and the
    fallback case is **depth 5**. Rev 3 guessed depth 2; the first
    measurement said depth 3 while the fraction gap was still a hardcoded
    `2 × thickness` guess; reading the MATH table's real
    `FractionNumeratorGapMin` / `FractionDenominatorGapMin` (round-3 F4)
    moved the flagship from 0.732 to 0.867 and the boundary to depth 5. The
    round-2 hand-arithmetic estimate of ~0.85 was right all along; the 0.732
    was inflated by the guessed gap.
    Round 2 predicted exactly this trap. Two things worth keeping: depth 2
    scores *higher* than depth 1 because the binding constraint flips from
    descent to ascent as nesting grows asymmetrically, so "deeper is always
    tighter" is false; and the test **searches** for the tripping depth rather
    than hardcoding it, so a font or metric change cannot silently leave the
    fallback arm unexercised.
13. **Math italic (F7, R2-2)** — `$x$` renders the math-italic glyph, not
    roman `x`; `$h$` resolves through the U+210E hole rather than the 1D4xx
    run; digits in `$x2$` stay upright; **`$\alpha$` renders math-italic Greek
    and `$\Gamma$` stays upright**, so `$\alpha x$` is uniformly italic rather
    than mixed.
14. **Overlays (Q#MS11)** — a selection endpoint inside a span unsuppresses
    it; a selection enclosing a rendered span leaves it rendered and washes
    the whole reserved rectangle; a peer caret inside a rendered span draws at
    the span start.
15. **Deferred syntax degrades, not corrupts** — `$$x$$` renders as ordinary
    source text through the empty-span error path, with no panic and no
    half-rendered box (F9c).
16. **Font provenance** — the bundled licence file is the GUST Font License
    and is distinct from the existing `fonts/OFL.txt`; a build with the MATH
    table absent or unparseable falls back to source and surfaces the error
    rather than failing silently (Q#MS7).
17. **Feature declaration is differential, not absolute** — the `ttf-parser`
    feature set from `cargo tree -e features` is **byte-identical with and
    without this crate's dependency line**. Asserting "`std` is absent" would
    be wrong and would fail a correct implementation: `std` is *already*
    enabled upstream, because `fontdb` declares `std = ["ttf-parser/std"]`.
    What the declaration must not do is *widen* the set, which only a
    before/after comparison can show.
18. Full gate suite per `CLAUDE.md`, including `PMACS_REQUIRE_GPU=1`.

## 6. Deferred (named)

Display math `$$…$$` and `\[…\]`; big operators; stretchy fences and glyph
variant/assembly (with the `MathItem` glyph-ID variant they require);
radicals; accents; `\text{}`; style overrides; the full ~200-entry symbol map;
the red-squiggle error treatment (parent Q#IM4); the `MathBox` cache (Q#MS9);
sub-expression hit-testing and caret projection inside rendered math (parent
Q#IM6); colour-by-context (parent Q#IM2); tree-sitter injection detection and
any `MathSpans` wire surface; the TUI's distinct-face fallback; Lua-registered
delimiters; **proper TeX style levels** (display/text/script/scriptscript with
per-level MATH constants), for which Q#MS10's uniform fit-to-line scale is a
deliberately cruder stand-in; **sub-range washes** inside a rendered box
(Q#MS11 washes the whole rectangle).


## 7. Interaction with other work

The slice's Tier 4 half edits `pmacs-gpu/src/main.rs`'s render path, which two
other lanes also claim:

- **Bottom panel** — Stage 1 is in review as **#155**; its **Stage 2** takes
  this render path *and* the next protocol version.
- **Folding Stage 3 (GPU)** — next ranked, still unframed, and inherits the
  `BufferSnapshot` fold-mirror-clear obligation on the same path.

This lane reserves **no protocol version** and adds no wire surface, so it
cannot collide there. For the render path the rule is the one the other two
framings already apply to each other: **whichever lands second re-scouts
against the first.** The parser and layout modules are new files and collide
with nothing; only the `rebuild_code_slice` / chunk / render hunks are
contended, and they are small and localised by design.

Sequencing preference: land after #155's Stage 1, whose merge does not touch
this path, and re-scout if bottom-panel Stage 2 or folding Stage 3 lands
first.

## 8. Prior art in pmacs

`SquiggleRenderer` (`pmacs-gpu/src/main.rs:2825`) for owning a custom pipeline
beside glyphon; the menu/background quad path for filled rectangles; inlay
hints (`ChunkSource::Adornment`) for interleaving non-source content and for
the anchor-snapping hit rule; `headless_diag_face_recolors_band_counter…`
(`:12022`) for asserting a rendering claim on real pixels; #144's query
overlay for the eventual tree-sitter detection upgrade.

## 9. Follow-up outside this lane

The parent framing (`docs/inline-math-framing.md`, rev 2, merged as #154)
carries the same font error F6 found here: its table row reads "Latin Modern
Math | Full | OFL (GUST)". It should be corrected to the GUST Font License,
with the ~717 KiB size, in its own docs change rather than in this branch —
the parent is a merged document and this lane should not quietly edit it.
