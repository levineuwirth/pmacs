-- comment.lua --- language-aware comment/uncomment (Arc 2).
--
-- `M-;` (`edit.toggle-comment`) comments or uncomments the current
-- line — or every line the region touches — using the language's line
-- prefix from the public `pmacs.comment.strings` table. Semantics
-- (Q#CT4): uncomment iff every non-blank line already starts (after
-- its indentation) with the prefix; otherwise comment, inserting
-- `prefix .. " "` at the minimum indentation of the span's non-blank
-- lines (Emacs comment-region alignment). Blank lines are skipped in
-- both directions. The whole toggle is ONE `buf:replace` (Q#CT5): one
-- undo step, one CRDT op, one effective-edit verification.
--
-- Named deviation (Q#CT2): the no-region case is Emacs `comment-line`
-- (toggle, then move to the next line so repeated `M-;` walks a
-- block), not `comment-dwim`'s append-comment-at-EOL.
--
-- Framing: docs/archive/framings/comment-toggle-framing.md.

pmacs.comment = pmacs.comment or {}

local ed = pmacs.editor

-- Language → line-comment prefix (Q#CT3). Public and user-extensible,
-- like `pmacs.lsp.filetypes`: `pmacs.comment.strings.mylang = ";;"`.
-- Block comments are a named deferral.
pmacs.comment.strings = {
  rust = "//",
  c = "//",
  cpp = "//",
  go = "//",
  zig = "//",
  javascript = "//",
  typescript = "//",
  javascriptreact = "//",
  typescriptreact = "//",
  lua = "--",
  python = "#",
  bash = "#",
  sh = "#",
  toml = "#",
  yaml = "#",
  -- Lean 4 (framing Q#LN5). `--` only: Lean's block comment is `/- -/` and
  -- its docstring `/-- -/`, but block-comment toggling is the comment arc's
  -- own named deferral and this lane does not front-run it.
  lean4 = "--",
}

-- Start of the line containing `pos`: chunked backward scan for the
-- last newline strictly before it (same chunk discipline as
-- killring's forward scan — giant lines stay safe).
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

-- Byte offset of the first newline at or after `pos`, or `len`.
local function line_end_at(buf, pos, len)
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

-- Split span text (no trailing newline) into lines, preserving empties.
local function split_lines(text)
  local lines = {}
  local i = 1
  while true do
    local nl = text:find("\n", i, true)
    if not nl then
      table.insert(lines, text:sub(i))
      break
    end
    table.insert(lines, text:sub(i, nl - 1))
    i = nl + 1
  end
  return lines
end

local function is_blank(line)
  return line:match("^%s*$") ~= nil
end

-- Leading indentation in BYTES. `[ \t]` rather than `%s` so a CR on a
-- CRLF line never counts as indent.
local function indent_of(line)
  return line:match("^[ \t]*")
end

-- edit.toggle-comment body.
function pmacs.comment.toggle()
  local buf = pmacs.window.buffer()
  if not buf then
    ed.set_status("no buffer")
    return false
  end
  local lang = pmacs.lsp.active_buffer_language()
  local prefix = lang and pmacs.comment.strings[lang]
  if not prefix then
    ed.set_status("no comment syntax known for " .. (lang or "this buffer"))
    return false
  end

  local len = buf:len()
  local region = ed.region()
  local has_region = region ~= nil and region["end"] > region.start
  local span_first, span_last_end
  if has_region then
    span_first = line_start_before(buf, region.start)
    -- The last line the region TOUCHES: a region ending at column 0
    -- stops at the previous line (Emacs comment-region), hence end-1.
    span_last_end = line_end_at(buf, region["end"] - 1, len)
  else
    local cursor = ed.cursor()
    span_first = line_start_before(buf, cursor)
    span_last_end = line_end_at(buf, cursor, len)
  end

  local lines = split_lines(buf:slice(span_first, span_last_end))

  -- Classify (Q#CT4): uncomment iff every non-blank line is commented;
  -- blank lines neither count nor contribute to the min indent.
  local any_nonblank = false
  local all_commented = true
  local min_indent = nil
  for _, line in ipairs(lines) do
    if not is_blank(line) then
      any_nonblank = true
      local ind = indent_of(line)
      if line:sub(#ind + 1, #ind + #prefix) ~= prefix then
        all_commented = false
      end
      if min_indent == nil or #ind < min_indent then min_indent = #ind end
    end
  end
  if not any_nonblank then
    ed.set_status("nothing to comment")
    return false
  end

  for i, line in ipairs(lines) do
    if not is_blank(line) then
      if all_commented then
        local ind = indent_of(line)
        local rest = line:sub(#ind + 1 + #prefix)
        if rest:sub(1, 1) == " " then rest = rest:sub(2) end
        lines[i] = ind .. rest
      else
        lines[i] = line:sub(1, min_indent)
          .. prefix
          .. " "
          .. line:sub(min_indent + 1)
      end
    end
  end
  local new_text = table.concat(lines, "\n")

  -- One replace = one undo step, one CRDT op (Q#CT5). Same intercept
  -- discipline as killring: a rejection reports rather than throws,
  -- and any deviation of the EFFECTIVE edit from the request means an
  -- intercept rewrote it — the interceptor's result stands and the
  -- cursor fix-up is skipped (a moved span makes it meaningless).
  local ok, estart, estop, einserted = pcall(function()
    return buf:replace(span_first, span_last_end, new_text)
  end)
  if not ok then
    ed.set_status("comment toggle rejected by buffer intercept")
    return false
  end
  if estart ~= span_first or estop ~= span_last_end or einserted ~= #new_text then
    ed.set_status("comment toggle altered by buffer intercept")
    return false
  end

  if has_region then
    -- CUA convention after a region op: selection off, cursor at the
    -- span start (Q#CT2).
    ed.clear_selection()
    ed.goto_byte(span_first)
  else
    -- comment-line behavior: move to the next line so repeated M-;
    -- walks down the block. The byte after the rewritten span is the
    -- old trailing newline iff one existed.
    local new_end = span_first + #new_text
    if new_end < buf:len() then
      ed.goto_byte(new_end + 1)
    else
      ed.goto_byte(new_end)
    end
  end
  return true
end

pmacs.command.define {
  name = "edit.toggle-comment",
  description = "Comment or uncomment the current line or selected lines.",
  fn = function() pmacs.comment.toggle() end,
}

pmacs.keymap.bind { scope = "global", sequence = "M-;", command = "edit.toggle-comment" }
