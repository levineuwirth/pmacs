-- The theme setting and its restore (E10g.3).
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
  description = "The named theme: one of pmacs.theme.names(). Set in init.lua it beats a choice kept in pmacs.state; a theme defined in init.lua must be defined before this is set to it.",
  type = "enum",
  choices = pmacs.theme.names(),
  default = "default-dark",
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
