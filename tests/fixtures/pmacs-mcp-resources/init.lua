-- pmacs-mcp-resources/init.lua --- T M9.5 resources-as-buffers.
--
-- Public API:
--
--   local mcp_res = require("pmacs-mcp-resources")
--   local buf = mcp_res.open(server, uri)        -- returns BufferIdLua
--   mcp_res.close(buf)
--   mcp_res.is_stale(buf) -> bool                -- buffer hasn't refreshed since server died
--   mcp_res.children(buf) -> { uri1, uri2, ... } -- only meaningful for directory buffers
--
-- Implementation notes:
--
--   * `open` is called from inside a `pmacs.async(function() ... end)`
--     coroutine, since it awaits `pmacs.mcp.read_resource(...)`.
--   * Subscriptions: when `open` runs against a server whose
--     capabilities advertise `resources.subscribe`, send a
--     `resources/subscribe` request and register the (server, uri,
--     buffer) triple in a refresh registry.
--   * Refresh on `notifications/resources/updated`: hooked once at
--     module load via `pmacs.mcp.on_notification`. The handler
--     looks up the buffer by (server, uri) and dispatches a fresh
--     async coroutine to refetch + repaint.
--   * Server lifecycle: when a subscribed buffer's server transitions
--     to a non-Initialized state, the buffer is marked stale.
--     `is_stale(buf)` returns true; the buffer keeps its last-known
--     content. Re-opening the resource after the server returns to
--     Initialized re-establishes the subscription.
--   * The visible buffer is read-only via intercept; package paints
--     bypass with a per-buffer painting flag (M8 CC-1 pattern).

local view = require("pmacs-mcp-resources.view")

local M = {}

-- Per-buffer state. Keyed by `tostring(buf)`, which is stable per
-- underlying BufferId — see `builtin/runtime/syntax.lua`'s
-- `highlighted_buffers` and `pmacs-mcp-prompts`'s `_buffer_state`
-- (M9.7 audit finding) for the same pattern. NOT keyed by the
-- userdata itself: `pmacs.window.buffer()` and `pmacs.buffer.list()`
-- return fresh BufferIdLua wrappings on each call, so a userdata-
-- keyed table only finds the *first* wrapping and silently misses
-- every subsequent lookup. The `pmacs-mcp-resources.open-at-point`
-- command body fetches the active buffer via `pmacs.window.buffer()`
-- and feeds it into `open_child_at_point(buf)`; if the storage table
-- were keyed by the userdata itself, that lookup would miss and RET
-- would be a silent no-op. Pinned by
-- `m9_5_state_lookup_survives_fresh_buffer_userdata`.
--
-- Value is a record:
--   { server = McpServerIdLua, uri = "...", server_raw = u64,
--     painting = bool, stale = bool, children = {...},
--     subscribed = bool, kind = "...", mimeType = "..." }
local _state = {}

local function buffer_key(buf)
  return tostring(buf)
end

-- Reverse lookup: (server_raw, uri) -> buffer handle. Used by the
-- notification handler and the lifecycle observer.
local _by_server_uri = {}

local function reverse_key(server_raw, uri)
  return tostring(server_raw) .. "\0" .. uri
end

-- ---------------------------------------------------------------------------
-- Painting
-- ---------------------------------------------------------------------------

local function paint(buf, text)
  local s = _state[buffer_key(buf)]
  if s == nil then return end
  s.painting = true
  local ok, err = pcall(function()
    buf:replace(0, buf:len(), text)
  end)
  s.painting = false
  if not ok then error(err) end
end

local function make_readonly_intercept(buf)
  return function(_op)
    local s = _state[buffer_key(buf)]
    if s and s.painting then return nil end
    error("pmacs-mcp-resources: resource buffers are read-only; " ..
          "use pmacs.mcp.send_request('tools/call', ...) to mutate " ..
          "server-side resources.")
  end
end

-- ---------------------------------------------------------------------------
-- Server-capability check
-- ---------------------------------------------------------------------------

