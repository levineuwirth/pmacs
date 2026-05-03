-- builtin/runtime/lsp.lua --- T M4.12 default LSP integration.
--
-- Wires the LSP request/response surface into a usable editor UX:
--   * Declarative server config (`pmacs.lsp.config[language]`).
--   * Auto-attach + did_open / did_change / did_close on buffer events.
--   * `pmacs.lsp.go_to_definition` / `pmacs.lsp.format_buffer` /
--     `pmacs.lsp.hover_at_cursor` / `pmacs.lsp.signature_help_at_cursor`,
--     bound to default chords below.
--
-- v0.1 scope: one server per language across all buffers; same-file
-- definition jumps only; synchronous request → poll → react cycle
-- (sub-second for hot servers). Cross-file navigation, async-await
-- coroutines, rename, code actions, inlay hints, semantic tokens, and
-- file-watch capability registration are all v0.2 work.

pmacs.lsp = pmacs.lsp or {}
pmacs.lsp.config = pmacs.lsp.config or {}

-- Default rust-analyzer config. Users replace any field from init.lua
-- before any rust file opens.
pmacs.lsp.config.rust = pmacs.lsp.config.rust or {
  command = "rust-analyzer",
  args = {},
  init_options = {
    cargo = { allFeatures = true },
    checkOnSave = { command = "clippy" },
    procMacro = { enable = true },
  },
}

-- Per-buffer attachment record: { language, server, uri, version }.
-- Keyed by `tostring(BufferIdLua)` because BufferIdLua hands out fresh
-- userdata each call (so two handles to the same buffer wouldn't hash
-- equal as raw keys).
local attachments = {}

-- Minimal file:// percent-encoder. Matches src/lsp.rs's policy: ASCII
-- alpha-num + a small set of path-safe punctuation pass through; every
-- other byte goes through %XX. Iterates per-byte (`gmatch(".")` is
-- byte-wise) so multibyte UTF-8 components encode cleanly.
local function file_uri_for(path)
  if not path then return nil end
  local out = "file://"
  for ch in path:gmatch(".") do
    local b = string.byte(ch)
    if (b >= 48 and b <= 57)         -- 0-9
        or (b >= 65 and b <= 90)     -- A-Z
        or (b >= 97 and b <= 122)    -- a-z
        or b == 47 or b == 45 or b == 95
        or b == 46 or b == 126 or b == 58 then
      out = out .. ch
    else
      out = out .. string.format("%%%02X", b)
    end
  end
  return out
end

local function active_buffer_text()
  local b = pmacs.window.buffer()
  if not b then return "" end
  return b:slice(0, b:len())
end

local function active_buffer_path()
  return pmacs.editor.file_path()
end

local function active_buffer_language()
  local path = active_buffer_path()
  if not path then return nil end
  return pmacs.parse.language_for_path(path)
end

local function ensure_server(language)
  local cfg = pmacs.lsp.config[language]
  if not cfg or not cfg.command then return nil end
  -- Reuse an existing same-language server if one is up. Multi-root
  -- scoping (one server per project root) ships post-v0.1.
  for _, info in ipairs(pmacs.lsp.list()) do
    if info.language_id == language and info.state then
      local kind = info.state.kind
      if kind ~= "crashed" and kind ~= "stopped" then
        return info.id
      end
    end
  end
  local ok, sid = pcall(pmacs.lsp.spawn, {
    label = "default-" .. language,
    language_id = language,
    command = cfg.command,
    args = cfg.args or {},
    init_options = cfg.init_options,
  })
  if ok then return sid end
  return nil
end

-- True iff `sid` is still registered with the manager and isn't dead.
-- Stale attachments — left behind by a server that crashed, was
-- forgotten, or was spawned against a now-replaced `pmacs.lsp.config`
-- entry — get rebuilt on the next attach attempt.
local function server_is_live(sid)
  if not sid then return false end
  for _, info in ipairs(pmacs.lsp.list()) do
    if tostring(info.id) == tostring(sid) then
      local kind = info.state and info.state.kind
      return kind ~= "crashed" and kind ~= "stopped"
    end
  end
  return false
end

