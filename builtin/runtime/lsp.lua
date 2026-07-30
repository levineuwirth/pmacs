-- builtin/runtime/lsp.lua --- T M4.12 default LSP integration.
--
-- Wires the LSP request/response surface into a usable editor UX:
--   * Declarative server config (`pmacs.lsp.config[language]`).
--   * Auto-attach + did_open / did_change / did_close on buffer events.
--   * `pmacs.lsp.go_to_definition` / `pmacs.lsp.format_buffer` /
--     `pmacs.lsp.hover_at_cursor` / `pmacs.lsp.signature_help_at_cursor`
--     / `pmacs.lsp.rename` / `pmacs.lsp.code_actions` /
--     `pmacs.lsp.inlay_hints` / `pmacs.lsp.semantic_tokens`, bound to
--     default chords below.
--
-- Scope: one server per language across all buffers; async-await
-- request/react (the editor never blocks). Landed: cross-file
-- go-to-definition (L1), multi-file rename / WorkspaceEdit applier
-- (L2) with `textDocument/prepareRename` gating when the server
-- supports it, code actions + `workspace/executeCommand` + server→client
-- `workspace/applyEdit` (L3), ordered resource-op edits
-- (create/rename/delete file) with buffer-registry reconciliation
-- (L4), inlay hints, and semantic tokens (each data + modeline;
-- wiring them into rendering is a separate rendering milestone),
-- incl. the server→client `workspace/inlayHint/refresh` and
-- `workspace/semanticTokens/refresh` requests, and dynamic
-- `workspace/didChangeWatchedFiles` registration backed by a
-- polling snapshot-diff watcher.

pmacs.lsp = pmacs.lsp or {}
-- Per-language server config. Each entry:
--   command       (string)  required — server binary
--   args          (list)    argv after command
--   env           (table)   extra environment
--   init_options  (table)   `initializationOptions`
--   settings      (table)   answered to `workspace/configuration`
--   root  (string|function) optional explicit project root; overrides
--                           the `pmacs.project.detect` marker walk used
--                           to set `rootUri`/`cwd`. A `function(path) ->
--                           string|nil` is resolved per file and
--                           memoized per directory; returning nil
--                           declines and falls through to the marker
--                           walk (see project_root_for)
pmacs.lsp.config = pmacs.lsp.config or {}

-- Default rust-analyzer config. Users replace any field from init.lua
-- before any rust file opens.
pmacs.lsp.config.rust = pmacs.lsp.config.rust or {
  command = "rust-analyzer",
  args = {},
  init_options = {
    cargo = { allFeatures = true },
    checkOnSave = { command = "clippy" },
    procMacro = { enable = true },
  },
}

-- Default Python config: basedpyright (an MIT fork of pyright that
-- re-enables inlay hints / semantic tokens in the open-source server,
-- which upstream pyright withholds for Pylance). `--stdio` is the
-- LSP transport. `settings` is answered to basedpyright's
-- `workspace/configuration` pull (pmacs now advertises that
-- capability) — `basic` keeps the diagnostics gutter from being
-- flooded by the fork's stricter defaults. A project's
-- `pyrightconfig.json` / `[tool.pyright]` still wins where present.
-- Users override any field from init.lua before a .py opens;
-- swapping to upstream pyright is just `command = "pyright-langserver"`.
pmacs.lsp.config.python = pmacs.lsp.config.python or {
  command = "basedpyright-langserver",
  args = { "--stdio" },
  settings = {
    python = { analysis = { typeCheckingMode = "basic" } },
    basedpyright = { analysis = { typeCheckingMode = "basic" } },
  },
}

-- C / C++ via clangd. One clangd binary serves both; `config.c` and
-- `config.cpp` are separate entries only so the `language_id` sent in
-- `didOpen` is accurate (clangd respects it). clangd takes its
-- project model from `compile_commands.json` / `compile_flags.txt`
-- at the project root, not `workspace/configuration`, so no
-- `settings` here; `--background-index` enables cross-file features.
-- Users override from init.lua before a C/C++ file opens.
pmacs.lsp.config.c = pmacs.lsp.config.c or {
  command = "clangd",
  args = { "--background-index" },
}
pmacs.lsp.config.cpp = pmacs.lsp.config.cpp or {
  command = "clangd",
  args = { "--background-index" },
}

-- CUDA (`.cu`/`.cuh`) via clangd — the same binary serves it; `config.cuda`
-- is a separate entry so the `language_id` sent in `didOpen` is `cuda`.
-- clangd, however, picks the compiler *language* from the file extension,
-- NOT that `language_id`: it recognizes `.cu` (→ `-x cuda`) but not `.cuh`,
-- so a standalone header with no compile command otherwise fails to build
-- an AST (`fe_expected_compiler_job`). `initializationOptions.fallbackFlags
-- = { "-xcuda" }` supplies `-x cuda` for any file this server opens that
-- lacks a `compile_commands.json` entry, which fixes `.cuh` (and bare `.cu`)
-- headers; a real compile command still wins where present. This server
-- only ever serves `.cu`/`.cuh`, so the fallback can't mis-flag C/C++.
-- Like C/C++, clangd takes the project model from `compile_commands.json`
-- / `compile_flags.txt`, so no `settings` here. For real analysis clangd
-- must also locate a CUDA toolkit: it probes common install roots (e.g.
-- `/usr/local/cuda`), and a project can pin `--cuda-path=` / the GPU arch
-- through its compile flags; absent those, navigation and hover still work
-- but diagnostics may be noisy. Users override from init.lua before a CUDA
-- file opens.
pmacs.lsp.config.cuda = pmacs.lsp.config.cuda or {
  command = "clangd",
  args = { "--background-index" },
  init_options = { fallbackFlags = { "-xcuda" } },
}

-- Go via gopls. `gopls` with no args serves LSP over stdio. gopls
-- pulls its configuration via `workspace/configuration` (now
-- answered, #13) under the `gopls` section; an empty section means
-- "use defaults" — present, not null, which gopls prefers. Users
-- populate it (e.g. staticcheck, analyses) from init.lua.
pmacs.lsp.config.go = pmacs.lsp.config.go or {
  command = "gopls",
  args = {},
  settings = { gopls = {} },
}

-- TypeScript / JavaScript via typescript-language-server (the
-- tsserver wrapper). One binary serves the whole family; like
-- `c`/`cpp` these are separate config entries purely so the
-- `language_id` sent in `didOpen` is accurate — tsserver keys
-- diagnostics and some code actions off it (typescript /
-- typescriptreact / javascript / javascriptreact). `--stdio` is the
-- LSP transport. The project model comes from `tsconfig.json` /
-- `jsconfig.json` (no `workspace/configuration` pull). Users
-- override from init.lua before a TS/JS file opens; swapping to
-- `vtsls` is just `command = "vtsls"`.
for _, lid in ipairs({ "typescript", "typescriptreact", "javascript", "javascriptreact" }) do
  pmacs.lsp.config[lid] = pmacs.lsp.config[lid] or {
    command = "typescript-language-server",
    args = { "--stdio" },
  }
end

-- Lua via lua-language-server (sumneko). Speaks LSP over stdio with
-- no transport flag. It pulls configuration via
-- `workspace/configuration` under the `Lua` section; an empty table
-- is "use defaults" — present, not null (same rationale as gopls).
-- `.lua` is tree-sitter-grammar-backed, so `pmacs.parse` already
-- resolves the `lua` language and this is the config that attaches.
-- Users populate `settings.Lua` (runtime.version, workspace.library,
-- diagnostics.globals = { "pmacs" }, …) from init.lua.
pmacs.lsp.config.lua = pmacs.lsp.config.lua or {
  command = "lua-language-server",
  args = {},
  settings = { Lua = {} },
}

-- Bash / shell via bash-language-server. `start` is its
-- LSP-over-stdio subcommand. No project config; it shells out to
-- `shellcheck` (diagnostics) and `shfmt` (formatting) when those are
-- on PATH. Users override from init.lua before a shell file opens.
pmacs.lsp.config.bash = pmacs.lsp.config.bash or {
  command = "bash-language-server",
  args = { "start" },
}

-- Dockerfile via docker-langserver (dockerfile-language-server-nodejs).
-- `--stdio` is the LSP transport. No project config. The `dockerfile`
-- language id is set from the bundled grammar / filename map, so a file
-- named `Dockerfile` (no extension) attaches. Users override from
-- init.lua before a Dockerfile opens. Make has no language server, so
-- there is deliberately no `config.make`.
pmacs.lsp.config.dockerfile = pmacs.lsp.config.dockerfile or {
  command = "docker-langserver",
  args = { "--stdio" },
}

-- CMake via cmake-language-server (the Python server). It speaks LSP over
-- stdio with no transport flag. Unlike gopls/taplo it does NOT pull a
-- `workspace/configuration` section: it reads `buildDirectory` from the
-- `initialize` request's `initializationOptions`, and drives its project
-- model off CMake's File API under `<buildDirectory>/.cmake/api/` (not
-- `compile_commands.json`). So the config is an `init_options`, defaulting
-- to the conventional out-of-source `build/`; users override
-- `init_options.buildDirectory` from init.lua.
pmacs.lsp.config.cmake = pmacs.lsp.config.cmake or {
  command = "cmake-language-server",
  args = {},
  init_options = { buildDirectory = "build" },
}

-- TOML via taplo. `taplo lsp stdio` serves LSP over stdio. taplo
-- pulls configuration via `workspace/configuration` under the
-- `taplo` section (empty ⇒ defaults, present not null); a project
-- `.taplo.toml` still wins. Users populate it from init.lua.
pmacs.lsp.config.toml = pmacs.lsp.config.toml or {
  command = "taplo",
  args = { "lsp", "stdio" },
  settings = { taplo = {} },
}

-- Zig via zls. `zls` with no args serves LSP over stdio; it reads
-- `zls.json` / `build.zig` for the project model (no
-- `workspace/configuration` pull). Users override from init.lua
-- before a `.zig` / `.zon` file opens.
pmacs.lsp.config.zig = pmacs.lsp.config.zig or {
  command = "zls",
  args = {},
}

-- JSON via the VS Code JSON server, binary `vscode-json-language-server`.
-- It is a PUSH-model server: it reads config from
-- `workspace/didChangeConfiguration` (the daemon now sends one after
-- `initialized`) and does NOT issue `workspace/configuration` pulls, so
-- without that push these settings would be inert. `json.validate.enable`
-- is set explicitly true — the server treats a MISSING value as false, so
-- an empty `json = {}` would silently disable validation. Schema
-- retrieval performs NETWORK ACCESS for remote `$schema` URLs (left
-- enabled; `handledSchemaProtocols = {"file"}` would disable it but break
-- remote schemas without a `vscode/content` impl). Note: the server does
-- NOT auto-associate `package.json`/`tsconfig.json` — it starts with empty
-- contributions; explicit `$schema` refs or configured `json.schemas` /
-- a `json/schemaAssociations` push (not implemented) are required.
-- Provider: pin `@t1ckbase/vscode-langservers-extracted@2.0.2`
-- (`npm install -g @t1ckbase/vscode-langservers-extracted@2.0.2`).
-- Its published payload bundles the JSON server from VS Code 1.129.0,
-- preserves this command name, and was live-smoked through initialize →
-- config push → invalid-JSON diagnostic → shutdown. The older unscoped
-- package is stale and the current `@zed-industries` payload has a broken
-- JSON launcher; neither is the recommended provider.
pmacs.lsp.config.json = pmacs.lsp.config.json or {
  command = "vscode-json-language-server",
  args = { "--stdio" },
  settings = {
    json = { validate = { enable = true } },
    http = {},
  },
}

-- YAML via Red Hat `yaml-language-server`. On
-- `workspace/didChangeConfiguration` (now pushed after `initialized`) it
-- reads the `yaml`, `http`, `[yaml]`, `editor`, and `files` sections — all
-- ship present-not-null (empty ⇒ server defaults). SchemaStore / remote
-- schema retrieval performs NETWORK ACCESS by default. The standalone
-- server does not upload telemetry itself — it emits `telemetry/event`
-- notifications to its client, and pmacs has no telemetry uploader, so a
-- `redhat.telemetry` setting would be inert and is not shipped. Sections
-- live-observed with Red Hat `yaml-language-server@1.24.0`: its initial
-- pull requests exactly those five sections, and opening a YAML document
-- requests a second scoped `[yaml]` section. The standalone smoke reached
-- a real syntax diagnostic and clean shutdown. The PATH-gated pmacs
-- acceptance also proves auto-attach, initialization, config pulls, a
-- syntax diagnostic, and continued server liveness with both catalogs
-- disabled for network-free determinism.
pmacs.lsp.config.yaml = pmacs.lsp.config.yaml or {
  command = "yaml-language-server",
  args = { "--stdio" },
  settings = {
    yaml = {},
    http = {},
    ["[yaml]"] = {},
    editor = {},
    files = {},
  },
}

-- LSP-side extension → language map, deliberately independent of the
-- tree-sitter detection in `pmacs.parse`. Consulted only when
-- `pmacs.parse.language_for_path` finds nothing (an extension with a
-- server but no bundled grammar), so grammar-backed languages keep their
-- existing detection. Every language with an LSP config now also ships a
-- grammar, so this is mainly the LSP-only fallback that keeps a language
-- id stable if a grammar is ever dropped, plus the seam for user-added
-- mappings. Extensible from init.lua: `pmacs.lsp.filetypes.foo = "bar"`.
pmacs.lsp.filetypes = pmacs.lsp.filetypes or {}
pmacs.lsp.filetypes.py = pmacs.lsp.filetypes.py or "python"
pmacs.lsp.filetypes.pyi = pmacs.lsp.filetypes.pyi or "python"
-- C. `.h` is ambiguous C/C++; default it to C (clangd copes either
-- way, and users can remap `pmacs.lsp.filetypes.h = "cpp"`).
pmacs.lsp.filetypes.c = pmacs.lsp.filetypes.c or "c"
pmacs.lsp.filetypes.h = pmacs.lsp.filetypes.h or "c"
-- C++.
for _, ext in ipairs({ "cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp", "inl", "cppm" }) do
  pmacs.lsp.filetypes[ext] = pmacs.lsp.filetypes[ext] or "cpp"
end
-- CUDA. pmacs bundles a CUDA grammar, so `language_for_path` already
-- resolves `.cu`/`.cuh` to `cuda` and this map is never consulted for
-- them in practice; the entries are the LSP-only fallback that keeps
-- the language id stable if that grammar is ever dropped (same role as
-- the `lua` entry below).
pmacs.lsp.filetypes.cu = pmacs.lsp.filetypes.cu or "cuda"
pmacs.lsp.filetypes.cuh = pmacs.lsp.filetypes.cuh or "cuda"
-- Go.
pmacs.lsp.filetypes.go = pmacs.lsp.filetypes.go or "go"
-- Tier 1 single-binary servers. TypeScript / JavaScript distinguish
-- the JSX variants so the server enables the JSX parser.
for _, ext in ipairs({ "ts", "mts", "cts" }) do
  pmacs.lsp.filetypes[ext] = pmacs.lsp.filetypes[ext] or "typescript"
end
pmacs.lsp.filetypes.tsx = pmacs.lsp.filetypes.tsx or "typescriptreact"
for _, ext in ipairs({ "js", "mjs", "cjs" }) do
  pmacs.lsp.filetypes[ext] = pmacs.lsp.filetypes[ext] or "javascript"
