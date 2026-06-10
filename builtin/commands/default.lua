-- builtin/commands/default.lua --- M2.5 default commands.
--
-- Defines named, introspectable commands for every M1 editor action.
-- The bodies delegate to `pmacs.editor.*` (Rust primitives) so users
-- can compose them or rebind their keys without touching the binary.
-- The default keymap (builtin/keymaps/default.lua) maps chord
-- sequences onto these names.

local cmd = pmacs.command.define
local ed = pmacs.editor

-- Cursor motion --------------------------------------------------------------

cmd { name = "cursor.left",       description = "Move cursor one codepoint left.",
      fn = function() ed.move_left() end }
cmd { name = "cursor.right",      description = "Move cursor one codepoint right.",
      fn = function() ed.move_right() end }
cmd { name = "cursor.up",         description = "Move cursor up one line, preserving column.",
      fn = function() ed.move_up() end }
cmd { name = "cursor.down",       description = "Move cursor down one line, preserving column.",
      fn = function() ed.move_down() end }
cmd { name = "cursor.line-start", description = "Move cursor to start of line.",
      fn = function() ed.move_line_start() end }
cmd { name = "cursor.line-end",   description = "Move cursor to end of line.",
      fn = function() ed.move_line_end() end }
cmd { name = "cursor.word-left",  description = "Move cursor one word backward.",
      fn = function() ed.move_word_left() end }
cmd { name = "cursor.word-right", description = "Move cursor one word forward.",
      fn = function() ed.move_word_right() end }
cmd { name = "cursor.page-up",    description = "Move cursor up by one screenful.",
      fn = function() ed.move_page_up() end }
cmd { name = "cursor.page-down",  description = "Move cursor down by one screenful.",
      fn = function() ed.move_page_down() end }
cmd { name = "cursor.paragraph-up",
      description = "Move cursor backward to the previous paragraph break.",
      fn = function() ed.move_paragraph_up() end }
cmd { name = "cursor.paragraph-down",
      description = "Move cursor forward to the next paragraph break.",
      fn = function() ed.move_paragraph_down() end }

-- Buffer editing -------------------------------------------------------------

-- CUA region semantics: with an active selection, Backspace / Delete
-- consume the region (cursor lands at its start, selection clears).
-- `delete_region` returns false when no region is active, so the
-- single-codepoint behavior is untouched outside selections.
cmd { name = "buffer.delete-backward",
      description = "Delete the active region, or the codepoint before the cursor.",
      fn = function()
        if not ed.delete_region() then ed.backspace() end
      end }
cmd { name = "buffer.delete-forward",
      description = "Delete the active region, or the codepoint at the cursor.",
      fn = function()
        if not ed.delete_region() then ed.delete_forward() end
      end }
cmd { name = "buffer.delete-word-backward",
      description = "Delete from the cursor back to the start of the previous word.",
      fn = function() ed.delete_word_backward() end }
cmd { name = "buffer.delete-word-forward",
      description = "Delete from the cursor forward to the end of the next word.",
      fn = function() ed.delete_word_forward() end }

-- Selection-extending motion (CUA-style Shift+motion). Each select-*
-- command anchors at the current cursor (if no region is already
-- active) and then performs the underlying motion. Plain motion
-- commands are unchanged: they preserve existing selections.
local function ensure_anchor()
  if ed.region() == nil then
    ed.begin_selection(ed.cursor())
  end
end

cmd { name = "cursor.select-left",
      description = "Extend selection by one codepoint left.",
      fn = function() ensure_anchor(); ed.move_left() end }
cmd { name = "cursor.select-right",
      description = "Extend selection by one codepoint right.",
      fn = function() ensure_anchor(); ed.move_right() end }
cmd { name = "cursor.select-up",
      description = "Extend selection upward by one line.",
      fn = function() ensure_anchor(); ed.move_up() end }