local function attach_buffer(buf)
  if not buf then return nil end
  local key = tostring(buf)
  local existing = attachments[key]
  if existing and server_is_live(existing.server) then return existing end
  if existing then attachments[key] = nil end
  local language = active_buffer_language()
  if not language then return nil end
  local sid = ensure_server(language)
  if not sid then return nil end
  local path = active_buffer_path()
  local uri = file_uri_for(path)
  if not uri then return nil end
  local rec = { language = language, server = sid, uri = uri, version = 1 }
  attachments[key] = rec
  -- did_open is a notification; the manager queues it cleanly even
  -- while the server is in `starting` / `initializing`.
  pcall(pmacs.lsp.did_open, sid, uri, rec.version, active_buffer_text())
  return rec
end

local function attached_for_active()
  local buf = pmacs.window.buffer()
  if not buf then return nil end
  return attachments[tostring(buf)] or attach_buffer(buf)
end

-- Hooks --------------------------------------------------------------------

pmacs.hook.add("buffer.after-load", function()
  pcall(attach_buffer, pmacs.window.buffer())
end)

pmacs.hook.add("buffer.after-edit", function()
  local buf = pmacs.window.buffer()
  if not buf then return end
  local rec = attachments[tostring(buf)]
  if not rec then return end
  rec.version = rec.version + 1
  pcall(pmacs.lsp.did_change, rec.server, rec.uri, rec.version, active_buffer_text())
end)

-- Synchronous poll: tick the supervisor + manager in tight loops until
-- `predicate()` returns a truthy value or the deadline elapses. Uses
-- wall-clock `pmacs.now_ms` because `os.clock` counts CPU time, and
-- the inner ticks block on subprocess I/O instead of burning CPU.
-- The v0.2 LSP UX pass swaps this for async-await coroutines.
local function poll_until(predicate, timeout_ms)
  timeout_ms = timeout_ms or 250
  local deadline = pmacs.now_ms() + timeout_ms
  while pmacs.now_ms() < deadline do
    pmacs.process._tick()
    pmacs.lsp._tick()
    local ok, value = pcall(predicate)
    if ok and value then return value end
  end
  return nil
end

-- Cursor positioning ------------------------------------------------------
--
-- LSP positions are 0-based (line, character). pmacs.editor.cursor_line
-- and pmacs.editor.cursor_col already return 0-based byte counts; for
-- ASCII / UTF-8 text without astral codepoints, byte == UTF-16 code
-- unit, which is what every shipped server actually accepts. Multi-byte
-- conversion lands with the v0.2 LSP hardening pass.

local function move_active_cursor_to(line, col)
  -- Walk via primitives so all overlay observers see the navigation.
  pmacs.editor.move_line_start()
  -- Move to row 0 first, then step down `line` rows.
  while pmacs.editor.cursor_line() > 0 do
    pmacs.editor.move_up()
  end
  for _ = 1, line do pmacs.editor.move_down() end
  for _ = 1, col do pmacs.editor.move_right() end
end

-- Compute the byte offset of (line, col) within `text` where lines are
-- separated by `\n`. Used by `apply_text_edits` to map LSP coordinates
-- to byte positions on the rope.
local function byte_offset_for(text, line, col)
  if line == 0 then return col end
  local pos = 0
  local current_line = 0
  while current_line < line do
    local nl = text:find("\n", pos + 1, true)
    if not nl then return #text end
    pos = nl
    current_line = current_line + 1
  end
  return pos + col
end

local function apply_text_edits(edits)
  if not edits or #edits == 0 then return 0 end
  local buf = pmacs.window.buffer()
  if not buf then return 0 end
  local text = active_buffer_text()
  -- Resolve every edit against the *original* text, then sort by start
  -- byte descending so each replacement leaves earlier offsets valid.
  local resolved = {}
  for _, e in ipairs(edits) do
    table.insert(resolved, {
      start = byte_offset_for(text, e.start_line, e.start_col),
      stop  = byte_offset_for(text, e.end_line, e.end_col),
      text  = e.new_text,
    })
  end
  table.sort(resolved, function(a, b) return a.start > b.start end)
  for _, e in ipairs(resolved) do
    if e.start == e.stop then
      buf:insert(e.start, e.text)
    elseif e.text == "" then
      buf:delete(e.start, e.stop)
    else
      buf:replace(e.start, e.stop, e.text)
    end
  end
  return #resolved
end

-- Commands ----------------------------------------------------------------

