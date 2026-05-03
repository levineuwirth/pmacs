-- builtin/runtime/async.lua --- T M3.3 Lua-side async runtime.
--
-- The Rust runtime (`pmacs._async.*`) gives us seven primitives:
--
--   _dispatch_sleep(ms)  -> JobId
--   _dispatch_sum(n)     -> JobId
--   _cancel(id)
--   _is_complete(id)     -> boolean
--   _is_cancelled(id)    -> boolean
--   _take_result(id)     -> status, value
--   _tick()              -> { ids... }
--
-- This file builds the friendly surface on top of them:
--
--   pmacs.async(fn)               -- spawn a coroutine
--   pmacs.workers.dispatch(name, args) -- name-based dispatcher
--   pmacs.workers.sleep(ms)       -- builtin sleep handler
--   pmacs.workers.compute_sum(n)  -- builtin sum handler
--   handle:await()                -- yield until handle settles
--   handle:cancel()               -- cooperatively cancel
--   handle:on_complete(fn)        -- non-coroutine callback
--   handle:id()                   -- runtime-assigned id
--   handle:is_complete()          -- has the runtime observed a result?
--
-- R45: cancelled awaits raise `error({ tag = "cancelled", id = id })`.
-- R46: package code uses `:await()` rather than `coroutine.yield`. The
-- yield call below is *runtime* code, exempt from R46 by definition.

local async_mod = pmacs._async
assert(async_mod, "pmacs._async must be installed before the async builtin loads")

-- ---------------------------------------------------------------------------
-- Internal pending tables.
-- ---------------------------------------------------------------------------

-- handle id -> coroutine waiting on it.
local parked_coroutines = {}
-- handle id -> array of on_complete callbacks.
local on_complete_callbacks = {}
-- coroutine -> the handle the coroutine yielded with last (so we can
-- detect a coroutine that yields a non-handle and surface it).
local coroutine_waiting_on = setmetatable({}, { __mode = "k" })

-- ---------------------------------------------------------------------------
-- Handle class.
-- ---------------------------------------------------------------------------

local Handle = {}
Handle.__index = Handle
Handle._is_pmacs_handle = true

local function new_handle(id)
  return setmetatable({ _id = id }, Handle)
end

function Handle:id()
  return self._id
end

function Handle:is_complete()
  return async_mod._is_complete(self._id)
end

function Handle:cancel()
  async_mod._cancel(self._id)
end

function Handle:_take()
  -- Single-shot: removes the entry from the runtime and returns
  -- (status, value). Subsequent calls return ("pending", nil).
  return async_mod._take_result(self._id)
end

-- :await() is the canonical user-facing yield point. It MUST run
-- inside a coroutine spawned by pmacs.async --- a bare call from main
-- thread will raise on the first yield.
function Handle:await()
  if not async_mod._is_complete(self._id) then
    -- Yield self so pmacs.async's step() can park us. R46 carve-out:
    -- this `coroutine.yield` is runtime code; package code uses
    -- :await(), never raw yield.
    coroutine.yield(self)
  end
  local status, value = self:_take()
  if status == "ok" then
    return value
  elseif status == "cancelled" then
    -- R45: structured error with tag = "cancelled".
    error({ tag = "cancelled", id = self._id })
  elseif status == "failed" then
    error({ tag = "failed", id = self._id, message = value })
  else
    -- "pending" --- runtime told us we were complete but the entry
    -- vanished, or the coroutine was resumed without the runtime's
    -- bookkeeping. Either is a bug; raise loudly.
    error({
      tag = "internal",
      message = "await: unexpected status '" .. tostring(status) ..
        "' on handle " .. tostring(self._id),
    })
  end
end

function Handle:on_complete(fn)
  if type(fn) ~= "function" then
    error("Handle:on_complete expects a function, got " .. type(fn))
  end
  if self:is_complete() then
    -- Already settled --- fire immediately. Take the result so the
    -- callback sees the same shape an awaited handle would.
    local status, value = self:_take()
    fn(status, value)
    return
  end
  local list = on_complete_callbacks[self._id]
  if list == nil then
    list = {}
    on_complete_callbacks[self._id] = list
  end
  table.insert(list, fn)
end

-- ---------------------------------------------------------------------------
-- Stream Handle (T M3.5).
-- ---------------------------------------------------------------------------
--
-- A streaming handle is a Handle that delivers batches via
-- `:on_batch(fn)` rather than a single result via `:await()`.
-- `:on_close(fn)` fires once with the terminal outcome (status,
-- value) when the stream settles. `:cancel()` requests cooperative
-- cancellation.
--
-- Stream handles are created by handlers that opt into streaming
-- (e.g. `pmacs.workers.emit_n`); request/reply handlers
-- (`compute_sum`, `sleep`) stay non-stream. Mixing `:await` on a
-- stream handle is a programming error: streams settle via
-- `:on_close`, not as a single value.