cmd { name = "cursor.select-down",
      description = "Extend selection downward by one line.",
      fn = function() ensure_anchor(); ed.move_down() end }
cmd { name = "cursor.select-word-left",
      description = "Extend selection by one word left.",
      fn = function() ensure_anchor(); ed.move_word_left() end }
cmd { name = "cursor.select-word-right",
      description = "Extend selection by one word right.",
      fn = function() ensure_anchor(); ed.move_word_right() end }
cmd { name = "cursor.select-paragraph-up",
      description = "Extend selection to the previous paragraph break.",
      fn = function() ensure_anchor(); ed.move_paragraph_up() end }
cmd { name = "cursor.select-paragraph-down",
      description = "Extend selection to the next paragraph break.",
      fn = function() ensure_anchor(); ed.move_paragraph_down() end }
cmd { name = "cursor.select-line-start",
      description = "Extend selection to start of line.",
      fn = function() ensure_anchor(); ed.move_line_start() end }
cmd { name = "cursor.select-line-end",
      description = "Extend selection to end of line.",
      fn = function() ensure_anchor(); ed.move_line_end() end }
-- CUA type-over: inserting with an active selection replaces it
-- (`delete_region` is a no-op without one). The pmacs-gpu frontend
-- relies on this: its optimistic-insert path detects an own-window
-- selection and round-trips the key so these commands run.
cmd { name = "buffer.newline",
      description = "Insert a newline at the cursor, replacing the active region.",
      fn = function() ed.delete_region(); ed.insert_char(10) end }
cmd { name = "buffer.tab",
      description = "Insert a tab at the cursor, replacing the active region.",
      fn = function() ed.delete_region(); ed.insert_char(9) end }
cmd { name = "buffer.self-insert",
      description = "Insert the codepoint argument at the cursor, replacing the active region.",
      fn = function(codepoint) ed.delete_region(); ed.insert_char(codepoint) end }

-- History --------------------------------------------------------------------

cmd { name = "buffer.undo", description = "Undo the most recent edit.",
      fn = function() ed.undo() end }
cmd { name = "buffer.redo", description = "Redo the most recently undone edit.",
      fn = function() ed.redo() end }

-- Region operations (M2.12) --------------------------------------------------

cmd { name = "region.delete",
      description = "Delete the active region (set by mouse drag).",
      fn = function()
        if not ed.delete_region() then
          ed.set_status("no region")
        end
      end }
cmd { name = "region.cancel",
      description = "Drop any active selection without changing the cursor.",
      fn = function() ed.clear_selection() end }

-- File I/O -------------------------------------------------------------------

cmd { name = "buffer.save", description = "Save the current buffer to its backing file.",
      fn = function()
        if not pmacs.hook.run("buffer.before-save") then
          ed.set_status("save vetoed by buffer.before-save")
          return
        end
        if ed.save() then
          pmacs.hook.run("buffer.after-save")
        end
      end }

-- Editor session -------------------------------------------------------------

cmd { name = "editor.quit",   description = "Exit the editor.",
      fn = function()
        if not pmacs.hook.run("editor.before-quit") then
          ed.set_status("quit vetoed by editor.before-quit")
          return
        end
        ed.quit()
      end }
cmd { name = "editor.cancel", description = "Cancel a pending key prefix.",
      fn = function() ed.cancel() end }

-- Window splits (M2.8) -------------------------------------------------------

cmd { name = "window.split-horizontal",
      description = "Split the active window horizontally (children stack top-to-bottom).",
      fn = function() pmacs.window.split_horizontal() end }
cmd { name = "window.split-vertical",
      description = "Split the active window vertically (children sit side-by-side).",
      fn = function() pmacs.window.split_vertical() end }
cmd { name = "window.focus-next",
      description = "Move focus to the next window in iteration order.",
      fn = function() pmacs.window.focus_next() end }
cmd { name = "window.focus-prev",
      description = "Move focus to the previous window in iteration order.",
      fn = function() pmacs.window.focus_prev() end }
