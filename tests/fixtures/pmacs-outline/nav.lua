-- pmacs-outline/nav.lua --- Outline navigation commands (T M8.9).
--
-- Three commands satisfying acceptance bullet 4: next-headline,
-- parent-headline, fold-subtree (the toggle is in init.lua's
-- toggle_fold). All three operate in the *visible* buffer: they
-- read cursor position, map the visible line back to a source byte
-- offset, query the parser for the target entry, then map the
-- target's source byte forward to a visible line and move the
-- cursor there.
--
-- Cursor positioning workaround: pmacs v0.1 has no `move_to_byte`
-- or `move_to_line` (SP-1 in V0.2-PREREQUISITES.md). We walk via
-- move_up to bottom-out at line 0, then move_down N times --- same
-- pattern dired-class and magit-class use.

local M = {}

local function move_cursor_to_visible_line(target_line)
  local guard = 0
  local prev = pmacs.editor.cursor_line()
  while guard < 100000 do
    pmacs.editor.move_up()
    local now = pmacs.editor.cursor_line()
    if now == prev then break end
    prev = now
    guard = guard + 1
  end
  for _ = 1, target_line do
    pmacs.editor.move_down()
  end
end

M.move_cursor_to_visible_line = move_cursor_to_visible_line

local function current_source_byte(handle, view)
  local cur_line = pmacs.editor.cursor_line()
  return view.source_byte_at_visible_line(handle.projection, cur_line)
end

local function move_to_source_byte(handle, view, source_byte)
  local target_line = view.visible_line_at_source_byte(
    handle.projection, source_byte)
  if target_line < 0 then target_line = 0 end
  move_cursor_to_visible_line(target_line)
end

-- True iff `byte` falls inside any of the projection's folded
-- (hidden) ranges. Each fold record is `{ headline_byte_end,
-- byte_end, entry }`; the visible headline of a folded entry sits
-- in `[entry.byte_start, headline_byte_end)`, so its own byte_start
-- is *not* hidden, but every descendant's byte_start IS.
local function is_byte_hidden(folds, byte)
  for _, f in ipairs(folds) do
    if byte >= f[1] and byte < f[2] then return true end
  end
  return false
end

function M.next_headline(handle, parser, view)
  local cur = current_source_byte(handle, view)
  local entries = parser.entries(handle.parser_handle)
  local folds = (handle.projection and handle.projection.folds) or {}
  for _, e in ipairs(entries) do
    if e.byte_start > cur and not is_byte_hidden(folds, e.byte_start) then
      move_to_source_byte(handle, view, e.byte_start)
      return
    end
  end
end

function M.parent_headline(handle, parser, view)
  local cur = current_source_byte(handle, view)
  local entry = parser.entry_at(handle.parser_handle, cur)
  if not entry then return end
  local p = parser.parent(handle.parser_handle, entry)
  if p then
    move_to_source_byte(handle, view, p.byte_start)
  end
end

return M