local stream_batch_callbacks = {}  -- stream id -> array of fn(items)
local stream_close_callbacks = {}  -- stream id -> array of fn(status, value)

local Stream = {}
Stream.__index = Stream
Stream._is_pmacs_handle = true
Stream._is_pmacs_stream = true

local function new_stream(id)
  return setmetatable({ _id = id }, Stream)
end

function Stream:id() return self._id end
function Stream:is_complete() return async_mod._is_complete(self._id) end
function Stream:cancel() async_mod._cancel(self._id) end

function Stream:on_batch(fn)
  if type(fn) ~= "function" then
    error("Stream:on_batch expects a function, got " .. type(fn))
  end
  local list = stream_batch_callbacks[self._id]
  if list == nil then
    list = {}
    stream_batch_callbacks[self._id] = list
  end
  table.insert(list, fn)
end

function Stream:on_close(fn)
  if type(fn) ~= "function" then
    error("Stream:on_close expects a function, got " .. type(fn))
  end
  local list = stream_close_callbacks[self._id]
  if list == nil then
    list = {}
    stream_close_callbacks[self._id] = list
  end
  table.insert(list, fn)
end

-- ---------------------------------------------------------------------------
-- pmacs.async --- coroutine spawn.
-- ---------------------------------------------------------------------------

local function step(co)
  local ok, yielded = coroutine.resume(co)
  if not ok then
    -- An uncaught error inside the coroutine. Surface via pmacs.error
    -- if available; fall back to a plain error otherwise.
    if pmacs.error then
      pmacs.error("pmacs.async: coroutine raised: " .. tostring(yielded))
    else
      error("pmacs.async: coroutine raised: " .. tostring(yielded))
    end
    return
  end
  if coroutine.status(co) == "dead" then
    return -- normal completion
  end
  -- Coroutine yielded; expect a Handle.
  if type(yielded) == "table" and yielded._is_pmacs_handle then
    parked_coroutines[yielded._id] = co
    coroutine_waiting_on[co] = yielded._id
  else
    if pmacs.error then
      pmacs.error("pmacs.async: coroutine yielded a non-Handle value (" ..
        type(yielded) .. "); use Handle:await() per R46")
    else
      error("pmacs.async: coroutine yielded a non-Handle value")
    end
  end
end

function pmacs.async(fn)
  if type(fn) ~= "function" then
    error("pmacs.async expects a function, got " .. type(fn))
  end
  local co = coroutine.create(fn)
  step(co)
end

-- ---------------------------------------------------------------------------
-- pmacs.workers --- name-based dispatch surface.
-- ---------------------------------------------------------------------------

pmacs.workers = pmacs.workers or {}

-- T M3.4: every dispatch surface accepts an optional `opts` table.
-- The single recognised key is `supersede = "<name>"`: a new dispatch
-- under the same supersede name cancels any in-flight predecessor
-- before the new job starts (per spec §6.3 cancellation /
-- "supersede semantics").
local function supersede_key(opts)
  if opts == nil then return nil end
  if type(opts) ~= "table" then
    error("dispatch opts must be a table, got " .. type(opts))
  end
  local k = opts.supersede
  if k == nil then return nil end
  if type(k) ~= "string" then
    error("opts.supersede must be a string, got " .. type(k))
  end
  return k
end

local function dispatch_sleep(ms, opts)
  if type(ms) ~= "number" then
    error("pmacs.workers.sleep: ms must be a number, got " .. type(ms))
  end
  return new_handle(async_mod._dispatch_sleep(math.floor(ms), supersede_key(opts)))
end

local function dispatch_sum(n, opts)
  if type(n) ~= "number" or n < 0 then
    error("pmacs.workers.compute_sum: n must be a non-negative number")
  end
  return new_handle(async_mod._dispatch_sum(math.floor(n), supersede_key(opts)))
end

local function dispatch_emit_n(count, opts)
  if type(count) ~= "number" or count < 0 then
    error("pmacs.workers.emit_n: count must be a non-negative number")
  end
  local max_batch
  if type(opts) == "table" and opts.max_batch ~= nil then
    if type(opts.max_batch) ~= "number" or opts.max_batch < 1 then
      error("opts.max_batch must be a positive number")
    end
    max_batch = math.floor(opts.max_batch)
  end
  return new_stream(async_mod._dispatch_emit_n(
    math.floor(count), supersede_key(opts), max_batch))
end