cmd { name = "window.close",
      description = "Close the active window (refused when only one remains).",
      fn = function()
        if not pmacs.window.close() then
          pmacs.editor.set_status("only one window left")
        end
      end }
cmd { name = "window.close-others",
      description = "Close every window except the active one.",
      fn = function() pmacs.window.close_others() end }

-- Buffer cycling (Doom-style C-x <right> / C-x <left>) -----------------------
--
-- Walks the active window through the registry's buffer list in a
-- ring; `next` wraps from the last buffer to the first, `previous`
-- wraps the other way. Compares BufferIdLua handles via tostring()
-- because two handles to the same buffer are different userdata
-- instances that don't hash equal.

local function cycle_buffer(step)
  local current = pmacs.window.buffer()
  local ids = pmacs.buffer.list()
  if #ids <= 1 then return end
  local cur_str = tostring(current)
  for i, id in ipairs(ids) do
    if tostring(id) == cur_str then
      local target = ((i - 1 + step) % #ids) + 1
      pmacs.window.switch_buffer(ids[target])
      return
    end
  end
  -- Active buffer somehow isn't in the registry list (shouldn't
  -- happen). Fall through to the first buffer so the user isn't
  -- stranded.
  pmacs.window.switch_buffer(ids[1])
end

cmd { name = "editor.next-buffer",
      description = "Switch the active window to the next buffer in the registry, wrapping at the end.",
      fn = function() cycle_buffer(1) end }

cmd { name = "editor.previous-buffer",
      description = "Switch the active window to the previous buffer in the registry, wrapping at the start.",
      fn = function() cycle_buffer(-1) end }

-- Buffer list (Emacs Buffer-menu-mode-style) ---------------------------------
--
-- `editor.list-buffers` renders one line per registered buffer into a
-- regular buffer named `*buffer-list*`, then switches the active
-- window to it. Inside that buffer a small per-buffer keymap turns the
-- listing into a navigable mode:
--
--   RET / SPC : visit the buffer on the current line.
--   n / down  : next line.   p / up : previous line.
--   d         : mark current line for deletion (renders a `D` in col 0).
--   u         : unmark.
--   x         : kill every marked buffer, then refresh.
--   k         : kill the buffer on the current line immediately.
--   q         : return to the buffer that was active when the list opened.
--   g         : refresh the listing.
--
-- Marks live in a Lua-side set keyed by `tostring(BufferIdLua)`; the
-- `D` is part of the rendering, so any toggle re-renders the buffer.
-- Killing routes through `pmacs.buffer.kill`, which redirects any
-- window showing the doomed buffer to `*scratch*` first so windows
-- never end up pointing at a missing id.

local LIST_NAME = "*buffer-list*"
local LIST_HEADER = "  Buffer                          Size"

local list_state = {
  buffer_id = nil,        -- the *buffer-list* BufferIdLua, once created
  prev_buffer_id = nil,   -- buffer to return to on `q`
  marks = {},             -- { [tostring(id)] = BufferIdLua } -- mark-for-kill set
  line_to_buffer = {},    -- 1-based: data line N -> BufferIdLua
  bound = false,          -- buffer-local bindings installed?
}

local function find_list_buffer()
  for _, id in ipairs(pmacs.buffer.list()) do
    if pmacs.describe.buffer(id).name == LIST_NAME then
      return id
    end
  end
  return nil
end

