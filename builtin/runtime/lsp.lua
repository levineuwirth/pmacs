-- builtin/runtime/lsp.lua --- T M4.12 default LSP integration.
--
-- Wires the LSP request/response surface into a usable editor UX:
--   * Declarative server config (`pmacs.lsp.config[language]`).
--   * Auto-attach + did_open / did_change / did_close on buffer events.
--   * `pmacs.lsp.go_to_definition` / `pmacs.lsp.format_buffer` /
--     `pmacs.lsp.hover_at_cursor` / `pmacs.lsp.signature_help_at_cursor`
--     / `pmacs.lsp.rename` / `pmacs.lsp.code_actions` /
--     `pmacs.lsp.inlay_hints` / `pmacs.lsp.semantic_tokens`, bound to
--     default chords below.
--
-- Scope: one server per language across all buffers; async-await
-- request/react (the editor never blocks). Landed: cross-file
-- go-to-definition (L1), multi-file rename / WorkspaceEdit applier
-- (L2), code actions + `workspace/executeCommand` + server→client
-- `workspace/applyEdit` (L3), ordered resource-op edits
-- (create/rename/delete file) with buffer-registry reconciliation
-- (L4), inlay hints, and semantic tokens (each data + modeline;
-- wiring them into rendering is a separate rendering milestone).
-- File-watch capability registration is a later layer.

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

-- Default Python config: basedpyright (an MIT fork of pyright that
-- re-enables inlay hints / semantic tokens in the open-source server,
-- which upstream pyright withholds for Pylance). `--stdio` is the
-- LSP transport. `settings` is answered to basedpyright's
-- `workspace/configuration` pull (pmacs now advertises that
-- capability) — `basic` keeps the diagnostics gutter from being
-- flooded by the fork's stricter defaults. A project's
-- `pyrightconfig.json` / `[tool.pyright]` still wins where present.
-- Users override any field from init.lua before a .py opens;
-- swapping to upstream pyright is just `command = "pyright-langserver"`.
pmacs.lsp.config.python = pmacs.lsp.config.python or {
  command = "basedpyright-langserver",
  args = { "--stdio" },
  settings = {
    python = { analysis = { typeCheckingMode = "basic" } },
    basedpyright = { analysis = { typeCheckingMode = "basic" } },
  },
}

-- C / C++ via clangd. One clangd binary serves both; `config.c` and
-- `config.cpp` are separate entries only so the `language_id` sent in
-- `didOpen` is accurate (clangd respects it). clangd takes its
-- project model from `compile_commands.json` / `compile_flags.txt`
-- at the project root, not `workspace/configuration`, so no
-- `settings` here; `--background-index` enables cross-file features.
-- Users override from init.lua before a C/C++ file opens.
pmacs.lsp.config.c = pmacs.lsp.config.c or {
  command = "clangd",
  args = { "--background-index" },
}
pmacs.lsp.config.cpp = pmacs.lsp.config.cpp or {
  command = "clangd",
  args = { "--background-index" },
}

