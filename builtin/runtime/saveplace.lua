-- saveplace.lua --- remember the cursor position per file (Arc 3 Q#PS3b).
--
-- Records the active file buffer's (cursor byte, view_top) on save and
-- at quit, and restores it when the file is reopened. Storage is the
-- `places` state file, one `<cursor> <view_top> <path>` line per file
-- (numbers first so the path, which may contain spaces, is the
-- whitespace-split remainder). LRU-capped.
--
-- On by default; disable from init.lua with
-- `pmacs.saveplace.enable(false)`. Inert when no state dir is
-- configured (cfg(test) / no HOME), so the lib suite writes nothing.
--
-- Framing: docs/persistence-framing.md.

pmacs.saveplace = pmacs.saveplace or {}

local STATE_KEY = "places"
local MAX_ENTRIES = 200

local enabled = true
function pmacs.saveplace.enable(on)
  enabled = (on ~= false)
end

local function active_ready()
  return enabled and pmacs.state.available() and pmacs.editor.file_path() ~= nil
end

-- Load the places file into an ordered list of {path, cursor, view_top}
-- (most-recently-recorded first) plus a path->index lookup.
local function load_places()
  local list, index = {}, {}
  local text = pmacs.state.read(STATE_KEY)
  if not text then return list, index end
  for line in text:gmatch("([^\n]+)") do
    -- "<cursor> <view_top> <path>"
    local cur, vt, path = line:match("^(%d+)%s+(%d+)%s+(.+)$")
    if path and not index[path] then
      list[#list + 1] = { path = path, cursor = tonumber(cur), view_top = tonumber(vt) }
      index[path] = #list
    end
  end
  return list, index
end

local function save_places(list)
  local lines = {}
  for i = 1, math.min(#list, MAX_ENTRIES) do
    local e = list[i]
    lines[#lines + 1] = string.format("%d %d %s", e.cursor, e.view_top, e.path)
  end
  pmacs.state.write(STATE_KEY, table.concat(lines, "\n") .. (#lines > 0 and "\n" or ""))
end

-- Record the active buffer's place, moving it to the front (LRU).
local function record_active()
  if not active_ready() then return end
  local path = pmacs.editor.file_path()
  local cursor = pmacs.editor.cursor()
  local view_top = pmacs.editor.view_top and pmacs.editor.view_top() or 0
  local list, index = load_places()
  if index[path] then table.remove(list, index[path]) end
  table.insert(list, 1, { path = path, cursor = cursor, view_top = view_top })
  save_places(list)
end

-- Restore the just-loaded file's place, if we have one.
local function restore_active()
  if not active_ready() then return end
  local path = pmacs.editor.file_path()
  local list, index = load_places()
  local i = index[path]
  if not i then return end
  local e = list[i]
  pmacs.editor.goto_byte(e.cursor)
  if pmacs.editor.set_view_top then pmacs.editor.set_view_top(e.view_top) end
end

-- Record on save and on quit; restore on open. before-save /
-- before-quit are short-circuit hooks — returning nil never vetoes.
pmacs.hook.add("buffer.before-save", function()
  pcall(record_active)
end)

pmacs.hook.add("editor.before-quit", function()
  pcall(record_active)
end)

pmacs.hook.add("buffer.after-load", function()
  pcall(restore_active)
end)