local function render_list(buf)
  list_state.line_to_buffer = {}
  local lines = { LIST_HEADER }
  for _, id in ipairs(pmacs.buffer.list()) do
    local d = pmacs.describe.buffer(id)
    if d ~= nil then
      local marked = list_state.marks[tostring(id)] ~= nil
      local mark = marked and "D" or " "
      local modified = d.modified and "*" or " "
      table.insert(lines, string.format("%s%s %-30s %d bytes",
        mark, modified, d.name, d.length))
      list_state.line_to_buffer[#lines - 1] = id
    end
  end
  local body = table.concat(lines, "\n")
  local len = buf:len()
  if len > 0 then buf:delete(0, len) end
  if #body > 0 then buf:insert(0, body) end
end

local function bind_local_keymap(buf)
  local function bind(seq, command)
    pmacs.keymap.bind { scope = "buffer", buffer = buf, sequence = seq, command = command }
  end
  bind("RET",     "editor.buffer-list-visit")
  bind("SPC",     "editor.buffer-list-visit")
  bind("n",       "cursor.down")
  bind("<down>",  "cursor.down")
  bind("p",       "cursor.up")
  bind("<up>",    "cursor.up")
  bind("d",       "editor.buffer-list-mark-delete")
  bind("u",       "editor.buffer-list-unmark")
  bind("x",       "editor.buffer-list-execute")
  bind("k",       "editor.buffer-list-kill-now")
  bind("q",       "editor.buffer-list-quit")
  bind("g",       "editor.buffer-list-refresh")
end

local function ensure_list_buffer()
  local existing = find_list_buffer()
  if existing then
    list_state.buffer_id = existing
    return existing, false
  end
  local buf = pmacs.buffer.create(LIST_NAME)
  list_state.buffer_id = buf
  return buf, true
end

local function current_buffer_at_cursor()
  local line = pmacs.editor.cursor_line()
  if line < 1 then return nil end
  return list_state.line_to_buffer[line]
end

local function refresh()
  if list_state.buffer_id == nil then return end
  -- Drop any marks whose target buffer no longer exists.
  local pruned = {}
  for _, id in ipairs(pmacs.buffer.list()) do
    local key = tostring(id)
    if list_state.marks[key] then
      pruned[key] = id
    end
  end
  list_state.marks = pruned
  -- Wholesale-rewriting the buffer leaves the cursor at a stale byte
  -- offset (the engine adjusts text-view caches on edit but doesn't
  -- touch window.cursor). Save the line, rewrite, then re-seat the
  -- cursor on the same line (clamped to the new data extent).
  local saved_line = pmacs.editor.cursor_line()
  render_list(list_state.buffer_id)
  pmacs.window.switch_buffer(list_state.buffer_id)
  local data_count = #list_state.line_to_buffer
  local target = math.min(saved_line, data_count)
  if target < 1 and data_count >= 1 then target = 1 end
  for _ = 1, target do pmacs.editor.move_down() end
end

cmd { name = "editor.list-buffers",
      description = "Show the buffer list with buffer-menu-mode-style bindings.",
      fn = function()
        local active = pmacs.window.buffer()
        local buf, fresh = ensure_list_buffer()
        -- Don't overwrite prev_buffer_id when re-entering from inside
        -- *buffer-list* itself; preserve the original return target.
        if tostring(active) ~= tostring(buf) then
          list_state.prev_buffer_id = active
        end
        render_list(buf)
        if fresh or not list_state.bound then
          bind_local_keymap(buf)
          list_state.bound = true
        end
        pmacs.window.switch_buffer(buf)
        -- Land on the first data line, not the header.
        if #list_state.line_to_buffer >= 1 then
          pmacs.editor.move_down()
        end
      end }

cmd { name = "editor.buffer-list-visit",
      description = "Switch to the buffer named on the current *buffer-list* line.",
      fn = function()
        local target = current_buffer_at_cursor()
        if target == nil then
          pmacs.editor.set_status("not on a buffer line")
          return
        end
        pmacs.window.switch_buffer(target)
      end }

cmd { name = "editor.buffer-list-mark-delete",
      description = "Mark the buffer on the current line for deletion (column 0 = `D`).",
      fn = function()
        local target = current_buffer_at_cursor()
        if target == nil then
          pmacs.editor.set_status("not on a buffer line")
          return
        end
        list_state.marks[tostring(target)] = target
        refresh()
        pmacs.editor.move_down()
      end }

cmd { name = "editor.buffer-list-unmark",
      description = "Clear the deletion mark on the current line.",
      fn = function()
        local target = current_buffer_at_cursor()
        if target == nil then
          pmacs.editor.set_status("not on a buffer line")
          return
        end
        list_state.marks[tostring(target)] = nil
        refresh()
        pmacs.editor.move_down()
      end }

cmd { name = "editor.buffer-list-execute",
      description = "Kill every buffer marked with `D`, then refresh the listing.",
      fn = function()
        if list_state.buffer_id == nil then return end
        local doomed = {}
        for _, id in pairs(list_state.marks) do
          if tostring(id) ~= tostring(list_state.buffer_id) then
            table.insert(doomed, id)
          end
        end
        list_state.marks = {}
        local killed = 0
        for _, id in ipairs(doomed) do
          local ok = pcall(pmacs.buffer.kill, id)
          if ok then killed = killed + 1 end
        end
        refresh()
        pmacs.editor.set_status(string.format("killed %d buffer(s)", killed))
      end }

cmd { name = "editor.buffer-list-kill-now",
      description = "Kill the buffer on the current line immediately, then refresh.",
      fn = function()
        local target = current_buffer_at_cursor()
        if target == nil then
          pmacs.editor.set_status("not on a buffer line")
          return
        end
        if tostring(target) == tostring(list_state.buffer_id) then
          pmacs.editor.set_status("can't kill *buffer-list* from inside itself")
          return
        end
        local ok, err = pcall(pmacs.buffer.kill, target)
        if not ok then
          pmacs.editor.set_status("kill failed: " .. tostring(err))
          return
        end
        list_state.marks[tostring(target)] = nil
        refresh()
      end }

cmd { name = "editor.buffer-list-quit",
      description = "Switch the active window back to the buffer that was active when the list opened.",
      fn = function()
        local prev = list_state.prev_buffer_id
        if prev == nil then return end
        -- The previous buffer may have been killed in the meantime;
        -- fall back to *scratch* (creating it on demand) so the user
        -- still gets a valid buffer in the window.
        local exists = false
        for _, id in ipairs(pmacs.buffer.list()) do
          if tostring(id) == tostring(prev) then exists = true; break end
        end
        if not exists then
          local scratch
          for _, id in ipairs(pmacs.buffer.list()) do
            if pmacs.describe.buffer(id).name == "*scratch*" then
              scratch = id; break
            end
          end
          prev = scratch or pmacs.buffer.create("*scratch*")
        end
        pmacs.window.switch_buffer(prev)
      end }

cmd { name = "editor.buffer-list-refresh",
      description = "Re-render the *buffer-list* listing to reflect the current registry.",
      fn = function() refresh() end }

cmd { name = "editor.switch-buffer",
      description = "Switch the active window to a buffer chosen via M-x-style prompt.",
      fn = function()
        pmacs.minibuffer.read {
          prompt = "Switch to buffer: ",
          source = "buffers",
          history = "buffer",
          on_accept = function(name)
            if name == nil or name == "" then return end
            for _, id in ipairs(pmacs.buffer.list()) do
              if pmacs.describe.buffer(id).name == name then
                pmacs.window.switch_buffer(id)
                return
              end
            end
            pmacs.editor.set_status("no buffer: " .. name)
          end,
        }
      end }

-- Command palette (M-x) ------------------------------------------------------
--
-- Opens the minibuffer with a "commands" completion source, then
-- invokes whichever command the user accepts. History is bucketed
-- under "command" so M-x recalls recent picks.

cmd { name = "editor.execute-command",
      description = "Run a named command via the minibuffer (M-x).",
      fn = function()
        pmacs.minibuffer.read {
          prompt = "M-x ",
          source = "commands",
          history = "command",
          on_accept = function(name)
            if name == nil or name == "" then return end
            local ok, err = pcall(pmacs.command.invoke, name)
            if not ok then
              -- mlua's `tostring(err)` includes a Lua stack traceback
              -- separated by newlines. The status line is one row;
              -- show just the first (informative) line so the
              -- traceback doesn't leak into the terminal grid.
              local msg = tostring(err)
              local first = msg:match("^[^\n]*") or msg
              pmacs.editor.set_status("M-x error: " .. first)
            end
          end,
        }
      end }

-- Workers and project search (T M3.6, T M3.7) -------------------------------
--
-- `pmacs.workers.grep` is the runtime API; the user-facing entry
-- point is `M-x project.search`, which prompts for a query and
-- streams matches into a `*search-results*` buffer. Subsequent
-- searches cancel any in-flight predecessor through the
-- `supersede = "search"` key (M3.6 acceptance: cancel within 50 ms).
-- `pmacs.project.search(query, opts)` is the same logic without
-- the prompt --- callable from Lua scripts and tests.

pmacs.project = pmacs.project or {}

local SEARCH_RESULTS_NAME = "*search-results*"
local active_search_id = nil

local function search_results_buffer()
  for _, id in ipairs(pmacs.buffer.list()) do
    if pmacs.describe.buffer(id).name == SEARCH_RESULTS_NAME then
      return id
    end
  end
  return pmacs.buffer.create(SEARCH_RESULTS_NAME)
end

function pmacs.project.search(query, opts)
  if type(query) ~= "string" or query == "" then
    return nil
  end
  opts = opts or {}
  local root = opts.root or "."
  local buf = search_results_buffer()
  -- Replace any prior contents in one shot, then append a header.
  -- The buffer is reused across searches; clearing here is what
  -- gives the user a fresh page per query.
  if buf:len() > 0 then buf:delete(0, buf:len()) end
  buf:insert(0, "Searching for: " .. query .. "\n\n")
  pmacs.window.switch_buffer(buf)
  local stream = pmacs.workers.grep(
    { root = root, pattern = query },
    { supersede = "search" })
  active_search_id = stream:id()
  -- Capture the id at registration time so callbacks for a
  -- superseded predecessor (whose worker hasn't yet observed
  -- cancel) drop their late batches instead of polluting the
  -- successor's results.
  local my_id = active_search_id
  stream:on_batch(function(items)
    if my_id ~= active_search_id then return end
    for _, m in ipairs(items) do
      buf:insert(buf:len(), string.format("%s:%d:%d: %s\n",
        m.file, m.line, m.match_start, m.text))
    end
  end)
  stream:on_close(function(status, _value)
    if my_id ~= active_search_id then return end
    buf:insert(buf:len(), string.format("\n-- search %s --\n", status))
  end)
  return stream
end

cmd { name = "project.search",
      description = "Parallel grep across the project; new queries cancel the predecessor.",
      fn = function()
        pmacs.minibuffer.read {
          prompt = "Search: ",
          history = "search",
          on_accept = function(query)
            if query == nil or query == "" then return end
            pmacs.project.search(query)
          end,
        }
      end }

cmd { name = "editor.list-workers",
      description = "Open the *workers* observability buffer in the active window.",
      fn = function() pmacs.window.switch_buffer(pmacs.workers.show()) end }

-- T M5.6f: describe-instance ------------------------------------------------
--
-- Two commands by design (pmacs has no prefix-arg system in v0.1):
--   * `editor.describe-instance`        echoes one line to the status row.
--   * `editor.describe-instance-buffer` opens *pmacs-instance* with detail.
--
-- Both work whether or not an outbound attachment is in scope. In v0.1
-- Local mode the attachment is always nil (the process is its own
-- instance) so the output describes the running process self.

cmd { name = "editor.describe-instance",
      description = "Echo a one-line summary of this pmacs instance to the status row.",
      fn = function() pmacs.editor.set_status(pmacs.instance.echo_line()) end }

-- Track the instance buffer id so we only rebind `q` once per
-- incarnation (mirrors the workers buffer's C-c C-k rebind pattern).
local instance_buffer_id = nil

cmd { name = "editor.describe-instance-buffer",
      description = "Open the *pmacs-instance* detail buffer in the active window.",
      fn = function()
        local id = pmacs.instance.show()
        if instance_buffer_id ~= id then
          pcall(function()
            pmacs.keymap.unbind { scope = "buffer", buffer = id, sequence = "q" }
          end)
          pmacs.keymap.bind {
            scope = "buffer",
            buffer = id,
            sequence = "q",
            command = "buffer.kill-this",
          }
          instance_buffer_id = id
        end
        pmacs.window.switch_buffer(id)
      end }

cmd { name = "buffer.kill-this",
      description = "Kill the buffer shown in the active window.",
      fn = function()
        local id = pmacs.window.buffer()
        if id ~= nil then pmacs.buffer.kill(id) end
      end }

-- describe-command (M9.6 acceptance lever) ---------------------------------
--
-- M9.6's third acceptance bullet ("describe-command reports the
-- tool's schema as the documentation") needs a user-callable entry
-- point — `pmacs.describe.command(name)` returns the table at the
-- Rust layer, but without this M-x command there's no way to reach
-- it interactively. Modeled on `editor.describe-instance-buffer`:
-- a single `*help*` buffer reused across invocations, with a
-- buffer-local `q` → `buffer.kill-this` for dismissal.

local HELP_BUFFER_NAME = "*help*"
local help_buffer_id = nil

local function find_or_create_help_buffer()
  for _, id in ipairs(pmacs.buffer.list()) do
    local d = pmacs.describe.buffer(id)
    if d ~= nil and d.name == HELP_BUFFER_NAME then
      return id, false
    end
  end
  return pmacs.buffer.create(HELP_BUFFER_NAME), true
end

local function show_help_text(text)
  local buf, fresh = find_or_create_help_buffer()
  -- `buf:delete(0, len)` matches `editor.list-buffers` render_list — we
  -- replace contents wholesale rather than diffing, since *help* is
  -- always reflowed for the new subject.
  local len = buf:len()
  if len > 0 then buf:delete(0, len) end
  if #text > 0 then buf:insert(0, text) end
  if fresh or help_buffer_id ~= buf then
    pcall(function()
      pmacs.keymap.unbind { scope = "buffer", buffer = buf, sequence = "q" }
    end)
    pmacs.keymap.bind {
      scope = "buffer",
      buffer = buf,
      sequence = "q",
      command = "buffer.kill-this",
    }
    help_buffer_id = buf
  end
  pmacs.window.switch_buffer(buf)
end

cmd { name = "editor.describe-command",
      description = "Prompt for a command name and render its description in *help*.",
      fn = function()
        pmacs.minibuffer.read {
          prompt = "Describe command: ",
          source = "commands",
          history = "command",
          on_accept = function(name)
            if name == nil or name == "" then return end
            local info = pmacs.describe.command(name)
            if info == nil then
              pmacs.editor.set_status("describe-command: no such command: " .. name)
              return
            end
            local lines = { name, "" }
            local desc = info.description
            if type(desc) ~= "string" or desc == "" then
              desc = "(no description)"
            end
            lines[#lines + 1] = desc
            local key_bindings = info.key_bindings
            if type(key_bindings) == "table" and #key_bindings > 0 then
              lines[#lines + 1] = ""
              lines[#lines + 1] = "Bindings:"
              for _, b in ipairs(key_bindings) do
                local seq = (type(b) == "table" and b.sequence) or tostring(b)
                lines[#lines + 1] = "  " .. tostring(seq)
              end
            end
            show_help_text(table.concat(lines, "\n"))
          end,
        }
      end }
