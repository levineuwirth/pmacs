-- help.lua --- the discovery command family (P4 Stage 1).
-- Framing: docs/archive/framings/discovery-stage1-command-family-framing.md.
--
-- `COHERENCE.md` §5 grades discoverability "substrate without surface —
-- the sharpest instance of §1.1": the registries already carry
-- descriptions, source locations and reverse key lookup, and almost none
-- of it was reachable. This file is the surface. It adds NO Rust: every
-- command below renders data `pmacs.describe.*`, `pmacs.keymap.list()`,
-- `pmacs.command.list()` and `pmacs.config.list()` already return.
--
-- ORDERING CONTRACT: loads after `commands/default.lua` (for
-- `pmacs.editor._show_help` and the two renamed commands it forwards to)
-- and after `runtime/welcome.lua` (whose `pmacs.welcome.entries` the
-- index reads).
--
-- TWO DISCIPLINES THIS FILE KEEPS
--
-- 1. **One owner for `*help*` writes.** Every command renders through
--    `pmacs.editor._show_help` and never touches a buffer itself. That
--    does NOT make a later migration to `src/help.rs` a one-site change
--    — that layer has renderers for command/key/buffer/mode/hook/view
--    and none for settings, lists or apropos, and `_show_help` takes
--    already-flattened text. What the funnel buys is the shared policy
--    in one place: reuse-by-name, wholesale replacement, the `q`
--    binding, and the foreign-`*help*` hazard (found-by-name is not
--    ownership — a user's own `*help*` is adopted and cleared; the
--    missing guarantee is ownership identity, which `listview` has as
--    `panels` and dired as its handle table, and this does not).
--
-- 2. **Rendering is a named per-subject function**, and the command body
--    does nothing but call it and hand the result to `_show_help`. That
--    keeps the semantics addressable, so the future help-unification
--    stage is enumerated per subject — replace the four `src/help.rs`
--    already covers, write three new Rust renderers for settings, lists
--    and apropos — rather than discovered per call site.

pmacs.help = pmacs.help or {}

local function show(text)
  pmacs.editor._show_help(text)
end

