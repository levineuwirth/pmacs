-- pmacs-mcp-prompts/init.lua --- T M9.7 prompts-as-result-buffers.
--
-- Public API:
--
--   local mcp_prompts = require("pmacs-mcp-prompts")
--   mcp_prompts.register(server)            -- fetch prompts/list, define each as a command
--   mcp_prompts.unregister(server)          -- drop the server's commands
--   mcp_prompts.commands_for(server)        -- list of registered command names
--   mcp_prompts.command_name(label, prompt) -- compute the normalized command name
--
-- Spec interpretation (T M9.7):
--
--   Invoking a prompt with arguments produces a *result buffer*; the
--   buffer's view interprets the result format (text / code / markdown).
--   Format determination is *explicit* via `_meta.format` on the
--   `prompts/get` response — the package does not infer format from
--   content. Recognized values: `"text"`, `"code"`, `"markdown"`.
--   Anything else (or absent) falls back to text rendering with a
--   logged warning. Servers that want their prompts rendered as code
--   or markdown set the field; servers that don't get plain text.
--
--   `_meta.language` accompanies `format = "code"` and names the
--   tree-sitter language to attach. `markdown` always uses the
--   `markdown` grammar.
--
-- ===========================================================================
-- IMPORTANT — `_meta.format` is a pmacs convention, not standard MCP.
-- ===========================================================================
--
-- The MCP spec (2025-11-25) defines `_meta` as a free-form metadata
-- bag for prompts/resources/tools but does not standardize a
-- `format` field. Pmacs's choice to key off `_meta.format` is
-- deliberate (explicit beats inferred) and documented here so that
-- the M9.10 deliverable (TRANSITION-M9.md and the
-- MCP-for-package-authors guide) can spread the rule:
--
--   * Existing real-world MCP servers (Anthropic's filesystem,
--     github, slack, etc., as of 2026-Q1) do NOT set `_meta.format`.
--     Every prompt from those servers therefore renders as
--     plain text. This is correct behavior, not a bug — it's the
--     "servers that don't, get plain text" branch above.
--   * Authors of *new* MCP servers that want their prompts to
--     render with code or markdown highlighting MUST set
--     `_meta.format = "code"` (with a `_meta.language`) or
--     `_meta.format = "markdown"` on each `prompts/get` response.
--   * If/when the upstream MCP spec adopts a format-hint field,
--     this package will switch to whatever the spec defines and
--     keep `_meta.format` as a backwards-compatibility fallback
--     until v2.0.
--
-- M9.10 owners: surface this rule in TRANSITION-M9.md and the
-- MCP-for-package-authors guide so package authors know the
-- contract before they ship a server.
-- ===========================================================================
--
-- The package consumes M9.5's notifications dispatcher with zero new
-- pmacs.mcp.* APIs — that's the M9.5 framing's structural property.
-- M9.7 ships exactly like M9.6: fixture-package + fake-server
-- extensions, no Rust API growth. The one v0.1 surface expansion is
-- the markdown grammar registered in `BUILTIN_LANGUAGES` (M11
-- measurement counts public Lua/Rust API; grammars and Cargo deps
-- are different dimensions).
--
-- Implementation notes:
--
--   * Command names follow `<server.label>-<prompt.name>` with both
--     halves passed through the same `[a-zA-Z0-9_.-]` allow-list (M9.6
--     finding 11). A label like `"my server!"` and a prompt named
--     `code/review` together produce `my-server--code-review`.
--   * Cross-source collisions (a builtin or another package owning
--     the normalized name) → set_status warn, route through
--     pmacs.error, skip the prompt; rest of the server's prompts
--     register cleanly. M9.6 finding 6 + 10 carry-forward.
--   * Required-arg flow: pmacs.minibuffer.read with chained on_accept
--     callbacks. Optional args are not prompted in v0.1. Prompt-arg
--     coercion does NOT apply (M9.6 finding 12 doesn't carry forward
--     to M9.7) — MCP's `prompts/list` returns arguments as
--     `{name, description?, required?}` triples with NO per-arg
--     schema, so all wire values are strings; coercion would have
--     nothing to coerce against.
--   * Result buffer named `*mcp:<label>:<prompt>*` (parallel to M9.5's
--     `*mcp:<uri>*`). Reused on re-invocation (M9.5 pattern); cursor
--     resets to (0, 0) and overlays clear via `switch_active_buffer`.
--     Read-only via M8 `add_intercept`, painted with a per-buffer
--     `painting` flag bypass (M9.5 prior art).
--   * Format dispatch: text / code / markdown. Code uses
--     `_meta.language`; markdown always uses the `markdown` grammar.
--     Unknown formats fall back to text with a warning.
--   * Multi-message rendering: each message gets a `## <role>`
--     level-2 header followed by content; messages are separated by
--     blank lines.
--   * Single-message rendering: render the content directly with no
--     role header. Matches the spec's "text-format prompt result
--     renders as a plain buffer" reading literally — a one-shot
--     prompt result does not need a `## user` ceremonial line.
--   * Non-text content (`type = "image"` / `"resource"`) renders as
--     `[image: <mimeType>]` / `[resource: <uri>]` placeholders
--     rather than being silently dropped.
--   * Reconciliation on `notifications/prompts/list_changed`: refetch
--     `prompts/list`, hash each advertised prompt, diff against the
--     registered set, register additions, unregister removals,
--     re-register schema changes. Hash is order-sensitive on the
--     required-args list (M9.6 finding 4 carry-forward) so a reorder
--     re-registers and the closure picks up the new prompt order.
--   * Server-gone teardown: dispatch detects "unknown server" /
--     "not ready for requests" on get_prompt failure and unregisters
--     (M9.6 finding 5 carry-forward) so dead commands don't linger
--     in pmacs.command.list().
--   * Notification-subscription refcount: subscribe iff
--     `_registered_count > 0`. M9.6 finding 3 carry-forward.