end
pmacs.lsp.filetypes.jsx = pmacs.lsp.filetypes.jsx or "javascriptreact"
-- Lua (lua-language-server). pmacs bundles a Lua grammar, so
-- `language_for_path` resolves `.lua` first; this entry is the LSP
-- fallback and keeps the language id stable if that ever changes.
pmacs.lsp.filetypes.lua = pmacs.lsp.filetypes.lua or "lua"
-- Bash / shell (bash-language-server). pmacs bundles a bash grammar, so
-- `language_for_path` already resolves these to `bash`; this map is the
-- LSP-only fallback that keeps the language id stable if that grammar is
-- ever dropped (same role as the `lua`/`cuda` entries). The wider shell
-- family (`.zsh`/`.ksh`/`.ash`/`.bats`) rides the same server; shellcheck
-- declines zsh, so `.zsh` diagnostics may be sparse.
for _, ext in ipairs({ "sh", "bash", "zsh", "ksh", "ash", "bats" }) do
  pmacs.lsp.filetypes[ext] = pmacs.lsp.filetypes[ext] or "bash"
end
-- Dockerfile / Make / CMake. Bundled grammars resolve these via
-- `language_for_path` (and filename map for extensionless files); these
-- extension entries are the LSP-only fallback that keeps the id stable if
-- a grammar is dropped. `.mk`/`.make` map to `make`, which has no server
-- (grammar highlight only); `.dockerfile`/`.cmake` attach their servers.
pmacs.lsp.filetypes.dockerfile = pmacs.lsp.filetypes.dockerfile or "dockerfile"
pmacs.lsp.filetypes.containerfile = pmacs.lsp.filetypes.containerfile or "dockerfile"
pmacs.lsp.filetypes.cmake = pmacs.lsp.filetypes.cmake or "cmake"
for _, ext in ipairs({ "mk", "make" }) do
  pmacs.lsp.filetypes[ext] = pmacs.lsp.filetypes[ext] or "make"
end
-- TOML (taplo).
pmacs.lsp.filetypes.toml = pmacs.lsp.filetypes.toml or "toml"
-- Zig (zls). `.zon` is Zig Object Notation, handled by the same server.
pmacs.lsp.filetypes.zig = pmacs.lsp.filetypes.zig or "zig"
pmacs.lsp.filetypes.zon = pmacs.lsp.filetypes.zon or "zig"
-- JSON / YAML. Both ship grammars, so `language_for_path` already resolves
-- these and the map is the stable-id fallback (same role as `lua`/`cuda`).
pmacs.lsp.filetypes.json = pmacs.lsp.filetypes.json or "json"
pmacs.lsp.filetypes.yaml = pmacs.lsp.filetypes.yaml or "yaml"
pmacs.lsp.filetypes.yml = pmacs.lsp.filetypes.yml or "yaml"

-- Per-buffer attachment record: { language, server, uri, version }.
-- Keyed by `tostring(BufferIdLua)` because BufferIdLua hands out fresh
-- userdata each call (so two handles to the same buffer wouldn't hash
-- equal as raw keys).
local attachments = {}

-- Minimal file:// percent-encoder. Matches src/lsp.rs's policy: ASCII
-- alpha-num + a small set of path-safe punctuation pass through; every
-- other byte goes through %XX. Iterates per-byte (`gmatch(".")` is
-- byte-wise) so multibyte UTF-8 components encode cleanly.
local function file_uri_for(path)
  if not path then return nil end
  local out = "file://"
  for ch in path:gmatch(".") do
    local b = string.byte(ch)
    if (b >= 48 and b <= 57)         -- 0-9
        or (b >= 65 and b <= 90)     -- A-Z
        or (b >= 97 and b <= 122)    -- a-z
        or b == 47 or b == 45 or b == 95
        or b == 46 or b == 126 or b == 58 then
      out = out .. ch
    else
      out = out .. string.format("%%%02X", b)
    end
  end
  return out
end

local function active_buffer_text()
  local b = pmacs.window.buffer()
  if not b then return "" end
  return b:slice(0, b:len())
end

local function buffer_text(buf)
  if not buf then return "" end
  return buf:slice(0, buf:len())
end

-- didChange coalescing (typing perf) -----------------------------------------
--
-- Document sync is full-text, so each `textDocument/didChange` ships
-- the entire buffer. Sending one per keystroke cost three O(file)
-- copies plus an O(file) JSON write to the server pipe *per typed
-- character* — the dominant daemon-side typing cost on large files.
-- The after-edit hook now only bumps the version, marks the cached
-- render families stale (cheap), and records the buffer as dirty;
-- the actual notification ships from the async tick once the buffer
-- has been quiet for DID_CHANGE_QUIET_MS, or unconditionally once
-- the oldest unsent edit is DID_CHANGE_MAX_LAG_MS old (so the server
-- keeps converging during continuous typing). Versions may skip
-- values across a coalesced burst; LSP only requires that they
-- increase. Anything that asks the server about a document flushes
-- it first so no request is answered against stale text.
local DID_CHANGE_QUIET_MS = 75
local DID_CHANGE_MAX_LAG_MS = 400

-- Dirty buffers: key (tostring(buf)) -> {
--   rec      = the attachment record the edits belong to,
--   first_ms = monotonic time of the oldest unsent edit,
--   last_ms  = monotonic time of the newest unsent edit,
-- }
local pending_did_change = {}

-- Forward declaration — defined below (needs helpers that follow);
-- `flush_did_change` re-pulls inlay hints after each coalesced send.
local pull_inlay_hints_quiet
-- Same, for semantic tokens (Arc 1c). They are pull-model too, and
-- nothing was pulling them.
local pull_semantic_tokens_quiet

local function flush_did_change(key)
  local pending = pending_did_change[key]
  if not pending then return end
  pending_did_change[key] = nil
  local rec = pending.rec
  -- The attachment may have been torn down or replaced (server
  -- crash -> re-attach) since the edit was recorded; only the live
  -- record's server should hear about the buffer.
  if attachments[key] ~= rec then return end
  local ok, text = pcall(buffer_text, rec.buffer)
  if not ok then return end
  pcall(pmacs.lsp.did_change, rec.server, rec.uri, rec.version, text)
  -- Inlay hints are pull-model: the store's stale flag (set per edit)
  -- only clears on a fresh `textDocument/inlayHint` response, and the
  -- server never volunteers one. Re-request at flush cadence so
  -- hints come back shortly after each pause instead of staying
  -- suppressed until the next attach/refresh. The request is
  -- supersede-keyed per (server, method, uri), so a burst of flushes
  -- cancels its own predecessors rather than piling up.
  pcall(pull_inlay_hints_quiet, rec)
  -- Semantic tokens are pull-model on exactly the same terms (Arc 1c).
  pcall(pull_semantic_tokens_quiet, rec)
end

local function flush_did_change_for(rec)
  if rec and rec.buffer then flush_did_change(tostring(rec.buffer)) end
end

local function flush_all_did_changes()
  for key in pairs(pending_did_change) do
    flush_did_change(key)
  end
end

local function flush_due_did_changes()
  if next(pending_did_change) == nil then return end
  local now = pmacs.editor.monotonic_ms()
  for key, pending in pairs(pending_did_change) do
    if now - pending.last_ms >= DID_CHANGE_QUIET_MS
        or now - pending.first_ms >= DID_CHANGE_MAX_LAG_MS then
      flush_did_change(key)
    end
  end
end

-- Exposed for tests and for glue that must synchronize the server's
-- document view before an out-of-band operation (e.g. a save hook).
function pmacs.lsp._flush_did_changes()
  flush_all_did_changes()
end

local function document_end_position(text)
  local line, col = 0, 0
  for i = 1, #text do
    if text:byte(i) == 10 then
      line = line + 1
      col = 0
    else
      col = col + 1
    end
  end
  return line, col
end

local function active_buffer_path()
  return pmacs.editor.file_path()
end

local function buffer_language(buf)
  local ok, path = pcall(function() return buf and buf:path() end)
  if not ok or not path then return nil end
  -- Syntax owns the fresh-load inference and pin. LSP retains only its
  -- path-eligibility rule: without a backing path it cannot construct a URI or
  -- project root, even when syntax can infer a grammar from a buffer name.
  return pmacs.parse.buffer_language(buf)
end
-- Public: the pinned per-buffer language. Auto-pairing resolves relevance
-- against the buffer its typed-edit record names — which a context-switching
-- command may have left inactive by callback time — so the parameterized form
-- is the primitive and the active-buffer form delegates.
pmacs.lsp.buffer_language = buffer_language

local function active_buffer_language()
  return buffer_language(pmacs.window.buffer())
end
-- Public: comment-toggle and other language-aware Lua reuse the same pin.
pmacs.lsp.active_buffer_language = active_buffer_language

-- Directory component of a path, or nil if it has none.
local function dir_of(path)
  if not path then return nil end
  return path:match("^(.*)/[^/]*$")
end

-- The project root to send as `rootUri` / `cwd` in `initialize`, for
-- the file at `path`. Without this a server spawned by the auto-attach
-- hook gets the *editor's* cwd as its root (the `build_initialize`
-- fallback), which is wrong for project-model-strict servers: gopls
-- and rust-analyzer only analyze files under the module/workspace at
-- `rootUri`, so opening a file from outside the launch directory
-- yields zero diagnostics/hover/definition. Resolution order:
--   1. explicit `pmacs.lsp.config[language].root` override,
--   2. `pmacs.project.detect` marker walk (go.mod / Cargo.toml /
--      pyproject.toml / package.json / .git …) — the same detector
--      the rest of the editor uses, honoring set_search_boundary,
--   3. the file's own directory (a lone file still gets a sane root
--      rather than leaking the editor cwd).
-- Returns `root, source`, where `source` is "config", "detected", or
-- "fallback" — and nil alongside a nil root. The source matters because
-- only the first two mean a root was actually *found*; `ensure_server`
-- keys server affinity on those and treats the fallback as rootless.
--
-- `config[language].root` may be a `function(path) -> string|nil` as
-- well as a plain string, for languages whose root rule the shared
-- marker walk cannot express (an innermost-wins walk cannot find an
-- *outermost* marker). A resolver that returns nil declines, and
-- resolution falls through to the marker walk.
--
-- **A configured root — string or resolver return — MUST be a canonical
-- absolute path.** The `"detected"` arm is canonicalized for free
-- (`pmacs.project.detect` canonicalizes before walking), but a
-- configured one is fed to `file_uri_for` exactly as written, and the
-- affinity key is that URI. On macOS a resolver returning `/var/…` and
-- a detected `/private/var/…` are different keys for the same
-- directory, which silently yields two servers for one project. There
-- is no Lua-side canonicalizer to normalize this for you.
--
-- Resolver results are memoized per directory, because `ensure_server`
-- resolves the root on the *reuse* path as well as the spawn path — so
-- an unmemoized filesystem-walking resolver would re-walk on every
-- attach rather than once per project. The memo is keyed by the
-- resolver function itself, weakly: replacing `config[lang].root`
-- installs a new key and the old memo is collected, so a swapped
-- resolver can never serve a root the previous one computed.
local root_resolver_memo = setmetatable({}, { __mode = "k" })

local function resolve_root_fn(language, resolver, path)
  local dir = dir_of(path)
  if not dir then return nil end
  local memo = root_resolver_memo[resolver]
  if not memo then
    memo = {}
    root_resolver_memo[resolver] = memo
  end
  local hit = memo[dir]
  -- `false` is the memoized form of "this resolver declined"; nil means
  -- "not yet asked", so the two must stay distinguishable.
  if hit ~= nil then
    return hit or nil
  end
  local ok, resolved = pcall(resolver, path)
  -- COHERENCE §1.2: background wiring must not DISCARD a failure. A
  -- resolver that raises, or that returns something other than a string
  -- or nil, is a config bug — and the memo below would otherwise bury
  -- it permanently for this directory, so it is never observed again.
  -- Returning nil is the documented decline and stays silent.
  local failure
  if not ok then
    failure = "raised: " .. tostring(resolved)
  elseif resolved ~= nil and type(resolved) ~= "string" then
    failure = "returned a " .. type(resolved) .. "; want string or nil"
  end
  if failure then
    local msg = string.format(
      "LSP: %s root resolver for %s %s", language, dir, failure)
    -- Report on the channel that EXISTS. `pmacs.error` is referenced by
    -- fifteen guarded call sites across the runtime and is defined
    -- nowhere in production (only by a test stub in `src/editor.rs`), so
    -- `if pmacs.error then ...` alone would be a sixteenth report that
    -- never fires — the unwired-guard shape, not a fix for it. The
    -- status line is what lsp.lua already uses for every other LSP
    -- error. The `pmacs.error` arm rides along so this upgrades for free
    -- if that channel is ever built.
    --
    -- Both reports are pcall'd: a broken reporting channel must not turn
    -- a declined root into a failed attach.
    pcall(pmacs.editor.set_status, msg)
    if pmacs.error then pcall(pmacs.error, msg) end
    resolved = nil
  end
  if type(resolved) ~= "string" then resolved = nil end
  memo[dir] = resolved or false
  return resolved
end

local function project_root_for(language, path)
  local cfg = pmacs.lsp.config[language]
  local configured = cfg and cfg.root
  -- Truthiness, not `~= nil`: `root = false` has always read as "unset",
  -- and a `false` leaking through as a root would reach `file_uri_for`.
  if configured and type(configured) ~= "function" then
    return configured, "config"
  end
  if not path then return nil, nil end
  if configured then
    local resolved = resolve_root_fn(language, configured, path)
    if resolved then return resolved, "config" end
  end
  local ok, det = pcall(pmacs.project.detect, path)
  if ok and det and det.root then return det.root, "detected" end
  return dir_of(path), "fallback"
end

-- Servers created by the automatic config-driven path. This is the
-- ownership fact a caller-supplied `label` cannot provide: labels are
-- public, unreserved display strings, while entries here are written
-- only after this module itself successfully spawns a server.
local default_servers = {}

local function ensure_server(language, path)
  local cfg = pmacs.lsp.config[language]
  if not cfg or not cfg.command then return nil end
  -- Reuse an existing same-language server *serving the same root*.
  -- One server per project root: `lake serve` is bound to one Lake
  -- package and rust-analyzer/gopls to one workspace, so handing the
  -- second project's files to the first project's server yields
  -- unresolvable imports and empty diagnostics.
  --
  -- The affinity key is the root only when a root was actually FOUND
  -- (config override or marker walk). `project_root_for` never returns
  -- nil for a file that has a path — its last resort is the file's own
  -- directory — so keying on the fallback would give every directory
  -- of loose scratch files its own server, for every language: two
  -- stray .py files in different directories would spawn two pyrights
  -- where today they share one. The fallback therefore keys on nil.
  --
  -- Matching is on the spawned spec's `root_uri`, nil matching nil, so
  -- the fallback spawn must pass `root_uri = nil` for the key and the
  -- stored spec to agree. `cwd` still carries the directory and
  -- `build_initialize` derives the identical `rootUri` from it when the
  -- field is None (src/lsp.rs), so the initialize payload is unchanged
  -- for that case — only what this loop matches on changes.
  --
  -- Consequence, deliberate: a server hand-spawned from `init.lua` with
  -- only `cwd` set also reads back nil, so a root-bearing attach will
  -- not adopt it. We cannot know which root it was meant to serve, and
  -- guessing wrongly routes a project's files to the wrong server.
  local root, source = project_root_for(language, path)
  local key_uri = nil
  if source == "config" or source == "detected" then
    key_uri = file_uri_for(root)
  end
  for _, info in ipairs(pmacs.lsp.list()) do
    if info.language_id == language and info.state
        and info.root_uri == key_uri then
      local kind = info.state.kind
      if kind ~= "crashed" and kind ~= "stopped" then
        return info.id
      end
    end
  end
  local ok, sid = pcall(pmacs.lsp.spawn, {
    label = "default-" .. language,
    language_id = language,
    command = cfg.command,
    args = cfg.args or {},
    env = cfg.env,
    init_options = cfg.init_options,
    settings = cfg.settings,
    cwd = root,
    root_uri = key_uri,
  })
  if ok then
    default_servers[tostring(sid)] = language
    return sid
  end
  return nil
