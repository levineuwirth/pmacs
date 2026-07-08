-- recentf.lua --- a most-recently-visited file list (Arc 3 Q#PS4).
--
-- Records every file buffer opened (buffer.after-load) or re-visited
-- (buffer.after-switch) into a deduped, capped, MRU-ordered `recentf`
-- state file (newline-delimited paths). `M-x recent-files` (bound
-- C-x C-r) opens the list in the minibuffer picker and visits the
-- choice.
--
-- On by default; disable from init.lua with
-- `pmacs.recentf.enable(false)`. Inert when no state dir is configured
-- (cfg(test) / no HOME).
--
-- Framing: docs/persistence-framing.md.

pmacs.recentf = pmacs.recentf or {}

local STATE_KEY = "recentf"
local MAX_ENTRIES = 50

local enabled = true
function pmacs.recentf.enable(on)
  enabled = (on ~= false)
end

local function load_list()
  local out = {}
  local text = pmacs.state.read(STATE_KEY)
  if not text then return out end
  for line in text:gmatch("([^\n]+)") do
    out[#out + 1] = line
  end
  return out
end

-- Move `path` to the front (MRU), dedup, cap.
local function record(path)
  if not (enabled and pmacs.state.available()) or not path then return end
  local list = load_list()
  local kept = { path }
  for _, p in ipairs(list) do
    if p ~= path and #kept < MAX_ENTRIES then
      kept[#kept + 1] = p
    end
  end
  pmacs.state.write(STATE_KEY, table.concat(kept, "\n") .. (#kept > 0 and "\n" or ""))
end

local function record_active()
  pcall(record, pmacs.editor.file_path())
end

-- First open and every re-visit of an already-open file refresh MRU.
pmacs.hook.add("buffer.after-load", record_active)
pmacs.hook.add("buffer.after-switch", record_active)

-- The public list (MRU-first), for the picker or a user script.
function pmacs.recentf.list()
  return load_list()
end

pmacs.command.define {
  name = "recent-files",
  description = "Visit a recently opened file (Arc 3).",
  fn = function()
    local list = load_list()
    if #list == 0 then
      pmacs.editor.set_status("recentf: no recent files")
      return
    end
    pmacs.minibuffer.read {
      prompt = "Recent file: ",
      source = function() return list end,
      history = "recent-files",
      on_accept = function(path)
        if path == nil or path == "" then return end
        local ok, err = pcall(pmacs.buffer.find_or_open, path)
        if not ok then
          pmacs.editor.set_status("recentf: " .. tostring(err))
        end
      end,
    }
  end,
}

pmacs.keymap.bind { scope = "global", sequence = "C-x C-r", command = "recent-files" }

