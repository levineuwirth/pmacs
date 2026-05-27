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
-- step.
local pending_parse_jobs = {}
local parse_job_buffer_keys = {}
local inflight_parse_by_buffer = {}
local parse_buffer_by_key = {}
local parse_lang_by_buffer = {}
local reparse_requested_by_buffer = {}

local raw_dispatch = pmacs.parse._dispatch

-- Wrap `_dispatch` so every dispatched parse job lands in our
-- pending set. Calls into `_dispatch` from outside this file (e.g.
-- a hand-rolled user script) get the same tracking for free.
function pmacs.parse._dispatch(buf, lang)
  local key = tostring(buf)
  parse_buffer_by_key[key] = buf
  parse_lang_by_buffer[key] = lang
  local inflight = inflight_parse_by_buffer[key]
  if inflight then
    reparse_requested_by_buffer[key] = true
    return inflight
  end
  local job_id = raw_dispatch(buf, lang)
  pending_parse_jobs[job_id] = true
  parse_job_buffer_keys[job_id] = key
  inflight_parse_by_buffer[key] = job_id
  return job_id
end

-- Set of buffer ids that already have a highlight overlay
-- attached, keyed by raw id (number). A buffer that opens, gets
-- highlights, gets killed, and is reopened needs a fresh overlay
-- attach; the kill path clears the entry below if/when it lands.
local highlighted_buffers = {}

local function attach_for_active_buffer()
  local buf = pmacs.window.buffer()
  if not buf then return end
  local path = buf:name()
  if not path then return end
  local lang = pmacs.parse.language_for_path(path)
  if not lang then return end
  pmacs.parse._dispatch(buf, lang)
  -- T M4.3: install the syntax-highlight overlay for this buffer.
  -- Idempotent --- repeated calls for the same buffer are a no-op
  -- on our side, and the Rust side is also tolerant of double
  -- attach (it pushes a fresh overlay each call, but `attach_for
  -- _active_buffer` itself is gated by the `highlighted_buffers`
  -- table so we only push once per (buffer, after-load) cycle).
  -- `tostring(buf)` is stable per BufferId (the metamethod
  -- formats the wrapped id), so it's a safe table-key
  -- replacement for a `:id()` method we don't have to expose.
  local key = tostring(buf)
  if not highlighted_buffers[key] then
    local ok = pmacs.parse._attach_highlight(buf, lang)
    if ok then highlighted_buffers[key] = true end
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

local function reparse_active_buffer_after_edit()
  local buf = pmacs.window.buffer()
  if not buf then return end
  if not pmacs.parse._has_view(buf) then return end
  local pending = pmacs.parse._pending_edits(buf)
  if not pending or pending == 0 then return end
  local path = buf:name()
  if not path then return end
  local lang = pmacs.parse.language_for_path(path)
  if not lang then return end
  pmacs.parse._dispatch(buf, lang)
end

pmacs.hook.add("buffer.after-edit", function()
  -- `ParseView:on_edit` records incremental edits synchronously, but
  -- highlight overlays only see new spans after a fresh parse settles.
  local ok, err = pcall(reparse_active_buffer_after_edit)
  if not ok and pmacs.error then
    pmacs.error("syntax.after-edit: " .. tostring(err))
  end
end)

local function dispatch_follow_up_if_dirty(key)
  local buf = parse_buffer_by_key[key]
  local lang = parse_lang_by_buffer[key]
  if not buf or not lang then return end
  local pending = pmacs.parse._pending_edits(buf)
  local requested = reparse_requested_by_buffer[key]
  reparse_requested_by_buffer[key] = nil
  if requested or (pending and pending > 0) then
    pmacs.parse._dispatch(buf, lang)
  end
end

-- After-tick step: any parse job that has settled gets its bundle
-- installed into the buffer's view (and its pending entry drained).
-- Extension hook on top of the async runtime's tick rather than a
-- Rust-side tick callback because the install path is policy
-- (which view to install into), not mechanism.
local prior_tick = pmacs._async.tick
pmacs._async.tick = function(...)
  local ret = prior_tick(...)
  for job_id in pairs(pending_parse_jobs) do
    if pmacs._async._is_complete(job_id) then
      local key = parse_job_buffer_keys[job_id]
      pmacs.parse._install_settled(job_id)
      pending_parse_jobs[job_id] = nil
      parse_job_buffer_keys[job_id] = nil
      if key and inflight_parse_by_buffer[key] == job_id then
        inflight_parse_by_buffer[key] = nil
        dispatch_follow_up_if_dirty(key)
      end
    end
  end
  return ret
end
