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
-- Framing: docs/archive/framings/auto-indent-framing.md.

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
-- bytes[line_start .. min(first_non_ws, split)]. Forward chunked scan
-- from the line start, stopping at the first non-whitespace byte —
-- never materializing more of the line than the indent itself plus
-- one chunk (Enter at the end of a giant minified line must not copy
-- the whole line just to produce an empty indent). `[ \t]` rather
-- than `%s` so a CR on a CRLF line never counts as indent.
local function indent_before(buf, split)
  local start = line_start_before(buf, split)
  local parts = {}
  local p = start
  while p < split do
    local chunk_to = math.min(p + 4096, split)
    local chunk = buf:slice(p, chunk_to)
    local ws = chunk:match("^[ \t]*")
    table.insert(parts, ws)
    if #ws < #chunk then break end
    p = chunk_to
  end
  return table.concat(parts)
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

-- TAB (E7b.4): Emacs's `tab-always-indent` set to `complete`. TAB
-- indents the line, and when the line is already at the indentation
-- TAB would produce, TAB completes at point instead.
--
-- The indent function is the one this file already has: a line's
-- indentation is the leading whitespace of the nearest non-blank line
-- above it, copied byte for byte, and nothing at the top. Language
-- knows nothing here, deliberately --- E1.6's clause asks for
-- language-aware indentation and stays the owner's; this settles what
-- the TAB key does on both frontends, and nothing about what the right
-- indentation is.
--
-- The decision follows Emacs's exactly: TAB completes only when the
-- indent command changed neither the buffer nor point. So a line
-- whose indentation is wrong is re-indented (point inside the old
-- indentation lands after the new one; point past it keeps its
-- distance from the text); a line whose indentation is right with
-- point inside it moves point to the indentation, and only then, on
-- the next TAB, does completion open. With an active region the key
-- keeps the CUA type-over it had (`buffer.tab`): indenting a region
-- needs the engine E1.6 owns.
--
-- One fallback past Emacs's rule (C7b fix round 1): when there is
-- nothing to complete --- nothing but whitespace, or the line's start,
-- immediately before point --- TAB is the indent alone. Without it an
-- empty first line, at its indentation by definition, opened the
-- popup with an empty prefix on the first TAB and the second TAB, the
-- popup's accept, inserted whatever the server listed first. A word
-- or a symbol before point (`prin`, `self.`) still completes.

-- The byte after the last character of the line holding `pos`: the
-- position of its newline, or the buffer's length on the last line.
-- Chunked forward scan, like `indent_before`.
local function line_end_after(buf, pos)
  local len = buf:len()
  local p = pos
  while p < len do
    local chunk_to = math.min(p + 4096, len)
    local chunk = buf:slice(p, chunk_to)
    local nl = chunk:find("\n", 1, true)
    if nl then return p + nl - 1 end
    p = chunk_to
  end
  return len
end

-- The indentation TAB would give the line starting at `line_start`:
-- the leading whitespace of the nearest non-blank line above it, ""
-- when there is none.
local function indent_wanted(buf, line_start)
  local p = line_start
  while p > 0 do
    local prev_start = line_start_before(buf, p - 1)
    local line = buf:slice(prev_start, p - 1)
    if line:match("[^ \t\r]") then
      return line:match("^[ \t]*")
    end
    p = prev_start
  end
  return ""
end

-- Whether point has nothing before it on its line but whitespace:
-- the buffer's start, a newline, a space or a tab.
local function nothing_to_complete_before(buf, cursor)
  if cursor <= 0 then return true end
  local ok, ch = pcall(function() return buf:slice(cursor - 1, cursor) end)
  if not ok or type(ch) ~= "string" then return true end
  return ch == "\n" or ch == " " or ch == "\t"
end

function pmacs.indent.tab()
  local buf = pmacs.window.buffer()
  if not buf then
    ed.set_status("no buffer")
    return false
  end
  local region = ed.region()
  if region ~= nil and region["end"] > region.start then
    ed.insert_char_over_region(9)
    return true
  end
  local cursor = ed.cursor()
  local start = line_start_before(buf, cursor)
  local stop = line_end_after(buf, cursor)
  local have = indent_before(buf, stop)
  local want = indent_wanted(buf, start)
  local indent_end = start + #have
  if have ~= want then
    local ok, estart, estop, einserted = pcall(function()
      return buf:replace(start, indent_end, want)
    end)
    if not ok then
      ed.set_status("indent rejected by buffer intercept")
      return false
    end
    -- Point inside the old indentation lands after the new one; point
    -- past it keeps its distance from the text. One formula, through
    -- the effective edit, as `newline` does.
    if cursor <= indent_end then
      ed.goto_byte(estart + einserted)
    else
      ed.goto_byte(translate(cursor, estart, estop, einserted))
    end
    return true
  end
  if cursor < indent_end then
    ed.goto_byte(indent_end)
    return true
  end
  if nothing_to_complete_before(buf, cursor) then
    return true
  end
  pmacs.command.invoke("completion.at-point")
  return true
end

pmacs.command.define {
  name = "edit.indent-or-complete",
  description = "Indent the line to the line above's indentation; if it already is and a word or symbol is before point, complete at point.",
  fn = function() pmacs.indent.tab() end,
}
