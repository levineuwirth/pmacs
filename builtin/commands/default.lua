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
-- CUA type-over: inserting with an active selection replaces it in a
-- SINGLE edit (one undo step — `insert_char_over_region` emits one
-- `Replace`, not a `delete` + `insert` pair). Without a selection it
-- is a plain insert. The pmacs-gpu frontend relies on this: its
-- optimistic-insert path detects an own-window selection and
-- round-trips the key so these commands run daemon-side.
cmd { name = "buffer.newline",
      description = "Insert a newline at the cursor, replacing the active region.",
      fn = function() ed.insert_char_over_region(10) end }
cmd { name = "buffer.tab",
      description = "Insert a tab at the cursor, replacing the active region.",
      fn = function() ed.insert_char_over_region(9) end }
cmd { name = "buffer.self-insert",
      description = "Insert the codepoint argument at the cursor, replacing the active region.",
      fn = function(codepoint) ed.insert_char_over_region(codepoint) end }

-- Incremental search ---------------------------------------------------------
--
-- C-s / C-r begin a live in-buffer isearch: the match under the cursor
-- highlights as you type, the same key steps to the next/previous
-- match, RET accepts (keeping the highlights until the next edit), and
-- C-g / Esc restore the pre-search cursor. C-M-s / C-M-r start a regex
-- search, and M-r toggles literal <-> regex mid-search (handled in
-- Rust). While a search is running every keystroke is intercepted in
-- Rust (dispatch_search_key), so these commands only run to *start* a
-- search from an idle keymap. ed.search_start(forward, regex).
cmd { name = "search.forward",
      description = "Start an incremental search forward from the cursor.",
      fn = function() ed.search_start(true, false) end }
cmd { name = "search.backward",
      description = "Start an incremental search backward from the cursor.",
      fn = function() ed.search_start(false, false) end }
cmd { name = "search.forward-regex",
      description = "Start an incremental regex search forward from the cursor.",
      fn = function() ed.search_start(true, true) end }
cmd { name = "search.backward-regex",
      description = "Start an incremental regex search backward from the cursor.",
      fn = function() ed.search_start(false, true) end }

