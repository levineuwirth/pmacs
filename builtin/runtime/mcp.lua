-- builtin/runtime/mcp.lua --- T M9.1 friendly Lua surface for pmacs.mcp.*
--
-- The Rust binding (`pmacs.mcp._send_request_raw`) returns a raw
-- async-runtime JobId. Package code wants the same dispatch shape
-- as `pmacs.workers.compute_sum`, `pmacs.fs.read_dir`, and friends:
--
--   pmacs.async(function()
--     local result = pmacs.mcp.send_request(server, "ping", {}):await()
--     ...
--   end)
--
-- This file replaces `pmacs.mcp.send_request` with the Handle-
-- returning wrapper. The Rust manager registers each request with
-- the async runtime and settles the job when the JSON-RPC response
-- lands on the supervisor pipe; the Handle's :await() resumes the
-- parked coroutine with the response's `result` value (already
-- translated to a Lua table) — or raises `{ tag = "failed",
-- message = ... }` if the server returned a JSON-RPC error, or
-- `{ tag = "cancelled", id = ... }` if the awaiter cancelled or the
-- server died before responding.

local mcp_mod = pmacs.mcp
assert(mcp_mod, "pmacs.mcp must be installed before mcp.lua loads")
assert(mcp_mod._send_request_raw,
  "pmacs.mcp._send_request_raw missing; lua_bindings::install_mcp not run?")

local workers_mod = pmacs.workers
assert(workers_mod and workers_mod._new_handle,
  "pmacs.workers._new_handle missing; did async.lua load before mcp.lua?")

local raw_send_request = mcp_mod._send_request_raw
local new_handle = workers_mod._new_handle

-- pmacs.mcp.send_request(server, method, params) -> Handle
--
-- `server` is the McpServerIdLua handle returned by pmacs.mcp.spawn.
-- `method` is a string. `params` is an optional table (or nil). The
-- return value is a Handle that completes with the response's
-- `result` table.
function mcp_mod.send_request(server, method, params)
  if type(method) ~= "string" then
    error("pmacs.mcp.send_request: method must be a string, got " .. type(method))
  end
  if params ~= nil and type(params) ~= "table" then
    error("pmacs.mcp.send_request: params must be a table or nil, got " .. type(params))
  end
  local job_id = raw_send_request(server, method, params)
  return new_handle(job_id)
end

-- T M9.2: pmacs.mcp.read_resource(server, uri) -> Handle
--
-- Cache-aware MCP resource fetch. Three observable outcomes, all
-- delivered through the same Handle shape:
--
--   * cache hit: handle settles with the cached result (one tick late)
--   * in-flight coalesce: handle attaches to an existing in-flight
--     request, settles with the same result
--   * cache miss: dispatches a fresh `resources/read`, settles with
--     the response
--
-- `pmacs.mcp.invalidate_resource(server, uri)` (installed directly
-- by Rust as a non-Handle function) drops the cache entry; subsequent
-- read_resource calls re-dispatch.
local raw_read_resource = mcp_mod._read_resource_raw
assert(raw_read_resource,
  "pmacs.mcp._read_resource_raw missing; lua_bindings::install_mcp not run?")

function mcp_mod.read_resource(server, uri)
  if type(uri) ~= "string" then
    error("pmacs.mcp.read_resource: uri must be a string, got " .. type(uri))
  end
  local job_id = raw_read_resource(server, uri)
  return new_handle(job_id)
end