local function server_supports_subscribe(server)
  local caps = pmacs.mcp.capabilities(server)
  if type(caps) ~= "table" then return false end
  local r = caps.resources
  if type(r) ~= "table" then return false end
  return r.subscribe == true
end

-- ---------------------------------------------------------------------------
-- Render-into-buffer
-- ---------------------------------------------------------------------------

local function render_into(buf, content_response)
  local rendered = view.render(content_response)
  local s = _state[buffer_key(buf)]
  if s ~= nil then
    s.kind = rendered.kind
    s.mimeType = rendered.mimeType
    s.children = rendered.children or {}
  end
  paint(buf, rendered.body)
end

-- ---------------------------------------------------------------------------
-- Refresh
-- ---------------------------------------------------------------------------
--
-- Called from the notifications/resources/updated handler. Dispatches
-- an async coroutine that re-reads the resource and re-paints the
-- buffer. If the read fails (e.g. server crashed mid-update), the
-- error is swallowed and the buffer is marked stale.

local function refresh(buf)
  local s = _state[buffer_key(buf)]
  if s == nil then return end
  pmacs.async(function()
    -- The M9.2 cache holds the prior content. notifications/resources/
    -- updated is exactly the signal that the cached content is no
    -- longer valid; invalidate before re-reading so we get a fresh
    -- wire fetch rather than the stale cached value.
    pmacs.mcp.invalidate_resource(s.server, s.uri)
    local ok, response = pcall(function()
      return pmacs.mcp.read_resource(s.server, s.uri):await()
    end)
    if not ok then
      s.stale = true
      if pmacs.error then
        pmacs.error("pmacs-mcp-resources: refresh failed for " ..
          s.uri .. ": " .. tostring(response))
      end
      return
    end
    render_into(buf, response)
    s.stale = false
  end)
end

-- ---------------------------------------------------------------------------
-- Notification handler — hooked once at module load.
-- ---------------------------------------------------------------------------

pmacs.mcp.on_notification("notifications/resources/updated",
  function(server, params)
    local uri = params and params.uri
    if type(uri) ~= "string" then return end
    local buf = _by_server_uri[reverse_key(server:raw(), uri)]
    if buf ~= nil then refresh(buf) end
  end)

-- ---------------------------------------------------------------------------
-- Open child (directory navigation)
-- ---------------------------------------------------------------------------

-- Resolve the URI at the cursor's current line (in a directory
-- buffer). Returns the URI string or nil.
--
-- Line indexing: `pmacs.editor.cursor_line()` is *0-based* — the
-- first displayed line is line 0. The directory buffer renders one
-- child URI per line, so cursor line N corresponds to
-- `s.children[N + 1]` (Lua arrays are 1-indexed). An earlier draft
-- compared `line < 1` against the 0-based result, which silently
-- swallowed line 0 (RET on the first URI did nothing) and shifted
-- every other line up by one (RET on line 1 opened children[1] —
-- still the first URI — instead of children[2]). Pinned by
-- `m9_5_directory_ret_keybinding_opens_first_child`.
local function child_uri_at_cursor(buf)
  local s = _state[buffer_key(buf)]
  if s == nil or s.kind ~= "directory" then return nil end
  if pmacs.editor == nil or pmacs.editor.cursor_line == nil then
    -- Fallback: use the first child if we can't query the cursor
    -- (test environments without a window may not have cursor_line).
    return s.children[1]
  end
  local line = pmacs.editor.cursor_line()
  if type(line) ~= "number" or line < 0 then return nil end
  return s.children[line + 1]
end

-- Bound to RET on directory buffers. Opens the URI under cursor
-- via M.open. The new buffer is pushed via pmacs.window.show if
-- available.
local function open_child_at_point(buf)
  local s = _state[buffer_key(buf)]
  if s == nil then return end
  local child = child_uri_at_cursor(buf)
  if child == nil then return end
  -- Capture server reference before async to avoid closure-over
  -- mutated state.
  local server = s.server
  pmacs.async(function()
    local _ = M.open(server, child)
  end)
end

