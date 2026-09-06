# Inline math rendering — framing

**Revision 2 — framing only; no implementation. Ground truth re-scouted
against canonical `main` @ `ddaa80d`, protocol v20, 2026-07-24; every
anchor re-checked at `f07b75b` (the vterm PTY-flake fix #153, test-only,
which moved no anchor here).**

Revision 1 was written against protocol v18, before LaTeX Stage 1 (#144),
web grammars (#146), folding Stages 1–2 (#142/#149) and the GPU initial
target (#148) landed. **Revision 2 changes no design decision.** It
corrects the ground truth those merges invalidated, and records the
staging decision the sibling substrate framing already took. A reader who
knows revision 1 can read §0 alone.

## 0. What the landed-state re-scout corrected

| # | Revision 1 said | Current state |
| --- | --- | --- |
| C1 | A MATH-table crate must be added; "neither is in the tree today" | **Both are already in `Cargo.lock`.** `ttf-parser` 0.25.1 reaches `pmacs-gpu` non-optionally via `fontdb` → `cosmic-text` → `glyphon`, and `pmacs-gpu` already calls `fontdb` directly in `build_font_system` (`pmacs-gpu/src/main.rs:217`). `read-fonts` 0.37.0, `skrifa`, `swash` and `font-types` are present too. The MATH module is feature-gated, so the declaration must name its features deliberately — see Tier 3 §A. |
| C2 | Use "one of `ttf-parser` or `read-fonts`" | **Not interchangeable.** `ttf-parser` ships the MATH table (`tables/math.rs`: `Constants::axis_height`, `display_operator_min_height`, `script_percent_scale_down`, the `MathValue`/`MathValues` accessors Tier 3 names). `read-fonts` 0.37.0 exposes no MATH table. Only `ttf-parser` satisfies Tier 3. |
| C3 | "Protocol is v18 … `SUPPORTED=[6..=18]`" (Q#IM1) | Protocol is **v20**, `SUPPORTED=[6..=20]`. v19 = the vterm terminal family; v20 = the GPU initial-target bootstrap. The Q#IM1 *decision* (query font size locally, no protocol change) is unaffected. |
| C4 | `rebuild_code_slice()` at `pmacs-gpu/src/main.rs:4849` | Now `:6136`. The file grew through vterm Stage 3, tab-width parity and the initial-target work. |
| C5 | Tier 1 must patch an upstream grammar or ship an overlay; "defers enumerating which grammars need this" | **Already available for `.tex`.** LaTeX Stage 1 (#144) bundles `codebook-tree-sitter-latex`, whose grammar exposes `math_delimiter` and `math_environment`, and the in-repo overlay `builtin/queries/latex/highlights.scm` already captures both. The overlay mechanism this tier proposed is proven, not speculative. Markdown still has no math capture. |
| C6 | (silent on staging) | A sibling framing, `docs/archive/framings/latex-grammar-math-substrate-framing.md` (rev 3), names this note its parent and **decides Tier 2's staging in Q#LX5**: the parser lands *beside* its Tier 3 consumer, never ahead of it. Recorded in §Tier 2 below so it is not re-litigated. |
| C7 | (silent on contention) | Tier 4's render path is contended. Folding Stage 2 (#149) landed fold projection in the paint path, and its **Stage 3 (GPU) is unframed**. The **bottom-panel arc** is implementing Stage 1 now; its Stage 2 claims both `pmacs-gpu/src/main.rs`'s render path and the next protocol version. Tier 4 must re-scout against whichever lands first. |

Unchanged and re-verified: `InlineAdornment` (`pmacs-protocol/src/message.rs:1391`),
`SquiggleRenderer` (`pmacs-gpu/src/main.rs:2825`), `TextView`
(`src/text_view.rs:45`), and the three referenced framings
(`multi-language-injections`, `pmacs-gpu-wavy-squiggles`,
`semantic-frontend-protocol`) all exist as described.

**Status: framing only; no implementation.**
This note frames a feature that currently does not exist: rendering
LaTeX math expressions as typed math (not raw source) in any buffer.
It proposes a four-tier pipeline and identifies the substrate changes
needed in the Rust core, the semantic protocol, and the GPU frontend.
The TUI frontend is explicitly scoped out (terminal cells cannot
express positioned math glyphs at interactive rates). If this is
adopted, the TUI path would be "display the raw `$...$` source with a
distinct face as a lossy fallback" and nothing more.

The note assumes no dependency on KaTeX / MathJax / JavaScript
runtimes. Everything from parsing through layout to rendering is
native Rust.

## Design contract (inherits the existing semantic protocol boundary)

From `docs/semantic-frontend-protocol.md`: **the instance never learns a
pixel.** Inline math does not reopen this. The instance detects math
ranges, parses LaTeX, and produces a position-independent math layout
tree; the frontend positions glyphs at pixel coordinates using its own
font metrics and viewport. The instance-to-frontend contract for math
is a list of positioned-glyph or font-size-scalable records, not a
pre-rasterized bitmap.

### Contract additions specific to math

1. **Math is text, not an image.** A math expression is selectable,
   copy-pastes as its LaTeX source, and re-renders on edit with no
   round-trip. Invariant: the raw `$...$` text in the rope is always
   the canonical source; the rendered glyphs are a projection.

2. **Cache invalidation is the frontend's job.** The instance does not
   track which expressions are visible or dirty. The GPU frontend
   maintains a math layout cache keyed by source-text hash; only the
   expression under the cursor invalidates on each keystroke.

3. **Display math is a block, not an inline layer.** `$$...$$` regions
   introduce vertical space and center the formula. They collapse to a
   single "block" cell in the base text layout and are rendered as a
   full-width overlay in a separate pass.

## Four-tier architecture

```
   buffer text
       |
   [1. Detection]    — regex / tree-sitter → byte ranges tagged Math
       |
   [2. Parsing]     — recursive-descent → MathNode tree
       |
   [3. Layout]      — MATH-font metrics → positioned glyph runs
       |
   [4. Rendering]   — wgpu / glyphon pass at computed coordinates
```

### Tier 1: Detection

Every buffer, after every edit, scan for math delimiters. The default
set:

| Delimiter | Kind | Notes |
|-----------|------|-------|
| `$...$` | inline | Single-dollar, non-greedy |
| `$$...$$` | display | Double-dollar, greedy across newlines |
| `\(...\)` | inline | Alt inline (LaTeX convention) |
| `\[...\]` | display | Alt display |

The detection layer emits byte ranges with a tag:

```rust
enum MathKind { Inline, Display }
struct MathSpan { start: BytePos, end: BytePos, kind: MathKind }
```

**Where it runs:**
- For buffers with a tree-sitter grammar: a math node in the grammar
  signals scanned injection ranges (reuses the existing `ParseTreeBundle`
  + `Layer` machinery from
  `docs/archive/framings/multi-language-injections-framing.md`). Each grammar that can
  contain LaTeX needs a math capture rule — either by patching the
  upstream grammar or shipping an in-repo query overlay.

  **This is no longer speculative for `.tex` (C5).** LaTeX Stage 1 (#144)
  bundles `codebook-tree-sitter-latex`, whose grammar already exposes
  `math_delimiter` and `math_environment`, and the in-repo overlay
  `builtin/queries/latex/highlights.scm` already captures both (`:57`,
  `:290`). The overlay pattern this tier proposed therefore exists and is
  proven; adding a `@math` capture beside the existing highlight captures
  is an edit to a file the repo already owns, not new machinery. The node
  names are the grammar's own — `math_environment` / `math_delimiter`,
  **not** the `(math_expression)` this framing originally guessed.

  Markdown remains unaddressed: its grammar comes from the crate query
  constants (the #146 web-grammars pattern, no in-repo overlay), so a
  markdown math capture needs the overlay treatment first. Enumerating
  further grammars stays deferred to adoption.
- For buffers with no grammar: a fast byte-level scanner in Rust
  (two-pass: find `$` / `$$` / `\(` / `\[` boundaries, match pairs,
  handle escapes). Runs in the GPU frontend's `rebuild_code_slice()`
  (`pmacs-gpu/src/main.rs:6136`) as a post-shape hook, not the edit
  path, so keystroke latency is unaffected. (The TUI's `src/text_view.rs`
  is a separate line-index view that does not participate.)

**Cost note:** the scan runs on every buffer after every edit,
regardless of whether the buffer is likely to contain LaTeX. A log file
with a bare `$` will be scanned, find no pair, and exit. This is a
negligible cost per edit — the scan is O(length of changed region), not
O(file) — but the framing notes it as a minor inefficiency. An
extension-based gate (only scan buffers whose language is in a
configured set) is a trivial v1 optimization.

**Extensibility:** A Lua hook `pmacs.math.delimiters` registers
additional patterns per mode. Example:

```lua
pmacs.math.add_delimiter("markdown", "`$", "`$")  -- `` $...$ `` in md
```

### Tier 2: Parsing

A recursive-descent parser converts the LaTeX math source into an AST.
LaTeX math mode is a constrained grammar — far smaller than full LaTeX.
The node set covers what appears in real mathematical writing:

```rust
enum MathNode {
    /// Characters and identifiers
    Char(char),
    Identifier(SmolStr),           // \sin, \alpha, x

    /// Subscript / superscript
    Sub(Box<MathNode>),            // _{...}
    Super(Box<MathNode>),          // ^{...}
    SubSuper(Box<MathNode>, Box<MathNode>), // _{...}^{...}

    /// Fractions
    Fraction(Box<MathNode>, Box<MathNode>),

    /// Radicals
    Sqrt(Box<MathNode>),
    SqrtN(Box<MathNode>, Box<MathNode>),   // \sqrt[n]{...}

    /// Big operators (sum, prod, int — limits above/below)
    BigOp {
        op: MathOp,
        lower: Option<Box<MathNode>>,
        upper: Option<Box<MathNode>>,
        sub: Option<Box<MathNode>>,   // \sum_{i=1} vs \sum_{i=1}^\infty
        sup: Option<Box<MathNode>>,
    },

    /// Fences that may stretch
    Fenced {
        left: Delimiter,
        body: Box<MathNode>,
        right: Delimiter,
    },

    /// Accents
    Accent { accent: AccentKind, base: Box<MathNode> },

    /// Style overrides (e.g. \displaystyle)
    Styled(Box<MathNode>, MathStyle),

    /// \text{...} — literal text in math mode
    Text(String),

    /// Generic stretchy: \overline, \underbrace, etc.
    Stretch(StretchKind, Box<MathNode>),

    /// Sequences and groups
    Group(Vec<MathNode>),
}
```

**~500 lines of Rust.** No lookup tables beyond symbol names → Unicode
codepoints (the `\alpha` → `U+03B1` map is ~200 entries for Greek
+ Hebrew + arrows + operators). The parser does not handle macro
definitions, preamble material, or LaTeX3 — only math-mode markup.

**Staging: this tier lands beside Tier 3, never ahead of it (C6).**
`docs/archive/framings/latex-grammar-math-substrate-framing.md` (rev 3) carved out the
frontend-agnostic LaTeX substrate as its own lane and, in **Q#LX5**,
deliberately excluded this parser from it. Reviewer and author concurred
and recorded the decision "so it is not re-litigated". The rationale is
that `MathNode`'s shape is only validated by a layout consumer, so
building the parser first would fix an AST no one has exercised — the
same no-build-ahead discipline folding applied to `BlockAdornments` and
gpu-invocation applied to `FILE`.

The practical consequence: **Tier 2 is not an independently shippable
slice.** It is pure, dependency-free and conflict-free, which makes it
tempting to land alone while other lanes hold the render path; that
temptation is exactly what Q#LX5 refused. A PR that ships the parser
must also ship enough of Tier 3 to exercise the AST.

### Tier 3: Layout

This is the hard part and the place where pmacs would do something no
interactive editor currently does natively: position math glyphs using
an OpenType MATH table. The pipeline:

**A. Font metrics.** Load a math font with an OpenType MATH table. The
candidates, ranked:

| Font | Table quality | License | Notes |
|------|--------------|---------|-------|
| **Latin Modern Math** | Full | **GUST Font License** (not OFL) | Reference, ships with TeX Live, most widely tested; ~717 KiB. Bundled by #158 as `pmacs-gpu/fonts/latinmodern-math.otf` |
| **STIX Two Math** | Full | OFL | Broader Unicode coverage |
| **Cambria Math** | Full | Proprietary | Ships with Office; unavailable on Linux |
| **Libertinus Math** | Full | OFL | Derivative of Latin Modern, wider |

Bundled default: Latin Modern Math. The existing font-loading path
(`build_font_system()` → `fontdb::Database::load_font_source()` in
`pmacs-gpu/src/main.rs:217`, via cosmic-text) already handles .ttf/.otf.

**Reading the MATH table needs `ttf-parser`, and it is already in the
build graph (C1, C2).** Revision 1 said a MATH crate must be added and
that "neither is in the tree today"; both parts were wrong:

- `ttf-parser` 0.25.1 already reaches `pmacs-gpu` through
  `fontdb` → `cosmic-text` → `glyphon` — the same `fontdb` the frontend
  already calls directly. It is not an optional or dev-only path. Adding
  it to `pmacs-gpu/Cargo.toml` declares a dependency the build already
  compiles, so it adds no new supply-chain surface.

  **Declare the feature set deliberately, or the "no rebuild" part stops
  being true.** The MATH module is gated: `ttf-parser` re-exports `math`
  under `#[cfg(feature = "opentype-layout")]`. It is compiled today only
  because `fontdb` asks for it — and `fontdb` asks with
  `default-features = false`, features `["opentype-layout",
  "apple-layout", "variable-fonts", "glyph-names", "no-std-float"]`,
  which is **not** `ttf-parser`'s own default set (that one adds `std`
  and drops `no-std-float`). A plain `ttf-parser = "0.25"` therefore
  unions `std` in and forces a one-time rebuild of `ttf-parser`,
  `fontdb`, `cosmic-text` and `glyphon`. The zero-rebuild spelling is
  `default-features = false, features = ["opentype-layout"]` — a subset
  of what `fontdb` already enables.
- The choice is **not** "one of `ttf-parser` or `read-fonts`". Only
  `ttf-parser` exposes the MATH table (`tables/math.rs`), and it supplies
  exactly the constants this tier names below — `Constants::axis_height`,
  `display_operator_min_height`, `script_percent_scale_down`, and the
  `MathValue` / `MathValues` per-glyph accessors. `read-fonts` 0.37.0,
  though present in the tree, has no MATH table; selecting it would be a
  dead end.

`skrifa`, `swash` and `font-types` are also present (via cosmic-text) but
are not MATH-table providers either. The bundled font itself is still a
real addition: ~200 KB for Latin Modern Math in `pmacs-gpu`.

**B. Box model.** Each `MathNode` lays out into a `MathBox`:

```rust
struct MathBox {
    width: f32,
    height: f32,
    ascent: f32,   // above baseline
    descent: f32,  // below baseline
    italic_correction: f32,
    glyphs: Vec<PosGlyph>,
}

struct PosGlyph {
    glyph_id: GlyphId,
    x: f32,
    y: f32,       // relative to box baseline
    font_size: f32,
    color: Color, // usually inherited from theme
}
```

Layout rules per node type (following Knuth's math layout algorithm,
simplified to the common cases):

- **Group**: lay out children left-to-right, accumulate width. Insert
  italic corrections between adjacent slanted glyphs (from the MATH
  table's `MathItalicsCorrection`).

- **Fraction**: lay out numerator and denominator at 70% font size,
  centered horizontally; draw a rule between them at the math axis
  height (from MATH table `MathConstants::AxisHeight`); the box ascent
  = num.ascent + axis + rule_thickness/2, descent = den.descent + space
  + rule_thickness/2.

- **Subscript / Superscript**: scale to ~70%, shift superscript up by
  `SuperscriptShiftUp` (from MATH table), shift subscript down by
  `SubscriptShiftDown`. If both present, adjust so they don't overlap.

- **BigOp**: select the display-size glyph variant from the MATH table
  (`GlyphVariantRecord` chain). Place lower limit below, upper above,
  using `DisplayOperatorMinHeight` for minimum size.

- **Fences** (`\left(...\right)`): measure the enclosed box height;
  select the smallest fully-enclosing glyph variant from the stretchy
  chain; if the height exceeds the largest single glyph, assemble from
  the `GlyphConstruction` parts (top, bottom, repeatable extender).

- **Sqrt**: lay out the radicand; draw the radical sign extending from
  the top-left to cover the radicand height, using the `RadicalKern`,
  `RadicalExtraAscender`, and `RadicalRuleThickness` constants from the
  MATH table.

**C. Caching.** The key performance insight: a laid-out `MathBox` is
hashable (by the source LaTeX string). The cache is a `HashMap<u64,
Arc<MathBox>>` keyed by SipHash of the source bytes. The GPU frontend
holds this cache across reshape calls. On edit, only the expression
whose source range overlaps the edit range is evicted.

Latency target: < 500 µs for the common case (a 20-node expression).
Worst case (a full-page display equation with nested fractions and
summations): < 5 ms. Cache hit: < 1 µs.

**Bulk-doc note:** for a LaTeX document with hundreds of inline
expressions on screen simultaneously (e.g., a dense math paper at high
zoom-out), each visible expression is parsed and laid out independently.
The 5 ms worst case *per expression* could add up: 50 visible
expressions at 1 ms each = 50 ms. The hash cache absorbs re-parses
across frames (an expression re-appears on re-scroll at hash cost only),
so the expensive path is only the first render of each expression after
an edit or a fresh scroll. For v0 this is acceptable; if it proves hot,
v1 can add a frame budget — parse until deadline, render cached results
for the rest.

### Tier 4: Rendering (GPU frontend)

The GPU frontend (`pmacs-gpu/src/main.rs`) already renders text through
cosmic-text + glyphon with per-span styling. Math expressions are a new
layer inserted into the existing z-order.

**This tier is contended and must re-scout before it is scheduled (C7).**
Revision 1 described this render path as though math were its only
claimant. Two other arcs now converge on it:

- **Folding Stage 3 (GPU)** — Stages 1–2 landed (#142/#149); Stage 3 is
  the next ranked item and is **unframed**. It inherits a named
  obligation on this path: the `BufferSnapshot` fold-mirror clear.
- **The bottom-panel arc** — Stage 1 (core + TUI) is implementing now;
  its **Stage 2 owns both** this render path (a projected panel cell grid
  painted as a band) **and the next protocol version**.

Neither blocks the design below, and this framing reserves no protocol
version (see §Protocol surface). But Tier 4's anchors are the ones most
likely to move, so whichever of the three lands second re-scouts against
the first — the same rule the bottom-panel and folding framings already
apply to each other.

```
Backgrounds → Squiggles → Code Text → Math → Gutter → Caret → Minimap → Minibuffer → Completion → Context menu
```

**Inline math (`$...$`):**
1. `rebuild_code_slice()` detects `MathSpan`s in the visible range.
2. For each inline span, extract the source text, hash-lookup the math
   cache, parse+layout on miss.
3. Delete the raw `$...$` glyphs from the cosmic-text buffer. Insert a
   zero-width placeholder with explicit width = `MathBox.width`.
4. Collect all `PosGlyph` runs into a `Vec<MathDraw>` list, offset by
   the line's baseline position.
5. In `render()`, after the code text draw call, iterate `MathDraw`s
   and issue glyphon `TextArea` calls for each glyph run at its
   computed absolute position.

**Display math (`$$...$$`):**
1. The math source occupies full lines. Detect via the text layout that
   the `MathSpan` spans entire visual lines.
2. Replace the affected visual lines with a single spacer glyph.
3. Insert vertical space before and after (from `\abovedisplayskip` and
   `\belowdisplayskip` equivalents — hardcoded constants are fine for
   v0).
4. Render the math block centered horizontally at the spacer position.

**Stretchy delimiters** (the hardest rendering case): the MATH table's
`GlyphConstruction` entries describe how to assemble a vertically
stretched glyph from top/middle/bottom/extender pieces. The render pass
for a stretchy glyph draws 3–5 separate glyph IDs at computed
positions, bottom-to-top. This is analogous to the existing squiggle
shader (a custom WGSL path for diagnostic underlines); a stretchy-glyph
shader or vertex-buffer assembly follows the same pattern.

## Protocol surface

No new wire types for v0. The GPU frontend detects math locally from
the text content it already receives via `BufferSnapshot`. The semantic
protocol stays unchanged; math rendering is a pure frontend
responsibility in v0.

Note: this means frontend-local detection duplicates regex scanning for
every reshape. In the common case (a handful of visible math spans) the
cost is negligible; for a full-screen display of a LaTeX document with
hundreds of inline expressions the scan cost accumulates linearly with
visible byte count. The hash cache absorbs re-parse of unchanged
expressions, but the delimiter scan itself is unavoidable. If this
proves hot, v1 moves detection to the instance side.

If this proves out, v1 would add an optional `MathSpans` variant to
`InstanceMessage` so the instance's tree-sitter detection (tier 1) is
authoritative and the frontend does not reimplement detection. This is
deferred until there is a frontend consumer to validate the wire shape.
As a secondary benefit, a single instance-side scan serves all connected
frontends — the v0 approach pays the scan cost per frontend.

## Integration points

| Component | Change | Risk |
|-----------|--------|------|
| `src/math_parse.rs` (new) | ~500-line recursive-descent parser | Low; pure fn, no deps. **Ships with Tier 3, not alone (Q#LX5).** |
| `src/math_layout.rs` (new) | MATH-table loading + box layout | Medium; depends on `ttf-parser` |
| `pmacs-gpu/Cargo.toml` | Declare `ttf-parser` as `default-features = false, features = ["opentype-layout"]` (already transitive via `fontdb`); bundle Latin Modern Math | Low; no new crate enters the graph, ~200 KB font. Spelling the features matters — bare `ttf-parser = "0.25"` unions `std` in and rebuilds the font chain |
| `pmacs-gpu/src/main.rs` | Detect math spans in visible text, insert MathBox draw calls | **High, and contended** — see C7: folding Stage 3 (GPU) is unframed and the bottom-panel arc's Stage 2 claims this same render path |
| `builtin/queries/latex/highlights.scm` | Add a `@math` capture beside the existing `math_environment` / `math_delimiter` captures | Low; the overlay already exists |
| `semantic_render.rs` | No changes (v0) | None by design |

The row that changed most is `pmacs-gpu/src/main.rs`. Revision 1 rated it
"Medium; touches main render path" when that path was uncontested. It now
has two other arcs converging on it, so Tier 4 cannot be scheduled
without knowing which of them lands first.

## Open questions

### Q#IM1 — Font size and DPI scaling

Math glyph positioning includes the font size as an explicit parameter.
Inline math should match the base text font size; display math may use
a slightly larger size. How does the math layout engine receive the
current font size? Via the existing `FontFacts` protocol message
(`protocol v17`), or queried from the `State` fields directly?

**Proposed:** query `self.font_size` directly in the GPU frontend, same
as code text does. No protocol change. (Protocol is **v20** at revision 2
— `SUPPORTED=[6..=20]`; v19 added the vterm terminal family and v20 the
GPU initial-target bootstrap. The decision is version-independent: it
holds precisely because it adds no wire surface. C3.)

### Q#IM2 — Color inheritance

Math glyphs should render in the foreground face color of the
surrounding text, including themed faces. At minimum: math inside a
string literal should inherit the string face color; math in comments
should inherit the comment color. This implies the detection layer
must cross-reference style spans at the boundary byte ranges.

**Proposed (v0):** render all math in the default foreground face.
Color-by-context is deferred to v1.

Caveat: in markdown buffers, math inside a fenced code block should
render differently from math in prose. Since v0 defaults to foreground
color everywhere, math inside a ` ```rust ` block will be
indistinguishable from inline prose math. This is acceptable for v0 but
should be the first upgrade in v1.

### Q#IM3 — Copy-paste fidelity

If a user copies a visual region containing a rendered `\int_a^b`,
what goes on the clipboard?

**Proposed:** the raw LaTeX source from the rope. This matches the
"math is text" invariant. A future refinement could copy the Unicode
math representation (e.g., `∫ₐᵇ`) when that is available, but that is
a specialization for the clipboard protocol, not the rendering path.

### Q#IM4 — Error fallback

What does an unparseable expression (unbalanced braces, unknown
command) look like?

**Proposed:** render the raw LaTeX source with a red wavy underline
(reuses the existing diagnostic squiggle shader from
`docs/archive/framings/pmacs-gpu-wavy-squiggles-framing.md`). The expression is still
editable and copy-pasteable; the squiggle signals the parse failure
without losing the text.

### Q#IM5 — Delimiter pairing visibility

`$` and `$$` are invisible delimiters in the rendered view. How does
the user know where the math region begins and ends when the cursor
is inside it?

**Proposed:** when the cursor is inside a math span, draw a subtle
background highlight over the entire range (reuses the `Decorations`
mechanism — `MathFocus` decoration produced by the GPU frontend
locally). The `$` characters themselves are never hidden from the
underlying text buffer; they are merely suppressed in the glyph
pipeline. When the cursor approaches the boundary, the raw `$`
reappears.

### Q#IM6 — Cursor navigation inside a math expression

The cursor moves through the raw LaTeX source at the byte level, not
through the rendered glyphs. The user edits `\frac{a}{b}` and sees it
render; the cursor steps through `\`, `f`, `r`, `a`, `c`, `{`, `a`,
`}`, `{`, `b`, `}`. The visual cursor position is a best-effort
projection: position the cursor at the x-offset of the *rendered* glyph
that corresponds to the nearest byte offset in the source.

**Proposed:** no structural cursor changes for v0. Cursor rendering
uses the existing `CursorByte` mechanism, which operates on the raw
text. The visual cursor may appear at a fractional screen position
inside a rendered expression; this is equivalent to how it appears in
an LSP inlay hint (already deployed — inline adornments hide their
source text and the cursor skips over them). V1 could smooth this.

## Categorical bets

- **No JavaScript runtime.** KaTeX is the best existing math renderer,
  but embedding a JS runtime (deno_core, quickjs, or a full V8) to run
  it would be the most expensive dependency pmacs has ever taken.
  Native Rust parsing + layout is ~2,000 lines vs ~30 MB of JS +
  WASM + runtime. The math layout algorithm is Knuth's 1978 design,
  well-documented and bounded in scope. Write it.

- **No SVG or pre-rasterization.** Every expression renders fresh from
  the MATH table on every frame. This avoids a cache-invalidation
  explosion (what happens to a cached SVG when the user changes the
  font size? the theme? the DPI?) and keeps the rendering path uniform
  with code text (same glyphon pipeline, same GPU atlas).

- **The TUI is out for v0.** Terminal cells cannot position glyphs at
  sub-cell resolution, cannot scale glyphs per-expression (except at
  great pain via Sixel / Kitty protocol), and cannot vary font size
  within a line. The TUI will render `$...$` as-is with a distinct
  face (e.g., italic + a highlight color). This is honest: the feature
  is GPU-only, like the squiggle shader.

  Note: the TUI still needs tier-1 detection (to know which `$...$`
  spans are math, so it can apply the distinct face). This makes
  detection a shared service — either the GPU frontend computes spans
  and the TUI queries them (impossible without a protocol message), or
  both frontends run their own detection. The v0 approach of
  frontend-local detection means the TUI must duplicate the delimiter
  scan. For v0 this is acceptable (the scan is cheap), but v1 should
  centralize detection in the instance and emit `MathSpans` on the wire.

- **v0 is read-and-edit, not write-assist.** No `\begin{align}`
  completion, no auto-closing `}`, no preview of incomplete
  expressions. Those are UI conveniences layered on top once the
  pipeline exists.

## Non-goals (explicitly excluded from this framing)

- Full LaTeX document rendering (titles, sections, bibliographies,
  cross-references). That is a separate mode (`latex-mode.lua`), not a
  frontend rendering concern.
- MathML input. Detection parses `$...$` / `\(...\)` syntax only.
  MathML→rendered would be a separate pipeline.
- Real-time preview of incomplete expressions. A partially typed
  `\frac{` will fail to parse and render as red-squiggled source; it
  does not show a partial fraction line.
- Equation numbering and `\ref` / `\label`. That is LaTeX-mode
  substrate, not math rendering.

## Acceptance

All acceptance is GPU-side (the TUI renders raw `$...$` and is not
separately tested for math). Tests run against `pmacs-gpu` with a
Vulkan device (`PMACS_REQUIRE_GPU=1`). Scratch buffers should clear
`pmacs.lsp.config` to avoid spurious server starts unless the test
exercises the LSP path.

1. **Inline math renders**: a buffer containing `The sum $\sum_{i=1}^n i$`
   opens; the `$...$` range is suppressed in the glyph stream; a glyph
   run for `∑`, baseline-shifted `i=1` and `n`, and the summation sign
   appears at the correct position between "The sum " and the following
   text. Visual assertion via GPU snapshot or caret/x-extent comparison.

2. **Display math centers**: a buffer containing `$$\int_a^b f(x)\,dx$$`
   opens; the math block occupies its own visual lines, centered
   horizontally, with vertical spacing above and below.

3. **Cache hit on re-scroll**: scroll an expression off-screen and back;
   the second render takes < 1 µs (cache fetch). Measured via the GPU
   frontend's per-frame timing (already emitted to stderr).

4. **Cache eviction on edit**: edit inside a `$...$` range; the
   expression re-parses (cache miss). Edit outside any math range; all
   cache entries survive.

5. **Error fallback renders as red-squiggled source**: a buffer
   containing `$\frac{a$` (unbalanced brace) opens; the raw source text
   `\frac{a$` is visible with a red wavy underline (reuses the
   `SquiggleRenderer` pipeline).

6. **No false positive on bare `$`**: a buffer containing `Price: $5.00`
   (single `$` with no pair) opens; no math spans are detected. The text
   renders as ordinary code text.

7. **Display math stores raw source for copy**: select a visual region
   containing a rendered `$$\sum_{i=1}^\infty a_i$$`; the clipboard
   receives the raw LaTeX source `$$\sum_{i=1}^\infty a_i$$`, not
   rendered glyphs.

8. **Cursor moves through raw source**: cursor-right through
   `$\alpha$` steps `$`, `\`, `a`, `l`, `p`, `h`, `a`, `$` (eight
   cursor positions) even though the visual rendering shows a single
   `α` glyph.

9. **Stretchy fences assemble from glyph parts**: a buffer containing
   `$\left(\frac{a}{b}\right)$` renders the parentheses at least as
   tall as the fraction. Visual assertion (the parens enclose the
   fraction without clipping).

10. **Math survives buffer switch**: open A (with math) and B (plain
    text), switch back; A's math cache is intact and no re-parse occurs.

## Adjacent prior art in pmacs

- **Multi-language injections** (`docs/archive/framings/multi-language-injections-framing.md`):
  The `ParseTreeBundle` + `Layer` machinery already supports child
  parse trees at injected ranges. Math detection as a tree-sitter
  injection is a direct extension of this design, not a new mechanism.

- **InlineAdornments** (M11.3, `docs/semantic-frontend-protocol.md`):
  LSP inlay hints demonstrate that the frontend can interleave virtual
  text at a byte offset without occupying document bytes. Math
  expressions need something strictly stronger — suppressing the raw
  `$...$` glyphs and replacing them with positioned math glyphs at
  potentially different widths. This suppression+replacement mechanism
  does not exist today (InlineAdornments are additive only — `ChunkSource::Source`
  and `ChunkSource::Adornment` are interleaved, not exclusive). The math
  pipeline would build it, roughly following the same anchor/offset
  pattern but with a `ByteRange` to suppress and a `MathBox` to render
  in its place.

- **GPU squiggle shader** (`docs/archive/framings/pmacs-gpu-wavy-squiggles-framing.md`):
  Custom WGSL shader for diagnostic underlines demonstrates that
  pmacs-gpu already extends its rendering pipeline beyond basic glyph
  drawing. A stretchy-delimiter assembly pass or math-background pass
  follows the same architectural pattern (vertex buffer → dedicated
  shader → draw call in z-order).

- **Theme-faces** (Themes Arc 4, `docs/archive/framings/theme-faces-framing.md`):
  The face resolution chain (`face-attribute → color → fallback`)
  would extend naturally to a `math-face` or `math-display-face` for
  themed math colors.

## Sibling lane (added in revision 2)

`docs/archive/framings/latex-grammar-math-substrate-framing.md` (rev 3) names this note its
parent and carves out the frontend-agnostic LaTeX substrate that can land
without touching a contended file. Revision 1 did not reference it, which
left the reader of this note unaware that part of its Tier 1 had already
shipped (#144) and that its Tier 2 staging had already been decided.

Read that lane before scheduling any tier here:

- its **Q#LX5** governs Tier 2 (parser lands beside Tier 3, not ahead);
- its landed portion supplies the `.tex` grammar and the query-overlay
  precedent Tier 1 depends on;
- it explicitly defers Tier 3 layout, Tier 4 GPU render, and
  instance-side `(math_environment) @math` injection detection back to
  this arc.