-- T M9.3: pmacs.mcp.invoke_tool(server, name, args) -> Handle
--
-- Tool invocation via JSON-RPC `tools/call`. The handle settles
-- with the response's `result` table (`{ content = [...],
-- isError = false }` shape from the MCP spec) on success, or
-- raises a Lua error on either of two failure paths:
--
--   * JSON-RPC error response from the server: standard async
--     failure path.
--   * MCP "tool errored" success response (`isError: true`):
--     translated by the manager into a Failed outcome with the
--     extracted text content as the message. This is a deliberate
--     API choice (see M9.3 audit) — invoke_tool's contract is
--     "raises Lua errors on tool failure", so callers don't have
--     to write `if r.isError then ... end` boilerplate at every
--     call site.
--
-- Callers needing structured access to the raw `{isError, content}`
-- table use `pmacs.mcp.send_request(server, "tools/call", { name = ...,
-- arguments = ... })` to bypass the translator.
local raw_invoke_tool = mcp_mod._invoke_tool_raw
assert(raw_invoke_tool,
  "pmacs.mcp._invoke_tool_raw missing; lua_bindings::install_mcp not run?")

function mcp_mod.invoke_tool(server, name, args)
  if type(name) ~= "string" then
    error("pmacs.mcp.invoke_tool: name must be a string, got " .. type(name))
  end
  if args ~= nil and type(args) ~= "table" then
    error("pmacs.mcp.invoke_tool: args must be a table or nil, got " .. type(args))
  end
  local job_id = raw_invoke_tool(server, name, args)
  return new_handle(job_id)
end

-- T M9.4: pmacs.mcp.get_prompt(server, name [, args]) -> Handle
--
-- Resolve a prompt template via JSON-RPC `prompts/get`. The handle
-- settles with the response's `result` table:
--
--   { description = "...", messages = [{ role = "user", content = ... }, ...] }
--
-- on success, or raises a Lua error (via the async runtime's
-- `tag = "failed"` path) on JSON-RPC errors — including the
-- "missing required argument" case (`-32602`) which is how MCP
-- servers report missing args.
--
-- Unlike `invoke_tool`, there is no `isError`-style translator —
-- `prompts/get` has no semantic-failure path in the MCP spec; either
-- the prompt resolves (success) or the protocol-level call fails
-- (Lua error).
--
-- `args` may be omitted, nil, or an empty table — all three send
-- `arguments: {}` on the wire (the MCP spec requires the field).
local raw_get_prompt = mcp_mod._get_prompt_raw
assert(raw_get_prompt,
  "pmacs.mcp._get_prompt_raw missing; lua_bindings::install_mcp not run?")

function mcp_mod.get_prompt(server, name, args)
  if type(name) ~= "string" then
    error("pmacs.mcp.get_prompt: name must be a string, got " .. type(name))
  end
  if args ~= nil and type(args) ~= "table" then
    error("pmacs.mcp.get_prompt: args must be a table or nil, got " .. type(args))
  end
  local job_id = raw_get_prompt(server, name, args)
  return new_handle(job_id)
end

-- T M9.5: notification dispatcher.
--
-- pmacs.mcp.on_notification(method, fn) -> token
--
-- Register `fn` to be called whenever an MCP server emits a
-- notification with the given method. Multiple handlers per method
-- are allowed; they fire in registration order. The handler
-- receives `(server, params)` where `server` is an McpServerIdLua
-- and `params` is the notification's params table.
--
-- Returns an opaque token; pass it to `pmacs.mcp.off_notification`
-- to unregister.
--
-- The dispatcher hooks into pmacs._async.tick: on each tick, the
-- Rust manager's drain_notifications API is called, and pending
-- notifications are dispatched to registered handlers. Single
-- per-tick walk regardless of how many packages have registered;
-- M9.5/M9.6/M9.7 all share the mechanism.
local subscribe_raw = mcp_mod._subscribe_notification
local unsubscribe_raw = mcp_mod._unsubscribe_notification
local drain_raw = mcp_mod._drain_notifications
assert(subscribe_raw and unsubscribe_raw and drain_raw,
  "pmacs.mcp._subscribe_notification / _unsubscribe_notification / _drain_notifications missing")

-- Per-method handler list. `_handlers[method]` is a table where
-- entries are `{ token = <unique>, fn = <function> }`. Tokens are
-- monotonic so unregistration is unambiguous.
local _handlers = {}
local _next_token = 1

function mcp_mod.on_notification(method, fn)
  if type(method) ~= "string" then
    error("pmacs.mcp.on_notification: method must be a string, got " .. type(method))
  end
  if type(fn) ~= "function" then
    error("pmacs.mcp.on_notification: fn must be a function, got " .. type(fn))
  end
  local list = _handlers[method]
  if list == nil then
    list = {}
    _handlers[method] = list
    -- First handler for this method — register interest with the
    -- Rust manager so notifications/<method> get queued.
    subscribe_raw(method)
  end
  local token = _next_token
  _next_token = _next_token + 1
  list[#list + 1] = { token = token, fn = fn }
  return token
end

function mcp_mod.off_notification(method, token)
  local list = _handlers[method]
  if list == nil then return end
  for i, entry in ipairs(list) do
    if entry.token == token then
      table.remove(list, i)
      break
    end
  end
  if #list == 0 then
    _handlers[method] = nil
    -- Last handler dropped — tell Rust to stop queuing.
    unsubscribe_raw(method)
  end
end

-- Tick hook: drain queued notifications and dispatch.
local _orig_async_tick = pmacs._async.tick
function pmacs._async.tick()
  _orig_async_tick()
  local drained = drain_raw()
  for method, entries in pairs(drained) do
    local list = _handlers[method]
    if list ~= nil then
      for _, entry in ipairs(entries) do
        for _, handler in ipairs(list) do
          local ok, err = pcall(handler.fn, entry.server, entry.params)
          if not ok and pmacs.error then
            pmacs.error("pmacs.mcp.on_notification(" .. method ..
              ") handler raised: " .. tostring(err))
          end
        end
      end
    end
  end
end
