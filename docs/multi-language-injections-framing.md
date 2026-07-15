# Multi-language injections — framing (side quest, highlight family)

**Intent.** Teach the syntax engine that one buffer can hold more than
one language. Today pmacs parses a buffer with exactly one grammar and
runs exactly one `highlights.scm` over it ("first language wins"). This
adds tree-sitter *injection layers*: after the root parse, run the
grammar's `injections.scm` to find embedded regions (a markdown code
fence, an HTML `<script>`, a rust `macro!` body), parse each with the
injected language's grammar, and merge every layer's highlight spans.
It is the load-bearing enabler the side-quest backlog names: it unlocks
markdown fenced code, embedded languages, and is the honest gate on the
per-cell notebook path.

The first *consumer* that ships with the engine is **markdown fenced
code blocks plus inline markdown**, chosen because it needs **zero new
grammars**: the markdown block grammar is already bundled and already
ships an injection query, and every language a fence can name (rust,
python, bash, js, ts, go, toml, …) already has a grammar from the
grammar-gap PR (#118). The engine is grammar-agnostic; other injection
sites (HTML embedding, JS template literals, comment-embedded langs)
become follow-ups as their grammars land.

---

## Ground truth (as of `main` @ `0ba01fe`, #119)

The syntax stack assumes **one tree, one language, one query per
buffer** end to end. Every layer of that assumption has to grow.

- **`ParseView` / `ParseTreeBundle`** (`src/syntax.rs`) — the per-buffer
  parse state holds a single `language`, source mirror, pending-`InputEdit`
  list, and `current: Option<Arc<ParseTreeBundle>>`. `ParseTreeBundle`
  holds one `tree`, one `source`, one `language_name`. `run_parse`
  (`syntax.rs:106`) sets one language and returns one tree.
- **Highlight query** — `SyntaxRegistry::highlights_query(name)`
  (`syntax.rs:655`) lazily compiles and caches **one** `highlights.scm`
  per language into the main-thread `Rc<SyntaxRegistry>`. `LanguageEntry`
  (`syntax.rs:318`) carries `highlights_query: &'static [&'static str]`
  but **no injection query**.
- **Two style producers, both single-tree:**
  - Grid/TUI: `SyntaxHighlightView` (`src/highlight.rs:289`) is
    constructed with **one** `Arc<Query>` and caches spans for the one
    `parse.current()` bundle, invalidated by `Arc::ptr_eq`.
  - Daemon→GPU wire: `scoped_style_spans` (`src/semantic_render.rs:1640`)
    reads the one `bundle`, the one `highlights_query(language_name)`,
    runs `compute_highlight_spans_in_range` scoped to the viewport, maps
    captures→theme→`StyleSpan`. Perf-gated by `StyleGate`
    (`semantic_render.rs:1612`) keyed on the bundle `Arc`.
- **The wire path re-sorts spans by start — producer order is NOT
  preserved.** The GPU applies `StyleSpans` through
  `replace_style_spans` (full, `pmacs-gpu/src/main.rs:4112` — collects
  segments then `sort_by_key(range.start)`) and `merge_style_spans`
  (incremental, `main.rs:4130` — clips, appends, re-sorts). So any
  "emit root-then-depth order and rely on it downstream" scheme is dead
  on arrival; overlaps must be resolved **before** the wire (Q#IJ6).
- **The two `StyleSpan` consumers also disagree on overlap:**
  `SemanticModel::effective_style_at` (`src/semantic_client.rs:334`)
  folds **every** covering span via `merge_styles`; the GPU
  `source_color_at` (`pmacs-gpu/src/main.rs:6718`) returns the **first**
  covering span's fg and stops. Both are addressed by Q#IJ6.
- **Policy A** — grammar-backed buffer ⇒ styled solely by tree-sitter;
  else solely by LSP tokens; never both on the wire
  (`semantic_render.rs:1641`).
- **Dispatch/settle** (`builtin/runtime/syntax.lua`) — one language per
  buffer, pinned at first attach; the `_dispatch` wrapper seam is
  `syntax.lua:26`. The async tick installs the settled bundle.
- **Worker discipline** — `ParseRequest` is fully owned (`Send`); parsing
  runs on a worker. The registry (`Rc`) and any Lua table are main-thread
  only — neither highlight-query resolution nor Lua-set config can happen
  on the worker (Q#IJ2, Q#IJ4).

**Confirmed tree-sitter mechanics this design rests on** (tree-sitter
0.26, verified against vendored sources + the tree-sitter injection
docs):

1. `Parser::set_included_ranges(&[Range])` restricts a parse to given
   byte ranges of the **full source**; resulting node offsets stay
   **absolute** into the full buffer — injected spans are already in
   buffer coordinates. Ranges must be **sorted, non-overlapping,
   non-empty** or the call returns `IncludedRangesError`.
2. `tree_sitter_md::INJECTION_QUERY_BLOCK` exists on the already-bundled
   crate, using **both** forms: dynamic `(info_string (language)
   @injection.language)` + `(code_fence_content) @injection.content`, and
   static `((inline) @injection.content (#set! injection.language
   "markdown_inline"))` (also `html`/`yaml`/`toml`).
3. **The injection contract excludes child ranges and intersects with the
   parent.** Unless `#set! injection.include-children` is set, the injected
   ranges are the content node's extent **minus its NAMED children's
   ranges**, then **intersected with the parent layer's own included
   ranges** so a nested injection cannot reintroduce bytes its parent
   excluded. *(Implementation note, round 1: excluding ALL children — not
   just named — shreds a markdown block `inline` node, whose children are
   anonymous text tokens, into unparseable fragments. Excluding only named
   children matches `tree-sitter-md`'s own inline splitter,
   `parser.rs:406-425`, and is correct for every real injection site.)*
   This makes `markdown_inline` a genuine multi-range case (Q#IJ5).
4. `LanguageEntry.loader` (`fn() -> Language`) and query-source `&'static
   str` consts are `Send` and touch no grammar C-object until called — a
   worker resolves injected languages **lazily** by indexing `&'static
   BUILTIN_LANGUAGES`, preserving the M4.2 lazy-load invariant.
5. Injection-query availability is per-crate and inconsistently named
   (rust `INJECTIONS_QUERY`; markdown `INJECTION_QUERY_BLOCK`; bash none)
   — the same pattern `highlights_query` already absorbs as a `&'static
   [&'static str]` slice.

---

## Decisions

### Q#IJ1 — Two node types: worker `RawLayer`, settled `Layer`; the layer set lives in `ParseTreeBundle`

```
RawLayer { language_name: String, tree: Tree, depth: u16 }          // worker out
Layer    { language_name: String, tree: Tree, depth: u16,           // settled
           highlight_query: Option<Arc<Query>> }
```

`ParseTreeBundle` grows to an ordered `Vec<Layer>` (root = layer 0,
whole buffer; children depth-ascending). One `Arc<ParseTreeBundle>` is
installed **atomically** per settle, so the `StyleGate` `Arc::ptr_eq`
gate and the `SyntaxHighlightView` cache stay **unchanged** — a layered
reparse mints a fresh Arc and flips both gates. `bundle.language_name`
stays (root label). Per-layer `included_ranges: Vec<Range>` (Q#IJ5) are
the worker's parse *input*, not retained on the settled `Layer` (styling
reads the tree; offsets are absolute).

### Q#IJ2 — Two-stage handoff: worker builds trees, settle resolves queries

- **Stage 1 — worker (`run_parse_layered`)** builds the whole layer
  *structure*: root tree (incremental) + every child tree, via injection
  queries and `set_included_ranges`. It resolves injected languages by
  indexing `&'static BUILTIN_LANGUAGES` (loaders + a new `injections_query:
  &'static [&'static str]` field on `LanguageEntry`, mirroring
  `highlights_query`), loading/compiling only what a file injects.
  Touches no highlight query, no theme. Output `Vec<RawLayer>`.
- **Stage 2 — settle/install (main thread)** resolves each raw layer's
  `highlights_query(name)` from the registry cache, builds `Vec<Layer>`,
  wraps one `ParseTreeBundle`, installs the `Arc` **atomically**.

Highlight-query compilation stays main-thread/cached/shared with the
producers; tree parsing stays on the worker. The only dynamic worker
input beyond the static table is the alias snapshot (Q#IJ4), carried in
`ParseRequest`.

*Rejected:* (a) layer-build on the main thread in settle — moves child
parsing onto the frame path; (b) a `Send + Sync` query store so the
worker resolves everything — larger registry blast radius, deferred as
an option if Stage 2 bottlenecks.

**Limitation named:** injection targets resolve only against
`BUILTIN_LANGUAGES`; runtime/Lua-registered languages are not injectable
in v1. Every headline case is bundled.

### Q#IJ3 — Bounded recursion: depth cap, generous layer backstop, visited guard, child-only failure

- **max depth** (default 3),
- **max total layer count** — a *runaway backstop* set well above any
  real document (default **4096**), decoupled from performance; the real
  perf bound is the Q#IJ10 settle-time guard. Hitting it is **surfaced, not
  silent**: `run_parse` sets a `ParseTreeBundle::injection_capped` flag,
  and `syntax.lua`'s settle tick raises it once per buffer via
  `pmacs.error` (`_injection_capped`). A modest markdown doc's inline
  layers (one per paragraph/heading) sit far under the backstop. A real
  boundary test drives just over 4096 fences and asserts the flag + capped
  count + intact root.
- a **visited set on `(language_name, ranges)`** so a same-language
  self-injection over a non-shrinking region can't reproduce itself.

**Failure is isolated to the child:** unknown/unresolvable language, cap
hit, or child parse error drops **that child layer only** — never the
root, never a sibling. The root always installs.

### Q#IJ4 — Dynamic fence names: registry-held alias map, case-folded, snapshot to the worker

Raw `@injection.language` text (`JS`, `ts`, `sh`, `py`, `Rust`, `c++`,
`jsx`, `tsx`) won't exact-match bundled names — the job tree-sitter's
per-language injection-regex does. pmacs does it with a **case-folded
alias table**: lowercase the text, look up an alias table before the
bundled-name table (`js`→javascript, `jsx`→javascriptreact,
`ts`→typescript, `tsx`→typescriptreact, `py`→python, `rs`→rust,
`sh`/`shell`→bash, `c++`/`cxx`→cpp, `yml`→yaml, …). Unresolved → region
skipped (no error, root intact).

**Worker-safe extensibility:** the map lives in the registry (static
defaults + Lua-driven overrides via a Rust setter that
`pmacs.parse.injection_aliases` writes through). Because the worker can't
read Lua or the `Rc` registry, each `_dispatch` (the `syntax.lua:26`
seam) **snapshots the merged map into an `Arc<HashMap<String,String>>`
carried in `ParseRequest`**. Acceptance mutates the alias set from Lua,
then runs an **asynchronous** injected parse and asserts the new alias
resolves — proving the bridge, not just the static resolver.

### Q#IJ5 — Included ranges are `Vec<Range>` per layer: exclude children, intersect with parent

A layer's ranges are built from its `@injection.content` node(s):

- `#set! injection.include-children` → `[node.range]`;
- otherwise (default; the markdown-inline case) → the node's extent
  **minus its NAMED children's ranges** (anonymous token children are the
  injected text itself and are kept — mechanic #3), **then intersected
  with the parent layer's included ranges**. Ranges come out ordered and
  non-overlapping; empty ranges dropped.

Fenced code (`code_fence_content`, no children) → one range; a **multi-line**
container (a blockquote/list whose inline node carries a named
`block_continuation` child) → several. Core, not deferred —
`markdown_inline` needs it. Acceptance asserts `content_node_ranges`
returns **>1 range** for a multi-line blockquote and that both sides parse
and highlight (a one-line paragraph would give a single range and could
not falsify multi-range support).

### Q#IJ6 — The wire producer flattens layers into disjoint effective spans

Because the wire re-sorts by start (`main.rs:4117`/`4130`), producer
order cannot carry overlap precedence. So overlaps are resolved **in the
producer, before the wire**:

- **`scoped_style_spans` flattens** all layers over the viewport into
  **disjoint effective spans** — a sweep-line that, at each byte, takes
  the deepest layer covering it, and within a layer the existing
  wider-first "narrower overrides" rule. Output is disjoint runs of a
  single folded style. The viewport already bounds the sweep, so this is
  O(boundaries) in the visible range. Positional re-sorting downstream is
  then a no-op on precedence, and the disjoint output aligns with the
  model's existing style-tile disjointness invariant. (Per-byte effective
  style is unchanged from today for single-layer buffers; only the *shape*
  goes overlapping→disjoint, so existing `StyleSpans` shape assertions are
  updated to match.)
- **The GPU `source_color_at` fold fix is kept** (fold all covering spans
  in order, matching `effective_style_at`) — defense-in-depth and a
  contract alignment, correct even for any residual same-start overlap.
- The **grid** producer (`SyntaxHighlightView`) has no wire and no sort:
  it paints layer-by-layer in depth order (later overrides via the cell
  merge), which is already correct.

Acceptance test #9 drives the **full message-application path**
(`replace_style_spans` → render → `source_color_at`) with an overlapping
parent-red / child-green case, not a direct `source_color_at` call.

### Q#IJ7 — Both producers walk layers; Policy A unchanged at buffer scope

`scoped_style_spans` and `SyntaxHighlightView` iterate `bundle.layers`
using each `layer.highlight_query`, reusing viewport-scoped
`compute_highlight_spans_in_range` per layer. Policy A unchanged at the
buffer level (grammar-backed ⇒ tree-sitter across *all* layers; LSP-only
untouched; no new `LspStyleView` interaction). `SyntaxHighlightView`
drops its single-`query` constructor param and reads per-layer
`Arc<Query>` from the bundle, so it still holds only `Send` state — the
largest single code change.

### Q#IJ8 — Incrementality: root incremental, children cold each settle

Root keeps `InputEdit`-accumulation + `prior_tree`. Child layers rebuild
**cold** each settle (an edit can add/remove/resize regions, so child
identity isn't stable). **Deferred:** child-tree incrementality and
range-scoped rebuild. Named cost: one cold inline layer per markdown
paragraph/heading — made a measured acceptance guard by Q#IJ10, not a
hope.

### Q#IJ9 — `injection.combined` deferred

Each injection **match** is its own layer/parse (multi-range *within* a
match is Q#IJ5, in scope). Combined injections (many matches → one shared
parse; PHP-in-HTML, some comment schemes) are **deferred**. Markdown
fenced/inline are not combined.

### Q#IJ10 — First consumer: markdown fenced code **and** inline

Ships fenced-code **and** `markdown_inline` (zero new grammars; same
crate; block grammar already injects inline via `#set!`). Inline
exercises the static path and the multi-range path (Q#IJ5), completes
markdown, and retires the M9.7 "block-only" floor. Its cost is real (a
cold child parse per paragraph/heading), so a **many-paragraph
settle-time acceptance test** guards it **and asserts the final paragraph
receives a layer/capture** (not merely that parsing finished). While that
stays green, child incrementality (Q#IJ8) is not required in v1. HTML,
JS/TS template literals, and doc-comment code are follow-ups gated on
their grammars.

### Q#IJ11 — Perf gate & parse-in-flight stay correct for free

`StyleGate.bundle` is the root bundle Arc; a layered reparse installs a
fresh Arc atomically (single dispatch), flipping the gate with no
half-styled frame. `grammar_style_parse_not_ready` unchanged.

---

## Bets

1. Absolute node offsets under `set_included_ranges` make layer spans
   buffer-coordinate-native (mechanic #1).
2. Static-table worker + two-stage settle + alias snapshot (Q#IJ2/IJ4)
   preserves lazy loading, keeps parsing off the main thread, and keeps
   query caching where it is.
3. Producer-side flattening (Q#IJ6) is the one definite overlap strategy
   given the wire re-sort; the GPU fold fix rides along.
4. Cold child reparse is fast enough at real paragraph/fence counts; the
   Q#IJ10 guard is the measured backstop, Q#IJ8 the escape hatch.

## Deferred (named)

- `injection.combined` (Q#IJ9).
- Child-tree incrementality + range-scoped rebuild (Q#IJ8), gated open by
  the Q#IJ10 perf guard.
- Injectable runtime/Lua-registered languages (Q#IJ2).
- HTML/CSS/GraphQL/SQL grammars and their injection sites (Q#IJ10).
- Notebook per-cell layering (needs JSON grammar + this engine).
- `Send + Sync` query store so the worker resolves highlight queries
  directly (Q#IJ2 alternative b).

## Acceptance (bite-verified where it guards a real gap)

1. `injection_query_block_compiles` — `INJECTION_QUERY_BLOCK` compiles
   against the md grammar.
2. `layered_parse_builds_child_for_fenced_code` — a ` ```rust ` fence
   yields ≥2 layers; child language == rust, roots at `source_file`.
3. `child_layer_offsets_are_absolute` — a `fn` inside the fence has byte
   offsets matching its position in the **full** markdown source.
4. `dynamic_alias_resolves` — ` ```py `→python, ` ```JS `→javascript
   (case-folded); ` ```nonsense ` → no child, no error, root intact.
5. `lua_alias_override_resolves_on_async_parse` — mutate
   `injection_aliases` in Lua, then an **async** injected parse resolves
   the new alias (Q#IJ4 bridge). Plus `sync_parse_now_resolves_alias`
   (round 1): the same must hold on the **synchronous** `_parse_now` path.
6. `inline_layer_multi_range_excludes_block_continuation` — a **multi-line**
   blockquote's inline node carries a named `block_continuation`;
   `content_node_ranges` returns **>1 range** and both sides parse +
   highlight (Q#IJ5). (A one-line paragraph gives a single range and can't
   falsify multi-range.)
7. `recursion_bounds_terminate` — rust macro self-injection terminates
   within the depth bound; `injection_layer_cap_surfaces_and_preserves_root`
   drives >4096 fences and asserts the surfaced `injection_capped` flag,
   the bounded count, and an intact root; a failing child drops only itself
   (Q#IJ3).
8. `wire_producer_emits_disjoint_child_spans_in_fence` — `scoped_style_spans`
   over a ` ```rust ` fence emits disjoint `StyleSpan`s covering a rust
   keyword **inside** the fence. **Bite-verified** vs the single-layer
   producer. Plus `full_buffer_summary_scales_on_large_grammar_file`
   (round 1): the whole-buffer summary path stays ~linear under the event
   sweep (a quadratic flatten regresses it).
9. `source_color_folds_overlapping_child_over_parent` — parent-red /
   child-green overlap driven through the real `spans_from_segments`
   (`replace_style_spans` body): the child (green) wins the fold.
   **Bite-verified** vs the first-span-wins path (Q#IJ6).
10. `grid_paints_injected_child_keyword` — `SyntaxHighlightView` paints a
    rust-keyword cell inside the fence.
11. `incremental_edit_reflects_in_child_and_new_fence_adds_layer` — editing
    inside a fence shows in child spans after reparse; a **new** fence adds
    a layer (Q#IJ8).
12. `many_paragraph_settle_under_budget_with_tail_covered` — a large
    all-inline markdown buffer settles within a comfortable budget **and
    the final paragraph receives an inline layer** (Q#IJ3 cap + Q#IJ10
    guard).
13. `non_injecting_buffer_single_layer` — a plain `.rs` buffer still
    yields exactly one layer (regression guard).

## Risks / interactions

- **Perf** (Q#IJ8/IJ10) — cold child reparse per settle, one inline layer
  per paragraph; test 12 is the measured guard (with tail coverage),
  Q#IJ8 the release valve.
- **Wire overlap** (Q#IJ6) — the wire re-sorts by start, so flattening in
  the producer is mandatory; the GPU fold fix rides along. Tests 8/9 pin
  both.
- **Single-layer wire shape** — flattening turns overlapping spans
  disjoint for *all* grammar buffers; per-byte style is unchanged, but
  existing `StyleSpans` shape assertions are updated.
- **`set_included_ranges` contract** — sorted, non-overlapping, non-empty;
  the Q#IJ5 exclusion+intersection yields this by construction, but guard
  empty inline nodes and non-UTF-8 info strings.
- **M9.7 prompt-result markdown buffers** now get fenced + inline
  highlighting (a bonus, retiring the block-only floor); verify the
  `_attach_highlight`-for-markdown path doesn't crash.
- **Themes main quest** — untouched. Highlight *structure*, not color;
  capture→style still flows through `Theme::lookup`. No protocol bump
  (`StyleSpan` wire shape unchanged).
