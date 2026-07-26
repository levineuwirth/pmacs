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
  -- Q#TC8a/Q#TC9: copy mode is ADDITIVE. The live keys above are
  -- unchanged; this is one more leaf beside them. `C-t` is globally
  -- `edit.transpose-chars`, which is meaningless in a read-only
  -- terminal buffer, and binding it buffer-locally is the scoped
  -- idiom rather than a shadow — `keymap.bind`'s strictness rejects
  -- binding a PREFIX of an existing sequence within a scope, not
  -- cross-scope shadowing.
  --
  -- Physically typed as `C-c C-t`: in a terminal every unescaped key
  -- goes to the child, so terminal-local bindings are reached through
  -- the escape. That also matches emacs-libvterm's own chord.
  bind("C-t", "terminal.copy-mode")
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

-- Every diagnostic below renders a caller- or user-supplied value, so
-- rendering must never be the thing that fails. `%q` is partial — it
-- raises on a table or function — and a profile name arrives straight
-- from `open { profile = ... }`.
local function describe_name(name)
  if type(name) == "string" then return string.format("%q", name) end
  return string.format("<%s %s>", type(name), tostring(name))
end

local function validate_profile(name, profile)
  local shown = describe_name(name)
  if type(profile) ~= "table" then
    error(string.format("terminal profile %s must be a table", shown), 0)
  end
  for key, value in pairs(profile) do
    local expected = PROFILE_FIELDS[key]
    if not expected then
      error(string.format("terminal profile %s: unknown field %q", shown, tostring(key)), 0)
    end
    if type(value) ~= expected then
      error(string.format(
        "terminal profile %s: field %q must be a %s, got %s",
        shown, key, expected, type(value)), 0)
    end
  end
  return profile
end

-- `terminal.profiles` is a raw user table, so its keys are whatever the
-- user wrote. Sorting them directly raises "attempt to compare number
-- with string" the moment the table holds both a string and a numeric
-- key — and it raises on the UNKNOWN-PROFILE path, replacing the very
-- error this list exists to explain with an opaque one. Sorting DISPLAY
-- strings is total over every key type, so the diagnostic survives a
-- malformed table.
local function known_profile_names()
  local names = {}
  for name in pairs(terminal.profiles) do names[#names + 1] = tostring(name) end
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
      "terminal profile %s is not defined; known profiles: %s",
      describe_name(name), listed), 0)
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

