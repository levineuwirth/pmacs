# Web grammars (HTML + CSS) + HTML injections — framing

**Revision 3 — pre-implementation. Ground truth: canonical `main` @
`4daa1b8` (after LaTeX #144 and inline-math-docs #145), 2026-07-23. Status:
framing only; no implementation.** Rev 2 settled the capture set, injection
scope, and LSP claim (round 1); rev 3 corrects the `#match?` predicate analysis
(tree-sitter evaluates it natively) and refreshes the folding footprint to the
current branch (round 2). See §0.1.

## 0.1 Revision history

### Round 1 (rev 1 → rev 2)

- **F1 (high).** Q#WEB4 deferred the capture mapping. The upstream queries
  settle it: HTML and CSS need exactly **`tag`** and **`attribute`**; `@tag.error`
  prefix-walks to `tag`; there is **no** `@tag.delimiter` (rev-1 speculation).
  Q#WEB4 now names both mappings and their styles, and acceptance 6 paints an
  HTML **attribute** explicitly (a tag/property-only test could pass with
  `attribute` unverified since `property` is already recognized).
- **F2 (medium).** The HTML injections query has **exactly two** patterns —
  `script_element`→javascript, `style_element`→css. Q#WEB5 no longer claims
  event-handler / `style=` coverage; inline-attribute injection is now a named
  deferral.
- **F3 (medium).** No HTML/CSS language server is bundled: `lsp.lua`'s default
  config list ends at YAML (`:247`) and `:521` returns `nil` without a
  configured command. §2 and §5 corrected — the grammars give stable language
  IDs so a *user-supplied* config attaches automatically, but nothing ships or
  starts by default.

### Round 2 (rev 2 → rev 3)

- **R2-1 (medium).** The rev-2 claim that pmacs *ignores* `#match?` was wrong.
  `src/syntax.rs:1701` passes the source to `QueryCursor::captures`, so
  tree-sitter evaluates the standard `#match?`/`#eq?`/`#any-of?` text predicates
  natively (pmacs special-cases only `#is? local`, `:1703`). Corrected in §2 and
  Q#WEB4: an ordinary CSS property has a single `@property` capture; only a
  `--custom-property` also receives `@variable` (a benign double-capture).
  Acceptance 6 no longer claims an ordinary property pins custom-property
  precedence — it just verifies `color` → `@property` paints.
- **R2-2 (low).** The folding footprint in §0 was stale. At
  `githubsucks/folding` @ `036a994` the branch touches `src/fold.rs`,
  `editor.rs`, `editor_core.rs`, `lib.rs`, `lua_bindings/{fold.rs,mod.rs}`,
  `semantic_render.rs`, `fold.lua`, and tests — not
  `overlay.rs`/`daemon.rs`/`syntax.rs`. §0 now lists the real set; the
  zero-overlap conclusion is unchanged (stronger, if anything).

Add tree-sitter **HTML** and **CSS** grammars so `.html`/`.htm` and `.css`
buffers get lexical highlighting, and — the north-star payoff — light up
HTML's embedded-language **injections**: `<script>` → JavaScript (already
registered) and `<style>` → CSS (added here). This is the side-quest
backlog's "HTML/CSS grammars that light up more injection consumers."

## 0. Why this lane, why now (parallel-safety)

