# JSON + YAML grammars — framing (side quest, highlight family)

**Revision 4 — 2026-07-21. Status: PR #123 open and awaiting review;
the public and checkpoint branches are at fully gated `5c202c5`, rebased
onto `main` `f8096ff`.**
The JSON provider and Red Hat YAML 1.24.0 have each passed their
PATH-gated pmacs acceptance, in addition to the deterministic fake-server
config-push proof and the YAML standalone protocol smoke.

**Intent.** Add `tree-sitter-json` and `tree-sitter-yaml` grammars (plus
their language servers) to the bundle. Two config formats that pmacs
currently renders as plain text, and — the reason this is the natural
next side quest — the **honest gate on the Jupyter `.ipynb` path** (JSON)
and an **immediate payoff from the injection engine just shipped (#122)**:
the markdown block grammar's `injections.scm` already sets
`injection.language "yaml"` for `---` frontmatter and `"toml"` for `+++`
frontmatter, so registering YAML lights up YAML frontmatter highlighting
with zero extra wiring (TOML frontmatter already works — `toml` landed in
#118 and injections in #122). This is a mostly-additive grammar-gap-style
change, following the #118 pattern, with the frontmatter/fence synergy as
the demonstrable headline.

---

## Ground truth (as of `main` @ `56eb67e`, #121)

- **Adding a grammar** is a one-line `LanguageEntry` in
  `crate::syntax::BUILTIN_LANGUAGES` (`name`, `extensions`, `loader`,
  `highlights_query`, `injections_query`) + a `tree-sitter-foo` dep. The
  Lua `buffer.after-load` path picks it up automatically; detection is
  extension → LSP filetype → filename → shebang
  (`resolve_active_language`).
- **Grammar name MUST equal the `pmacs.lsp.config.<name>` key** — grammar
  detection wins over the filetype map, so the name it resolves is the id
  the LSP client keys off (the #118 invariant; there's an acceptance test
  that pins every grammar-gap language to its config key).
- **LSP configs** are `pmacs.lsp.config.<name> = … or { command, args,
  [settings|init_options] }` (`builtin/runtime/lsp.lua`); no json/yaml
  config today. `pmacs.lsp.filetypes` is the LSP-only extension fallback
  (consulted only when `language_for_path` misses).
- **Injection synergy (#122).** The bundled `tree_sitter_md::
  INJECTION_QUERY_BLOCK` contains:
  - `((minus_metadata) @injection.content (#set! injection.language
    "yaml"))` — `---`-fenced frontmatter,
  - `((plus_metadata) @injection.content (#set! injection.language
    "toml"))` — `+++`-fenced frontmatter,
  - fenced code blocks via the dynamic info-string.
  So a registered `yaml` grammar is injected into markdown frontmatter
  automatically, and ` ```json `/` ```yaml `/` ```yml ` fences resolve
  (`yml`→yaml is already in `default_injection_aliases`; `json` is the
  bundled name).

**Confirmed crate facts** (probed against the registry + a build under
tree-sitter 0.26):

1. `tree-sitter-json` **0.24.8** — `pub const LANGUAGE: LanguageFn` (via
   `tree-sitter-language`, the modern shared ABI) + `HIGHLIGHTS_QUERY`.
   Compiles and links under our tree-sitter 0.26. No `INJECTIONS_QUERY`
   (JSON embeds nothing).
2. `tree-sitter-yaml` **0.7.2** — same shape (`LANGUAGE: LanguageFn`,
   `HIGHLIGHTS_QUERY`, `tree-sitter-language` dep). Compiles under 0.26.
   No `INJECTIONS_QUERY`.
3. Both are the ABI-current crates — **not** a `tree-sitter ^0.20` fork
   (the dockerfile trap from #118). A single build confirmed link +
   compile; runtime `set_language` is pinned by the ABI acceptance test.

---

## Decisions

### Q#JY1 — Two `LanguageEntry`s, self-contained highlights, no injections

Add `json` and `yaml` to `BUILTIN_LANGUAGES`, each
`highlights_query: &[…::HIGHLIGHTS_QUERY]` (self-contained, no
`; inherits:` delta), `injections_query: &[]`. Extensions:

- **json:** `.json`. (`.jsonc`/`.json5` — comment/trailing-comma variants
  the plain JSON grammar rejects — are **deferred**; a `.jsonc` grammar
  or a lenient mode is a separate call.)
- **yaml:** `.yaml`, `.yml`.

Root kinds (pinned by the ABI test): json `document`, yaml `stream`.

### Q#JY2 — LSP configs: `vscode-json-language-server` + `yaml-language-server`

- **json:** binary `vscode-json-language-server --stdio` (the VS Code
  JSON server). It is **push-model**: it reads config from
  `workspace/didChangeConfiguration` and does **not** issue
  `workspace/configuration` pulls — so pmacs, which previously only
  *answered* pulls, must now also **push** a `didChangeConfiguration`
  after `initialized` (a general LSP-client fix in `src/lsp.rs`; pull
  servers ignore it). `json.validate.enable` is set **explicitly true**
  — a missing value reads as false and silently disables validation, so
  an empty `json = {}` is wrong. The server does **not** auto-associate
  `package.json`/`tsconfig.json` (it starts with empty contributions);
  explicit `$schema` refs or configured `json.schemas` / a
  `json/schemaAssociations` push (not implemented) are required. Schema
  retrieval performs **network access** for remote `$schema` URLs, left
  enabled (`handledSchemaProtocols = {"file"}` would disable it but break
  remote schemas without a `vscode/content` impl). **Provider:** pin
  `@t1ckbase/vscode-langservers-extracted@2.0.2`
  (`npm install -g @t1ckbase/vscode-langservers-extracted@2.0.2`). Its
  published payload bundles the JSON server from VS Code 1.129.0,
  preserves the `vscode-json-language-server` command, and was
  live-smoked through initialize → config push → invalid-JSON diagnostic
  → shutdown. The unscoped package is stale; the current
  `@zed-industries` payload has a broken JSON launcher, so neither is the
  recommended provider.
- **yaml:** `yaml-language-server --stdio` (Red Hat). Its settings handler
  reads the sections **`yaml`, `http`, `[yaml]`, `editor`, `files`** (via
  `didChangeConfiguration` / pulls) — all ship present-not-null. It does
  **not** upload telemetry itself (it emits `telemetry/event` to the
  client; pmacs has no uploader), so a `redhat.telemetry` setting is inert
  and is not shipped. SchemaStore / remote schema retrieval performs
  **network access** by default.

Both servers stay **external** (installed by the user), adding **no
licensing payload** to pmacs; if either is ever bundled, retain its MIT +
dependency notices. The exact sections are **pinned in the config + a
test** (not merely "some non-nil table exists"). The pinned JSON
provider was installed into an isolated temporary prefix and
live-smoked through pmacs. Red Hat `yaml-language-server@1.24.0` was
also installed in an isolated prefix and live-smoked over stdio: its
initial configuration pull was exactly `yaml`, `http`, `[yaml]`,
`editor`, `files`; opening the document caused a second scoped
`[yaml]` pull; invalid YAML produced a parser diagnostic; shutdown was
clean. Both providers have also passed their PATH-gated pmacs acceptance.
Config-push delivery is proven deterministically through the fake server's
config sink. Servers activate only if installed; the grammar is the
always-on value.

### Q#JY3 — Filetype fallback + alias entries

Add `pmacs.lsp.filetypes` entries (`json`→json, `yaml`/`yml`→yaml) as the
stable-id fallback (grammar detection wins in practice, same role as the
`cuda`/`lua` entries). `default_injection_aliases` already has `yml`→yaml;
`json`/`yaml` are bundled names needing no alias. Special *filenames*
(`.prettierrc`, `docker-compose.yml` is already `.yml`, extensionless
CI/config yaml) are **deferred** to the filename map as a follow-up.

### Q#JY4 — Frontmatter/fence highlighting is the headline, and it's free

No new injection wiring: registering `yaml` makes the existing markdown
`minus_metadata`→yaml injection resolve, and ` ```json `/` ```yaml `
fences resolve through the #122 engine. Acceptance proves both end to end
(this is the demonstrable payoff and the tie-back to injections).

---

## Bets

1. The two crates are ABI-current and drop in like the #118 grammar-gap
   languages — verified by a build; the ABI test is the runtime pin.
2. The frontmatter/fence synergy needs zero engine changes — it falls out
   of #122 + the markdown injection query.
3. The two configuration models are now observed: JSON consumes the
   pushed full settings object; YAML 1.24.0 pulls the five documented
   sections plus a document-scoped `[yaml]` request. The remaining bet
   was that pmacs answers the real YAML server correctly end to end;
   the PATH-gated acceptance now proves that against version 1.24.0.

## Deferred (named)

- `.jsonc` / `.json5` (comments / trailing commas) — needs a lenient
  grammar or variant entry.
- Special-filename detection for extensionless config files (`.prettierrc`,
  CI yaml) via the filename map.
- JSON **schema** wiring (custom `json.schemas` / `yaml.schemas` settings)
  beyond the servers' built-in schema stores.
- The Jupyter `.ipynb` arc itself (JSON is its prerequisite, not its
  delivery).

## Acceptance

1. `builtin_languages_include_json_and_yaml` — entries present, claim
   their extensions, ship non-empty highlights.
2. `json_grammar_loads_and_parses` — ABI: `set_language` + parse a JSON
   object; root `document`, no error (the runtime ABI pin).
3. `yaml_grammar_loads_and_parses` — ABI: parse a YAML mapping; root
   `stream`, no error.
4. `json_yaml_highlights_compile` — both highlights queries compile and
   resolve several capture classes.
5. `language_for_path_resolves_json_yaml` — `.json`→json, `.yaml`/`.yml`→
   yaml.
6. `json_yaml_align_with_lsp_configs` — grammar name == the
   `pmacs.lsp.config.<name>` key (the #118 invariant).
7. **`yaml_frontmatter_injects_in_markdown`** — a markdown doc with a
   `---\nkey: val\n---` frontmatter yields a `yaml` child layer that
   highlights; **the headline synergy with #122**.
8. `json_fence_injects_in_markdown` — a ` ```json ` fence yields a `json`
   child layer.
9. `m4_json_yaml_lsp_configs_pin_command_and_sections` — the configs
   pin the binary + `json.validate.enable = true` + the exact section
   sets (json: `json`,`http`; yaml: `yaml`,`http`,`[yaml]`,`editor`,
   `files`; no inert `redhat.telemetry`) — pinned, not merely non-nil.
10. `m4_5_initial_config_pushed_via_did_change_configuration` — the
    daemon PUSHES `workspace/didChangeConfiguration` after `initialized`
    (the push-model delivery path), verified through the fake server's
    config sink. Without it, push-only servers' settings are inert.
11. `m4_real_json_provider_receives_config_and_reports_diagnostics` —
    PATH-gated live smoke for the pinned provider: initialize through
    pmacs, receive the pushed default config, open invalid JSON, and
    publish a syntax diagnostic. Skips when the binary is absent.
12. `m4_real_yaml_provider_pulls_config_and_reports_diagnostics` —
    PATH-gated live smoke for Red Hat `yaml-language-server@1.24.0`:
    auto-attach through pmacs, disable SchemaStore and Kubernetes CRD
    catalog network access for determinism, reach initialized, open
    invalid YAML, publish a diagnostic, and remain alive.

## Risks / interactions

- **LSP configuration** (Q#JY2) — JSON push, YAML standalone pulls, and
  the real YAML-through-pmacs path are observed. Both live provider
  tests remain PATH-gated, so release verification must put the pinned
  binaries on PATH rather than accepting their skip paths.
- **Themes / injections** — untouched. This is pure grammar+detection
  addition; it consumes the #122 engine, doesn't change it. No protocol
  bump.
- **`.yml` vs `.yaml`** — both map to `yaml`; no collision with any
  existing entry.