-- Query-replace (Arc 2). Two chained minibuffer prompts collect the
-- from/to strings (separate history buckets so search patterns and
-- replacement text don't mix), then ed.query_replace_start begins the
-- core interactive session (y/n/!/./q handled by a dispatcher shadow).
-- An empty FROM is rejected (nothing to search); an empty TO is valid
-- and means deletion (Q#QR3).
local function begin_query_replace(regex)
  pmacs.minibuffer.read {
    prompt = regex and "Query replace regexp: " or "Query replace: ",
    history = "query-replace-from",
    on_accept = function(from)
      if from == nil or from == "" then
        pmacs.editor.set_status("query-replace: empty search string")
        return
      end
      pmacs.minibuffer.read {
        prompt = string.format(
          regex and "Query replace regexp %s with: " or "Query replace %s with: ", from),
        history = "query-replace-to",
        on_accept = function(to)
          ed.query_replace_start(from, to or "", regex)
        end,
      }
    end,
  }
end

cmd { name = "query-replace",
      description = "Interactively replace a string from the cursor forward (M-%).",
      fn = function() begin_query_replace(false) end }
cmd { name = "query-replace-regexp",
      description = "Interactively replace a regexp from the cursor forward (C-M-%).",
      fn = function() begin_query_replace(true) end }

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

-- Clipboard (Q#CM6) ----------------------------------------------------------
-- Copy/cut publish the selection to the OS clipboard (OSC 52 in the TUI,
-- arboard in the GPU); paste inserts the in-app slot, which Ctrl-V /
-- bracketed paste also refreshes. The default bindings are the Emacs
-- kill/yank set (M-w / C-w / C-y, C-x h), which were all free.

-- The cut/copy/paste trio delegates to the kill ring (Arc 2): kills
-- accumulate, C-y yanks the head, M-y cycles older entries. Resolution
-- happens at invoke time, so chunk load order doesn't matter; the
-- context menu invokes these by name and inherits the ring for free.
cmd { name = "edit.copy",
      description = "Save the active region to the kill ring (and OS clipboard).",
      fn = function() pmacs.killring.copy() end }
cmd { name = "edit.cut",
      description = "Kill the active region into the kill ring (and OS clipboard).",
      fn = function() pmacs.killring.cut() end }
cmd { name = "edit.paste",
      description = "Yank the most recent kill at the cursor, replacing any region.",
      fn = function() pmacs.killring.yank() end }
cmd { name = "edit.select-all",
      description = "Select the whole buffer.",
      fn = function() ed.select_all() end }

-- File I/O -------------------------------------------------------------------

cmd { name = "buffer.save", description = "Save the current buffer to its backing file.",
      fn = function()
        if not pmacs.hook.run("buffer.before-save") then
          ed.set_status("save vetoed by buffer.before-save")
          return
        end
        -- `ed.save()` refuses when the file changed on disk since this
        -- buffer read it, rather than clobbering the other writer. It
        -- reports how to override; `buffer.after-save` must not fire.
        if ed.save() then
          pmacs.hook.run("buffer.after-save")
        end
      end }

cmd { name = "buffer.save-anyway",
      description = "Save, overwriting a file that changed on disk since it was read.",
      fn = function()
        if not pmacs.hook.run("buffer.before-save") then
          ed.set_status("save vetoed by buffer.before-save")
          return
        end
        if ed.save_ignoring_disk_changes() then
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
cmd { name = "window.toggle-line-numbers",
      description = "Toggle the active window's line-number gutter (off / absolute).",
      fn = function()
        pmacs.window.set_line_numbers(
          pmacs.window.line_numbers() == "off" and "absolute" or "off")
      end }

-- Pick a line-number mode directly from the completion dropdown, rather
-- than cycling. Arrow-navigable candidates (off/absolute/relative/hybrid).
cmd { name = "window.set-line-numbers",
      description = "Set the active window's line-number mode (off/absolute/relative/hybrid).",
      fn = function()
        pmacs.minibuffer.read {
          prompt = "Line numbers: ",
          source = function() return { "off", "absolute", "relative", "hybrid" } end,
          history = "line-numbers",
          on_accept = function(mode)
            if mode == nil or mode == "" then return end
            local ok, err = pcall(pmacs.window.set_line_numbers, mode)
            if not ok then
              pmacs.editor.set_status("line-numbers: " .. (tostring(err):match("^[^\n]*") or ""))
            end
          end,
        }
      end }
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

-- find-file (dired arc Stage 0; docs/archive/framings/dired-framing.md Q#DR11) ----------------
--
-- Until now pmacs had no discoverable way to open a file by path: a file
-- entered a session only from the CLI, an LSP jump, a project-search
-- visit, or `C-x C-r` (whose prompt does pass free text through, but
-- completes only over the recent list). This is that surface.
--
-- Two substrate facts shape it, and both are load-bearing:
--
-- 1. COMPLETION IS FLAT. `source = "files"` lists ONE directory and
--    yields bare basenames (`minibuffer.rs` `list_directory`), capped at
--    the shared candidate limit. A custom function source could not do
--    better: sources are called with NO arguments, so a callback cannot
--    see the input to re-root on, and it runs synchronously outside any
--    coroutine, where `Handle:await()` raises --- so it cannot list a
--    directory either. Hierarchical completion is a named Rust change in
--    the framing, not something this command can fake.
--
-- 2. A SELECTED CANDIDATE SHADOWS TYPED TEXT. `recompute_candidates`
--    sets `selected = Some(0)` whenever the candidate list is non-empty,
--    and `resolve_accepted_value` returns the CANDIDATE whenever
--    anything is selected. So `on_accept` receives typed text only when
--    the input filters every candidate away --- which, since candidates
--    are basenames and the filter is a subsequence match, is exactly
--    when the input contains a `/`. That makes the deeper-path case work
--    (`sub/inner.txt` matches no basename, so it arrives verbatim) and
--    leaves TWO documented consequences, each pinned by a test rather
--    than left to be rediscovered:
--
--    (a) typing a NEW bare name that happens to be a subsequence of an
--        existing entry opens the existing file instead of creating the
--        new one --- `find_file_selected_candidate_shadows_typed_text`.
--        A new bare name that matches nothing is unaffected and creates
--        normally (`find_file_bare_new_name_creates_in_the_root`).
--    (b) accepting on EMPTY input opens the first candidate in sort
--        order. `fuzzy_score` returns `Some(0)` for an empty needle, so
--        everything ties and `filter_and_sort` falls back to
--        lexicographic order --- which puts dotfiles first, and can put
--        a DIRECTORY first, in which case the open fails and reports.
--        This is the same mechanism `M-x` and `switch-buffer` already
--        have, so it is inherited rather than introduced; it is recorded
--        as decided, not overlooked, and listed in the framing's
--        deferrals beside the accept-semantics fix that would close it.
--
-- The root is the active buffer's directory, or the process cwd when the
-- buffer has no backing path (`source_root` defaults to "." Rust-side,
-- so the nil case needs no special handling here). It appears in the
-- prompt because the field itself must stay empty: any prefill would
-- contain a `/` and filter every candidate away, killing completion.

-- Directory part of a path. "/a/b" -> "/a"; "/a" -> "/"; "a" -> nil.
local function find_file_dirname(path)
  local dir = path:match("^(.*)/[^/]*$")
  if dir == nil then return nil end
  if dir == "" then return "/" end
  return dir
end

-- Expand a leading `~` component using $HOME: `~` -> $HOME, `~/x` ->
-- $HOME/x. `~user` is left alone (no passwd lookup), matching the core's
-- own `expand_tilde`.
--
-- This has to happen HERE, before the path reaches the core, because
-- `get_or_load_buffer` normalizes the path it STORES but loads from the
-- raw one --- so a `~/...` path deduplicates against an already-open
-- buffer yet fails to load when the file is not open yet. Expanding up
-- front makes both halves agree.
local function find_file_expand_tilde(path)
  local home = os.getenv("HOME")
  if home == nil or home == "" then return path end
  if home:sub(-1) == "/" then home = home:sub(1, -2) end
  if path == "~" then return home end
  local rest = path:match("^~/(.*)$")
  if rest == nil then return path end
  return home .. "/" .. rest
end

-- Turn an accepted value into a path. The value is either a bare
-- basename (a selected candidate) or whatever the user typed, so a
-- non-absolute value joins onto the prompt's root --- which resolves
-- both cases to the same file when they name the same one.
local function find_file_resolve(root, value)
  local path = find_file_expand_tilde(value)
  if path:sub(1, 1) == "/" then return path end
  local base = root or "."
  if base:sub(-1) == "/" then return base .. path end
  return base .. "/" .. path
end

-- The active buffer's directory, or nil when it has no backing path.
local function find_file_root()
  local buf = pmacs.window.buffer()
  if buf == nil then return nil end
  local ok, path = pcall(function() return buf:path() end)
  if not (ok and path) then return nil end
  return find_file_dirname(path)
end

cmd { name = "find-file",
      description = "Open a file by path, completing within one directory.",
      fn = function()
        local root = find_file_root()
        pmacs.minibuffer.read {
          prompt = "Find file (" .. (root or ".") .. "): ",
          source = "files",
          source_root = root,
          history = "find-file",
          on_accept = function(value)
            if value == nil or value == "" then return end
            local path = find_file_resolve(root, value)
            -- A path that does not exist yet CREATES a buffer bound to
            -- it: `display_file` routes through `resolve_target_buffer`,
            -- which on NotFound creates, binds, and sets "[new file]".
            -- That is Emacs parity and deliberate, so only a real
            -- failure (a directory, a permission error) reaches here.
            local ok, err = pcall(pmacs.window.display_file, path, { select = true })
            if not ok then
              pmacs.editor.set_status("find-file: " .. tostring(err))
            end
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
            -- invoke_interactive records the command boundary (kill
            -- ring Q#KR2), so chain-sensitive commands behave as under
            -- Emacs's execute-extended-command: M-x kill-line then C-k
            -- appends; C-k then M-x kill-line does not.
            local ok, err = pcall(pmacs.command.invoke_interactive, name)
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

-- Workers and project search (T M3.6, T M3.7; grep-mode upgrade per
-- docs/archive/framings/compile-mode-framing.md Q#CM7) ---------------------------------------
--
-- `pmacs.workers.grep` is the runtime API; the user-facing entry
-- point is `M-x project.search`, which prompts for a query and
-- streams matches into a `*search-results*` buffer. Subsequent
-- searches cancel any in-flight predecessor through the
-- `supersede = "search"` key (M3.6 acceptance: cancel within 50 ms).
-- `pmacs.project.search(query, opts)` is the same logic without
-- the prompt --- callable from Lua scripts and tests.
--
-- The panel is a first-class locations buffer: read-only intercept
-- with bypass writes, RET visits, n/p walk match lines, q restores,
-- undo chords are no-ops, and each run claims the unified
-- next-error source so M-g n walks matches. The worker's matches
-- are already structured — no regex parsing. Every producer write
-- runs the revision check BEFORE mutating so a no-hook external
-- edit cannot be masked by the next batch advancing the expected
-- revision.

pmacs.project = pmacs.project or {}

local SEARCH_RESULTS_NAME = "*search-results*"
local GREP_DESYNC_MARKER = "\n[output desynced by external edit]\n"
local active_search_id = nil
local active_search_stream = nil
-- Panel state: buffer incarnation, the root this search ran with
-- (retained across interactive supersedes issued from inside the
-- pathless panel), match locations, and the revision guard.
local search_panel = nil

local function grep_count_newlines(s)
  local n = 0
  local i = 0
  while true do
    i = s:find("\n", i + 1, true)
    if not i then return n end
    n = n + 1
  end
end

local function search_panel_alive()
  return search_panel ~= nil
    and search_panel.buf ~= nil
    and search_panel.buf:is_valid()
end

-- Resync after an external edit (the Q#CM2 discipline, grep shape):
-- drop row anchors (a revision carries no edit range), append the
-- marker, and recompute the row epoch. The match-location list
-- survives for M-g n.
local function search_panel_resync()
  local p = search_panel
  for _, m in ipairs(p.matches) do
    m.row = nil
  end
  local buf = p.buf
  buf:insert(buf:len(), GREP_DESYNC_MARKER, { bypass_intercept = true })
  p.next_row = grep_count_newlines(buf:slice(0, buf:len()))
  p.expected_rev = buf:revision()
end

local function search_panel_check_rev()
  if not search_panel_alive() then return false end
  local p = search_panel
  if p.expected_rev ~= nil and p.buf:revision() ~= p.expected_rev then
    search_panel_resync()
  end
  return true
end

-- Revision-checked producer append: check BEFORE the write (so a
-- mismatch is marked rather than masked), record after. Returns the
-- row the text landed on, or nil when the panel is gone.
local function search_panel_append(text)
  if not search_panel_check_rev() then return nil end
  local p = search_panel
  local row = p.next_row
  local buf = p.buf
  buf:insert(buf:len(), text, { bypass_intercept = true })
  p.expected_rev = buf:revision()
  p.next_row = row + grep_count_newlines(text)
  return row
end

local SEARCH_UNDO_CHORDS = { "C-/", "C-_", "C-4", "C-x u", "C-?", "C-S-_", "C-x r" }

local function ensure_search_panel()
  if search_panel_alive() then return search_panel end
  local p = search_panel or { matches = {}, match_index = 0 }
  search_panel = p
  local buf
  for _, id in ipairs(pmacs.buffer.list()) do
    if pmacs.describe.buffer(id).name == SEARCH_RESULTS_NAME then
      buf = id
      break
    end
  end
  p.buf = buf or pmacs.buffer.create(SEARCH_RESULTS_NAME)
  pmacs.buffer.add_intercept(p.buf, function()
    error(SEARCH_RESULTS_NAME .. " is read-only")
  end)
  pmacs.buffer.set_round_trip_input(p.buf, true)
  -- Kill-mid-search safety (Q#CM7): cancel the stream and
  -- invalidate the id so late callbacks drop instead of writing
  -- through a stale handle; the next search recreates the buffer.
  pcall(pmacs.buffer.on_removed, p.buf, function()
    if active_search_stream then
      pcall(function() active_search_stream:cancel() end)
    end
    active_search_id = nil
    active_search_stream = nil
    p.buf = nil
  end)
  local function bind(seq, command)
    pmacs.keymap.bind {
      scope = "buffer", buffer = p.buf, sequence = seq, command = command,
    }
  end
  bind("RET", "project-search.visit")
  bind("n", "project-search.next-line")
  bind("p", "project-search.previous-line")
  bind("q", "project-search.quit")
  for _, seq in ipairs(SEARCH_UNDO_CHORDS) do
    bind(seq, "compile.undo-noop")
  end
  return p
end

-- Immediate command-path recovery for the panel (the Q#CM2 trigger
-- the compile slots already have): M-x/menu edits fire
-- buffer.after-edit, and after a COMPLETED search no producer write
-- or navigation may ever come — without this, an M-x buffer.undo
-- left corrupted output unmarked indefinitely (PR #113 round-1
-- finding 2). Hook edits don't re-fire the hook, so the resync
-- marker can be appended from here safely.
pmacs.hook.add("buffer.after-edit", function()
  local cur = pmacs.window.buffer()
  if cur and search_panel_alive() and cur == search_panel.buf then
    search_panel_check_rev()
  end
end)

-- Cursor walk via primitives (the lsp.lua visit idiom; 0-based).
-- Movement-bounded like compile.lua's walk: a match pointing past
-- EOF/EOL clamps instead of looping (the file may have changed on
-- disk since the worker scanned it).
local function search_move_cursor_to(line, col)
  pmacs.editor.move_line_start()
  while pmacs.editor.cursor_line() > 0 do
    pmacs.editor.move_up()
  end
  for _ = 1, line do
    local before = pmacs.editor.cursor_line()
    pmacs.editor.move_down()
    if pmacs.editor.cursor_line() == before then break end
  end
  local row = pmacs.editor.cursor_line()
  for _ = 1, col do
    local before = pmacs.editor.cursor()
    pmacs.editor.move_right()
    if pmacs.editor.cursor() == before then break end
    if pmacs.editor.cursor_line() ~= row then
      pmacs.editor.move_left()
      break
    end
  end
end

local function visit_match(idx)
  local p = search_panel
  local m = p and p.matches[idx]
  if not m then return end
  -- Worker match paths are relative to the search root, not the
  -- cwd — resolve against the root this search ran with.
  local path = m.file
  if path:sub(1, 1) ~= "/" then
    local root = p.root or "."
    if root:sub(-1) ~= "/" then root = root .. "/" end
    path = root .. path
  end
  pmacs.editor.push_jump()
  local ok, err = pcall(pmacs.buffer.find_or_open, path)
  if not ok then
    pmacs.editor.jump_back()
    pmacs.editor.set_status("search: failed to open " .. path .. ": " .. tostring(err))
    return
  end
  search_move_cursor_to(m.line, m.col)
  p.match_index = idx
end

-- Root resolution (Q#CM7): explicit opt > the panel's stored root
-- when searching from inside the pathless panel (the natural
-- supersede path would otherwise silently degrade to ".") > the
-- active file's project root > ".".
local function resolve_search_root(opts)
  if opts.root then return opts.root end
  local cur = pmacs.window.buffer()
  if cur and search_panel_alive() and cur == search_panel.buf and search_panel.root then
    return search_panel.root
  end
  if cur then
    local okp, path = pcall(function() return cur:path() end)
    if okp and path then
      local okd, proj = pcall(pmacs.project.detect, path)
      if okd and proj and proj.root then return proj.root end
    end
  end
  return "."
end

function pmacs.project.search(query, opts)
  if type(query) ~= "string" or query == "" then
    return nil
  end
  opts = opts or {}
  local root = resolve_search_root(opts)
  local p = ensure_search_panel()
  -- q-target discipline: never capture a generated buffer.
  local cur = pmacs.window.buffer()
  if cur
    and not (pmacs.compile
      and pmacs.compile.is_generated_buffer
      and pmacs.compile.is_generated_buffer(cur))
  then
    p.prev = cur
  end
  p.root = root
  p.matches = {}
  p.match_index = 0
  -- Replace any prior contents in one shot, then append a header.
  -- The buffer is reused across searches; clearing here is what
  -- gives the user a fresh page per query.
  local buf = p.buf
  if buf:len() > 0 then buf:delete(0, buf:len(), { bypass_intercept = true }) end
  local header = "Searching for: " .. query .. "\n\n"
  buf:insert(0, header, { bypass_intercept = true })
  p.next_row = grep_count_newlines(header)
  p.expected_rev = buf:revision()
  pmacs.window.switch_buffer(buf)
  local stream = pmacs.workers.grep(
    { root = root, pattern = query },
    { supersede = "search" })
  active_search_id = stream:id()
  active_search_stream = stream
  -- Claim the unified next-error source (Q#CM5): M-g n walks the
  -- match list, which survives desync epochs. Guarded: this chunk
  -- loads before the runtime chunks, and a minimal harness may
  -- invoke search without compile.lua installed.
  local claim = pmacs.errors and pmacs.errors.claim or function() end
  claim {
    name = "grep",
    next = function()
      if not p.matches or #p.matches == 0 then
        pmacs.editor.set_status("search: no matches")
        return
      end
      if p.match_index >= #p.matches then
        pmacs.editor.set_status("no more errors")
        return
      end
      visit_match(p.match_index + 1)
    end,
    previous = function()
      if p.match_index <= 1 then
        pmacs.editor.set_status("no more errors")
        return
      end
      visit_match(p.match_index - 1)
    end,
  }
  -- Capture the id at registration time so callbacks for a
  -- superseded predecessor (whose worker hasn't yet observed
  -- cancel) drop their late batches instead of polluting the
  -- successor's results.
  local my_id = active_search_id
  stream:on_batch(function(items)
    if my_id ~= active_search_id then return end
    if not search_panel_alive() then return end
    for _, m in ipairs(items) do
      local row = search_panel_append(string.format("%s:%d:%d: %s\n",
        m.file, m.line, m.match_start, m.text))
      if row then
        -- Grep normalization (Q#CM7): line is 1-based → minus one;
        -- match_start is already a 0-based byte offset in the line.
        p.matches[#p.matches + 1] = {
          file = m.file,
          line = m.line - 1,
          col = m.match_start,
          row = row,
        }
      end
    end
  end)
  stream:on_close(function(status, _value)
    if my_id ~= active_search_id then return end
    if not search_panel_alive() then return end
    search_panel_append(string.format("\n-- search %s --\n", status))
  end)
  return stream
end

local function match_on_row(row)
  local p = search_panel
  if not p then return nil end
  for i, m in ipairs(p.matches) do
    if m.row == row then return i end
  end
  return nil
end

local function active_is_search_panel()
  local cur = pmacs.window.buffer()
  return cur ~= nil and search_panel_alive() and cur == search_panel.buf
end

cmd { name = "project-search.visit",
      description = "Visit the match on the current line of *search-results*.",
      fn = function()
        if not active_is_search_panel() then return end
        if not search_panel_check_rev() then return end
        local idx = match_on_row(pmacs.editor.cursor_line())
        if not idx then
          pmacs.editor.set_status("no match on this line")
          return
        end
        visit_match(idx)
      end }

local function search_step_line(direction)
  if not active_is_search_panel() then return end
  if not search_panel_check_rev() then return end
  local from = pmacs.editor.cursor_line()
  local best = nil
  for _, m in ipairs(search_panel.matches) do
    if m.row then
      if direction > 0 and m.row > from and (not best or m.row < best) then
        best = m.row
      elseif direction < 0 and m.row < from and (not best or m.row > best) then
        best = m.row
      end
    end
  end
  if not best then
    pmacs.editor.set_status("no more errors")
    return
  end
  local cur = from
  while cur < best do
    pmacs.editor.move_down()
    cur = cur + 1
  end
  while cur > best do
    pmacs.editor.move_up()
    cur = cur - 1
  end
  pmacs.editor.move_line_start()
end

cmd { name = "project-search.next-line",
      description = "Move to the next match line within *search-results*.",
      fn = function() search_step_line(1) end }

cmd { name = "project-search.previous-line",
      description = "Move to the previous match line within *search-results*.",
      fn = function() search_step_line(-1) end }

cmd { name = "project-search.quit",
      description = "Leave *search-results*, restoring the previous buffer.",
      fn = function()
        if not active_is_search_panel() then return end
        local target = search_panel.prev
        if not (target and target:is_valid()) then
          for _, id in ipairs(pmacs.buffer.list()) do
            if pmacs.describe.buffer(id).name == "*scratch*" then
              target = id
              break
            end
          end
          target = target or pmacs.buffer.create("*scratch*")
        end
        pmacs.window.switch_buffer(target)
      end }

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

--- Internal seam (journey Stage 1b-3): render `text` into the shared
--- `*help*` buffer. Exposed under the underscore convention so
--- `runtime/welcome.lua`'s `M-x help` renders through THIS mechanism
--- rather than growing a second help surface — `commands/default.lua`
--- loads before the runtime chunks, so the seam is present by then.
---
--- Inherits this mechanism's two known gaps, recorded rather than
--- papered over: it writes with `delete`/`insert` instead of
--- `set_generated_contents` (so the buffer stays ordinarily editable and
--- keeps its undo history), and it finds `*help*` BY NAME, so a foreign
--- buffer of that name would be cleared.
function pmacs.editor._show_help(text)
  show_help_text(text)
end

-- The two family commands below call `pmacs.editor._show_help`, NOT the
-- local `show_help_text`, even though they are in the same file and the
-- local is in scope. That is deliberate: discovery Stage 1's funnel
-- ("one owner for `*help*` writes") is only real if every command goes
-- through the PUBLIC seam — a command calling the local bypasses any
-- later change made at the seam, and bypassed the acceptance pin that
-- counts seam calls, which is how this was caught.

cmd { name = "help.describe-command",
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
            pmacs.editor._show_help(table.concat(lines, "\n"))
          end,
        }
      end }

-- describe-setting (config registry, acceptance 33) -------------------------
--
-- The configuration registry's discovery surface. `pmacs.config.describe`
-- returns the metadata table at the Rust layer; this is the interactive
-- way in, modeled on `editor.describe-command` directly above and sharing
-- its `*help*` buffer handling.
--
-- The prompt now completes. That comment used to say `source` is "a fixed
-- vocabulary ("commands", "buffers") resolved in Rust" — it is not:
-- `parse_completion_source` also accepts a Lua **function**, which
-- becomes `CompletionSource::Custom` and is called for candidates. So a
-- settings source needs no Rust at all (discovery Stage 1).
--
-- **Completion here is assistance, not validation.**
-- `resolve_accepted_value` returns the literal typed text whenever no
-- candidate is selected, so a non-matching typo still reaches
-- `on_accept` and the `no such setting` path below still earns its
-- keep. Refusing a non-candidate outright is Rust work and is deferred.

local function describe_setting_lines(name, info)
  -- Header block mirrors help.rs's `format_hook_text`: aligned label
  -- column, then a blank line, then the description as prose.
  local lines = {
    "Setting: " .. name,
    "  Type:       " .. tostring(info.type),
    "  Default:    " .. tostring(info.default),
    "  Value:      " .. tostring(info.value),
    "  Mutability: " .. tostring(info.mutability),
    "  Source:     " .. tostring(info.source),
  }
  if info.min ~= nil then lines[#lines + 1] = "  Min:        " .. tostring(info.min) end
  if info.max ~= nil then lines[#lines + 1] = "  Max:        " .. tostring(info.max) end
  if type(info.choices) == "table" and #info.choices > 0 then
    lines[#lines + 1] = "  Choices:    " .. table.concat(info.choices, ", ")
  end
  lines[#lines + 1] = ""
  local desc = info.description
  if type(desc) ~= "string" or desc == "" then desc = "(no description)" end
  lines[#lines + 1] = desc
  lines[#lines + 1] = ""
  -- Overrides. `global` is the global-chain resolution regardless of the
  -- buffer argument, so "same as default" is a real, distinguishable
  -- state from "overridden to the same value" only via is_set — which is
  -- why this reports the resolved values rather than claiming presence.
  lines[#lines + 1] = "Global value: " .. tostring(info.global)
  if info.buffer_local ~= nil then
    lines[#lines + 1] = "Buffer-local override: " .. tostring(info.buffer_local)
  end
  return lines
end

cmd { name = "help.describe-setting",
      description = "Prompt for a setting name and render its definition in *help*.",
      fn = function()
        pmacs.minibuffer.read {
          prompt = "Describe setting: ",
          history = "command",
          -- Sorted for DETERMINISTIC POOL CONSTRUCTION, not display
          -- order: `recompute_candidates` runs `filter_and_sort`, which
          -- ranks by fuzzy score and tie-breaks lexically, so this order
          -- never reaches the user. It matters because
          -- `.take(CANDIDATE_LIMIT)` is applied to the filtered iterator
          -- BEFORE that sort, so pool order decides which candidates
          -- survive truncation; registration order would make that vary
          -- with an unrelated config edit.
          source = function()
            local names = {}
            for _, d in ipairs(pmacs.config.list()) do
              names[#names + 1] = d.name
            end
            table.sort(names)
            return names
          end,
          on_accept = function(name)
            if name == nil or name == "" then return end
            -- An undefined name raises NotFound rather than returning nil
            -- (the define-before-set posture, Q#CR10), so this must pcall.
            local buf = pmacs.window.buffer()
            local ok, info = pcall(pmacs.config.describe, name, buf)
            if not ok or type(info) ~= "table" then
              pmacs.editor.set_status("describe-setting: no such setting: " .. name)
              return
            end
            pmacs.editor._show_help(table.concat(describe_setting_lines(name, info), "\n"))
          end,
        }
      end }
