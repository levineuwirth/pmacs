-- pmacs-mcp-tools/init.lua --- T M9.6 tools-as-commands.
--
-- Public API:
--
--   local mcp_tools = require("pmacs-mcp-tools")
--   mcp_tools.register(server)         -- fetch tools/list, define each as a command
--   mcp_tools.unregister(server)       -- drop the server's commands
--   mcp_tools.commands_for(server)     -- list of registered command names
--   mcp_tools.command_name(label, tool)-- compute the normalized command name
--
-- Spec interpretation (T M9.6):
--
--   The spec phrases registration as "for each connected MCP server,
--   register its tools as Lua commands". The trigger is not specified.
--   This package follows the same explicit-call pattern as
--   pmacs-mcp-resources (T M9.5): the user spawns the server via
--   `pmacs.mcp.spawn`, then calls `mcp_tools.register(server)` once
--   the server is initialized. From there, the package's
--   notifications/tools/list_changed handler keeps the registered
--   commands in sync without further user intervention. The shipped
--   user-facing example that wires this up automatically is M9.8's
--   AI-assistance package; M9.6's contract is the per-server register
--   primitive plus its lifecycle.
--
--   The package currently lives in tests/fixtures/ rather than
--   builtin/packages/ because M9.5/M9.6/M9.7 are deliberately
--   primitives — M9.8 is the milestone that ships a user-installable
--   MCP example. The test fixture exercises the full primitive surface
--   so M9.8 can compose it cleanly.
--
-- The package consumes M9.5's notifications/tools/list_changed
-- dispatcher with zero new pmacs.mcp.* APIs — that's the structural
-- property the M9.5 framing predicted: the second consumer of
-- on_notification lands without forcing further core surface.
--
-- Implementation notes:
--
--   * Command names follow `<server.label>-<tool.name>`, with both
--     halves passed through the same character normalizer. The spec
--     example `filesystem-read-file` reads as "<server name>-<tool>";
--     in pmacs's vocabulary the spawn-time `label` is the server name
--     (see `pmacs.mcp.spawn { label = ... }`), so the two framings line
--     up. Any character outside `[a-zA-Z0-9_.-]` becomes `-`. Both
--     halves are normalized so a label like "my server!" still
--     produces a registry-clean command name; the registry's `define`
--     only validates non-empty, so an unnormalized label would be
--     accepted but produce surprising command palette entries.
--   * Two tools normalizing to the same command name → second
--     registration is skipped with a warning. Silent overwrite would
--     hide a real configuration bug. The same warn-and-skip path
--     handles cross-source collisions (a tool whose normalized name
--     is already owned by a builtin, a user definition, or a
--     different MCP server's tools-as-commands registration). A
--     second MCP server with the same `label` whose tools normalize
--     to names already taken by the first server will see its tools
--     skipped — that's intentional. Disambiguation is the operator's
--     job at spawn time (use distinct labels), not the package's.
--   * describe-command is satisfied by stuffing the tool's schema
--     into the registered command's `description` field; pmacs
--     describe.command surfaces description verbatim.
--   * Required-arg flow: pmacs.minibuffer.read with chained on_accept
--     callbacks. Each accept either kicks off the next prompt or
--     dispatches the tool. Optional args are not prompted in v0.1
--     (spec mandates required only); callers wanting full arg surface
--     use pmacs.mcp.invoke_tool directly. Typed-arg coercion (integer,
--     number, boolean) happens at accept time so MCP servers receive
--     the JSON shape their schema advertises rather than a string.
--     Empty input for a required arg is sent as the empty string and
--     left to the server to validate; v0.1 doesn't second-guess the
--     server's interpretation of "missing" arguments.
--   * Result delivery: pmacs.editor.set_status with first-line
--     truncation; the frontend handles terminal-width clipping.
--     Multi-line / large results are M9.8's result-buffer job; v0.1
--     delivers a "did the call work?" signal.
--   * Reconciliation on list_changed: refetch tools/list, hash each
--     advertised tool, diff against the registered set, register
--     additions, unregister removals, re-register schema changes.
--     The hash is order-sensitive on `inputSchema.required` because
--     prompt order is determined by the captured closure, so a
--     reorder of required-args is a meaningful change that must
--     trigger re-registration.

local M = {}

-- Per-server state, keyed by server.id (the integer from the
-- McpServerIdLua handle). Value is a record:
--   { label = "...", tools = { [tool_name] = { command_name, hash } },
--     in_flight = bool, rerun = bool, cancelled = bool }
-- `tools` keys are the *original* tool names (preserve `/`); the
-- `command_name` is the normalized form actually registered.
-- `in_flight` / `rerun` serialize concurrent reconciles (initial fetch
-- vs. notifications/tools/list_changed firing during it). `cancelled`
-- lets a mid-flight reconcile bail when M.unregister has run.
local _by_server = {}

-- Number of currently-registered servers. Used to off_notification
-- once the count drops to zero, so the package's
-- notifications/tools/list_changed subscription is balanced rather
-- than leaked across register/unregister cycles.
local _registered_count = 0

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

local function server_id(server)
  -- McpServerIdLua exposes :raw() which returns the underlying integer
  -- directly. Earlier drafts parsed digits out of tostring(server),
  -- which silently broke if the Display impl ever changed shape.
  -- Wrapped in pcall so a future runtime that renames/removes :raw()
  -- surfaces as a clean register-time error rather than an opaque
  -- "attempt to call a nil value" mid-coroutine.
  if type(server) ~= "userdata" and type(server) ~= "table" then
    error("pmacs-mcp-tools: server handle must be McpServerIdLua, got "
      .. type(server))
  end
  local ok, raw = pcall(function() return server:raw() end)
  if not ok then
    error("pmacs-mcp-tools: server:raw() failed; runtime contract "
      .. "broken (was the McpServerIdLua API renamed?): " .. tostring(raw))
  end
  return raw
end

local function server_label(server)
  -- The label was set at spawn time and surfaces through
  -- pmacs.mcp.list(). One walk per register() call is fine — server
  -- counts are typically single-digit, and the label is then cached
  -- on `_by_server[sid].label` so reconcile() doesn't rewalk.
  for _, row in ipairs(pmacs.mcp.list()) do
    if row.id == server then
      return row.label or "unnamed"
    end
  end
  return "unnamed"
end

-- Heuristic: does this Lua-error string look like the server has gone
-- away? `pmacs.mcp.invoke_tool` raises `unknown server: <sid>` when
-- the server has been stopped/dropped from the manager, and `server
-- <sid> is not ready for requests` when the state is non-Initialized
-- (Crashed/ShuttingDown/Stopped). Either way the registered commands
-- are stale; trigger a teardown.
local function looks_like_server_gone(err)
  local s = type(err) == "table" and tostring(err.message or "") or tostring(err)
  return s:find("unknown server", 1, true) ~= nil
      or s:find("not ready for requests", 1, true) ~= nil
end

-- Notify the user about a non-fatal anomaly. set_status gives an
-- immediate one-line echo; pmacs.error (when the host has installed
-- it) is the project's persistent log surface — same convention as
-- builtin/runtime/{async,mcp,syntax}.lua. Without the pmacs.error
-- branch, a collision warning could be overwritten by the very next
-- set_status from any source, leaving no trace of the misconfiguration.
local function notify(msg)
  pmacs.editor.set_status(msg)
  if pmacs.error then
    pmacs.error("pmacs-mcp-tools: " .. msg)
  end
end

-- Normalize one character. The allow-list is `[a-zA-Z0-9_.-]`; anything
-- else collapses to `-`. Defensive against unusual MCP tool-name
-- characters (whitespace, unicode punctuation, etc).
local function normalize_char(c)
  if c:match("[%a%d_%.%-]") then return c end
  return "-"
end

function M.command_name(label, tool_name)
  -- Both halves run through normalize_char so unusual server labels
  -- (e.g., "my server!" or "filesystem/v2") produce registry-clean
  -- command names. The Rust CommandRegistry only validates non-empty,
  -- so without label normalization a `pmacs.command.define` call
  -- would happily accept "my server!-echo" — passing M-x completion
  -- but breaking the look-it-up-and-rebind workflow because the name
  -- contains characters the keymap parser doesn't expect.
  local out = ""
  for i = 1, #label do
    out = out .. normalize_char(label:sub(i, i))
  end
  out = out .. "-"
  for i = 1, #tool_name do
    out = out .. normalize_char(tool_name:sub(i, i))
  end
  return out
end

-- Hash a tool definition. Stable identity = name + description +
-- inputSchema shape. Two tools advertised back-to-back with the same
-- shape produce the same hash; mutating any field changes it. Not
-- cryptographic; collision resistance is per-server-per-name only.
--
-- `required` is hashed in document order — make_command_body's prompt
-- closure prompts in that order, so a reorder of `required` (same
-- membership, different sequence) is a meaningful change that must
-- trigger re-registration. An earlier draft sorted a copy of `required`
-- before hashing, which made reorder-only mutations invisible to
-- the diff and left the closure prompting in stale order.
-- `properties` keys are still sorted because the iteration order of a
-- Lua table from a JSON object is implementation-defined; sorting
-- gives a deterministic hash without losing meaningful information.
local function tool_hash(tool)
  local parts = { tool.name or "", tool.description or "" }
  local schema = tool.inputSchema
  if type(schema) == "table" then
    local req = schema.required
    if type(req) == "table" then
      parts[#parts + 1] = "required:" .. table.concat(req, ",")
    end
    local props = schema.properties
    if type(props) == "table" then
      local keys = {}
      for k, _ in pairs(props) do keys[#keys + 1] = k end
      table.sort(keys)
      for _, k in ipairs(keys) do
        local p = props[k]
        local t = (type(p) == "table" and p.type) or ""
        parts[#parts + 1] = k .. ":" .. tostring(t)
      end
    end
  end
  return table.concat(parts, "|")
end

-- ---------------------------------------------------------------------------
-- Schema rendering for describe-command
-- ---------------------------------------------------------------------------

local function render_schema_doc(tool)
  local lines = {}
  local desc = tool.description
  if type(desc) ~= "string" or desc == "" then
    desc = "(no description)"
  end
  lines[#lines + 1] = desc
  local schema = tool.inputSchema
  local props = (type(schema) == "table") and schema.properties or nil
  local required = (type(schema) == "table") and schema.required or nil
  -- Build a set of required names for O(1) lookup.
  local req_set = {}
  if type(required) == "table" then
    for _, name in ipairs(required) do req_set[name] = true end
  end
  if type(props) == "table" and next(props) ~= nil then
    lines[#lines + 1] = ""
    lines[#lines + 1] = "Arguments:"
    -- Stable iteration: required first (in the spec's required order),
    -- then any remaining properties alphabetically.
    local ordered = {}
    if type(required) == "table" then
      for _, name in ipairs(required) do
        if props[name] ~= nil then ordered[#ordered + 1] = name end
      end
    end
    local optional_keys = {}
    for k, _ in pairs(props) do
      if not req_set[k] then optional_keys[#optional_keys + 1] = k end
    end
    table.sort(optional_keys)
    for _, k in ipairs(optional_keys) do ordered[#ordered + 1] = k end

    for _, name in ipairs(ordered) do
      local p = props[name] or {}
      local ty = p.type or "any"
      local d = p.description or ""
      local req_tag = req_set[name] and ", required" or ""
      local suffix = (d ~= "" and (": " .. d)) or ""
      lines[#lines + 1] = "  " .. name .. " (" .. ty .. req_tag .. ")" .. suffix
    end
  end
  return table.concat(lines, "\n")
end

-- ---------------------------------------------------------------------------
-- Result delivery
-- ---------------------------------------------------------------------------

-- Extract the response's first text content and keep just the first
-- line. The first-line clip matters: a multi-line set_status would
-- corrupt the row layout. The *width* clipping is the frontend's job
-- — `emit_status_overlay` already truncates to terminal width
-- (frontend.rs:`status_overlay_truncates_text_to_terminal_width`).
-- Earlier drafts also imposed a hardcoded 80-char "..." cap here,
-- which clipped useful detail on wide terminals (and the package has
-- no width info to do better). The honest fix: emit the full first
-- line, let the frontend truncate at the actual edge.
local function format_status(prefix, text)
  local first_line = text:match("([^\n]*)") or text
  return prefix .. ": " .. first_line
end

local function content_text(response)
  if type(response) ~= "table" then return tostring(response) end
  local content = response.content
  if type(content) ~= "table" then return "" end
  local out = {}
  for _, entry in ipairs(content) do
    if type(entry) == "table" and entry.type == "text" and type(entry.text) == "string" then
      out[#out + 1] = entry.text
    end
  end
  return table.concat(out, "")
end

local function deliver_result(tool_name, response)
  local text = content_text(response)
  if text == "" then text = "(no text content)" end
  pmacs.editor.set_status(format_status("MCP " .. tool_name, text))
end

local function deliver_error(tool_name, err)
  -- The async runtime raises errors as either a string or a table
  -- `{ tag = "...", message = "..." }`. Normalize.
  local msg
  if type(err) == "table" and type(err.message) == "string" then
    msg = err.message
  else
    msg = tostring(err)
  end
  pmacs.editor.set_status(format_status("MCP " .. tool_name .. " error", msg))
end

-- ---------------------------------------------------------------------------
-- Argument prompting
-- ---------------------------------------------------------------------------
--
-- Sequential minibuffer prompts. `required` is the ordered list of
-- arg names; the prompt for arg N's `on_accept` either kicks off
-- prompt N+1 or dispatches the tool with the assembled args.

-- Forward declaration so dispatch can reach the public unregister
-- when it detects the server has gone away mid-flight (issue 5).
local _unregister_for_teardown

local function dispatch(server, tool_name, args)
  pmacs.async(function()
    local ok, response_or_err = pcall(function()
      return pmacs.mcp.invoke_tool(server, tool_name, args):await()
    end)
    if ok then
      deliver_result(tool_name, response_or_err)
    else
      deliver_error(tool_name, response_or_err)
      -- If the failure shape says the server is gone, drop the now-
      -- stale registrations. Without this the registered commands
      -- linger past the server's lifetime, and every subsequent
      -- invocation hits the same dead-server error. The status line
      -- already reflects the underlying failure (deliver_error above);
      -- the teardown is a silent side-effect.
      if looks_like_server_gone(response_or_err) then
        local sid = server_id(server)
        if sid ~= nil and _by_server[sid] ~= nil then
          _unregister_for_teardown(server)
        end
      end
    end
  end)
end

-- Coerce a minibuffer-typed string into the JSON shape the tool's
-- schema declares. v0.1 covers the scalar types (string, integer,
-- number, boolean); compound types (array, object, null) and unknown
-- types fall through as strings so the server can decide. Returns
-- `(coerced_value, nil)` on success or `(nil, err_msg)` on a parse
-- failure so the caller can route the error to the status line and
-- abort the dispatch instead of sending malformed JSON to the server.
local function coerce_arg(value, p)
  local ty = (type(p) == "table") and p.type or nil
  if ty == nil or ty == "string" or ty == "any" then
    return value, nil
  end
  if ty == "integer" then
    local n = tonumber(value)
    if n == nil then
      return nil, string.format("expected integer, got %q", value)
    end
    if n ~= math.floor(n) then
      return nil, string.format("expected integer, got %q", value)
    end
    return math.floor(n), nil
  end
  if ty == "number" then
    local n = tonumber(value)
    if n == nil then
      return nil, string.format("expected number, got %q", value)
    end
    return n, nil
  end
  if ty == "boolean" then
    if value == "true" then return true, nil end
    if value == "false" then return false, nil end
    return nil, string.format("expected true/false, got %q", value)
  end
  -- Unknown type — pass the literal string through. Servers that need
  -- structured input via M-x can prompt callers to use
  -- `pmacs.mcp.invoke_tool` directly instead.
  return value, nil
end

local function prompt_chain(server, tool_name, required, props, args, idx)
  if idx > #required then
    dispatch(server, tool_name, args)
    return
  end
  local arg_name = required[idx]
  local p = (type(props) == "table") and props[arg_name] or nil
  local ty = (type(p) == "table") and (p.type or "any") or "any"
  local prompt = string.format("%s (%s) %s: ", tool_name, ty, arg_name)
  pmacs.minibuffer.read {
    prompt = prompt,
    on_accept = function(value)
      local coerced, err = coerce_arg(value or "", p)
      if err ~= nil then
        pmacs.editor.set_status(string.format(
          "MCP %s arg %s: %s", tool_name, arg_name, err))
        return
      end
      args[arg_name] = coerced
      prompt_chain(server, tool_name, required, props, args, idx + 1)
    end,
    on_cancel = function()
      pmacs.editor.set_status("MCP " .. tool_name .. ": cancelled")
    end,
  }
end

-- ---------------------------------------------------------------------------
-- Command body
-- ---------------------------------------------------------------------------

local function make_command_body(server, tool_name, schema)
  return function()
    local props = (type(schema) == "table") and schema.properties or {}
    local required = (type(schema) == "table") and schema.required or {}
    if type(required) ~= "table" or #required == 0 then
      dispatch(server, tool_name, {})
      return
    end
    prompt_chain(server, tool_name, required, props, {}, 1)
  end
end

-- ---------------------------------------------------------------------------
-- Register / unregister
-- ---------------------------------------------------------------------------

local function fetch_tools(server)
  -- send_request returns a Handle; await inside an async coroutine.
  local response = pmacs.mcp.send_request(server, "tools/list", {}):await()
  local list = (type(response) == "table") and response.tools or nil
  if type(list) ~= "table" then return {} end
  local out = {}
  for _, tool in ipairs(list) do
    if type(tool) == "table" and type(tool.name) == "string" then
      out[#out + 1] = tool
    end
  end
  return out
end

local function register_one(state, server, label, tool)
  local cmd_name = M.command_name(label, tool.name)
  -- Collision check is live against this server's currently-registered
  -- commands. An earlier draft used a precomputed `seen` set, which
  -- broke schema-change reconciliation: unregister_one + register_one
  -- ran inside the same loop iteration, but `seen` was built before
  -- the unregister, so the re-register saw the pending name as a
  -- collision and silently skipped. The live read is the simple
  -- fix — `state.tools` always reflects what's registered now.
  for tname, entry in pairs(state.tools) do
    if entry.command_name == cmd_name and tname ~= tool.name then
      notify(string.format(
        "collision on %q (skipping %q)",
        cmd_name, tool.name))
      return
    end
  end
  -- Cross-source collision: the normalized name is already taken by
  -- something outside this package — a builtin command, a user
  -- definition, or another MCP server's mcp-tools registration. The
  -- in-package collision branch above wouldn't see it (state.tools is
  -- per-server), and pmacs.command.define would raise DuplicateName,
  -- which propagates out of the async coroutine and aborts further
  -- registrations. Skip + warn instead so the rest of the server's
  -- tools still register cleanly.
  if pmacs.command.exists(cmd_name) then
    notify(string.format(
      "command %q already defined (skipping %q)",
      cmd_name, tool.name))
    return
  end
  pmacs.command.define {
    name = cmd_name,
    description = render_schema_doc(tool),
    fn = make_command_body(server, tool.name, tool.inputSchema),
  }
  state.tools[tool.name] = {
    command_name = cmd_name,
    hash = tool_hash(tool),
  }
end

local function unregister_one(state, tool_name)
  local entry = state.tools[tool_name]
  if entry == nil then return end
  pmacs.command.unregister(entry.command_name)
  state.tools[tool_name] = nil
end

-- Apply the fresh tools/list against the per-server state. Same diff
-- shape used by both initial register and notification-driven
-- reconcile — keeping the mutation in one function lets the in-flight
-- guard serialize them.
local function apply_fresh(state, server, fresh)
  local fresh_by_name = {}
  for _, t in ipairs(fresh) do fresh_by_name[t.name] = t end
  local to_drop = {}
  for name, _ in pairs(state.tools) do
    if fresh_by_name[name] == nil then to_drop[#to_drop + 1] = name end
  end
  for _, name in ipairs(to_drop) do unregister_one(state, name) end
  for _, t in ipairs(fresh) do
    local existing = state.tools[t.name]
    if existing == nil then
      register_one(state, server, state.label, t)
    else
      local fresh_hash = tool_hash(t)
      if fresh_hash ~= existing.hash then
        -- Schema changed. Unregister and re-register so the
        -- prompt-flow closure picks up the new required-args list.
        --
        -- Important: the unregister and register MUST stay in the
        -- same synchronous Lua block — register_one's cross-source
        -- guard (`pmacs.command.exists(cmd_name)`) returns true if
        -- the slot is still occupied. If a future refactor inserts
        -- an await between these two lines, the cross-source check
        -- will fire on what is logically the same package's slot
        -- and silently skip the re-registration, leaving the closure
        -- with the stale schema. Keep them adjacent.
        unregister_one(state, t.name)
        register_one(state, server, state.label, t)
      end
    end
  end
end

-- reconcile() is the single mutation entry point: initial register
-- routes through it, and so does every notifications/tools/list_changed
-- arrival. The in-flight guard collapses overlapping calls into one
-- in-flight + at-most-one queued, so a notification arriving during
-- the initial fetch doesn't run two interleaving tools/list coroutines
-- against the same `state.tools`.
local function reconcile(server)
  local sid = server_id(server)
  local state = _by_server[sid]
  if state == nil then return end
  if state.in_flight then
    state.rerun = true
    return
  end
  state.in_flight = true
  state.rerun = false
  pmacs.async(function()
    local ok, fresh_or_err = pcall(fetch_tools, server)
    -- M.unregister may have run while we were awaiting tools/list. If
    -- so, `state` is detached from `_by_server` and any registrations
    -- it tries would resurrect commands that were just torn down.
    if state.cancelled or _by_server[sid] ~= state then
      state.in_flight = false
      return
    end
    if ok then
      apply_fresh(state, server, fresh_or_err)
    elseif looks_like_server_gone(fresh_or_err) then
      -- The server vanished mid-fetch (e.g. crashed between the
      -- initial register and tools/list returning). Drop registrations
      -- so M-x doesn't keep advertising dead commands. Clear the
      -- in_flight flag *before* unregister so the orphaned `state`
      -- doesn't carry a stale "true" — even though `state` is detached
      -- from `_by_server` immediately after, future code that holds a
      -- pre-teardown reference (or future test seams) sees a clean
      -- terminal value.
      state.in_flight = false
      _unregister_for_teardown(server)
      return
    end
    -- else: transient JSON-RPC failure on a still-alive server; leave
    -- the existing registrations in place.
    state.in_flight = false
    if state.rerun and _by_server[sid] == state and not state.cancelled then
      reconcile(server)
    end
  end)
end

-- ---------------------------------------------------------------------------
-- Notification dispatcher (M9.5 second consumer)
-- ---------------------------------------------------------------------------

local _notification_method = "notifications/tools/list_changed"
local _notification_token = nil

local function ensure_notification_handler()
  if _notification_token ~= nil then return end
  _notification_token = pmacs.mcp.on_notification(
    _notification_method,
    function(server, _params)
      reconcile(server)
    end)
end

-- Drop the package's notification subscription once no servers remain
-- registered. Without this, ensure_notification_handler set the token
-- on the first M.register and the package kept consuming list_changed
-- events forever — harmless (reconcile bails when state is nil) but
-- an unbalanced subscribe.
local function release_notification_handler()
  if _notification_token == nil then return end
  if _registered_count > 0 then return end
  pmacs.mcp.off_notification(_notification_method, _notification_token)
  _notification_token = nil
end

-- Test seam (unstable, do not rely from external code).
-- Reports whether the package currently holds a
-- notifications/tools/list_changed subscription. The bool surfaces
-- the lifecycle invariant ("subscribed iff at least one server is
-- registered") to acceptance tests without exposing the token. The
-- underscore prefix marks this as internal — package authors building
-- on top of pmacs-mcp-tools must not depend on this symbol; it can
-- change shape between any two milestones without notice.
function M._has_notification_subscription()
  return _notification_token ~= nil
end

-- ---------------------------------------------------------------------------
-- Public API
-- ---------------------------------------------------------------------------

function M.register(server)
  local sid = server_id(server)
  if sid == nil then
    error("pmacs-mcp-tools.register: server handle has no resolvable id")
  end
  if _by_server[sid] ~= nil then
    -- Idempotent re-register: reconcile against the live tool list.
    reconcile(server)
    return
  end
  ensure_notification_handler()
  local label = server_label(server)
  _by_server[sid] = {
    label = label,
    tools = {},
    in_flight = false,
    rerun = false,
    cancelled = false,
  }
  _registered_count = _registered_count + 1
  -- Initial population goes through reconcile so it shares the
  -- in-flight guard with notification-driven re-fetches. From an
  -- empty state.tools, apply_fresh's diff degenerates to "register
  -- every tool" — same effect as the old explicit loop.
  reconcile(server)
end

function M.unregister(server)
  local sid = server_id(server)
  if sid == nil then return end
  local state = _by_server[sid]
  if state == nil then return end
  -- Mark the state cancelled before any unregister_one runs; an
  -- in-flight reconcile coroutine checks `state.cancelled` after it
  -- awakens from its tools/list await and bails without re-defining
  -- commands we just dropped.
  state.cancelled = true
  for name, _ in pairs(state.tools) do
    pmacs.command.unregister(state.tools[name].command_name)
  end
  _by_server[sid] = nil
  _registered_count = _registered_count - 1
  if _registered_count < 0 then _registered_count = 0 end
  release_notification_handler()
end

-- Non-public alias so dispatch / reconcile can teardown without a
-- forward-declaration dance against M.unregister's later definition.
_unregister_for_teardown = M.unregister

function M.commands_for(server)
  local sid = server_id(server)
  local state = _by_server[sid]
  if state == nil then return {} end
  local out = {}
  for _, entry in pairs(state.tools) do
    out[#out + 1] = entry.command_name
  end
  table.sort(out)
  return out
end

-- Test seam (unstable, do not rely from external code).
-- Renders the documentation string the package would attach to a
-- registered command for `tool` (a tools/list entry shape).
-- Underscore prefix marks it as not a stable user-facing API; M9.6
-- acceptance tests use it to pin the "(no description)" fallback
-- shape without spinning up a server. The format may change between
-- milestones — package authors who need to render schema docs should
-- duplicate the rendering rather than depend on this seam.
function M._render_schema_doc(tool)
  return render_schema_doc(tool)
end

-- Test seam (unstable, do not rely from external code).
-- Computes the same identity hash the reconcile loop uses to detect
-- meaningful schema changes. Acceptance tests use this to pin the
-- "required-arg order is part of the identity" property without
-- driving a live server through change_tool_schema.
function M._tool_hash(tool)
  return tool_hash(tool)
end

return M
