-- builtin/runtime/syntax.lua --- T M4.2 auto-attach grammar by extension.
--
-- The Rust side (`crate::syntax::BUILTIN_LANGUAGES`) keeps a config
-- table of `name → loader + extensions`. This file is the Lua-side
-- glue that fires on every successful file load: it asks
-- `pmacs.parse.language_for_path` whether the loaded file maps to a
-- known grammar, and dispatches a parse if so. The settle path is
-- handled by an after-tick step that drains finished parse jobs and
-- installs them into the buffer's view.
--
-- Adding a new grammar requires only:
--   1. A `tree-sitter-foo = "X.Y"` line in `Cargo.toml`.
--   2. A new `LanguageEntry` in `crate::syntax::BUILTIN_LANGUAGES`.
-- Nothing in this file needs to change.

-- Set of dispatched-but-not-yet-installed parse job ids. Populated
-- when `pmacs.parse._dispatch` is called below; drained by the tick
-- step. The scheduler tables below add per-buffer coalescing on top:
-- at most one parse runs for a buffer, and edits that arrive while it
-- is in flight queue exactly one follow-up parse.
local pending_parse_jobs = {}
local job_to_buffer = {}
local inflight_by_buffer = {}
local dirty_buffers = {}
local buffer_records = {}

local raw_dispatch = pmacs.parse._dispatch

-- Wrap `_dispatch` so every dispatched parse job lands in our
-- pending set. Calls into `_dispatch` from outside this file (e.g.
-- a hand-rolled user script) get the same tracking for free.
function pmacs.parse._dispatch(buf, lang)
  local job_id = raw_dispatch(buf, lang)
  pending_parse_jobs[job_id] = true
  return job_id
end

-- Fallback set for hosts that do not expose overlay introspection.
-- Highlight overlays are window-local, not buffer-local: a buffer can
-- already have a parse view while the active window still needs its
-- own syntax-highlight overlay/cache.
local highlighted_windows = {}

local function active_window_key(buf)
  local ok, win = pcall(pmacs.window.current)
  if not ok or win == nil then return tostring(buf) end
  return tostring(win) .. ":" .. tostring(buf)
end

local function active_window_has_highlight()
  if pmacs.window._overlay_kinds then
    local ok, kinds = pcall(pmacs.window._overlay_kinds)
    if ok and kinds then
      for _, kind in ipairs(kinds) do
        if kind == "syntax-highlight" then return true end
      end
      return false
    end
  end
  local buf = pmacs.window.buffer()
  return highlighted_windows[active_window_key(buf)] == true
end

local function record_for_buffer(buf)
  if not buf then return end
  local path = buf:name()
  if not path then return end
  local lang = pmacs.parse.language_for_path(path)
  if not lang then return end
  local key = tostring(buf)
  local rec = { buf = buf, lang = lang }
  buffer_records[key] = rec
  return key, rec
end

local function dispatch_parse_for(key, rec)
  if inflight_by_buffer[key] then return end
  local ok, job_id = pcall(pmacs.parse._dispatch, rec.buf, rec.lang)
  if not ok then
    if pmacs.error then pmacs.error("syntax.parse-dispatch: " .. tostring(job_id)) end
    return
  end
  inflight_by_buffer[key] = job_id
  job_to_buffer[job_id] = key
  dirty_buffers[key] = nil
end

local function mark_dirty(buf)
  local key, rec = record_for_buffer(buf)
  if not key then return end
  dirty_buffers[key] = rec
end

local function attach_for_active_buffer()
  local buf = pmacs.window.buffer()
  local key, rec = record_for_buffer(buf)
  if not key then return end
  dispatch_parse_for(key, rec)
  -- T M4.3: install the syntax-highlight overlay for this buffer.
  -- Idempotent --- repeated calls for the same buffer are a no-op
  -- on our side, and the Rust side is also tolerant of double
  -- attach (it pushes a fresh overlay each call, but the active
  -- window overlay check below keeps this path from double-pushing
  -- into the same window).
  -- `tostring(buf)` is stable per BufferId (the metamethod
  -- formats the wrapped id), so it is usable inside the fallback
  -- active-window key. Prefer direct overlay inspection when
  -- available because window switches clear overlays while keeping
  -- the same window id.
  if not active_window_has_highlight() then
    local ok = pmacs.parse._attach_highlight(rec.buf, rec.lang)
    if ok then highlighted_windows[active_window_key(rec.buf)] = true end
  end
end

pmacs.hook.add("buffer.after-load", function()
  -- Best-effort: a missing grammar / re-entry / stale buffer
  -- mustn't poison the rest of the after-load chain.
  local ok, err = pcall(attach_for_active_buffer)
  if not ok and pmacs.error then
    pmacs.error("syntax.after-load: " .. tostring(err))
  end
end)

pmacs.hook.add("buffer.after-edit", function(buf)
  -- The Rust ParseView has already mirrored the edit and recorded an
  -- InputEdit by the time this hook fires. Coalesce here; the tick
  -- step below performs the dispatch so bursts of typing do not stack
  -- multiple parse jobs for the same buffer.
  local ok, err = pcall(mark_dirty, buf or pmacs.window.buffer())
  if not ok and pmacs.error then
    pmacs.error("syntax.after-edit: " .. tostring(err))
  end
end)

-- After-tick step: any parse job that has settled gets its bundle
-- installed into the buffer's view (and its pending entry drained).
-- Then any dirty buffer with no in-flight parse dispatches one parse.
-- Extension hook on top of the async runtime's tick rather than a
-- Rust-side tick callback because the install path is policy (which
-- view to install into), not mechanism.
local prior_tick = pmacs._async.tick
pmacs._async.tick = function(...)
  local ret = prior_tick(...)
  for job_id in pairs(pending_parse_jobs) do
    if pmacs._async._is_complete(job_id) then
      local key = job_to_buffer[job_id]
      local ok, installed = pcall(pmacs.parse._install_settled, job_id)
      pending_parse_jobs[job_id] = nil
      job_to_buffer[job_id] = nil
      if key and inflight_by_buffer[key] == job_id then
        inflight_by_buffer[key] = nil
      end
      if not ok and pmacs.error then
        pmacs.error("syntax.parse-install: " .. tostring(installed))
      end
      if key and installed and buffer_records[key] then
        local ok_pending, n = pcall(pmacs.parse._pending_edits,
          buffer_records[key].buf)
        if ok_pending and n and n > 0 then
          dirty_buffers[key] = buffer_records[key]
        end
      end
    end
  end
  for key, rec in pairs(dirty_buffers) do
    dispatch_parse_for(key, rec)
  end
  return ret
end
