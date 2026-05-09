-- pmacs-mcp-ai/init.lua --- T M9.8 AI-assistance example package.
--
-- Public API:
--
--   local ai = require("pmacs-mcp-ai")
--   ai.configure {
--     server_label = "claude-mcp",         -- the label of an MCP server already
--                                          -- spawned via pmacs.mcp.spawn
--     prompts = {
--       fn      = "review_function",       -- which advertised prompt handles
--                                          -- the function-context flow
--       project = "review_project",        -- ... project-context flow
--       ask     = "ask_freeform",          -- ... freeform-question flow
--     },
--   }
--   ai.unconfigure()                        -- drop commands; no server change
--
-- The package's three commands (defined on first configure):
--
--   ai.ask-about-function   -- selects the enclosing function via tree-sitter
--                              and sends as code-context
--   ai.ask-about-project    -- collects all file-backed buffers and sends
--                              as a structured `files: [{path, content}, ...]`
--                              array (Q5: explicit JSON beats separator
--                              encoding)
--   ai.ask                  -- prompts for a question, sends with no buffer
--                              context
--
-- Architectural commitment (the M9.8 ship gate):
--
--   * Zero direct calls into the Rust core. Everything reaches the
--     Rust side through the public Lua surface (`pmacs.mcp.*`,
--     `pmacs.parse.*`, `pmacs.command.*`, etc.).
--   * Zero model-specific code. The package speaks MCP; the model
--     behind the configured server is interchangeable. The
--     `m9_8_server_pluggability_*` tests pin this by configure'ing
--     two distinct fake servers and verifying the same command
--     routes to whichever is currently configured.
--
-- Composition story:
--
--   * Rendering: composes with `pmacs-mcp-prompts.render(label,
--     prompt_name, response)` (promoted from internal to public on
--     M9.8's request as the second consumer). Result buffers land
--     in the M9.7 `*mcp:<label>:<prompt>*` namespace, so re-invoking
--     the underlying prompt from either path (M9.8's
--     `ai.ask-about-X` or M9.7's auto-registered
--     `<label>-<prompt>`) lands in the same buffer.
--   * Notifications: M9.8 doesn't subscribe directly. The user is
--     expected to also `require("pmacs-mcp-prompts")` and call
--     `register(server)` on their AI server if they want the auto-
--     registered prompt-commands surface. Either layer functions
--     standalone; the AI commands work without M9.7 registration.
--
-- Context selection (v0.1):
--
--   * Function context: walk the buffer's tree-sitter parse view to
--     find the deepest function-shaped node enclosing the cursor.
--     The "function-shaped" mapping is per-language with a generic
--     fallback for grammars without a hand-coded entry. v0.1 ships
--     with rust + lua mappings (matches the M4 builtin grammars).
--   * Project context: all open buffers whose name does NOT start
--     with `*`. The exclusion rule is intentional — anything in
--     `*name*` is a special buffer (REPL, *help*, *mcp:* result
--     buffers, *scratch*, etc.) by convention. Power users wanting
--     custom collection should call `pmacs.mcp.get_prompt` directly.
--   * Freeform: minibuffer-prompted question; no buffer context.
--
-- M9.6+M9.7 audit-finding carry-forward:
--
--   * Server-gone teardown (M9.6 finding 5): `dispatch` detects
--     "unknown server" / "not ready for requests" on get_prompt
--     failure and clears `_config` so subsequent invocations
--     surface the configure-needed message rather than the same
--     dead-server error. The user re-configures (or re-spawns the
--     server with the same label) to recover.
--   * Cross-source collision (M9.6 finding 6): the three command
--     names are namespaced under `ai.*` to minimize collision risk
--     with builtins. `pmacs.command.exists` is checked before
--     defining; on hit, the package warns and skips the colliding
--     command — the rest still register cleanly.
--   * `notify()` helper (M9.6 finding 10): warnings hit both
--     `set_status` and `pmacs.error` so they survive past the
--     next set_status overwrite.
--   * Notification subscription refcount (M9.6 finding 3): n/a —
--     M9.8 doesn't subscribe to notifications directly. M9.7's
--     package handles its own subscription lifecycle if the user
--     also registers it.

local mcp_prompts = require("pmacs-mcp-prompts")

local M = {}

-- Single global config. Re-configure replaces; unconfigure clears.
-- Shape: { server_label, prompts = { fn, project, ask } }
local _config = nil

-- Tracks whether the three commands are currently defined. Re-
-- configure does NOT redefine — the existing command bodies read
-- `_config` lazily, so flipping `server_label` between configure
-- calls reroutes invocations without touching the registry.
local _commands_defined = false

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

local function notify(msg)
  pmacs.editor.set_status(msg)
  if pmacs.error then
    pmacs.error("pmacs-mcp-ai: " .. msg)
  end
end

local function looks_like_server_gone(err)
  local s = type(err) == "table" and tostring(err.message or "") or tostring(err)
  return s:find("unknown server", 1, true) ~= nil
      or s:find("not ready for requests", 1, true) ~= nil
end

-- Resolve the configured server label to a live McpServerIdLua, or
-- return nil + a friendly message. Done at *invocation time*, not
-- configure time, so re-configure is observable on the very next
-- invocation without recomputing anything cached.
local function resolve_server()
  if _config == nil then
    return nil, "ai: not configured (call ai.configure first)"
  end
  for _, row in ipairs(pmacs.mcp.list()) do
    if row.label == _config.server_label then
      return row.id, nil
    end
  end
  return nil, string.format(
    "ai: no MCP server with label %q (spawn first, then configure)",
    _config.server_label)
end

-- ---------------------------------------------------------------------------
-- Tree-sitter context selection
-- ---------------------------------------------------------------------------

-- Per-language mapping of "function-shaped node types". Adding a new
-- language is a one-line addition; languages without a mapping fall
-- through to a generic set that covers most C-family / dynamic-
-- language grammars.
local _FUNCTION_NODE_TYPES = {
  rust = { "function_item" },
  lua  = { "function_declaration", "local_function", "function_definition" },
}

local _GENERIC_FUNCTION_NODE_TYPES = {
  "function_declaration",
  "function_definition",
  "function_item",
  "method_declaration",
  "method_definition",
}

local function function_types_set(language)
  local list = _FUNCTION_NODE_TYPES[language] or _GENERIC_FUNCTION_NODE_TYPES
  local set = {}
  for _, t in ipairs(list) do set[t] = true end
  return set
end

-- Find the deepest node of any type in `type_set` whose byte range
-- contains `byte_pos`. Returns the node or nil. Walks the parse tree
-- depth-first; deeper matches take precedence so a method inside a
-- struct returns the method (not the struct).
--
-- Boundary: tree-sitter `end_byte` is exclusive (`end_byte` is the
-- position just past the node's last byte). The check `byte_pos > eb`
-- — strictly greater than — therefore *includes* `byte_pos == eb`
-- as enclosing. This is deliberate and inclusive at the right edge:
-- a cursor that has just stepped past the closing brace of a function
-- still gets that function as context, which matches the way users
-- think about "I'm working on this function." The trade is that a
-- cursor on the very first byte of a sibling function will return
-- the previous function, since it's `eb` of the previous one *and*
-- `sb` of the next, and depth-first ordering visits the previous
-- one first. Pinned by `m9_8_find_enclosing_at_end_byte_includes_node`.
local function find_enclosing(node, byte_pos, type_set)
  if node == nil then return nil end
  local sb = node:start_byte()
  local eb = node:end_byte()
  if sb == nil or eb == nil then return nil end
  if byte_pos < sb or byte_pos > eb then return nil end
  local children = node:children()
  if type(children) == "table" then
    for _, child in ipairs(children) do
      local found = find_enclosing(child, byte_pos, type_set)
      if found ~= nil then return found end
    end
  end
  if type_set[node:type()] then return node end
  return nil
end

-- Test seam (unstable): returns the function-shaped node enclosing
-- byte_pos in `buf`, plus the language string and a failure-kind
-- string (or nil on success). The third return distinguishes the
-- two failure modes the body callers care about:
--
--   * `"no_tree"` — buffer has no parse view yet (common for
--     `pmacs.buffer.from_bytes` / `pmacs.buffer.create` buffers, or
--     in the brief async-parse window right after a file-open).
--   * `"no_enclosing"` — there is a tree, but no function-shaped
--     node contains the cursor (cursor in a comment, top-level
--     scope, etc.).
--
-- The seam exists so the M9.8 enclosing-walk test can pin the lookup
-- without driving a full M-x → minibuffer → render flow.
function M._find_enclosing_function(buf, byte_pos)
  local tree = pmacs.parse.tree(buf)
  if tree == nil then return nil, nil, "no_tree" end
  local language = tree:language()
  local type_set = function_types_set(language)
  local node = find_enclosing(tree:root(), byte_pos, type_set)
  if node == nil then return nil, language, "no_enclosing" end
  return node, language, nil
end

-- ---------------------------------------------------------------------------
-- Project context selection
-- ---------------------------------------------------------------------------

-- Collect all "user-content" buffers — file-backed or otherwise, but
-- excluding `*<anything>*` star-buffers (REPL, *help*, *mcp:*
-- result buffers, *scratch*, etc.). The shape returned to the wire is
-- `{path, content}` per Q5; `path` is the buffer name (which is the
-- file path for file-backed buffers).
--
-- Soft size guardrail: if the projected payload (sum of paths +
-- contents) exceeds `_PROJECT_PAYLOAD_WARN_BYTES`, surface a notify
-- so the user knows they're about to send (and pay for) a large
-- request. The collection still proceeds — the warning is
-- informational, not a hard cap. Power users wanting a hard limit
-- should call `pmacs.mcp.get_prompt` directly with their own
-- selection.
M._PROJECT_PAYLOAD_WARN_BYTES = 500 * 1024

function M._collect_project_files()
  local out = {}
  local total = 0
  for _, id in ipairs(pmacs.buffer.list()) do
    local d = pmacs.describe.buffer(id)
    if d ~= nil and type(d.name) == "string" and not d.name:match("^%*") then
      local content = id:slice(0, id:len())
      total = total + #content + #d.name
      out[#out + 1] = { path = d.name, content = content }
    end
  end
  if total > M._PROJECT_PAYLOAD_WARN_BYTES then
    notify(string.format(
      "project context is %d bytes (>%d KB warning threshold); proceeding",
      total, math.floor(M._PROJECT_PAYLOAD_WARN_BYTES / 1024)))
  end
  return out
end

-- ---------------------------------------------------------------------------
-- Dispatch
-- ---------------------------------------------------------------------------

local function dispatch(server, server_label, prompt_name, args)
  pmacs.async(function()
    local ok, response_or_err = pcall(function()
      return pmacs.mcp.get_prompt(server, prompt_name, args):await()
    end)
    if ok then
      mcp_prompts.render(server_label, prompt_name, response_or_err)
    else
      local msg
      if type(response_or_err) == "table" and type(response_or_err.message) == "string" then
        msg = response_or_err.message
      else
        msg = tostring(response_or_err)
      end
      pmacs.editor.set_status("ai " .. prompt_name .. " error: " .. msg)
      if looks_like_server_gone(response_or_err) then
        -- The configured server vanished. Clear `_config` so the next
        -- ai.* invocation surfaces the configure-needed message
        -- rather than the same dead-server error on every retry. The
        -- user re-configures (or re-spawns the server with the same
        -- label) to recover. Mirrors M9.6 finding 5 — but since this
        -- package's commands are stable across configure cycles, we
        -- clear the *config* rather than unregistering the commands.
        _config = nil
      end
    end
  end)
end

-- ---------------------------------------------------------------------------
-- Command bodies
-- ---------------------------------------------------------------------------

local function ask_about_function_body()
  local server, err = resolve_server()
  if server == nil then
    pmacs.editor.set_status(err)
    return
  end
  local prompt_name = (_config.prompts or {}).fn
  if type(prompt_name) ~= "string" or prompt_name == "" then
    pmacs.editor.set_status("ai: no `prompts.fn` configured for ask-about-function")
    return
  end
  local buf = pmacs.window.buffer()
  if buf == nil then
    pmacs.editor.set_status("ai: no active buffer")
    return
  end
  local cursor = pmacs.editor.cursor()
  local node, language, fail_kind = M._find_enclosing_function(buf, cursor)
  if fail_kind == "no_tree" then
    pmacs.editor.set_status(
      "ai: buffer not parsed yet (open as a file, or wait for the parse to settle)")
    return
  end
  if node == nil then
    pmacs.editor.set_status("ai: no enclosing function at cursor (place cursor inside a function)")
    return
  end
  local source = node:text()
  local file_path = (pmacs.describe.buffer(buf) or {}).name or "<unnamed>"
  dispatch(server, _config.server_label, prompt_name, {
    language = language or "text",
    file_path = file_path,
    source = source,
  })
end

local function ask_about_project_body()
  local server, err = resolve_server()
  if server == nil then
    pmacs.editor.set_status(err)
    return
  end
  local prompt_name = (_config.prompts or {}).project
  if type(prompt_name) ~= "string" or prompt_name == "" then
    pmacs.editor.set_status("ai: no `prompts.project` configured for ask-about-project")
    return
  end
  local files = M._collect_project_files()
  if #files == 0 then
    pmacs.editor.set_status("ai: no file-backed buffers to send as project context")
    return
  end
  dispatch(server, _config.server_label, prompt_name, { files = files })
end

local function ask_body()
  local server, err = resolve_server()
  if server == nil then
    pmacs.editor.set_status(err)
    return
  end
  local prompt_name = (_config.prompts or {}).ask
  if type(prompt_name) ~= "string" or prompt_name == "" then
    pmacs.editor.set_status("ai: no `prompts.ask` configured for ask")
    return
  end
  pmacs.minibuffer.read {
    prompt = "Ask: ",
    on_accept = function(question)
      if question == nil or question == "" then return end
      dispatch(server, _config.server_label, prompt_name, { question = question })
    end,
    on_cancel = function()
      pmacs.editor.set_status("ai: cancelled")
    end,
  }
end

-- ---------------------------------------------------------------------------
-- Command lifecycle
-- ---------------------------------------------------------------------------

-- Define-once + cross-source-collision skip (M9.6 finding 6
-- carry-forward). Each (name, body) pair is gated on
-- `pmacs.command.exists` so a builtin or user command already owning
-- the slot doesn't abort the whole register. Returns the count of
-- commands actually defined.
local _COMMAND_DEFS = {
  { name = "ai.ask-about-function",
    description = "Send the enclosing function as context to the configured AI server.",
    fn = ask_about_function_body },
  { name = "ai.ask-about-project",
    description = "Send all file-backed buffers as project context to the configured AI server.",
    fn = ask_about_project_body },
  { name = "ai.ask",
    description = "Prompt for a freeform question and send to the configured AI server.",
    fn = ask_body },
}

local function define_commands()
  local defined = 0
  for _, spec in ipairs(_COMMAND_DEFS) do
    if pmacs.command.exists(spec.name) then
      notify(string.format(
        "command %q already defined (skipping)", spec.name))
    else
      pmacs.command.define(spec)
      defined = defined + 1
    end
  end
  return defined
end

local function undefine_commands()
  for _, spec in ipairs(_COMMAND_DEFS) do
    if pmacs.command.exists(spec.name) then
      pmacs.command.unregister(spec.name)
    end
  end
end

-- ---------------------------------------------------------------------------
-- Public API
-- ---------------------------------------------------------------------------

function M.configure(opts)
  if type(opts) ~= "table" then
    error("pmacs-mcp-ai.configure: opts must be a table")
  end
  if type(opts.server_label) ~= "string" or opts.server_label == "" then
    error("pmacs-mcp-ai.configure: opts.server_label must be a non-empty string")
  end
  local prompts = opts.prompts
  if prompts ~= nil and type(prompts) ~= "table" then
    error("pmacs-mcp-ai.configure: opts.prompts must be a table or nil")
  end
  _config = {
    server_label = opts.server_label,
    prompts = prompts or {},
  }
  if not _commands_defined then
    define_commands()
    _commands_defined = true
  end
end

function M.unconfigure()
  _config = nil
  if _commands_defined then
    undefine_commands()
    _commands_defined = false
  end
end

-- Test seam (unstable): returns the current config table or nil.
-- The seam exists so configure / re-configure / unconfigure tests
-- can pin the state transitions without scraping commands_for or
-- pmacs.command.list.
function M._config()
  return _config
end

return M