-- Sorted command names. `pmacs.command.list()` returns registration
-- order, which is not meaningful to a reader.
local function sorted_command_names()
  local names = {}
  for _, n in ipairs(pmacs.command.list()) do names[#names + 1] = n end
  table.sort(names)
  return names
end

local function description_of(name)
  local ok, info = pcall(pmacs.describe.command, name)
  if ok and type(info) == "table" and type(info.description) == "string" then
    return info.description
  end
  return "(no description)"
end

-- ---------------------------------------------------------------------
-- Per-subject renderers (discipline 2)
-- ---------------------------------------------------------------------

function pmacs.help.render_key(seq, info)
  if type(info) ~= "table" then
    return string.format("Key: %s\n\n  (unbound in this buffer)\n", seq)
  end
  local lines = {
    "Key: " .. seq,
    "",
    "  Command:     " .. tostring(info.command),
    "  Scope:       " .. tostring(info.scope),
  }
  if info.source then lines[#lines + 1] = "  Source:      " .. tostring(info.source) end
  lines[#lines + 1] = ""
  lines[#lines + 1] = description_of(info.command)
  return table.concat(lines, "\n") .. "\n"
end

function pmacs.help.render_mode(info)
  if type(info) ~= "table" then return "Mode: (none)\n" end
  local lines = { "Mode: " .. tostring(info.name or "(none)"), "" }
  for k, v in pairs(info) do
    if k ~= "name" then
      lines[#lines + 1] = string.format("  %-12s %s", k .. ":", tostring(v))
    end
  end
  return table.concat(lines, "\n") .. "\n"
end

function pmacs.help.render_buffer(info)
  if type(info) ~= "table" then return "Buffer: (none)\n" end
  local lines = { "Buffer: " .. tostring(info.name), "" }
  for _, k in ipairs({ "path", "major_mode", "modified", "read_only", "length" }) do
    if info[k] ~= nil then
      lines[#lines + 1] = string.format("  %-12s %s", k .. ":", tostring(info[k]))
    end
  end
  return table.concat(lines, "\n") .. "\n"
end

function pmacs.help.render_hook(name, info)
  local lines = { "Hook: " .. name, "" }
  if type(info) ~= "table" then
    lines[#lines + 1] = "  (no listeners)"
    return table.concat(lines, "\n") .. "\n"
  end
  lines[#lines + 1] = string.format("  %-12s %s", "kind:", tostring(info.kind))
  local listeners = info.listeners
  if type(listeners) == "table" then
    lines[#lines + 1] = string.format("  %-12s %d", "listeners:", #listeners)
    for _, l in ipairs(listeners) do
      local src = (type(l) == "table" and l.source) or l
      lines[#lines + 1] = "    " .. tostring(src)
    end
  end
  return table.concat(lines, "\n") .. "\n"
end

function pmacs.help.render_where_is(name, bindings)
  local lines = { "Where is: " .. name, "" }
  if type(bindings) ~= "table" or #bindings == 0 then
    lines[#lines + 1] = "  (not bound to any key)"
    lines[#lines + 1] = ""
    lines[#lines + 1] = "  Run it with M-x " .. name
    return table.concat(lines, "\n") .. "\n"
  end
  for _, b in ipairs(bindings) do
    local seq = (type(b) == "table" and b.sequence) or tostring(b)
    local scope = (type(b) == "table" and b.scope) and ("   (" .. tostring(b.scope) .. ")") or ""
    lines[#lines + 1] = "  " .. tostring(seq) .. scope
  end
  return table.concat(lines, "\n") .. "\n"
end

function pmacs.help.render_command_list(names)
  local lines = { string.format("Commands (%d)", #names), "" }
  for _, n in ipairs(names) do
    lines[#lines + 1] = string.format("  %-34s %s", n, description_of(n))
  end
  return table.concat(lines, "\n") .. "\n"
end

function pmacs.help.render_keybinding_list(rows)
  -- Grouped by scope so buffer-local bindings are not mixed in with the
  -- global map; sorted within a group by sequence.
  local by_scope = {}
  local scopes = {}
  for _, r in ipairs(rows) do
    local scope = tostring(r.scope)
    if not by_scope[scope] then
      by_scope[scope] = {}
      scopes[#scopes + 1] = scope
    end
    table.insert(by_scope[scope], r)
  end
  table.sort(scopes)
  local lines = { string.format("Key bindings (%d)", #rows), "" }
  for _, scope in ipairs(scopes) do
    local group = by_scope[scope]
    table.sort(group, function(a, b) return tostring(a.sequence) < tostring(b.sequence) end)
    lines[#lines + 1] = scope .. ":"
    for _, r in ipairs(group) do
      lines[#lines + 1] = string.format("  %-18s %s", tostring(r.sequence), tostring(r.command))
    end
    lines[#lines + 1] = ""
  end
  return table.concat(lines, "\n")
end

function pmacs.help.render_settings_list(rows)
  local lines = { string.format("Settings (%d)", #rows), "" }
  for _, d in ipairs(rows) do
    lines[#lines + 1] = string.format("  %-34s %s", tostring(d.name),
      tostring(d.description or "(no description)"))
  end
  return table.concat(lines, "\n") .. "\n"
end

function pmacs.help.render_apropos(needle, hits)
  local lines = { string.format("Apropos %q (%d)", needle, #hits), "" }
  if #hits == 0 then
    lines[#lines + 1] = "  (nothing matched)"
    return table.concat(lines, "\n") .. "\n"
  end
  for _, h in ipairs(hits) do
    lines[#lines + 1] = string.format("  %-34s %s", h.name, h.description)
  end
  return table.concat(lines, "\n") .. "\n"
end

--- Commands whose name or description CONTAINS `needle`, case-insensitively.
---
--- **Substring, deliberately, not fuzzy** (framing Q#D3). `fuzzy_score`
--- is subsequence-based and descriptions are long sentences, so a short
--- query's letters almost always appear in order — fuzzy here would match
--- nearly every command and destroy the precision that makes apropos
--- worth having.
function pmacs.help.apropos_hits(needle)
  local lowered = tostring(needle):lower()
  local hits = {}
  if lowered == "" then return hits end
  for _, name in ipairs(sorted_command_names()) do
    local desc = description_of(name)
    if name:lower():find(lowered, 1, true) or desc:lower():find(lowered, 1, true) then
      hits[#hits + 1] = { name = name, description = desc }
    end
  end
  return hits
end

-- ---------------------------------------------------------------------
-- The index
-- ---------------------------------------------------------------------

--- Every command in the family, in the order the index lists them.
--- Public so the acceptance suite can assert the index is complete as a
--- PROPERTY — adding a twelfth canonical command without indexing it
--- must fail, not silently pass.
pmacs.help.family = {
  "help.describe-command",
  "help.describe-setting",
  "help.describe-key",
  "help.describe-mode",
  "help.describe-buffer",
  "help.describe-hook",
  "help.where-is",
  "help.list-commands",
  "help.list-keybindings",
  "help.list-settings",
  "help.apropos",
}

local function index_text()
  local lines = { "pmacs help", "" }
  if type(pmacs.welcome) == "table" and type(pmacs.welcome.entries) == "table" then
    lines[#lines + 1] = "Keys"
    lines[#lines + 1] = ""
    lines[#lines + 1] = string.format("  %-18s %s", "M-x", "run a command by name")
    for _, e in ipairs(pmacs.welcome.entries) do
      lines[#lines + 1] = string.format("  %-18s %s", e.keys, e.label)
    end
    lines[#lines + 1] = ""
  end
  lines[#lines + 1] = "Discovery commands"
  lines[#lines + 1] = ""
  for _, name in ipairs(pmacs.help.family) do
    lines[#lines + 1] = string.format("  %-26s %s", name, description_of(name))
  end
  lines[#lines + 1] = ""
  lines[#lines + 1] = "The full keymap reference is docs/keybindings.md."
  return table.concat(lines, "\n") .. "\n"
end

pmacs.command.define {
  name = "help",
  description = "Index of the pmacs help and discovery commands.",
  fn = function() show(index_text()) end,
}

-- ---------------------------------------------------------------------
-- The family
-- ---------------------------------------------------------------------

pmacs.command.define {
  name = "help.describe-key",
  description = "Describe what a key sequence is bound to in this buffer.",
  fn = function()
    pmacs.minibuffer.read {
      prompt = "Describe key: ",
      history = "command",
      on_accept = function(seq)
        if seq == nil or seq == "" then return end
        local ok, info = pcall(pmacs.describe.key, seq)
        show(pmacs.help.render_key(seq, ok and info or nil))
      end,
    }
  end,
}

pmacs.command.define {
  name = "help.describe-mode",
  description = "Describe the active buffer's major mode.",
  fn = function()
    local buf = pmacs.window.buffer()
    local ok, info = pcall(pmacs.describe.mode, buf)
    show(pmacs.help.render_mode(ok and info or nil))
  end,
}

pmacs.command.define {
  name = "help.describe-buffer",
  description = "Describe the active buffer.",
  fn = function()
    local buf = pmacs.window.buffer()
    local ok, info = pcall(pmacs.describe.buffer, buf)
    show(pmacs.help.render_buffer(ok and info or nil))
  end,
}

pmacs.command.define {
  name = "help.describe-hook",
  description = "Describe a hook and list its listeners.",
  fn = function()
    pmacs.minibuffer.read {
      prompt = "Describe hook: ",
      history = "command",
      on_accept = function(name)
        if name == nil or name == "" then return end
        local ok, info = pcall(pmacs.describe.hook, name)
        show(pmacs.help.render_hook(name, ok and info or nil))
      end,
    }
  end,
}

pmacs.command.define {
  name = "help.where-is",
  description = "Show which keys run a command.",
  fn = function()
    pmacs.minibuffer.read {
      prompt = "Where is command: ",
      source = "commands",
      history = "command",
      on_accept = function(name)
        if name == nil or name == "" then return end
        local ok, info = pcall(pmacs.describe.command, name)
        if not ok or type(info) ~= "table" then
          pmacs.editor.set_status("where-is: no such command: " .. name)
          return
        end
        show(pmacs.help.render_where_is(name, info.key_bindings))
      end,
    }
  end,
}

pmacs.command.define {
  name = "help.list-commands",
  description = "List every registered command with its description.",
  fn = function() show(pmacs.help.render_command_list(sorted_command_names())) end,
}

pmacs.command.define {
  name = "help.list-keybindings",
  description = "List every key binding, grouped by scope.",
  fn = function() show(pmacs.help.render_keybinding_list(pmacs.keymap.list())) end,
}

pmacs.command.define {
  name = "help.list-settings",
  description = "List every registered setting with its description.",
  fn = function() show(pmacs.help.render_settings_list(pmacs.config.list())) end,
}

pmacs.command.define {
  name = "help.apropos",
  description = "Search command names and descriptions by substring.",
  fn = function()
    pmacs.minibuffer.read {
      prompt = "Apropos (substring): ",
      history = "command",
      on_accept = function(needle)
        if needle == nil or needle == "" then return end
        show(pmacs.help.render_apropos(needle, pmacs.help.apropos_hits(needle)))
      end,
    }
  end,
}

-- ---------------------------------------------------------------------
-- Forwarders (framing Q#D2)
-- ---------------------------------------------------------------------
--
-- `help.*` is canonical, so typing `help` at M-x surfaces the whole
-- family. These two keep the documented names working for users whose
-- muscle memory and whose `docs/keybindings.md` predate the rename.
--
-- Two names for one thing is the duplication §5 complains about; it is
-- the bounded price of not breaking documented commands, and it carries
-- a deprecation path a later stage can take.

-- `invoke`, NOT `invoke_interactive`. The forwarder must work however it
-- was itself reached, and `pmacs.command.invoke('editor.describe-setting')`
-- is a real caller (`tests/config_registry_acceptance.rs`) — CI caught
-- that, because the acceptance pin here only drove the M-x path.
--
-- Plain `invoke` is also the correct semantics, not merely the working
-- one: the interactive-command boundary is rotated once, by whatever
-- entry point the user actually used, for the name the user actually
-- typed. Rotating again on the inner call would record a second
-- boundary for a command the user never invoked.
local function forward(old_name, new_name)
  pmacs.command.define {
    name = old_name,
    description = string.format("Deprecated alias for `%s`.", new_name),
    fn = function() pmacs.command.invoke(new_name) end,
  }
end

forward("editor.describe-command", "help.describe-command")
forward("editor.describe-setting", "help.describe-setting")
