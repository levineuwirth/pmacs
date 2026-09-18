-- diffmode.lua --- the `diff` major mode: hunk motion over a unified
-- diff (E7.2).
--
-- A buffer whose major mode is `diff` gets `n` and `p` bound to the
-- next and previous hunk, where a hunk begins at a line starting with
-- `@@`. That is the whole mode: no grammar (there is no bundled `diff`
-- grammar, and the buffer stays plain text), no gutter, no apply. The
-- keymap rides the mode, as dired's does (Q#DR8), so any buffer that
-- takes the mode takes the keys, and `*git-diff*` takes it when it is
-- created.
--
-- Motion sets the cursor's line and leaves the viewport to the frame:
-- the paint pass brings the focused window's cursor into view
-- (`prepare_window_cursor_visible` in `src/editor.rs`) and the motion
-- primitives themselves do not scroll (`move_down` touches no
-- `view_top`; only paging does), so walking line by line would buy
-- nothing but calls. Measured in `tests/e7_diff_mode_acceptance.rs`:
-- after a jump to a hunk seven hundred lines down, `view_top` is
-- unchanged until the next paint, and that paint shows the hunk.

pmacs.diffmode = pmacs.diffmode or {}

--- The 0-based line numbers at which hunks begin in `text`: every line
--- that starts with `@@`. Pure, so the boundary cases (no hunk, one
--- hunk, a hunk on the last line without a newline) are testable
--- without a buffer.
function pmacs.diffmode.hunk_lines(text)
  local out = {}
  local line = 0
  local pos = 1
  local len = #text
  while pos <= len do
    local nl = text:find("\n", pos, true)
    local stop = nl and (nl - 1) or len
    if text:sub(pos, pos + 1) == "@@" then
      out[#out + 1] = line
    end
    if not nl then break end
    pos = nl + 1
    line = line + 1
  end
  return out
end

local function active_diff_buffer()
  local buf = pmacs.window.buffer()
  if not buf then return nil end
  local ok, mode = pcall(pmacs.buffer.major_mode, buf)
  if not ok or mode ~= "diff" then
    pmacs.editor.set_status("diff: not a diff buffer")
    return nil
  end
  return buf
end

local function current_hunks(buf)
  local ok, text = pcall(function() return buf:slice(0, buf:len()) end)
  if not ok or type(text) ~= "string" then return {} end
  return pmacs.diffmode.hunk_lines(text)
end

-- Put the cursor at the start of line `target`.
local function go_to_line(target)
  pmacs.editor.move_to_line(target)
  pmacs.editor.move_line_start()
end

--- `n`: the first hunk that begins after the cursor's line. At the last
--- hunk (or past it) the cursor stays and the status line says so; in
--- a diff with no hunk it says that instead.
function pmacs.diffmode.next_hunk()
  local buf = active_diff_buffer()
  if not buf then return end
  local hunks = current_hunks(buf)
  if #hunks == 0 then
    pmacs.editor.set_status("diff: no hunks")
    return
  end
  local here = pmacs.editor.cursor_line()
  for _, line in ipairs(hunks) do
    if line > here then
      go_to_line(line)
      return
    end
  end
  pmacs.editor.set_status("diff: no next hunk")
end

--- `p`: the last hunk that begins before the cursor's line. At the
--- first hunk (or above it, in the header) the cursor stays and the
--- status line says so.
function pmacs.diffmode.previous_hunk()
  local buf = active_diff_buffer()
  if not buf then return end
  local hunks = current_hunks(buf)
  if #hunks == 0 then
    pmacs.editor.set_status("diff: no hunks")
    return
  end
  local here = pmacs.editor.cursor_line()
  for i = #hunks, 1, -1 do
    if hunks[i] < here then
      go_to_line(hunks[i])
      return
    end
  end
  pmacs.editor.set_status("diff: no previous hunk")
end

pmacs.command.define {
  name = "diff.next-hunk",
  description = "Move to the next hunk of the diff in the current buffer.",
  fn = pmacs.diffmode.next_hunk,
}

pmacs.command.define {
  name = "diff.previous-hunk",
  description = "Move to the previous hunk of the diff in the current buffer.",
  fn = pmacs.diffmode.previous_hunk,
}

pmacs.keymap.bind { scope = "mode", mode = "diff", sequence = "n", command = "diff.next-hunk" }
pmacs.keymap.bind { scope = "mode", mode = "diff", sequence = "p", command = "diff.previous-hunk" }
