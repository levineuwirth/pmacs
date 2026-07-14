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

-- Shebang → language detection ------------------------------------------
--
-- Extension detection (`language_for_path`) misses extensionless scripts
-- (`scripts/deploy`, git hooks, `configure`), which are the common case
-- for shell. This fills that gap by sniffing the first line: `#!interp`
-- maps the interpreter's basename to a language. It is a *fallback* —
-- every caller tries extension detection first, so a `.py`/`.sh` file is
-- never re-classified by a stray shebang. Deliberately does not cover
-- special filenames (`.bashrc`, `Dockerfile`): those wait on grammars for
-- the languages behind them.
--
-- The map is user-extensible from init.lua, e.g.
-- `pmacs.parse.shebangs.ruby = "ruby"`. Only interpreters whose language
-- pmacs can act on (grammar and/or LSP config) are seeded; an entry whose
-- language has neither is harmless but inert.
pmacs.parse.shebangs = pmacs.parse.shebangs or {
  sh = "bash", bash = "bash", dash = "bash", ash = "bash",
  ksh = "bash", mksh = "bash", zsh = "bash",
  python = "python", python2 = "python", python3 = "python",
  pypy = "python", pypy3 = "python",
  node = "javascript", nodejs = "javascript",
  lua = "lua", luajit = "lua",
}

-- First line of `buf` (up to 256 bytes), sans trailing newline, or nil.
-- 256 bytes is well past any real shebang and keeps the slice cheap on
-- the hot path (callers only reach here when extension detection missed).
local function first_line(buf)
  local ok_len, n = pcall(function() return buf:len() end)
  if not ok_len or not n or n <= 0 then return nil end
  if n > 256 then n = 256 end
  local ok, s = pcall(function() return buf:slice(0, n) end)
  if not ok or type(s) ~= "string" or #s == 0 then return nil end
  local nl = s:find("\n", 1, true)
  if nl then s = s:sub(1, nl - 1) end
  return s
end

-- Resolve the interpreter basename from a shebang line, resolving the
-- `env` indirection (`#!/usr/bin/env python3` → `python3`, skipping
-- `env`'s own options / `VAR=val` assignments). Returns the language
-- name from `pmacs.parse.shebangs`, or nil.
function pmacs.parse.language_from_shebang(buf)
  if not buf then return nil end
  local line = first_line(buf)
  if not line or line:sub(1, 2) ~= "#!" then return nil end
  local rest = line:sub(3):gsub("^%s+", "")
  local first = rest:match("^(%S+)")
  if not first then return nil end
  local base = first:match("([^/]+)$") or first
  if base == "env" then
    base = nil
    local seen_env = false
    local skip_next = false
    for tok in rest:gmatch("%S+") do
      if not seen_env then
        seen_env = true -- the `env` path token itself
      elseif skip_next then
        skip_next = false -- the operand consumed by the previous option
      elseif tok:find("=", 1, true) then
        -- `VAR=value` env assignment, or a `--long=value` option: both
        -- self-contained, skip.
      elseif tok:sub(1, 1) == "-" then
        -- An option. A few GNU-env short options and their long forms
        -- consume the *next* token as an operand (`-u NAME`, `-C DIR`,
        -- `-a NAME`); skip that operand too, or its value is mistaken for
        -- the interpreter. `-S`/`--split-string` is deliberately absent:
        -- the string it introduces contains the interpreter, which the
        -- walk then picks up. An option with an attached operand
        -- (`-uNAME`) is one self-contained token and needs no skip.
        if tok == "-u" or tok == "-C" or tok == "-a"
            or tok == "--unset" or tok == "--chdir" or tok == "--argv0" then
          skip_next = true
        end
      else
        base = tok:match("([^/]+)$") or tok
        break
      end
    end
    if not base then return nil end
  end
  return pmacs.parse.shebangs[base]
end

-- Set of buffer ids that already have a highlight overlay
-- attached, keyed by raw id (number). A buffer that opens, gets
-- highlights, gets killed, and is reopened needs a fresh overlay
-- attach; the kill path clears the entry below if/when it lands.
local highlighted_buffers = {}

-- Filetype-aware language resolution for the active buffer, in
-- precedence order: grammar extension → LSP filetype map → shebang. The
-- shebang is consulted ONLY when the extension is unrecognized (a known
-- non-grammar extension like `.py` must not fall through to a stray
-- `#!/bin/sh` and be misparsed as bash). Keyed on `buf:name()` for the
-- extension parts (matching the historical behavior — path-less buffers
-- that resolve a grammar by name keep working); the shebang reads buffer
-- content directly.
local function resolve_active_language(buf)
  local name = buf:name()
  if name then
    local grammar = pmacs.parse.language_for_path(name)
    if grammar then return grammar end
    local ext = name:match("%.([%w_]+)$")
    local by_ext = ext and pmacs.lsp and pmacs.lsp.filetypes and pmacs.lsp.filetypes[ext]
    -- A recognized (even non-grammar) extension is authoritative; do not
    -- consult the shebang for it.
    if by_ext then return by_ext end
  end
  return pmacs.parse.language_from_shebang(buf)
end

local function attach_for_active_buffer()
  local buf = pmacs.window.buffer()
  if not buf then return end
  -- Gate dispatch on `_has_language`: the chain above also resolves
  -- languages with no grammar (python, javascript), and dispatching one
  -- would raise "unknown language" (caught, but noise) — and an
  -- extensionless script must never get a wrong-grammar parse tree.
  local lang = resolve_active_language(buf)
  if not lang or not pmacs.parse._has_language(lang) then return end
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

pmacs.hook.add("buffer.after-switch", function()
  -- Arc 1b: switching buffers clears the window's overlays
  -- (`switch_active_buffer` resets window view state), so the
  -- highlight view must be re-pushed for the now-active buffer.
  -- Dropping the `highlighted_buffers` entry first lets
  -- `attach_for_active_buffer` re-attach; the just-cleared window
  -- makes that exactly-once per switch. Without this, C-x b /
  -- panel navigation permanently stripped syntax color.
  local ok, err = pcall(function()
    local buf = pmacs.window.buffer()
    if not buf then return end
    highlighted_buffers[tostring(buf)] = nil
    attach_for_active_buffer()
  end)
  if not ok and pmacs.error then
    pmacs.error("syntax.after-switch: " .. tostring(err))
  end
end)

local function reparse_active_buffer_after_edit()
  local buf = pmacs.window.buffer()
  if not buf then return end
  if not pmacs.parse._has_view(buf) then return end
  local pending = pmacs.parse._pending_edits(buf)
  if not pending or pending == 0 then return end
  -- Reparse with the language pinned when the view was first attached ---
  -- never re-resolve from the path or (mutable) shebang. The Rust side is
  -- "first language wins"; re-sniffing a shebang the user just edited
  -- would either raise "unknown language" (sh → python) or swap the parse
  -- tree to a new grammar while the highlight overlay still holds the
  -- original grammar's query (sh → lua). Language changes need a
  -- close/reopen, exactly as they do for a renamed extension.
  local lang = parse_lang_by_buffer[tostring(buf)]
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
