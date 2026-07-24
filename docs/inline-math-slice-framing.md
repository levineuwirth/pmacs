# Inline math — the first vertical slice (framing)

**Revision 1 — pre-implementation, framing only. Ground truth scouted against
canonical `main` @ `352bf0b`, protocol v20, 2026-07-24.**

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
    Char(char),                                 // x, 2, +
    Symbol(char),                               // \alpha → U+03B1 (seed map)
    Group(Vec<MathNode>),
    Script { base: Box<MathNode>, sub: Option<Box<MathNode>>, sup: Option<Box<MathNode>> },
    Fraction { num: Box<MathNode>, den: Box<MathNode> },
}
```

This subset is chosen because it is the smallest one that **forces the MATH
table to matter**. Characters alone could be positioned by guesswork and prove
nothing. Scripts require `ScriptPercentScaleDown`, `SuperscriptShiftUp` and
`SubscriptShiftDown`; fractions require `AxisHeight` and the fraction rule
constants, plus nested box composition. Get those right and the remaining node
kinds are more of the same; get them wrong and no amount of breadth helps.

The symbol map ships as a **seed** (Greek letters only, ~50 entries), not the
parent's full ~200. Growing it is mechanical and needs no design.

### Q#MS3 — Detection is the frontend byte scanner, inline only

A two-pass scan over the visible slice for unescaped `$…$` pairs, run where
the parent specifies — in `rebuild_code_slice`, after shaping decisions, not
on the edit path. `\$` is an escape and does not open or close a span. An
unpaired `$` yields no span (acceptance 6 of the parent).

Tree-sitter injection detection is deliberately not used, even though #144
gives us `math_environment` / `math_delimiter` for `.tex`: that path is
instance-side, the substrate lane already deferred it to this arc, and the
slice must work in the grammar-less buffers where most inline math is typed.
It stays available as the natural upgrade.

### Q#MS4 — Suppression is a new chunk kind, and it owns its hit runs

`ChunkSource` gains a variant carrying the suppressed source range and the
projected width the box occupies. The chunk contributes **no glyphs** to the
cosmic-text buffer; it reserves width so the surrounding text lays out around
it, and the math is drawn over that reserved space in a later pass.

The invariant `build_hit_runs` exists to preserve — that the hit map is
derived from the same chunks glyphon shaped — is not weakened: the new
variant participates like any other. A click inside a math box maps to the
**start byte of the suppressed range**, the same "snap to anchor" rule
`Adornment` already uses. Sub-expression hit-testing is deferred; it needs a
box→byte map that this slice deliberately does not build.

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

**Characters, not glyph IDs, is a deliberate boundary.** Glyph-ID work exists
to select *variants* from the MATH table's `GlyphVariantRecord` / `Glyph‐
Construction` chains — which is precisely what stretchy fences and big
operators need, and precisely what this slice defers. Positioning characters
lets each item be drawn with the existing text machinery. The slice must not
pretend this generalises: when stretchy delimiters arrive they will need glyph
IDs, and `MathItem` will gain a variant then.

The fraction rule is a filled quad on the existing quad pipeline, not a glyph.

### Q#MS7 — The MATH font and its feature declaration

Bundle **Latin Modern Math** (OFL, GUST) as `pmacs-gpu/fonts/`, embedded with
`include_bytes!` beside JetBrains Mono, with its licence file. Two consumers
read the same bytes: `fontdb`/cosmic-text for drawing, and `ttf-parser`
directly for the MATH table, which cosmic-text does not expose.

Declare the dependency exactly as the parent's rev-2 C1 records:

```toml
ttf-parser = { version = "0.25", default-features = false, features = ["opentype-layout"] }
```

Bare `ttf-parser = "0.25"` unions `std` in and rebuilds the font chain.

A font whose MATH table is absent or unparseable is a **hard startup error in
the math path only** — math spans fall back to raw source (Q#MS8), the editor
does not fail. Bundled-font regressions must not be silent.

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

- **B1 — the reserved-width chunk composes with the existing pipeline.**
  Falsified if reserving width for a suppressed range requires changing how
  cosmic-text shapes the surrounding line, rather than adding a chunk kind
  that `chunks_for_line` and `build_hit_runs` both already iterate.
- **B2 — scripts and fractions are enough to validate `MathBox`.** Falsified
  if adding a deferred node kind later forces a change to `MathBox`'s width /
  ascent / descent / origin contract, rather than only adding a `MathItem`
  variant.
- **B3 — character positioning suffices for the subset.** Falsified if any
  node in Q#MS2 cannot be drawn correctly without selecting a glyph variant.
- **B4 — the cursor rule removes the caret problem rather than hiding it.**
  Falsified if any caret position, selection, or click inside or across a math
  span still needs a projected-position approximation to behave correctly.
- **B5 — `ttf-parser` supplies every constant the subset needs.** Falsified if
  script or fraction layout requires a MATH value `ttf-parser` does not
  expose.

## 5. Acceptance

Parser and layout are pure and get ordinary unit tests. Everything that claims
something reaches the screen runs on a real device through
`headless_or_skip` + `render_offscreen`, under `PMACS_REQUIRE_GPU=1`.

1. **Parser** — `x^2`, `x_i`, `x_i^2`, `\frac{a}{b}`, `\alpha`, nested
   `\frac{x^2}{y}` produce the expected `MathNode` trees. Unbalanced `{`,
   unknown command, and an empty span are errors, not panics.
2. **Detection** — `$x^2$` yields one span; `Price: $5.00` yields none;
   `\$5` yields none; `$a$ and $b$` yields two.
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
10. **Surrounding layout is undisturbed** — `before $x^2$ after` keeps
    `before`/`after` at the same positions as when the span is not math.
11. **Feature declaration** — `cargo tree -e features` shows `ttf-parser`
    without `std`, i.e. the declaration did not widen the shared feature set.
12. Full gate suite per `CLAUDE.md`, including `PMACS_REQUIRE_GPU=1`.

## 6. Deferred (named)

Display math `$$…$$` and `\[…\]`; big operators; stretchy fences and glyph
variant/assembly (with the `MathItem` glyph-ID variant they require);
radicals; accents; `\text{}`; style overrides; the full ~200-entry symbol map;
the red-squiggle error treatment (parent Q#IM4); the `MathBox` cache (Q#MS9);
sub-expression hit-testing and caret projection inside rendered math (parent
Q#IM6); colour-by-context (parent Q#IM2); tree-sitter injection detection and
any `MathSpans` wire surface; the TUI's distinct-face fallback; Lua-registered
delimiters.

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