-- Go via gopls. `gopls` with no args serves LSP over stdio. gopls
-- pulls its configuration via `workspace/configuration` (now
-- answered, #13) under the `gopls` section; an empty section means
-- "use defaults" — present, not null, which gopls prefers. Users
-- populate it (e.g. staticcheck, analyses) from init.lua.
pmacs.lsp.config.go = pmacs.lsp.config.go or {
  command = "gopls",
  args = {},
  settings = { gopls = {} },
}

-- LSP-side extension → language map, deliberately independent of the
-- tree-sitter detection in `pmacs.parse` (which is grammar-gated:
-- Python has an LSP server but no bundled grammar). Consulted only
-- when `pmacs.parse.language_for_path` finds nothing, so grammar-
-- backed languages keep their existing detection. Extensible from
-- init.lua: `pmacs.lsp.filetypes.foo = "bar"`.
pmacs.lsp.filetypes = pmacs.lsp.filetypes or {}
pmacs.lsp.filetypes.py = pmacs.lsp.filetypes.py or "python"
pmacs.lsp.filetypes.pyi = pmacs.lsp.filetypes.pyi or "python"
-- C. `.h` is ambiguous C/C++; default it to C (clangd copes either
-- way, and users can remap `pmacs.lsp.filetypes.h = "cpp"`).
pmacs.lsp.filetypes.c = pmacs.lsp.filetypes.c or "c"
pmacs.lsp.filetypes.h = pmacs.lsp.filetypes.h or "c"
-- C++.
for _, ext in ipairs({ "cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp", "inl", "cppm" }) do
  pmacs.lsp.filetypes[ext] = pmacs.lsp.filetypes[ext] or "cpp"
end
-- Go.
pmacs.lsp.filetypes.go = pmacs.lsp.filetypes.go or "go"

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
  -- Grammar-backed detection first (keeps rust/.rs etc. exactly as
  -- before); fall back to the LSP-only filetype map so languages
  -- with a server but no tree-sitter grammar (Python) still attach.
  local lang = pmacs.parse.language_for_path(path)
  if lang then return lang end
  local ext = path:match("%.([%w_]+)$")
  return ext and pmacs.lsp.filetypes[ext] or nil
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
    env = cfg.env,
    init_options = cfg.init_options,
    settings = cfg.settings,
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

-- Async request surface (T M4.5 async bridge). The Rust manager
-- registers each `textDocument/*` request with the async runtime and
-- returns a job id; the JSON-RPC response (or a server-teardown
-- drain) settles it. `_request_*_raw` is the raw job-id-returning
-- binding (mirrors `pmacs.mcp._send_request_raw`); the wrappers below
-- hand back a `pmacs.workers` Handle whose `:await()` resumes the
-- caller when the response lands. The pre-v1.0 `poll_until` tick-loop
-- this replaced blocked the editor for the whole request; awaiting
-- yields the coroutine instead.
local workers_mod = pmacs.workers
assert(workers_mod and workers_mod._new_handle,
  "pmacs.workers._new_handle missing; did async.lua load before lsp.lua?")
assert(pmacs.lsp._request_completion_raw,
  "pmacs.lsp._request_completion_raw missing; lua_bindings::install_lsp not run?")
local new_handle = workers_mod._new_handle

local function wrap_request(raw)
  -- Raises on dispatch failure (e.g. "server not ready"), matching
  -- `mcp.lua`'s wrapper; callers pcall the `request():await()` chain.
  return function(...)
    return new_handle(raw(...))
  end
end

pmacs.lsp.request_completion = wrap_request(pmacs.lsp._request_completion_raw)
pmacs.lsp.request_hover = wrap_request(pmacs.lsp._request_hover_raw)
pmacs.lsp.request_signature_help = wrap_request(pmacs.lsp._request_signature_help_raw)
pmacs.lsp.request_definition = wrap_request(pmacs.lsp._request_definition_raw)
pmacs.lsp.request_formatting = wrap_request(pmacs.lsp._request_formatting_raw)
pmacs.lsp.request_references = wrap_request(pmacs.lsp._request_references_raw)
pmacs.lsp.request_declaration = wrap_request(pmacs.lsp._request_declaration_raw)
pmacs.lsp.request_type_definition = wrap_request(pmacs.lsp._request_type_definition_raw)
pmacs.lsp.request_implementation = wrap_request(pmacs.lsp._request_implementation_raw)
pmacs.lsp.request_document_symbol = wrap_request(pmacs.lsp._request_document_symbol_raw)
pmacs.lsp.request_workspace_symbol = wrap_request(pmacs.lsp._request_workspace_symbol_raw)
pmacs.lsp.request_document_highlight = wrap_request(pmacs.lsp._request_document_highlight_raw)
pmacs.lsp.request_rename = wrap_request(pmacs.lsp._request_rename_raw)
pmacs.lsp.request_code_action = wrap_request(pmacs.lsp._request_code_action_raw)
pmacs.lsp.request_execute_command = wrap_request(pmacs.lsp._request_execute_command_raw)
pmacs.lsp.request_inlay_hint = wrap_request(pmacs.lsp._request_inlay_hint_raw)
pmacs.lsp.request_semantic_tokens = wrap_request(pmacs.lsp._request_semantic_tokens_raw)

-- Render an `:await()` failure into a modeline-friendly reason.
-- `Handle:await()` raises `{ tag = "cancelled", ... }` when the
-- server went away mid-request (drain in `lsp.rs`) and
-- `{ tag = "failed", message = ... }` for a JSON-RPC error response;
-- a raw dispatch failure surfaces as a plain string.
local function lsp_await_error(err)
  if type(err) == "table" then
    if err.tag == "cancelled" then
      return "server unavailable (request cancelled)"
    elseif err.tag == "failed" then
      return err.message or "server error"
    end
    return tostring(err.tag or "error")
  end
  return tostring(err)
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

-- T M4.5 L2/L4 — apply a parsed LSP `WorkspaceEdit` given as the
-- ordered op list `pmacs.rename.ops` / `code_action.edit` /
-- `_parse_workspace_edit` hand back: each entry is tagged `op` =
-- "edit" | "create" | "rename" | "delete". Order is the server's and
-- is honoured exactly, because the spec sequences ops (a `create`
-- must precede the `edit` that fills the new file).
--
-- Atomicity: a true cross-buffer/disk transaction is out of scope, so
-- the applier refuses to mutate *anything* unless every URI it
-- touches resolves to a real file path first (`path_for_uri`). An op
-- naming an `untitled:`/non-file document aborts the whole edit
-- cleanly, origin buffer untouched, rather than half-applying.
--
-- Text edits go through `apply_text_edits` (offsets resolved against
-- that buffer's *original* text, applied reverse-start) after
-- `find_or_open` makes the target active. Resource ops go through
-- `pmacs.buffer.apply_resource_op` (filesystem + buffer-registry
-- reconciliation). The buffer the user invoked from is restored last
-- (best-effort: it may itself have been renamed/deleted). Returns
-- `edit_count, file_count, resource_op_count` on success, or
-- `nil, message` if the preflight rejected the edit.
local function apply_workspace_edit(ops)
  local plan = {}
  for _, op in ipairs(ops or {}) do
    if op.op == "edit" then
      if op.edits and #op.edits > 0 then
        local path = pmacs.lsp.path_for_uri(op.uri)
        if not path then return nil, "cannot resolve " .. tostring(op.uri) end
        plan[#plan + 1] = { kind = "edit", path = path, edits = op.edits }
      end
    elseif op.op == "create" then
      local path = pmacs.lsp.path_for_uri(op.uri)
      if not path then return nil, "cannot resolve " .. tostring(op.uri) end
      plan[#plan + 1] = {
        kind = "create", path = path,
        overwrite = op.overwrite, ignore_if_exists = op.ignore_if_exists,
      }
    elseif op.op == "rename" then
      local from = pmacs.lsp.path_for_uri(op.old_uri)
      local to = pmacs.lsp.path_for_uri(op.new_uri)
      if not from or not to then
        return nil, "cannot resolve rename " ..
          tostring(op.old_uri) .. " -> " .. tostring(op.new_uri)
      end
      plan[#plan + 1] = {
        kind = "rename", old_path = from, new_path = to,
        overwrite = op.overwrite, ignore_if_exists = op.ignore_if_exists,
      }
    elseif op.op == "delete" then
      local path = pmacs.lsp.path_for_uri(op.uri)
      if not path then return nil, "cannot resolve " .. tostring(op.uri) end
      plan[#plan + 1] = {
        kind = "delete", path = path,
        recursive = op.recursive, ignore_if_not_exists = op.ignore_if_not_exists,
      }
    end
  end
  if #plan == 0 then return 0, 0, 0 end
  local origin = active_buffer_path()
  local edit_total, files, res_ops = 0, 0, 0
  for _, item in ipairs(plan) do
    if item.kind == "edit" then
      pmacs.buffer.find_or_open(item.path)
      edit_total = edit_total + apply_text_edits(item.edits)
      files = files + 1
    else
      pmacs.buffer.apply_resource_op(item)
      res_ops = res_ops + 1
    end
  end
  -- Return the user to where they invoked from — best-effort, since
  -- that path may have just been renamed or deleted.
  if origin then pcall(pmacs.buffer.find_or_open, origin) end
  return edit_total, files, res_ops
end

-- T M4.5 L3 — server→client `workspace/applyEdit` pump.
--
-- After a code action's `executeCommand`, servers (rust-analyzer,
-- gopls, …) deliver the actual change as a `workspace/applyEdit`
-- *request* — surfaced by the manager as a `request` event on the
-- server's event stream (the same "expose the request to the
-- consumer" path as `workspace/configuration`, minus the built-in
-- answer). We drain attachment servers' events each async tick, apply
-- any applyEdit through the shared applier, and reply `{ applied }`.
--
-- Only servers in `attachments` are drained, so a test (or package)
-- that owns its own directly-spawned server and reads its events
-- itself is unaffected. Server ids are snapshotted before the loop
-- because `apply_workspace_edit` → `find_or_open` can attach a new
-- buffer mid-iteration (mutating `attachments`).
local function handle_apply_edit_requests()
  local sids, seen = {}, {}
  for _, rec in pairs(attachments) do
    local sid = rec.server
    if sid then
      local k = tostring(sid)
      if not seen[k] then
        seen[k] = true
        sids[#sids + 1] = sid
      end
    end
  end
  for _, sid in ipairs(sids) do
    local ok, evs = pcall(pmacs.lsp.events_take, sid)
    if ok and evs then
      for _, ev in ipairs(evs) do
        if ev.kind == "request" and ev.method == "workspace/applyEdit" then
          local edit = ev.params and ev.params.edit
          local applied, reason = false, nil
          if edit then
            local parsed = pmacs.lsp._parse_workspace_edit(edit)
            local n, info = apply_workspace_edit(parsed.ops)
            if n then applied = true else reason = info end
          else
            reason = "missing edit"
          end
          local result = { applied = applied }
          if not applied then result.failureReason = tostring(reason) end
          pcall(pmacs.lsp.send_response, sid, ev.request_id, result)
        end
      end
    end
  end
end

if pmacs._async and pmacs._async.tick then
  local _prior_async_tick = pmacs._async.tick
  pmacs._async.tick = function(...)
    local ret = _prior_async_tick(...)
    pcall(handle_apply_edit_requests)
    return ret
  end
end

-- Commands ----------------------------------------------------------------

-- Each command captures the cursor/target at invocation time, then
-- spawns a coroutine that awaits the request and reacts. The editor
-- never blocks: the command function returns immediately and the
-- modeline updates when the response lands (or the await fails).
-- `:await()` sequences the work and surfaces server-gone / server-
-- error as structured errors; the normalized typed store (hybrid
-- model) is still the read path, so LSP result-shape variance
-- (Location | Location[] | LocationLink[], MarkupContent, …) stays
-- parsed in one place in Rust rather than re-derived here.

function pmacs.lsp.go_to_definition()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.definition.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_definition(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local locs = pmacs.definition.locations(rec.server, rec.uri)
    if not locs or #locs == 0 then
      pmacs.editor.set_status("LSP: no definition found")
      return
    end
    local first = locs[1]
    if first.uri == rec.uri then
      -- Same file: record the origin so M-, returns here, then move.
      pmacs.editor.push_jump()
      move_active_cursor_to(first.line, first.col)
      pmacs.editor.set_status(string.format(
        "LSP: definition at %d:%d", first.line + 1, first.col + 1))
    else
      -- Cross-file (SP-4): decode the URI, record the jump origin
      -- *before* switching away, open-or-reuse the target buffer,
      -- then position the cursor. `find_or_open` switches the active
      -- buffer and fires `buffer.after-load`, which attaches an LSP
      -- to the newly opened file.
      local path = pmacs.lsp.path_for_uri(first.uri)
      if not path then
        pmacs.editor.set_status(
          "LSP: cannot open non-file definition " .. first.uri)
        return
      end
      pmacs.editor.push_jump()
      local ok2, oerr = pcall(pmacs.buffer.find_or_open, path)
      if not ok2 then
        -- Open failed: drop the origin we just pushed so M-, isn't
        -- left pointing at a jump that never happened.
        pmacs.editor.jump_back()
        pmacs.editor.set_status(
          "LSP: failed to open " .. path .. ": " .. tostring(oerr))
        return
      end
      move_active_cursor_to(first.line, first.col)
      pmacs.editor.set_status(string.format(
        "LSP: definition at %s:%d:%d",
        path, first.line + 1, first.col + 1))
    end
  end)
end

function pmacs.lsp.find_references()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.references.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_references(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local locs = pmacs.references.locations(rec.server, rec.uri)
    if not locs or #locs == 0 then
      pmacs.editor.set_status("LSP: no references found")
      return
    end
    -- v1 surfaces a modeline summary (count + first hit); a
    -- references list buffer is future UX work, like the hover panel.
    local first = locs[1]
    pmacs.editor.set_status(string.format(
      "LSP: %d reference%s; first at %s:%d:%d",
      #locs, (#locs == 1 and "" or "s"),
      first.uri, first.line + 1, first.col + 1))
  end)
end

function pmacs.lsp.document_symbols()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.document_symbol.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_document_symbol(rec.server, rec.uri):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local syms = pmacs.document_symbol.symbols(rec.server, rec.uri)
    if not syms or #syms == 0 then
      pmacs.editor.set_status("LSP: no symbols")
      return
    end
    -- v1 modeline summary (count + first symbol); a structured
    -- outline buffer driven off this store is future UX work, like
    -- the references list and hover panel.
    local first = syms[1]
    pmacs.editor.set_status(string.format(
      "LSP: %d symbol%s; first '%s' at %d:%d",
      #syms, (#syms == 1 and "" or "s"),
      first.name, first.line + 1, first.col + 1))
  end)
end

-- T M4.5 — inlay hints for the whole buffer. Requests over a range
-- spanning the document, stores the parsed hints, and surfaces a
-- modeline summary (count + first). Inline virtual-text rendering is
-- a separate milestone (the cell-overlay model does not yet reflow
-- real glyphs around inserted columns); a render layer subscribes to
-- the same `pmacs.inlay_hint` store when it lands — same staged
-- approach as the references list / hover panel.
function pmacs.lsp.inlay_hints()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  -- Whole-document range: (0,0) .. (one past the last line, 0). An
  -- over-wide end line is fine — servers clamp to the document.
  local text = active_buffer_text()
  local nl = 0
  for _ in text:gmatch("\n") do nl = nl + 1 end
  pmacs.inlay_hint.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_inlay_hint(
        rec.server, rec.uri, 0, 0, nl + 1, 0):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local hints = pmacs.inlay_hint.hints(rec.server, rec.uri)
    if not hints or #hints == 0 then
      pmacs.editor.set_status("LSP: no inlay hints")
      return
    end
    local first = hints[1]
    pmacs.editor.set_status(string.format(
      "LSP: %d inlay hint%s; first '%s' at %d:%d",
      #hints, (#hints == 1 and "" or "s"),
      first.label, first.line + 1, first.col + 1))
  end)