function pmacs.lsp.go_to_definition()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.definition.clear(rec.server, rec.uri)
  local ok = pcall(pmacs.lsp.request_definition, rec.server, rec.uri, line, col)
  if not ok then
    pmacs.editor.set_status("LSP: server not ready")
    return
  end
  local locs = poll_until(function()
    local ls = pmacs.definition.locations(rec.server, rec.uri)
    if #ls > 0 then return ls end
    return nil
  end, 500)
  if not locs then
    pmacs.editor.set_status("LSP: no definition found")
    return
  end
  local first = locs[1]
  if first.uri == rec.uri then
    move_active_cursor_to(first.line, first.col)
    pmacs.editor.set_status(string.format(
      "LSP: definition at %d:%d", first.line + 1, first.col + 1))
  else
    pmacs.editor.set_status("LSP: definition lives in " .. first.uri)
  end
end

function pmacs.lsp.format_buffer()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.formatting.clear(rec.server, rec.uri)
  local ok = pcall(pmacs.lsp.request_formatting, rec.server, rec.uri, 4, true)
  if not ok then
    pmacs.editor.set_status("LSP: server not ready")
    return
  end
  local edits = poll_until(function()
    local es = pmacs.formatting.edits(rec.server, rec.uri)
    if #es > 0 then return es end
    return nil
  end, 1000)
  if not edits then
    pmacs.editor.set_status("LSP: no formatting edits")
    return
  end
  local n = apply_text_edits(edits)
  pmacs.editor.set_status(string.format("LSP: applied %d edits", n))
end

function pmacs.lsp.hover_at_cursor()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.hover.clear(rec.server, rec.uri)
  local ok = pcall(pmacs.lsp.request_hover, rec.server, rec.uri,
    pmacs.editor.cursor_line(), pmacs.editor.cursor_col())
  if not ok then
    pmacs.editor.set_status("LSP: server not ready")
    return
  end
  local hover = poll_until(function()
    return pmacs.hover.current(rec.server, rec.uri)
  end, 500)
  if not hover then
    pmacs.editor.set_status("LSP: no hover info")
    return
  end
  -- v0.1 surfaces the first line of the hover body in the modeline. The
  -- popup view subscribes to the same store; future work can wire one
  -- in here when the keybinding is meant to surface a panel.
  local first = (hover.contents or ""):match("^[^\n]*") or ""
  pmacs.editor.set_status(first ~= "" and ("LSP: " .. first) or "LSP: hover empty")
end

function pmacs.lsp.signature_help_at_cursor()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.signature.clear(rec.server, rec.uri)
  local ok = pcall(pmacs.lsp.request_signature_help, rec.server, rec.uri,
    pmacs.editor.cursor_line(), pmacs.editor.cursor_col())
  if not ok then
    pmacs.editor.set_status("LSP: server not ready")
    return
  end
  local help = poll_until(function()
    return pmacs.signature.current(rec.server, rec.uri)
  end, 500)
  if not help or not help.signatures or #help.signatures == 0 then
    pmacs.editor.set_status("LSP: no signature help")
    return
  end
  local active = help.signatures[(help.active_signature or 0) + 1]
  pmacs.editor.set_status(active and ("LSP: " .. active.label) or "LSP: signature unknown")
end

-- Default commands + keymap entries --------------------------------------

pmacs.command.define {
  name = "lsp.go-to-definition",
  description = "Jump to the definition of the symbol under the cursor (LSP).",
  fn = pmacs.lsp.go_to_definition,
}

pmacs.command.define {
  name = "lsp.format-buffer",
  description = "Format the active buffer through the attached LSP server.",
  fn = pmacs.lsp.format_buffer,
}

pmacs.command.define {
  name = "lsp.hover",
  description = "Surface the hover documentation for the symbol under the cursor.",
  fn = pmacs.lsp.hover_at_cursor,
}

pmacs.command.define {
  name = "lsp.signature-help",
  description = "Surface the signature of the function call at the cursor.",
  fn = pmacs.lsp.signature_help_at_cursor,
}

-- Default chords. M-. follows the cross-editor convention for
-- go-to-definition; the others sit on `C-c` to keep printable letters
-- self-inserting. The user can override or unbind any of these from
-- init.lua.
pmacs.keymap.bind { scope = "global", sequence = "M-.",   command = "lsp.go-to-definition" }
pmacs.keymap.bind { scope = "global", sequence = "C-c h", command = "lsp.hover" }
pmacs.keymap.bind { scope = "global", sequence = "C-c s", command = "lsp.signature-help" }
pmacs.keymap.bind { scope = "global", sequence = "C-c f", command = "lsp.format-buffer" }
