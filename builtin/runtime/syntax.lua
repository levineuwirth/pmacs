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
-- Fresh-load language decision, including an explicit false sentinel for
-- "resolved none". Syntax, LSP, pairing, comments, and initial major mode all
-- consume this pin rather than re-sniffing mutable file content independently.
local detected_language_by_buffer = {}
-- Buffers already warned about hitting the injection layer cap (Q#IJ3);
-- keyed like the others so we warn once, and re-arm if the file stops
-- capping (an edit removed the excess regions).
local injection_cap_warned = {}

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

-- Injection language aliases (framing Q#IJ4). The registry holds the
-- merged map (seeded with defaults on the Rust side), and each dispatch
-- snapshots it into the parse request so the worker can resolve dynamic
-- fence names (`py` → python, `ts` → typescript). Exposed as a
-- write-through proxy so users add fence-name aliases from init.lua:
--   pmacs.parse.injection_aliases.mylang = "rust"
-- Reads are not proxied (the canonical map lives Rust-side); this is a
-- write-only extension surface.
pmacs.parse.injection_aliases = setmetatable({}, {
  __newindex = function(_, alias, lang)
    pmacs.parse._register_injection_alias(alias, lang)
  end,
})

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
    local tokens = {}
    for tok in rest:gmatch("%S+") do
      tokens[#tokens + 1] = tok
    end
    local i = 1
    while i <= #tokens do
      local tok = tokens[i]
      i = i + 1
      -- `-S`/`--split-string` introduces a complete env argument list,
      -- not necessarily an interpreter first: it may begin with more env
      -- options or `VAR=value` assignments. GNU env accepts that value
      -- ATTACHED — `-Spython3`, `-vSpython3` (after no-operand short flags
      -- i/v/0), or `--split-string=python3`. Put an attached payload back
      -- into this token stream so it goes through the same option/operand/
      -- assignment state machine as a separated payload.
      local split_attached =
        tok:match("^%-[iv0]*S(.+)$") or tok:match("^%-%-split%-string=(.+)$")
      if not seen_env then
        seen_env = true -- the `env` path token itself
      elseif skip_next then
        skip_next = false -- the operand consumed by the previous option
      elseif split_attached then
        table.insert(tokens, i, split_attached)
      elseif tok == "-S" or tok == "--split-string" then
        -- Separated split-string: the next token starts the string, i.e.
        -- another env argument — keep walking.
      elseif tok:find("=", 1, true) then
        -- `VAR=value` env assignment, or another `--long=value` option:
        -- self-contained, skip.
      elseif tok:sub(1, 1) == "-" then
        -- An option. A few GNU-env short options and their long forms
        -- consume the *next* token as an operand (`-u NAME`, `-C DIR`,
        -- `-a NAME`); skip that operand too, or its value is mistaken for
        -- the interpreter. An option with an attached operand (`-uNAME`)
        -- is one self-contained token and needs no skip.
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

-- Filename → language detection --------------------------------------------
--
-- Some files are identified by their whole BASENAME, not an extension:
-- `Dockerfile`, `Makefile`, `CMakeLists.txt`, and rc dotfiles like
-- `.bashrc`/`PKGBUILD` (highlighted by the already-bundled bash grammar).
-- Consulted after extension detection (a recognized extension still wins)
-- and before the shebang — a `Makefile` never carries a shebang, and the
-- basename is the more reliable signal. User-extensible from init.lua,
-- e.g. `pmacs.parse.filenames["Vagrantfile"] = "ruby"`.
pmacs.parse.filenames = pmacs.parse.filenames or {
  ["Dockerfile"] = "dockerfile",
  ["Containerfile"] = "dockerfile",
  ["Makefile"] = "make",
  ["makefile"] = "make",
  ["GNUmakefile"] = "make",
  ["BSDmakefile"] = "make",
  ["CMakeLists.txt"] = "cmake",
  -- Shell rc / config files → the bundled bash grammar.
  [".bashrc"] = "bash",
  [".bash_profile"] = "bash",
  [".bash_logout"] = "bash",
  [".profile"] = "bash",
  [".zshrc"] = "bash",
  [".zprofile"] = "bash",
  [".zshenv"] = "bash",
  ["PKGBUILD"] = "bash",
}

-- Language for a path's basename, or nil. `name` may be a full path.
function pmacs.parse.language_from_filename(name)
  if not name then return nil end
  local base = name:match("([^/]+)$") or name
  return pmacs.parse.filenames[base]
end

-- Modeline → language detection ---------------------------------------------

local MODELINE_WINDOW_BYTES = 8 * 1024
local VIM_MODELINE_LINES = 5
local MODELINE_NAME_BYTES = 128

pmacs.parse.modeline_aliases = pmacs.parse.modeline_aliases or {}
local default_modeline_aliases = {
  ["c++"] = "cpp",
  cxx = "cpp",
  sh = "bash",
  shell = "bash",
  ["shell-script"] = "bash",
  zsh = "bash",
  py = "python",
  js = "javascript",
  js2 = "javascript",
  jsx = "javascriptreact",
  ts = "typescript",
  tsx = "typescriptreact",
  yml = "yaml",
  makefile = "make",
  docker = "dockerfile",
  -- Lean 4 (framing Q#LN2). The grammar entry is named `lean4` because that
  -- name becomes the `didOpen` language_id, but an Emacs `-*- mode: lean -*-`
  -- or a Vim `ft=lean` line is what people actually write, so neither
  -- spelling strands a file.
  lean = "lean4",
}
for name, language in pairs(default_modeline_aliases) do
  if pmacs.parse.modeline_aliases[name] == nil then
    pmacs.parse.modeline_aliases[name] = language
  end
end

local function normalize_modeline_name(name)
  if type(name) ~= "string" then return nil end
  name = name:gsub("^[ \t]+", ""):gsub("[ \t]+$", ""):lower()
  if #name == 0 or #name > MODELINE_NAME_BYTES then return nil end
  if not name:match("^[a-z0-9][a-z0-9+_-]*$") then return nil end
  local alias = pmacs.parse.modeline_aliases[name]
  if alias == nil then return name end
  if type(alias) ~= "string" or #alias == 0 or #alias > MODELINE_NAME_BYTES then
    return nil
  end
  if not alias:match("^[a-z0-9][a-z0-9+_-]*$") then return nil end
  return alias
end

local function without_trailing_cr(line)
  if line:sub(-1) == "\r" then return line:sub(1, -2) end
  return line
end

-- Return the first five and last five complete logical lines. Each entry is
-- `{ text, offset }`, where offset is the zero-based buffer byte position.
-- The suffix's leading partial line is discarded and does not consume a slot.
local function modeline_edge_lines(buf)
  local ok_len, length = pcall(function() return buf:len() end)
  if not ok_len or not length or length <= 0 then return {}, {} end

  local prefix_end = math.min(length, MODELINE_WINDOW_BYTES)
  local ok_prefix, prefix = pcall(function() return buf:slice(0, prefix_end) end)
  if not ok_prefix or type(prefix) ~= "string" then return {}, {} end

  local front = {}
  local pos = 1
  while #front < VIM_MODELINE_LINES and pos <= #prefix do
    local newline = prefix:find("\n", pos, true)
    if not newline then
      if prefix_end < length then break end
      newline = #prefix + 1
    end
    front[#front + 1] = {
      text = without_trailing_cr(prefix:sub(pos, newline - 1)),
      offset = pos - 1,
    }
    if newline > #prefix then break end
    pos = newline + 1
  end

  local tail_start = 0
  local scan_start = 1
  if length > MODELINE_WINDOW_BYTES then
    -- Keep the one-byte line-boundary probe plus suffix content within the
    -- same 8 KiB read budget.
    tail_start = length - (MODELINE_WINDOW_BYTES - 1)
    local ok_probe, probe =
      pcall(function() return buf:slice(tail_start - 1, tail_start) end)
    if not ok_probe or probe ~= "\n" then
      scan_start = nil
    end
  end

  local tail = prefix
  if tail_start > 0 then
    local ok_tail
    ok_tail, tail = pcall(function() return buf:slice(tail_start, length) end)
    if not ok_tail or type(tail) ~= "string" then return front, front end
  end
  if scan_start == nil then
    local first_newline = tail:find("\n", 1, true)
    if not first_newline then return front, front end
    scan_start = first_newline + 1
  end

  local reverse_tail = {}
  local line_end = #tail
  if line_end >= scan_start and tail:byte(line_end) == 10 then
    line_end = line_end - 1
  end
  while line_end >= scan_start and #reverse_tail < VIM_MODELINE_LINES do
    local i = line_end
    while i >= scan_start and tail:byte(i) ~= 10 do
      i = i - 1
    end
    local line_start = i + 1
    reverse_tail[#reverse_tail + 1] = {
      text = without_trailing_cr(tail:sub(line_start, line_end)),
      offset = tail_start + line_start - 1,
    }
    line_end = i - 1
  end

  local edges = {}
  local seen = {}
  for _, entry in ipairs(front) do
    edges[#edges + 1] = entry
    seen[entry.offset] = true
  end
  for i = #reverse_tail, 1, -1 do
    local entry = reverse_tail[i]
    if not seen[entry.offset] then
      edges[#edges + 1] = entry
      seen[entry.offset] = true
    end
  end
  return front, edges
end

local function emacs_mode_on_line(entry, consider)
  local line = entry.text
  local search_from = 1
  while true do
    local open = line:find("-*-", search_from, true)
    if not open then return end
    local close = line:find("-*-", open + 3, true)
    if not close then return end
    local payload = line:sub(open + 3, close - 1)
    if payload:find(":", 1, true) then
      for part_at, part in payload:gmatch("()([^;]+)") do
        local key, value =
          part:match("^[ \t]*([%w_-]+)[ \t]*:[ \t]*(.-)[ \t]*$")
        if key and key:lower() == "mode" then
          local mode = normalize_modeline_name(value)
          if mode then consider(mode, entry.offset + open + part_at) end
        end
      end
    else
      local mode = normalize_modeline_name(payload)
      if mode then consider(mode, entry.offset + open) end
    end
    search_from = close + 3
  end
end

local function vim_assignment(token)
  return token:match("^ft=(.+)$") or token:match("^filetype=(.+)$")
end

local function vim_mode_at(entry, marker_at, marker, consider)
  local line = entry.text
  local rest_at = marker_at + #marker
  local rest = line:sub(rest_at)
  local full_set_at = rest:match("^[ \t]*set[ \t]+()")
  local option_at = full_set_at
  if not option_at and marker ~= "Vim:" then
    option_at = rest:match("^[ \t]*se[ \t]+()")
  end
  if marker == "Vim:" and not full_set_at then return end

  if option_at then
    local terminator = rest:find(":", option_at, true)
    if not terminator then return end
    local options = rest:sub(option_at, terminator - 1)
    for token_at, token in options:gmatch("()([^ \t]+)") do
      local value = vim_assignment(token)
      local mode = value and normalize_modeline_name(value)
      if mode then
        consider(mode, entry.offset + rest_at + option_at + token_at - 3)
      end
    end
    return
  end

  for token_at, token in rest:gmatch("()([^ \t:]+)") do
    local value = vim_assignment(token)
    local mode = value and normalize_modeline_name(value)
    if mode then consider(mode, entry.offset + rest_at + token_at - 2) end
  end
end

local function vim_modes_on_line(entry, consider)
  local line = entry.text
  for _, marker in ipairs({ "vim:", "vi:", "Vim:" }) do
    local search_from = 1
    while true do
      local marker_at = line:find(marker, search_from, true)
      if not marker_at then break end
      local previous = marker_at > 1 and line:sub(marker_at - 1, marker_at - 1)
      if marker_at == 1 or previous == " " or previous == "\t" then
        vim_mode_at(entry, marker_at, marker, consider)
      end
      search_from = marker_at + 1
    end
  end
end

function pmacs.parse.language_from_modeline(buf)
  if not buf then return nil end
  local front, edges = modeline_edge_lines(buf)
  local mode
  local mode_at = -1
  local function consider(candidate, candidate_at)
    if candidate_at >= mode_at then
      mode = candidate
      mode_at = candidate_at
    end
  end

  if front[1] then emacs_mode_on_line(front[1], consider) end
  if front[1] and front[1].text:sub(1, 2) == "#!" and front[2] then
    emacs_mode_on_line(front[2], consider)
  end
  for _, entry in ipairs(edges) do
    vim_modes_on_line(entry, consider)
  end
  return mode
end

-- Set of buffer ids that already have a highlight overlay
-- attached, keyed by raw id (number). A buffer that opens, gets
-- highlights, gets killed, and is reopened needs a fresh overlay
-- attach; the kill path clears the entry below if/when it lands.
local highlighted_buffers = {}

-- Fresh language inference for a buffer, in precedence order: explicit
-- modeline → grammar extension → LSP filetype map → filename → shebang.
-- Path components intentionally come from `buf:name()` to preserve syntax's
-- historical grammar-by-name behavior for pathless buffers.
local function detect_buffer_language(buf)
  local modeline = pmacs.parse.language_from_modeline(buf)
  if modeline then return modeline end
  local name = buf:name()
  if name then
    local grammar = pmacs.parse.language_for_path(name)
    if grammar then return grammar end
    local ext = name:match("%.([%w_]+)$")
    local by_ext = ext and pmacs.lsp and pmacs.lsp.filetypes and pmacs.lsp.filetypes[ext]
    if by_ext then return by_ext end
    local by_name = pmacs.parse.language_from_filename(name)
    if by_name then return by_name end
  end
  return pmacs.parse.language_from_shebang(buf)
end

local function refresh_buffer_language(buf)
  local language = detect_buffer_language(buf)
  detected_language_by_buffer[tostring(buf)] = language or false
  return language
end

function pmacs.parse.buffer_language(buf)
  if not buf then return nil end
  local key = tostring(buf)
  local language = detected_language_by_buffer[key]
  if language ~= nil then return language or nil end
  return refresh_buffer_language(buf)
end

local function attach_for_active_buffer(initialize_mode)
  local buf = pmacs.window.buffer()
  if not buf then return end
  local key = tostring(buf)
  -- A genuine load refreshes every detection signal and replaces the initial
  -- major mode, including with nil. Switches consume the pin: editing a
  -- shebang/modeline cannot silently swap parser/LSP language, while a
  -- registry-only hidden buffer still resolves when first visited.
  local lang = initialize_mode and refresh_buffer_language(buf)
    or pmacs.parse.buffer_language(buf)
  if initialize_mode then pmacs.buffer.set_major_mode(buf, lang) end
  -- The resolution chain can yield a valid mode with no grammar. Gate dispatch
  -- so custom/server-only modes stay quiet rather than raising "unknown
  -- language" from the parse worker.
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
  -- replacement for a `:id()` method we don't have to expose (`key` is
  -- computed once at the top of this function).
  if not highlighted_buffers[key] then
    local ok = pmacs.parse._attach_highlight(buf, lang)
    if ok then highlighted_buffers[key] = true end
  end
end

pmacs.hook.add("buffer.after-load", function()
  -- Best-effort: a missing grammar / re-entry / stale buffer
  -- mustn't poison the rest of the after-load chain.
  local ok, err = pcall(function() attach_for_active_buffer(true) end)
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
    attach_for_active_buffer(false)
  end)
  if not ok and pmacs.error then
    pmacs.error("syntax.after-switch: " .. tostring(err))
  end
end)

-- Major mode is window-local presentation state because each split may show
-- a different buffer. The provider therefore reads ctx.buffer rather than
-- the focused buffer. Empty text omits the segment without tripping the
-- statusline failure latch.
pmacs.statusline.register {
  name = "mode",
  side = "left",
  priority = 0,
  face = "ui.modeline",
  fn = function(ctx)
    local mode = pmacs.buffer.major_mode(ctx.buffer)
    if mode == nil then return "" end
    return "(" .. mode .. ")"
  end,
}

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
      -- Surface the injection layer backstop (Q#IJ3) once per buffer rather
      -- than dropping embedded regions silently. Best-effort: a missing
      -- buffer or error here must not stall the settle loop.
      local capped_buf = key and parse_buffer_by_key[key]
      if capped_buf and pmacs.parse._injection_capped(capped_buf) then
        if not injection_cap_warned[key] and pmacs.error then
          injection_cap_warned[key] = true
          pmacs.error(
            "syntax: injection layer cap reached; some embedded regions are unhighlighted")
        end
      elseif key then
        injection_cap_warned[key] = nil
      end
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