end

-- Internal ownership seam for builtins whose lifecycle follows the
-- config-driven server set (currently Lean's one-shot fallback). A
-- user-managed server may deliberately use the same language id, label,
-- command, and root; none of those make it ours.
function pmacs.lsp._is_default_server(sid, language)
  local owned_language = default_servers[tostring(sid)]
  return owned_language ~= nil
    and (language == nil or owned_language == language)
end

local function server_state_kind(sid)
  if not sid then return nil end
  for _, info in ipairs(pmacs.lsp.list()) do
    if tostring(info.id) == tostring(sid) then
      return info.state and info.state.kind
    end
  end
  return nil
end

-- True iff `sid` is still registered with the manager and isn't dead.
-- Stale attachments — left behind by a server that crashed, was
-- forgotten, or was spawned against a now-replaced `pmacs.lsp.config`
-- entry — get rebuilt on the next attach attempt.
local function server_is_live(sid)
  local kind = server_state_kind(sid)
  return kind ~= nil and kind ~= "crashed" and kind ~= "stopped"
end

local function server_is_initialized(sid)
  local ok, state = pcall(pmacs.lsp.status, sid)
  return ok and state and state.kind == "initialized"
end

local function server_supports_inlay_hints(sid)
  local ok, caps = pcall(pmacs.lsp.capabilities, sid)
  if not ok or not caps then return false end
  return caps.inlayHintProvider ~= nil and caps.inlayHintProvider ~= false
end

-- Assigns the forward-declared local above (so `flush_did_change`
-- can re-pull); a fresh `local function` here would shadow it.
function pull_inlay_hints_quiet(rec)
  if not rec or not server_is_initialized(rec.server) then return end
  if not server_supports_inlay_hints(rec.server) then return end
  -- The server must see the current text before being asked to
  -- compute positions against it (didChange is debounced). A no-op
  -- when called from `flush_did_change` itself (the pending entry is
  -- removed before the send), so this cannot recurse.
  flush_did_change_for(rec)
  local end_line, end_col = document_end_position(buffer_text(rec.buffer))
  pmacs.async(function()
    pcall(function()
      pmacs.lsp.request_inlay_hint(
        rec.server, rec.uri, 0, 0, end_line, end_col):await()
    end)
  end)
end

-- LSP defines `semanticTokensProvider.full` and `.range` as optional,
-- INDEPENDENT capabilities: a provider may be range-only, and sending
-- it /full gets a rejection the pull path would swallow. Gate each
-- request kind on its own capability.
local function semantic_provider(sid)
  local ok, caps = pcall(pmacs.lsp.capabilities, sid)
  if not ok or not caps then return nil end
  local p = caps.semanticTokensProvider
  if p == nil or p == false then return nil end
  return p
end

local function server_supports_semantic_full(sid)
  local p = semantic_provider(sid)
  if type(p) ~= "table" then return false end
  return p.full == true or type(p.full) == "table"
end

local function server_supports_semantic_range(sid)
  local p = semantic_provider(sid)
  if type(p) ~= "table" then return false end
  return p.range == true or type(p.range) == "table"
end

-- Arc 1c. Semantic tokens are pull-model, exactly like inlay hints: the
-- server never volunteers them, and the store only fills from a
-- `textDocument/semanticTokens/*` response. Until now the ONLY automatic
-- pull was in reply to a server-initiated `workspace/semanticTokens
-- /refresh` --- which most servers never send --- so semantic styling
-- silently never appeared unless the user ran `M-x lsp.semantic-tokens`
-- by hand. Attach and edit-flush now pull it, the same two points that
-- already pull inlay hints.
--
-- Assigns the forward-declared local above (a fresh `local function`
-- here would shadow it, leaving `flush_did_change`'s upvalue nil).
-- Whether the server negotiated DELTA semantic-token support:
-- `semanticTokensProvider.full` must be a table with `delta == true`.
-- Holding a `resultId` does NOT imply delta capability — servers may
-- return one from /full regardless — and a conforming full-only server
-- rejects /full/delta. The pull path swallows request errors, so
-- requesting delta without the capability would leave styling silently
-- stale after the first edit.
local function server_supports_semantic_delta(sid)
  local p = semantic_provider(sid)
  if type(p) ~= "table" then return false end
  return type(p.full) == "table" and p.full.delta == true
end

function pull_semantic_tokens_quiet(rec)
  if not rec or not server_is_initialized(rec.server) then return end
  local has_full = server_supports_semantic_full(rec.server)
  local has_range = server_supports_semantic_range(rec.server)
  if not has_full and not has_range then return end
  -- The server must see the current text before computing token
  -- positions against it. A no-op when called from `flush_did_change`
  -- itself (the pending entry is removed before the send).
  flush_did_change_for(rec)
  -- /full when negotiated (delta only when full.delta == true and a
  -- prior resultId is held); a RANGE-ONLY provider gets a
  -- whole-document /range request instead — never an unsupported
  -- /full. Never clear the store first: a delta splices against the
  -- retained raw stream.
  local prev = nil
  if has_full and server_supports_semantic_delta(rec.server) then
    prev = pmacs.semantic_tokens.result_id(rec.server, rec.uri)
  end
  local end_line, end_col
  if not has_full then
    end_line, end_col = document_end_position(buffer_text(rec.buffer))
  end
  pmacs.async(function()
    pcall(function()
      if prev then
        pmacs.lsp.request_semantic_tokens_delta(rec.server, rec.uri, prev):await()
      elseif has_full then
        pmacs.lsp.request_semantic_tokens(rec.server, rec.uri):await()
      else
        pmacs.lsp.request_semantic_tokens_range(
          rec.server, rec.uri, 0, 0, end_line, end_col):await()
      end
    end)
  end)
end

-- M_B1: buffers that already had an `LspStyleView` overlay pushed,
-- so the after-load / on-demand attach paths don't stack duplicate
-- overlays. Mirrors `highlighted_buffers` in `syntax.lua`; the entry
-- is keyed by the same `tostring(buf)` the attachments table uses.
local styled_buffers = {}

-- M4.6 (task #23): buffers that already had a `DiagnosticView`
-- overlay pushed. Same dedup discipline as `styled_buffers` —
-- `pmacs.diag._attach_view` stacks a fresh overlay on every call,
-- so the after-load path must gate itself.
local diag_viewed_buffers = {}

local function attach_buffer(buf)
  if not buf then return nil end
  local key = tostring(buf)
  local existing = attachments[key]
  if existing and server_is_live(existing.server) then return existing end
  if existing then
    local kind = server_state_kind(existing.server)
    if kind == "crashed" or kind == "stopped" then
      -- A terminal OnCrash client may still have `next_restart_at`
      -- armed. Spawning beside it creates two same-root servers when
      -- the old id restarts. `forget` is the terminal-state operation:
      -- it removes the client and cancels that pending restart before
      -- the replacement is created.
      pcall(pmacs.lsp.forget, existing.server)
    end
    attachments[key] = nil
    -- Unsent edits targeted the dead attachment; the did_open below
    -- carries the full current text, superseding them.
    pending_did_change[key] = nil
  end
  local language = buffer_language(buf)
  if not language then return nil end
  -- Path resolved before spawn so the server's `rootUri` can be
  -- derived from the file's project (see `project_root_for`).
  local path = active_buffer_path()
  local sid = ensure_server(language, path)
  if not sid then return nil end
  local uri = file_uri_for(path)
  if not uri then return nil end
  local rec = {
    buffer = buf,
    language = language,
    server = sid,
    uri = uri,
    version = 1,
  }
  attachments[key] = rec
  -- did_open is a notification; the manager queues it cleanly even
  -- while the server is in `starting` / `initializing`.
  pcall(pmacs.lsp.did_open, sid, uri, rec.version, active_buffer_text())
  -- M_B3: dual-authority styling. Always push the LSP style overlay
  -- when an LSP server is up — whether or not the buffer has a
  -- bundled tree-sitter grammar. When the grammar exists too,
  -- `SyntaxHighlightView` paints first (lexical: keywords, strings,
  -- operators) and `LspStyleView` paints after (semantic: function /
  -- type / macro / namespace identifiers from clangd's tokens); their
  -- styles compose via `crate::overlay::merge_styles`, so the final
  -- cell carries both authorities' contributions. This replaces
  -- M_B1's policy-A exclusivity, which left grammar-backed languages
  -- (Rust, C, C++) without LSP semantic refinement.
  if not styled_buffers[key] then
    local ok, attached = pcall(pmacs.lsp._attach_style, buf)
    if ok and attached then styled_buffers[key] = true end
  end
  -- M4.6 (task #23): attach the DiagnosticView so the TUI grid
  -- renderer underlines bytes covered by published diagnostics.
  -- Keyed by `uri` to match the diag store; the view re-reads the
  -- store on every render, so no further wiring is needed when
  -- diagnostics update.
  if not diag_viewed_buffers[key] then
    local ok, attached = pcall(pmacs.diag._attach_view, buf, uri)
    if ok and attached then diag_viewed_buffers[key] = true end
  end
  pull_inlay_hints_quiet(rec)
  -- Arc 1c: the LspStyleView was just attached above, but nothing ever
  -- filled the semantic-token store it reads. Pull once on attach.
  pull_semantic_tokens_quiet(rec)
  return rec
end

local function attached_for_active()
  local buf = pmacs.window.buffer()
  if not buf then return nil end
  local key = tostring(buf)
  local rec = attachments[key]
  -- A record whose server is dead is worse than no record: every
  -- command below issues requests against it and gets silence. Rebuild
  -- instead, which is what `attach_buffer` does for a stale attachment
  -- anyway — this just stops the dead record short-circuiting that.
  --
  -- Load-bearing for anything that retires a server out from under open
  -- buffers (Arc 8 Stage 3b's fallback latch retires every Lean server
  -- at once). Buffers in OTHER frontends get no `buffer.after-switch`
  -- in this one, so an eager repair sweep keyed on the ambient active
  -- buffer cannot reach them; healing at the point of USE is
  -- frontend-agnostic, because whichever frontend runs the command is
  -- the active one while it runs.
  if rec and not server_is_live(rec.server) then
    rec = nil
  end
  if rec then
    -- Every interactive command resolves its attachment here before
    -- issuing requests; flushing now means the server answers those
    -- requests against the current text (didChange is debounced).
    flush_did_change(key)
    return rec
  end
  return attach_buffer(buf)
end

-- Internal command-path resolver for builtin request producers outside
-- this module. Unlike `active_attachment` it may replace a dead record;
-- unlike `attachment_for_request` it is called only from an explicit
-- user command, where attach-on-use is the intended policy.
function pmacs.lsp._attachment_for_command()
  return attached_for_active()
end

-- Pure, side-effect-free attachment lookup for the active buffer:
-- returns the live record (with `.uri`) when a server is already
-- attached, else nil. Unlike `attached_for_active`, it never *triggers*
-- an attach --- the context menu (Q#CM3) calls it to decide whether to
-- show symbol/diagnostic items, and must not perturb LSP state just by
-- opening.
function pmacs.lsp.active_attachment()
  local buf = pmacs.window.buffer()
  if not buf then return nil end
  return attachments[tostring(buf)]
end

-- Re-run the attach for the ACTIVE buffer, rebuilding it against the
-- current `pmacs.lsp.config`.
--
-- Exists for the Arc 8 Stage 3b fallback latch (Q#LN7): after that latch
-- stops a server that failed to start and rewrites `config.lean4`,
-- something has to actually spawn the replacement and re-point the
-- buffer at it. Nothing else does — `attach_buffer` early-returns for a
-- live attachment, and no hook re-fires on a config change, so without
-- this the buffer stays bound to the stopped server and the "fallback"
-- is a config edit with no effect.
--
-- Deliberately keyed on the active buffer, matching `attach_buffer`'s
-- own use of `active_buffer_path()`; it is not a general re-attach for
-- arbitrary buffers and must not be used as one.
function pmacs.lsp._attach_buffer()
  return attach_buffer(pmacs.window.buffer())
end

-- Arc 4 stage 3: pure modeline projection.  This reads the private
-- per-buffer attachment map directly so passive split windows report their
-- own buffer instead of the focused window.  It never attaches, flushes
-- didChange, or issues a request.
pmacs.statusline.register {
  name = "lsp",
  side = "right",
  priority = 0,
  face = "ui.modeline.lsp",
  fn = function(ctx)
    local rec = attachments[tostring(ctx.buffer)]
    if not rec then return nil end
    return "LSP:" .. pmacs.lsp.modeline_label(rec.server)
  end,
}

-- Flushing variant for request-issuing callers outside this file
-- (Q#C8): when the active buffer already has a server attached,
-- flush any debounced didChange first and return the record, so the
-- caller's request is answered against the current text. Unlike the
-- local `attached_for_active`, this NEVER triggers an attach: the
-- in-buffer completion driver calls it on ordinary typing, and
-- spawning language servers as a typing side effect is wrong (and,
-- concretely, wedged the m4 suite with per-keystroke spawn attempts
-- across parallel tests). Attachment remains buffer-open policy.
function pmacs.lsp.attachment_for_request()
  local buf = pmacs.window.buffer()
  if not buf then return nil end
  local key = tostring(buf)
  local rec = attachments[key]
  if not rec then return nil end
  -- Same liveness rule as `attached_for_active`: a record naming a dead
  -- server is worse than none, because the caller issues a request
  -- against it and waits for a reply that cannot come. Unlike that
  -- function this one is deliberately non-attaching (it must not
  -- perturb LSP state), so a dead record reads as "no attachment"
  -- rather than triggering a rebuild.
  if not server_is_live(rec.server) then
    -- Preserve the record. A crashed OnCrash server may restart under
    -- the SAME id; clearing the map here would orphan that recovered
    -- server, while this non-attaching lookup has no authority to
    -- cancel the restart or create a replacement.
    return nil
  end
  flush_did_change(key)
  return rec
end

-- Hooks --------------------------------------------------------------------

pmacs.hook.add("buffer.after-load", function()
  pcall(attach_buffer, pmacs.window.buffer())
end)

pmacs.hook.add("buffer.after-switch", function()
  -- Arc 1b: switching buffers clears the window's overlays, and
  -- `attach_buffer` early-returns for a live attachment without
  -- touching views — so a switch back to an attached buffer must
  -- re-push the LSP style + diagnostic views itself. The just-
  -- cleared window makes this exactly-once per switch; the dedup
  -- tables keep gating the after-load path only. Without this,
  -- navigating between attached buffers looked like "the LSP
  -- deactivated" (no semantic color, no underlines).
  local buf = pmacs.window.buffer()
  if not buf then return end
  local key = tostring(buf)
  local rec = attachments[key]
  if not rec then return end
  local ok_s, attached_s = pcall(pmacs.lsp._attach_style, buf)
  if ok_s and attached_s then styled_buffers[key] = true end
  local ok_d, attached_d = pcall(pmacs.diag._attach_view, buf, rec.uri)
  if ok_d and attached_d then diag_viewed_buffers[key] = true end
end)

-- Arc 1d: signature-help auto-trigger ----------------------------------
--
-- A *typed character* is recognized by the input-origin signal, not by
-- cursor-delta inference: inside `buffer.after-edit`,
-- `pmacs.editor.this_command() == "buffer.self-insert"` names an edit
-- produced by typing — on either frontend (the daemon classifies
-- single-codepoint optimistic inserts the same way), per-frontend (no
-- cross-frontend misclassification), with no prior-edit snapshot (the
-- first character typed in a buffer triggers). Paste, undo, kill,
-- pointer, and every other input leave `this_command` as something
-- else — a one-byte paste of "(" can never trigger.

-- The last full UTF-8 codepoint ending at `cursor`, as a string. LSP
-- trigger characters are strings, not ASCII bytes, so this must be
-- codepoint-aware: read up to 4 bytes back and take the suffix from
-- the last non-continuation byte.
local function char_before(buf, cursor)
  if cursor <= 0 then return nil end
  local from = cursor - 4
  if from < 0 then from = 0 end
  local ok, s = pcall(function() return buf:slice(from, cursor) end)
  if not ok or type(s) ~= "string" or #s == 0 then return nil end
  for i = #s, 1, -1 do
    local b = s:byte(i)
    if b < 0x80 or b >= 0xC0 then return s:sub(i) end
  end
  return nil
end

-- The set of characters that should (re)open signature help, as the
-- server declares them. `retriggerCharacters` (usually `,`) refreshes an
-- open call's active parameter. A provider that declares neither still
-- gets the universal pair, which is what `(` auto-trigger means in
-- practice; no provider means no auto-trigger at all.
local function signature_trigger_chars(sid)
  local ok, caps = pcall(pmacs.lsp.capabilities, sid)
  if not ok or not caps then return nil end
  local p = caps.signatureHelpProvider
  if not p or p == false then return nil end
  local chars = {}
  for _, c in ipairs(p.triggerCharacters or {}) do chars[c] = true end
  for _, c in ipairs(p.retriggerCharacters or {}) do chars[c] = true end
  if next(chars) == nil then
    chars["("] = true
    chars[","] = true
  end
  return chars
end

-- Like `pmacs.lsp.signature_help_at_cursor`, but silent: an auto-trigger
-- that announced "no signature help" on every `(` in a comment would be
-- unusable. Only a real signature reaches the status line.
local function signature_help_quiet(rec)
  if not server_is_initialized(rec.server) then return end
  -- The server must see the character we just typed before it can tell
  -- us which parameter we are inside of.
  flush_did_change_for(rec)
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.signature.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok = pcall(function()
      pmacs.lsp.request_signature_help(rec.server, rec.uri, line, col):await()
    end)
    if not ok then return end
    local help = pmacs.signature.current(rec.server, rec.uri)
    if not help or not help.signatures or #help.signatures == 0 then return end
    local active = help.signatures[(help.active_signature or 0) + 1]
    if active and active.label then
      pmacs.editor.set_status("LSP: " .. active.label)
    end
  end)
end

pmacs.hook.add("buffer.after-edit", function()
  local buf = pmacs.window.buffer()
  if not buf then return end
  local key = tostring(buf)
  local rec = attachments[key]
  if not rec then return end
  rec.version = rec.version + 1
  -- Stale suppression must stay keystroke-accurate even though the
  -- O(file) didChange send below is coalesced: render families
  -- anchored to pre-edit positions are hidden from this edit on.
  pcall(pmacs.lsp._mark_document_stale, rec.server, rec.uri)
  -- Arc 1d: was this edit a typed character? The input-origin signal
  -- (see the trigger block below).
  local typed = pmacs.editor.this_command
    and pmacs.editor.this_command() == "buffer.self-insert"
  local now = pmacs.editor.monotonic_ms()
  local pending = pending_did_change[key]
  if pending and pending.rec == rec then
    pending.last_ms = now
  else
    pending_did_change[key] = { rec = rec, first_ms = now, last_ms = now }
  end
  -- Fire *after* queuing the pending didChange: `signature_help_quiet`
  -- flushes it, so the server sees the character we are asking about.
  if not typed then return end
  local ch = char_before(buf, pmacs.editor.cursor())
  if not ch then return end
  local triggers = signature_trigger_chars(rec.server)
  if not (triggers and triggers[ch]) then return end
  pcall(signature_help_quiet, rec)
end)

-- Async request surface (T M4.5 async bridge). The Rust manager
-- registers each `textDocument/*` request with the async runtime and
-- returns a job id; the JSON-RPC response (or a server-teardown
-- drain) settles it. `_request_*_raw` is the raw job-id-returning
-- binding (mirrors `pmacs.mcp._send_request_raw`); the wrappers below
-- hand back a `pmacs.workers` Handle whose `:await()` resumes the
-- caller when the response lands. The pre-v1.0 `poll_until` tick-loop
-- this replaced blocked the editor for the whole request; awaiting
-- yields the coroutine instead.
local workers_mod = pmacs.workers
assert(workers_mod and workers_mod._new_handle,
  "pmacs.workers._new_handle missing; did async.lua load before lsp.lua?")
assert(pmacs.lsp._request_completion_raw,
  "pmacs.lsp._request_completion_raw missing; lua_bindings::install_lsp not run?")
local new_handle = workers_mod._new_handle

local function wrap_request(raw)
  -- Raises on dispatch failure (e.g. "server not ready"), matching
  -- `mcp.lua`'s wrapper; callers pcall the `request():await()` chain.
  return function(...)
    return new_handle(raw(...))
  end
end

pmacs.lsp.request_completion = wrap_request(pmacs.lsp._request_completion_raw)
pmacs.lsp.request_hover = wrap_request(pmacs.lsp._request_hover_raw)
pmacs.lsp.request_signature_help = wrap_request(pmacs.lsp._request_signature_help_raw)
pmacs.lsp.request_definition = wrap_request(pmacs.lsp._request_definition_raw)
pmacs.lsp.request_formatting = wrap_request(pmacs.lsp._request_formatting_raw)
pmacs.lsp.request_references = wrap_request(pmacs.lsp._request_references_raw)
pmacs.lsp.request_declaration = wrap_request(pmacs.lsp._request_declaration_raw)
pmacs.lsp.request_type_definition = wrap_request(pmacs.lsp._request_type_definition_raw)
pmacs.lsp.request_implementation = wrap_request(pmacs.lsp._request_implementation_raw)
pmacs.lsp.request_document_symbol = wrap_request(pmacs.lsp._request_document_symbol_raw)
pmacs.lsp.request_workspace_symbol = wrap_request(pmacs.lsp._request_workspace_symbol_raw)
pmacs.lsp.request_document_highlight = wrap_request(pmacs.lsp._request_document_highlight_raw)
pmacs.lsp.request_rename = wrap_request(pmacs.lsp._request_rename_raw)
pmacs.lsp.request_prepare_rename = wrap_request(pmacs.lsp._request_prepare_rename_raw)
pmacs.lsp.request_code_action = wrap_request(pmacs.lsp._request_code_action_raw)
pmacs.lsp.request_execute_command = wrap_request(pmacs.lsp._request_execute_command_raw)
pmacs.lsp.request_inlay_hint = wrap_request(pmacs.lsp._request_inlay_hint_raw)
pmacs.lsp.request_semantic_tokens = wrap_request(pmacs.lsp._request_semantic_tokens_raw)
pmacs.lsp.request_semantic_tokens_range =
  wrap_request(pmacs.lsp._request_semantic_tokens_range_raw)
pmacs.lsp.request_semantic_tokens_delta =
  wrap_request(pmacs.lsp._request_semantic_tokens_delta_raw)

-- Render an `:await()` failure into a modeline-friendly reason.
-- `Handle:await()` raises `{ tag = "cancelled", ... }` when the
-- server went away mid-request (drain in `lsp.rs`) and
-- `{ tag = "failed", message = ... }` for a JSON-RPC error response;
-- a raw dispatch failure surfaces as a plain string.
local function lsp_await_error(err)
  if type(err) == "table" then
    if err.tag == "cancelled" then
      return "server unavailable (request cancelled)"
    elseif err.tag == "failed" then
      return err.message or "server error"
    end
    return tostring(err.tag or "error")
  end
  return tostring(err)
end

-- Cursor positioning ------------------------------------------------------
--
-- LSP positions are 0-based (line, character). The `character` field
-- crossing the wire is already converted to/from a pmacs byte offset
-- by the transport layer (`PositionEncoding` negotiation +
-- `char_to_byte`/`byte_to_char` in src/lsp.rs), so `col` here is a
-- byte offset, not a UTF-16 unit. The one residual: the walk below
-- steps `col` times with `move_right` (one codepoint per step), which
-- equals the byte offset only for single-byte-per-codepoint text —
-- multi-byte lines land the cursor short. Byte-accurate cursor
-- placement is the remaining position-encoding follow-up.

local function move_active_cursor_to(line, col)
  -- Walk via primitives so all overlay observers see the navigation.
  pmacs.editor.move_line_start()
  -- Move to row 0 first, then step down `line` rows.
  while pmacs.editor.cursor_line() > 0 do
    pmacs.editor.move_up()
  end
  for _ = 1, line do pmacs.editor.move_down() end
  for _ = 1, col do pmacs.editor.move_right() end
end

-- Compute the byte offset of (line, col) within `text` where lines are
-- separated by `\n`. Used by `apply_text_edits` to map LSP coordinates
-- to byte positions on the rope.
local function byte_offset_for(text, line, col)
  if line == 0 then return col end
  local pos = 0
  local current_line = 0
  while current_line < line do
    local nl = text:find("\n", pos + 1, true)
    if not nl then return #text end
    pos = nl
    current_line = current_line + 1
  end
  return pos + col
end

local function apply_text_edits(edits)
  if not edits or #edits == 0 then return 0 end
  local buf = pmacs.window.buffer()
  if not buf then return 0 end
  local text = active_buffer_text()
  -- Resolve every edit against the *original* text, then sort by start
  -- byte descending so each replacement leaves earlier offsets valid.
  local resolved = {}
  for _, e in ipairs(edits) do
    table.insert(resolved, {
      start = byte_offset_for(text, e.start_line, e.start_col),
      stop  = byte_offset_for(text, e.end_line, e.end_col),
      text  = e.new_text,
    })
  end
  table.sort(resolved, function(a, b) return a.start > b.start end)
  for _, e in ipairs(resolved) do
    if e.start == e.stop then
      buf:insert(e.start, e.text)
    elseif e.text == "" then
      buf:delete(e.start, e.stop)
    else
      buf:replace(e.start, e.stop, e.text)
    end
  end
  return #resolved
end

-- T M4.5 L2/L4 — apply a parsed LSP `WorkspaceEdit` given as the
-- ordered op list `pmacs.rename.ops` / `code_action.edit` /
-- `_parse_workspace_edit` hand back: each entry is tagged `op` =
-- "edit" | "create" | "rename" | "delete". Order is the server's and
-- is honoured exactly, because the spec sequences ops (a `create`
-- must precede the `edit` that fills the new file).
--
-- Atomicity: a true cross-buffer/disk transaction is out of scope, so
-- the applier refuses to mutate *anything* unless every URI it
-- touches resolves to a real file path first (`path_for_uri`). An op
-- naming an `untitled:`/non-file document aborts the whole edit
-- cleanly, origin buffer untouched, rather than half-applying.
--
-- Text edits go through `apply_text_edits` (offsets resolved against
-- that buffer's *original* text, applied reverse-start) after
-- `find_or_open` makes the target active. Resource ops go through
-- `pmacs.buffer.apply_resource_op` (filesystem + buffer-registry
-- reconciliation). The buffer the user invoked from is restored last
-- (best-effort: it may itself have been renamed/deleted), on the
-- failure path as well as the success path. Returns
-- `edit_count, file_count, resource_op_count` on success, or
-- `nil, message, applied_op_count, execution_started` if the preflight
-- rejected the edit OR any op failed while executing. No exception
-- escapes this function (Q#RD7) — the three callers all handle
-- `nil, message` already, and a raise reaching them meant an unattended
-- server request went unanswered.
--
-- The third failure value is load-bearing, not decoration: Q#RD3
-- permits partial application, so `applied_op_count > 0` means earlier
-- plan items ARE still applied and no caller may say otherwise.
-- `execution_started` is independently load-bearing: a plan item can
-- mutate before it fails (multiple text edits are sequential, and a
-- resource primitive may have intermediate filesystem effects), so
-- zero fully-applied items does NOT prove that nothing was mutated.

-- True when `a` and `b` name the same path, or one lies beneath the
-- other. Component-aware, like the Rust side's `Path::starts_with`: a
-- raw string prefix would make `/tree` an ancestor of `/tree-sibling`.
--
-- Compare on the buffer registry's lexical canonical form, not the raw
-- decoded URI spelling. `file:///tree/./x` and `file:///tree/x` reach
-- the same filesystem entry; treating them as unrelated would judge a
-- later delete against the initial filesystem and fabricate NotFound
-- for an earlier create. This is deliberately lexical — resolving
-- symlinks would change filesystem identity and fail for a new path.
local function paths_related(a, b)
  a, b = pmacs.path.canonicalize(a), pmacs.path.canonicalize(b)
  if a == b then return true end
  if #a < #b then a, b = b, a end
  return a:sub(1, #b) == b and (b == "/" or a:sub(#b + 1, #b + 1) == "/")
end

local function apply_workspace_edit(ops)
  local plan = {}
  -- Paths an EARLIER op in this same batch creates, renames onto,
  -- renames away from, or removes. A delete whose target is related to
  -- one of them cannot be judged from the filesystem's *initial*
  -- state, which is the only state the plan loop can see.
  --
  -- Why defer rather than simulate. Q#RD3 already calls this check a
  -- FILTER, not a transaction, so declining to judge an op is within
  -- its contract; refusing a legal batch is not. Simulating instead
  -- would mean modelling filesystem presence AND the buffer registry's
  -- path bindings across create/rename/edit — the transaction Q#RD3
  -- declines to build — and a simulation that got it wrong would
  -- produce false `clear` verdicts, which is the dangerous direction.
  -- Deferring only forgoes the early, cheap report; the primitive's
  -- own four-phase guard is untouched and is what actually stands
  -- between a server and unsaved work.
  --
  -- `edit` ops are deliberately NOT in this set. An edit changes no
  -- path's existence; it can only dirty a buffer, i.e. only turn a
  -- plan-time `clear` into a primitive-time refusal. That is the
  -- under-refusal Q#RD3 documents and accepts, and adding edits here
  -- would merely delay a refusal that is already certain.
  local batch_changes = {}
  local function batch_will_change(path)
    for _, other in ipairs(batch_changes) do
      if paths_related(path, other) then return true end
    end
    return false
  end
  for _, op in ipairs(ops or {}) do
    if op.op == "edit" then
      if op.edits and #op.edits > 0 then
        local path = pmacs.lsp.path_for_uri(op.uri)
        if not path then
          return nil, "cannot resolve " .. tostring(op.uri), 0, false
        end
        plan[#plan + 1] = { kind = "edit", path = path, edits = op.edits }
      end
    elseif op.op == "create" then
      local path = pmacs.lsp.path_for_uri(op.uri)
      if not path then
        return nil, "cannot resolve " .. tostring(op.uri), 0, false
      end
      plan[#plan + 1] = {
        kind = "create", path = path,
        overwrite = op.overwrite, ignore_if_exists = op.ignore_if_exists,
      }
      batch_changes[#batch_changes + 1] = path
    elseif op.op == "rename" then
      local from = pmacs.lsp.path_for_uri(op.old_uri)
      local to = pmacs.lsp.path_for_uri(op.new_uri)
      if not from or not to then
        return nil, "cannot resolve rename " ..
          tostring(op.old_uri) .. " -> " .. tostring(op.new_uri), 0, false
      end
      plan[#plan + 1] = {
        kind = "rename", old_path = from, new_path = to,
        overwrite = op.overwrite, ignore_if_exists = op.ignore_if_exists,
      }
      batch_changes[#batch_changes + 1] = from
      batch_changes[#batch_changes + 1] = to
    elseif op.op == "delete" then
      local path = pmacs.lsp.path_for_uri(op.uri)
      if not path then
        return nil, "cannot resolve " .. tostring(op.uri), 0, false
      end
      -- Delete precondition check (Q#RD3). This is a FILTER, not a
      -- transaction. It catches, before anything in the batch is
      -- mutated: a plan-time modified or mid-edit buffer, a known
      -- missing target without `ignore_if_not_exists`, and a stat we
      -- could not answer.
      --
      -- What it deliberately does not catch: `documentChanges` are
      -- sequential, so an earlier text edit can dirty a clean buffer
      -- and an earlier rename can move a modified buffer into a later
      -- delete's subtree, both after this snapshot. The primitive then
      -- refuses mid-batch, leaving earlier ops applied. Reporting that
      -- honestly is Q#RD7's job, not this check's to prevent.
      --
      -- The verdict comes from the same Rust helper the primitive
      -- uses, so the two layers cannot disagree. `no-op` and `clear`
      -- both pass: rejecting `no-op` would refuse an op the primitive
      -- treats as doing nothing.
      --
      -- Skipped entirely when an earlier op in this batch can change
      -- this target (see `batch_changes`). Judging `delete X` against
      -- the initial filesystem when an earlier `create X` or
      -- `rename A -> X` has not run yet reports a NotFound that the
      -- batch itself was about to fix, and refuses a legal edit.
      if not batch_will_change(path) then
        local verdict = pmacs.buffer._delete_verdict {
          path = path,
          recursive = op.recursive,
          ignore_if_not_exists = op.ignore_if_not_exists,
        }
        if verdict.kind == "refuse" then
          return nil, verdict.message, 0, false
        end
      end
      plan[#plan + 1] = {
        kind = "delete", path = path,
        recursive = op.recursive, ignore_if_not_exists = op.ignore_if_not_exists,
      }
      batch_changes[#batch_changes + 1] = path
    end
  end
  if #plan == 0 then return 0, 0, 0 end
  -- G1 — capture the origin BUFFER, not its path. A path captured here
  -- is a plain Lua local, and no amount of reconciliation can reach an
  -- already-captured local: once the batch renames or deletes the active
  -- file, that string names something that is no longer there. The
  -- handle follows a rename for free, because the buffer is what moved.
  --
  -- The framing's G1 described the failure as a "phantom buffer" created
  -- by `resolve_target_buffer`'s NotFound arm. **That is not what
  -- happens on this path, and the wrong explanation is recorded here
  -- rather than left to be rediscovered.** `pmacs.buffer.find_or_open`
  -- calls `crate::file_io::load_file` directly and maps the error, so a
  -- missing path RAISES; the NotFound arm belongs to
  -- `EditorCore::resolve_target_buffer`, which serves
  -- `pmacs.window.display_file` and the startup/daemon target, not this
  -- binding. The real defect is quieter: `restore_origin` runs under a
  -- `pcall`, so the raise is swallowed and the user is left in whatever
  -- buffer the last applied op made active. And when the old path DOES
  -- still resolve -- a batch that deletes and then recreates it -- the
  -- fallback silently opens a file the user asked to delete.
  local origin_buf = pmacs.window.buffer()
  local edit_total, files, res_ops = 0, 0, 0
  -- Plan items fully applied before a failure. Q#RD3 permits partial
  -- application, so this is what stops a caller claiming "nothing was
  -- mutated" when something was.
  local applied_ops = 0
  -- Return the user to where they invoked from. Runs on the FAILURE
  -- path too (Q#RD7): previously this ran only after a successful loop,
  -- so a mid-batch refusal stranded the user in whatever buffer the last
  -- applied op left active.
  --
  -- **No path fallback (G1).** If the origin buffer is gone — the batch
  -- deleted its file and reconciliation killed it — restore NOTHING.
  -- "Return the user somewhere plausible" is not worth re-opening a path
  -- the batch just destroyed, and when that path has been recreated the
  -- fallback would drop the user into a file they asked to delete.
  local function restore_origin()
    if not origin_buf then return end
    pcall(pmacs.window.switch_buffer, origin_buf)
  end
  for _, item in ipairs(plan) do
    local ok, err
    if item.kind == "edit" then
      ok, err = pcall(function()
        pmacs.buffer.find_or_open(item.path)
        edit_total = edit_total + apply_text_edits(item.edits)
        files = files + 1
      end)
    else
      -- Every failure becomes a value. The primitive raises for a
      -- refusal or an I/O error; converting here is what lets all
      -- three callers keep using the existing `nil, message` shape
      -- instead of each growing its own pcall.
      ok, err = pcall(pmacs.buffer.apply_resource_op, item)
      if ok then res_ops = res_ops + 1 end
    end
    if not ok then
      restore_origin()
      return nil, tostring(err), applied_ops, true
    end
    applied_ops = applied_ops + 1
  end
  restore_origin()
  return edit_total, files, res_ops
end

-- Render an `apply_workspace_edit` failure for a human or for a
-- server's `failureReason`. One renderer for both, so the two cannot
-- disagree about what happened.
--
-- Q#RD3 explicitly permits partial application: an earlier text edit
-- can apply and dirty a buffer before a later delete refuses. So
-- "nothing was mutated" is reserved for failures before execution.
-- `applied == 0` after execution began proves only that no whole plan
-- item finished; a multi-edit item or resource primitive can still
-- have changed state before its error.
local function workspace_edit_failure(message, applied, execution_started)
  applied = applied or 0
  if applied > 0 then
    return string.format(
      "failed after %d operation%s completed — those earlier changes remain " ..
      "applied; the failing operation may also have changed state: %s",
      applied, (applied == 1 and "" or "s"), tostring(message))
  end
  if execution_started then
    return "failed during the first operation — it may have changed state " ..
      "before failing: " .. tostring(message)
  end
  return "aborted, nothing was mutated: " .. tostring(message)
end

-- Re-pull a per-`(server, uri)` store for every buffer attached to
-- `sid`. Fire-and-forget: the response absorbs into its store via the
-- request's route, exactly like the explicit command path — no await
-- needed. `request_fn(sid, uri)` issues the re-pull.
local function repull_for_attachments(sid, request_fn)
  for _, rec in pairs(attachments) do
    if rec.server == sid and rec.uri then
      -- Server-initiated repulls (diagnostics refresh, semantic
      -- tokens refresh) must also see the latest text first.
      flush_did_change_for(rec)
      pcall(request_fn, sid, rec.uri, rec)
    end
  end
end

-- T M4.5 — workspace file watching (workspace/didChangeWatchedFiles).
--
-- Servers register watchers dynamically via client/registerCapability.
-- pmacs has no kernel file-watch, so each registration runs a polling
-- snapshot-diff coroutine: walk the base dir into a { relpath = sig }
-- map and, every tick, diff against the previous map to emit per-file
-- created/changed/deleted FileEvents (filtered by the glob and the
-- WatchKind bitmask), batched into one notification. Coarser than an
-- inotify bridge but accurate; a watcher self-cancels when the server
-- dies or the capability is unregistered.

local FILE_WATCH_INTERVAL_MS = 250

-- file_watchers[tostring(sid)][registrationId] = list of watch records
-- ({ cancelled = bool, _sleep = handle? }), one per glob watcher.
local file_watchers = {}

-- WatchKind is a bitmask (Create=1, Change=2, Delete=4); test it
-- arithmetically so this stays valid under luajit (no 5.3 `&`).
local function kind_has(mask, bit)
  return mask % (bit * 2) >= bit
end

-- Expand `{a,b}` alternations into brace-free globs (nested handled
-- by recursing the remainder; unbalanced braces left literal).
local function expand_braces(glob)
  local open = glob:find("{", 1, true)
  if not open then return { glob } end
  local depth, close = 0, nil
  for i = open, #glob do
    local c = glob:sub(i, i)
    if c == "{" then
      depth = depth + 1
    elseif c == "}" then
      depth = depth - 1
      if depth == 0 then
        close = i
        break
      end
    end
  end
  if not close then return { glob } end
  local prefix, body, suffix =
    glob:sub(1, open - 1), glob:sub(open + 1, close - 1), glob:sub(close + 1)
  local parts, d2, start = {}, 0, 1
  for i = 1, #body do
    local c = body:sub(i, i)
    if c == "{" then
      d2 = d2 + 1
    elseif c == "}" then
      d2 = d2 - 1
    elseif c == "," and d2 == 0 then
      parts[#parts + 1] = body:sub(start, i - 1)
      start = i + 1
    end
  end
  parts[#parts + 1] = body:sub(start)
  local out = {}
  for _, alt in ipairs(parts) do
    for _, tail in ipairs(expand_braces(suffix)) do
      out[#out + 1] = prefix .. alt .. tail
    end
  end
  return out
end

-- Translate one brace-free glob to an anchored Lua pattern.
local function glob_one_to_pattern(glob)
  local p, i, n = "^", 1, #glob
  while i <= n do
    local c = glob:sub(i, i)
    if c == "*" then
      if glob:sub(i + 1, i + 1) == "*" then
        -- Lua patterns can't quantify a group, so `**/` (zero+ path
        -- segments) becomes the lazy `.-` (`.` spans `/`); a bare
        -- `**` becomes `.*`.
        if glob:sub(i + 2, i + 2) == "/" then
          p, i = p .. ".-", i + 3
        else
          p, i = p .. ".*", i + 2
        end
      else
        p, i = p .. "[^/]*", i + 1
      end
    elseif c == "?" then
      p, i = p .. "[^/]", i + 1
    elseif c == "[" then
      local j = i + 1
      if glob:sub(j, j) == "!" then j = j + 1 end
      if glob:sub(j, j) == "]" then j = j + 1 end
      while j <= n and glob:sub(j, j) ~= "]" do j = j + 1 end
      local cls = glob:sub(i + 1, j - 1):gsub("^!", "^")
      p, i = p .. "[" .. cls .. "]", j + 1
    else
      if c:match("[%(%)%.%%%+%-%^%$%[%]%*%?]") then
        p = p .. "%" .. c
      else
        p = p .. c
      end
      i = i + 1
    end
  end
  return p .. "$"
end

local function glob_matcher(glob)
  local pats = {}
  for _, g in ipairs(expand_braces(glob)) do
    pats[#pats + 1] = glob_one_to_pattern(g)
  end
  return function(rel)
    for _, pat in ipairs(pats) do
      if rel:match(pat) then return true end
    end
    return false
  end
end

-- Recursively list files under `base` → { relpath = sig }. `sig`
-- folds size+mtime+kind so a content/metadata change flips it.
-- Symlinks are recorded, not traversed (loop-safe). Awaits fs
-- primitives, so call from inside an async coroutine.
local function scan_tree(base, matches)
  local out = {}
  local function walk(dir, rel_prefix)
    local ok, entries = pcall(function()
      return pmacs.fs.read_dir(dir):await()
    end)
    if not ok or not entries then return end
    for _, e in ipairs(entries) do
      local rel = (rel_prefix == "") and e.name or (rel_prefix .. "/" .. e.name)
      if e.kind == "dir" then
        walk(dir .. "/" .. e.name, rel)
      elseif matches(rel) then
        out[rel] = table.concat({
          tostring(e.size), tostring(e.mtime),
          tostring(e.mtime_nsec), tostring(e.kind),
        }, "|")
      end
    end
  end
  walk(base, "")
  return out
end

local FC_CREATED, FC_CHANGED, FC_DELETED = 1, 2, 3

local function start_file_watcher(sid, base, glob, kind_mask, record)
  local matches = glob_matcher(glob)
  pmacs.async(function()
    local prev = scan_tree(base, matches)
    while not record.cancelled and server_is_live(sid) do
      local sh = pmacs.workers.sleep(FILE_WATCH_INTERVAL_MS)
      record._sleep = sh
      pcall(function() sh:await() end)
      record._sleep = nil
      if record.cancelled or not server_is_live(sid) then break end

      local cur = scan_tree(base, matches)
      local changes = {}
      for rel, sig in pairs(cur) do
        local was = prev[rel]
        if was == nil then
          if kind_has(kind_mask, 1) then
            changes[#changes + 1] =
              { uri = file_uri_for(base .. "/" .. rel), type = FC_CREATED }
          end
        elseif was ~= sig and kind_has(kind_mask, 2) then
          changes[#changes + 1] =
            { uri = file_uri_for(base .. "/" .. rel), type = FC_CHANGED }
        end
      end
      for rel in pairs(prev) do
        if cur[rel] == nil and kind_has(kind_mask, 4) then
          changes[#changes + 1] =
            { uri = file_uri_for(base .. "/" .. rel), type = FC_DELETED }
        end
      end
      if #changes > 0 then
        pcall(pmacs.lsp.did_change_watched_files, sid, changes)
      end
      prev = cur
    end
  end)
end

-- Resolve a GlobPattern (string | { baseUri, pattern }) to
-- (base_dir, pattern). A bare string with no base falls back to the
-- directory of an attached file on `sid` (best effort).
local function resolve_watcher(sid, gp)
  if type(gp) == "table" and gp.baseUri then
    return pmacs.lsp.path_for_uri(gp.baseUri), gp.pattern or "**"
  end
  if type(gp) == "string" then
    for _, rec in pairs(attachments) do
      if rec.server == sid and rec.uri then
        local p = pmacs.lsp.path_for_uri(rec.uri)
        local dir = p and p:match("^(.*)/[^/]*$")
        if dir then return dir, gp end
      end
    end
  end
  return nil, nil
end

local function register_file_watchers(sid, registrations)
  local skey = tostring(sid)
  file_watchers[skey] = file_watchers[skey] or {}
  for _, reg in ipairs(registrations or {}) do
    if reg.method == "workspace/didChangeWatchedFiles" then
      local recs = {}
      for _, w in ipairs((reg.registerOptions or {}).watchers or {}) do
        local base, pat = resolve_watcher(sid, w.globPattern)
        if base and pat then
          local r = { cancelled = false }
          recs[#recs + 1] = r
          start_file_watcher(sid, base, pat, w.kind or 7, r)
        end
      end
      file_watchers[skey][reg.id] = recs
    end
  end
end

local function unregister_file_watchers(sid, unregs)
  local byid = file_watchers[tostring(sid)]
  if not byid then return end
  for _, u in ipairs(unregs or {}) do
    if u.method == "workspace/didChangeWatchedFiles" and byid[u.id] then
      for _, r in ipairs(byid[u.id]) do
        r.cancelled = true
        if r._sleep then pcall(function() r._sleep:cancel() end) end
      end
      byid[u.id] = nil
    end
  end
end

-- T M4.5 — server→client request pump.
--
-- Some server→client *requests* are surfaced by the manager as a
-- `request` event on the server's event stream (the same "expose the
-- request to the consumer" path as `workspace/configuration`, minus a
-- built-in answer). Each async tick we drain attachment servers'
-- events and handle:
--
--   * `workspace/applyEdit` (L3) — apply the edit through the shared
--     applier, reply `{ applied }`. After a code action's
--     `executeCommand`, servers (rust-analyzer, gopls, …) deliver the
--     actual change this way.
--   * `workspace/inlayHint/refresh` /
--     `workspace/semanticTokens/refresh` — the server signals its
--     cached hints/tokens are stale; reply `null` and re-pull that
--     family for every attached document so the matching store
--     (`pmacs.inlay_hint` / `pmacs.semantic_tokens`) stays fresh.
--   * `client/registerCapability` / `client/unregisterCapability` —
--     start/stop the file-watch coroutines for any
--     `workspace/didChangeWatchedFiles` registration; reply `null`.
--
-- Only servers in `attachments` are drained, so a test (or package)
-- that owns its own directly-spawned server and reads its events
-- itself is unaffected. Server ids are snapshotted before the loop
-- because `apply_workspace_edit` → `find_or_open` can attach a new
-- buffer mid-iteration (mutating `attachments`).
-- Server-originated notification / response seams (framing Q#LN9) -------
--
-- Before this, `handle_server_requests` handled five `request` methods
-- and `initialized`, and dropped every `notification` and `response` on
-- the floor. Dropping responses made `pmacs.lsp.send_request` a
-- write-only API from Lua: the reply was drained and discarded, so
-- nothing outside Rust's typed stores could ever consume one.
--
-- Both seams route through the *existing* drain. A second
-- `events_take` caller would steal events from this one — `take_events`
-- removes the queue — so any new consumer must extend this loop rather
-- than open its own.
--
-- method -> array of subscriber fns. Persistent; `pmacs.hook` has no
-- `remove` and neither does this, deliberately matching it.
local notification_subs = {}
-- tostring(sid) -> { [request_id] = { fn = fn, attempt = n } }. One-shot.
local pending_responses = {}

local function report_subscriber_error(what, err)
  local msg = string.format("LSP: %s subscriber failed: %s", what,
    tostring(err))
  -- COHERENCE §1.2: a pcall around background wiring must report, not
  -- discard. `pmacs.editor.set_status` is the channel that exists;
  -- `pmacs.error` is referenced by fifteen call sites and defined
  -- nowhere in production, so it rides along rather than standing alone.
  pcall(pmacs.editor.set_status, msg)
  if pmacs.error then pcall(pmacs.error, msg) end
end

-- Current spawn attempt for `sid`, or nil if the manager has forgotten
-- it. A restart reuses the sid but bumps the attempt, which is how a
-- pending one-shot tells "my server is still here" from "my server died
-- and a new generation took its id".
local function server_attempt(sid)
  local skey = tostring(sid)
  for _, info in ipairs(pmacs.lsp.list()) do
    if tostring(info.id) == skey then
      return info.attempt or 0
    end
  end
  return nil
end

-- fn(sid, params); persistent, fires for every server.
function pmacs.lsp.on_notification(method, fn)
  if type(method) ~= "string" or type(fn) ~= "function" then
    error("pmacs.lsp.on_notification(method, fn): want string, function")
  end
  local subs = notification_subs[method]
  if not subs then
    subs = {}
    notification_subs[method] = subs
  end
  subs[#subs + 1] = fn
end

-- fn(result, err); ONE-SHOT, keyed to the exact request.
-- `request_id` is what `pmacs.lsp.send_request` returned.
--
-- **Register only against a server with an attached buffer.** The drain
-- that delivers replies visits only sids present in `attachments`, so a
-- one-shot on an unattached server will not fire on its reply — the
-- reply sits in that server's queue and the handler is invoked only when
-- the purge below decides the server is gone. That is fire-on-death, not
-- fire-on-reply, and it looks exactly like a hung request while
-- debugging. The attach path is the ordinary way to get a sid; a
-- hand-spawned one from `init.lua` is the case to watch.
function pmacs.lsp.on_response(sid, request_id, fn)
  if not sid or type(request_id) ~= "number" or type(fn) ~= "function" then
    error("pmacs.lsp.on_response(sid, request_id, fn): want sid, number, function")
  end
  local skey = tostring(sid)
  local pend = pending_responses[skey]
  if not pend then
    pend = {}
    pending_responses[skey] = pend
  end
  -- The attempt is captured at registration so a restart under the same
  -- sid purges this entry rather than leaving it waiting on a reply the
  -- dead generation was going to send.
  pend[request_id] = { fn = fn, attempt = server_attempt(sid) or 0 }
end

local function dispatch_notification(sid, ev)
  local subs = notification_subs[ev.method]
  if not subs then return end
  -- Length captured up front: a subscriber that registers another one
  -- must not be able to extend the list being walked.
  local n = #subs
  for i = 1, n do
    local ok, err = pcall(subs[i], sid, ev.params)
    if not ok then
      report_subscriber_error("notification " .. tostring(ev.method), err)
    end
  end
end

local function deliver_response(sid, ev)
  local skey = tostring(sid)
  local pend = pending_responses[skey]
  if not pend then return end
  local entry = pend[ev.request_id]
  if not entry then return end
  -- Removed UNCONDITIONALLY, so a handler that raises is still retired
  -- and cannot be invoked a second time by the purge. Removing first is
  -- the defensive order and costs nothing, but it is not what defends
  -- against re-invocation: `pcall` catches the raise either way, so
  -- before-vs-after is unobservable without a re-entrant drain. The
  -- reachable bug is gating removal on a clean return, which acceptance
  -- 32 bites (2 != 1).
  pend[ev.request_id] = nil
  if next(pend) == nil then pending_responses[skey] = nil end
  local ok, err = pcall(entry.fn, ev.result, ev.error)
  if not ok then
    report_subscriber_error("response " .. tostring(ev.method), err)
  end
end

-- Settle every one-shot whose server can no longer answer it.
--
-- Deliberately driven off `pmacs.lsp.list()` and NOT off a death event
-- observed in the drain, because the drain cannot be relied on to reach
-- the server in question: `handle_server_requests` builds its sid list
-- from `attachments`, and a sid leaves that table whenever
-- `attach_buffer` finds it dead and rebuilds the attachment against a
-- fresh server. So the very event that should trigger the purge —
-- `crashed` / `stopped` — is the one most likely to go undrained. A
-- one-shot settled only by the drain would leak exactly when it matters.
--
-- `pmacs.lsp.list()` enumerates the manager directly and is unaffected
-- by attachment bookkeeping, which is what makes it the right authority.
local function purge_dead_pending()
  if next(pending_responses) == nil then return end
  local ok, rows = pcall(pmacs.lsp.list)
  -- A failed enumeration is not evidence that every server died; leaving
  -- the registrations alone is the safe read of "we don't know".
  if not ok or not rows then return end
  local alive = {}
  for _, info in ipairs(rows) do
    local kind = info.state and info.state.kind
    if kind ~= "crashed" and kind ~= "stopped" then
      alive[tostring(info.id)] = info.attempt or 0
    end
  end
  for skey, pend in pairs(pending_responses) do
    local attempt = alive[skey]
    local dead = {}
    for rid, entry in pairs(pend) do
      -- Absent or terminal, or the same sid running a NEW generation:
      -- in every case the request this entry awaits is unanswerable.
      --
      -- The generation half is **defensive and not covered by the
      -- acceptance suite**, stated plainly rather than left to look
      -- tested. Reaching it requires a crash and its restart to both
      -- fall inside a gap with no `_async.tick` — the crash backoff is
      -- 500ms (`src/lsp.rs:1007`), so any tick during that window sees
      -- `crashed` and the absent-or-terminal test above fires first. A
      -- stalled or idle editor can produce such a gap, and then this is
      -- the only thing standing between a one-shot and waiting forever
      -- on a reply the dead generation owed. Every attempt to stage it
      -- deterministically ended up exercising the `crashed` path
      -- instead, so it is kept as insurance and labelled as such.
      if attempt == nil or attempt ~= entry.attempt then
        dead[#dead + 1] = rid
      end
    end
    for _, rid in ipairs(dead) do
      local entry = pend[rid]
      pend[rid] = nil
      local ok_h, err = pcall(entry.fn, nil,
        { message = "server gone before response" })
      if not ok_h then
        report_subscriber_error("response purge", err)
      end
    end
    if next(pend) == nil then pending_responses[skey] = nil end
  end
end

local function handle_server_requests()
  local sids, seen = {}, {}
  for _, rec in pairs(attachments) do
    local sid = rec.server
    if sid then
      local k = tostring(sid)
      if not seen[k] then
        seen[k] = true
        sids[#sids + 1] = sid
      end
    end
  end
  for _, sid in ipairs(sids) do
    local ok, evs = pcall(pmacs.lsp.events_take, sid)
    if ok and evs then
      for _, ev in ipairs(evs) do
        if ev.kind == "request" and ev.method == "workspace/applyEdit" then
          local edit = ev.params and ev.params.edit
          local applied, reason = false, nil
          if edit then
            -- Wrap parse AND apply, not apply alone (Q#RD7).
            -- `_parse_workspace_edit` sits one line above the applier
            -- and is itself fallible, so a parse failure escaped the
            -- old wrap entirely, was swallowed by the
            -- `pcall(handle_server_requests)` at the bottom of this
            -- file, and left the server unanswered — the exact defect
            -- being fixed, one line out of scope. The wrap costs
            -- nothing and makes the boundary uniform regardless of
            -- which call fails.
            local ok, a, b, c, d = pcall(function()
              local parsed = pmacs.lsp._parse_workspace_edit(edit)
              return apply_workspace_edit(parsed.ops)
            end)
            if not ok then
              -- A raise from the parse: nothing in the batch ran.
              reason = workspace_edit_failure(a, 0)
            elseif a then
              applied = true
            else
              -- `c` is the count of plan items already applied; `d`
              -- says execution began at all. The server needs both,
              -- because a failing plan item can itself mutate before
              -- returning an error.
              reason = workspace_edit_failure(b, c, d)
            end
          else
            reason = "missing edit"
          end
          local result = { applied = applied }
          if not applied then
            result.failureReason = tostring(reason)
            -- The durable trace (Q#RD7): one call site, one label,
            -- written at the layer that actually knows the outcome. A
            -- Lua preflight rejection never reaches the Rust
            -- primitive, so logging there would miss the common
            -- unattended case entirely.
            pcall(pmacs.buffer._append_error_record,
              "lsp:workspace/applyEdit", tostring(reason))
          end
          -- Always ATTEMPT a response while the response channel
          -- remains live. `send_response` is itself under an ignored
          -- pcall, so whether it lands is the transport's business and
          -- is not observable from here.
          pcall(pmacs.lsp.send_response, sid, ev.request_id, result)
        elseif ev.kind == "request"
            and ev.method == "workspace/inlayHint/refresh" then
          -- Result is `null` on success per the LSP spec; then
          -- re-pull so the store reflects the server's new state.
          pcall(pmacs.lsp.send_response, sid, ev.request_id, nil)
          repull_for_attachments(sid, function(_, _, rec)
            pull_inlay_hints_quiet(rec)
          end)
        elseif ev.kind == "request"
            and ev.method == "workspace/semanticTokens/refresh" then
          pcall(pmacs.lsp.send_response, sid, ev.request_id, nil)
          repull_for_attachments(sid, pmacs.lsp.request_semantic_tokens)
        elseif ev.kind == "request"
            and ev.method == "client/registerCapability" then
          pcall(pmacs.lsp.send_response, sid, ev.request_id, nil)
          pcall(register_file_watchers, sid,
            ev.params and ev.params.registrations)
        elseif ev.kind == "request"
            and ev.method == "client/unregisterCapability" then
          pcall(pmacs.lsp.send_response, sid, ev.request_id, nil)
          -- LSP spells the field "unregisterations".
          pcall(unregister_file_watchers, sid,
            ev.params and ev.params.unregisterations)
        elseif ev.kind == "notification" then
          dispatch_notification(sid, ev)
        elseif ev.kind == "response" then
          deliver_response(sid, ev)
        elseif ev.kind == "initialized" then
          -- Buffers attach before the server finishes initializing, so
          -- the pulls in `attach_buffer` are no-ops for the FIRST file
          -- (their `server_is_initialized` guard is false). This is the
          -- site that actually lands them. Inlay hints were pulled here;
          -- semantic tokens were not, which is why semantic styling never
          -- appeared on the file that started the server (Arc 1c).
          repull_for_attachments(sid, function(_, _, rec)
            pull_inlay_hints_quiet(rec)
            pull_semantic_tokens_quiet(rec)
          end)
        end
      end
    end
  end
end

if pmacs._async and pmacs._async.tick then
  local _prior_async_tick = pmacs._async.tick
  pmacs._async.tick = function(...)
    local ret = _prior_async_tick(...)
    pcall(handle_server_requests)
    -- After the drain, so a response delivered this tick settles its
    -- one-shot normally rather than being purged as "server gone" in the
    -- same pass when the server died right after answering.
    pcall(purge_dead_pending)
    pcall(flush_due_did_changes)
    return ret
  end
end

-- Commands ----------------------------------------------------------------

-- Each command captures the cursor/target at invocation time, then
-- spawns a coroutine that awaits the request and reacts. The editor
-- never blocks: the command function returns immediately and the
-- modeline updates when the response lands (or the await fails).
-- `:await()` sequences the work and surfaces server-gone / server-
-- error as structured errors; the normalized typed store (hybrid
-- model) is still the read path, so LSP result-shape variance
-- (Location | Location[] | LocationLink[], MarkupContent, …) stays
-- parsed in one place in Rust rather than re-derived here.

function pmacs.lsp.go_to_definition()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.definition.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_definition(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local locs = pmacs.definition.locations(rec.server, rec.uri)
    if not locs or #locs == 0 then
      pmacs.editor.set_status("LSP: no definition found")
      return
    end
    local first = locs[1]
    if first.uri == rec.uri then
      -- Same file: record the origin so M-, returns here, then move.
      pmacs.editor.push_jump()
      move_active_cursor_to(first.line, first.col)
      pmacs.editor.set_status(string.format(
        "LSP: definition at %d:%d", first.line + 1, first.col + 1))
    else
      -- Cross-file (SP-4): decode the URI, record the jump origin
      -- *before* switching away, open-or-reuse the target buffer,
      -- then position the cursor. `find_or_open` switches the active
      -- buffer and fires `buffer.after-load`, which attaches an LSP
      -- to the newly opened file.
      local path = pmacs.lsp.path_for_uri(first.uri)
      if not path then
        pmacs.editor.set_status(
          "LSP: cannot open non-file definition " .. first.uri)
        return
      end
      pmacs.editor.push_jump()
      -- Bottom-panel arc (Q#BP11b): the target-aware load. `find_or_open`
      -- switches the ACTIVE window, which would replace a focused panel;
      -- `display_file` resolves the DOCUMENT target first and fires the
      -- load/switch hook with that window active.
      local ok2, oerr = pcall(pmacs.window.display_file, path, { select = true })
      if not ok2 then
        -- Open failed: drop the origin we just pushed so M-, isn't
        -- left pointing at a jump that never happened.
        pmacs.editor.jump_back()
        pmacs.editor.set_status(
          "LSP: failed to open " .. path .. ": " .. tostring(oerr))
        return
      end
      move_active_cursor_to(first.line, first.col)
      pmacs.editor.set_status(string.format(
        "LSP: definition at %s:%d:%d",
        path, first.line + 1, first.col + 1))
    end
  end)
end

-- LSP SymbolKind (1..=26) -> short outline tag (Arc 1b phase 2).
local SYMBOL_KIND_TAGS = {
  "file", "module", "namespace", "package", "class", "method",
  "property", "field", "constructor", "enum", "interface", "function",
  "variable", "constant", "string", "number", "boolean", "array",
  "object", "key", "null", "enum-member", "struct", "event",
  "operator", "type-parameter",
}

-- Visit one LSP location (Arc 1b): the SP-4 cross-file template ---
-- jump ring, find-or-open, cursor walk. Same-buffer hits skip the
-- open. Shared by the references panel (and the outline in phase 2).
local function visit_location(loc)
  local path = pmacs.lsp.path_for_uri(loc.uri)
  if not path then
    pmacs.editor.set_status("LSP: cannot decode target uri " .. tostring(loc.uri))
    return
  end
  pmacs.editor.push_jump()
  -- Bottom-panel arc (Q#BP11b): a visit FROM a panel must land in the
  -- document target and leave the panel intact.
  local ok, err = pcall(pmacs.window.display_file, path, { select = true })
  if not ok then
    -- Open failed: drop the origin we just pushed so M-, isn't left
    -- pointing at a jump that never happened.
    pmacs.editor.jump_back()
    pmacs.editor.set_status("LSP: failed to open " .. path .. ": " .. tostring(err))
    return
  end
  move_active_cursor_to(loc.line, loc.col)
end

-- Shorten `path` against the project root of `relative_to` (a path in
-- the same project) for panel display; falls back to the full path.
local function display_path(path, relative_to)
  local ok, proj = pcall(pmacs.project.detect, relative_to or path)
  if ok and proj and proj.root then
    local root = proj.root
    if root:sub(-1) ~= "/" then root = root .. "/" end
    if path:sub(1, #root) == root then
      return path:sub(#root + 1)
    end
  end
  return path
end

function pmacs.lsp.find_references()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.references.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_references(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local locs = pmacs.references.locations(rec.server, rec.uri)
    if not locs or #locs == 0 then
      pmacs.editor.set_status("LSP: no references found")
      return
    end
    -- Arc 1b: a browsable *references* panel. RET visits (jump ring
    -- included, so M-, returns); q restores this buffer.
    local here = pmacs.lsp.path_for_uri(rec.uri)
    local rows = {}
    for _, loc in ipairs(locs) do
      local path = pmacs.lsp.path_for_uri(loc.uri) or loc.uri
      rows[#rows + 1] = {
        text = string.format("%s:%d:%d", display_path(path, here), loc.line + 1, loc.col + 1),
        item = loc,
      }
    end
    pmacs.listview.open {
      name = "*references*",
      header = string.format(
        "%d reference%s   RET visit  n/p move  q quit",
        #locs, (#locs == 1 and "" or "s")),
      rows = rows,
      on_visit = visit_location,
    }
    pmacs.editor.set_status(string.format(
      "LSP: %d reference%s", #locs, (#locs == 1 and "" or "s")))
  end)
end

function pmacs.lsp.document_symbols()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.document_symbol.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_document_symbol(rec.server, rec.uri):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local syms = pmacs.document_symbol.symbols(rec.server, rec.uri)
    if not syms or #syms == 0 then
      pmacs.editor.set_status("LSP: no symbols")
      return
    end
    -- Arc 1b phase 2: a browsable *outline* panel. Symbols arrive
    -- FLAT with a `depth` field --- indent, don't recurse. RET
    -- visits (jump ring: M-, returns to the outline row); q restores.
    local source_buf = rec.buffer
    local rows = {}
    for _, sym in ipairs(syms) do
      local tag = SYMBOL_KIND_TAGS[sym.kind] or "symbol"
      rows[#rows + 1] = {
        text = string.format(
          "%s%s  [%s]", string.rep("  ", sym.depth or 0), sym.name, tag),
        item = sym,
      }
    end
    pmacs.listview.open {
      name = "*outline*",
      header = string.format(
        "%d symbol%s   RET visit  n/p move  q quit",
        #syms, (#syms == 1 and "" or "s")),
      rows = rows,
      on_visit = function(sym)
        pmacs.editor.push_jump()
        local okv = pcall(pmacs.window.switch_buffer, source_buf)
        if not okv then
          pmacs.editor.jump_back()
          pmacs.editor.set_status("LSP: outline source buffer is gone")
          return
        end
        move_active_cursor_to(sym.line, sym.col)
      end,
    }
    pmacs.editor.set_status(string.format(
      "LSP: %d symbol%s", #syms, (#syms == 1 and "" or "s")))
  end)
end

-- T M4.5 — inlay hints for the whole buffer. Requests over a range
-- spanning the document, stores the parsed hints, and surfaces a
-- modeline summary (count + first). Inline virtual-text rendering is
-- a separate milestone (the cell-overlay model does not yet reflow
-- real glyphs around inserted columns); a render layer subscribes to
-- the same `pmacs.inlay_hint` store when it lands — same staged
-- approach as the references list / hover panel.
function pmacs.lsp.inlay_hints()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  -- Whole-document range: (0,0) .. exact document end. Some servers,
  -- including rust-analyzer, reject one-past or otherwise over-wide
  -- line numbers instead of clamping.
  local text = active_buffer_text()
  local end_line, end_col = document_end_position(text)
  pmacs.inlay_hint.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_inlay_hint(
        rec.server, rec.uri, 0, 0, end_line, end_col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local hints = pmacs.inlay_hint.hints(rec.server, rec.uri)
    if not hints or #hints == 0 then
      pmacs.editor.set_status("LSP: no inlay hints")
      return
    end
    local first = hints[1]
    pmacs.editor.set_status(string.format(
      "LSP: %d inlay hint%s; first '%s' at %d:%d",
      #hints, (#hints == 1 and "" or "s"),
      first.label, first.line + 1, first.col + 1))
  end)
end

-- T M4.5 — semantic tokens for the whole buffer. Incremental: if a
-- prior result id exists for this buffer, request a
-- `/full/delta` against it (the store keeps the raw int stream to
-- splice on); otherwise a `/full` pull. Either way the store ends
-- with the complete token set + a fresh result id, and a modeline
-- summary (count + first token's type, resolved through the legend)
-- is shown. Data only: wiring LSP tokens into styling (a second
-- authority alongside tree-sitter) is a separate rendering
-- milestone — a render layer subscribes to the same
-- `pmacs.semantic_tokens` store when it lands.
--
-- `pmacs.lsp.request_semantic_tokens_range` is also exposed (no
-- default command) for a future viewport-aware caller: the bundle
-- has no on-screen-range source, so a "range" command here would
-- just duplicate `/full`.
function pmacs.lsp.semantic_tokens()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  -- Don't clear: a delta splices against the retained raw stream.
  -- Same gating as the auto-pull path: /full only when negotiated
  -- (delta only under full.delta); a range-only provider gets a
  -- whole-document /range request.
  local has_full = server_supports_semantic_full(rec.server)
  local has_range = server_supports_semantic_range(rec.server)
  if not has_full and not has_range then
    pmacs.editor.set_status("LSP: server has no semantic-token support")
    return
  end
  local prev = nil
  if has_full and server_supports_semantic_delta(rec.server) then
    prev = pmacs.semantic_tokens.result_id(rec.server, rec.uri)
  end
  local end_line, end_col
  if not has_full then
    end_line, end_col = document_end_position(buffer_text(rec.buffer))
  end
  pmacs.async(function()
    local ok, err = pcall(function()
      if prev then
        pmacs.lsp.request_semantic_tokens_delta(
          rec.server, rec.uri, prev):await()
      elseif has_full then
        pmacs.lsp.request_semantic_tokens(rec.server, rec.uri):await()
      else
        pmacs.lsp.request_semantic_tokens_range(
          rec.server, rec.uri, 0, 0, end_line, end_col):await()
      end
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local toks = pmacs.semantic_tokens.tokens(rec.server, rec.uri)
    if not toks or #toks == 0 then
      pmacs.editor.set_status("LSP: no semantic tokens")
      return
    end
    local first = toks[1]
    -- Resolve the type index through the legend (0-based index ->
    -- 1-based Lua array); fall back to the raw index if no legend.
    local legend = pmacs.semantic_tokens.legend(rec.server)
    local tname = legend and legend.token_types
      and legend.token_types[first.token_type + 1]
      or tostring(first.token_type)
    pmacs.editor.set_status(string.format(
      "LSP: %d semantic token%s%s; first '%s' at %d:%d",
      #toks, (#toks == 1 and "" or "s"),
      (prev and " (delta)" or ""),
      tname, first.line + 1, first.start + 1))
  end)
end

function pmacs.lsp.format_buffer()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.formatting.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_formatting(rec.server, rec.uri, 4, true):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local edits = pmacs.formatting.edits(rec.server, rec.uri)
    if not edits or #edits == 0 then
      pmacs.editor.set_status("LSP: no formatting edits")
      return
    end
    local n = apply_text_edits(edits)
    pmacs.editor.set_status(string.format("LSP: applied %d edits", n))
  end)
end

-- T M4.5 — rename the symbol under the cursor.
--
-- When the server advertises `renameProvider.prepareProvider`, a
-- `textDocument/prepareRename` round-trip runs first: it gates the
-- prompt (a `null` result means "not renameable here" — abort with a
-- status, never open the prompt) and pre-fills the placeholder the
-- server suggests. Servers that don't advertise prepare (or advertise
-- `renameProvider: true`) skip straight to the prompt — the original
-- L2 behavior, unchanged. The cursor position is captured *before*
-- the prompt opens so the request still targets the original symbol
-- even though the minibuffer session moved focus.
function pmacs.lsp.rename()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()

  local function open_prompt(initial)
    pmacs.minibuffer.read {
      prompt = "Rename symbol to: ",
      initial = initial,
      on_cancel = function()
        pmacs.editor.set_status("LSP: rename cancelled")
      end,
      on_accept = function(new_name)
        if not new_name or new_name == "" then
          pmacs.editor.set_status("LSP: rename needs a new name")
          return
        end
        pmacs.rename.clear(rec.server, rec.uri)
        pmacs.async(function()
          local ok, err = pcall(function()
            pmacs.lsp.request_rename(rec.server, rec.uri, line, col, new_name):await()
          end)
          if not ok then
            pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
            return
          end
          local ops = pmacs.rename.ops(rec.server, rec.uri)
          if not ops or #ops == 0 then
            pmacs.editor.set_status("LSP: rename produced no edits")
            return
          end
          local n, files, res, execution_started = apply_workspace_edit(ops)
          if not n then
            -- On failure the second value is the message and the third
            -- is how many plan items already applied. It is NOT always
            -- zero (Q#RD3), so this must not say "nothing was mutated"
            -- unconditionally — that was false in exactly the
            -- edit-then-delete case the framing predicted.
            pmacs.editor.set_status(
              "LSP: rename " ..
              workspace_edit_failure(files, res, execution_started))
            return
          end
          local msg = string.format(
            "LSP: renamed — %d edit%s across %d file%s",
            n, (n == 1 and "" or "s"),
            files, (files == 1 and "" or "s"))
          if res and res > 0 then
            msg = msg .. string.format(
              " (+%d file op%s)", res, (res == 1 and "" or "s"))
          end
          pmacs.editor.set_status(msg)
        end)
      end,
    }
  end

  -- Gate on the server advertising prepareRename. `renameProvider`
  -- is `boolean | { prepareProvider?: boolean }`.
  local caps = pmacs.lsp.capabilities(rec.server)
  local rp = caps and caps.renameProvider
  if not (type(rp) == "table" and rp.prepareProvider == true) then
    open_prompt(nil)
    return
  end

  pmacs.prepare_rename.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_prepare_rename(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local pr = pmacs.prepare_rename.result(rec.server, rec.uri)
    if not pr or not pr.allowed then
      pmacs.editor.set_status("LSP: cannot rename here")
      return
    end
    open_prompt(pr.placeholder)
  end)
end

-- T M4.5 L3 — code actions at the cursor. Requests the actions,
-- then applies the first one: an inline `edit` goes through the
-- shared WorkspaceEdit applier; a `command` is dispatched via
-- `workspace/executeCommand` (after which the server usually drives
-- the change with a server→client `workspace/applyEdit`, handled by
-- the pump installed below). A selection UI over multiple actions is
-- future UX work, like the references list and hover panel — v1
-- acts on the first and reports how many were offered.
-- Apply one code action (Arc 1b phase 2: shared by the direct path
-- and the picker). Runs its WorkspaceEdit inline and/or awaits its
-- executeCommand, then reports what happened. Must run inside a
-- `pmacs.async` coroutine.
local function apply_code_action(rec, act)
  local bits = {}
  if act.has_edit then
    local n, files, res, execution_started = apply_workspace_edit(act.edit)
    if not n then
      -- Same failure shape as the rename caller: `files` is the
      -- message, `res` the applied-op count (Q#RD3 permits partial
      -- application, so it can be non-zero).
      pmacs.editor.set_status(
        "LSP: code action " ..
        workspace_edit_failure(files, res, execution_started))
      return
    end
    local b = string.format("%d edit(s) / %d file(s)", n, files)
    if res and res > 0 then b = b .. string.format(" / %d file op(s)", res) end
    table.insert(bits, b)
  end
  if act.command then
    local ok, cerr = pcall(function()
      pmacs.lsp.request_execute_command(
        rec.server, act.command.command, act.command.arguments):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: command failed: " .. lsp_await_error(cerr))
      return
    end
    table.insert(bits, "ran '" .. act.command.command .. "'")
  end
  local detail = (#bits > 0) and (" — " .. table.concat(bits, ", ")) or ""
  pmacs.editor.set_status(string.format(
    "LSP: code action '%s'%s", act.title, detail))
end

function pmacs.lsp.code_actions()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.code_action.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_code_action(
        rec.server, rec.uri, line, col, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local acts = pmacs.code_action.actions(rec.server, rec.uri)
    if not acts or #acts == 0 then
      pmacs.editor.set_status("LSP: no code actions")
      return
    end
    -- Arc 1b phase 2: one action applies directly (today's behavior,
    -- now correct instead of lucky); several open the minibuffer
    -- dropdown so the USER picks — v1 applied acts[1] blind.
    if #acts == 1 then
      apply_code_action(rec, acts[1])
      return
    end
    local labels = {}
    for i, a in ipairs(acts) do
      labels[i] = string.format("%d: %s", i, a.title)
    end
    pmacs.minibuffer.read {
      prompt = string.format("Code action (%d): ", #acts),
      source = function() return labels end,
      on_accept = function(choice)
        if not choice or choice == "" then return end
        -- Accept both the completed candidate ("2: Inline fix")
        -- and a bare typed index ("2").
        local idx = tonumber(choice:match("^(%d+)"))
        local act = idx and acts[idx]
        if not act then
          pmacs.editor.set_status("LSP: no such code action")
          return
        end
        pmacs.async(function()
          apply_code_action(rec, act)
        end)
      end,
    }
  end)
end

function pmacs.lsp.hover_at_cursor()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.hover.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_hover(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local hover = pmacs.hover.current(rec.server, rec.uri)
    if not hover then
      pmacs.editor.set_status("LSP: no hover info")
      return
    end
    -- Surface the first line of the hover body in the modeline. The
    -- popup view subscribes to the same store; a panel can wire in
    -- here when the keybinding is meant to surface one.
    local first = (hover.contents or ""):match("^[^\n]*") or ""
    pmacs.editor.set_status(first ~= "" and ("LSP: " .. first) or "LSP: hover empty")
  end)
end

-- Arc 1b phase 2: the full (multi-line) hover body in a *lsp-help*
-- panel --- `lsp.hover` keeps its one-line echo-area summary; this is
-- the "show me everything" companion. Rows are non-visitable
-- (item = nil, so RET is a no-op); q restores the source buffer.
function pmacs.lsp.hover_doc()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.hover.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_hover(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local hover = pmacs.hover.current(rec.server, rec.uri)
    if not hover or not hover.contents or hover.contents == "" then
      pmacs.editor.set_status("LSP: no hover info")
      return
    end
    local rows = {}
    for l in (hover.contents .. "\n"):gmatch("(.-)\n") do
      rows[#rows + 1] = { text = l }
    end
    pmacs.listview.open {
      name = "*lsp-help*",
      header = "hover documentation   q quit",
      rows = rows,
    }
  end)
end

function pmacs.lsp.signature_help_at_cursor()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.signature.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_signature_help(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local help = pmacs.signature.current(rec.server, rec.uri)
    if not help or not help.signatures or #help.signatures == 0 then
      pmacs.editor.set_status("LSP: no signature help")
      return
    end
    local active = help.signatures[(help.active_signature or 0) + 1]
    pmacs.editor.set_status(active and ("LSP: " .. active.label) or "LSP: signature unknown")
  end)
end

-- Default commands + keymap entries --------------------------------------

pmacs.command.define {
  name = "lsp.go-to-definition",
  description = "Jump to the definition of the symbol under the cursor (LSP).",
  fn = pmacs.lsp.go_to_definition,
}

pmacs.command.define {
  name = "lsp.format-buffer",
  description = "Format the active buffer through the attached LSP server.",
  fn = pmacs.lsp.format_buffer,
}

pmacs.command.define {
  name = "lsp.hover-doc",
  description = "Show the full hover documentation in a *lsp-help* panel.",
  fn = pmacs.lsp.hover_doc,
}

pmacs.command.define {
  name = "lsp.hover",
  description = "Surface the hover documentation for the symbol under the cursor.",
  fn = pmacs.lsp.hover_at_cursor,
}

pmacs.command.define {
  name = "lsp.signature-help",
  description = "Surface the signature of the function call at the cursor.",
  fn = pmacs.lsp.signature_help_at_cursor,
}

pmacs.command.define {
  name = "lsp.find-references",
  description = "Find references to the symbol under the cursor (LSP).",
  fn = pmacs.lsp.find_references,
}

pmacs.command.define {
  name = "lsp.document-symbols",
  description = "List the symbols (outline) of the active buffer (LSP).",
  fn = pmacs.lsp.document_symbols,
}

pmacs.command.define {
  name = "lsp.rename",
  description = "Rename the symbol under the cursor across the workspace (LSP).",
  fn = pmacs.lsp.rename,
}

pmacs.command.define {
  name = "lsp.code-actions",
  description = "Apply a code action for the symbol/range under the cursor (LSP).",
  fn = pmacs.lsp.code_actions,
}

pmacs.command.define {
  name = "lsp.inlay-hints",
  description = "Fetch inlay hints (inferred types / parameter names) for the buffer (LSP).",
  fn = pmacs.lsp.inlay_hints,
}

pmacs.command.define {
  name = "lsp.semantic-tokens",
  description = "Fetch semantic tokens (type-aware classification) for the buffer (LSP).",
  fn = pmacs.lsp.semantic_tokens,
}

-- T M4.5 L1 — unwind the cross-file jump ring. Pairs with the
-- `pmacs.editor.push_jump()` every navigation action records before
-- it moves the cursor.
pmacs.command.define {
  name = "lsp.jump-back",
  description = "Return to the location before the last LSP navigation jump.",
  fn = function()
    if not pmacs.editor.jump_back() then
      pmacs.editor.set_status("LSP: jump ring empty")
    end
  end,
}

-- Default chords. M-. follows the cross-editor convention for
-- go-to-definition; the others sit on `C-c` to keep printable letters
-- self-inserting. The user can override or unbind any of these from
-- init.lua.
pmacs.keymap.bind { scope = "global", sequence = "M-.",   command = "lsp.go-to-definition" }
pmacs.keymap.bind { scope = "global", sequence = "M-?",   command = "lsp.find-references" }
pmacs.keymap.bind { scope = "global", sequence = "M-,",   command = "lsp.jump-back" }
pmacs.keymap.bind { scope = "global", sequence = "C-c o", command = "lsp.document-symbols" }
pmacs.keymap.bind { scope = "global", sequence = "C-c r", command = "lsp.rename" }
pmacs.keymap.bind { scope = "global", sequence = "C-c a", command = "lsp.code-actions" }
pmacs.keymap.bind { scope = "global", sequence = "C-c i", command = "lsp.inlay-hints" }
pmacs.keymap.bind { scope = "global", sequence = "C-c y", command = "lsp.semantic-tokens" }
pmacs.keymap.bind { scope = "global", sequence = "C-c h", command = "lsp.hover" }
pmacs.keymap.bind { scope = "global", sequence = "C-c H", command = "lsp.hover-doc" }
pmacs.keymap.bind { scope = "global", sequence = "C-c s", command = "lsp.signature-help" }
pmacs.keymap.bind { scope = "global", sequence = "C-c f", command = "lsp.format-buffer" }

-- Diagnostic navigation (task #23, M4.6 surface) -----------------------------
--
-- Emacs's `M-g n` / `M-g p` jump between compile/next-error locations. We
-- reuse the chord for LSP diagnostics: walk the diag store for the active
-- buffer's URI and move the cursor to the next/previous diagnostic. Wraps
-- around (`pmacs.diag.next`'s default), so repeated taps cycle.
local function navigate_diagnostic(direction)
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("diag: no LSP server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  local found
  if direction == "next" then
    found = pmacs.diag.next(rec.uri, line, col)
  else
    found = pmacs.diag.previous(rec.uri, line, col)
  end
  if not found then
    pmacs.editor.set_status("diag: no diagnostics in buffer")
    return
  end
  pmacs.editor.push_jump()
  move_active_cursor_to(found.start_line, found.start_col)
  pmacs.editor.set_status(string.format("diag (%s): %s",
    found.severity or "?", found.message or ""))
end

pmacs.command.define {
  name = "diag.next",
  description = "Jump to the next diagnostic in the active buffer (wraps).",
  fn = function() navigate_diagnostic("next") end,
}

pmacs.command.define {
  name = "diag.previous",
  description = "Jump to the previous diagnostic in the active buffer (wraps).",
  fn = function() navigate_diagnostic("previous") end,
}

pmacs.keymap.bind { scope = "global", sequence = "M-g n", command = "diag.next" }
pmacs.keymap.bind { scope = "global", sequence = "M-g p", command = "diag.previous" }

-- Resource reconciliation ---------------------------------------------------
--
-- dired Stage 2a, §5. A rename or delete moves or destroys a path that
-- FOURTEEN URI-keyed store families, the `documents` mirror, the pending
-- response routes and the attached diagnostic overlays are all keyed by.
-- `EditorCore` reconciles the buffer's own path and name; these two
-- subscribers reconcile the LSP layer, which is buffer-keyed here
-- (`rec.uri` is cached per buffer and read at dozens of sites, so ONE
-- rebind reaches all of them) and URI-keyed in Rust.
--
-- These subscribers are independent of every other `resource.renamed`
-- consumer by construction: this one touches URI-keyed state, dired's
-- touches its own handle table, and neither reads what the other wrote.
-- That matters because `all-must-succeed` does NOT abort the fan-out —
-- `run_all_must_succeed` collects each callback's error and continues —
-- so a subscriber may not rely on a raising peer to stop the sequence,
-- and the ordered teardown below is ordered INTERNALLY rather than by
-- registration.

-- Every attachment whose document is `path` or lies beneath it, as
-- `{ key, rec, path }`. Resolved through `path_for_uri` and compared
-- with `paths_related`, so the comparison is component-aware and runs on
-- the same canonical form the buffer registry keys on.
local function attachments_under(path)
  local out = {}
  for key, rec in pairs(attachments) do
    local rec_path = rec.uri and pmacs.lsp.path_for_uri(rec.uri)
    if rec_path and paths_related(rec_path, path) then
      out[#out + 1] = { key = key, rec = rec, path = rec_path }
    end
  end
  return out
end

-- How many attributed failures one status line spells out before
-- collapsing the rest into a count.
local RESOURCE_REPORT_LIMIT = 2

-- A failure collector for a reconciliation fan-out.
--
-- **Why this exists rather than a bare `pcall` per step.** Every step
-- below is fallible for reasons outside this file's control -- a stale
-- server id makes `forget_uri` raise, a stopped server makes `did_close`
-- raise -- and an IGNORED `pcall` makes the hook callback RETURN
-- SUCCESSFULLY. `resource.renamed` and `resource.deleted` are
-- `all-must-succeed`, so the registry's error logger is the mechanism
-- that surfaces a failing subscriber; a callback that swallows its own
-- failures gives that logger nothing to log, and the concrete outcome is
-- silent: `forget_uri` fails, the callback carries on, and the old
-- stores, routes and `documents` entry stay live under a URI the editor
-- no longer holds.
--
-- It must NOT abort the loop. One unreachable server must not leave
-- every other attachment unreconciled, so failures accumulate and are
-- raised once, after every attachment has been processed.
local function failure_sink(hook_name)
  local sink = { hook = hook_name, items = {} }

  -- Run `fn(...)`, and on a raise record it attributed to `what`.
  -- Returns `ok, value` like `pcall`, so a caller can branch.
  function sink:step(what, fn, ...)
    local ok, value = pcall(fn, ...)
    if not ok then
      self.items[#self.items + 1] = string.format("%s: %s", what, tostring(value))
    end
    return ok, value
  end

  -- Report everything collected, on BOTH channels, and raise.
  --
  -- The raise is what the `all-must-succeed` logger needs in order to
  -- write an attributed record to *errors*; the status line is what the
  -- user actually sees, because stale LSP state looks like the editor
  -- quietly breaking. `pmacs.error` is deliberately not used: it is
  -- defined only by a test stub, so writing there would reproduce the
  -- silence this replaces.
  function sink:finish()
    if #self.items == 0 then return end
    local shown, n = {}, #self.items
    for i = 1, math.min(n, RESOURCE_REPORT_LIMIT) do shown[i] = self.items[i] end
    local summary = table.concat(shown, "; ")
    if n > #shown then
      summary = summary .. string.format("; and %d more", n - #shown)
    end
    pcall(pmacs.editor.set_status,
      string.format("LSP %s: %d reconciliation failure%s -- %s",
        self.hook, n, (n == 1 and "" or "s"), summary))
    error(string.format("%s: %s", self.hook, table.concat(self.items, "; ")), 0)
  end

  return sink
end

pmacs.hook.add("resource.renamed", function(old_path, new_path)
  if type(old_path) ~= "string" or type(new_path) ~= "string" then return end
  local sink = failure_sink("resource.renamed")
  for _, hit in ipairs(attachments_under(old_path)) do
    local key, rec, old_uri = hit.key, hit.rec, hit.rec.uri
    -- The buffer's own path was rebound before this hook fired, so ask
    -- it rather than reconstructing the tail ourselves. A buffer that
    -- somehow lost its path (killed, unbound) cannot be re-opened, and
    -- falls through to the teardown-only path below. Not routed through
    -- the sink: a pathless buffer is a legitimate state here, not a
    -- reconciliation failure.
    local ok_path, new_buf_path = pcall(function() return rec.buffer:path() end)
    local new_uri = (ok_path and new_buf_path) and file_uri_for(new_buf_path) or nil

    -- 1. Flush any pending didChange for the OLD uri, so the server is
    --    not left holding an edit it can no longer attribute.
    sink:step("flush didChange for " .. old_uri, flush_did_change_for, rec)
    pending_did_change[key] = nil

    -- 2. didClose the old uri — this removes the open-document
    --    registration and nothing else.
    sink:step("didClose " .. old_uri, pmacs.lsp.did_close, rec.server, old_uri)

    -- 3. Purge the routes, drain their awaiters, and clear all fourteen
    --    stores plus `documents` for the old key. Runs against the OLD
    --    server, which matters when step 4 picks a different one.
    --    A failure here is the one that most needs reporting: the
    --    callback would otherwise continue with the old stores, routes
    --    and `documents` entry all still live.
    sink:step("forget_uri " .. old_uri, pmacs.lsp.forget_uri, rec.server, old_uri)

    if not new_uri then
      attachments[key] = nil
      styled_buffers[key] = nil
      diag_viewed_buffers[key] = nil
    else
      -- 4. Re-run ensure_server. Server affinity keys on the detected
      --    project root, so a rename ACROSS roots needs a different
      --    server; a same-root rename reuses the existing one.
      local ok_sid, sid = sink:step("ensure_server for " .. new_buf_path,
        ensure_server, rec.language, new_buf_path)
      if not (ok_sid and sid) then
        attachments[key] = nil
        styled_buffers[key] = nil
        diag_viewed_buffers[key] = nil
      else
        -- 5. didOpen the new uri with the buffer's current text and a
        --    fresh version. This also reclaims the tombstone for
        --    exactly (server, new uri).
        rec.server = sid
        rec.uri = new_uri
        rec.version = 1
        local ok_text, text = sink:step("read " .. new_uri, buffer_text, rec.buffer)
        sink:step("didOpen " .. new_uri, pmacs.lsp.did_open,
          sid, new_uri, rec.version, ok_text and text or "")
        -- 6. Re-root the diagnostic overlays. `DiagnosticView.uri` is
        --    set once at construction and is private, so this is the
        --    only way to move it — and the sweep reaches PASSIVE
        --    windows, which the attach path cannot, while preserving
        --    each overlay's position in the composition order.
        sink:step("re-root diagnostics to " .. new_uri,
          pmacs.diag._rename_resource, old_uri, new_uri)
      end
    end
  end
  -- Raised only after EVERY attachment has been processed: one
  -- unreachable server must not leave the rest unreconciled.
  sink:finish()
end)

pmacs.hook.add("resource.deleted", function(path)
  if type(path) ~= "string" then return end
  local sink = failure_sink("resource.deleted")
  for _, hit in ipairs(attachments_under(path)) do
    local key, rec = hit.key, hit.rec
    -- No flush: the document is gone, and shipping a didChange for a
    -- file the server can no longer read buys nothing.
    pending_did_change[key] = nil
    sink:step("didClose " .. rec.uri, pmacs.lsp.did_close, rec.server, rec.uri)
    sink:step("forget_uri " .. rec.uri, pmacs.lsp.forget_uri, rec.server, rec.uri)
    -- Drop the record unconditionally, INCLUDING after a failure above.
    -- The buffer may be gone entirely (an unmodified visited file is
    -- killed), in which case a retained record is a dangling handle that
    -- `repull_for_attachments` would iterate; and a modified buffer kept
    -- alive has no file to analyze until it is saved, which re-attaches
    -- through the ordinary path. Keeping a record whose teardown failed
    -- would be strictly worse than dropping it: the failure is reported
    -- either way, and a retained one is re-swept every refresh.
    attachments[key] = nil
    styled_buffers[key] = nil
    diag_viewed_buffers[key] = nil
  end
  sink:finish()
end)