-- Define a buffer-scoped command for RET dispatch.
if pmacs.command and pmacs.command.define then
  pmacs.command.define {
    name = "pmacs-mcp-resources.open-at-point",
    description = "Open the MCP resource URI on the current line.",
    fn = function()
      if pmacs.window == nil or pmacs.window.buffer == nil then return end
      local buf = pmacs.window.buffer()
      if buf ~= nil then open_child_at_point(buf) end
    end,
  }
end

-- Test seam: open the resource at line N of a directory buffer
-- without requiring window/cursor APIs. Used by tests and by the
-- RET binding's fallback path.
function M.open_child_at_line(buf, line)
  local s = _state[buffer_key(buf)]
  if s == nil or s.kind ~= "directory" then return nil end
  local child = s.children[line]
  if child == nil then return nil end
  return M.open(s.server, child)
end

-- ---------------------------------------------------------------------------
-- Open
-- ---------------------------------------------------------------------------

-- Helper: is the (Lua-visible) server currently in the
-- `Initialized` lifecycle state?
local function server_is_initialized(server)
  if pmacs.mcp == nil or pmacs.mcp.list == nil then return false end
  local raw = server:raw()
  for _, row in ipairs(pmacs.mcp.list()) do
    if row.id and row.id:raw() == raw then
      return row.state and row.state.kind == "initialized"
    end
  end
  return false
end

-- Helper: fetch + render + (re-)subscribe an existing buffer.
-- Used by open()'s fresh-buffer path AND by the stale-recovery
-- path (Pass-2 finding 3). Returns true on success, false on
-- failure (in which case the buffer is left in its prior state).
local function fetch_subscribe_render(buf, server, uri)
  local response
  do
    local ok, result = pcall(function()
      return pmacs.mcp.read_resource(server, uri):await()
    end)
    if not ok then return false, result end
    response = result
  end
  render_into(buf, response)
  -- Subscribe if the server supports it. Best-effort: a subscribe
  -- failure leaves the buffer rendered but un-subscribed.
  if server_supports_subscribe(server) then
    local ok, err = pcall(function()
      pmacs.mcp.send_request(server, "resources/subscribe",
        { uri = uri }):await()
    end)
    if ok then
      _state[buffer_key(buf)].subscribed = true
    elseif pmacs.error then
      pmacs.error("pmacs-mcp-resources: resources/subscribe failed " ..
        "for " .. uri .. ": " .. tostring(err) ..
        " (buffer will not auto-refresh)")
    end
  end
  return true
end

