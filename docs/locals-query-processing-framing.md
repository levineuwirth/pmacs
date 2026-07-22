# Locals-query processing - syntax-highlight completion

**Status:** Revision 1, implementation active by user direction, 2026-07-22.

**Base:** `githubsucks/main` at `8bd8298` (mode system #129, Vterm Stage 2
#130, modeline detection #132, and landed-state handoff #133). Protocol remains
v18.

## Problem

pmacs runs each grammar's `highlights.scm` directly through a
`tree_sitter::QueryCursor`. That cursor evaluates text predicates such as
`#eq?`, `#match?`, and `#any-of?`, but it does not assign or evaluate semantic
properties. In particular, JavaScript's bundled highlight query contains:

```scheme
((identifier) @variable.builtin
 (#match? @variable.builtin "^(arguments|module|console|window|document)$")
 (#is-not? local))

((identifier) @function.builtin
 (#eq? @function.builtin "require")
 (#is-not? local))
```

A correct highlighter must first run the grammar's `LOCALS_QUERY`, resolve
lexical definitions and references, and then apply `#is? local` /
`#is-not? local` while selecting highlight captures. Without those facts,
a local `console` or `require` can be styled as a builtin.

The current implementation fails closed by dropping every highlight pattern
that carries a `local` property predicate. That prevents the false-positive
shadowed-builtin style, but it also drops legitimate builtin highlighting for
non-shadowed `console`, `window`, `require`, and peers. This is the first
remaining item in the side-quest north-star list at
`docs/side-quest-backlog.md:244-249`.

## Goal

Run the bundled locals query as part of each settled syntax layer, retain a
compact lexical-local map, and use that map to evaluate `#is? local` and
`#is-not? local` in both syntax-highlight producers:

1. `highlight::SyntaxHighlightView` (TUI / overlay path), and
2. `semantic_render::scoped_style_spans` (semantic/GPU path).

A shadowed builtin must retain its ordinary fallback capture but lose the
builtin refinement. A non-shadowed builtin must regain the builtin capture.
The result must stay correct for nested scopes, TypeScript's inherited locals
query, injected-language layers, viewport-limited rendering, and edits that
settle a new parse bundle.

## Scope

### In

- Add bundled `locals.scm` fragments to syntax grammar metadata.
- Compose inherited locals fragments base-first, exactly as highlight fragments
  already compose.
- Implement Tree-sitter's local-scope conventions:
  - `@local.scope`
  - `@local.definition`
  - `@local.definition-value`
  - `@local.reference`
  - `#set! local.scope-inherits false`
- Evaluate positive and negative `local` property predicates, including the
  optional capture-qualified form accepted by Tree-sitter's query parser.
- Cache local facts on each settled parse layer.
- Feed those facts to both highlight producers.
- Cover lexical behavior and end-to-end rendering under both supported Lua
  backends.

### Out

- No new grammar, parser, or language-detection behavior.
- No Lua API, command, config key, theme key, protocol field, or frontend-only
  state.
- No LSP semantic-token changes.
- No user-defined query registration surface.
- No cross-language name resolution between a host layer and an injected
  child layer.
- No Tree-sitter-highlight `reference_highlight` propagation from a local
  definition's syntactic capture to all references. The target is property
  predicate correctness. Adding propagation would change ordinary variable
  styling beyond the reported builtin bug and needs separate framing.
- No general engine for arbitrary `#is?` property keys. This side quest owns
  the Tree-sitter-defined boolean `local` property only; other property
  predicates retain their current behavior.

## Existing contracts to preserve

1. **Layering:** every injection `Layer` owns its grammar tree and highlight
   query. Deeper layers and later same-depth siblings retain their existing
   precedence.
2. **Viewport work:** the semantic producer's highlight capture walk remains
   restricted with `QueryCursor::set_byte_range`; rendering a frame must not
   launch a whole-file locals walk.
3. **Settle freshness:** a new parse bundle replaces the old bundle
   atomically. Local facts must travel with the same bundle, never in a side
   cache that can pair old scopes with a new tree.
4. **Incremental edits:** a completed reparse produces new local facts before
   the bundle becomes visible. Until settle, producers continue to use the
   previous internally consistent bundle.
5. **Query inheritance:** fragments are newline-joined base-first. Bare
   concatenation can extend a trailing Scheme comment and is forbidden.
6. **Fallback styling:** suppressing a builtin refinement must not suppress
   an independent ordinary-variable capture for the same identifier.
7. **No protocol change:** all work is internal render state.

## Decisions

### Q#LQ1 - Grammar metadata carries locals fragments

`LanguageEntry` gains `locals_query: &'static [&'static str]`, parallel to
`highlights_query` and `injections_query`.

The bundled crates currently exposing `LOCALS_QUERY` are wired as follows:

| pmacs language | effective locals fragments |
| --- | --- |
| `lua` | `tree_sitter_lua::LOCALS_QUERY` |
| `javascript` | `tree_sitter_javascript::LOCALS_QUERY` |
| `javascriptreact` | `tree_sitter_javascript::LOCALS_QUERY` |
| `typescript` | JavaScript locals, then TypeScript locals |
| `typescriptreact` | JavaScript locals, then TypeScript locals |

All other entries use an empty slice. TypeScript's locals query is a small
parameter-definition delta, so compiling it alone would omit JavaScript's
scopes, declarations, and references. JSX uses the JavaScript grammar and
therefore the JavaScript locals query. TSX uses the TypeScript crate's TSX
language with the same base-plus-delta locals composition.

`SyntaxRegistry` lazily compiles and caches one effective locals query per
language, including cached failure/no-query results, matching the existing
highlight-query policy.

### Q#LQ2 - Local facts follow Tree-sitter lexical semantics

A locals capture walk maintains a stack of scopes. The stack begins with one
non-inheriting root scope covering the layer. Captures are processed in query
order and source order:

1. `@local.scope` pushes a scope covering that node. It inherits outer
   definitions unless its pattern sets `local.scope-inherits` to `false`.
2. `@local.definition` records the identifier in the innermost scope and marks
   that identifier range local.
3. A sibling `@local.definition-value` capture records the initializer/value
   range. That new definition is not visible while resolving references inside
   its value, so an outer definition with the same name can still win there.
4. `@local.reference` searches definitions newest-first, then scopes
   innermost-first. Search stops at a non-inheriting scope. A resolved
   reference range is marked local; an unresolved reference remains non-local.
5. Scopes whose end precedes the next capture are popped.

Definition names are compared as borrowed source byte slices. No identifier
strings are allocated. Invalid or out-of-bounds ranges do not produce local
facts.

The result is an opaque `LocalFacts` value containing sorted, deduplicated
`(start_byte, end_byte)` ranges. Highlight predicate checks use binary search;
they do not hash, copy source text, or rebuild scope state.

### Q#LQ3 - Predicate evaluation is per emitted capture

Before emitting a highlight capture, inspect
`Query::property_predicates(pattern_index)`:

- `#is? local` passes only when the selected node is in `LocalFacts`.
- `#is-not? local` passes only when the selected node is not in `LocalFacts`.
- If the property names a capture, test that capture's node.
- If it does not name a capture, test the highlight capture currently being
  considered, matching Tree-sitter-highlight's per-node behavior.
- Multiple `local` predicates on one pattern are conjunctive.
- A missing or failed locals query yields no local ranges: negative predicates
  pass and positive predicates fail. This preserves useful non-local
  highlighting without inventing local classifications.
- Predicates with keys other than `local` remain ignored, preserving the
  current query engine's scope.

Text predicates remain the query cursor's responsibility. Property settings
such as `local.scope-inherits` are read only by local analysis; they are not
mistaken for highlight predicates.

### Q#LQ4 - Facts are computed once at settle and owned by the layer

`SyntaxRegistry::resolve_layer_queries` remains the main-thread Stage 2 handoff.
For each raw worker layer it:

1. resolves the cached highlight query;
2. checks whether that query contains any `local` property predicate;
3. only when needed, resolves the cached locals query and walks the layer tree;
4. stores `Arc<LocalFacts>` beside the layer's tree and highlight query.

This is a single whole-layer analysis per completed parse, not per render.
Languages whose highlights never ask about `local` pay only the cheap predicate
scan and carry no facts. This includes Lua today even though its locals query
is correctly registered for future local-sensitive highlight patterns.

Putting facts on `Layer` makes the consistency invariant structural:

```text
settled Layer = tree + grammar + highlight query + local facts
```

A producer cannot accidentally retrieve facts for another buffer revision.
Child injection layers compute facts against their own tree and grammar; local
bindings never cross layer boundaries. A deliberately combined injection
layer shares one tree and therefore one lexical environment, matching its
combined parse semantics.

### Q#LQ5 - Both producers share one capture-selection function

`compute_highlight_spans_for` accepts the layer's optional local facts and
owns predicate evaluation. The whole-buffer overlay path and the
viewport-limited semantic path both call this function. There is no second
predicate implementation in a frontend.

`compute_highlight_spans_in_range` passes the root layer's facts. Injection
producers pass each layer's facts. Existing wider-first ordering and
cross-layer merge precedence remain unchanged after captures are selected.

### Q#LQ6 - Performance boundary

Local analysis is linear in the locals-query captures for one settled layer.
It runs only for a language whose highlight query actually contains a `local`
predicate, and only once when a fresh parse bundle settles.

The render hot paths remain:

- TUI: cached whole-layer highlight spans, rebuilt only when the bundle pointer
  changes;
- semantic/GPU: viewport-bounded highlight query plus binary-search local
  checks.

No frame performs a whole-file locals query. Memory is two `u32` offsets per
local definition/resolved reference plus vector capacity; ranges are sorted
and deduplicated before storage.

Incremental locals invalidation is intentionally bundle-granular. A local edit
can alter all later name resolution in a scope, so attempting to splice only
changed ranges without a scope dependency graph risks stale classifications.
The parse itself remains incremental; this bounded lexical pass is the boring,
correct cutover.

## Data flow

```text
worker parse
  -> raw ParseTreeBundle { Layer { tree, language, no queries/facts } }
  -> main-thread resolve_layer_queries
       -> cached highlights query
       -> cached locals query (only if highlights uses local predicates)
       -> lexical capture walk -> sorted LocalFacts
  -> settled ParseTreeBundle
       -> TUI SyntaxHighlightView cache rebuild
       -> semantic/GPU viewport capture walk
            -> shared local predicate filter
            -> existing span ordering/merge/theme lookup
```

## Acceptance criteria

1. **Non-shadowed builtin restored:** JavaScript `console`, `window`, or
   `require` with no matching lexical definition emits its bundled
   `*.builtin` capture.
2. **Shadowed builtin suppressed:** a parameter or local declaration named
   `console`/`require` and references resolved to it do not emit a builtin
   capture; their ordinary variable captures remain.
3. **Scope correctness:** shadowing is confined to its lexical scope. A builtin
   before/after the scope remains builtin, while the definition and references
   inside are local.
4. **Positive predicate:** a focused custom highlight query using `#is? local`
   emits resolved definitions/references and rejects an unresolved identifier.
5. **TypeScript inheritance:** parameter shadowing in TypeScript and TSX uses
   the JavaScript base locals query plus the TypeScript delta; query compilation
   and classification succeed for both grammars.
6. **End-to-end render:** opening a JavaScript buffer through the shipped
   runtime and applying a theme that styles only `variable.builtin` renders an
   unshadowed builtin with that style and a shadowed occurrence without it.
7. **Edit freshness:** after changing a shadowing identifier and settling the
   new parse, the next render reflects the new local/non-local classification;
   no stale local facts survive.
8. **Producer coverage:** both the overlay and semantic/GPU callsites pass the
   corresponding layer facts to the shared capture walk; injected layers keep
   their own facts.
9. **Backend parity:** the focused acceptance test passes under default Luau
   and `--no-default-features --features lua54`.
10. **No regressions:** formatting, Clippy, default library tests, CRDT library
    tests, M4 acceptance (excluding the machine-broken basedpyright case),
    required GPU tests, workspace sweep, and `git diff --check` pass.

## Expected files

- `src/syntax.rs` - grammar metadata, locals-query cache, lexical analysis,
  layer facts, predicate selection, unit coverage.
- `src/highlight.rs` - pass per-layer facts to the overlay producer.
- `src/semantic_render.rs` - pass per-layer facts to the viewport producer.
- `tests/m4_acceptance.rs` - end-to-end local/non-local rendering and edit
  freshness.
- `docs/agent-handoff.md` and `docs/active-work.md` - updated only after the
  implementation is proven and published according to their protocols.

No other runtime, Lua, frontend, protocol, or theme file should need a behavior
change.