-- T M3.6: parallel directory grep.
--   pmacs.workers.grep({ root = "/path", pattern = "needle",
--                        case_sensitive = true,
--                        max_file_bytes = 16 * 1024 * 1024,
--                        max_match_text = 4096,
--                        max_results = 0,
--                        fanout = 8 }, opts) -> Stream
--
-- Each batch handed to :on_batch is an array of match tables:
--   { file = "rel/path", line = 12, match_start = 4, match_end = 9, text = "..." }
local function dispatch_grep(spec, opts)
  if type(spec) ~= "table" then
    error("pmacs.workers.grep: spec must be a table")
  end
  if type(spec.root) ~= "string" or spec.root == "" then
    error("pmacs.workers.grep: spec.root must be a non-empty string")
  end
  if type(spec.pattern) ~= "string" then
    error("pmacs.workers.grep: spec.pattern must be a string")
  end
  local max_batch
  if type(opts) == "table" and opts.max_batch ~= nil then
    if type(opts.max_batch) ~= "number" or opts.max_batch < 1 then
      error("opts.max_batch must be a positive number")
    end
    max_batch = math.floor(opts.max_batch)
  end
  return new_stream(async_mod._dispatch_grep(spec, supersede_key(opts), max_batch))
end

pmacs.workers.sleep = dispatch_sleep
pmacs.workers.compute_sum = dispatch_sum
pmacs.workers.emit_n = dispatch_emit_n
pmacs.workers.grep = dispatch_grep

-- Name-based dispatch matching the spec example:
--   pmacs.workers.dispatch("grep", { ... }, { supersede = "grep" }):await()
-- v0.1 ships with the two stub handlers above; M4 adds tree-sitter,
-- LSP, project indexing, etc. via the same surface.
local handlers = {
  ["sleep"] = function(args, opts)
    return dispatch_sleep((type(args) == "table" and args.ms) or args or 0, opts)
  end,
  ["compute_sum"] = function(args, opts)
    return dispatch_sum((type(args) == "table" and args.n) or args or 0, opts)
  end,
  ["emit_n"] = function(args, opts)
    return dispatch_emit_n((type(args) == "table" and args.count) or args or 0, opts)
  end,
  ["grep"] = function(args, opts)
    if type(args) ~= "table" then
      error("pmacs.workers.dispatch('grep', ...): args must be a spec table")
    end
    return dispatch_grep(args, opts)
  end,
}

function pmacs.workers.dispatch(name, args, opts)
  local handler = handlers[name]
  if handler == nil then
    error("pmacs.workers.dispatch: unknown handler '" .. tostring(name) .. "'")
  end
  return handler(args, opts)
end

function pmacs.workers.register(name, handler)
  -- Allows future Rust-side modules (or test harnesses) to register
  -- additional dispatchable names. v0.1 has no plugin loader but the
  -- shape is here so M4 builders use it consistently.
  if type(name) ~= "string" then
    error("pmacs.workers.register: name must be a string")
  end
  if type(handler) ~= "function" then
    error("pmacs.workers.register: handler must be a function")
  end
  handlers[name] = handler
end

-- ---------------------------------------------------------------------------
-- Tick: drain the bus, resume parked coroutines, fire on_complete callbacks.
-- ---------------------------------------------------------------------------

function pmacs._async.tick()
  local settled = async_mod._tick()
  for _, id in ipairs(settled) do
    -- Fire on_complete callbacks before resuming the parked coroutine.
    -- Both can run on the same id if the user installed an
    -- on_complete *and* awaited the handle elsewhere; but await
    -- removes the entry on resume, so order matters: callbacks first.
    local callbacks = on_complete_callbacks[id]
    if callbacks ~= nil then
      on_complete_callbacks[id] = nil
      for _, cb in ipairs(callbacks) do
        local status, value = async_mod._take_result(id)
        local ok, err = pcall(cb, status, value)
        if not ok and pmacs.error then
          pmacs.error("on_complete callback failed: " .. tostring(err))
        end
      end
    end
    local co = parked_coroutines[id]
    if co ~= nil then
      parked_coroutines[id] = nil
      coroutine_waiting_on[co] = nil
      step(co)
    end
  end

  -- T M3.5: drain stream batches. One callback invocation per
  -- (stream, frame), regardless of how many items the worker
  -- emitted in between. The Rust runtime caps each batch at the
  -- stream's `max_batch`; if there are more items pending, they
  -- come in the next batch on the next tick.
  local batches = async_mod._take_stream_batches()
  for _, batch in ipairs(batches) do
    local id = batch.id
    local items = batch.items
    if #items > 0 then
      local list = stream_batch_callbacks[id]
      if list ~= nil then
        for _, cb in ipairs(list) do
          local ok, err = pcall(cb, items)
          if not ok and pmacs.error then
            pmacs.error("on_batch callback failed: " .. tostring(err))
          end
        end
      end
    end
    if batch.closed then
      -- One last fan-out to on_close subscribers, then GC the entry.
      local closers = stream_close_callbacks[id]
      if closers ~= nil then
        for _, cb in ipairs(closers) do
          local ok, err = pcall(cb, batch.status, batch.value)
          if not ok and pmacs.error then
            pmacs.error("on_close callback failed: " .. tostring(err))
          end
        end
      end
      stream_batch_callbacks[id] = nil
      stream_close_callbacks[id] = nil
    end
  end