local M = {}

-- Per-server state, keyed by raw server id.
--   { label, prompts = { [prompt_name] = { command_name, hash } },
--     in_flight, rerun, cancelled }
local _by_server = {}

-- Per-buffer state for the read-only intercept + paint flag.
--
-- Keyed by `tostring(buf)`, which is stable per underlying BufferId
-- (the metamethod formats the wrapped id) — see `builtin/runtime/
-- syntax.lua`'s `highlighted_buffers` for the same pattern. NOT
-- keyed by the userdata itself: `pmacs.buffer.list()` and
-- `pmacs.window.buffer()` return fresh BufferIdLua userdata each
-- call, so a userdata-keyed table only finds the *first* wrapping
-- and silently misses every subsequent lookup. Result: `paint`
-- would return early on every re-invocation, the buffer would
-- never repaint, and the bug would only surface in tests that
-- assert buffer body content (not just buffer count). M9.8's
-- composition test (`m9_8_composes_with_m9_7_render_into_same_buffer`)
-- forces this — string keys are the fix.
local _buffer_state = {}

local function buffer_key(buf)
  return tostring(buf)
end

-- Refcount of currently-registered servers. Used to balance the
-- notification subscription so off_notification fires once the count
-- drops to zero (M9.6 finding 3).
local _registered_count = 0

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

local function server_id(server)
  -- McpServerIdLua exposes :raw() — see M9.6 finding 2 (don't parse
  -- digits from tostring).
  if type(server) ~= "userdata" and type(server) ~= "table" then
    error("pmacs-mcp-prompts: server handle must be McpServerIdLua, got "
      .. type(server))
  end
  local ok, raw = pcall(function() return server:raw() end)
  if not ok then
    error("pmacs-mcp-prompts: server:raw() failed; runtime contract "
      .. "broken (was the McpServerIdLua API renamed?): " .. tostring(raw))
  end
  return raw
end

local function server_label(server)
  for _, row in ipairs(pmacs.mcp.list()) do
    if row.id == server then
      return row.label or "unnamed"
    end
  end
  return "unnamed"
end

local function looks_like_server_gone(err)
  local s = type(err) == "table" and tostring(err.message or "") or tostring(err)
  return s:find("unknown server", 1, true) ~= nil
      or s:find("not ready for requests", 1, true) ~= nil
end

local function notify(msg)
  pmacs.editor.set_status(msg)
  if pmacs.error then
    pmacs.error("pmacs-mcp-prompts: " .. msg)
  end
end

