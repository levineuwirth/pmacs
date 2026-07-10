-- indent.lua --- auto-indent on newline (Arc 2).
--
-- RET (`edit.newline-and-indent`) inserts a newline plus the current
-- line's leading whitespace, verbatim, clipped at the split point
-- (Q#AI3): copying bytes is the only policy that cannot be wrong about
-- tabs-vs-spaces, and the clip keeps a split inside the indent from
-- double-indenting the carried text. The whole thing is ONE edit — one
-- undo step, one CRDT op. With a region it is one `buf:replace` (CUA
-- type-over, Q#AI4); the selection is cleared after every successful
-- edit, region or not (a zero-length selection would otherwise go live
-- the moment the cursor moves off the anchor). `buffer.newline` stays
-- bound-free as the plain-newline escape hatch (Q#AI2).
--
-- Framing: docs/auto-indent-framing.md.

pmacs.indent = pmacs.indent or {}

local ed = pmacs.editor

-- Start of the line containing `pos`: chunked backward scan for the
-- last newline strictly before it (comment.lua's scan — there is no
-- line-access API on buffers; giant lines stay safe).
local function line_start_before(buf, pos)
  local p = pos
  while p > 0 do
    local from = math.max(0, p - 4096)
    local chunk = buf:slice(from, p)
    local nl = chunk:match("()\n[^\n]*$")
    if nl then return from + nl end
    p = from
  end
  return 0
end

-- The indent to carry over a split at `split` (Q#AI3):
-- bytes[line_start .. min(first_non_ws, split)]. Slicing the line head
-- up to the split point and taking its leading `[ \t]*` run IS that
-- clip — the match cannot run past the slice's end. `[ \t]` rather
-- than `%s` so a CR on a CRLF line never counts as indent.
local function indent_before(buf, split)
  local start = line_start_before(buf, split)
  if split <= start then return "" end
  return buf:slice(start, split):match("^[ \t]*")
end

-- Right-gravity translation of `pos` through the effective edit
-- (Q#AI5; the daemon optimistic-arm shape). `estop` is the PRE-edit
-- end of the replaced range; an insert has estart == estop.
local function translate(pos, estart, estop, einserted)
  if pos < estart then return pos end
  if pos > estop then return pos - (estop - estart) + einserted end
  return estart + einserted
end

-- edit.newline-and-indent body.
function pmacs.indent.newline()
  local buf = pmacs.window.buffer()
  if not buf then
    ed.set_status("no buffer")
    return false
  end

  -- Snapshot the context BEFORE the edit (Q#AI5): intercepts run with
  -- the registry borrow released and may switch window or buffer; the
  -- fix-up below must never touch whatever is active afterwards.
  local win0 = pmacs.window.current()
  local cursor0 = ed.cursor()

  local region = ed.region()
  local has_region = region ~= nil and region["end"] > region.start
  local rstart, rstop
  if has_region then
    rstart, rstop = region.start, region["end"]
  else
    rstart, rstop = cursor0, cursor0
  end
  local text = "\n" .. indent_before(buf, rstart)

  -- One edit = one undo step, one CRDT op. Same intercept discipline
  -- as killring/comment: a rejection reports rather than throws and
  -- leaves no state behind.
  local ok, estart, estop, einserted = pcall(function()
    if has_region then
      return buf:replace(rstart, rstop, text)
    end
    return buf:insert(rstart, text)
  end)
  if not ok then
    ed.set_status("newline-and-indent rejected by buffer intercept")
    return false
  end

  -- Context guard (Q#AI5): fix up only the window that made the edit.
  if pmacs.window.current() ~= win0 or pmacs.window.buffer() ~= buf then
    ed.set_status("newline-and-indent: context changed during edit")
    return false
  end

  -- A deviating effective edit means an intercept rewrote it — the
  -- interceptor's positional result stands (M6.4: kind and payload
  -- are immutable). Cursor repair uses ONE formula for the clean and
  -- transformed paths alike: translate the pre-edit cursor through
  -- the effective edit, then goto_byte (which clamps). The clean
  -- insert-at-cursor case lands at estart + einserted — right after
  -- the carried indent.
  local deviated = estart ~= rstart or estop ~= rstop or einserted ~= #text
  if deviated then
    ed.set_status("newline-and-indent altered by buffer intercept")
  end
  ed.goto_byte(translate(cursor0, estart, estop, einserted))
  ed.clear_selection()
  return not deviated
end

pmacs.command.define {
  name = "edit.newline-and-indent",
  description = "Insert a newline carrying the current line's indentation.",
  fn = function() pmacs.indent.newline() end,
}