end

-- ---------------------------------------------------------------------------
-- Tunable knobs (T M3.5).
-- ---------------------------------------------------------------------------

pmacs.async_config = {}

function pmacs.async_config.frame_target_ms(ms)
  if ms == nil then
    return async_mod._frame_target_ms()
  end
  if type(ms) ~= "number" or ms < 1 then
    error("frame_target_ms must be a positive number")
  end
  async_mod._set_frame_target_ms(math.floor(ms))
end

function pmacs.async_config.default_max_batch(n)
  if n == nil then
    return async_mod._default_max_batch()
  end
  if type(n) ~= "number" or n < 1 then
    error("default_max_batch must be a positive number")
  end
  async_mod._set_default_max_batch(math.floor(n))
end

-- ---------------------------------------------------------------------------
-- T M3.7: *workers* observability buffer.
-- ---------------------------------------------------------------------------
--
--   pmacs.workers.snapshot() -> { active = {...}, completed = {...} }
--   pmacs.workers.show()     -> BufferIdLua    -- create/refresh and bind
--   pmacs.workers.hide()                       -- stop auto-refresh
--   pmacs.workers.cancel_at_point()            -- bound to C-c C-k
--   pmacs.workers.is_visible()
--
-- The buffer auto-refreshes once per frame while visible, satisfying
-- the spec's 100ms acceptance bound (default frame target is 16ms).

local workers_buffer_id = nil
local workers_buffer_visible = false

function pmacs.workers.snapshot()
  return async_mod._workers_snapshot()
end

function pmacs.workers.is_visible()
  return workers_buffer_visible
end

function pmacs.workers.show()
  if async_mod._show_workers_buffer == nil then
    error("pmacs.workers.show: no buffer registry was wired into the async runtime")
  end
  local id = async_mod._show_workers_buffer()
  workers_buffer_visible = true
  -- Bind the buffer-local cancel binding once per buffer
  -- incarnation: the runtime keeps the same id while the
  -- buffer lives, so a subsequent show() is a no-op for the
  -- keymap path. If the buffer was killed and recreated under
  -- the same name, the id changes and we rebind.
  if workers_buffer_id ~= id then
    -- best-effort unbind in case a stale buffer-local binding
    -- exists; ignore failures.
    pcall(function()
      pmacs.keymap.unbind { scope = "buffer", buffer = id, sequence = "C-c C-k" }
    end)
    pmacs.keymap.bind {
      scope = "buffer",
      buffer = id,
      sequence = "C-c C-k",
      command = "workers.cancel-at-point",
    }
    workers_buffer_id = id
  end
  return id
end

function pmacs.workers.hide()
  workers_buffer_visible = false
end

-- Resolve the job id at the cursor in the *workers* buffer (or any
-- buffer whose contents follow the same `#<digits>` per-row format).
-- Returns the cancelled id, or nil if the cursor isn't on a job row.
function pmacs.workers.cancel_at_point()
  if pmacs.window == nil or pmacs.editor == nil then
    return nil
  end
  local buf = pmacs.window.buffer()
  local pos = pmacs.editor.cursor()
  if buf == nil or pos == nil then return nil end
  local id = async_mod._job_id_at_byte(buf, pos)
  if id ~= nil then async_mod._cancel(id) end
  return id
end

-- Register the cancel-at-point command. Defining it here (rather than
-- in the user's init.lua) means a fresh editor always has the binding
-- available --- the user's only step is `pmacs.workers.show()`.
if pmacs.command and pmacs.command.define then
  pmacs.command.define {
    name = "workers.cancel-at-point",
    description = "Cancel the worker job named on the current line of *workers*.",
    fn = pmacs.workers.cancel_at_point,
  }
end

-- Hook the auto-refresh into the existing tick. We wrap the original
-- _async.tick (which fires every frame from the editor's run loop)
-- so the *workers* buffer re-renders after every drain.
local original_tick = pmacs._async.tick
function pmacs._async.tick()
  original_tick()
  if workers_buffer_visible and async_mod._show_workers_buffer ~= nil then
    -- A failed render (e.g. registry wasn't wired) becomes silent ---
    -- not worth raising every frame.
    pcall(async_mod._show_workers_buffer)
  end
end

-- Diagnostic / test helpers: number of parked coroutines, number of
-- pending Rust-side jobs. Used by Rust integration tests to drive the
-- runtime to quiescence.
function pmacs._async.parked_count()
  local n = 0
  for _ in pairs(parked_coroutines) do n = n + 1 end
  return n
end

function pmacs._async.pending_count()
  return async_mod._pending_len()
end
