# LaTeX grammar + math parser — framing (inline-math substrate)

**Revision 3 — pre-implementation. Ground truth: canonical `main` @
`96d0bae`, protocol v19, 2026-07-23. Status: framing only; no
implementation.** Rev 1's pmacs-side scouting passed review with every
in-repo anchor verified exactly; rev 2 corrected the external crate ground
truth (the originally named crate provably cannot link) and refreshed the
in-flight-lane snapshot; rev 3 folds in two provenance-wording corrections —
the fork-vs-upstream attribution and the now-discharged provenance diff — and
relaxes the `Cargo.toml` sequencing note. See §0.1 for the changelog.

Parent arc: `docs/inline-math-framing.md` — currently an **untracked,
desktop-only framing** not yet committed to the repo, so this path will not
resolve on a fresh clone; committing it as its own docs PR (or listing it in
the handoff's machine-local doc inventory) is a tracked follow-up. That note
frames the full four-tier inline-math renderer; its Tier 4 (GPU render) lands on
`pmacs-gpu/src/main.rs`, the render path that folding and any future math
draw pass also contend for. **This lane deliberately carves out only the
frontend-agnostic, conflict-free substrate** — the parts that can land now,
in a sibling worktree, without touching a single file the two in-flight
efforts will own.

## 0. Why this lane, why now (parallel-safety)

Two efforts are in flight; both share files with this lane, and the
disjointness must be described against their **current** state, not a stale
one (F5, verified 2026-07-23):

- **folding** (`folding` branch) is at **rev 5, APPROVED, Stage 1
  implementing** (head `40a820a`, clean worktree) — not framing-only. Its
  eventual touch set owns `semantic_render.rs`, `src/overlay.rs`,
  `src/syntax.rs` (the `ParseViewHandle` reads at `:696`/`:706`),
  `src/daemon.rs` (grid render + `dispatch_key` pre-edit unfold), the Lua
  command/dispatch layer + `src/lua_bindings/mod.rs`, **both frontends'
  gutter/render**, and `pmacs-gpu/src/main.rs` (fold mirror).
- **gpu-invocation** (`gpu-invocation` branch) is **not framing-only either**:
  its worktree carries **uncommitted implementation right now, including edits
  to root `Cargo.toml`** — one of this lane's two shared files. It will own
  `src/main.rs`, `pmacs-gpu/src/main.rs` (argv/startup),
  `pmacs-gpu/src/attach.rs`, root `Cargo.toml` (`default-run`), and the README.
- **inline-math Tier 4** (deferred, this lane's downstream) would own
  `pmacs-gpu/src/main.rs` (the render path — the three-way hotspot).

This lane touches **none** of those code paths. Its entire footprint is: one
`BUILTIN_LANGUAGES` append, one `Cargo.toml` dependency line, one new in-repo
query overlay file, and one new pure module `src/math_parse.rs`. The only two
shared files, with disjointness re-verified:

- `src/syntax.rs` — a **localized append to the language table at `:1095`**
  (after the `yaml` entry's `},`, before `];` at `:1096`), disjoint from
  folding's `ParseViewHandle` reads at `:696`/`:706`.
- root `Cargo.toml` — a **dependency line** in the grammar block
  (`:142`–`:219`). gpu-invocation's live edit is the `default-run` key in
  `[package]`; **no `default-run` key exists yet**, and the grammar dep block
  is a different region ~100 lines away. The two edits are **disjoint hunks
  that git auto-merges cleanly** — the same config-registry ∥ Vterm precedent
  this section cites — so no ordering is technically required. Flagging the
  overlap is coordination courtesy, not a merge risk.

This is the localized-single-region-append pattern that already rebased with
**zero conflicts** when config-registry and Vterm Stage 1 shared
`editor.rs`/`mod.rs` (`docs/active-work.md`, "Closed since the last
snapshot"). Its precondition — agree the file split before either lane starts,
keep each lane's footprint in shared files to a localized region — holds here
by construction; the now-active `Cargo.toml` overlap is disjoint (above) and
needs only coordination courtesy.

## 0.1 Revision history

### Round 1 (rev 1 → rev 2)

- **F1 (severe).** Rev 1's `tree-sitter-latex` 0.1.0 (crates.io) **cannot
  link.** It is a third-party repackage (publisher `cijiugechu`) of an old
  latex-lsp grammar snapshot; its declared repo
  `github.com/tree-sitter/tree-sitter-latex` **does not exist** (404, and the
  crate's VCS sha is absent from latex-lsp history). The grammar declares
  external tokens but the tarball **ships no `scanner.c`**, and its `build.rs`
  compiles the scanner only "if it exists." Proven empirically: an offline
  path-dep build fails with undefined
  `tree_sitter_latex_external_scanner_scan`/`_serialize`/`_deserialize`. The
  rev-1 scouted facts (`LANGUAGE: LanguageFn`, shim ABI compatibility) were
  each individually true — docs.rs cannot surface a *link* failure, which is
  why the scout missed it. **Fix: Q#LX1 re-based on a working vehicle (F2).**
- **F2 (severe).** A working, verified vehicle exists:
  **`codebook-tree-sitter-latex` 0.6.1** (26k downloads). See Q#LX1.
- **F3 (severe).** Q#LX2's vendoring source did not exist: latex-lsp ships
  **no `queries/` directory** (its `include = ["queries/*"]` is vestigial),
  and no first-party LaTeX `highlights.scm` exists anywhere. **Fix: Q#LX2
  re-pointed at nvim-treesitter's query.**
- **F4 (major).** Q#LX2's "real work" is broader than capture renaming — the
  fall-through classes and predicate-support gap are now named in Q#LX2.
- **F5 (major).** §0's in-flight snapshot was stale (folding is rev 5 /
  approved / implementing; gpu-invocation has live uncommitted `Cargo.toml`
  edits). §0 rewritten to reality; the disjointness argument survives with one
  added sequencing obligation.
- **F6 (minor).** Parser is **~500 lines** (parent framing, Tier 2 + the
  integration table), not ~1,000. `LanguageEntry` has **six** fields.
  `math_parse` sorts **before** `mcp` in `src/lib.rs` (alphabetical). The
  0.6.1 crate is **~3.1 MB compressed**. All corrected below.

### Round 2 (rev 2 → rev 3)

- **R2-1.** Corrected the source attribution: the crate is a republish **from
  the codebook project's fork** of `latex-lsp/tree-sitter-latex` (the fork
  modernized the bindings to the `LanguageFn` shim and renamed the crate), not
  a cut from an upstream `latex-lsp` commit. GitHub serves fork-network objects
  through the parent repo's API, which made the fork commit `948b89c` look
  upstream in rev 1/2. §2 fixed.
- **R2-2.** The Q#LX1 provenance-diff gate is **discharged, not deferred**: the
  diff against `latex-lsp` master is byte-identical on `grammar.js`/`scanner.c`
  (460 node types in both; metadata-only drift in
  `grammar.json`/`node-types.json`); the sole residual is generated `parser.c`,
  covered by the parse smokes. §2 and Q#LX1 now record the result instead of
  the obligation.
- **R2-3.** Relaxed §0's `Cargo.toml` note: the dep append and gpu-invocation's
  `default-run` key are disjoint hunks ~100 lines apart that git auto-merges
  cleanly (the cited config-registry ∥ Vterm precedent). Kept as coordination
  courtesy; dropped the inaccurate "can't assume a clean auto-merge" rationale.

## 1. What ships

- **Stage 1 — LaTeX/TeX syntax highlighting.** A bundled LaTeX grammar
  (Q#LX1) plus a vendored highlights overlay (Q#LX2) lights up `.tex` /
  `.latex` / `.sty` / `.cls` buffers. Self-contained, user-visible, shippable
  as its own PR. This is the "basic LaTeX mode" foothold and the tier-1
  detection substrate the inline-math arc needs.
- **Stage 2 — math parser: DEFERRED by decision (Q#LX5).** The pure
  `src/math_parse.rs` recursive-descent parser is framed but **not built in
  this lane** — the reviewer and author concur it should land beside its
  Tier 3 layout consumer, not ahead of it. Recorded in §Deferred with its
  rationale so the decision is not re-litigated.

Also deferred to the inline-math arc proper (§Deferred): Tier 3 layout,
Tier 4 GPU render, and instance-side `(math_environment) @math` injection
detection.

## 2. Ground truth (scouted 2026-07-23, `main` @ `96d0bae`; pmacs-side
anchors verified in review, external facts corrected per §0.1)

### Grammar table

- **`LanguageEntry` struct — `src/syntax.rs:766-801`**, **six** fields:
  `name` (`:769`), `extensions: &[&str]` (`:774`),
  `loader: fn() -> tree_sitter::Language` (`:778`),
  `highlights_query: &[&str]` (`:789`), `locals_query: &[&str]` (`:793`),
  `injections_query: &[&str]` (`:800`). There is no `tags`/`filename`/`shebang`
  field — extensions are the only Rust-side detection signal.
- **`BUILTIN_LANGUAGES` table — `src/syntax.rs:816` … `];` at `:1096`.** A new
  entry appends after the `yaml` entry's `},` at `:1095`. The "adding a
  grammar" checklist is documented in-place at `:803-815`.
- Example entry, verbatim (`src/syntax.rs:985-992`):
  ```rust
  LanguageEntry {
      name: "python",
      extensions: &["py", "pyi"],
      loader: || tree_sitter_python::LANGUAGE.into(),
      highlights_query: &[tree_sitter_python::HIGHLIGHTS_QUERY],
      locals_query: &[],
      injections_query: &[],
  },
  ```
- Grammar crate deps live in `Cargo.toml:136-219` (`tree-sitter = "0.26"` at
  `:136`; grammar crates `:142`–`:219`).

### Detection chain

- The `extensions` field drives `language_name_for_extension`
  (`src/syntax.rs:1223`) and `language_name_for_path` (`:1235`).
- The full precedence chain (PR #132 modeline work) is Lua,
  `detect_buffer_language` at `builtin/runtime/syntax.lua:452-466`: **modeline
  → grammar extension → LSP filetype map → filename map → shebang.** Because
  grammar-extension detection sits *ahead* of the LSP filetype map, adding
  `tex`/`latex`/`sty`/`cls` to the new entry's `extensions` **wires the whole
  chain with no Lua edit**.

### Query sourcing

- **No in-repo `.scm` highlight overlays exist today.** Every grammar sources
  its queries from crate-exported `&'static str` constants (naming varies:
  `HIGHLIGHTS_QUERY`, `HIGHLIGHT_QUERY`, `HIGHLIGHT_QUERY_BLOCK`; documented at
  `src/syntax.rs:779-800`). The CUDA entry (`:919-923`) shows queries composed
  as a **slice of fragments** in the field.
- The only `.scm` in the tree is `audit/audit-rules.scm`, pulled via
  `include_str!` at `src/audit/mod.rs:76` — the sole precedent for shipping
  static query text from a repo file.
- **Highlight-capture handling** (F4): the prefix-walk at
  `src/highlight.rs:189` already absorbs `@function.macro`, `@keyword.*`,
  `@string`, `@comment`, and `@punctuation.*`. But `@markup.*` (headings,
  italic, bold, math), `@module`, and `@label` **fall through to default
  style** — i.e. the most LaTeX-distinctive content would paint as plain text
  unless the vendored query's captures are renamed onto the recognized set.
- **Predicate support** (F4): pmacs evaluates only the `#is? local` property
  predicate; `#match?` / `#eq?` / `#any-of?` / `#lua-match?` are **silently
  ignored** for every grammar (pre-existing, not a blocker). The nvim query
  has 8 such uses that must be curated out or accepted as benign over-matching.

### Injection seam (future math detection)

- `collect_injection_matches` (`src/syntax.rs:375-427`) is the capture-
  consumption point: it reads `@injection.content` (`:381`) and
  `injection.language` (`:385-388`), plus `#set!` properties (`:397-404`).
  Child-language resolution is `resolve_injected_language` (`:304`). The
  markdown→rust fenced-code injection is the working precedent. A future
  `(math_environment) @injection.content` + `#set! injection.language "math"`
  hooks in here unchanged (Q#LX4, deferred).

### Module registration & tests

- Crate modules are declared in the alphabetized `pub mod` block at
  `src/lib.rs:42-142`; `mod math_parse;` sorts **before** `mcp` (`:100`).
- Grammar test convention (CUDA template, `src/syntax.rs:2130-2221`): a table
  guard `builtin_languages_include_<lang>`, a smoke test
  `<lang>_grammar_loads_and_parses_*` (`reg.language(...)` → parse → assert
  root kind + `!has_error()`), a highlight-resolve test, and an extension-
  resolution test. End-to-end paint template: `src/highlight.rs:1296-1346`.
- **Greenfield confirmed** — no `math`/`latex`/`tex` code anywhere in `src/`,
  `builtin/`, `pmacs-gpu/`, `pmacs-protocol/`.

### Crate facts (external, corrected + reviewer-verified 2026-07-23)

- **Rejected — `tree-sitter-latex` 0.1.0.** Broken repackage; dead declared
  repo; **no `scanner.c`** for its declared external tokens; proven offline
  link failure (undefined `tree_sitter_latex_external_scanner_*`). Not
  linkable at any pin. (F1.)
- **Chosen — `codebook-tree-sitter-latex` 0.6.1** (26k downloads,
  ~3.1 MB compressed). A republish **from the codebook project's fork of
  `latex-lsp/tree-sitter-latex`** (the real, maintained grammar — MIT,
  texlab's author). The fork **modernized the bindings to the `LanguageFn`
  shim and renamed the crate**; genuine `latex-lsp` master still carries
  old-style bindings (`tree-sitter = 0.24.1` as a *direct* dependency, no
  shim). (Rev 1/2 mis-attributed this to a Dec-2025 upstream `latex-lsp`
  commit — GitHub serves fork-network objects through the parent repo's
  commit/contents API, so the fork's commit `948b89c`, whose `lib.rs` already
  carries the `codebook_tree_sitter_latex` name, looked upstream.) It **ships
  `scanner.c`**, exports **`LANGUAGE: LanguageFn`** over
  `tree-sitter-language 0.1` with `tree-sitter` **dev-only** — the exact shim
  pattern every pmacs grammar uses. Verified end-to-end: builds against
  `tree-sitter 0.26` and parses a document containing a **verbatim
  environment** (which exercises the external scanner) → root kind
  `source_file`, no errors. The canonical name `tree-sitter-latex` is squatted
  by the broken crate, hence the `codebook-` prefix.
- **Provenance diff — run, not deferred (R2-2).** The 0.6.1 crate was diffed
  against genuine `latex-lsp` master: `grammar.js` and `scanner.c` — the
  grammar definition and the hand-written C, the security-relevant surfaces —
  are **byte-identical**. `grammar.json` / `node-types.json` differ **only** by
  newer tree-sitter-cli output metadata (supertype population, extra flags):
  **460 node types in both**, with type sets, named flags, and rule counts all
  identical. The **sole residual** is `parser.c`, which is generated and not
  committed upstream, so it cannot be diffed — it is exercised by the parse
  smokes (acceptance 2), and an optional full discharge is regenerating it with
  `tree-sitter-cli` at implementation time.

## 3. Decisions

### Q#LX1 — Bundle `codebook-tree-sitter-latex` 0.6.1

Add `codebook-tree-sitter-latex = "0.6"` to the `Cargo.toml` grammar block;
loader is `|| codebook_tree_sitter_latex::LANGUAGE.into()` (confirmed against
the crate's `[lib] name = "codebook_tree_sitter_latex"`). This is the linkable
republish from the codebook project's fork of `latex-lsp/tree-sitter-latex`,
which modernized the bindings to the `LanguageFn` shim (§2); the
originally-named `tree-sitter-latex` 0.1.0 is rejected as unlinkable (§2, F1).
ABI is fine via the `tree-sitter-language` shim, exactly as every current
grammar. The provenance concern (dirty-tree publish, squatted name) is
**discharged, not deferred**: the diff against `latex-lsp` master is
byte-identical on `grammar.js`/`scanner.c`, with only tree-sitter-cli metadata
drift elsewhere and generated `parser.c` as the sole residual (§2, R2-2).
Rejected: vendoring generated `parser.c`/`scanner.c` directly (compile +
maintenance burden the crate absorbs) and re-pinning `tree-sitter`
(unnecessary — the shim decouples us).

### Q#LX2 — Vendor nvim-treesitter's highlights query (no first-party source exists)

latex-lsp ships **no** `queries/` directory and no first-party
`highlights.scm` exists anywhere (F3). Vendor **nvim-treesitter's
`queries/latex/highlights.scm`** (342 lines, **Apache-2.0** — compatible with
pmacs' `MIT OR Apache-2.0`; Helix's MPL-2.0 version is the fallback), host it
at `builtin/queries/latex/highlights.scm`, and wire it into the entry's
`highlights_query` via `include_str!` (the `audit-rules.scm` precedent). This
establishes the first **in-repo grammar-query overlay convention**, which the
ruby/php/html/css backlog then reuses.

The **real work** (F4), beyond copying the file:
1. **Capture reconciliation.** Rename/curate the query's `@markup.*`,
   `@module`, and `@label` captures onto pmacs' recognized set
   (`src/highlight.rs:189`) or they paint as default — and those are the
   LaTeX-distinctive captures (headings, emphasis, math, environment names).
2. **Predicate curation.** The 8 `#match?`/`#eq?`/`#any-of?`/`#lua-match?`
   uses are silently ignored (§2); curate them out, or accept the resulting
   over-matching, per rule.
3. **Node-name compatibility.** Acceptance test 3 (`Query::new` compiles the
   vendored query against the chosen grammar, §6) **doubles as the
   grammar/query node-name compatibility check** — it is the test that catches
   a grammar-version/query mismatch.

Prefer `include_str!` of a `.scm` file over an in-source `&'static str` const
so the query stays diffable and lintable as query source.

### Q#LX3 — Extensions and detection

`extensions: &["tex", "latex", "sty", "cls"]`. A single `BUILTIN_LANGUAGES`
append at `:1095` wires the full detection chain (grammar-extension precedes
the LSP filetype map in `syntax.lua:457`). No Lua edit; no
modeline/filename/shebang changes.

### Q#LX4 — Locals and injections empty for v0

`locals_query: &[]`, `injections_query: &[]`. LaTeX has no lexical-scope
locals worth shipping in v0. The `(math_environment) @math` injection — the
instance-side, wire-authoritative alternative to the inline-math framing's
frontend-local detection — is **deferred to the inline-math arc**; it hooks
`collect_injection_matches` (`syntax.rs:375-427`) unchanged when a `"math"`
layer exists. Verbatim/`listings`/`minted` code-block injections are a later
quality pass.

### Q#LX5 — The math parser is deferred, not front-run (decided)

`src/math_parse.rs` (LaTeX math → the `MathNode` AST in
`docs/inline-math-framing.md` §Tier 2, **~500 lines** + a ~200-entry symbol
map) is pure, dependency-free, and conflict-free. Reviewer and author concur:
**defer it** and land it beside its Tier 3 layout consumer. Rationale, beyond
the no-build-ahead discipline (folding refuses `BlockAdornments`;
gpu-invocation refuses `FILE`):

- **No design pressure until layout exists.** The parent framing places the
  hard part in Tier 3 layout; the `MathNode` shape is only validated once
  layout consumes it. Freezing the AST now risks rework.
- **The parallelization argument is void.** The parser is conflict-free *by
  construction* — it can land at any time without lane contention — so there
  is nothing to gain by front-running it. Front-running only trades that free
  optionality for speculative, unexercised code.

Stage 1 (grammar) therefore is this lane's shippable PR; the parser lands with
the inline-math arc. Recorded here so it is not re-opened — the defer is
ratified by the user's approval of this framing, per the workflow (author +
reviewer concurrence proposes it; approval decides it).

### Q#LX6 — No protocol, frontend, or GPU change in this lane

No wire type, no `pmacs-protocol` touch, no TUI/GPU change. Stage 1 is a pure
instance-side grammar-table addition. This is what keeps the lane orthogonal
to both in-flight efforts.

## 4. Categorical bets

1. **The `LanguageFn` shim makes crate-version skew a non-issue.** Every
   grammar already crosses the `tree-sitter 0.26` boundary through
   `tree-sitter-language`; `codebook-tree-sitter-latex` is no different —
   confirmed by a real build. The linkability risk was in the *scanner*, not
   the shim, and is resolved by choosing a crate that ships `scanner.c` (F1/F2).
2. **LaTeX highlighting is worth ~3.1 MB of grammar.** It is the largest
   single grammar in the tree — a real binary-size and compile-time cost —
   justified by being the LaTeX-mode foothold and a heavily-used authoring
   format. A Cargo feature-gate is a v1 escape hatch if the size bites builds
   that never edit `.tex`.
3. **An in-repo query overlay is a convention to establish, not a blocker.**
   It generalizes to every grammar crate that ships no Rust query constant —
   directly unblocking the ruby/php/html/css backlog.
4. **A verified republish beats a canonical-but-broken name.** The squatted
   `tree-sitter-latex` name would have been the "obvious" pick and does not
   link; the provenance diff (Q#LX1, now discharged — grammar and scanner
   byte-identical to upstream) is the cost, already paid, of using the working
   republish safely.

## 5. Deferred (named)

- **Math parser `src/math_parse.rs`** — decided defer (Q#LX5); lands with
  Tier 3 layout, not this lane.
- **Tier 3 math layout** (`src/math_layout.rs`, a font crate — `read-fonts`
  or `ttf-parser` — + OpenType MATH-table box layout). New file, no contested
  touch, but pulls a dependency and has real complexity; belongs to the
  inline-math arc beside its GPU consumer.
- **Tier 4 GPU render** (`pmacs-gpu/src/main.rs` render path). The three-way
  hotspot; wait for folding and gpu-invocation to settle before opening it.
- **Instance-side `(math_environment) @math` injection detection** + a `"math"`
  `BUILTIN_LANGUAGES` layer (the wire-authoritative upgrade over the framing's
  frontend-local scan).
- **A real `latex-mode.lua`**: texlab LSP config, sectioning folds (rides
  Arc 6), `\ref`/`\label` navigation, environment/snippet completion,
  auto-`\end{}`. Full-document LaTeX, not math rendering.
- **Curated highlight refinements** beyond the vendored query, including
  re-instating the ignored predicates should pmacs' predicate support grow;
  verbatim/`listings`/`minted` code injections.

## 6. Acceptance

**Stage 1 (grammar).** Tests follow the CUDA convention
(`src/syntax.rs:2130-2221`) + the paint template (`src/highlight.rs:1296`):

1. **Table guard** `builtin_languages_include_latex`: the entry exists, claims
   `tex`/`latex`/`sty`/`cls`, and carries the overlay highlights fragment.
2. **Load-and-parse smoke** `latex_grammar_loads_and_parses`:
   `reg.language("latex")`, parse a minimal
   `\documentclass{article}\begin{document}Hello $x$\end{document}` **plus a
   `verbatim` environment** (to exercise the external scanner), assert the
   root node kind `source_file` and `!has_error()`.
3. **Highlights resolve + node-name compatibility**
   `latex_highlights_resolve`: `reg.highlights_query("latex")` compiles the
   vendored query against the grammar (this is also the grammar/query
   compatibility gate, Q#LX2), and a `\command`, a `{group}`, and a
   sectioning/emphasis construct paint the reconciled capture classes (not
   default).
4. **Extension resolution** `language_for_path_resolves_latex_extensions`:
   `foo.tex` / `foo.latex` / `pkg.sty` / `cls.cls` all resolve to `"latex"`.
5. **Full gate suite** per `CLAUDE.md`: `cargo fmt --check`; `cargo clippy
   --workspace --all-targets -- -D warnings`; `cargo test --lib`; `cargo test
   --lib --features crdt`; the touched acceptance suites; `cargo test --test
   m4_acceptance -- --skip basedpyright`; `PMACS_REQUIRE_GPU=1 cargo test -p
   pmacs-gpu`; `git diff --check`.

(Stage 2 parser acceptance moves to the inline-math arc with the parser
itself, per Q#LX5.)

## 7. Prior art in pmacs

- `docs/inline-math-framing.md` — the parent arc; this lane is its Tier 1
  substrate, with the Tier 2 parser and Tiers 3–4 deferred.
- `docs/multi-language-injections-framing.md` / PR #122 — the
  `ParseTreeBundle` + `Layer` injection engine that a future
  `(math_environment) @math` capture rides (Q#LX4).
- PR #132 (modeline detection) — the extension→language chain
  (`syntax.lua:452-466`) that a single `extensions` field plugs into.
- CUDA grammar (`src/syntax.rs:919-923`, tests `:2130-2221`) — the closest
  precedent for a multi-fragment highlights query and the grammar test
  convention; `audit/audit-rules.scm` (`src/audit/mod.rs:76`) — the sole
  precedent for an `include_str!`-loaded `.scm`.