local function normalize_char(c)
  if c:match("[%a%d_%.%-]") then return c end
  return "-"
end

-- Apply `normalize_char` byte-by-byte. Shared by `command_name` and
-- by the buffer-name builder below so the two surfaces can never
-- drift: the buffer that a command lands in has the same shape as
-- the command name.
local function normalize_half(s)
  local out = ""
  for i = 1, #s do
    out = out .. normalize_char(s:sub(i, i))
  end
  return out
end

function M.command_name(label, prompt_name)
  return normalize_half(label) .. "-" .. normalize_half(prompt_name)
end

-- Buffer name for a `<label>, <prompt>` pair. Both halves go through
-- `normalize_half` so a label like `"my server!"` and prompt
-- `"code/review"` produce `*mcp:my-server--code-review*` rather than
-- a name with raw spaces and slashes — and the buffer name stays
-- aligned with the command name `my-server--code-review`.
function M.buffer_name(label, prompt_name)
  return string.format("*mcp:%s:%s*", normalize_half(label), normalize_half(prompt_name))
end

-- Hash a prompt-list entry. Identity = name + description +
-- arguments-in-document-order (each arg's name + required-flag). The
-- hash is order-sensitive on `arguments` because the prompt-flow
-- closure captures the required-args sequence at register time, so a
-- reorder is a meaningful change that must trigger re-registration.
-- See M9.6 finding 4 for the audit history.
local function prompt_hash(entry)
  local parts = { entry.name or "", entry.description or "" }
  local args = entry.arguments
  if type(args) == "table" then
    for _, a in ipairs(args) do
      if type(a) == "table" then
        local req_flag = a.required == true and "1" or "0"
        parts[#parts + 1] = (a.name or "") .. ":" .. req_flag
      end
    end
  end
  return table.concat(parts, "|")
end

-- ---------------------------------------------------------------------------
-- Format-hint dispatch
-- ---------------------------------------------------------------------------

-- Recognized v0.1 formats. Anything else falls back to text with a
-- warning. The function is exposed as a test seam so the unknown-
-- format-falls-back test can pin the recognized list without going
-- through a server.
local _RECOGNIZED_FORMATS = { text = true, code = true, markdown = true }

function M._recognized_format(value)
  return _RECOGNIZED_FORMATS[value] == true
end

-- Read `_meta.format` and `_meta.language` from a prompts/get response.
-- Returns `(format, language, was_recognized)`. An absent `_meta`,
-- absent `format`, or `format == nil` defaults to "text" silently.
-- An *explicit* unknown format value (`_meta.format = "rtf"`) returns
-- `("text", nil, false)` so the caller can route the warning.
local function resolve_format(response)
  local meta = (type(response) == "table") and response._meta or nil
  if type(meta) ~= "table" then return "text", nil, true end
  local fmt = meta.format
  if fmt == nil then return "text", nil, true end
  if type(fmt) ~= "string" then return "text", nil, false end
  if not M._recognized_format(fmt) then return "text", nil, false end
  local lang = nil
  if fmt == "code" then
    if type(meta.language) == "string" and meta.language ~= "" then
      lang = meta.language
    end
  elseif fmt == "markdown" then
    lang = "markdown"
  end
  return fmt, lang, true
end

-- ---------------------------------------------------------------------------
-- Message rendering
-- ---------------------------------------------------------------------------

-- Render a single content entry to a string. Text content passes
-- through; image/resource content render as a placeholder line so the
-- shape is readable without binary noise. v0.1 only fully renders
-- text; image/resource fidelity is M9.8+ work.
local function render_content(content)
  if type(content) ~= "table" then return "" end
  local ct = content.type
  if ct == "text" then
    return tostring(content.text or "")
  elseif ct == "image" then
    local mt = content.mimeType or "?"
    return string.format("[image: %s]", mt)
  elseif ct == "resource" then
    -- Per the MCP spec the resource shape is `{ resource = { uri, ... } }`;
    -- be defensive about the inner field's location.
    local uri = "?"
    if type(content.resource) == "table" then
      uri = tostring(content.resource.uri or "?")
    elseif type(content.uri) == "string" then
      uri = content.uri
    end
    return string.format("[resource: %s]", uri)
  end
  return string.format("[%s: unsupported content type]", tostring(ct))
end

-- Build the body text from a `messages` array.
--
-- Single-message prompts (the common case for text-format prompts):
-- render the content directly, no role header — matches the spec's
-- "renders as a plain buffer" reading literally. A user reading a
-- one-shot prompt result does not need a `## user` ceremonial line.
--
-- Multi-message prompts: each message gets a `## <role>` level-2
-- header followed by a blank line + the rendered content. Messages
-- are separated by blank lines. Level-2 (rather than level-1) keeps
-- level-1 free for any actual title content the messages contain —
-- small consistency that pays off when markdown highlighting is
-- active.
function M._format_messages(messages)
  if type(messages) ~= "table" then return "" end
  if #messages == 1 then
    local msg = messages[1]
    if type(msg) == "table" then
      return render_content(msg.content)
    end
    return ""
  end
  local lines = {}
  for i, msg in ipairs(messages) do
    if type(msg) == "table" then
      local role = tostring(msg.role or "user")
      if i > 1 then lines[#lines + 1] = "" end
      lines[#lines + 1] = "## " .. role
      lines[#lines + 1] = ""
      lines[#lines + 1] = render_content(msg.content)
    end
  end
  return table.concat(lines, "\n")
end

-- ---------------------------------------------------------------------------
-- Result buffer
-- ---------------------------------------------------------------------------

local function make_readonly_intercept(buf)
  -- Capture the *key* (string) at intercept-attach time, not the
  -- userdata. The intercept fires on every buffer mutation, including
  -- ones routed through fresh userdata wrappings — keying by string
  -- means every fire resolves correctly.
  local key = buffer_key(buf)
  return function(_op)
    local s = _buffer_state[key]
    if s and s.painting then return nil end
    error("pmacs-mcp-prompts: result buffers are read-only; "
      .. "the buffer is repainted on prompt re-invocation.")
  end
end

local function paint(buf, text)
  local s = _buffer_state[buffer_key(buf)]
  if s == nil then return end
  s.painting = true
  local ok, err = pcall(function()
    buf:replace(0, buf:len(), text)
  end)
  s.painting = false
  if not ok then error(err) end
end

-- Look up an existing result buffer for `<label>:<prompt>` or create
-- one. On first creation, attaches the read-only intercept. Returns
-- the buffer handle.
local function find_or_create_result_buffer(label, prompt_name)
  local buf_name = M.buffer_name(label, prompt_name)
  for _, id in ipairs(pmacs.buffer.list()) do
    local d = pmacs.describe.buffer(id)
    if d ~= nil and d.name == buf_name then
      -- Existing buffer. State entry was inserted at original
      -- create time and is keyed by `tostring(buf)`, which is
      -- stable per underlying BufferId — so the fresh userdata
      -- returned here finds the same entry.
      return id
    end
  end
  local buf = pmacs.buffer.create(buf_name)
  _buffer_state[buffer_key(buf)] = { painting = false }
  pmacs.buffer.add_intercept(buf, make_readonly_intercept(buf))
  return buf
end

-- Render an MCP `prompts/get` response into a result buffer.
--
-- Public from M9.8 onward (was `render_result` internal in M9.7's
-- ship cut). Promoted to public on M9.8's request as the second
-- consumer — `pmacs-mcp-ai` composes with this rather than
-- duplicating ~80 LoC of rendering logic. This is the
-- "promote-on-second-consumer" discipline working as designed,
-- not a M9.7 oversight.
--
--   buf = mcp_prompts.render(label, prompt_name, response)
--
-- Arguments:
--
--   * `label`       — string used as the first half of the buffer
--                     name (`*mcp:<label>:<prompt>*`). For M9.8
--                     callers, pass the configured server's label
--                     so the buffer reuses the same slot M9.7's
--                     auto-registered command would use.
--   * `prompt_name` — second half of the buffer name; the MCP prompt
--                     name as advertised by `prompts/list`.
--   * `response`    — the *result* object from `pmacs.mcp.get_prompt`'s
--                     awaited handle (`{description?, _meta?, messages}`).
--                     Format dispatch reads `_meta.format` /
--                     `_meta.language`; missing or unrecognized
--                     formats fall back to text with a warning.
--
-- Returns the buffer handle. Side effects:
--
--   * Buffer `*mcp:<label>:<prompt>*` is created if absent or
--     repainted in place if present (cursor / region / scroll
--     reset via `switch_active_buffer`).
--   * Active window switches to the buffer.
--   * For `_meta.format = "code"` / `"markdown"`, tree-sitter
--     dispatch + highlight attach are pcall'd; an unknown language
--     falls through to text rendering with a notify.
--
-- Stability: this is the public surface M9.8 depends on. The shape
-- (label, prompt_name, response) → buffer is locked. Internal
-- behavior (which buffer name format, how unknown content types
-- render) may evolve in v0.2; the function-shape contract holds.
function M.render(label, prompt_name, response)
  local buf = find_or_create_result_buffer(label, prompt_name)
  local fmt, lang, recognized = resolve_format(response)
  if not recognized then
    local meta = (type(response) == "table") and response._meta or nil
    local raw = (type(meta) == "table") and tostring(meta.format) or "?"
    notify(string.format(
      "unknown format hint %q for %s; falling back to text",
      raw, prompt_name))
  end
  local body = M._format_messages(response and response.messages)
  if body == "" then body = "(empty result)" end
  paint(buf, body)
  -- Switch the active window to the result buffer. This resets cursor
  -- to (0,0), clears overlays, resets view_top — covering the Q4
  -- commitment (cursor reset, region cleared, scroll reset). Even if
  -- the user is already viewing this buffer, the switch re-resets.
  pmacs.window.switch_buffer(buf)
  -- Format-specific highlight attach. The `_attach_highlight` call
  -- requires the active window to be on this buffer, which is now
  -- guaranteed by the switch above.
  --
  -- pcall around `_dispatch` + `_attach_highlight`: a server can
  -- name any language (`_meta.language = "klingon"`); pmacs only
  -- ships grammars for languages registered in `BUILTIN_LANGUAGES`.
  -- An unknown language makes `_dispatch` throw "unknown language:
  -- <lang>". Without the pcall the throw escapes the surrounding
  -- async coroutine and the user sees a cryptic error for what is
  -- a routine "we don't have that grammar" condition. Fall back to
  -- text rendering and route a notify so the user knows why.
  if fmt == "code" or fmt == "markdown" then
    if lang ~= nil then
      local ok, err = pcall(function()
        pmacs.parse._dispatch(buf, lang)
        pmacs.parse._attach_highlight(buf, lang)
      end)
      if not ok then
        notify(string.format(
          "no grammar for %q (%s); rendered as text",
          lang, tostring(err)))
      end
    end
  end
  return buf
end

-- Internal alias retained so the dispatch path inside this package
-- doesn't pay the table-lookup cost on every prompt invocation.
-- Same function, just bound to a local for the hot path.
local render_result = M.render

-- ---------------------------------------------------------------------------
-- Argument prompting
-- ---------------------------------------------------------------------------

-- Forward declaration so dispatch can reach the public unregister
-- when it detects the server has gone away mid-flight (M9.6 finding 5).
local _unregister_for_teardown

local function dispatch(server, label, prompt_name, args)
  pmacs.async(function()
    local ok, response_or_err = pcall(function()
      return pmacs.mcp.get_prompt(server, prompt_name, args):await()
    end)
    if ok then
      render_result(label, prompt_name, response_or_err)
    else
      local msg
      if type(response_or_err) == "table" and type(response_or_err.message) == "string" then
        msg = response_or_err.message
      else
        msg = tostring(response_or_err)
      end
      pmacs.editor.set_status("MCP " .. prompt_name .. " error: " .. msg)
      if looks_like_server_gone(response_or_err) then
        local sid = server_id(server)
        if sid ~= nil and _by_server[sid] ~= nil then
          _unregister_for_teardown(server)
        end
      end
    end
  end)
end

local function prompt_chain(server, label, prompt_name, required, args, idx)
  if idx > #required then
    dispatch(server, label, prompt_name, args)
    return
  end
  local arg_name = required[idx]
  local prompt_text = string.format("%s %s: ", prompt_name, arg_name)
  pmacs.minibuffer.read {
    prompt = prompt_text,
    on_accept = function(value)
      args[arg_name] = value or ""
      prompt_chain(server, label, prompt_name, required, args, idx + 1)
    end,
    on_cancel = function()
      pmacs.editor.set_status("MCP " .. prompt_name .. ": cancelled")
    end,
  }
end

-- ---------------------------------------------------------------------------
-- Command body
-- ---------------------------------------------------------------------------

-- Pull required-arg names (in document order) out of a prompts/list
-- entry's `arguments` array. Each arg is `{name, description?, required?}`;
-- include only those with `required = true`. Returns an ordered Lua
-- array.
local function required_args(entry)
  local out = {}
  local args = entry.arguments
  if type(args) ~= "table" then return out end
  for _, a in ipairs(args) do
    if type(a) == "table" and a.required == true and type(a.name) == "string" then
      out[#out + 1] = a.name
    end
  end
  return out
end

local function make_command_body(server, label, prompt_name, entry_required)
  return function()
    if #entry_required == 0 then
      dispatch(server, label, prompt_name, {})
      return
    end
    prompt_chain(server, label, prompt_name, entry_required, {}, 1)
  end
end

-- ---------------------------------------------------------------------------
-- Schema rendering for describe-command
-- ---------------------------------------------------------------------------

local function render_prompt_doc(entry)
  local lines = {}
  local desc = entry.description
  if type(desc) ~= "string" or desc == "" then
    desc = "(no description)"
  end
  lines[#lines + 1] = desc
  local args = entry.arguments
  if type(args) == "table" and #args > 0 then
    lines[#lines + 1] = ""
    lines[#lines + 1] = "Arguments:"
    for _, a in ipairs(args) do
      if type(a) == "table" and type(a.name) == "string" then
        local req_tag = a.required == true and ", required" or ""
        local d = a.description or ""
        local suffix = (d ~= "" and (": " .. d)) or ""
        lines[#lines + 1] = "  " .. a.name .. " (string" .. req_tag .. ")" .. suffix
      end
    end
  end
  return table.concat(lines, "\n")
end

function M._render_prompt_doc(entry)
  return render_prompt_doc(entry)
end

function M._prompt_hash(entry)
  return prompt_hash(entry)
end

-- ---------------------------------------------------------------------------
-- Register / unregister
-- ---------------------------------------------------------------------------

local function fetch_prompts(server)
  local response = pmacs.mcp.send_request(server, "prompts/list", {}):await()
  local list = (type(response) == "table") and response.prompts or nil
  if type(list) ~= "table" then return {} end
  local out = {}
  for _, entry in ipairs(list) do
    if type(entry) == "table" and type(entry.name) == "string" then
      out[#out + 1] = entry
    end
  end
  return out
end

local function register_one(state, server, label, entry)
  local cmd_name = M.command_name(label, entry.name)
  -- Live in-package collision check (M9.6 finding 8 carry-forward).
  for pname, pentry in pairs(state.prompts) do
    if pentry.command_name == cmd_name and pname ~= entry.name then
      notify(string.format(
        "collision on %q (skipping %q)",
        cmd_name, entry.name))
      return
    end
  end
  -- Cross-source collision (M9.6 finding 6 + 10).
  if pmacs.command.exists(cmd_name) then
    notify(string.format(
      "command %q already defined (skipping %q)",
      cmd_name, entry.name))
    return
  end
  local req = required_args(entry)
  pmacs.command.define {
    name = cmd_name,
    description = render_prompt_doc(entry),
    fn = make_command_body(server, label, entry.name, req),
  }
  state.prompts[entry.name] = {
    command_name = cmd_name,
    hash = prompt_hash(entry),
  }
end

local function unregister_one(state, prompt_name)
  local entry = state.prompts[prompt_name]
  if entry == nil then return end
  pmacs.command.unregister(entry.command_name)
  state.prompts[prompt_name] = nil
end

local function apply_fresh(state, server, fresh)
  local fresh_by_name = {}
  for _, p in ipairs(fresh) do fresh_by_name[p.name] = p end
  local to_drop = {}
  for name, _ in pairs(state.prompts) do
    if fresh_by_name[name] == nil then to_drop[#to_drop + 1] = name end
  end
  -- Per-iteration cancellation check. The reconcile() entry point
  -- gates against cancellation between the fetch and the apply, but
  -- a long apply on a server with many prompts can overlap a fast
  -- unregister-then-shutdown. Bail out cleanly mid-loop rather than
  -- reviving commands the user has just dropped.
  for _, name in ipairs(to_drop) do
    if state.cancelled then return end
    unregister_one(state, name)
  end
  for _, p in ipairs(fresh) do
    if state.cancelled then return end
    local existing = state.prompts[p.name]
    if existing == nil then
      register_one(state, server, state.label, p)
    else
      local fresh_hash = prompt_hash(p)
      if fresh_hash ~= existing.hash then
        -- Schema changed. Unregister and re-register so the captured
        -- required-args closure picks up the new list. Keep unregister
        -- + register adjacent — see M9.6 finding 8 comment.
        unregister_one(state, p.name)
        register_one(state, server, state.label, p)
      end
    end
  end
end

local function reconcile(server)
  local sid = server_id(server)
  local state = _by_server[sid]
  if state == nil then return end
  if state.in_flight then
    state.rerun = true
    return
  end
  state.in_flight = true
  state.rerun = false
  pmacs.async(function()
    local ok, fresh_or_err = pcall(fetch_prompts, server)
    if state.cancelled or _by_server[sid] ~= state then
      state.in_flight = false
      return
    end
    if ok then
      apply_fresh(state, server, fresh_or_err)
    elseif looks_like_server_gone(fresh_or_err) then
      state.in_flight = false
      _unregister_for_teardown(server)
      return
    end
    state.in_flight = false
    if state.rerun and _by_server[sid] == state and not state.cancelled then
      reconcile(server)
    end
  end)
end

-- Test seams for the apply_fresh cancellation-during-apply
-- regression test (audit finding E2). Build a state with no
-- registrations, then drive apply_fresh with cancelled = true and
-- verify nothing landed. Leading-underscore: not stable surface.
function M._make_test_state(label)
  return { label = label, prompts = {}, in_flight = false, rerun = false, cancelled = false }
end

function M._apply_fresh(state, server, fresh)
  apply_fresh(state, server, fresh)
end

-- ---------------------------------------------------------------------------
-- Notification dispatcher (M9.5 third consumer — M9.5 + M9.6 + M9.7)
-- ---------------------------------------------------------------------------

local _notification_method = "notifications/prompts/list_changed"
local _notification_token = nil

local function ensure_notification_handler()
  if _notification_token ~= nil then return end
  _notification_token = pmacs.mcp.on_notification(
    _notification_method,
    function(server, _params)
      reconcile(server)
    end)
end

local function release_notification_handler()
  if _notification_token == nil then return end
  if _registered_count > 0 then return end
  pmacs.mcp.off_notification(_notification_method, _notification_token)
  _notification_token = nil
end

function M._has_notification_subscription()
  return _notification_token ~= nil
end

-- ---------------------------------------------------------------------------
-- Public API
-- ---------------------------------------------------------------------------

function M.register(server)
  local sid = server_id(server)
  if sid == nil then
    error("pmacs-mcp-prompts.register: server handle has no resolvable id")
  end
  if _by_server[sid] ~= nil then
    reconcile(server)
    return
  end
  ensure_notification_handler()
  local label = server_label(server)
  _by_server[sid] = {
    label = label,
    prompts = {},
    in_flight = false,
    rerun = false,
    cancelled = false,
  }
  _registered_count = _registered_count + 1
  reconcile(server)
end

function M.unregister(server)
  local sid = server_id(server)
  if sid == nil then return end
  local state = _by_server[sid]
  if state == nil then return end
  state.cancelled = true
  for name, _ in pairs(state.prompts) do
    pmacs.command.unregister(state.prompts[name].command_name)
  end
  _by_server[sid] = nil
  _registered_count = _registered_count - 1
  if _registered_count < 0 then _registered_count = 0 end
  release_notification_handler()
end

_unregister_for_teardown = M.unregister

function M.commands_for(server)
  local sid = server_id(server)
  local state = _by_server[sid]
  if state == nil then return {} end
  local out = {}
  for _, entry in pairs(state.prompts) do
    out[#out + 1] = entry.command_name
  end
  table.sort(out)
  return out
end

return M
