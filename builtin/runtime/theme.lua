-- The theme setting, its restore, and the picker (E10g.3, E10g.4).
--
-- A theme is chosen three ways, and the order between them is the
-- owner's ruling of 2026-10-27: an explicit `init.lua` setting of
-- `ui.theme` beats a choice the picker kept in `pmacs.state`, which beats
-- the default. `init.lua`'s own `pmacs.theme.set` and `merge` calls are
-- a fourth thing, not a choice: they are the user layer, which every
-- load reapplies over the theme it loads (`ThemeLibrary`,
-- `src/highlight.rs`), so neither a restored choice nor the picker
-- discards them.
--
-- The theme is the editor's, and in a daemon the daemon's: loading one
-- re-themes every attached frontend at once.

local STATE_KEY = "theme"

pmacs.config.define {
  name = "ui.theme",
  -- The choices grow as themes are defined (`pmacs.theme.define` adds
  -- each name), so an `init.lua` that sets this to a theme of its own
  -- defines the theme first.
  description = "The named theme: one of pmacs.theme.names(). Set in init.lua it beats the one the theme picker (M-x theme-select) kept; a theme defined in init.lua must be defined before this is set to it.",
  type = "enum",
  choices = pmacs.theme.names(),
  default = "default-dark",
  mutability = "live",
}

-- E10g.5: how many colors a terminal UI paints with. `auto` is what the
-- terminal is detected to show, from TERM alone (`ColorDepth::detect`).
-- Lives here, in the editor (the daemon's, for an attached terminal),
-- and the frontend's detection caps it.
pmacs.config.define {
  name = "ui.color-depth",
  description = "How many colors a terminal UI paints with: auto (16 where TERM is linux, a vt model, or a bare xterm or screen; truecolor otherwise), truecolor, 256 or 16, never more than the terminal is detected to show. A theme's colors take their authored fallbacks, any other color the nearest. A terminal attached to a daemon is painted at 256 for anything below truecolor and reduces to 16 itself where it detects 16. The GPU frontend is not affected.",
  type = "enum",
  choices = { "auto", "truecolor", "256", "16" },
  default = "auto",
  mutability = "live",
}

-- Setting it loads it, from init.lua, the picker or anywhere else.
pmacs.config.on_change("ui.theme", function(name)
  pmacs.theme.load(name)
end)

local function defined(name)
  for _, n in ipairs(pmacs.theme.names()) do
    if n == name then return true end
  end
  return false
end

-- The choice kept by the picker, or nil. Whole-file parse, as zoom's:
-- the file is the value, one name and a newline.
local function kept_choice()
  if not pmacs.state.available() then return nil end
  local text = pmacs.state.read(STATE_KEY)
  if not text then return nil end
  return text:match("^([^\n]+)\n$")
end

-- Resolve the theme at startup. Called from the Rust side once state is
-- readable (`install_state_dirs`), after init.lua, as `pmacs.zoom.restore`
-- is: a read at module load would find nothing, always.
function pmacs.theme.restore()
  local name
  if pmacs.config.is_set("ui.theme") then
    -- init.lua chose; its `on_change` already loaded it.
    name = pmacs.config.get("ui.theme")
  else
    name = kept_choice()
    if name and not defined(name) then
      pmacs.editor.set_status(string.format(
        "theme: the kept theme %q is not defined; using %s",
        name, pmacs.config.get("ui.theme")))
      name = nil
    end
    if name then
      pmacs.config.set("ui.theme", name)
    else
      name = pmacs.config.get("ui.theme")
    end
  end
  pmacs.theme.load(name)
  return name
end

-- The picker (E10g.4). The list is every defined theme, the one in
-- force first; moving the selection loads the selected theme at once
-- (the preview, `on_select`), RET keeps it and writes it to state, and
-- C-g loads again the theme in force when the picker opened, the user
-- layer with it.
pmacs.command.define {
  name = "theme-select",
  description = "Choose a theme: moving the selection previews it, RET keeps it, C-g restores the one before",
  fn = function()
    local original = pmacs.theme.loaded() or pmacs.config.get("ui.theme")
    local names = { original }
    for _, n in ipairs(pmacs.theme.names()) do
      if n ~= original then names[#names + 1] = n end
    end
    pmacs.minibuffer.read {
      prompt = "Theme: ",
      -- Ranked here, by a plain substring, so the order stays this one
      -- and the theme in force is the first, selected at open.
      ranked = true,
      source = function(needle)
        local shown = {}
        for _, n in ipairs(names) do
          if n:find(needle, 1, true) then shown[#shown + 1] = n end
        end
        return shown
      end,
      on_select = function(name)
        if defined(name) then pmacs.theme.load(name) end
      end,
      on_accept = function(name)
        if not defined(name) then
          pmacs.theme.load(original)
          pmacs.editor.set_status(string.format("theme: no theme named %q", name))
          return
        end
        pmacs.config.set("ui.theme", name)
        -- `set` loads only on a change; the preview may already show it.
        pmacs.theme.load(name)
        if pmacs.state.available() then
          pmacs.state.write(STATE_KEY, name .. "\n")
        end
        pmacs.editor.set_status("theme: " .. name)
      end,
      on_cancel = function()
        pmacs.theme.load(original)
      end,
      -- Another prompt opening over the picker (a code action's chooser
      -- arriving from a server, say) ends it with neither RET nor C-g,
      -- so its preview would outlive it: undo it then too.
      on_replace = function()
        pmacs.theme.load(original)
      end,
    }
  end,
}