-- === Copy mode (Stage 2, Q#TC6) =========================================
--
-- `terminal.copy-mode` MATERIALIZES the retained rows into an ordinary
-- read-only document buffer instead of adding a modal state to the
-- terminal. That choice is the whole design:
--
--  * isearch, motion, selection, `M-w` and the kill ring all work with no
--    new substrate — the snapshot is a rope, so `SearchStore` and the
--    existing match painting apply unchanged;
--  * "keys must not reach the child" dissolves structurally rather than
--    being guarded: the transport arm keys on `is_terminal(buffer)`, and
--    a snapshot buffer is not a terminal, so it never fires;
--  * the dispatch-shadow count stays at SIX (`COHERENCE.md` §6) and
--    `describe-key` keeps telling the truth, because the bindings are
--    buffer-local and inspectable.

local raw_copy_retained = assert(terminal._copy_retained,
  "pmacs.terminal._copy_retained is required")

-- An ARRAY of `{ terminal = <buf>, buffer = <buf> }`, scanned linearly and
-- compared with `==`, following dired's handle table (F7).
--
-- Not `snapshots[name]`, and not `snapshots[buf]`, for two separate
-- reasons — both of which were live defects in review round 1:
--
--  * **A terminal name is not a unique key.** `TerminalManager::open`
--    uniquifies only the DERIVED name; an explicitly passed
--    `name = "*same*"` is inserted verbatim
--    (`src/terminal/session.rs`, `if spec.name.is_some()`). Two valid
--    terminals can therefore share a name, and a name-keyed table gives
--    them one snapshot between them: the second invocation silently
--    retargets it, `q` returns to the wrong terminal, and killing either
--    one removes the shared buffer.
--  * **A buffer handle is not a stable table key.** `BufferIdLua`
--    implements `__eq` but each wrapper is a distinct table key, so
--    `snapshots[buf]` would miss on a freshly minted handle for the same
--    buffer. Comparison works; hashing does not. Hence the scan.
local handles = {}

-- Compact dead entries first, so a command in a killed snapshot sees
-- "not in copy mode" rather than operating on dead state.
local function live_handles()
  local live = {}
  for _, h in ipairs(handles) do
    local term_ok, term_valid = pcall(h.terminal.is_valid, h.terminal)
    local snap_ok, snap_valid = pcall(h.buffer.is_valid, h.buffer)
    if term_ok and term_valid and snap_ok and snap_valid then
      live[#live + 1] = h
    end
  end
  handles = live
  return live
end

local function handle_for_terminal(term_buf)
  if term_buf == nil then return nil end
  for _, h in ipairs(live_handles()) do
    if h.terminal == term_buf then return h end
  end
  return nil
end

local function handle_for_snapshot(buf)
  if buf == nil then return nil end
  for _, h in ipairs(live_handles()) do
    if h.buffer == buf then return h end
  end
  return nil
end

local function buffer_name(buf)
  local ok, described = pcall(pmacs.describe.buffer, buf)
  if ok and described then return described.name end
  return nil
end

local function buffer_named(name)
  for _, id in ipairs(pmacs.buffer.list()) do
    local ok, described = pcall(pmacs.describe.buffer, id)
    if ok and described and described.name == name then return id end
  end
  return nil
end

-- `*terminal:bash*` -> `*terminal-copy: terminal:bash*`. The surrounding
-- asterisks are stripped before nesting so the result reads as one
-- generated-buffer name rather than two.
local function snapshot_base_name(term_buf)
  local name = buffer_name(term_buf) or "terminal"
  return string.format("*terminal-copy: %s*", (name:gsub("^%*", ""):gsub("%*$", "")))
end

-- How far the `<2>`, `<3>`, ... disambiguation walks before giving up.
local NAME_VARIANT_LIMIT = 99

-- `pmacs.buffer.create` takes any caller-chosen name, so a foreign buffer
-- may already be called `*terminal-copy: sh*` — and two same-named
-- terminals legitimately produce the same base name. Painting into a
-- buffer we did not create would clobber a user's data through
-- `bypass_intercept`, so **found-by-name is NOT adoption**: ownership
-- means "this buffer is in the handle table above", exactly as in dired.
local function unique_snapshot_name(term_buf)
  local name = snapshot_base_name(term_buf)
  if buffer_named(name) == nil then return name end
  for i = 2, NAME_VARIANT_LIMIT do
    local candidate = string.format("%s<%d>", name, i)
    if buffer_named(candidate) == nil then return candidate end
  end
  error(string.format(
    "terminal.copy-mode: %s is taken and no free variant remains", name), 0)
end

-- Q#TC7: the snapshot text comes from the SAME serializer selection-copy
-- uses, so soft wraps, wide glyphs, clusters and trailing blanks cannot
-- drift between the two.
local function render_snapshot(record)
  local text = raw_copy_retained(record.terminal) or ""
  local buf = record.buffer
  local len = buf:len()
  -- Snapshot writes bypass the read-only intercept; everything else is
  -- rejected by it.
  if len > 0 then buf:delete(0, len, { bypass_intercept = true }) end
  if #text > 0 then buf:insert(0, text, { bypass_intercept = true }) end
end

local function claim_snapshot(term_buf)
  -- Q#TC8: re-invoking against the same terminal refreshes IN PLACE.
  -- Identity is the terminal BUFFER, so two same-named terminals get two
  -- snapshots and neither can retarget the other's.
  local existing = handle_for_terminal(term_buf)
  if existing then return existing end

  local name = unique_snapshot_name(term_buf)
  local buf = pmacs.buffer.create(name)
  local record = { terminal = term_buf, buffer = buf }
  handles[#handles + 1] = record

  -- Q#TC6a — BOTH calls, and the second is the load-bearing one.
  --
  -- An intercept guards the dispatch/edit path only. It does NOT set
  -- `Buffer::read_only` (deliberately independent), and no Lua binding
  -- sets that flag at all, so an optimistic CRDT op from a semantic
  -- frontend bypasses the intercept AND passes `ensure_writable()` —
  -- mutating the daemon buffer in lockstep with the mirror, with no
  -- divergence to notice. `set_round_trip_input` prevents that at the
  -- only point it can be prevented: `dispatch_idle_for` reports false
  -- while this buffer is focused, so the frontend never applies
  -- optimistically and never emits the op. It is the guard, not
  -- hardening.
  pmacs.buffer.add_intercept(buf, function()
    error(name .. " is read-only")
  end)
  pmacs.buffer.set_round_trip_input(buf, true)

  pmacs.keymap.bind { scope = "buffer", buffer = buf,
    sequence = "g", command = "terminal.copy-refresh" }
  pmacs.keymap.bind { scope = "buffer", buffer = buf,
    sequence = "q", command = "terminal.copy-quit" }

  -- Q#TC8 lifecycle, both directions. Killing the terminal takes ITS
  -- snapshot with it — `record`, captured here, not "whatever is
  -- currently filed under this name"; killing the snapshot alone leaves
  -- the terminal running, and `live_handles` compacts the entry out so a
  -- later invoke rebuilds.
  --
  -- `on_removed` is sound here because every user-facing kill path
  -- routes through `pmacs.buffer.kill`, which fires the callbacks. The
  -- terminal manager's own `prune` does not — but it never removes a
  -- buffer either; it REACTS to one already gone from the registry. A
  -- child exiting therefore leaves both the terminal and its snapshot
  -- alive, which is what makes reading back a finished command's output
  -- work at all.
  pcall(pmacs.buffer.on_removed, term_buf, function()
    local ok, valid = pcall(record.buffer.is_valid, record.buffer)
    if ok and valid then pcall(pmacs.buffer.kill, record.buffer) end
  end)

  return record
end

-- The snapshot record whose buffer the active window shows, or nil.
local function snapshot_for_current_buffer()
  return handle_for_snapshot(pmacs.window.buffer())
end

function terminal.copy_mode(term_buf)
  term_buf = term_buf or pmacs.window.buffer()
  assert(term_buf, "terminal.copy-mode: no active buffer")
  if not terminal.is_terminal(term_buf) then
    error("terminal.copy-mode: the current buffer is not a terminal", 0)
  end
  local record = claim_snapshot(term_buf)
  render_snapshot(record)
  pmacs.window.switch_buffer(record.buffer)
  return record.buffer
end

pmacs.command.define {
  name = "terminal.copy-mode",
  description = "Open a searchable read-only snapshot of this terminal's scrollback.",
  fn = function() return terminal.copy_mode() end,
}

pmacs.command.define {
  name = "terminal.copy-refresh",
  description = "Re-snapshot the source terminal into this copy buffer.",
  fn = function()
    local record = snapshot_for_current_buffer()
    if not record then return end
    if not record.terminal:is_valid() then
      pmacs.editor.set_status("terminal.copy-refresh: the source terminal is gone")
      return
    end
    render_snapshot(record)
  end,
}

pmacs.command.define {
  name = "terminal.copy-quit",
  description = "Return to the terminal this copy buffer was taken from.",
  fn = function()
    local record = snapshot_for_current_buffer()
    if not record then return end
    if record.terminal:is_valid() then
      pmacs.window.switch_buffer(record.terminal)
    end
  end,
}

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
