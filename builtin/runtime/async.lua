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
-- coroutines waiting for the next async tick without dispatching a worker job.
local next_tick_coroutines = {}
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
  -- Journey Stage 1a (Q#JR14b): `pmacs.window.commit_to` scopes the
  -- acting frontend for the dynamic extent of its callback, using an
  -- RAII guard on the Rust stack. Yielding out of that extent would
  -- restore the scope while this coroutine is still parked, so the rest
  -- of the commit would resume ambient -- silently reintroducing the
  -- misrouting the scope exists to prevent. Do the awaiting BEFORE
  -- entering the commit, which is what dired does with its listing.
  if async_mod._in_commit_scope() then
    error("await: cannot await inside pmacs.window.commit_to; " ..
      "await first, then commit")
  end
  -- Worker identity Stage 1 (Q#W-2 rule 1): `pmacs.workers.dispatch`
  -- pushes the registered handler's name for the dynamic extent of the
  -- handler call, so that jobs allocated inside it are attributable to
  -- the third party that asked for them. Parking here would leave the
  -- name pushed while this coroutine is suspended, and every job
  -- allocated in the meantime --- in any coroutine, on any later tick
  -- --- would inherit it. Same hazard, same shape, same remedy as the
  -- commit-scope refusal above.
  --
  -- Two properties this placement buys, both load-bearing:
  --
  --  * it rejects BEFORE parking (ahead of the `_is_complete` check and
  --    the `coroutine.yield`), because a guard consulted after the yield
  --    has already happened guards nothing;
  --  * it rejects UNCONDITIONALLY, not only when a yield would really
  --    occur. A guard that fires only for an incomplete handle would
  --    pass or fail depending on whether the job happened to settle
  --    first --- green under test, intermittent in production.
  if async_mod._in_dispatch_name_scope() then
    error("await: cannot await inside pmacs.workers.dispatch; " ..
      "await first, then dispatch")
  end
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
  elseif type(yielded) == "table" and yielded._is_pmacs_next_tick then
    next_tick_coroutines[#next_tick_coroutines + 1] = co
  else
    if pmacs.error then
      pmacs.error("pmacs.async: coroutine yielded a non-Handle value (" ..
        type(yielded) .. "); use Handle:await() per R46")
    else
      error("pmacs.async: coroutine yielded a non-Handle value")
    end
  end
end

local function spawn_async(fn)
  if type(fn) ~= "function" then
    error("pmacs.async expects a function, got " .. type(fn))
  end
  local co = coroutine.create(fn)
  step(co)
end

local async_public = {}

setmetatable(async_public, {
  __call = function(_, fn)
    return spawn_async(fn)
  end,
})

-- The SECOND supported yield API. `Handle:await()` is the first; any
-- rule about a non-yieldable dynamic extent has to cover both, or the
-- extent stays open through a second door.
--
-- Both refusals below are that rule. The commit-scope one is a
-- **pre-existing gap being closed** (worker identity framing Q#W-7):
-- Journey Stage 1a's Q#JR14b invariant was enforced on `:await()` only,
-- so a coroutine inside `pmacs.window.commit_to` could park through here
-- and produce exactly the misrouting that guard exists to prevent.
--
-- Placement is the whole point: both fire *before* the `coroutine.yield`
-- below, and both fire unconditionally. A refusal sited after the yield
-- would never run in the case it exists for.
function async_public.yield_to_next_tick()
  if async_mod._in_commit_scope() then
    error("yield_to_next_tick: cannot yield inside pmacs.window.commit_to; " ..
      "yield first, then commit")
  end
  if async_mod._in_dispatch_name_scope() then
    error("yield_to_next_tick: cannot yield inside pmacs.workers.dispatch; " ..
      "yield first, then dispatch")
  end
  coroutine.yield({ _is_pmacs_next_tick = true })
end

pmacs.async = async_public

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

-- Runtime-internal: expose the Handle / Stream factories so other
-- builtin runtime files (pmacs.fs in M8.1, future siblings) can
-- construct handles for ids dispatched through their own raw
-- _dispatch_* primitives without re-implementing the class. The
-- underscore prefix marks these as not part of the documented
-- package-author surface; package code uses :await() / :cancel() /
-- :on_complete() on the returned handles, never these factories.
pmacs.workers._new_handle = new_handle
pmacs.workers._new_stream = new_stream

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

-- Worker identity Stage 1 (Q#W-2): `name` used to die here.
--
-- The audit's "every third-party job renders under a builtin's label" is
-- exact, and the reason is this function: the handler is arbitrary Lua,
-- nothing below it takes a name, and a handler that reaches straight for
-- `pmacs._async._dispatch_*` bypasses the wrapper layer entirely. So the
-- name is pushed onto a runtime-owned stack for the dynamic extent of
-- the handler call and read at `allocate`, the single funnel every job
-- passes through. Seven rules govern it; five are visible here:
--
--   1. The extent is NON-YIELDABLE, and that is enforced rather than
--      assumed --- see the refusals in `Handle:await` and
--      `pmacs.async.yield_to_next_tick`.
--   3. Nesting is a stack; innermost wins.
--   4. Fan-out shares the name: five jobs dispatched by one handler are
--      five jobs named alike. They *were* all dispatched under it.
--   5. UNWIND-SAFE, and this is the one that makes a naive version worse
--      than none. A handler that raises must still pop --- otherwise one
--      failure poisons every subsequent dispatch in the session with a
--      stale name, and the feature starts lying silently instead of
--      failing loudly. Hence pcall, pop, rethrow.
--   7. Outside any extent nothing changes: a builtin invoked directly
--      records its own purpose.
--
-- Rule 2 (work dispatched later, from an `on_complete` callback or a
-- resumed coroutine, is deliberately NOT covered) and rule 6
-- (composition, `"<name>: <purpose>"`) live on the Rust side.
--
-- The pop/rethrow half, hoisted so it is written once and allocates
-- nothing per dispatch.
--
-- Varargs across a function boundary, NOT `local ok, result = pcall(…)`:
-- this function used to be `return handler(args, opts)`, which
-- propagates EVERY return value, and bracketing it must not silently
-- truncate a handler that returns more than one. `table.pack` /
-- `table.unpack` would say the same thing but are Lua 5.2 surface, and
-- LuaJIT is this project's default backend (`Cargo.toml`:
-- `default = ["luajit"]`).
local function finish_dispatch(ok, ...)
  async_mod._pop_dispatch_name()
  if not ok then
    -- Level 0: the handler's error travels unchanged. R45's structured
    -- errors are tables, and a re-raise that appended position info
    -- would corrupt a plain-string error and be silently ignored for a
    -- table one --- so neither shape is served by the default level.
    error((...), 0)
  end
  return ...
end

function pmacs.workers.dispatch(name, args, opts)
  local handler = handlers[name]
  if handler == nil then
    error("pmacs.workers.dispatch: unknown handler '" .. tostring(name) .. "'")
  end
  async_mod._push_dispatch_name(name)
  return finish_dispatch(pcall(handler, args, opts))
end

-- Worker identity Stage 1: the name registered here is DISPLAY TEXT.
--
-- It used to be type-checked and nothing more, which was defensible
-- while it died inside `dispatch`. It no longer dies there: the ambient
-- carries it into every job the handler allocates, and it is composed
-- into `purpose` as `"<name>: <purpose>"`, which the `*workers*` table
-- and the modeline indicator both render. So it gets the same
-- meaningful-value standard `purpose` already gets in
-- `required_purpose` (`src/lua_bindings/mod.rs`) --- and one rule
-- `purpose` deliberately does NOT get.
--
-- The asymmetry is the point. A purpose may legitimately contain a
-- newline: a filesystem path can, and `pmacs-magit`'s spawn purpose is a
-- whole argv --- so its one-line constraint is enforced by ESCAPING at
-- the surfaces that have one row (`purpose_for_one_row`), following the
-- `#228` decision on `Command.description`. A registered handler NAME
-- has no such case. It is an identifier a package chooses for itself and
-- passes back to `dispatch`, so a control character in it is a mistake
-- or an attempt at one, and refusing at the source costs nobody
-- anything.
function pmacs.workers.register(name, handler)
  -- Allows future Rust-side modules (or test harnesses) to register
  -- additional dispatchable names. v0.1 has no plugin loader but the
  -- shape is here so M4 builders use it consistently.
  if type(name) ~= "string" then
    error("pmacs.workers.register: name must be a string")
  end
  -- Empty and whitespace-only satisfy the type and say nothing --- the
  -- exact pair `required_purpose` rejects, and the exact pair R42
  -- rejects for config descriptions.
  if name:match("^%s*$") ~= nil then
    error("pmacs.workers.register: name must not be empty or whitespace-only")
  end
  -- `%c` is the C control class: NUL, the C0 range, DEL. A newline
  -- forges a row in `*workers*`, a CR rewrites one on a terminal and an
  -- ESC starts a sequence in one. Checked AFTER the whitespace rule so
  -- a name that is only "\n" reports the emptier problem, which is the
  -- one the caller can act on.
  if name:find("%c") ~= nil then
    error("pmacs.workers.register: name must not contain control characters")
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
  local ready_next_tick = next_tick_coroutines
  next_tick_coroutines = {}
  for _, co in ipairs(ready_next_tick) do
    step(co)
  end

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

-- ---------------------------------------------------------------------------
-- Statusline activity indicator (worker identity Stage 1, Q#W-3/Q#W-6).
-- ---------------------------------------------------------------------------
--
-- `COHERENCE.md` §9 records that no progress indicator exists anywhere
-- --- no spinner, no busy count --- which makes §3's promise of "visible
-- asynchronous work" false unless the user knows to run
-- `M-x editor.list-workers`. This is the fourth `pmacs.statusline.register`
-- adopter (after `mode`, `terminal` and `lsp`) and the first thing that
-- makes background work visible without a command.
--
-- No wire change: `pmacs.statusline.register` rides the existing
-- `StatuslineSegments` vector, so a fourth provider adds an ELEMENT, not
-- a variant. That is what lets this lane run beside the two holding the
-- protocol-bump slot.

-- A visibility toggle, and only that (Q#W-6). A permanently-visible
-- statusline element is different in kind from an internal behaviour: it
-- costs modeline width on every frame, and "I do not want this in my
-- modeline" is a preference someone genuinely holds on day one. There is
-- deliberately NO setting for purpose capture itself --- that is
-- substrate, not preference.
pmacs.config.define {
  name = "ui.activity-indicator",
  description = "Show a modeline count of in-flight background jobs, with the oldest job's purpose. Absent entirely when nothing is running.",
  type = "boolean",
  default = true,
  mutability = "live",
}

pmacs.statusline.register {
  name = "activity",
  side = "right",
  -- Above `terminal` (10) and `lsp` (0): when the modeline is too narrow
  -- for everything, "the editor is busy, on this" is the segment worth
  -- keeping. Right-side display order is priority-ascending, so it also
  -- lands nearest the protected cursor/scroll group.
  priority = 20,
  face = "ui.modeline.activity",
  fn = function(_ctx)
    if pmacs.config.get("ui.activity-indicator") ~= true then return nil end
    -- `_activity_summary` rather than `pmacs.workers.snapshot()`: this
    -- runs once per visible window per frame, and a snapshot would clone
    -- the whole 64-entry completed ring that the indicator never reads.
    local summary = async_mod._activity_summary()
    -- nil, not "" and not "0 jobs": the evaluator treats an empty string
    -- as "no segment" too, but a zero-count string would be a segment
    -- that costs width forever to say nothing is happening. Absence is
    -- the design (Q#W-3), so absence is what this returns.
    if summary == nil then return nil end
    return "⋯" .. tostring(summary.in_flight) .. " " .. summary.purpose
  end,
}

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