end

-- T M4.5 — semantic tokens for the whole buffer. Requests
-- `textDocument/semanticTokens/full`, stores the decoded absolute
-- tokens, and surfaces a modeline summary (count + first token's
-- type, resolved through the server's legend). Data only: wiring
-- LSP tokens into styling (a second authority alongside tree-sitter)
-- is a separate rendering milestone — a render layer subscribes to
-- the same `pmacs.semantic_tokens` store when it lands.
function pmacs.lsp.semantic_tokens()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.semantic_tokens.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_semantic_tokens(rec.server, rec.uri):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local toks = pmacs.semantic_tokens.tokens(rec.server, rec.uri)
    if not toks or #toks == 0 then
      pmacs.editor.set_status("LSP: no semantic tokens")
      return
    end
    local first = toks[1]
    -- Resolve the type index through the legend (0-based index ->
    -- 1-based Lua array); fall back to the raw index if no legend.
    local legend = pmacs.semantic_tokens.legend(rec.server)
    local tname = legend and legend.token_types
      and legend.token_types[first.token_type + 1]
      or tostring(first.token_type)
    pmacs.editor.set_status(string.format(
      "LSP: %d semantic token%s; first '%s' at %d:%d",
      #toks, (#toks == 1 and "" or "s"),
      tname, first.line + 1, first.start + 1))
  end)