function M.open(server, uri)
  if server == nil then
    error("pmacs-mcp-resources.open: server is nil")
  end
  if type(uri) ~= "string" then
    error("pmacs-mcp-resources.open: uri must be a string, got " .. type(uri))
  end

  local server_raw = server:raw()
  local key = reverse_key(server_raw, uri)
  local existing = _by_server_uri[key]
  if existing ~= nil then
    -- Pass-2 finding 3: an existing stale buffer should attempt
    -- to recover when the server is back to Initialized. If the
    -- server is still non-Initialized, return the stale buffer
    -- unchanged (caller can poll is_stale and retry, or close +
    -- reopen if they want fresh bookkeeping).
    local s = _state[buffer_key(existing)]
    if s and s.stale and server_is_initialized(server) then
      -- Re-bind the server handle (the caller may have a fresh
      -- McpServerIdLua even though server_raw matches), then
      -- refetch + re-subscribe.
      s.server = server
      s.subscribed = false
      local ok = fetch_subscribe_render(existing, server, uri)
      if ok then
        s.stale = false
      end
    end
    return existing
  end

  local buf = pmacs.buffer.create("*mcp:" .. uri .. "*")
  _state[buffer_key(buf)] = {
    server = server,
    server_raw = server_raw,
    uri = uri,
    painting = false,
    stale = false,
    children = {},
    subscribed = false,
    kind = "raw",
    mimeType = "",
  }
  _by_server_uri[key] = buf

  -- Read-only intercept (M8 CC-1 pattern).
  pmacs.buffer.add_intercept(buf, make_readonly_intercept(buf))

  -- Pass-2 finding 2 / Pass-3 finding 1: wrap the initial fetch in
  -- pcall so a read-failure doesn't leave a half-initialized entry
  -- in the registry OR a dead `*mcp:...*` buffer in the editor's
  -- buffer list. On failure, drop the registry entries, remove the
  -- buffer, and re-raise so the caller knows it didn't work and
  -- can retry.
  local ok, err = pcall(function()
    local response = pmacs.mcp.read_resource(server, uri):await()
    render_into(buf, response)
  end)
  if not ok then
    _state[buffer_key(buf)] = nil
    _by_server_uri[key] = nil
    -- Best-effort buffer cleanup. If the remove itself errors (e.g.
    -- buffer already gone), ignore it — the original `err` is what
    -- the caller cares about.
    pcall(function() pmacs.buffer.remove(buf) end)
    error(err)
  end

  -- Subscribe if the server supports it (best-effort; subscribe
  -- failure leaves the buffer un-subscribed but otherwise valid).
  if server_supports_subscribe(server) then
    local sub_ok, sub_err = pcall(function()
      pmacs.mcp.send_request(server, "resources/subscribe",
        { uri = uri }):await()
    end)
    if sub_ok then
      _state[buffer_key(buf)].subscribed = true
    elseif pmacs.error then
      pmacs.error("pmacs-mcp-resources: resources/subscribe failed " ..
        "for " .. uri .. ": " .. tostring(sub_err) ..
        " (buffer will not auto-refresh)")
    end
  end

  -- Bind RET on directory buffers to open-child-at-point.
  if _state[buffer_key(buf)].kind == "directory" and pmacs.keymap and pmacs.keymap.bind then
    pmacs.keymap.bind {
      scope = "buffer",
      buffer = buf,
      sequence = "RET",
      command = "pmacs-mcp-resources.open-at-point",
    }
  end

  return buf
end

-- ---------------------------------------------------------------------------
-- Close
-- ---------------------------------------------------------------------------

function M.close(buf)
  local s = _state[buffer_key(buf)]
  if s == nil then return end
  if s.subscribed then
    -- Best-effort unsubscribe — if the server has died, this errors
    -- and we ignore it.
    pcall(function()
      pmacs.mcp.send_request(s.server, "resources/unsubscribe",
        { uri = s.uri }):await()
    end)
  end
  _by_server_uri[reverse_key(s.server_raw, s.uri)] = nil
  _state[buffer_key(buf)] = nil
  -- v0.1: leave buffer destruction to the caller / editor's normal
  -- buffer-lifecycle path. Future work: pmacs.buffer.destroy.
end

-- ---------------------------------------------------------------------------
-- Stale state
-- ---------------------------------------------------------------------------

function M.is_stale(buf)
  local s = _state[buffer_key(buf)]
  if s == nil then return false end
  return s.stale == true
end

-- Mark all buffers for `server_raw` stale. Called from the lifecycle
-- watchdog when a subscribed server transitions to non-Initialized.
local function mark_server_stale(server_raw)
  for _, buf in pairs(_by_server_uri) do
    local s = _state[buffer_key(buf)]
    if s ~= nil and s.server_raw == server_raw and s.subscribed then
      s.stale = true
    end
  end
end

-- Watchdog: hook into the same async tick to observe server state
-- transitions. Cheap enough to run every tick (one walk over the
-- mcp.list output, which is small).
local _orig_async_tick = pmacs._async.tick
function pmacs._async.tick()
  _orig_async_tick()
  if pmacs.mcp == nil or pmacs.mcp.list == nil then return end
  local rows = pmacs.mcp.list()
  for _, row in ipairs(rows) do
    if row.state and row.state.kind ~= "initialized" then
      mark_server_stale(row.id:raw())
    end
  end
end

-- Test seams (mirror the M8 fixture pattern).
M.__pmacs_mcp_resources_test_state = function(buf) return _state[buffer_key(buf)] end
M.__pmacs_mcp_resources_test_buffer_for = function(server_raw, uri)
  return _by_server_uri[reverse_key(server_raw, uri)]
end

return M