Board at this snapshot: LaTeX #144 and inline-math-docs #145 merged;
**folding Stage 1 (#142) is the only open PR** — a headless fold engine. Its
current diff (`main...githubsucks/folding` @ `036a994`) touches `src/fold.rs`
(new), `src/editor.rs`, `src/editor_core.rs`, `src/lib.rs`,
`src/lua_bindings/{fold.rs,mod.rs}`, `src/semantic_render.rs`,
`builtin/runtime/fold.lua`, and tests — **none of which this lane touches**.
gpu-invocation has landed. The GPU render path (`pmacs-gpu/src/main.rs`) remains
contended by folding's later Stage 3 — this lane never touches it.

This lane's footprint: two `Cargo.toml` grammar deps, two `BUILTIN_LANGUAGES`
entries, and — per Q#WEB4 — a small extension of `src/highlight.rs`'s recognized
capture set. **Zero file overlap with folding**; the three files it edits:

- `src/syntax.rs` — a localized append to the language table (after the `latex`
  entry, currently ~`:1111`). Folding does not touch this file.
- `Cargo.toml` — two dependency lines in the grammar block (`:142`–`:220`).
- `src/highlight.rs` — appends to the capture-style `entries` table
  (`:143-172`). Folding does not touch this file.

All are localized, low-conflict edits — the same pattern #144 and the
config-registry ∥ Vterm lanes landed conflict-free.

Unlike LaTeX (#144), **no in-repo query overlay is needed** — both crates
export their queries as constants. The overlay convention #144 established
stays available for future crates that don't (e.g. some of ruby/php).

## 1. What ships

- **HTML grammar** (`.html`/`.htm`/`.xhtml`) — tags, attributes, text,
  comments, doctype.
- **CSS grammar** (`.css`) — selectors, properties, values, at-rules,
  comments.
- **HTML injections** — `<script>…</script>` parses as JavaScript and
  `<style>…</style>` parses as CSS, via HTML's crate-exported injections query
  riding the #122 injection engine. CSS is registered here so the `<style>`
  injection resolves.

## 2. Ground truth (scouted 2026-07-23, `main` @ `4daa1b8`)

### Crate facts (verified)

- **`tree-sitter-html` 0.23.2** (2026-06). Exports `LANGUAGE: LanguageFn`,
  **`HIGHLIGHTS_QUERY`**, **`INJECTIONS_QUERY`**, `NODE_TYPES`. Deps:
  `tree-sitter-language ^0.1` (the shim every pmacs grammar uses),
  `tree-sitter ^0.24` **dev-only** → ABI-compatible with our `tree-sitter 0.26`
  via `.into()`. Its `INJECTIONS_QUERY` is the load-bearing piece: it names
  `javascript` and `css` via `#set! injection.language`.
- **`tree-sitter-css` 0.25.0** (2026-05). Exports `LANGUAGE: LanguageFn`,
  **`HIGHLIGHTS_QUERY`**, `NODE_TYPES`. Deps: `tree-sitter-language ^0.1`,
  `tree-sitter ^0.25` **dev-only** → shim-ABI-fine. No injections query (CSS
  injects nothing).

Both export their highlights query as a constant, so **no overlay/vendoring**
(the LaTeX complication) applies.

### Codebase

- **`BUILTIN_LANGUAGES`** (`src/syntax.rs:816`…`];`) currently ends at the
  `latex` entry (~`:1104-1111`, added by #144). No `html`/`css` entry. Append
  the two new entries before `];`.
- **`javascript` is registered** (`src/syntax.rs:1010`, `extensions: js/mjs/cjs`,
  `injections_query: &[]`), so HTML's `<script>` → `javascript` resolves today.
  `css` does not exist → HTML's `<style>` → `css` resolves only once this lane
  adds it.
- **Injection engine** — `collect_injection_matches` (`src/syntax.rs:375-427`)
  reads `@injection.content` (`:381`) + `injection.language` (dynamic node text
  or `#set!`, `:387-402`) + `injection.include-children` (`:402`).
  `resolve_injected_language` (`:304`) resolves the name against
  `BUILTIN_LANGUAGES` + `default_injection_aliases` (`:234`). Working
  precedents: rust `INJECTIONS_QUERY` (`:823`), markdown block/inline
  (`:848`/`:862`). HTML's `#set! injection.language "javascript"|"css"` is the
  same shape.
- **Recognized capture set** — `src/highlight.rs:143-172`, resolved by a
  dotted-prefix walk (`:189`). Confirmed against the two upstream queries
  (HTML v0.23.2 `highlights.scm`, CSS v0.25.0 `highlights.scm`): the **only**
  captures not already recognized are **`@tag`** and **`@attribute`** — used by
  BOTH grammars (`@tag.error` prefix-walks to `tag`; there is no `@tag.delimiter`).
  Everything else maps: HTML's `@constant` (doctype), `@string` (attribute
  value), `@comment`, `@punctuation.bracket`; CSS's `@operator`/`@property`/
  `@function`/`@keyword`/`@number`/`@type`/`@string.special`/`@punctuation.*`.
  CSS also has two `#match?`-gated `@variable` patterns for `--custom-props`.
  pmacs passes the buffer text to `QueryCursor::captures` (`src/syntax.rs:1701`),
  so tree-sitter **evaluates** the standard `#match?`/`#eq?`/`#any-of?` text
  predicates natively; pmacs adds handling only for the `#is? local` property
  predicate (the `property_predicates … "local"` filter at `:1703`). So an
  ordinary property (`color`) has a single `@property` capture, and only a custom
  property (`--brand`) additionally receives `@variable` — see Q#WEB4.
- **Detection** — the `extensions` field drives `language_name_for_path`
  (`:1223`) ahead of the LSP filetype map in the Lua chain
  (`builtin/runtime/syntax.lua:452-466`); no Lua edit. **No HTML/CSS language
  server is bundled**: `builtin/runtime/lsp.lua`'s default config list ends at
  YAML (`:247`), and `:521` returns `nil` without a configured command. The
  grammars provide stable language IDs (`html`/`css`), so a **user-supplied**
  `pmacs.lsp.config.html`/`.css` attaches automatically, but nothing ships or
  starts by default.

## 3. Decisions

### Q#WEB1 — Bundle `tree-sitter-html` 0.23 + `tree-sitter-css` 0.25

Add both to the `Cargo.toml` grammar block; loaders
`|| tree_sitter_html::LANGUAGE.into()` and `|| tree_sitter_css::LANGUAGE.into()`.
ABI is fine via the shared `tree-sitter-language` shim, as every current
grammar. No provenance saga (these are the official tree-sitter-org grammars,
not squatted republishes).

### Q#WEB2 — Register both; CSS before HTML's injection can resolve

- `html`: `highlights_query: &[tree_sitter_html::HIGHLIGHTS_QUERY]`,
  `injections_query: &[tree_sitter_html::INJECTIONS_QUERY]`, `locals_query: &[]`.
- `css`: `highlights_query: &[tree_sitter_css::HIGHLIGHTS_QUERY]`, injections and
  locals empty.

Both live in the same `BUILTIN_LANGUAGES`, so `resolve_injected_language`
finds `css` (and the existing `javascript`) when HTML's injections query fires.

### Q#WEB3 — Extensions

`html`: `["html", "htm", "xhtml"]`. `css`: `["css"]`. SCSS/LESS/Sass are
distinct grammars (`scss`/`less` node sets) and are deferred (§5).

### Q#WEB4 — Add exactly two capture entries: `tag` and `attribute`

The LaTeX lane could rename captures because it owned an editable overlay.
Here the highlights queries are **crate constants** — not editable — so the
reconciliation is to **extend `src/highlight.rs`'s `entries` table**
(`:143-172`). The upstream queries settle the exact set: the **only** captures
neither grammar's query already resolves are `tag` and `attribute`. Add exactly
two entries:

- `("tag", fg(5))` — HTML `(tag_name)` and CSS element/nesting/universal
  selectors, in the keyword hue (magenta) but non-bold to stay light in dense
  markup. `@tag.error` (HTML erroneous end tags) prefix-walks to this entry, so
  it needs no separate mapping.
- `("attribute", fg(3))` — HTML `(attribute_name)` and CSS
  pseudo-/attribute-selector names, in the type hue (yellow) — distinct from
  `tag`, from `property`/`operator` (cyan), and from `string` values (green).

There is **no** `tag.delimiter` (rev-1 speculation, removed); HTML's `<`/`>`/
`</`/`/>` are `@punctuation.bracket`, already handled. This extension is:

- **low-conflict** — folding does not touch `highlight.rs`;
- **general** — `tag`/`attribute` are standard tree-sitter web captures, so it
  also serves future html-ish grammars (vue/svelte/astro).

**On CSS custom properties (corrected in rev 3):** pmacs passes the buffer text
to `QueryCursor::captures` (`src/syntax.rs:1701`), so tree-sitter evaluates the
standard `#match?`/`#eq?`/`#any-of?` predicates natively — pmacs special-cases
only `#is? local`. So an ordinary property (`color`) matches only the
unconditional `(property_name) @property` and paints cleanly; a custom property
(`--brand`) additionally satisfies `#match? "^--"` and also receives `@variable`
— a benign double-capture whose winner is a within-layer precedence detail, out
of scope for v0 and not relied upon. Rejected: a shadow overlay re-capturing the
same nodes with recognized names (fragile, precedence-dependent, duplicative).

### Q#WEB5 — Injection scope: script + style only

HTML's `INJECTIONS_QUERY` (v0.23.2) has **exactly two** patterns:
`(script_element (raw_text) @injection.content) (#set! injection.language "javascript")`
and the same for `(style_element …)` → `"css"`. There is **no** event-handler
(`onclick=…`) or inline `style=` attribute injection — those are a named
deferral (§5). Each element is its own subtree, so `injection.combined` (many
matches → one shared parse, the PHP-in-HTML case) is not needed and stays
deferred. Acceptance pins script + style.

### Q#WEB6 — No protocol, frontend, or GPU change

Pure instance-side: two grammar entries, one injections query, a capture-table
extension. No wire type, no TUI/GPU edit.

## 4. Categorical bets

1. **The shim makes ABI a non-issue.** Both crates ride `tree-sitter-language
   0.1`, exactly like every bundled grammar.
2. **HTML's value is the injection, not the tags.** Highlighting a web page
   *with* its embedded JS and CSS is the north-star injection consumer;
   registering CSS is the prerequisite, which is why the two grammars ship
   together as one lane.
3. **Extending the core capture table is the correct reconciliation for
   crate-exported queries.** You cannot rename a `const` query's captures;
   teaching the highlighter the standard web captures is the general fix and
   costs one localized, uncontended edit.

## 5. Deferred (named)

- **SCSS / LESS / Sass** grammars (distinct node sets; own extensions).
- **`injection.combined`** (PHP-in-HTML and other many→one-parse schemes) — the
  existing side-quest deferral; not needed for script/style.
- **CSS-in-JS / HTML-in-JS** via `tree-sitter-javascript`'s `INJECTIONS_QUERY`
  (the `javascript` entry is `injections_query: &[]` today) — a clean
  follow-up: add the JS injections query so tagged template literals
  (`` css`…` ``, `` html`…` ``) parse. Separate consumer, separate PR.
- **Inline HTML attribute injection** — event handlers (`onclick=…`) and
  `style=` attributes are not in the upstream injections query (Q#WEB5); adding
  them would need a pmacs-side injections overlay.
- **Vue / Svelte / Astro** single-file-component grammars.
- **Bundled HTML/CSS LSP configs** — no server ships today (§2). Shipping
  default `pmacs.lsp.config.html`/`.css` entries (vscode-langservers-extracted)
  is a separate follow-up; until then the grammar's stable language ID lets a
  user config attach automatically.

## 6. Acceptance

Mirrors the #144 / CUDA conventions (`src/syntax.rs` table guards + smokes;
`src/highlight.rs` paint template `grid_paints_injected_child_keyword`).

1. **Table guards** `builtin_languages_include_html` / `_css`: entries exist,
   claim their extensions, carry non-empty highlights; `html` also carries
   `INJECTIONS_QUERY`.
2. **Load-and-parse smokes**: a minimal HTML document and a CSS rule parse with
   the expected root node kind and `!has_error()` (exact kinds — likely
   `document` / `stylesheet` — confirmed at implementation).
3. **Highlights resolve**: `reg.highlights_query("html")` / `("css")` compile
   against their grammars (the node-name compatibility gate).
4. **Extension resolution**: `.html`/`.htm`/`.xhtml` → `html`; `.css` → `css`.
5. **Injection (the payoff)**: an HTML buffer with
   `<style>a{color:red}</style>` and `<script>let x=1</script>` produces a
   `ParseTreeBundle` whose child layers resolve to `css` and `javascript`, and
   a grid-paint asserts an injected CSS property and JS keyword paint **inside**
   the embedded regions (mirroring `grid_paints_injected_child_keyword`).
6. **Paint (non-vacuous highlighting)**: parse `<a href="x">` and assert **both**
   the `<a>` tag (`@tag`) **and** the `href` attribute name (`@attribute`) paint
   the two new non-default styles — the attribute assertion is load-bearing, since
   a tag-only test could pass with `@attribute` unverified. Also assert a CSS
   selector (`@tag`) paints the new `tag` style and an ordinary CSS property
   (`color` → `@property`) paints non-default (an ordinary property has a single
   unconditional capture, so no precedence subtlety — Q#WEB4).
7. **Full gate suite** per `CLAUDE.md`.

## 7. Prior art in pmacs

- **LaTeX lane #144** (`docs/latex-grammar-math-substrate-framing.md`) — the
  grammar-add mechanics, the table-guard/smoke/paint test conventions, and the
  compile-gate-plus-paint-test discipline. (Its overlay convention is not
  needed here.)
- **Multi-language injections #122**
  (`docs/multi-language-injections-framing.md`) — the `ParseTreeBundle` +
  `Layer` engine HTML's injections ride.
- **markdown → rust fenced code** (`src/syntax.rs:848`, tested by
  `src/highlight.rs`'s `grid_paints_injected_child_keyword`) — the working
  injection precedent this lane's acceptance 5 mirrors.