end

function pmacs.lsp.format_buffer()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  pmacs.formatting.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_formatting(rec.server, rec.uri, 4, true):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local edits = pmacs.formatting.edits(rec.server, rec.uri)
    if not edits or #edits == 0 then
      pmacs.editor.set_status("LSP: no formatting edits")
      return
    end
    local n = apply_text_edits(edits)
    pmacs.editor.set_status(string.format("LSP: applied %d edits", n))
  end)
end

-- T M4.5 L2 — rename the symbol under the cursor. Prompts for the new
-- name in the minibuffer; on accept, sends `textDocument/rename`,
-- awaits the `WorkspaceEdit`, and drives it through the multi-file
-- applier. The position is captured *before* the prompt opens so the
-- request still targets the original symbol even though the minibuffer
-- session moved focus.
function pmacs.lsp.rename()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.minibuffer.read {
    prompt = "Rename symbol to: ",
    on_cancel = function()
      pmacs.editor.set_status("LSP: rename cancelled")
    end,
    on_accept = function(new_name)
      if not new_name or new_name == "" then
        pmacs.editor.set_status("LSP: rename needs a new name")
        return
      end
      pmacs.rename.clear(rec.server, rec.uri)
      pmacs.async(function()
        local ok, err = pcall(function()
          pmacs.lsp.request_rename(rec.server, rec.uri, line, col, new_name):await()
        end)
        if not ok then
          pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
          return
        end
        local ops = pmacs.rename.ops(rec.server, rec.uri)
        if not ops or #ops == 0 then
          pmacs.editor.set_status("LSP: rename produced no edits")
          return
        end
        local n, files, res = apply_workspace_edit(ops)
        if not n then
          -- Preflight rejected it; nothing was mutated.
          pmacs.editor.set_status("LSP: rename aborted: " .. tostring(files))
          return
        end
        local msg = string.format(
          "LSP: renamed — %d edit%s across %d file%s",
          n, (n == 1 and "" or "s"),
          files, (files == 1 and "" or "s"))
        if res and res > 0 then
          msg = msg .. string.format(
            " (+%d file op%s)", res, (res == 1 and "" or "s"))
        end
        pmacs.editor.set_status(msg)
      end)
    end,
  }
