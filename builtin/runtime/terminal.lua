-- terminal.lua --- Friendly Vterm Stage 2 command and modeline surface.

local terminal = assert(pmacs.terminal, "pmacs.terminal raw bindings are required")
local raw_open = assert(terminal._open, "pmacs.terminal._open is required")

-- Q#TC2a. Every default reproduces today's behavior exactly, so a tree
-- with no settings written and no profiles registered behaves as before.
pmacs.config.define {
  name = "terminal.default-profile",
  type = "string",
  default = "",
  allow_empty = true,
  mutability = "live",
  description = "Profile name from pmacs.terminal.profiles to open by default. " ..
    "Empty means no profile: fall back to $SHELL.",
}

pmacs.config.define {
  name = "terminal.scrollback-rows",
  type = "integer",
  default = 10000,
  min = 0,
  max = 4000000,
  mutability = "live",
  description = "Rows of scrollback retained per terminal. " ..
    "0 retains no history.",
}

pmacs.config.define {
  name = "terminal.escape-key",
  type = "string",
  default = "C-c",
  mutability = "live",
  description = "Chord that escapes to the editor from a terminal. " ..
    "Pressing it twice sends the chord itself to the child.",
}

local function bind_terminal_keys(buffer)
  local function bind(sequence, command)
    pmacs.keymap.bind {
      scope = "buffer",
      buffer = buffer,
      sequence = sequence,
      command = command,
    }
  end
  bind("M-w", "terminal.copy-selection")
  bind("M-v", "terminal.page-up")
  bind("C-v", "terminal.page-down")
  bind("M-<", "terminal.scroll-oldest")
  bind("M->", "terminal.scroll-bottom")
end

-- Q#TC1: profiles are a raw Lua table, not a config setting. The
-- registry stores four scalars and has no table kind, so a profile —
-- inherently `{ command, args, cwd, env }` — lives here beside
-- `pmacs.lsp.config` and `pmacs.pair.sets` until table-valued settings
-- exist.
terminal.profiles = terminal.profiles or {}

local PROFILE_FIELDS = {
  command = "string",
  args = "table",
  cwd = "string",
  env = "table",
}

local function validate_profile(name, profile)
  if type(profile) ~= "table" then
    error(string.format("terminal profile %q must be a table", name), 0)
  end
  for key, value in pairs(profile) do
    local expected = PROFILE_FIELDS[key]
    if not expected then
      error(string.format("terminal profile %q: unknown field %q", name, tostring(key)), 0)
    end
    if type(value) ~= expected then
      error(string.format(
        "terminal profile %q: field %q must be a %s, got %s",
        name, key, expected, type(value)), 0)
    end
  end
  return profile
end

local function known_profile_names()
  local names = {}
  for name in pairs(terminal.profiles) do names[#names + 1] = name end
  table.sort(names)
  return names
end

-- Q#TC2 / Q#TC3a: resolve a profile by name, or nil when none is
-- selected. An explicitly requested profile that does not exist is an
-- error even when `terminal.default-profile` is valid — a typo must not
-- silently fall back to the default.
local function resolve_profile(requested)
  local name = requested
  if name == nil then
    local configured = pmacs.config.get("terminal.default-profile")
    if configured == nil or configured == "" then return nil end
    name = configured
  end
  local profile = terminal.profiles[name]
  if profile == nil then
    local known = known_profile_names()
    local listed = #known > 0 and table.concat(known, ", ") or "(none defined)"
    error(string.format(
      "terminal profile %q is not defined; known profiles: %s", name, listed), 0)
  end
  return validate_profile(name, profile)
end

-- Q#TC3a merge order, per field: explicit open field, then the profile's
-- field, then the scalar setting, then the built-in fallback. `env` is
-- the one field where "first wins" would be wrong, so it MERGES with
-- explicit entries overriding the profile's — any other reading silently
-- drops half a user's environment.
local function merge_env(profile_env, explicit_env)
  if profile_env == nil then return explicit_env end
  local merged = {}
  for key, value in pairs(profile_env) do merged[key] = value end
  for key, value in pairs(explicit_env or {}) do merged[key] = value end
  return merged
end

function terminal.open(spec)
  spec = spec or {}
  local resolved = {}
  for key, value in pairs(spec) do
    if key ~= "profile" then resolved[key] = value end
  end

  local profile = resolve_profile(spec.profile)
  if profile then
    for key in pairs(PROFILE_FIELDS) do
      if key ~= "env" and resolved[key] == nil then resolved[key] = profile[key] end
    end
    resolved.env = merge_env(profile.env, spec.env)
  end

  -- The two open-time settings resolve through the GLOBAL chain
  -- (Q#TC2b): they are read before the identity buffer exists, so there
  -- is no terminal to resolve a buffer-local against.
  if resolved.scrollback_rows == nil then
    resolved.scrollback_rows = pmacs.config.get("terminal.scrollback-rows")
  end
  if resolved.command == nil then
    resolved.command = os.getenv("SHELL") or "/bin/sh"
  end

  local buffer = raw_open(resolved)
  bind_terminal_keys(buffer)
  return buffer
end

pmacs.command.define {
  name = "terminal",
  description = "Open a terminal running the configured profile, or $SHELL.",
  fn = function(profile)
    return terminal.open { profile = profile }
  end,
}

-- Q#TC10: the opening binding. `COHERENCE.md` Priority 1 names a
-- terminal keybinding as part of protecting the golden journey, and §2
-- step 8 grades the terminal "works but undiscoverable". `C-c` is
-- already a live global prefix (fold's `C-c @ ...`), so this is a new
-- leaf under it rather than a shadow.
--
-- Named limitation: unreachable from INSIDE a terminal window, where
-- `C-c` is consumed as the escape. `M-x terminal` still works there.
pmacs.keymap.bind { scope = "global", sequence = "C-c t", command = "terminal" }

pmacs.command.define {
  name = "terminal.copy-selection",
  description = "Copy the active terminal selection.",
  fn = function() return terminal.copy_selection() end,
}

pmacs.command.define {
  name = "terminal.page-up",
  description = "Scroll the active terminal viewport up one page.",
  fn = function() return terminal._scroll_page(1) end,
}

pmacs.command.define {
  name = "terminal.page-down",
  description = "Scroll the active terminal viewport down one page.",
  fn = function() return terminal._scroll_page(-1) end,
}

pmacs.command.define {
  name = "terminal.scroll-oldest",
  description = "Scroll the active terminal viewport to the oldest retained row.",
  fn = function() return terminal.scroll(math.maxinteger) end,
}

pmacs.command.define {
  name = "terminal.scroll-bottom",
  description = "Return the active terminal viewport to the live tail.",
  fn = function() return terminal.scroll_to_bottom() end,
}

pmacs.statusline.register {
  name = "terminal",
  side = "right",
  priority = 10,
  face = "ui.modeline.terminal",
  fn = function(ctx)
    if not terminal.is_terminal(ctx.buffer) then return nil end
    local state = terminal.state(ctx.buffer)
    local view = terminal.view_state(ctx)
    if not view then return nil end

    local process = state.process
    local text
    if process.kind == "running" then
      text = "TERM"
    elseif process.kind == "exited" then
      text = "TERM:" .. tostring(process.code)
    elseif process.kind == "signaled" then
      text = "TERM:" .. process.signal
    else
      text = "TERM:ERR"
    end
    if view.scroll_offset > 0 then
      text = text .. " ↑" .. tostring(view.scroll_offset)
    end
    return text
  end,
}