end

-- T M4.5 L3 — code actions at the cursor. Requests the actions,
-- then applies the first one: an inline `edit` goes through the
-- shared WorkspaceEdit applier; a `command` is dispatched via
-- `workspace/executeCommand` (after which the server usually drives
-- the change with a server→client `workspace/applyEdit`, handled by
-- the pump installed below). A selection UI over multiple actions is
-- future UX work, like the references list and hover panel — v1
-- acts on the first and reports how many were offered.
function pmacs.lsp.code_actions()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.code_action.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_code_action(
        rec.server, rec.uri, line, col, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local acts = pmacs.code_action.actions(rec.server, rec.uri)
    if not acts or #acts == 0 then
      pmacs.editor.set_status("LSP: no code actions")
      return
    end
    local first = acts[1]
    local bits = {}
    if first.has_edit then
      local n, files, res = apply_workspace_edit(first.edit)
      if not n then
        pmacs.editor.set_status("LSP: code action aborted: " .. tostring(files))
        return
      end
      local b = string.format("%d edit(s) / %d file(s)", n, files)
      if res and res > 0 then b = b .. string.format(" / %d file op(s)", res) end
      table.insert(bits, b)
    end
    if first.command then
      local ok2, cerr = pcall(function()
        pmacs.lsp.request_execute_command(
          rec.server, first.command.command, first.command.arguments):await()
      end)
      if not ok2 then
        pmacs.editor.set_status("LSP: command failed: " .. lsp_await_error(cerr))
        return
      end
      table.insert(bits, "ran '" .. first.command.command .. "'")
    end
    local detail = (#bits > 0) and (" — " .. table.concat(bits, ", ")) or ""
    pmacs.editor.set_status(string.format(
      "LSP: code action '%s'%s (%d available)",
      first.title, detail, #acts))
  end)
end

function pmacs.lsp.hover_at_cursor()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.hover.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_hover(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local hover = pmacs.hover.current(rec.server, rec.uri)
    if not hover then
      pmacs.editor.set_status("LSP: no hover info")
      return
    end
    -- Surface the first line of the hover body in the modeline. The
    -- popup view subscribes to the same store; a panel can wire in
    -- here when the keybinding is meant to surface one.
    local first = (hover.contents or ""):match("^[^\n]*") or ""
    pmacs.editor.set_status(first ~= "" and ("LSP: " .. first) or "LSP: hover empty")
  end)
end

function pmacs.lsp.signature_help_at_cursor()
  local rec = attached_for_active()
  if not rec then
    pmacs.editor.set_status("LSP: no server for active buffer")
    return
  end
  local line = pmacs.editor.cursor_line()
  local col = pmacs.editor.cursor_col()
  pmacs.signature.clear(rec.server, rec.uri)
  pmacs.async(function()
    local ok, err = pcall(function()
      pmacs.lsp.request_signature_help(rec.server, rec.uri, line, col):await()
    end)
    if not ok then
      pmacs.editor.set_status("LSP: " .. lsp_await_error(err))
      return
    end
    local help = pmacs.signature.current(rec.server, rec.uri)
    if not help or not help.signatures or #help.signatures == 0 then
      pmacs.editor.set_status("LSP: no signature help")
      return
    end
    local active = help.signatures[(help.active_signature or 0) + 1]
    pmacs.editor.set_status(active and ("LSP: " .. active.label) or "LSP: signature unknown")
  end)
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

pmacs.command.define {
  name = "lsp.find-references",
  description = "Find references to the symbol under the cursor (LSP).",
  fn = pmacs.lsp.find_references,
}

pmacs.command.define {
  name = "lsp.document-symbols",
  description = "List the symbols (outline) of the active buffer (LSP).",
  fn = pmacs.lsp.document_symbols,
}

pmacs.command.define {
  name = "lsp.rename",
  description = "Rename the symbol under the cursor across the workspace (LSP).",
  fn = pmacs.lsp.rename,
}

pmacs.command.define {
  name = "lsp.code-actions",
  description = "Apply a code action for the symbol/range under the cursor (LSP).",
  fn = pmacs.lsp.code_actions,
}

pmacs.command.define {
  name = "lsp.inlay-hints",
  description = "Fetch inlay hints (inferred types / parameter names) for the buffer (LSP).",
  fn = pmacs.lsp.inlay_hints,
}

pmacs.command.define {
  name = "lsp.semantic-tokens",
  description = "Fetch semantic tokens (type-aware classification) for the buffer (LSP).",
  fn = pmacs.lsp.semantic_tokens,
}

-- T M4.5 L1 — unwind the cross-file jump ring. Pairs with the
-- `pmacs.editor.push_jump()` every navigation action records before
-- it moves the cursor.
pmacs.command.define {
  name = "lsp.jump-back",
  description = "Return to the location before the last LSP navigation jump.",
  fn = function()
    if not pmacs.editor.jump_back() then
      pmacs.editor.set_status("LSP: jump ring empty")
    end
  end,
}

-- Default chords. M-. follows the cross-editor convention for
-- go-to-definition; the others sit on `C-c` to keep printable letters
-- self-inserting. The user can override or unbind any of these from
-- init.lua.
pmacs.keymap.bind { scope = "global", sequence = "M-.",   command = "lsp.go-to-definition" }
pmacs.keymap.bind { scope = "global", sequence = "M-?",   command = "lsp.find-references" }
pmacs.keymap.bind { scope = "global", sequence = "M-,",   command = "lsp.jump-back" }
pmacs.keymap.bind { scope = "global", sequence = "C-c o", command = "lsp.document-symbols" }
pmacs.keymap.bind { scope = "global", sequence = "C-c r", command = "lsp.rename" }
pmacs.keymap.bind { scope = "global", sequence = "C-c a", command = "lsp.code-actions" }
pmacs.keymap.bind { scope = "global", sequence = "C-c i", command = "lsp.inlay-hints" }
pmacs.keymap.bind { scope = "global", sequence = "C-c y", command = "lsp.semantic-tokens" }
pmacs.keymap.bind { scope = "global", sequence = "C-c h", command = "lsp.hover" }
pmacs.keymap.bind { scope = "global", sequence = "C-c s", command = "lsp.signature-help" }
pmacs.keymap.bind { scope = "global", sequence = "C-c f", command = "lsp.format-buffer" }
